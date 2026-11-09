use super::*;
use bruhs_models::PersonMergeResolution;

impl MeetingsHub {
    pub fn refresh_people_reconciled(
        &mut self,
        mut referenced_ids: HashSet<String>,
        _cx: &mut Context<'_, Self>,
    ) {
        if self.people_refreshing {
            self.people_refresh_again = true;
            return;
        }
        self.people_refreshing = true;
        let client = self.client.clone();
        referenced_ids.extend(self.people.iter().map(|p| p.id.clone()));
        referenced_ids.extend(self.pending_person_creates.iter().cloned());
        referenced_ids.extend(self.person_merges_waiting.iter().cloned());
        let ids: Vec<_> = referenced_ids
            .into_iter()
            .filter_map(|id| self.canonical_person_id(&id).ok().map(str::to_owned))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        self.run(async move {
            let people = client.list_people().await.map_err(|error| error.to_string())?;
            let present: HashSet<_> = people.iter().map(|p| p.id.as_str()).collect();
            let missing: Vec<_> = ids.into_iter().filter(|id| !present.contains(id.as_str())).collect();
            let mut redirects = Vec::new();
            // Bounded read-only batches; no POST create is allowed before resolution.
            for window in missing.chunks(64 * 4) {
                let mut requests = tokio::task::JoinSet::new();
                for batch in window.chunks(64) {
                    let client = client.clone();
                    let ids = batch.to_vec();
                    requests.spawn(async move { client.resolve_person_merges(&ids).await });
                }
                while let Some(result) = requests.join_next().await {
                    redirects.extend(
                        result
                            .map_err(|_| "Contact lookup task failed.".to_string())?
                            .map_err(|error| error.to_string())?,
                    );
                }
            }
            Ok::<_, String>((people, redirects))
        }, |hub, result, cx| {
            hub.people_refreshing = false;
            hub.people_refresh_failed = result.is_err();
            match result {
                Ok((people, redirects)) => {
                    hub.merge_people_cache(people, cx);
                    if hub.publish_person_merges(redirects, cx) {
                        hub.sync_pending_people(cx);
                    } else {
                        hub.people_refresh_failed = true;
                    }
                }
                Err(_) => hub.notify_notice("Contact reconciliation failed. Local contacts and drafts are retained; retry contact sync when the server is available.", true, cx),
            }
            if std::mem::take(&mut hub.people_refresh_again) { hub.refresh_people(cx); }
            cx.notify();
        });
    }

    pub(in crate::views::meetings_hub) fn publish_person_merge(
        &mut self,
        redirect: PersonMergeResolution,
        cx: &mut Context<'_, Self>,
    ) {
        if redirect.source_id == redirect.target.id
            || redirect.source_id.trim().is_empty()
            || redirect.target.id.trim().is_empty()
            || redirect.target.display_name.trim().is_empty()
        {
            self.notify_notice(
                "Invalid contact merge identity. Local records were retained.",
                true,
                cx,
            );
            return;
        }
        if self.person_merge_archives.contains_key(&redirect.source_id) {
            return;
        }
        self.person_merges_waiting
            .insert(redirect.source_id.clone());
        cx.emit(HubEvent::PersonMergeResolved(Box::new(redirect)));
    }

    fn publish_person_merges(
        &mut self,
        redirects: Vec<PersonMergeResolution>,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let mut sources = HashSet::with_capacity(redirects.len());
        if redirects.iter().any(|r| {
            r.source_id.trim().is_empty()
                || r.target.id.trim().is_empty()
                || r.target.display_name.trim().is_empty()
                || r.source_id == r.target.id
                || !sources.insert(r.source_id.as_str())
        }) {
            self.notify_notice("Invalid contact merge identities. Local records were retained; retry contact sync.", true, cx);
            return false;
        }
        // Protect every source before an event can run or queued creates resume.
        for redirect in &redirects {
            if !self.person_is_retired(&redirect.source_id) {
                self.person_merges_waiting
                    .insert(redirect.source_id.clone());
            }
        }
        if !redirects.is_empty() {
            cx.emit(HubEvent::PersonMergesResolved(redirects));
        }
        true
    }

