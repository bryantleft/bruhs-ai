//! Revision-checked metadata in the same database as workflow-created records.

use idb::{Transaction, TransactionMode};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use wasm_bindgen::JsValue;

use super::{open_db, STORE_METADATA};

mod codec;
mod json_safe;
mod save_scheduler;
pub(super) use codec::{encode, json_string};
pub use save_scheduler::yield_before_save;

#[derive(Debug)]
pub enum SaveError {
    Browser(idb::Error),
    Aborted,
    Conflict,
    Corrupt,
    Unavailable,
    Rejected(&'static str),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Browser(_) | Self::Aborted => f.write_str("Browser storage could not save this change. Keep this tab open and retry."),
            Self::Conflict => f.write_str("Another tab saved newer data. Your unsaved changes are kept here; copy them before reloading to review the latest version."),
            Self::Corrupt => f.write_str("Saved data could not be decoded. The original copy was preserved; no replacement was saved."),
            Self::Unavailable => f.write_str("Local storage is not ready. No records were changed."),
            Self::Rejected(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for SaveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Browser(error) => Some(error),
            _ => None,
        }
    }
}
impl From<idb::Error> for SaveError {
    fn from(error: idb::Error) -> Self {
        Self::Browser(error)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Metadata<T> {
    pub key: String,
    pub revision: u64,
    pub value: T,
}

/// Listen for terminal completion before issuing requests. The idb 0.6 future
/// resolves on bubbling request errors and leaves once-only callbacks attached;
/// multiple abort errors can then invoke an already-dropped WASM closure.
/// Native Promise callbacks remain valid until we detach them on drop.
pub struct LocalTransaction {
    inner: Transaction,
    raw: web_sys::IdbTransaction,
    completion: wasm_bindgen_futures::JsFuture,
}

impl LocalTransaction {
    pub fn new(tx: Transaction) -> Self {
        let raw: web_sys::IdbTransaction = tx.into();
        let promise = js_sys::Promise::new(&mut |resolve, reject| {
            raw.set_oncomplete(Some(&resolve));
            raw.set_onabort(Some(&reject));
        });
        Self {
            inner: raw.clone().into(),
            raw,
            // Attach rejection handling immediately, not after request awaits.
            completion: wasm_bindgen_futures::JsFuture::from(promise),
        }
    }
}

impl std::ops::Deref for LocalTransaction {
    type Target = Transaction;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl Drop for LocalTransaction {
    fn drop(&mut self) {
        self.raw.set_oncomplete(None);
        self.raw.set_onabort(None);
    }
}

/// Request success is not commit success. Abort all earlier writes on any
/// operation error, and never publish a checkpoint until the terminal event.
pub async fn finish<T>(
    mut tx: LocalTransaction,
    result: Result<T, SaveError>,
) -> Result<T, SaveError> {
    match result {
        Ok(value) => {
            (&mut tx.completion).await.map_err(|_| SaveError::Aborted)?;
            Ok(value)
        }
        Err(error) => {
            let _ = tx.raw.abort();
            let _ = (&mut tx.completion).await;
            Err(error)
        }
    }
}

pub async fn read_in<T: DeserializeOwned>(
    tx: &Transaction,
    key: &str,
) -> Result<Metadata<T>, SaveError> {
    let raw = tx
        .object_store(STORE_METADATA)?
        .get(JsValue::from_str(key))?
        .await?
        .ok_or(SaveError::Unavailable)?;
    codec::decode(raw).map_err(|_| SaveError::Corrupt)
}

pub async fn load_or_migrate<T: Serialize + DeserializeOwned + Default>(
    key: &str,
) -> Result<Metadata<T>, SaveError> {
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(&[STORE_METADATA], TransactionMode::ReadWrite)?);
    let result = async {
        let store = tx.object_store(STORE_METADATA)?;
        if let Some(raw) = store.get(JsValue::from_str(key))?.await? {
            return codec::decode(raw).map_err(|_| SaveError::Corrupt);
        }
        // Read the legacy copy only if IndexedDB has no authoritative version.
        // Preserve it as a recovery copy, including when migration fails.
        let legacy = web_sys::window()
            .ok_or(SaveError::Unavailable)?
            .local_storage()
            .map_err(|_| SaveError::Unavailable)?
            .ok_or(SaveError::Unavailable)?
            .get_item(key)
            .map_err(|_| SaveError::Unavailable)?;
        let value = match legacy {
            Some(json) => serde_json::from_str(&json).map_err(|_| SaveError::Corrupt)?,
            None => T::default(),
        };
        let record = Metadata {
            key: key.to_string(),
            revision: 1,
            value,
        };
        store.add(&encode(&record)?, None)?.await?;
        Ok(record)
    }
    .await;
    finish(tx, result).await
}

pub fn next_revision(current: u64, expected: u64) -> Result<u64, SaveError> {
    if current != expected {
        return Err(SaveError::Conflict);
    }
    current.checked_add(1).ok_or(SaveError::Conflict)
}

pub async fn save<T: Serialize + DeserializeOwned>(
    key: &str,
    expected: u64,
    value: &T,
) -> Result<u64, SaveError> {
    let revision = next_revision(expected, expected)?;
    let encoded = encode(&Metadata {
        key: key.to_string(),
        revision,
        value,
    })?;
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(&[STORE_METADATA], TransactionMode::ReadWrite)?);
    let result = async {
        let current: Metadata<T> = read_in(&tx, key).await?;
        next_revision(current.revision, expected)?;
        tx.object_store(STORE_METADATA)?
            .put(&encoded, None)?
            .await?;
        Ok(revision)
    }
    .await;
    finish(tx, result).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_revision_is_rejected_without_overwriting() {
        assert!(matches!(next_revision(7, 6), Err(SaveError::Conflict)));
    }
    #[test]
    fn revision_exhaustion_does_not_wrap() {
        assert!(matches!(
            next_revision(u64::MAX, u64::MAX),
            Err(SaveError::Conflict)
        ));
    }
    #[test]
    fn matching_revision_advances_once() {
        assert_eq!(next_revision(6, 6).unwrap(), 7);
    }
}
