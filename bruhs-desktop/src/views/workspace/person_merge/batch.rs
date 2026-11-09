use super::*;
use std::collections::HashMap;

pub(super) struct UniqueRecords<'a, T> {
    entries: HashMap<&'a str, Option<&'a T>>,
    duplicate_error: &'static str,
}

impl<'a, T> UniqueRecords<'a, T> {
    pub(super) fn new(
        records: &'a [T],
        key: impl Fn(&'a T) -> &'a str,
        duplicate_error: &'static str,
    ) -> Self {
        let mut entries = HashMap::with_capacity(records.len());
        for record in records {
            entries
                .entry(key(record))
                .and_modify(|value| *value = None)
                .or_insert(Some(record));
        }
        Self {
            entries,
            duplicate_error,
        }
    }

    pub(super) fn get(&self, id: &str) -> Result<Option<&'a T>, &'static str> {
        match self.entries.get(id) {
            Some(Some(record)) => Ok(Some(record)),
            Some(None) => Err(self.duplicate_error),
            None => Ok(None),
        }
    }
}

pub(super) struct MergeCandidate {
    pub source_id: String,
    pub target: Result<PersonProfile, &'static str>,
}

impl Workspace {
    pub(super) fn apply_person_merge_candidates(
        &mut self,
        candidates: Vec<MergeCandidate>,
        cx: &mut Context<'_, Self>,
    ) {
        if candidates.is_empty() {
            return;
        }
        self.meetings_hub.update(cx, |hub, _| {
            for candidate in &candidates {
                hub.wait_for_person_merge(candidate.source_id.clone());
            }
        });
        let hub = self.meetings_hub.read(cx);
        let people = UniqueRecords::new(
            &hub.people,
            |p| p.id.as_str(),
            "Duplicate contact identities prevent safe merge. Originals were retained.",
        );
        let relationships = UniqueRecords::new(
            &self.relationships,
            |p| p.person_id.as_str(),
            "Duplicate relationship identities prevent safe merge. Originals were retained.",
        );
        let source_ids: HashSet<_> = candidates.iter().map(|c| c.source_id.clone()).collect();
        let mut targets = BTreeMap::<String, RelationshipProfile>::new();
        let mut redirects = HashMap::new();
        let mut commits = Vec::new();
        let mut first_error = None;
        let mut recovered_name = false;
        for candidate in candidates {
            let id = candidate.source_id;
            let prepared = (|| {
                let redirect = PersonMergeResolution {
                    source_id: id.clone(),
                    target: candidate.target?,
                };
                if source_ids.contains(&redirect.target.id) {
                    return Err(
                        "Contact merge targets are not fully resolved. Originals were retained.",
                    );
                }
                self.validate_person_merge(&redirect, cx)?;
                let target_id = &redirect.target.id;
                let source_relationship = relationships.get(&id)?;
                let original_target = relationships.get(target_id)?;
                context::prepare_context(
                    &redirect,
                    people.get(&id)?,
                    people.get(target_id)?,
                    source_relationship,
                    targets.get(target_id).or(original_target),
                    hub.person_is_pending(&id),
                )
            })();
            match prepared {
                Ok(plan) => {
                    let target_id = plan.canonical.id.clone();
                    recovered_name |= plan.recovered_local_name;
                    if let Some(relationship) = plan.relationship {
                        targets.insert(target_id.clone(), relationship);
                    }
                    redirects.insert(id.clone(), target_id);
                    commits.push((id, plan.archive, plan.canonical));
                }
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        // Every successful source was preflighted against immutable originals and
        // staged survivor context. Conflicting sources stay untouched and pending.
        if !commits.is_empty() {
            self.relationships.retain(|p| {
                !redirects.contains_key(&p.person_id) && !targets.contains_key(&p.person_id)
            });
            self.relationships.extend(targets.into_values());
            self.retarget_private_person_links(&redirects);
            self.meetings_hub.update(cx, |hub, cx| {
                hub.commit_person_merges(commits, cx);
                if recovered_name {
                    hub.report_person_merge_conflict(RECOVERED_NAME_NOTICE.into(), cx);
                }
                hub.refresh_meetings(cx);
            });
            self.persist_and_notify(cx);
        }
        if let Some(error) = first_error {
            self.meetings_hub.update(cx, |hub, cx| {
                hub.report_person_merge_conflict(error.into(), cx)
            });
        }
    }
}
