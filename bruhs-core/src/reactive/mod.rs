//! Server-owned reactive automation. Configuration, event identities and work receipts are local.

use bruhs_models::reactive::{ReactiveCommand, ReactiveEvent, ReactiveSnapshot};
use rusqlite::Connection;
use serde_json::Value;
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::sync::{watch, Mutex};

#[cfg(test)]
mod automation_tests;
mod report_sources;
pub mod sources;
mod store;
#[cfg(test)]
mod tests;
mod worker;

#[derive(Debug, thiserror::Error)]
pub enum ReactiveError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("Reactive storage is unavailable: {0}")]
    Storage(String),
    #[error("The event source credential was not accepted.")]
    Unauthorized,
}

impl From<rusqlite::Error> for ReactiveError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Storage(format!(
            "SQLite {:?}; no successful save was acknowledged",
            error.sqlite_error_code()
        ))
    }
}

/// One event emitted by a source scan. All events and its cursor commit together.
#[derive(Debug, Clone)]
pub struct SourceEmission {
    pub key: String,
    pub payload: Value,
}

#[derive(Clone)]
pub struct ReactiveRuntime {
    connection: Arc<Mutex<Connection>>,
    _lock: Arc<store::RuntimeLock>,
    started: Arc<AtomicBool>,
    changes: watch::Sender<u64>,
}

/// Field order closes the SQLite connection before releasing its ownership lock.
/// Both survive cancellation of the async caller until the blocking operation ends.
struct ActiveCall {
    connection: tokio::sync::OwnedMutexGuard<Connection>,
    _owner: Arc<store::RuntimeLock>,
}

impl ReactiveRuntime {
    pub async fn weekly_reports(
        &self,
        before: Option<String>,
    ) -> Result<bruhs_models::weekly_reports::WeeklyReportsSnapshot, ReactiveError> {
        self.call(move |connection| store::reports::snapshot(connection, before.as_deref()))
            .await
    }

    pub async fn weekly_report(
        &self,
        id: String,
    ) -> Result<bruhs_models::weekly_reports::EngineeringReport, ReactiveError> {
        self.call(move |connection| store::reports::read(connection, &id))
            .await
    }

    pub async fn weekly_report_command(
        &self,
        command: bruhs_models::weekly_reports::WeeklyReportCommand,
    ) -> Result<bruhs_models::weekly_reports::WeeklyReportsSnapshot, ReactiveError> {
        let snapshot = self
            .call(move |connection| {
                store::reports::command(connection, command)?;
                store::reports::snapshot(connection, None)
            })
            .await?;
        self.changed();
        Ok(snapshot)
    }

