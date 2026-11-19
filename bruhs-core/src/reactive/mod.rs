//! Server-owned reactive automation. Configuration, event identities and work receipts are local.

use bruhs_models::reactive::{ReactiveCommand, ReactiveEvent, ReactiveSnapshot};
use rusqlite::Connection;
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
};
use tokio::sync::{watch, Mutex};

#[cfg(test)]
mod always_on_tests;
mod approval;
#[cfg(test)]
mod automation_tests;
pub mod clock;
#[cfg(test)]
mod computer_tests;
mod computers;
#[cfg(test)]
mod crew_tests;
mod notify;
mod report_sources;
#[cfg(test)]
mod room_tests;
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

pub use approval::BrowserAllowance;
pub use notify::{system_notifier, Notifier};

#[derive(Clone)]
pub struct ReactiveRuntime {
    connection: Arc<Mutex<Connection>>,
    _lock: Arc<store::RuntimeLock>,
    /// The resolved database file's parent. Every per-bruh path
    /// (`bruhs/<id>/work`, `bruhs/<id>/browser`) lives under this one root,
    /// so deleting a bruh has one tree to clean up.
    home: Arc<PathBuf>,
    started: Arc<AtomicBool>,
    changes: Arc<watch::Sender<u64>>,
    notifier: Arc<std::sync::RwLock<Notifier>>,
    /// The host that runs bruh browsers, once attached.
    computers: Arc<OnceLock<Arc<crate::local_tools::LocalToolsHost>>>,
    /// Whether the computer sync loop runs; there is one per runtime.
    syncing: Arc<AtomicBool>,
    /// Browser steps the gate let through, until the command route runs them.
    allowances: Arc<approval::Allowances>,
    /// The folders that hold the owner's saved logins, which no bruh run
    /// may reach (`secrets::private`).
    private_roots: Arc<PrivateRoots>,
}

/// The folders that hold the owner's saved logins, in the two forms the
/// checks need.
#[derive(Debug, Default)]
pub(super) struct PrivateRoots {
    /// Each folder resolved as a file tool's path is (symlinks followed;
    /// for a folder that doesn't exist yet, its nearest existing ancestor),
    /// so the resolved-path check compares like with like.
    resolved: Vec<PathBuf>,
    /// Each resolved folder, and also its path as named when that differs
    /// (a home or data folder reached through a symlink): Claude's deny
    /// rules may match the path a tool names, its sandbox the real one.
    named: Vec<PathBuf>,
}

impl PrivateRoots {
    fn new(roots: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut private = Self::default();
        for root in roots {
            let resolved = approval::resolve(&root, 0).unwrap_or_else(|| root.clone());
            if !private.named.contains(&resolved) {
                private.named.push(resolved.clone());
            }
            if !private.named.contains(&root) {
                private.named.push(root);
            }
            private.resolved.push(resolved);
        }
        private
    }
}

/// A bruh's work folder and the directories this call created for it.
struct NewFolder {
    folder: PathBuf,
    created: Vec<PathBuf>,
    /// The marker written in a tree this call created.
    marker: Option<PathBuf>,
}

impl NewFolder {
    /// Undo a folder made for a bruh that was not saved; never removes contents
    /// other than the marker it wrote.
    fn remove(&self) {
        if let Some(marker) = &self.marker {
            if let Err(error) = std::fs::remove_file(marker) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(%error, "An unused bruh folder's marker could not be removed");
                }
            }
        }
        for directory in self.created.iter().rev() {
            if let Err(error) = std::fs::remove_dir(directory) {
                tracing::warn!(%error, "An unused bruh folder could not be removed");
            }
        }
    }
}

