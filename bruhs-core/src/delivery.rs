//! Local delivery host: review frozen commands, execute once, retain receipts and PR events.

use bruhs_models::delivery::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{watch, Mutex};

mod branches;
mod command;
mod credits;
#[cfg(test)]
mod fingerprint_tests;
mod forge;
mod github;
mod gitlab;
mod native_stack;
mod plan;
mod preferences;
#[cfg(all(test, unix))]
mod repository_tests;
mod review_guide;
mod stack;
mod storage;
#[cfg(test)]
mod task_source_tests;
#[cfg(test)]
mod tests;
mod trackers;
mod verification;
mod watching;
pub mod writer;

#[derive(Debug, thiserror::Error)]
pub enum DeliveryError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Command(String),
    #[error("Delivery storage: {0}")]
    Storage(String),
}

pub type DeliveryResult<T> = Result<T, DeliveryError>;

struct DeliveryOwner(std::fs::File);
impl Drop for DeliveryOwner {
    fn drop(&mut self) {
        // A spawned CLI can briefly inherit the CLOEXEC descriptor before exec.
        // Explicit unlock ends ownership immediately when the last service exits.
        if let Err(error) = self.0.unlock() {
            tracing::warn!(%error, "Could not explicitly release the delivery owner lock");
        }
    }
}

#[derive(Clone)]
pub struct DeliveryService {
    root: Arc<PathBuf>,
    state: Arc<Mutex<Journal>>,
    guide_locks: Arc<Mutex<HashMap<String, std::sync::Weak<Mutex<()>>>>>,
    cancellations: Arc<Mutex<HashMap<String, watch::Sender<bool>>>>,
    changed: Arc<tokio::sync::Notify>,
    watching: Arc<std::sync::atomic::AtomicBool>,
    _lock: Arc<DeliveryOwner>,
    /// Injectable executable paths support offline tests without replacing the user's PATH.
    gh: Arc<PathBuf>,
    git: Arc<PathBuf>,
    glab: Arc<PathBuf>,
    tracker_client: reqwest::Client,
    tracker_secrets: crate::secrets::SecretStore,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    preferences: std::collections::BTreeMap<String, DeliveryOverrides>,
    actions: Vec<ActionReceipt>,
    watches: Vec<PullWatch>,
    events: Vec<WatchEvent>,
}

impl Default for Journal {
    fn default() -> Self {
        Self {
            version: 1,
            preferences: Default::default(),
            actions: Vec::new(),
            watches: Vec::new(),
            events: Vec::new(),
        }
    }
}

impl DeliveryService {
    pub fn default_path() -> DeliveryResult<PathBuf> {
        if let Some(directory) = std::env::var_os("BRUHS_DELIVERY_DIR") {
            return Ok(PathBuf::from(directory));
        }
        directories::BaseDirs::new()
            .map(|dirs| dirs.data_local_dir().join("bruhs-ai").join("delivery"))
            .ok_or_else(|| DeliveryError::Storage("The app data directory is unavailable.".into()))
    }

    pub async fn open(root: impl AsRef<Path>) -> DeliveryResult<Self> {
        let root = root.as_ref().to_owned();
        let directory = root.clone();
        let (journal, lock) = tokio::task::spawn_blocking(move || storage::open(&directory))
            .await
            .map_err(|error| DeliveryError::Storage(error.to_string()))??;
        let service = Self {
            root: Arc::new(root),
            state: Arc::new(Mutex::new(journal)),
            guide_locks: Default::default(),
            cancellations: Arc::new(Mutex::new(HashMap::new())),
            _lock: Arc::new(DeliveryOwner(lock)),
            changed: Arc::new(tokio::sync::Notify::new()),
            watching: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            gh: Arc::new(PathBuf::from("gh")),
            git: Arc::new(PathBuf::from("git")),
            glab: Arc::new(PathBuf::from("glab")),
            tracker_client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(crate::capture_client::CONNECT_TIMEOUT)
                .timeout(crate::capture_client::REQUEST_TIMEOUT)
                .build()
                .map_err(|error| DeliveryError::Command(error.to_string()))?,
            tracker_secrets: crate::secrets::SecretStore::os(),
        };
        service.mutate(|state| {
            for receipt in &mut state.actions {
                if receipt.status == ActionStatus::Running {
                    receipt.status = ActionStatus::Interrupted;
                    receipt.error = Some("The delivery host stopped. Inspect the repository and completed steps before preparing a new action; the operation was not replayed.".into());
                    if let Some(operation) = &mut receipt.native_operation {
                        if operation.state == NativeMergeState::Submitting {
                            operation.state = NativeMergeState::Unknown;
                            operation.message = "The host stopped while submitting. GitHub may have accepted it; inspect the server. This operation will not be replayed.".into();
                        }
                    }
                }
            }
            Ok(())
        }).await?;
        Ok(service)
    }