    pub async fn open(path: impl AsRef<Path>) -> Result<Self, ReactiveError> {
        let path = path.as_ref().to_owned();
        let (connection, lock) = tokio::task::spawn_blocking(move || store::open(&path))
            .await
            .map_err(|_| ReactiveError::Storage("Database worker stopped during open.".into()))??;
        let (changes, _) = watch::channel(0);
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            _lock: Arc::new(lock),
            started: Arc::new(AtomicBool::new(false)),
            changes,
        })
    }

    pub fn default_path() -> Result<std::path::PathBuf, ReactiveError> {
        if let Some(path) = std::env::var_os("BRUHS_REACTIVE_DB") {
            if path.is_empty() {
                return Err(ReactiveError::Invalid(
                    "BRUHS_REACTIVE_DB must not be empty.".into(),
                ));
            }
            return Ok(path.into());
        }
        directories::ProjectDirs::from("dev", "bruhs", "bruhs-ai")
            .map(|dirs| {
                dirs.data_local_dir()
                    .join("server")
                    .join("reactive.sqlite3")
            })
            .ok_or_else(|| {
                ReactiveError::Storage("Could not find the private app data directory.".into())
            })
    }

    async fn call<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Connection) -> Result<T, ReactiveError> + Send + 'static,
    ) -> Result<T, ReactiveError> {
        let owner = Arc::clone(&self._lock);
        let mut work = ActiveCall {
            connection: self.connection.clone().lock_owned().await,
            _owner: owner,
        };
        tokio::task::spawn_blocking(move || {
            let result = operation(&mut work.connection);
            drop(work);
            result
        })
        .await
        .map_err(|_| {
            ReactiveError::Storage(
                "Database worker stopped; reload before retrying an uncertain operation.".into(),
            )
        })?
    }

    fn changed(&self) {
        self.changes
            .send_modify(|version| *version = version.wrapping_add(1));
    }

    pub fn watch_changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    pub async fn snapshot(&self) -> Result<ReactiveSnapshot, ReactiveError> {
        self.call(store::snapshot).await
    }

    /// Checkpoint a finalized app record before acknowledging its upstream completion event.
    /// This stores only its identity, independently of the user's source configuration.
    pub async fn record_app_completion(
        &self,
        topic: &str,
        record_id: &str,
    ) -> Result<(), ReactiveError> {
        let (topic, record_id) = (topic.to_owned(), record_id.to_owned());
        self.call(move |connection| store::record_app_completion(connection, &topic, &record_id))
            .await?;
        self.changed();
        Ok(())
    }

    /// Identities with an acknowledged finalization checkpoint, for app-event reconciliation.
    pub async fn app_completions(&self, topic: &str) -> Result<Vec<String>, ReactiveError> {
        let topic = topic.to_owned();
        self.call(move |connection| store::app_completions(connection, &topic))
            .await
    }

    /// Direct user commands apply immediately; agent requests must use `Propose`.
    pub async fn command(
        &self,
        command: ReactiveCommand,
    ) -> Result<ReactiveSnapshot, ReactiveError> {
        let snapshot = self
            .call(move |connection| store::command(connection, command))
            .await?;
        self.changed();
        Ok(snapshot)
    }

    pub async fn publish(
        &self,
        source_id: &str,
        key: &str,
        payload: Value,
    ) -> Result<ReactiveEvent, ReactiveError> {
        let (source_id, key) = (source_id.to_owned(), key.to_owned());
        let event = self
            .call(move |connection| {
                store::transaction(connection, |connection| {
                    store::reports::guard_managed_identity(connection, &source_id)?;
                    store::publish(connection, &source_id, &key, payload)
                })
            })
            .await?;
        self.changed();
        Ok(event)
    }

    pub async fn ingest(
        &self,
        source: &bruhs_models::reactive::EventSource,
        events: Vec<SourceEmission>,
        cursor_key: &str,
        cursor: Value,
    ) -> Result<(), ReactiveError> {
        let (source, cursor_key) = (source.clone(), cursor_key.to_owned());
        self.call(move |connection| {
            store::ingest(connection, &source, events, &cursor_key, cursor)
        })
        .await?;
        self.changed();
        Ok(())
    }

    pub async fn cursor(&self, source_id: &str, key: &str) -> Result<Option<Value>, ReactiveError> {
        let (source_id, key) = (source_id.to_owned(), key.to_owned());
        self.call(move |connection| store::cursor(connection, &source_id, &key))
            .await
    }

    pub async fn source_configs(
        &self,
    ) -> Result<Vec<bruhs_models::reactive::EventSource>, ReactiveError> {
        self.call(|connection| store::active_sources(connection))
            .await
    }

    /// A conservative event batch size that leaves room for every enabled subscriber.
    pub async fn ingestion_capacity(
        &self,
        source: &bruhs_models::reactive::EventSource,
    ) -> Result<usize, ReactiveError> {
        let source = source.clone();
        self.call(move |connection| store::ingestion_capacity(connection, &source))
            .await
    }

    pub async fn source_status(
        &self,
        source_id: &str,
        error: Option<String>,
    ) -> Result<(), ReactiveError> {
        let source_id = source_id.to_owned();
        self.call(move |connection| store::source_status(connection, &source_id, error))
            .await?;
        self.changed();
        Ok(())
    }

    /// Credential disclosure is a separate local-user operation, never part of a snapshot.
    pub async fn webhook_token(&self, source_id: &str) -> Result<String, ReactiveError> {
        let source_id = source_id.to_owned();
        self.call(move |connection| store::webhook_token(connection, &source_id))
            .await
    }

    pub async fn publish_hook(
        &self,
        source_id: &str,
        token: &str,
        key: &str,
        payload: Value,
    ) -> Result<ReactiveEvent, ReactiveError> {
        let (source_id, token, key) = (source_id.to_owned(), token.to_owned(), key.to_owned());
        let event = self
            .call(move |connection| {
                store::transaction(connection, |connection| {
                    let expected = store::webhook_token(connection, &source_id)
                        .map_err(|_| ReactiveError::Unauthorized)?;
                    let mismatch = expected
                        .bytes()
                        .zip(token.bytes())
                        .fold(0_u8, |bits, (left, right)| bits | (left ^ right));
                    if expected.len() != token.len() || mismatch != 0 {
                        return Err(ReactiveError::Unauthorized);
                    }
                    store::publish(connection, &source_id, &key, payload)
                })
            })
            .await?;
        self.changed();
        Ok(event)
    }

    /// Retains every provider stream independently of HTTP or either application window.
    pub fn start(
        &self,
        app: Arc<crate::AppState>,
    ) -> Result<tokio::task::JoinHandle<()>, ReactiveError> {
        self.started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| {
                ReactiveError::Conflict("This reactive runtime is already running.".into())
            })?;
        struct Lease(Arc<AtomicBool>);
        impl Drop for Lease {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        // Created before spawning so cancellation before the task's first poll also releases it.
        let lease = Lease(self.started.clone());
        let runtime = self.clone();
        Ok(tokio::spawn(async move {
            let _lease = lease;
            worker::run(runtime, app).await;
        }))
    }
}
