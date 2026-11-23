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
    time::Duration,
};
use tokio::sync::{watch, Mutex};

#[cfg(test)]
mod always_on_tests;
mod approval;
mod apps;
#[cfg(test)]
mod apps_tests;
#[cfg(test)]
mod automation_tests;
pub mod clock;
#[cfg(test)]
mod computer_tests;
mod computers;
#[cfg(test)]
mod crew_tests;
mod halt;
mod notify;
mod report_sources;
#[cfg(test)]
mod room_tests;
pub mod sources;
mod store;
#[cfg(test)]
mod tests;
mod vault;
#[cfg(test)]
#[cfg(unix)]
mod vault_tests;
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

pub use approval::StepAllowance;
pub use notify::{system_notifier, Notifier};
#[cfg(unix)]
pub(crate) use store::restore::pause_all_for_restore;
pub use vault::VaultRefusal;

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
    /// When each bruh's recent emails went out (final review A2 M7).
    mail_sends: Arc<approval::MailSends>,
    /// The folders that hold the owner's saved logins, which no bruh run
    /// may reach (`secrets::private`).
    private_roots: Arc<PrivateRoots>,
    /// The owner's vault, once attached: bruh logins and keys (`vault`).
    vault: Arc<OnceLock<crate::secrets::SecretManager>>,
    /// The owner's connected apps and mail, once attached: app polls read
    /// through it.
    apps: Arc<OnceLock<crate::apps::AppsHost>>,
    /// The longest any vault lookup of this runtime waits before it fails
    /// closed.
    vault_wait: Duration,
    /// The running worker, which a restore stops (`halt`).
    worker: Arc<halt::WorkerSlot>,
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

pub enum WebhookTarget {
    Source(String),
    Task(String),
}

/// The desktop and the scheduler can live in separate processes. Hold this
/// advisory lock for the lifetime of a provider turn in an ordinary chat.
pub fn desktop_turn_lock(id: &bruhs_models::ids::ThreadId) -> std::io::Result<std::fs::File> {
    claim_desktop_turn(id, None)
}

/// A restarted desktop may only acquire the exact turn left by the previous
/// owner. Scheduled runs use the same lock and invalidate any older claim.
pub fn claim_desktop_turn(
    id: &bruhs_models::ids::ThreadId,
    interrupted: Option<uuid::Uuid>,
) -> std::io::Result<std::fs::File> {
    use sha2::{Digest, Sha256};
    let directory = crate::agent_extensions::directory()?.join("desktop-turns");
    std::fs::create_dir_all(&directory)?;
    let name = format!("{:x}.lock", Sha256::digest(id.as_str().as_bytes()));
    claim_turn_file(&directory.join(name), interrupted)
}