    pub fn saved_person_merge_target(&self, id: &str) -> Option<&str> {
        self.person_merge_archives
            .get(id)
            .map(|archive| archive.target_id.as_str())
    }

    pub fn person_is_pending(&self, id: &str) -> bool {
        self.pending_person_creates.contains(id)
    }

    pub fn person_is_retired(&self, id: &str) -> bool {
        self.person_merge_archives.contains_key(id)
    }

    pub fn has_retired_people(&self) -> bool {
        !self.person_merge_archives.is_empty()
    }

    /// Follow saved merge evidence without rewriting it or contacting the server.
    pub fn canonical_person_id<'a>(&'a self, id: &'a str) -> Result<&'a str, &'static str> {
        let mut current = id;
        for _ in 0..=self.person_merge_archives.len() {
            if current.trim().is_empty() {
                return Err("A saved contact redirect is blank. Originals are retained.");
            }
            let Some(archive) = self.person_merge_archives.get(current) else {
                return Ok(current);
            };
            current = &archive.target_id;
        }
        Err("Saved contact redirects form a cycle. Originals are retained.")
    }

    pub fn wait_for_person_merge(&mut self, id: String) {
        self.person_merges_waiting.insert(id);
    }

    pub fn retain_needed_person_recoveries(&mut self, referenced_ids: &HashSet<String>) {
        self.person_merges_waiting.retain(|id| {
            !self.person_merge_archives.contains_key(id) || referenced_ids.contains(id)
        });
    }

    /// Resolve only this document's current links; historical versions stay exact.
    pub fn canonicalize_person_links(
        &self,
        links: &mut [bruhs_models::KnowledgeLink],
    ) -> Result<(), &'static str> {
        if !self.has_retired_people() {
            return Ok(());
        }
        let changes = links
            .iter()
            .enumerate()
            .filter(|(_, link)| {
                link.kind == bruhs_models::KnowledgeLinkKind::Person
                    && self.person_is_retired(&link.target_id)
            })
            .map(|(index, link)| {
                self.canonical_person_id(&link.target_id)
                    .map(|id| (index, id.to_owned()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        for (index, target_id) in changes {
            links[index].target_id = target_id;
        }
        Ok(())
    }

    /// Caller preflights private references first; no save/event occurs mid-commit.
    pub fn commit_person_merge(
        &mut self,
        source_id: String,
        archive: PeopleMergeArchive,
        target: PersonProfile,
        cx: &mut Context<'_, Self>,
    ) {
        self.commit_person_merges(vec![(source_id, archive, target)], cx);
    }

    /// One cache pass and notification for a fully preflighted recovery batch.
    pub fn commit_person_merges(
        &mut self,
        commits: Vec<(String, PeopleMergeArchive, PersonProfile)>,
        cx: &mut Context<'_, Self>,
    ) {
        if commits.is_empty() {
            return;
        }
        let sources: HashSet<_> = commits.iter().map(|(id, _, _)| id.clone()).collect();
        let mut targets = BTreeMap::new();
        for (source_id, archive, target) in commits {
            self.pending_person_creates.remove(&source_id);
            self.person_merges_waiting.remove(&source_id);
            self.person_creates_attempted.remove(&source_id);
            targets.insert(target.id.clone(), target);
            self.archive_person_merge(source_id, archive);
        }
        self.people
            .retain(|p| !sources.contains(&p.id) && !targets.contains_key(&p.id));
        self.people.extend(targets.into_values());
        self.bump_search_revision();
        cx.notify();
    }

    fn archive_person_merge(&mut self, source_id: String, archive: PeopleMergeArchive) {
        match self.person_merge_archives.entry(source_id) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(Arc::new(archive));
            }
            std::collections::btree_map::Entry::Occupied(entry) => {
                if archive.source.is_some() || archive.source_relationship.is_some() {
                    self.person_merge_recoveries
                        .entry(entry.key().clone())
                        .or_default()
                        .push(Arc::new(archive));
                }
            }
        }
    }

    pub fn report_person_merge_conflict(&mut self, message: String, cx: &mut Context<'_, Self>) {
        self.notify_notice(message, true, cx);
    }
}