    async fn mutate<T>(
        &self,
        change: impl FnOnce(&mut Journal) -> DeliveryResult<T>,
    ) -> DeliveryResult<T> {
        let mut current = self.state.lock().await;
        let mut next = current.clone();
        let result = change(&mut next)?;
        // Same retained-finished-work bound as the existing background task registry.
        // Pending actions and unread events are never evicted.
        let terminal = |status| {
            matches!(
                status,
                ActionStatus::Completed
                    | ActionStatus::Failed
                    | ActionStatus::Interrupted
                    | ActionStatus::Cancelled
            )
        };
        let mut excess = next
            .actions
            .iter()
            .filter(|action| terminal(action.status))
            .count()
            .saturating_sub(crate::registry::MAX_FINISHED_TASKS);
        next.actions.retain(|action| {
            if excess > 0 && terminal(action.status) {
                excess -= 1;
                false
            } else {
                true
            }
        });
        let mut excess_events = next
            .events
            .iter()
            .filter(|event| event.acknowledged && event.delivered)
            .count()
            .saturating_sub(crate::registry::MAX_FINISHED_TASKS);
        next.events.retain(|event| {
            if excess_events > 0 && event.acknowledged && event.delivered {
                excess_events -= 1;
                false
            } else {
                true
            }
        });
        let directory = self.root.clone();
        let persisted = next.clone();
        tokio::task::spawn_blocking(move || storage::save(&directory, &persisted))
            .await
            .map_err(|error| DeliveryError::Storage(error.to_string()))??;
        *current = next;
        drop(current);
        self.changed.notify_one();
        Ok(result)
    }

    pub async fn snapshot(&self) -> DeliverySnapshot {
        let state = self.state.lock().await;
        let snapshot = DeliverySnapshot {
            actions: state.actions.iter().rev().cloned().collect(),
            watches: state.watches.clone(),
            events: state.events.iter().rev().cloned().collect(),
        };
        drop(state);
        snapshot
    }

    pub async fn prepare(&self, request: PrepareRequest) -> DeliveryResult<PreparedAction> {
        self.prepare_with_source(request, None).await
    }

    pub async fn task_context(
        &self,
        task: &bruhs_models::coordination::CoordinatedTask,
    ) -> DeliveryResult<TaskDeliveryContext> {
        let path = tokio::fs::canonicalize(&task.working_path)
            .await
            .map_err(|error| {
                DeliveryError::Invalid(format!("The task workspace is unavailable: {error}"))
            })?;
        let (head, branch) = self.repository_state(&path).await?;
        Ok(TaskDeliveryContext {
            source: TaskDeliverySource::from(task),
            path: path.to_string_lossy().into_owned(),
            branch,
            head,
            state: task.state,
        })
    }

    /// Preparation still runs no commands that publish or alter the checkout.
    /// A running agent may prepare a review; the receipt's Git/file leases are
    /// checked again when the owner explicitly executes it.
    pub async fn prepare_task(
        &self,
        task: &bruhs_models::coordination::CoordinatedTask,
        request: TaskPrepareRequest,
    ) -> DeliveryResult<PreparedAction> {
        self.prepare_with_source(
            PrepareRequest {
                path: task.working_path.clone(),
                forge: request.forge,
                action: request.action,
                timeout_seconds: request.timeout_seconds,
            },
            Some(TaskDeliverySource::from(task)),
        )
        .await
    }

