//! Transactional on-device meeting/contact storage. Only the blocking worker touches SQLite.

use super::{MeetingStoreError, MeetingStoreResult};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::Mutex;

mod adapter;
mod contact_sync;
mod contacts;
mod generation;
#[cfg(test)]
mod integrity_tests;
mod link_jobs;
mod local_focus;
#[cfg(test)]
mod performance_tests;
mod records;
mod schema;
#[cfg(test)]
mod speaker_linking_tests;
mod summaries;
#[cfg(test)]
mod tests;

pub struct SqliteMeetingStore {
    connection: Arc<Mutex<Connection>>,
}

impl SqliteMeetingStore {
    pub async fn open(path: impl AsRef<Path>) -> MeetingStoreResult<Self> {
        let path = path.as_ref().to_path_buf();
        let connection = tokio::task::spawn_blocking(move || schema::open(&path))
            .await
            .map_err(|_| {
                MeetingStoreError::PersistenceFailed("Database worker stopped during open.".into())
            })??;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub fn default_path() -> MeetingStoreResult<PathBuf> {
        if let Some(path) = std::env::var_os("BRUHS_MEETING_DB") {
            if path.is_empty() {
                return Err(MeetingStoreError::InvalidInput(
                    "BRUHS_MEETING_DB must not be empty.".into(),
                ));
            }
            return Ok(PathBuf::from(path));
        }
        directories::ProjectDirs::from("dev", "bruhs", "bruhs-ai")
            .map(|dirs| {
                dirs.data_local_dir()
                    .join("server")
                    .join("meetings.sqlite3")
            })
            .ok_or_else(|| {
                MeetingStoreError::PersistenceFailed(
                    "Could not resolve the private app data directory.".into(),
                )
            })
    }

    async fn call<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Connection) -> MeetingStoreResult<T> + Send + 'static,
    ) -> MeetingStoreResult<T> {
        // Acquire asynchronously before spawning: a backlog never occupies blocked worker threads.
        let mut connection = self.connection.clone().lock_owned().await;
        tokio::task::spawn_blocking(move || operation(&mut connection))
            .await
            .map_err(|_| {
                MeetingStoreError::PersistenceFailed(
                    "Database worker stopped; reload before retrying an uncertain operation."
                        .into(),
                )
            })?
    }
}

fn write<T>(
    connection: &mut Connection,
    operation: impl FnOnce(&Connection) -> MeetingStoreResult<T>,
) -> MeetingStoreResult<T> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let result = operation(&transaction)?;
    transaction.commit()?;
    Ok(result)
}

fn encode<T: Serialize>(value: &T) -> MeetingStoreResult<String> {
    serde_json::to_string(value).map_err(|_| {
        MeetingStoreError::InvalidInput("Record could not be encoded; no change was saved.".into())
    })
}
fn decode<T: DeserializeOwned>(value: String) -> MeetingStoreResult<T> {
    serde_json::from_str(&value).map_err(MeetingStoreError::Corrupt)
}
fn get<T: DeserializeOwned>(
    connection: &Connection,
    table: &'static str,
    id: &str,
) -> MeetingStoreResult<Option<T>> {
    let value: Option<String> = connection
        .query_row(
            &format!("SELECT payload FROM {table} WHERE id=?1"),
            [id],
            |row| row.get(0),
        )
        .optional()?;
    value.map(decode).transpose()
}
fn all<T: DeserializeOwned>(
    connection: &Connection,
    sql: &str,
    valid: impl Fn(&str, &T) -> bool,
) -> MeetingStoreResult<Vec<T>> {
    let mut statement = connection.prepare(sql)?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    rows.map(|row| {
        let (id, payload) = row?;
        let value = decode(payload)?;
        if !valid(&id, &value) {
            return Err(MeetingStoreError::PersistenceFailed(
                "Saved record is malformed or has a mismatched identity. Original data was preserved.".into(),
            ));
        }
        Ok(value)
    }).collect()
}

impl From<rusqlite::Error> for MeetingStoreError {
    fn from(error: rusqlite::Error) -> Self {
        // The message shows only the code; SQL text and private record
        // contents stay in the source, out of HTTP/log messages.
        Self::Database {
            code: error.sqlite_error_code(),
            source: error,
        }
    }
}