fn claim_turn_file(path: &Path, interrupted: Option<uuid::Uuid>) -> std::io::Result<std::fs::File> {
    use std::io::{Error, ErrorKind, Read};
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.try_lock()?;
    let len = file.metadata()?.len();
    if let Some(expected) = interrupted {
        let mut bytes = [0; 16];
        if len == bytes.len() as u64 {
            file.read_exact(&mut bytes)?;
        }
        if len != bytes.len() as u64 || bytes != *expected.as_bytes() {
            return Err(Error::new(
                ErrorKind::NotFound,
                "The interrupted turn has already ended.",
            ));
        }
    }
    if len > 0 {
        file.set_len(0)?;
        file.sync_data()?;
    }
    Ok(file)
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
            mail_sends: Arc::default(),
            private_roots: Arc::new(private_roots),
            vault: Arc::new(OnceLock::new()),
            apps: Arc::new(OnceLock::new()),
            vault_wait: crate::secrets::VAULT_READ_WAIT,
            worker: Arc::default(),
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

    /// Read bruh logins and keys from `vault`: the browser gate and the
    /// command route use it, and the sync pass deletes a deleted bruh's own
    /// entries. Once per runtime.
    pub fn attach_vault(&self, vault: crate::secrets::SecretManager) -> Result<(), ReactiveError> {
        self.vault.set(vault).map_err(|_| {
            ReactiveError::Conflict("This reactive runtime already has its vault.".into())
        })?;
        computers::spawn_sync(self);
        self.changed();
        Ok(())
    }

    /// The owner's vault, once attached.
    pub fn vault(&self) -> Option<&crate::secrets::SecretManager> {
        self.vault.get()
    }

    /// Poll app sources through `apps` (D20). Until this is attached, an
    /// app poll's status says apps aren't on this server. Once per runtime.
    pub fn attach_apps(&self, apps: crate::apps::AppsHost) -> Result<(), ReactiveError> {
        self.apps.set(apps).map_err(|_| {
            ReactiveError::Conflict("This reactive runtime already has its apps.".into())
        })?;
        self.changed();
        Ok(())
    }

    /// The owner's apps and mail, once attached.
    pub fn apps(&self) -> Option<&crate::apps::AppsHost> {
        self.apps.get()
    }

    /// This runtime with its vault lookups bounded by `wait` instead of
    /// [`crate::secrets::VAULT_READ_WAIT`]. The attached manager bounds each
    /// value read by its own wait (`SecretManager::with_read_wait`), which
    /// records the entry's attention.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn with_vault_wait(mut self, wait: Duration) -> Self {
        self.vault_wait = wait;
        self
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

    /// Check an offline local task database before forgetting its desktop conversations.
    pub fn has_scheduled_desktop_threads(
        ids: &[bruhs_models::ids::ThreadId],
    ) -> Result<bool, ReactiveError> {
        let path = Self::default_path()?;
        if !path
            .try_exists()
            .map_err(|e| ReactiveError::Storage(e.to_string()))?
        {
            return Ok(false);
        }
        let db = rusqlite::Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        if !db.table_exists(None, "desktop_task_threads")? {
            return Ok(false);
        }
        let mut query =
            db.prepare("SELECT EXISTS(SELECT 1 FROM desktop_task_threads WHERE thread_id=?1)")?;
        for id in ids {
            if query.query_row([id.as_str()], |row| row.get::<_, bool>(0))? {
                return Ok(true);
            }
        }
        Ok(false)
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

    pub async fn linked_schedule_inventory(
        &self,
        links: bool,
    ) -> Result<Vec<bruhs_models::reactive::ScheduledTaskSummary>, ReactiveError> {
        self.call(move |db| store::linked_threads::inventory(db, links))
            .await
    }

    pub async fn linked_thread(
        &self,
        task: String,
        thread: bruhs_models::ids::ThreadId,
        before: Option<String>,
    ) -> Result<bruhs_models::reactive::LinkedThreadPage, ReactiveError> {
        self.call(move |db| store::linked_threads::read(db, &task, &thread, before.as_deref()))
            .await
    }

    pub async fn reply_to_linked_thread(
        &self,
        task: String,
        thread: bruhs_models::ids::ThreadId,
        reply: bruhs_models::reactive::LinkedThreadReply,
    ) -> Result<(), ReactiveError> {
        self.call(move |db| store::linked_threads::send(db, &task, &thread, reply))
            .await?;
        self.changed();
        Ok(())
    }

    pub async fn scheduled_task_inventory(
        &self,
    ) -> Result<Vec<bruhs_models::reactive::ScheduledTaskSummary>, ReactiveError> {
        self.call(store::scheduled_task_inventory).await
    }

    pub async fn linked_task_definition(
        &self,
        id: String,
    ) -> Result<bruhs_models::reactive::LinkedTaskDefinition, ReactiveError> {
        self.call(move |db| store::linked_tasks::definition(db, &id))
            .await
    }

    pub async fn save_linked_task(
        &self,
        id: String,
        edit: bruhs_models::reactive::LinkedTaskSave,
        paths: Vec<String>,
    ) -> Result<bruhs_models::reactive::ScheduledTaskSummary, ReactiveError> {
        let result = self
            .call(move |db| store::linked_tasks::save_task(db, &id, edit, &paths))
            .await?;
        self.changed();
        Ok(result)
    }

    pub async fn set_linked_schedule_enabled(
        &self,
        id: String,
        enabled: bool,
    ) -> Result<bruhs_models::reactive::ScheduledTaskSummary, ReactiveError> {
        let task = self
            .call(move |db| store::set_linked_schedule_enabled(db, &id, enabled))
            .await?;
        self.changed();
        Ok(task)
    }

    pub async fn delete_linked_schedule(&self, id: String) -> Result<(), ReactiveError> {
        self.call(move |db| store::delete_linked_schedule(db, id))
            .await?;
        self.changed();
        Ok(())
    }

    pub async fn run_linked_schedule(
        &self,
        id: String,
        request: uuid::Uuid,
    ) -> Result<(), ReactiveError> {
        self.call(move |db| {
            store::transaction(db, |db| {
                store::reports::guard_managed_identity(db, &id)?;
                store::run_scheduled_task(db, &id, &request.to_string())
            })
        })
        .await?;
        self.changed();
        Ok(())
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
        self.agent_thread_runs(agent_id, before, None).await
    }

    /// Whether an enabled task targets this conversation or runs in its scratch folder.
    pub(crate) async fn uses_scratch(
        &self,
        folder: std::path::PathBuf,
        members: Vec<String>,
    ) -> Result<bool, ReactiveError> {
        self.call(move |db| {
            let mut query = db.prepare(
                "SELECT d.thread_id,a.thread_id,json_extract(a.payload,'$.project_path'),
                        json_extract(s.payload,'$.task_checkout.path')
                 FROM reactive_subscriptions s JOIN reactive_agents a
                   ON a.id=json_extract(s.payload,'$.agent_id')
                 LEFT JOIN task_destinations d ON d.subscription_id=s.id
                 WHERE json_extract(s.payload,'$.enabled')=1",
            )?;
            let mut rows = query.query([])?;
            while let Some(row) = rows.next()? {
                for col in [0, 1] {
                    if row
                        .get::<_, Option<String>>(col)?
                        .is_some_and(|id| members.contains(&id))
                    {
                        return Ok(true);
                    }
                }
                for col in [2, 3] {
                    if row.get::<_, Option<String>>(col)?.is_some_and(|p| {
                        !p.is_empty()
                            && crate::worktrees::resolve(std::path::Path::new(&p))
                                .starts_with(&folder)
                    }) {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        })
        .await
    }

    pub async fn task_threads(
        &self,
        project: String,
        query: String,
    ) -> Result<Vec<bruhs_models::reactive::TaskThreadChoice>, ReactiveError> {
        self.call(move |db| store::task_conversations::search(db, &project, &query))
            .await
    }

    pub async fn project_conversations(
        &self,
        project: String,
        query: String,
    ) -> Result<Vec<bruhs_models::reactive::TaskThreadChoice>, ReactiveError> {
        let mut scheduled = self.task_threads(project.clone(), query.clone()).await?;
        let desktop =
            tokio::task::spawn_blocking(move || store::desktop_tasks::choices(&project, &query))
                .await
                .map_err(|_| ReactiveError::Storage("Desktop thread search stopped.".into()))??;
        scheduled.retain(|c| c.desktop_model.is_none());
        scheduled.extend(desktop);
        Ok(scheduled)
    }

    pub async fn desktop_task_turns(
        &self,
        thread: bruhs_models::ids::ThreadId,
        after: i64,
    ) -> Result<Vec<bruhs_models::reactive::DesktopTaskTurn>, ReactiveError> {
        self.call(move |db| store::desktop_tasks::turns(db, &thread, after))
            .await
    }

    pub async fn agent_thread_runs(
        &self,
        agent_id: String,
        before: Option<String>,
        thread: Option<bruhs_models::ids::ThreadId>,
    ) -> Result<Vec<bruhs_models::reactive::ReactiveRun>, ReactiveError> {
        self.call(move |connection| {
            store::agent_runs(
                connection,
                &agent_id,
                before.as_deref(),
                thread.as_ref().map(|id| id.as_str()),
            )
        })
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

    /// [`Self::ingestion_capacity`], or `None` when no enabled subscriber
    /// would make a run of an event.
    pub async fn ingestion_room(
        &self,
        source: &bruhs_models::reactive::EventSource,
    ) -> Result<Option<usize>, ReactiveError> {
        let source = source.clone();
        self.call(move |connection| store::ingestion_room(connection, &source))
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

    pub async fn webhook_deliveries(
        &self,
        task: String,
    ) -> Result<Vec<bruhs_models::reactive::WebhookDeliverySummary>, ReactiveError> {
        self.call(move |db| store::webhook_deliveries::list(db, &task))
            .await
    }

    pub async fn webhook_delivery(
        &self,
        task: String,
        delivery: String,
    ) -> Result<bruhs_models::reactive::WebhookDelivery, ReactiveError> {
        self.call(move |db| store::webhook_deliveries::detail(db, &task, &delivery))
            .await
    }

    /// Credential disclosure is a separate local-user operation, never part of a snapshot.
    pub async fn webhook_token(&self, source_id: &str) -> Result<String, ReactiveError> {
        let source_id = source_id.to_owned();
        self.call(move |connection| store::webhook_token(connection, &source_id))
            .await
    }

    pub async fn task_webhook_token(
        &self,
        task: String,
        previous: Option<String>,
    ) -> Result<String, ReactiveError> {
        self.call(move |db| {
            store::transaction(db, |db| {
                store::task_webhooks::address(db, &task, previous.as_deref())
            })
        })
        .await
    }

    pub async fn publish_hook(
        &self,
        source_id: &str,
        token: &str,
        key: &str,
        payload: Value,
    ) -> Result<ReactiveEvent, ReactiveError> {
        self.publish_hook_request(source_id, token, key, payload, Default::default())
            .await
    }

    pub async fn publish_hook_request(
        &self,
        source_id: &str,
        token: &str,
        key: &str,
        payload: Value,
        request: bruhs_models::reactive::WebhookMetadata,
    ) -> Result<ReactiveEvent, ReactiveError> {
        self.publish_hook_received(
            source_id,
            token,
            key,
            payload,
            request,
            bruhs_models::EpochMs::now(),
        )
        .await
    }

    /// The trusted ingress timestamps the arrival before reading the body or waiting on storage.
    pub async fn publish_hook_received(
        &self,
        source_id: &str,
        token: &str,
        key: &str,
        payload: Value,
        request: bruhs_models::reactive::WebhookMetadata,
        received: bruhs_models::EpochMs,
    ) -> Result<ReactiveEvent, ReactiveError> {
        self.receive_webhook(source_id, token, key, Some(payload), request, received)
            .await
    }

    /// HTTP ingress supplies the raw body. Legacy callers can supply a parsed event.
    pub async fn receive_webhook(
        &self,
        source_id: &str,
        token: &str,
        key: &str,
        supplied: Option<Value>,
        request: bruhs_models::reactive::WebhookMetadata,
        received: bruhs_models::EpochMs,
    ) -> Result<ReactiveEvent, ReactiveError> {
        self.receive_addressed_webhook(
            WebhookTarget::Source(source_id.to_owned()),
            token,
            key,
            supplied,
            request,
            received,
        )
        .await
    }

    pub async fn receive_addressed_webhook(
        &self,
        target: WebhookTarget,
        token: &str,
        key: &str,
        supplied: Option<Value>,
        mut request: bruhs_models::reactive::WebhookMetadata,
        received: bruhs_models::EpochMs,
    ) -> Result<ReactiveEvent, ReactiveError> {
        use bruhs_models::reactive::{WebhookOutcome, MAX_PAYLOAD_BYTES};
        let (token, key) = (token.to_owned(), key.to_owned());
        let (event, rejected_signature) = self
            .call(move |connection| {
                let mut receipt = None;
                let mut rejected_source = None;
                let mut denials = store::webhook_security::Denials::new();
                let result = store::transaction(connection, |connection| {
                    let (source_id, expected, task, scope) = match &target {
                        WebhookTarget::Source(id) => (
                            id.clone(),
                            store::webhook_token(connection, id)
                                .map_err(|_| ReactiveError::Unauthorized)?,
                            None,
                            None,
                        ),
                        WebhookTarget::Task(id) => {
                            let (task, expected, scope) =
                                store::task_webhooks::lookup(connection, id)?;
                            (task.source_id.clone(), expected, Some(task), Some(scope))
                        }
                    };
                    let mismatch = expected
                        .bytes()
                        .zip(token.bytes())
                        .fold(0_u8, |bits, (left, right)| bits | (left ^ right));
                    if expected.len() != token.len() || mismatch != 0 {
                        return Err(ReactiveError::Unauthorized);
                    }
                    if request
                        .raw_body
                        .as_ref()
                        .is_some_and(|body| body.len() > MAX_PAYLOAD_BYTES)
                    {
                        return Err(ReactiveError::Invalid(
                            "Webhook body exceeds the 64 KiB source budget.".into(),
                        ));
                    }
                    let payload = match (request.payload(), supplied) {
                        (Some(actual), Some(expected)) if actual != expected => {
                            return Err(ReactiveError::Invalid(
                                "Webhook body differs from its supplied payload.".into(),
                            ))
                        }
                        (Some(actual), _) => actual,
                        (None, Some(expected)) => expected,
                        (None, None) => {
                            return Err(ReactiveError::Invalid(
                                "Webhook request body is missing.".into(),
                            ))
                        }
                    };
                    let tasks = match task {
                        Some(task) => vec![task],
                        None => store::source_subscriptions(connection, &source_id)?,
                    };
                    rejected_source = Some(source_id.clone());
                    let now = bruhs_models::EpochMs::now();
                    let received = received.min(now);
                    denials = store::webhook_security::inspect(
                        connection,
                        &tasks,
                        &mut request,
                        received,
                        now,
                    )?;
                    let rejected_signature = !tasks.is_empty()
                        && tasks.iter().all(|task| {
                            denials.get(&task.id) == Some(&WebhookOutcome::RejectedSignature)
                        });
                    let verified = tasks
                        .iter()
                        .filter(|task| {
                            task.enabled
                                && task.webhook_security.signature.is_some()
                                && denials.get(&task.id) != Some(&WebhookOutcome::RejectedSignature)
                        })
                        .map(|task| task.id.clone())
                        .collect();
                    receipt = Some(store::webhook_deliveries::Receipt::new(
                        &request, &payload, &token, received, verified,
                    )?);
                    let event = store::publish_with_metadata(
                        connection,
                        &source_id,
                        &key,
                        payload,
                        &request,
                        receipt.as_mut(),
                        store::TaskAdmission {
                            subscriptions: tasks,
                            denied: &denials,
                            scope: scope.as_deref(),
                        },
                    )?;
                    Ok((event, rejected_signature))
                });
                if let (
                    Err(error @ (ReactiveError::Conflict(_) | ReactiveError::Invalid(_))),
                    Some(receipt),
                    Some(source_id),
                ) = (&result, receipt.as_mut(), rejected_source.as_deref())
                {
                    // The failed dispatch rolled back. Only authenticated requests reach these errors.
                    store::transaction(connection, |db| {
                        store::webhook_deliveries::record_rejected(
                            db,
                            source_id,
                            match &target {
                                WebhookTarget::Task(id) => Some(id.as_str()),
                                WebhookTarget::Source(_) => None,
                            },
                            receipt,
                            error,
                            &denials,
                        )
                    })?;
                }
                result
            })
            .await?;
        self.changed();
        if rejected_signature {
            return Err(ReactiveError::Unauthorized);
        }
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
        let handle = tokio::spawn(async move {
            let _lease = lease;
            worker::run(runtime, app).await;
        });
        self.worker.keep(&handle);
        Ok(handle)
    }
}