    async fn prepare_with_source(
        &self,
        request: PrepareRequest,
        source: Option<TaskDeliverySource>,
    ) -> DeliveryResult<PreparedAction> {
        let mut prepared = plan::prepare(self, request).await?;
        prepared.source_task = source;
        let receipt = ActionReceipt {
            prepared: prepared.clone(),
            status: ActionStatus::Prepared,
            completed_commands: 0,
            checked_tree: None,
            checked_commit: None,
            output: String::new(),
            error: None,
            updated_at_ms: now(),
            native_operation: None,
        };
        self.mutate(move |state| {
            if state.actions.iter().filter(|action| matches!(action.status, ActionStatus::Prepared | ActionStatus::Running)).count() >= bruhs_models::reactive::HISTORY_LIMIT {
                return Err(DeliveryError::Conflict("The action review page is full. Finish or dismiss an existing action before adding another.".into()));
            }
            state.actions.push(receipt); Ok(())
        }).await?;
        Ok(prepared)
    }

    /// Forget copied personal context without changing the reviewed commands,
    /// checkout leases, execution status or the independent source run.
    pub async fn forget_task_source(&self, id: &str) -> DeliveryResult<ActionReceipt> {
        self.mutate(|state| {
            let receipt = state
                .actions
                .iter_mut()
                .find(|receipt| receipt.prepared.id == id)
                .ok_or_else(|| {
                    DeliveryError::NotFound("The delivery receipt does not exist.".into())
                })?;
            if receipt.status == ActionStatus::Running {
                return Err(DeliveryError::Conflict(
                    "Wait for the action to stop before removing its source details.".into(),
                ));
            }
            receipt.prepared.source_task = None;
            Ok(receipt.clone())
        })
        .await
    }

    /// An opaque proposal ID is the complete execution input. Clients cannot substitute commands.
    pub async fn execute(&self, id: &str) -> DeliveryResult<ActionReceipt> {
        self.execute_authorized(id, false).await
    }

    /// Only the private owner review route exposes this separate acknowledgement.
    /// Agent/MCP delivery execution always uses `execute`, which refuses native stacks.
    pub async fn execute_native_stack(
        &self,
        id: &str,
        acknowledge_server_heads: bool,
    ) -> DeliveryResult<ActionReceipt> {
        if !acknowledge_server_heads {
            return Err(DeliveryError::Invalid("Acknowledge that GitHub may merge server-current heads after preflight; this API has no atomic expected-head guard.".into()));
        }
        self.execute_authorized(id, true).await
    }

    async fn execute_authorized(
        &self,
        id: &str,
        native_owner_ack: bool,
    ) -> DeliveryResult<ActionReceipt> {
        let receipt = self.mutate(|state| {
            let receipt = state.actions.iter_mut().find(|item| item.prepared.id == id)
                .ok_or_else(|| DeliveryError::NotFound("The reviewed delivery action does not exist.".into()))?;
            if receipt.status != ActionStatus::Prepared {
                return Err(DeliveryError::Conflict("This action was already started or dismissed. Inspect its receipt instead of replaying it.".into()));
            }
            let native = matches!(&receipt.prepared.action, DeliveryAction::MergeNativeStack { .. });
            if native != native_owner_ack { return Err(DeliveryError::Conflict("Native stacks require the owner's separate current-head acknowledgement in Delivery. Regular action execution cannot grant it.".into())); }
            receipt.status = ActionStatus::Running;
            receipt.updated_at_ms = now();
            Ok(receipt.clone())
        }).await?;
        let (cancel, cancelled) = watch::channel(false);
        self.cancellations
            .lock()
            .await
            .insert(id.to_owned(), cancel);
        let service = self.clone();
        let prepared = receipt.prepared.clone();
        tokio::spawn(async move {
            let result = service.run_action(&prepared, cancelled).await;
            let outcome = service
                .mutate(|state| {
                    let receipt = state
                        .actions
                        .iter_mut()
                        .find(|item| item.prepared.id == prepared.id)
                        .ok_or_else(|| {
                            DeliveryError::NotFound(
                                "The running action lost its durable receipt.".into(),
                            )
                        })?;
                    match result {
                        Ok(()) => {
                            if receipt.status == ActionStatus::Running {
                                receipt.status = ActionStatus::Completed;
                            }
                        }
                        Err(error) => {
                            if receipt.status != ActionStatus::Cancelled {
                                receipt.status =
                                    if receipt.native_operation.as_ref().is_some_and(|operation| {
                                        matches!(
                                            operation.state,
                                            NativeMergeState::Pending
                                                | NativeMergeState::Enqueued
                                                | NativeMergeState::Unknown
                                                | NativeMergeState::Submitting
                                        )
                                    }) {
                                        ActionStatus::Interrupted
                                    } else {
                                        ActionStatus::Failed
                                    };
                            }
                            receipt.error = Some(error.to_string());
                        }
                    }
                    receipt.updated_at_ms = now();
                    Ok(())
                })
                .await;
            service.cancellations.lock().await.remove(&prepared.id);
            if let Err(error) = outcome {
                tracing::error!(%error, "Delivery result could not be persisted; inspect repository before retrying");
            }
        });
        Ok(receipt)
    }

