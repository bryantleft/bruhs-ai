//! App-owned relationship editor: strict loading, optimistic edits, one writer.

use bruhs_models::RelationshipProfile;
use leptos::prelude::*;

use crate::durable_metadata::SaveState;
use crate::state::{RelationshipsSignal, SetRelationshipsSignal};
use crate::storage::{
    metadata::{yield_before_save, SaveError},
    relationships,
};

mod drafts;
mod queue;
mod transaction;
pub(crate) use drafts::{profile_fields, EditorDraft};
use queue::{Failure, Queue};
pub use transaction::{ExpectedRelationships, RelationshipTransaction};

#[derive(Clone, Copy)]
pub struct RelationshipStore {
    records: ReadSignal<Vec<RelationshipProfile>>,
    set_records: WriteSignal<Vec<RelationshipProfile>>,
    queue: RwSignal<Queue>,
    loaded: RwSignal<bool>,
    loading: RwSignal<bool>,
    saving: RwSignal<bool>,
    exclusive: RwSignal<bool>,
    merge_locks: RwSignal<std::collections::HashSet<String>>,
    editor_drafts: RwSignal<std::collections::BTreeMap<String, EditorDraft>>,
    pub(crate) draft_reset: RwSignal<u64>,
    pub status: RwSignal<SaveState>,
}

impl RelationshipStore {
    /// Submitted changes and private form drafts outlive the People view.
    /// A loading error alone is not an unsaved edit.
    pub(crate) fn pending(self) -> bool {
        let queued = self.queue.with(|queue| !queue.is_empty());
        let exclusive = self.exclusive.get();
        let drafts = self.editor_drafts.with(|drafts| !drafts.is_empty());
        queued || exclusive || drafts
    }

    pub fn hydrated(self) -> bool {
        self.loaded.get()
    }

    pub fn install() {
        let RelationshipsSignal(records) = expect_context::<RelationshipsSignal>();
        let SetRelationshipsSignal(set_records) = expect_context::<SetRelationshipsSignal>();
        let store = Self {
            records,
            set_records,
            queue: RwSignal::new(Queue::default()),
            loaded: RwSignal::new(false),
            loading: RwSignal::new(false),
            saving: RwSignal::new(false),
            exclusive: RwSignal::new(false),
            merge_locks: RwSignal::new(Default::default()),
            editor_drafts: RwSignal::new(Default::default()),
            draft_reset: RwSignal::new(0),
            status: RwSignal::new(SaveState::Loading),
        };
        provide_context(store);
        store.hydrate();
    }

    pub(crate) fn editor_draft(self, id: &str) -> Option<EditorDraft> {
        self.editor_drafts
            .with_untracked(|drafts| drafts.get(id).cloned())
    }

    pub(crate) fn keep_editor_draft(self, id: String, draft: EditorDraft) {
        let dirty = draft.dirty();
        let unchanged = self.editor_drafts.with_untracked(|drafts| {
            if dirty {
                drafts.get(&id) == Some(&draft)
            } else {
                !drafts.contains_key(&id)
            }
        });
        if unchanged {
            return;
        }
        self.editor_drafts.update(|drafts| {
            if dirty {
                drafts.insert(id, draft);
            } else {
                drafts.remove(&id);
            }
        });
    }

    pub(crate) fn editor_draft_ids(self) -> Vec<String> {
        self.editor_drafts
            .with(|drafts| drafts.keys().take(24).cloned().collect())
    }

    pub(crate) fn editor_draft_count(self) -> usize {
        self.editor_drafts.with(|drafts| drafts.len())
    }

    pub(crate) fn editor_recovery_json(self) -> String {
        self.editor_drafts
            .with(|drafts| serde_json::to_string_pretty(drafts).unwrap_or_default())
    }

    pub(crate) fn discard_editor_draft(self, id: &str) {
        self.editor_drafts.update(|drafts| {
            drafts.remove(id);
        });
        // Active editors reload only on explicit discard, never on every key.
        self.draft_reset
            .update(|revision| *revision = revision.wrapping_add(1));
    }

