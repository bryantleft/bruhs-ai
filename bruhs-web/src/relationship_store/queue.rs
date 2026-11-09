use std::collections::{btree_map::Entry, BTreeMap};

use bruhs_models::RelationshipProfile;

#[derive(Clone, Debug)]
pub(super) struct Failure {
    pub message: String,
    pub conflict: bool,
}

#[derive(Clone)]
pub(super) struct Pending {
    pub expected: Option<RelationshipProfile>,
    pub draft: RelationshipProfile,
    pub failure: Option<Failure>,
}

/// At most one baseline and current draft per dirty person. Later edits are
/// coalesced without changing the baseline of an already-issued transaction.
#[derive(Default)]
pub(super) struct Queue(BTreeMap<String, Pending>);

impl Queue {
    pub fn edit(&mut self, expected: Option<RelationshipProfile>, draft: RelationshipProfile) {
        match self.0.entry(draft.person_id.clone()) {
            Entry::Occupied(mut entry) => entry.get_mut().draft = draft,
            Entry::Vacant(entry) => {
                entry.insert(Pending {
                    expected,
                    draft,
                    failure: None,
                });
            }
        }
    }

    pub fn next(&self) -> Option<Pending> {
        self.0
            .values()
            .find(|pending| pending.failure.is_none())
            .cloned()
    }

    pub fn saved(&mut self, saved: RelationshipProfile) {
        let Some(pending) = self.0.get_mut(&saved.person_id) else {
            return;
        };
        if pending.draft == saved {
            self.0.remove(&saved.person_id);
        } else {
            pending.expected = Some(saved);
        }
    }

    pub fn failed(&mut self, person_id: &str, failure: Failure) {
        if let Some(pending) = self.0.get_mut(person_id) {
            pending.failure = Some(failure);
        }
    }

    pub fn editable(&self, person_id: &str) -> bool {
        !self
            .0
            .get(person_id)
            .and_then(|pending| pending.failure.as_ref())
            .is_some_and(|failure| failure.conflict)
    }

    pub fn retry(&mut self) {
        for pending in self.0.values_mut() {
            if pending
                .failure
                .as_ref()
                .is_some_and(|failure| !failure.conflict)
            {
                pending.failure = None;
            }
        }
    }

    pub fn failure(&self) -> Option<&Failure> {
        self.0
            .values()
            .filter_map(|pending| pending.failure.as_ref())
            .find(|failure| !failure.conflict)
            .or_else(|| self.0.values().find_map(|pending| pending.failure.as_ref()))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn recovery_json(&self) -> String {
        serde_json::to_string_pretty(
            &self
                .0
                .values()
                .map(|pending| &pending.draft)
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| "Recovery data could not be serialized. Keep this tab open.".into())
    }
}

#[cfg(test)]
mod tests;
