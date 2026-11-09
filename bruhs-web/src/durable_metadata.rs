//! Optimistic metadata with one ordered writer, explicit errors, and CAS saves.

use leptos::prelude::*;
use serde::{de::DeserializeOwned, Serialize};

use crate::storage::metadata::{self, Metadata, SaveError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveState {
    Loading,
    Saved,
    Saving,
    Error { message: String, conflict: bool },
}

#[derive(Default)]
struct Writer {
    revision: u64,
    generation: u64,
    saved_generation: u64,
    saving: bool,
}

impl Writer {
    fn pending(&self, exclusive: bool) -> bool {
        exclusive || self.saving || self.generation != self.saved_generation
    }
}

pub struct DurableMetadata<T: 'static> {
    key: &'static str,
    value: RwSignal<T>,
    loaded: RwSignal<bool>,
    exclusive: RwSignal<bool>,
    writer: StoredValue<Writer>,
    pub status: RwSignal<SaveState>,
}

impl<T> Copy for DurableMetadata<T> {}
impl<T> Clone for DurableMetadata<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> DurableMetadata<T>
where
    T: Clone + PartialEq + Serialize + DeserializeOwned + Default + Send + Sync + 'static,
{
    pub fn new(key: &'static str) -> Self {
        let store = Self {
            key,
            value: RwSignal::new(T::default()),
            loaded: RwSignal::new(false),
            exclusive: RwSignal::new(false),
            writer: StoredValue::new(Writer::default()),
            status: RwSignal::new(SaveState::Loading),
        };
        store.hydrate();
        store
    }

    fn hydrate(self) {
        self.status.set(SaveState::Loading);
        wasm_bindgen_futures::spawn_local(async move {
            match metadata::load_or_migrate::<T>(self.key).await {
                Ok(record) => {
                    self.writer
                        .update_value(|writer| writer.revision = record.revision);
                    self.value.set(record.value);
                    self.status.set(SaveState::Saved);
                    self.loaded.set(true);
                }
                Err(error) => self.report(error),
            }
        });
    }

    pub fn ready(self) -> bool {
        self.loaded.get()
    }
    pub fn editable(self) -> bool {
        self.loaded.get()
            && !self.exclusive.get()
            && !matches!(self.status.get(), SaveState::Error { conflict: true, .. })
    }
    pub fn settled(self) -> bool {
        self.status.get() == SaveState::Saved && !self.exclusive.get()
    }
    /// Read failures alone do not put local edits at risk. Retain protection
    /// after a failed save and until the latest generation is acknowledged.
    pub(crate) fn pending(self) -> bool {
        // Writer generations publish through status, not the potentially large
        // metadata value. No records are copied to check navigation safety.
        self.status.track();
        let exclusive = self.exclusive.get();
        self.writer.with_value(|writer| writer.pending(exclusive))
    }
    pub fn get_untracked(self) -> T {
        self.value.get_untracked()
    }
    pub fn recovery_json(self) -> String {
        if !self.loaded.get() {
            return crate::state::load_pref(self.key).unwrap_or_else(|| {
                "Recovery preview unavailable. The original IndexedDB data was not replaced.".into()
            });
        }
        serde_json::to_string_pretty(&self.value.get())
            .unwrap_or_else(|_| "Recovery data could not be serialized. Keep this tab open.".into())
    }
    pub fn with<R>(self, f: impl FnOnce(&T) -> R) -> R {
        self.value.with(f)
    }

    pub fn update(self, f: impl FnOnce(&mut T)) {
        let _ = self.try_update(f);
    }
    pub fn try_update<R>(self, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        if !self.loaded.get_untracked()
            || self.exclusive.get_untracked()
            || matches!(
                self.status.get_untracked(),
                SaveState::Error { conflict: true, .. }
            )
        {
            return None;
        }
        let (changed, result) = self
            .value
            .with_untracked(|current| changed_value(current, f));
        let Some(changed) = changed else {
            return Some(result);
        };
        self.value.set(changed);
        self.writer
            .update_value(|writer| writer.generation = writer.generation.wrapping_add(1));
        self.save();
        Some(result)
    }

    pub fn retry(self) {
        if !self.loaded.get_untracked() {
            self.hydrate();
        } else if !matches!(
            self.status.get_untracked(),
            SaveState::Error { conflict: true, .. }
        ) {
            self.save();
        }
    }

    fn save(self) {
        if self.writer.with_value(|writer| writer.saving) {
            return;
        }
        self.writer.update_value(|writer| writer.saving = true);
        self.status.set(SaveState::Saving);
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                // Let optimistic edits and Saving render before cloning/encoding
                // a large history. Keep the writer locked, and snapshot after
                // yielding so edits queued in this interval are included.
                metadata::yield_before_save().await;
                let (revision, generation) = self
                    .writer
                    .with_value(|writer| (writer.revision, writer.generation));
                let value = self.value.get_untracked();
                match metadata::save(self.key, revision, &value).await {
                    Ok(revision) => {
                        self.writer.update_value(|writer| {
                            writer.revision = revision;
                            writer.saved_generation = generation;
                        });
                        if self
                            .writer
                            .with_value(|writer| writer.generation == generation)
                        {
                            break;
                        }
                    }
                    Err(error) => {
                        self.writer.update_value(|writer| writer.saving = false);
                        self.report(error);
                        return;
                    }
                }
            }
            self.writer.update_value(|writer| writer.saving = false);
            self.status.set(SaveState::Saved);
        });
    }

    /// Freeze metadata edits while a transaction checks authoritative grants
    /// and writes its domain record. Only a successful commit publishes state.
    pub fn begin_transaction(self) -> Result<Metadata<T>, SaveError> {
        let clean = self
            .writer
            .with_value(|writer| !writer.saving && writer.generation == writer.saved_generation);
        let usable = matches!(
            self.status.get_untracked(),
            SaveState::Saved
                | SaveState::Error {
                    conflict: false,
                    ..
                }
        );
        if !self.loaded.get_untracked() || !clean || !usable || self.exclusive.get_untracked() {
            return Err(SaveError::Unavailable);
        }
        self.exclusive.set(true);
        self.status.set(SaveState::Saving);
        Ok(Metadata {
            key: self.key.into(),
            revision: self.writer.with_value(|writer| writer.revision),
            value: self.value.get_untracked(),
        })
    }

    pub fn committed(self, record: Metadata<T>) {
        self.writer.update_value(|writer| {
            writer.revision = record.revision;
            writer.saved_generation = writer.generation;
        });
        self.value.set(record.value);
        self.exclusive.set(false);
        self.status.set(SaveState::Saved);
    }

    pub fn report(self, error: SaveError) {
        self.report_error(&error);
    }

    /// A failed multi-metadata transaction releases each participating writer.
    pub fn report_error(self, error: &SaveError) {
        self.exclusive.set(false);
        let conflict = matches!(error, SaveError::Conflict);
        self.status.set(SaveState::Error {
            message: error.to_string(),
            conflict,
        });
    }
}