/// Create `<home>/bruhs/<id>/work`, private to the owner on Unix, in a tree
/// marked as Bruhs' own.
fn work_folder(home: &Path, id: &str) -> Result<NewFolder, ReactiveError> {
    let tree = computers::bruh_tree(home, id)?;
    let folder = tree.join("work");
    let created: Vec<PathBuf> = folder
        .ancestors()
        .take_while(|directory| *directory != home && !directory.exists())
        .map(Path::to_path_buf)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    let marker = created
        .contains(&tree)
        .then(|| tree.join(computers::OWNED_MARKER));
    let work = NewFolder {
        folder,
        created,
        marker,
    };
    builder
        .create(&work.folder)
        .map_err(|_| ReactiveError::Storage("Could not create the bruh's work folder.".into()))?;
    if let Err(error) = computers::mark_tree(home, id) {
        work.remove();
        return Err(error);
    }
    Ok(work)
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
        let (connection, lock, home, private_roots) = tokio::task::spawn_blocking(move || {
            let (connection, lock) = store::open(&path)?;
            let home = std::fs::canonicalize(&path)
                .ok()
                .and_then(|path| path.parent().map(Path::to_path_buf))
                .ok_or_else(|| {
                    ReactiveError::Storage("Could not resolve the database folder.".into())
                })?;
            let private_roots = PrivateRoots::new(crate::secrets::private::vault_private_folders());
            Ok::<_, ReactiveError>((connection, lock, home, private_roots))
        })
        .await
        .map_err(|_| ReactiveError::Storage("Database worker stopped during open.".into()))??;
        let (changes, _) = watch::channel(0);
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            _lock: Arc::new(lock),
            home: Arc::new(home),
            started: Arc::new(AtomicBool::new(false)),
            changes: Arc::new(changes),
            notifier: Arc::new(std::sync::RwLock::new(system_notifier())),
            computers: Arc::new(OnceLock::new()),
            syncing: Arc::new(AtomicBool::new(false)),
            allowances: Arc::default(),
            private_roots: Arc::new(private_roots),
        })
    }

    /// The folders that hold the owner's saved logins, resolved: every bruh
    /// run is kept out of them by the resolved-path check.
    pub fn private_roots(&self) -> &[PathBuf] {
        &self.private_roots.resolved
    }

    /// The folders a bruh run's deny rules name: each of
    /// [`Self::private_roots`], and its unresolved path when that differs.
    pub(super) fn private_rule_folders(&self) -> &[PathBuf] {
        &self.private_roots.named
    }

    /// This runtime with `roots` in place of the owner's login folders.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn with_private_roots(mut self, roots: impl IntoIterator<Item = PathBuf>) -> Self {
        self.private_roots = Arc::new(PrivateRoots::new(roots));
        self
    }

    /// Run bruh browsers on `host`: it gets every bruh whose Computer is on,
    /// now and after each change, and closes the idle ones. Once per runtime.
    pub fn attach_computers(
        &self,
        host: Arc<crate::local_tools::LocalToolsHost>,
    ) -> Result<(), ReactiveError> {
        self.computers.set(Arc::clone(&host)).map_err(|_| {
            ReactiveError::Conflict("This reactive runtime already has its computers.".into())
        })?;
        drop(host.start_idle_reaper());
        computers::spawn_sync(self);
        self.changed();
        Ok(())
    }

    /// The host that runs bruh browsers, once attached.
    pub fn computers(&self) -> Option<Arc<crate::local_tools::LocalToolsHost>> {
        self.computers.get().cloned()
    }

    /// Replace how system notifications are shown (tests, or a host with its own).
    pub fn set_notifier(&self, notifier: Notifier) {
        if let Ok(mut current) = self.notifier.write() {
            *current = notifier;
        }
    }

    fn notifier(&self) -> Notifier {
        self.notifier
            .read()
            .map_or_else(|_| system_notifier(), |notifier| notifier.clone())
    }

    /// Unread and open-question counts plus the newest unread update.
    pub async fn outbox_signal(
        &self,
    ) -> Result<bruhs_models::reactive::OutboxSignal, ReactiveError> {
        self.call(|connection| store::signal(connection)).await
    }

    /// Deliver pending system notifications now; returns how many items were sent.
    pub async fn deliver_notifications(&self) -> Result<usize, ReactiveError> {
        notify::deliver_pending(self).await
    }

    /// Queue runs for due follow-ups now, as the worker does every second.
    pub async fn fire_follow_ups(&self) -> Result<usize, ReactiveError> {
        let fired = self.call(store::fire_follow_ups).await?;
        if fired > 0 {
            self.changed();
        }
        Ok(fired)
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
    /// A new bruh without a folder gets a private one under the runtime's home.
    pub async fn command(
        &self,
        mut command: ReactiveCommand,
    ) -> Result<ReactiveSnapshot, ReactiveError> {
        let home = Arc::clone(&self.home);
        let snapshot = self
            .call(move |connection| {
                let created = match &mut command {
                    ReactiveCommand::CreateBruh { agent, .. }
                        if agent.project_path.trim().is_empty() =>
                    {
                        let work = work_folder(&home, &agent.id)?;
                        let Some(folder) = work.folder.to_str() else {
                            work.remove();
                            return Err(ReactiveError::Storage(
                                "The bruh's work folder path is not valid text.".into(),
                            ));
                        };
                        agent.project_path = folder.to_owned();
                        Some(work)
                    }
                    _ => None,
                };
                let result = store::command(connection, command);
                if result.is_err() {
                    created.iter().for_each(NewFolder::remove);
                }
                result
            })
            .await?;
        self.changed();
        Ok(snapshot)
    }

    /// One agent's runs, newest first, a page of `HISTORY_LIMIT` at a time.
    /// `before` is the id of the last run on the previous page.
    pub async fn agent_runs(
        &self,
        agent_id: String,
        before: Option<String>,
    ) -> Result<Vec<bruhs_models::reactive::ReactiveRun>, ReactiveError> {
        self.call(move |connection| store::agent_runs(connection, &agent_id, before.as_deref()))
            .await
    }

    /// One room's messages, newest first, a page of `HISTORY_LIMIT` at a time.
    /// `before` is the id of the last message on the previous page.
    pub async fn room_messages(
        &self,
        room_id: String,
        before: Option<String>,
    ) -> Result<Vec<bruhs_models::crew::room::RoomMessage>, ReactiveError> {
        self.call(move |connection| store::rooms::messages(connection, &room_id, before.as_deref()))
            .await
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
        // Marks older work folders and removes deleted bruhs' leftover trees.
        computers::spawn_sync(self);
        let runtime = self.clone();
        Ok(tokio::spawn(async move {
            let _lease = lease;
            worker::run(runtime, app).await;
        }))
    }
}