    pub async fn cancel(&self, id: &str) -> DeliveryResult<()> {
        self.mutate(|state| {
            let receipt = state
                .actions
                .iter_mut()
                .find(|item| item.prepared.id == id)
                .ok_or_else(|| DeliveryError::NotFound("Delivery action not found.".into()))?;
            if !matches!(
                receipt.status,
                ActionStatus::Prepared | ActionStatus::Running
            ) {
                return Err(DeliveryError::Conflict(
                    "This action has already finished.".into(),
                ));
            }
            receipt.status = ActionStatus::Cancelled;
            receipt.updated_at_ms = now();
            Ok(())
        })
        .await?;
        if let Some(cancel) = self.cancellations.lock().await.get(id) {
            cancel.send_replace(true);
        }
        Ok(())
    }

    async fn run_action(
        &self,
        prepared: &PreparedAction,
        mut cancel: watch::Receiver<bool>,
    ) -> DeliveryResult<()> {
        if self
            .state
            .lock()
            .await
            .actions
            .iter()
            .find(|action| action.prepared.id == prepared.id)
            .is_none_or(|action| action.status != ActionStatus::Running)
        {
            return Err(DeliveryError::Conflict(
                "The reviewed action is no longer running; no command was started.".into(),
            ));
        }
        if prepared.repository.is_none()
            && (plan::requires_repository(&prepared.action)
                || prepared
                    .commands
                    .iter()
                    .any(|command| command.program == "gh"))
        {
            return Err(DeliveryError::Conflict(
                "This action has no reviewed repository. Prepare it again before any command can run."
                    .into(),
            ));
        }
        let path = Path::new(&prepared.path);
        if let Some(repository) = &prepared.repository {
            let current = self.forge_repository(path, repository.forge).await?;
            if current.as_ref() != Some(repository) {
                return Err(DeliveryError::Conflict("The forge repository or origin changed after review. Prepare the action again.".into()));
            }
        }
        let (head, branch) = self.repository_state(path).await?;
        if head != prepared.expected_head || branch != prepared.expected_branch {
            return Err(DeliveryError::Conflict("The repository HEAD or branch changed after review. Prepare and review the action again.".into()));
        }
        if let DeliveryAction::CommitPushPull { files, .. } = &prepared.action {
            if !prepared
                .worktree_fingerprint
                .as_deref()
                .is_some_and(|value| value.starts_with("v2:"))
            {
                return Err(DeliveryError::Conflict("This action has a missing or older source review fingerprint. Prepare and review it again before execution.".into()));
            }
            if Some(self.fingerprint(path, files).await?) != prepared.worktree_fingerprint {
                return Err(DeliveryError::Conflict("Selected files or tracked changes changed after review. Prepare and review the action again.".into()));
            }
        }
        if matches!(&prepared.action, DeliveryAction::Verify { .. }) {
            return self.run_verification(prepared, cancel).await;
        }
        if matches!(&prepared.action, DeliveryAction::CommitPushPull { .. }) {
            self.validate_delivery_verification(prepared).await?;
        }
        if let Some(destination) = prepared.workspace_path.as_deref().map(Path::new) {
            if let Some(parent) = destination.parent() {
                tokio::fs::create_dir_all(parent).await.map_err(|error| {
                    DeliveryError::Command(format!(
                        "Could not create the workspace parent: {error}"
                    ))
                })?;
            }
        }
        if let DeliveryAction::Review {
            number, head_oid, ..
        }
        | DeliveryAction::Merge {
            number, head_oid, ..
        } = &prepared.action
        {
            if self.pull_for(path, *number, prepared.forge).await?.head_oid != *head_oid {
                return Err(DeliveryError::Conflict("The pull request changed after review. Reload its diff and prepare the action again.".into()));
            }
        }
        if let Some(stack) = &prepared.stack {
            return self.run_stack(prepared, stack, cancel).await;
        }
        if let Some(native) = &prepared.native_stack {
            return self.run_native_stack(prepared, native, cancel).await;
        }
        match tokio::time::timeout(
            std::time::Duration::from_secs(prepared.timeout_seconds),
            self.run_commands(prepared, &mut cancel, None),
        ).await {
            Ok(result) => result,
            Err(_) => Err(DeliveryError::Command(format!(
                "Execution exceeded the reviewed {} second budget. The active process was stopped; inspect earlier completed steps before preparing another action.",
                prepared.timeout_seconds
            ))),
        }
    }

