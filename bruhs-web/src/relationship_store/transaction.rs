use std::collections::HashMap;

use bruhs_models::PersonReminderRollback;

use super::*;

pub type ExpectedRelationships = HashMap<String, Option<RelationshipProfile>>;

/// Owns the editor exclusion until every workflow path commits or exits.
pub struct RelationshipTransaction {
    store: RelationshipStore,
    expected: ExpectedRelationships,
}

impl RelationshipStore {
    pub fn settled(self) -> bool {
        self.loaded.get() && !self.exclusive.get() && self.queue.with(Queue::is_empty)
    }

    pub fn begin_transaction(
        self,
        person_ids: &[String],
    ) -> Result<RelationshipTransaction, SaveError> {
        if self
            .merge_locks
            .with_untracked(|locked| person_ids.iter().any(|id| locked.contains(id)))
        {
            return Err(SaveError::Rejected(
                "This relationship has a pending contact merge. Retry that merge in People first.",
            ));
        }
        self.begin_exclusive(person_ids)
    }

    pub fn set_merge_locks(self, ids: Vec<String>) {
        self.merge_locks.set(ids.into_iter().collect());
    }

    pub fn begin_merge(self, person_ids: &[String]) -> Result<RelationshipTransaction, SaveError> {
        if self
            .editor_drafts
            .with_untracked(|drafts| person_ids.iter().any(|id| drafts.contains_key(id)))
        {
            return Err(SaveError::Rejected(
                "Save the unsaved relationship details before merging these contacts.",
            ));
        }
        self.begin_exclusive(person_ids)
    }

    fn begin_exclusive(self, person_ids: &[String]) -> Result<RelationshipTransaction, SaveError> {
        if !self.settled() {
            return Err(SaveError::Rejected("Relationship data is loading, saving, or needs recovery. Open People and finish its save before running this workflow. No records were changed."));
        }
        if person_ids.is_empty() || person_ids.len() > 128 {
            return Err(SaveError::Rejected(
                "Invalid People workflow targets. No records were changed.",
            ));
        }
        let mut expected = person_ids
            .iter()
            .map(|id| (id.clone(), None))
            .collect::<ExpectedRelationships>();
        self.records.with_untracked(|records| {
            for record in records {
                if let Some(slot) = expected.get_mut(&record.person_id) {
                    if slot.is_some() {
                        return Err(SaveError::Corrupt);
                    }
                    *slot = Some(record.clone());
                }
            }
            Ok(())
        })?;
        self.exclusive.set(true);
        self.status.set(SaveState::Saving);
        Ok(RelationshipTransaction {
            store: self,
            expected,
        })
    }
}

impl RelationshipTransaction {
    pub fn expected(&self) -> &ExpectedRelationships {
        &self.expected
    }

    pub fn created(self, profile: RelationshipProfile) {
        self.store.set_records.update(|records| {
            if let Some(current) = records
                .iter_mut()
                .find(|current| current.person_id == profile.person_id)
            {
                *current = profile;
            } else {
                records.push(profile);
            }
        });
    }

    pub fn rolled_back(self, plan: PersonReminderRollback) {
        self.store.set_records.update(|records| plan.apply(records));
    }

    pub fn merged(self, source_id: &str, target: Option<RelationshipProfile>) {
        self.store.set_records.update(|records| {
            records.retain(|profile| profile.person_id != source_id);
            if let Some(target) = target {
                if let Some(current) = records
                    .iter_mut()
                    .find(|profile| profile.person_id == target.person_id)
                {
                    *current = target;
                } else {
                    records.push(target);
                }
            }
        });
    }
}

impl Drop for RelationshipTransaction {
    fn drop(&mut self) {
        // No optimistic workflow records are published on failure. The editor
        // remains on its original saved state; workflow status owns the error.
        self.store.exclusive.set(false);
        self.store.refresh_status();
    }
}