/// Merely opening a view must not save an identical seed and invalidate another
/// tab's revision. Mutate a candidate so no-op writes also avoid notifications.
fn changed_value<T: Clone + PartialEq, R>(
    current: &T,
    update: impl FnOnce(&mut T) -> R,
) -> (Option<T>, R) {
    let mut next = current.clone();
    let result = update(&mut next);
    ((&next != current).then_some(next), result)
}

#[cfg(test)]
mod tests {
    use super::{changed_value, Writer};

    #[test]
    fn unchanged_writer_has_no_unsaved_edits_even_after_a_read_error() {
        assert!(!Writer::default().pending(false));
    }

    #[test]
    fn failed_save_keeps_unacknowledged_generation_pending() {
        let writer = Writer {
            generation: 1,
            saving: false,
            ..Writer::default()
        };
        assert!(writer.pending(false));
    }

    #[test]
    fn older_acknowledgment_does_not_clear_newer_edits() {
        let writer = Writer {
            generation: 2,
            saved_generation: 1,
            ..Writer::default()
        };
        assert!(writer.pending(false));
    }

    #[test]
    fn latest_acknowledgment_clears_pending_state() {
        let writer = Writer {
            generation: 2,
            saved_generation: 2,
            ..Writer::default()
        };
        assert!(!writer.pending(false));
    }

    #[test]
    fn exclusive_transaction_is_pending_before_optimistic_publication() {
        assert!(Writer::default().pending(true));
    }

    #[test]
    fn running_writer_is_pending_until_it_releases_ownership() {
        let writer = Writer {
            saving: true,
            ..Writer::default()
        };
        assert!(writer.pending(false));
    }
    #[test]
    fn no_op_does_not_enqueue_a_save_or_notify_readers() {
        assert_eq!(changed_value(&vec![1], |_| 7), (None, 7));
    }
    #[test]
    fn changed_candidate_preserves_previous_state_for_failed_save_recovery() {
        let current = vec![1];
        assert_eq!(
            changed_value(&current, |next| next.push(2)).0,
            Some(vec![1, 2])
        );
        assert_eq!(current, vec![1]);
    }
}