    async fn run_commands(
        &self,
        prepared: &PreparedAction,
        cancel: &mut watch::Receiver<bool>,
        checkout: Option<&Path>,
    ) -> DeliveryResult<()> {
        for (index, specification) in prepared.commands.iter().enumerate() {
            if self
                .state
                .lock()
                .await
                .actions
                .iter()
                .find(|action| action.prepared.id == prepared.id)
                .is_none_or(|action| action.status != ActionStatus::Running)
            {
                return Err(DeliveryError::Command("Execution stopped before the next command; earlier completed steps remain applied.".into()));
            }
            if *cancel.borrow() {
                return Err(DeliveryError::Command(
                    "Execution cancelled; earlier completed steps remain applied.".into(),
                ));
            }
            let cwd = if let Some(checkout) = checkout {
                checkout
            } else if index > 0 {
                Path::new(prepared.workspace_path.as_deref().unwrap_or(&prepared.path))
            } else {
                Path::new(&prepared.path)
            };
            let checked_push = self
                .resolve_checked_command(prepared, specification)
                .await?;
            let specification = checked_push.as_ref().unwrap_or(specification);
            let output = self
                .run_command(cwd, specification, cancel, prepared.repository.as_ref())
                .await?;
            gitlab::validate_mutation_output(specification, &output)?;
            let output_full = self
                .mutate(|state| {
                    let receipt = state
                        .actions
                        .iter_mut()
                        .find(|item| item.prepared.id == prepared.id)
                        .ok_or_else(|| {
                            DeliveryError::NotFound(
                                "Delivery receipt was removed while running.".into(),
                            )
                        })?;
                    receipt.completed_commands = index + 1;
                    let full =
                        receipt.output.len().saturating_add(output.len()) > command::OUTPUT_BYTES;
                    if !full {
                        receipt.output.push_str(&output);
                        if !output.ends_with('\n') {
                            receipt.output.push('\n');
                        }
                    }
                    receipt.updated_at_ms = now();
                    Ok(full)
                })
                .await?;
            if output_full {
                return Err(DeliveryError::Command("The last command completed but its output exceeded the combined 1 MiB receipt budget. Its completed-step count was saved. Inspect the repository before preparing another action.".into()));
            }
            if let (0, None, Some(workspace)) =
                (index, checkout, prepared.workspace_path.as_deref())
            {
                // The reviewed `git worktree add` just made the issue
                // workspace; reuse the checkout's build output before
                // setup runs. Clones only, so nothing is copied or
                // committed and the owner's checkout is untouched.
                let seeded = crate::git::seed_worktree_build_dirs(
                    Path::new(&prepared.path),
                    Path::new(workspace),
                )
                .await;
                if !seeded.is_empty() {
                    tracing::info!("issue workspace {workspace} shares build folders {seeded:?}");
                }
            }
        }
        Ok(())
    }

    async fn repository_state(&self, path: &Path) -> DeliveryResult<(String, String)> {
        let (head, branch) = tokio::try_join!(
            self.read(path, "git", &["rev-parse", "HEAD"]),
            self.read(path, "git", &["symbolic-ref", "--short", "HEAD"])
        )?;
        Ok((head.trim().to_owned(), branch.trim().to_owned()))
    }
}

fn now() -> i64 {
    bruhs_models::EpochMs::now().as_millis()
}
