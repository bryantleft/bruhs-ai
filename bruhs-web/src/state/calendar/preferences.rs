//! Scheduling edits publish only after an authoritative IndexedDB commit.

use super::*;
use crate::storage::{
    metadata::SaveError,
    scheduling::{self, Policy},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreferencesError {
    Unavailable,
    Corrupt,
    Changed,
    SaveFailed,
    Busy,
}
impl std::fmt::Display for PreferencesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unavailable=>"Saved scheduling preferences could not be read. Restore browser storage access, then reload saved preferences.",
            Self::Corrupt=>"Saved scheduling preferences could not be decoded. The original data is preserved. Restore a valid copy, then reload saved preferences.",
            Self::Changed=>"Scheduling preferences or calendar data changed in another tab. Reload saved data before repeating this change.",
            Self::SaveFailed=>"The scheduling change could not be saved. Displayed settings are unchanged. Restore browser storage, reload saved preferences, then repeat the change.",
            Self::Busy=>"Scheduling preferences are still loading or saving. Wait for that operation to finish.",
        })
    }
}
impl std::error::Error for PreferencesError {}
fn map_error(error: &SaveError) -> PreferencesError {
    match error {
        SaveError::Conflict => PreferencesError::Changed,
        SaveError::Corrupt => PreferencesError::Corrupt,
        SaveError::Unavailable => PreferencesError::Unavailable,
        _ => PreferencesError::SaveFailed,
    }
}
fn legacy_raw() -> Result<Option<String>, PreferencesError> {
    web_sys::window()
        .ok_or(PreferencesError::Unavailable)?
        .local_storage()
        .map_err(|_| PreferencesError::Unavailable)?
        .ok_or(PreferencesError::Unavailable)?
        .get_item(scheduling::KEY)
        .map_err(|_| PreferencesError::Unavailable)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Operation {
    Idle,
    Loading,
    Saving,
}

#[derive(Clone, Copy)]
pub struct SchedulingPreferencesStore {
    value: RwSignal<SchedulingPreferences>,
    revision: RwSignal<Option<u64>>,
    operation: RwSignal<Operation>,
    error: RwSignal<Option<PreferencesError>>,
    recovery: RwSignal<String>,
}
impl SchedulingPreferencesStore {
    pub(super) fn new() -> Self {
        let store = Self {
            value: RwSignal::new(SchedulingPreferences::default()),
            revision: RwSignal::new(None),
            operation: RwSignal::new(Operation::Idle),
            error: RwSignal::new(None),
            recovery: RwSignal::new(String::new()),
        };
        wasm_bindgen_futures::spawn_local(async move {
            let _ = store.reload().await;
        });
        store
    }
    pub fn ready(self) -> bool {
        self.revision.get().is_some() && self.error.get().is_none()
    }
    pub fn busy(self) -> bool {
        self.operation.get() != Operation::Idle
    }
    pub fn loading(self) -> bool {
        self.operation.get() == Operation::Loading
            || (self.revision.get().is_none() && self.error.get().is_none())
    }
    pub fn error(self) -> Option<PreferencesError> {
        self.error.get()
    }
    pub fn get(self) -> SchedulingPreferences {
        self.value.get()
    }
    pub fn get_untracked(self) -> SchedulingPreferences {
        self.value.get_untracked()
    }
    pub fn read(self) -> impl std::ops::Deref<Target = SchedulingPreferences> {
        self.value.read()
    }
    pub fn with<R>(self, read: impl FnOnce(&SchedulingPreferences) -> R) -> R {
        self.value.with(read)
    }
    pub fn with_untracked<R>(self, read: impl FnOnce(&SchedulingPreferences) -> R) -> R {
        self.value.with_untracked(read)
    }
    pub async fn reload(self) -> Result<(), PreferencesError> {
        if self.operation.get_untracked() != Operation::Idle {
            return Err(PreferencesError::Busy);
        }
        self.operation.set(Operation::Loading);
        match scheduling::load().await {
            Ok(record) => {
                self.committed(record);
                Ok(())
            }
            Err(error) => self.failed(error).await,
        }
    }
    pub(crate) fn begin(self) -> Result<Policy, PreferencesError> {
        if self.operation.get_untracked() != Operation::Idle {
            return Err(PreferencesError::Busy);
        }
        if let Some(error) = self.error.get_untracked() {
            return Err(error);
        }
        let revision = self
            .revision
            .get_untracked()
            .ok_or(PreferencesError::Unavailable)?;
        self.operation.set(Operation::Saving);
        Ok(Policy {
            key: scheduling::KEY.into(),
            revision,
            value: self.value.get_untracked(),
        })
    }
    pub(crate) fn committed(self, record: Policy) {
        self.value.set(record.value);
        self.revision.set(Some(record.revision));
        self.error.set(None);
        self.operation.set(Operation::Idle);
    }
    pub(crate) async fn failed<R>(self, error: SaveError) -> Result<R, PreferencesError> {
        let error = map_error(&error);
        self.error.set(Some(error));
        self.operation.set(Operation::Idle);
        self.capture_recovery().await;
        Err(error)
    }
    pub async fn edit<R>(
        self,
        edit: impl FnOnce(&mut SchedulingPreferences) -> R,
    ) -> Result<R, PreferencesError> {
        let mut record = self.begin()?;
        let result = edit(&mut record.value);
        match scheduling::save(record.revision, record.value).await {
            Ok(record) => {
                self.committed(record);
                Ok(result)
            }
            Err(error) => self.failed(error).await,
        }
    }
    async fn capture_recovery(self) {
        let mut parts = Vec::new();
        match scheduling::inspect().await {
            Ok(Some(raw)) => parts.push(format!(
                "Saved scheduling record (IndexedDB JSON view):\n{raw}"
            )),
            Ok(None) => parts.push("No authoritative scheduling record is present.".into()),
            Err(_) => parts.push(
                "The authoritative scheduling record could not be read. No replacement was saved."
                    .into(),
            ),
        }
        match legacy_raw() {
            Ok(Some(raw)) => parts.push(format!(
                "Legacy recovery copy (localStorage, unchanged by migration):\n{raw}"
            )),
            Ok(None) => {}
            Err(_) => parts.push("The legacy recovery copy could not be read.".into()),
        }
        if self.revision.get_untracked().is_some() {
            if let Ok(json) = serde_json::to_string_pretty(&self.value.get_untracked()) {
                parts.push(format!(
                    "Last displayed settings for reference, not a fresh saved copy:\n{json}"
                ));
            }
        }
        let _ = self.recovery.try_set(parts.join("\n\n"));
    }
    pub fn recovery(self) -> String {
        self.recovery.get()
    }
}
