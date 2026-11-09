use super::*;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PeopleSnapshot {
    pub records: Vec<PersonProfile>,
    pub pending_creates: HashSet<String>,
    pub merge_archives: BTreeMap<String, Arc<PeopleMergeArchive>>,
    pub merge_recoveries: BTreeMap<String, Vec<Arc<PeopleMergeArchive>>>,
}

/// Exact local originals, saved atomically with the canonical contact and links.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeopleMergeArchive {
    /// None means the local source profile was absent, not an invented contact.
    pub source: Option<PersonProfile>,
    pub target: Option<PersonProfile>,
    pub source_was_pending: bool,
    pub source_relationship: Option<bruhs_models::RelationshipProfile>,
    pub target_relationship: Option<bruhs_models::RelationshipProfile>,
    pub target_id: String,
}

mod reconcile;

#[cfg(test)]
mod tests;

impl MeetingsHub {
    pub fn people_snapshot(&self) -> PeopleSnapshot {
        PeopleSnapshot {
            records: self.people.clone(),
            pending_creates: self.pending_person_creates.clone(),
            merge_archives: self.person_merge_archives.clone(),
            merge_recoveries: self.person_merge_recoveries.clone(),
        }
    }

    /// Restore before starting any remote refresh; no networking or notifications.
    pub fn restore_people(&mut self, snapshot: PeopleSnapshot) {
        self.people = snapshot.records;
        self.pending_person_creates = snapshot.pending_creates;
        self.person_merge_archives = snapshot.merge_archives;
        self.person_merge_recoveries = snapshot.merge_recoveries;
        self.person_merges_waiting.clear();
        self.people_refresh_failed = false;
        self.person_creates_in_flight.clear();
        self.person_creates_attempted.clear();
        self.bump_search_revision();
    }

    pub fn pending_people_count(&self) -> usize {
        self.pending_person_creates
            .union(&self.person_merges_waiting)
            .count()
    }

    pub fn needs_people_retry(&self) -> bool {
        self.pending_people_count() > 0 || self.people_refresh_failed
    }

    pub fn retry_pending_people(&mut self, cx: &mut Context<'_, Self>) {
        self.person_creates_attempted.clear();
        self.refresh_people(cx);
    }

    pub(super) fn people_changed(&mut self, cx: &mut Context<'_, Self>) {
        self.bump_search_revision();
        cx.emit(HubEvent::PeopleChanged);
        cx.notify();
    }

    pub(super) fn merge_people_cache(
        &mut self,
        people: Vec<PersonProfile>,
        cx: &mut Context<'_, Self>,
    ) {
        let mut changed = false;
        let mut positions: std::collections::HashMap<_, _> = self
            .people
            .iter()
            .enumerate()
            .map(|(index, person)| (person.id.clone(), index))
            .collect();
        for person in people {
            // A refresh is not an acknowledgment of this client's pending create.
            if self.pending_person_creates.contains(&person.id)
                || self.person_merge_archives.contains_key(&person.id)
            {
                continue;
            }
            if let Some(&index) = positions.get(&person.id) {
                let current = &mut self.people[index];
                if *current != person && current.updated_at <= person.updated_at {
                    *current = person;
                    changed = true;
                }
            } else {
                positions.insert(person.id.clone(), self.people.len());
                self.people.push(person);
                changed = true;
            }
        }
        if changed {
            self.people_changed(cx);
        }
    }

    pub(super) fn sync_pending_people(&mut self, cx: &mut Context<'_, Self>) {
        let available = 4usize.saturating_sub(self.person_creates_in_flight.len());
        let pending: Vec<_> = self
            .people
            .iter()
            .filter(|person| {
                self.pending_person_creates.contains(&person.id)
                    && !self.person_merge_archives.contains_key(&person.id)
                    && !self.person_merges_waiting.contains(&person.id)
                    && !self.person_creates_in_flight.contains(&person.id)
                    && !self.person_creates_attempted.contains(&person.id)
            })
            .take(available)
            .cloned()
            .collect();
        for person in pending {
            self.person_creates_in_flight.insert(person.id.clone());
            self.person_creates_attempted.insert(person.id.clone());
            let sent = person.clone();
            let client = self.client.clone();
            self.run(
                async move { client.create_person(&person).await },
                move |hub, result, cx| {
                    hub.person_creates_in_flight.remove(&sent.id);
                    hub.apply_person_create(&sent, result, cx);
                    hub.sync_pending_people(cx);
                },
            );
        }
        cx.notify();
    }

    fn apply_person_create(
        &mut self,
        sent: &PersonProfile,
        result: Result<PersonProfile, ServerError>,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.pending_person_creates.contains(&sent.id)
            || self.person_merge_archives.contains_key(&sent.id)
            || self.person_merges_waiting.contains(&sent.id)
        {
            return;
        }
        let Some(current) = self.people.iter_mut().find(|item| item.id == sent.id) else {
            return;
        };
        // A completion for an older capture cannot replace a newer local revision.
        if current != sent {
            return;
        }
        match result {
            Ok(saved) if saved.id == sent.id && saved.display_name == sent.display_name.trim() => {
                *current = saved;
                self.pending_person_creates.remove(&sent.id);
                self.server_online = true;
                self.people_changed(cx);
            }
            Ok(_) => self.notify_notice("Contact sync returned a different identity or name. The local contact is unchanged and still pending.", true, cx),
            Err(_) => self.notify_notice("Contact remains in this workspace; server sync failed. Check local save status. Retry contact sync from People when the server is available.", true, cx),
        }
    }

    #[cfg(test)]
    pub(crate) fn set_people_test_client(&mut self, client: ServerClient) {
        self.client = Arc::new(client);
    }
}