    fn hydrate(self) {
        if self.loading.get_untracked() {
            return;
        }
        self.loading.set(true);
        self.status.set(SaveState::Loading);
        wasm_bindgen_futures::spawn_local(async move {
            match relationships::load().await {
                Ok(records) => {
                    self.set_records.set(records);
                    self.loaded.set(true);
                    self.status.set(SaveState::Saved);
                }
                Err(error) => self.status.set(SaveState::Error {
                    message: format!("Relationship data could not load. {error}"),
                    // Retry a load is non-mutating, even for corrupt data.
                    conflict: false,
                }),
            }
            self.loading.set(false);
        });
    }

    pub fn ready(self) -> bool {
        self.loaded.get()
    }

    pub fn editable(self, person_id: &str) -> bool {
        self.loaded.get()
            && !self.exclusive.get()
            && !self.merge_locks.with(|ids| ids.contains(person_id))
            && self.queue.with(|queue| queue.editable(person_id))
    }

    pub fn update(self, person_id: &str, edit: impl FnOnce(&mut RelationshipProfile)) -> bool {
        if !self.loaded.get_untracked()
            || self.exclusive.get_untracked()
            || self
                .merge_locks
                .with_untracked(|ids| ids.contains(person_id))
            || !self.queue.with_untracked(|queue| queue.editable(person_id))
        {
            return false;
        }
        let expected = self.records.with_untracked(|records| {
            records
                .iter()
                .find(|profile| profile.person_id == person_id)
                .cloned()
        });
        let mut draft = expected
            .clone()
            .unwrap_or_else(|| RelationshipProfile::new(person_id, crate::clock::now_ms()));
        edit(&mut draft);
        if draft.person_id != person_id {
            return false;
        }
        if expected.as_ref() == Some(&draft) {
            return true;
        }
        self.queue
            .update(|queue| queue.edit(expected, draft.clone()));
        self.set_records.update(|records| {
            if let Some(current) = records
                .iter_mut()
                .find(|profile| profile.person_id == person_id)
            {
                *current = draft;
            } else {
                records.push(draft);
            }
        });
        self.refresh_status();
        self.save();
        true
    }

    pub fn retry(self) {
        if self.exclusive.get_untracked() {
            return;
        }
        if !self.loaded.get_untracked() {
            self.hydrate();
        } else {
            self.queue.update(Queue::retry);
            self.refresh_status();
            self.save();
        }
    }

    pub fn recovery_json(self) -> String {
        if !self.loaded.get() {
            return "Recovery preview unavailable. The original IndexedDB data was not replaced."
                .into();
        }
        self.queue.with(Queue::recovery_json)
    }

    fn refresh_status(self) {
        self.status.set(self.queue.with_untracked(|queue| {
            if let Some(failure) = queue.failure() {
                SaveState::Error {
                    message: failure.message.clone(),
                    conflict: failure.conflict,
                }
            } else if queue.is_empty() {
                SaveState::Saved
            } else {
                SaveState::Saving
            }
        }));
    }

    fn save(self) {
        if self.saving.get_untracked() {
            return;
        }
        self.saving.set(true);
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                // Render optimistic feedback before copying/encoding a dirty record.
                yield_before_save().await;
                let Some(pending) = self.queue.with_untracked(Queue::next) else {
                    break;
                };
                match relationships::save(pending.expected.as_ref(), &pending.draft).await {
                    Ok(()) => self.queue.update(|queue| queue.saved(pending.draft)),
                    Err(error) => self.queue.update(|queue| {
                        queue.failed(
                            &pending.draft.person_id,
                            Failure {
                                message: error.to_string(),
                                conflict: matches!(error, SaveError::Conflict | SaveError::Corrupt),
                            },
                        )
                    }),
                }
                self.refresh_status();
            }
            self.saving.set(false);
        });
    }
}
