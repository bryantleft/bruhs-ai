use super::*;
use crate::views::meetings_hub::PeopleMergeArchive;
use bruhs_models::{merge_relationship_context, PersonMergeResolution};
mod batch;
mod bulk;
mod context;
mod incoming;
const RECOVERED_NAME_NOTICE: &str = "Merged contact recovered. The unsynced local name is retained in the surviving relationship's notes; exact originals are kept in the workspace file. Check save status before closing.";

impl Workspace {
    fn private_referenced_person_ids(&self) -> HashSet<String> {
        let mut ids = HashSet::new();
        ids.extend(self.relationships.iter().map(|p| p.person_id.clone()));
        ids.extend(
            self.captures
                .iter()
                .filter(|capture| capture.kind == CaptureKind::Contact)
                .filter_map(|capture| capture.target_id.clone()),
        );
        ids.extend(
            self.knowledge_documents
                .iter()
                .flat_map(|document| &document.links)
                .filter(|link| link.kind == KnowledgeLinkKind::Person)
                .map(|link| link.target_id.clone()),
        );
        ids.extend(
            self.inbox_conversations
                .iter()
                .flat_map(|conversation| &conversation.participants)
                .filter_map(|participant| participant.person_id.clone()),
        );
        ids
    }

    pub(super) fn referenced_person_ids(&self, cx: &Context<'_, Self>) -> HashSet<String> {
        let mut ids = self.private_referenced_person_ids();
        for meeting in &self.meetings_hub.read(cx).meetings {
            ids.extend(
                meeting
                    .speakers
                    .iter()
                    .filter_map(|speaker| speaker.person_id.clone()),
            );
            if let Some(summary) = &meeting.summary {
                ids.extend(
                    summary
                        .action_items
                        .iter()
                        .filter_map(|item| item.assignee_person_id.clone()),
                );
            }
        }
        ids
    }

    pub(super) fn reconcile_person_merge(
        &mut self,
        redirect: &PersonMergeResolution,
        cx: &mut Context<'_, Self>,
    ) -> Result<(), String> {
        let source_id = &redirect.source_id;
        let target_id = &redirect.target.id;
        let hub = self.meetings_hub.read(cx);
        if hub.person_is_retired(source_id) {
            if hub.canonical_person_id(source_id)? != target_id {
                return Err("Contact recovery no longer matches the saved redirect chain. Originals are retained.".into());
            }
            if !self.has_private_person_reference(source_id)
                && !hub.people.iter().any(|p| &p.id == source_id)
            {
                return Ok(());
            }
        }
        self.validate_person_merge(redirect, cx)?;
        let source = unique_person(&hub.people, source_id)?;
        let target = unique_person(&hub.people, target_id)?;
        let source_relationship = unique_relationship(&self.relationships, source_id)?;
        let target_relationship = unique_relationship(&self.relationships, target_id)?;
        let plan = context::prepare_context(
            redirect,
            source,
            target,
            source_relationship,
            target_relationship,
            hub.person_is_pending(source_id),
        )?;
        // All fallible preflight above; private links and original evidence share
        // the same whole-file save. No remote mutation or workflow re-approval.
        self.relationships
            .retain(|p| &p.person_id != source_id && &p.person_id != target_id);
        if let Some(relationship) = plan.relationship {
            self.relationships.push(relationship);
        }
        self.retarget_private_person_links(
            &[(source_id.clone(), target_id.clone())]
                .into_iter()
                .collect(),
        );
        self.meetings_hub.update(cx, |hub, cx| {
            hub.commit_person_merge(source_id.clone(), plan.archive, plan.canonical, cx);
            if plan.recovered_local_name {
                hub.report_person_merge_conflict(RECOVERED_NAME_NOTICE.into(), cx);
            }
            hub.refresh_meetings(cx);
        });
        self.persist_and_notify(cx);
        Ok(())
    }

    fn retarget_private_person_links(
        &mut self,
        redirects: &std::collections::HashMap<String, String>,
    ) {
        for capture in &mut self.captures {
            if capture.kind == CaptureKind::Contact {
                if let Some(target_id) = capture.target_id.as_ref().and_then(|id| redirects.get(id))
                {
                    capture.target_id = Some(target_id.clone());
                }
            }
        }
        for document in &mut self.knowledge_documents {
            for link in &mut document.links {
                if link.kind == KnowledgeLinkKind::Person {
                    if let Some(target_id) = redirects.get(&link.target_id) {
                        link.target_id.clone_from(target_id);
                    }
                }
            }
        }
        for conversation in &mut self.inbox_conversations {
            for participant in &mut conversation.participants {
                if let Some(target_id) = participant
                    .person_id
                    .as_ref()
                    .and_then(|id| redirects.get(id))
                {
                    participant.person_id = Some(target_id.clone());
                }
            }
        }
    }
}

impl Workspace {
    fn has_private_person_reference(&self, id: &str) -> bool {
        self.relationships.iter().any(|p| p.person_id == id)
            || self
                .captures
                .iter()
                .any(|c| c.kind == CaptureKind::Contact && c.target_id.as_deref() == Some(id))
            || self
                .knowledge_documents
                .iter()
                .flat_map(|d| &d.links)
                .any(|l| l.kind == KnowledgeLinkKind::Person && l.target_id == id)
            || self
                .inbox_conversations
                .iter()
                .flat_map(|c| &c.participants)
                .any(|p| p.person_id.as_deref() == Some(id))
    }

    fn validate_person_merge(
        &self,
        redirect: &PersonMergeResolution,
        cx: &Context<'_, Self>,
    ) -> Result<(), &'static str> {
        let source_id = &redirect.source_id;
        let target_id = &redirect.target.id;
        let hub = self.meetings_hub.read(cx);
        if source_id == target_id
            || source_id.trim().is_empty()
            || target_id.trim().is_empty()
            || redirect.target.display_name.trim().is_empty()
            || hub.person_is_retired(target_id)
        {
            return Err("Contact merge reply is stale or invalid. Local records are retained; retry contact sync.");
        }
        if [&self.renaming_person_id, &self.expanded_person_id]
            .iter()
            .any(|id| {
                id.as_ref()
                    .is_some_and(|id| id == source_id || id == target_id)
            })
        {
            return Err("A contact was merged on the server. Save or close its open profile editor, then retry contact sync. Your draft is retained.");
        }
        if hub.person_is_pending(target_id) {
            return Err("The surviving contact has a pending local edit. Finish its sync before retrying merge recovery.");
        }
        Ok(())
    }
}

fn unique_person<'a>(
    people: &'a [PersonProfile],
    id: &str,
) -> Result<Option<&'a PersonProfile>, &'static str> {
    let mut matches = people.iter().filter(|p| p.id == id);
    let first = matches.next();
    if matches.next().is_some() {
        return Err("Duplicate contact identities prevent safe merge. Originals were retained.");
    }
    Ok(first)
}

fn unique_relationship<'a>(
    profiles: &'a [RelationshipProfile],
    id: &str,
) -> Result<Option<&'a RelationshipProfile>, &'static str> {
    let mut matches = profiles.iter().filter(|p| p.person_id == id);
    let first = matches.next();
    if matches.next().is_some() {
        return Err(
            "Duplicate relationship identities prevent safe merge. Originals were retained.",
        );
    }
    Ok(first)
}
