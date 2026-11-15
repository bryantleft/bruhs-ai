//! Durable task workspaces, independently owned by the local server.
use bruhs_models::{
    agent::{provider_for_model_id, PermissionMode},
    coordination::*,
    ids::ThreadId,
    thread_link::ThreadAccess,
    EpochMs,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{watch, Mutex, Semaphore};
use tokio_stream::StreamExt;

mod access;
mod board;
mod recovery;
mod wait;
mod wakes;
pub use access::{AccessGrant, AccessScope, IssuedAccess};
pub use board::{BoardLinks, LinkFuture, LinkSource};
pub use recovery::ResetWorker;
pub use wakes::WakeWorker;

// Match the existing 120,000-character composer budget at the maximum UTF-8
// width. One task's retained text is bounded before snapshot serialization.
const TEXT_BUDGET: usize = bruhs_models::logic::composer_text::MESSAGE_CHAR_LIMIT * 4;

#[derive(Debug, thiserror::Error)]
pub enum CoordinationError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Conflict(String),
    #[error("Task or specification not found: {0}")]
    NotFound(String),
    #[error("Task workspace storage failed: {0}")]
    Storage(String),
    #[error("This access link is expired, revoked, or does not permit this action.")]
    Unauthorized,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Data {
    #[serde(default)]
    revision: u64,
    #[serde(default)]
    tasks: BTreeMap<ThreadId, CoordinatedTask>,
    #[serde(default)]
    specs: BTreeMap<ThreadId, LivingSpec>,
    #[serde(default)]
    proposals: BTreeMap<ThreadId, GuestProposal>,
    #[serde(default)]
    grants: BTreeMap<String, AccessGrant>,
    #[serde(default)]
    wake_subscriptions: BTreeMap<ThreadId, WakeSubscription>,
    #[serde(default)]
    wakes: BTreeMap<ThreadId, TaskWake>,
}

struct CoordinationOwner(std::fs::File);
impl Drop for CoordinationOwner {
    fn drop(&mut self) {
        // Shared Inner references, including active workers, retain authority.
        // Explicit unlock also releases descriptors briefly inherited by fork.
        if let Err(error) = self.0.unlock() {
            tracing::warn!(%error, "Could not explicitly release the task workspace owner lock");
        }
    }
}

struct Inner {
    data: Mutex<Data>,
    committed: Mutex<Data>,
    path: PathBuf,
    _lock: CoordinationOwner,
    app: Arc<crate::AppState>,
    cancels: Mutex<BTreeMap<ThreadId, watch::Sender<bool>>>,
    slots: Arc<Semaphore>,
    permits: std::sync::Mutex<BTreeMap<ThreadId, wait::TaskSlot>>,
    changes: watch::Sender<u64>,
    wake_error: Mutex<Option<String>>,
    capacity: usize,
}

#[derive(Clone)]
pub struct CoordinationRuntime(Arc<Inner>);

impl CoordinationRuntime {
    pub fn default_path() -> Result<PathBuf, CoordinationError> {
        if let Some(root) = std::env::var_os("BRUHS_WORKBENCH_DIR") {
            if root.is_empty() {
                return Err(invalid("BRUHS_WORKBENCH_DIR cannot be empty."));
            }
            return Ok(PathBuf::from(root).join("task-workspaces.json"));
        }
        directories::ProjectDirs::from("dev", "bruhs", "bruhs-ai")
            .map(|dirs| dirs.data_dir().join("server/task-workspaces.json"))
            .ok_or_else(|| {
                CoordinationError::Storage("Cannot locate the app data directory.".into())
            })
    }

    pub async fn open(path: PathBuf, app: Arc<crate::AppState>) -> Result<Self, CoordinationError> {
        let (path, lock, mut data) = tokio::task::spawn_blocking(move || load(path))
            .await
            .map_err(storage)??;
        for task in data
            .tasks
            .values_mut()
            .filter(|task| task.state.active() || task.state == TaskState::Ready)
        {
            task.state = TaskState::Interrupted;
            task.approvals.clear();
            task.error = Some("The host stopped during this task. Review its files and send a message to resume; no command was replayed.".into());
        }
        for wake in data
            .wakes
            .values_mut()
            .filter(|wake| wake.state == TaskWakeState::Reserved)
        {
            wake.state = TaskWakeState::Unconfirmed;
            wake.error=Some("The host stopped after reserving this event. No continuation was replayed; inspect the task and send an explicit message to continue.".into());
        }
        let (changes, _) = watch::channel(data.revision);
        // A host runs at most one task worker per available logical CPU. The
        // OS supplies this resource bound; queued work does not hold a CLI.
        let slots = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
        let runtime = Self(Arc::new(Inner {
            committed: Mutex::new(data.clone()),
            data: Mutex::new(data),
            path,
            _lock: lock,
            app,
            cancels: Mutex::new(BTreeMap::new()),
            slots: Arc::new(Semaphore::new(slots)),
            permits: std::sync::Mutex::new(BTreeMap::new()),
            changes,
            wake_error: Mutex::new(None),
            capacity: slots,
        }));
        runtime.save().await?;
        Ok(runtime)
    }

    pub fn changes(&self) -> watch::Receiver<u64> {
        self.0.changes.subscribe()
    }

    pub async fn snapshot(&self) -> CoordinationSnapshot {
        let data = self.0.data.lock().await;
        let mut snapshot = snapshot(&data);
        drop(data);
        snapshot.wake_error = self.0.wake_error.lock().await.clone();
        snapshot
    }

    pub async fn command(
        &self,
        command: CoordinationCommand,
    ) -> Result<CoordinationSnapshot, CoordinationError> {
        match command {
            CoordinationCommand::SaveWakeSubscription {
                task_id,
                watch_id,
                kinds,
                expected_model,
                expected_permission_mode,
            } => {
                self.save_wake_subscription(
                    task_id,
                    watch_id,
                    kinds,
                    expected_model,
                    expected_permission_mode,
                )
                .await?
            }
            CoordinationCommand::SetWakeSubscription { id, enabled } => {
                self.set_wake_subscription(&id, enabled).await?
            }
            CoordinationCommand::DismissWake { id } => self.dismiss_wake(&id).await?,
            CoordinationCommand::Create { task, parent_id } => {
                let relationship = if parent_id.is_some() {
                    Relationship::Delegate
                } else {
                    Relationship::Root
                };
                self.create(task, parent_id, relationship).await?;
            }
            CoordinationCommand::Send { id, prompt } => {
                self.send(&id, prompt).await?;
            }
            CoordinationCommand::Cancel { id } => self.cancel(&id).await?,
            CoordinationCommand::Fork {
                id,
                title,
                prompt,
                relationship,
                model,
                isolation,
            } => {
                if relationship == Relationship::Root {
                    return Err(invalid("Choose fork, side chat, delegate, or verify."));
                }
                let parent = self.task(&id).await?;
                let permission_mode =
                    if matches!(relationship, Relationship::Verify | Relationship::SideChat) {
                        PermissionMode::Plan
                    } else {
                        parent.permission_mode
                    };
                let child = NewTask {
                    title,
                    prompt,
                    model,
                    isolation,
                    permission_mode,
                    project_path: parent.project_path,
                    spec_id: parent.spec_id,
                };
                self.create(child, Some(id), relationship).await?;
            }
            CoordinationCommand::Fanout {
                task,
                models,
                parent_id,
            } => {
                if models.len() > self.0.capacity {
                    return Err(invalid(
                        "Choose no more models than this host has logical CPUs; start another comparison after this one finishes.",
                    ));
                }
                if !matches!(task.isolation, Isolation::Worktree { .. }) {
                    return Err(invalid(
                        "Fan-out requires a separate worktree for each model.",
                    ));
                }
                if models.is_empty() {
                    return Err(invalid("Choose at least one model."));
                }
                for model in &models {
                    validate_text(model, "Model")?;
                }
                for model in models {
                    let mut child = task.clone();
                    child.model = model;
                    let relationship = if parent_id.is_some() {
                        Relationship::Delegate
                    } else {
                        Relationship::Root
                    };
                    self.create(child, parent_id.clone(), relationship).await?;
                }
            }
            CoordinationCommand::BringBack { id } => {
                let child = self.task(&id).await?;
                let parent = child
                    .parent_id
                    .ok_or_else(|| invalid("This task has no parent to bring results back to."))?;
                if child.state.active() || child.state == TaskState::Ready {
                    return Err(conflict("Wait for the child to finish or cancel it first."));
                }
                let result = child
                    .messages
                    .iter()
                    .rev()
                    .find(|message| message.role == "assistant")
                    .ok_or_else(|| invalid("This task has no completed answer to bring back."))?;
                self.send(
                    &parent,
                    format!(
                        "Results from {} ({}):\n{}",
                        child.title, child.id, result.text
                    ),
                )
                .await?;
            }
            CoordinationCommand::SaveSpec {
                id,
                project_path,
                title,
                content,
                expected_revision,
            } => {
                validate_text(&title, "Specification title")?;
                validate_text(&content, "Specification")?;
                let path = canonical_project(&project_path).await?;
                let mut data = self.0.data.lock().await;
                let id = id.unwrap_or_else(ThreadId::new_v4);
                let current = data.specs.get(&id).map_or(0, |spec| spec.revision);
                if current != expected_revision {
                    return Err(conflict(
                        "The specification changed. Refresh before saving your edits.",
                    ));
                }
                if data
                    .specs
                    .get(&id)
                    .is_some_and(|spec| spec.project_path != path)
                {
                    return Err(invalid("A specification cannot move to another project."));
                }
                data.specs.insert(
                    id.clone(),
                    LivingSpec {
                        id,
                        project_path: path,
                        title,
                        content,
                        revision: current
                            .checked_add(1)
                            .ok_or_else(|| conflict("Specification revision is exhausted."))?,
                        updated_at: EpochMs::now(),
                    },
                );
                self.persist(&mut data).await?;
                drop(data);
            }
            CoordinationCommand::AttachSpec { id, spec_id } => {
                let mut data = self.0.data.lock().await;
                let spec = data
                    .specs
                    .get(&spec_id)
                    .cloned()
                    .ok_or_else(|| not_found(&spec_id))?;
                let task = data.tasks.get_mut(&id).ok_or_else(|| not_found(&id))?;
                if task.state.active() {
                    return Err(conflict("Stop the task before changing its specification."));
                }
                if task.project_path != spec.project_path {
                    return Err(invalid("The specification belongs to a different project."));
                }
                task.spec_id = Some(spec_id);
                task.spec_revision = Some(spec.revision);
                task.session_id = None;
                self.persist(&mut data).await?;
                drop(data);
            }
            CoordinationCommand::Approval {
                id,
                request_id,
                allow,
            } => {
                {
                    let data = self.0.data.lock().await;
                    let task = data.tasks.get(&id).ok_or_else(|| not_found(&id))?;
                    if !task
                        .approvals
                        .iter()
                        .any(|approval| approval.id == request_id)
                    {
                        return Err(conflict(
                            "That approval is no longer pending for this task.",
                        ));
                    }
                    drop(data);
                }
                let decision = if allow {
                    crate::PermissionDecision::Allow
                } else {
                    crate::PermissionDecision::Deny { reason: None }
                };
                self.0
                    .app
                    .resolve_permission(&request_id, decision)
                    .await
                    .map_err(|error| conflict(&error.to_string()))?;
                let mut data = self.0.data.lock().await;
                if let Some(task) = data.tasks.get_mut(&id) {
                    answered_approval(task, &request_id);
                }
                self.persist(&mut data).await?;
                drop(data);
            }
            CoordinationCommand::GuestProposal { id, accept } => {
                let proposal = self
                    .0
                    .data
                    .lock()
                    .await
                    .proposals
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| not_found(&id))?;
                if accept {
                    self.send(&proposal.task_id, proposal.text).await?;
                }
                let mut data = self.0.data.lock().await;
                data.proposals.remove(&id);
                self.persist(&mut data).await?;
                drop(data);
            }
            CoordinationCommand::Delete { id } => {
                let mut data = self.0.data.lock().await;
                let task = data.tasks.get(&id).ok_or_else(|| not_found(&id))?;
                if task.state.active()
                    || data
                        .tasks
                        .values()
                        .any(|task| task.parent_id.as_ref() == Some(&id))
                {
                    return Err(conflict(
                        "Stop the task and remove its child tasks before deleting it. Files and branches are retained.",
                    ));
                }
                data.tasks.remove(&id);
                data.wake_subscriptions
                    .retain(|_, subscription| subscription.task_id != id);
                data.wakes.retain(|_, wake| wake.task_id != id);
                data.grants.retain(|_, grant| !matches!(&grant.scope, AccessScope::Guest { task_id } if task_id == &id));
                data.proposals.retain(|_, proposal| proposal.task_id != id);
                self.persist(&mut data).await?;
                drop(data);
            }
            CoordinationCommand::ScheduleReset { id } => self.schedule_reset(&id).await?,
            CoordinationCommand::CancelResume { id } => {
                let mut data = self.0.data.lock().await;
                let task = data.tasks.get_mut(&id).ok_or_else(|| not_found(&id))?;
                task.resume_at = None;
                self.persist(&mut data).await?;
                drop(data);
            }
            CoordinationCommand::Archive { id } => self.set_archived(&id, true).await?,
            CoordinationCommand::Unarchive { id } => self.set_archived(&id, false).await?,
            CoordinationCommand::AutoResume { id, enabled } => {
                let mut data = self.0.data.lock().await;
                let task = data.tasks.get_mut(&id).ok_or_else(|| not_found(&id))?;
                task.auto_resume_on_limit = enabled;
                if !enabled {
                    task.resume_at = None;
                }
                self.persist(&mut data).await?;
                drop(data);
            }
        }
        Ok(self.snapshot().await)
    }

    pub async fn task(&self, id: &ThreadId) -> Result<CoordinatedTask, CoordinationError> {
        self.0
            .data
            .lock()
            .await
            .tasks
            .get(id)
            .cloned()
            .ok_or_else(|| not_found(id))
    }

    async fn create(
        &self,
        request: NewTask,
        parent_id: Option<ThreadId>,
        relationship: Relationship,
    ) -> Result<ThreadId, CoordinationError> {
        validate_new_task(&request).map_err(invalid)?;
        let project_path = canonical_project(&request.project_path).await?;
        let id = ThreadId::new_v4();
        let (source_context, source_revision, shared_path) = {
            let data = self.0.data.lock().await;
            if let Some(parent_id) = &parent_id {
                let parent = data
                    .tasks
                    .get(parent_id)
                    .ok_or_else(|| not_found(parent_id))?;
                if parent.project_path != project_path {
                    return Err(invalid(
                        "A delegated task must remain in its parent's project.",
                    ));
                }
                if !narrower(parent.permission_mode, request.permission_mode) {
                    return Err(invalid("A child task cannot increase its parent's access."));
                }
            }
            if let Some(spec_id) = &request.spec_id {
                let spec = data.specs.get(spec_id).ok_or_else(|| not_found(spec_id))?;
                if spec.project_path != project_path {
                    return Err(invalid("The specification belongs to another project."));
                }
            }
            let source = parent_id
                .as_ref()
                .and_then(|id| data.tasks.get(id))
                .map(|parent| parent.messages.clone())
                .unwrap_or_default();
            let source_bytes = source.iter().fold(0usize, |total, message| {
                total.saturating_add(message.text.len())
            });
            if source_bytes.saturating_add(request.prompt.len()) > TEXT_BUDGET {
                return Err(invalid(
                    "The parent transcript is too large for a child context. Start a task with a concise handoff.",
                ));
            }
            let shared_path = parent_id
                .as_ref()
                .and_then(|id| data.tasks.get(id))
                .map_or_else(|| project_path.clone(), |task| task.working_path.clone());
            let source_revision = parent_id.as_ref().map(|_| data.revision);
            drop(data);
            (source, source_revision, shared_path)
        };
        let working_path = match request.isolation {
            Isolation::Shared => shared_path,
            Isolation::Worktree { base_branch } => {
                let branch = format!("bruhs/task-{id}");
                let dest = crate::git::worktree_dest(Path::new(&project_path), &branch)
                    .map_err(|error| invalid(&error.to_string()))?;
                crate::git::add_worktree(Path::new(&project_path), &base_branch, &dest, &branch)
                    .await
                    .map_err(|error| invalid(&error.to_string()))?;
                dest.to_string_lossy().into_owned()
            }
        };
        let now = EpochMs::now();
        let mut data = self.0.data.lock().await;
        let spec_revision = request
            .spec_id
            .as_ref()
            .and_then(|id| data.specs.get(id))
            .map(|spec| spec.revision);
        data.tasks.insert(
            id.clone(),
            CoordinatedTask {
                id: id.clone(),
                parent_id,
                source_context,
                source_revision,
                relationship,
                title: request.title,
                project_path,
                working_path,
                model: request.model,
                permission_mode: request.permission_mode,
                spec_id: request.spec_id,
                spec_revision,
                state: TaskState::Ready,
                messages: vec![TaskMessage {
                    role: "user".into(),
                    text: request.prompt,
                    at: now,
                }],
                live_output: String::new(),
                error: None,
                session_id: None,
                approvals: vec![],
                created_at: now,
                updated_at: now,
                resume_at: None,
                auto_resume_on_limit: false,
                limit_reported: false,
                archived_at: None,
            },
        );
        self.persist(&mut data).await?;
        drop(data);
        self.launch(id.clone()).await;
        Ok(id)
    }

    /// Archiving only files a finished task away: its record, transcript,
    /// worktree and branch stay, and a new message brings it back.
    async fn set_archived(&self, id: &ThreadId, archived: bool) -> Result<(), CoordinationError> {
        let mut data = self.0.data.lock().await;
        let task = data.tasks.get_mut(id).ok_or_else(|| not_found(id))?;
        if archived && (task.state.active() || task.state == TaskState::Ready) {
            return Err(conflict(
                "This task is still running or queued. Stop it or let it finish before archiving.",
            ));
        }
        let next = archived.then(|| task.archived_at.unwrap_or_else(EpochMs::now));
        if task.archived_at == next {
            return Ok(());
        }
        task.archived_at = next;
        self.persist(&mut data).await?;
        drop(data);
        Ok(())
    }

    async fn send(&self, id: &ThreadId, prompt: String) -> Result<(), CoordinationError> {
        validate_text(&prompt, "Message")?;
        if self.0.cancels.lock().await.contains_key(id) {
            return Err(conflict(
                "The previous provider process is still stopping. Retry after it releases this task.",
            ));
        }
        let mut data = self.0.data.lock().await;
        let task = data.tasks.get_mut(id).ok_or_else(|| not_found(id))?;
        if task.state.active() || task.state == TaskState::Ready {
            return Err(conflict(
                "This task is still running or queued. Stop it before sending another message.",
            ));
        }
        task.messages.push(TaskMessage {
            role: "user".into(),
            text: prompt,
            at: EpochMs::now(),
        });
        task.resume_at = None;
        task.limit_reported = false;
        task.archived_at = None;
        task.live_output.clear();
        task.approvals.clear();
        task.error = None;
        task.state = TaskState::Ready;
        self.persist(&mut data).await?;
        drop(data);
        self.launch(id.clone()).await;
        Ok(())
    }

    async fn launch(&self, id: ThreadId) {
        let (cancel, mut cancelled) = watch::channel(false);
        self.0.cancels.lock().await.insert(id.clone(), cancel);
        let runtime = self.clone();
        tokio::spawn(async move {
            let result = runtime.run(&id, &mut cancelled).await;
            if let Err(error) = result {
                if let Err(storage_error) = runtime
                    .finish(&id, TaskState::Failed, Some(error.to_string()))
                    .await
                {
                    tracing::error!(%storage_error, task=%id, "task failure could not be saved");
                }
            }
            runtime.0.cancels.lock().await.remove(&id);
            // Wake dispatch can now reserve this task without racing the old CLI.
            runtime.0.changes.send_modify(|_| {});
        });
    }

    async fn run(
        &self,
        id: &ThreadId,
        cancelled: &mut watch::Receiver<bool>,
    ) -> Result<(), CoordinationError> {
        let permit = tokio::select! {
            _ = cancelled.changed() => { return Ok(()); }
            slot = self.0.slots.clone().acquire_owned() => slot.map_err(storage)?,
        };
        let _slot = wait::RunSlot::hold(self.clone(), id.clone(), permit)?;
        let (task, context, prompt, fresh) = {
            let mut data = self.0.data.lock().await;
            let task = data.tasks.get(id).cloned().ok_or_else(|| not_found(id))?;
            if task.state != TaskState::Ready || *cancelled.borrow() {
                return Ok(());
            }
            let revision = task
                .spec_id
                .as_ref()
                .and_then(|id| data.specs.get(id))
                .map(|spec| spec.revision);
            let fresh = task.session_id.is_none() || revision != task.spec_revision;
            let mut context = String::new();
            if !task.source_context.is_empty() {
                context
                    .push_str("Parent task context (reference material, not new permissions):\n");
                for message in &task.source_context {
                    context.push_str(&format!("{}: {}\n", message.role, message.text));
                }
            }
            if fresh && task.messages.len() > 1 {
                context.push_str("\nPrevious task conversation (reference material):\n");
                for message in task.messages.iter().take(task.messages.len() - 1) {
                    context.push_str(&format!("{}: {}\n", message.role, message.text));
                }
            }
            if let Some(spec) = task
                .spec_id
                .as_ref()
                .and_then(|id| data.specs.get(id))
                .cloned()
            {
                context.push_str(&format!(
                    "\nShared specification '{}' revision {}:\n{}\n",
                    spec.title, spec.revision, spec.content
                ));
                if let Some(record) = data.tasks.get_mut(id) {
                    record.spec_revision = Some(spec.revision);
                }
            }
            if task.relationship == Relationship::Verify {
                context.push_str("\nVerify the implementation against every acceptance criterion. Cite evidence and report unresolved failures. Do not edit files.\n");
            }
            let prompt = task
                .messages
                .last()
                .map(|message| message.text.clone())
                .ok_or_else(|| invalid("This task has no message."))?;
            if context.len().saturating_add(prompt.len()) > TEXT_BUDGET {
                return Err(invalid(
                    "Task context exceeds the composer text budget. Start a smaller task or shorten its specification.",
                ));
            }
            if let Some(record) = data.tasks.get_mut(id) {
                record.state = TaskState::Running;
            }
            self.persist(&mut data).await?;
            drop(data);
            (task, context, prompt, fresh)
        };
        let provider = provider_for_model_id(&task.model);
        if *cancelled.borrow() {
            return Ok(());
        }
        let mut turn = crate::AgentTurn::new(
            id.clone(),
            prompt,
            task.working_path,
            task.model,
            provider,
            task.permission_mode,
        )
        .resume(task.session_id)
        .instructions(Some(context))
        .thread_access(ThreadAccess::Off);
        if fresh {
            turn = turn.fresh();
        }
        let mut events = self
            .0
            .app
            .send_message(turn)
            .await
            .map_err(|error| conflict(&error.to_string()))?;
        loop {
            let event = tokio::select! {
                _ = cancelled.changed() => { return Ok(()); }
                event = events.next() => event,
            };
            let Some(event) = event else {
                return self.finish(id, TaskState::Failed, Some("The provider ended without a completion result. Review the output before retrying.".into())).await;
            };
            match event {
                crate::OrchestratorEvent::Query(crate::QueryEvent::Result {
                    result,
                    session_id,
                    is_error,
                    ..
                }) => {
                    let mut data = self.0.data.lock().await;
                    if let Some(task) = data.tasks.get_mut(id) {
                        if task.state == TaskState::Cancelled {
                            return Ok(());
                        }
                        task.session_id = Some(session_id);
                        if task.live_output.is_empty() {
                            if result.len() > TEXT_BUDGET {
                                return Err(invalid(
                                    "The provider result exceeds this task's retained text budget.",
                                ));
                            }
                            task.live_output = result;
                        }
                    }
                    drop(data);
                    return self
                        .finish(
                            id,
                            if is_error {
                                TaskState::Failed
                            } else {
                                TaskState::Completed
                            },
                            is_error
                                .then(|| "The provider reported an unsuccessful result.".into()),
                        )
                        .await;
                }
                crate::OrchestratorEvent::Query(crate::QueryEvent::Error(error)) => {
                    return self.finish(id, TaskState::Failed, Some(error)).await;
                }
                crate::OrchestratorEvent::Query(crate::QueryEvent::SecretRequest(request)) => {
                    self.0
                        .app
                        .secrets()
                        .resolve(
                            id,
                            &request.id,
                            bruhs_models::secrets::SecretDecision::Decline,
                        )
                        .await
                        .map_err(|error| conflict(&error.to_string()))?;
                }
                crate::OrchestratorEvent::Query(event) => {
                    self.observe(id, event).await?;
                }
                crate::OrchestratorEvent::BackgroundTaskRegistered(_) => {}
            }
        }
    }

    async fn observe(
        &self,
        id: &ThreadId,
        event: crate::QueryEvent,
    ) -> Result<(), CoordinationError> {
        let mut data = self.0.data.lock().await;
        let Some(task) = data.tasks.get_mut(id) else {
            return Err(not_found(id));
        };
        if task.state == TaskState::Cancelled {
            return Ok(());
        }
        let durable = match event {
            crate::QueryEvent::Text(text) => {
                if task.live_output.len().saturating_add(text.len()) > TEXT_BUDGET {
                    return Err(invalid(
                        "Task output exceeded the retained composer text budget. The provider was stopped; existing output is retained.",
                    ));
                }
                task.live_output.push_str(&text);
                false
            }
            crate::QueryEvent::Init { session_id, .. } => {
                task.session_id = Some(session_id);
                true
            }
            crate::QueryEvent::PermissionRequest {
                id, tool, input, ..
            } => {
                if serde_json::to_vec(&input).map_err(storage)?.len() > TEXT_BUDGET {
                    return Err(invalid(
                        "The proposed tool input exceeds the task approval budget.",
                    ));
                }
                task.approvals.push(PendingApproval { id, tool, input });
                task.state = TaskState::WaitingApproval;
                true
            }
            crate::QueryEvent::PermissionCancelled { id } => {
                answered_approval(task, &id);
                true
            }
            crate::QueryEvent::Raw(value)
                if value["type"] == "rate_limit_event"
                    && value["rate_limit_info"]["status"] == "rejected" =>
            {
                task.limit_reported = true;
                true
            }
            _ => false,
        };
        task.updated_at = EpochMs::now();
        if durable {
            self.persist(&mut data).await?;
        }
        drop(data);
        if !durable {
            self.0
                .changes
                .send_modify(|revision| *revision = revision.wrapping_add(1));
        }
        Ok(())
    }

    async fn finish(
        &self,
        id: &ThreadId,
        state: TaskState,
        error: Option<String>,
    ) -> Result<(), CoordinationError> {
        let mut data = self.0.data.lock().await;
        let task = data.tasks.get_mut(id).ok_or_else(|| not_found(id))?;
        if task.state == TaskState::Cancelled {
            return Ok(());
        }
        if !task.live_output.is_empty() {
            task.messages.push(TaskMessage {
                role: "assistant".into(),
                text: std::mem::take(&mut task.live_output),
                at: EpochMs::now(),
            });
        }
        let state = if state == TaskState::Failed && task.limit_reported {
            TaskState::Limited
        } else {
            state
        };
        task.state = state;
        task.error = error;
        task.approvals.clear();
        task.updated_at = EpochMs::now();
        let retry = task.auto_resume_on_limit && state == TaskState::Limited;
        for wake in data
            .wakes
            .values_mut()
            .filter(|wake| wake.task_id == *id && wake.state == TaskWakeState::Reserved)
        {
            wake.state = TaskWakeState::Completed;
            wake.outcome = Some(state);
        }
        self.persist(&mut data).await?;
        drop(data);
        if retry {
            if let Err(error) = self.schedule_reset(id).await {
                tracing::debug!(%error, task=%id, "no provider-reported quota reset can resume this task");
            }
        }
        Ok(())
    }

    async fn cancel(&self, id: &ThreadId) -> Result<(), CoordinationError> {
        let mut data = self.0.data.lock().await;
        if !data.tasks.contains_key(id) {
            return Err(not_found(id));
        }
        let mut ids = vec![id.clone()];
        loop {
            let next: Vec<_> = data
                .tasks
                .values()
                .filter(|task| {
                    !ids.contains(&task.id)
                        && task
                            .parent_id
                            .as_ref()
                            .is_some_and(|parent| ids.contains(parent))
                })
                .map(|task| task.id.clone())
                .collect();
            if next.is_empty() {
                break;
            }
            ids.extend(next);
        }
        for subscription in data
            .wake_subscriptions
            .values_mut()
            .filter(|subscription| ids.contains(&subscription.task_id))
        {
            subscription.enabled = false;
        }
        for wake in data.wakes.values_mut().filter(|wake| {
            ids.contains(&wake.task_id)
                && matches!(wake.state, TaskWakeState::Queued | TaskWakeState::Reserved)
        }) {
            wake.state = TaskWakeState::Dismissed;
            wake.error = Some("The owner stopped this task tree.".into());
        }
        for id in &ids {
            if let Some(task) = data.tasks.get_mut(id) {
                task.resume_at = None;
            }
            if let Some(task) = data
                .tasks
                .get_mut(id)
                .filter(|task| task.state.active() || task.state == TaskState::Ready)
            {
                task.state = TaskState::Cancelled;
                task.approvals.clear();
                task.updated_at = EpochMs::now();
                if !task.live_output.is_empty() {
                    task.messages.push(TaskMessage {
                        role: "assistant".into(),
                        text: std::mem::take(&mut task.live_output),
                        at: EpochMs::now(),
                    });
                }
            }
        }
        self.persist(&mut data).await?;
        drop(data);
        let cancels = self.0.cancels.lock().await;
        for id in ids {
            if let Some(cancel) = cancels.get(&id) {
                cancel.send_replace(true);
            }
        }
        Ok(())
    }

    async fn save(&self) -> Result<(), CoordinationError> {
        let mut data = self.0.data.lock().await;
        self.persist(&mut data).await
    }

    async fn persist(&self, data: &mut Data) -> Result<(), CoordinationError> {
        data.revision = data
            .revision
            .checked_add(1)
            .ok_or_else(|| conflict("Task revision is exhausted."))?;
        let bytes = serde_json::to_vec(data).map_err(storage)?;
        let path = self.0.path.clone();
        let saved = tokio::task::spawn_blocking(move || {
            use std::io::Write as _;
            let parent = path
                .parent()
                .ok_or_else(|| invalid("Task store needs a parent directory."))?;
            let mut file = tempfile::NamedTempFile::new_in(parent).map_err(storage)?;
            file.write_all(&bytes).map_err(storage)?;
            file.as_file().sync_all().map_err(storage)?;
            file.persist(&path).map_err(storage)?;
            #[cfg(unix)]
            std::fs::File::open(parent)
                .and_then(|directory| directory.sync_all())
                .map_err(storage)?;
            Ok::<_, CoordinationError>(())
        })
        .await
        .map_err(storage)
        .flatten();
        if let Err(error) = saved {
            *data = self.0.committed.lock().await.clone();
            return Err(error);
        }
        *self.0.committed.lock().await = data.clone();
        self.0.changes.send_replace(data.revision);
        Ok(())
    }
}

fn load(path: PathBuf) -> Result<(PathBuf, CoordinationOwner, Data), CoordinationError> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("Task store needs a parent directory."))?;
    std::fs::create_dir_all(parent).map_err(storage)?;
    let path = std::fs::canonicalize(parent).map_err(storage)?.join(
        path.file_name()
            .ok_or_else(|| invalid("Task store needs a filename."))?,
    );
    let mut options = std::fs::OpenOptions::new();
    options.create(true).read(true).write(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let lock = options.open(path.with_extension("lock")).map_err(storage)?;
    lock.try_lock().map_err(|_| {
        conflict("Another process owns these task workspaces. Connect to that server instead.")
    })?;
    let lock = CoordinationOwner(lock);
    let data = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
            CoordinationError::Storage(format!(
                "Saved task data is invalid; the original file was retained: {error}"
            ))
        })?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Data::default(),
        Err(error) => return Err(storage(error)),
    };
    Ok((path, lock, data))
}

fn snapshot(data: &Data) -> CoordinationSnapshot {
    CoordinationSnapshot {
        can_control: true,
        revision: data.revision,
        tasks: data.tasks.values().cloned().collect(),
        specs: data.specs.values().cloned().collect(),
        guest_proposals: data.proposals.values().cloned().collect(),
        wake_subscriptions: data.wake_subscriptions.values().cloned().collect(),
        wakes: data.wakes.values().cloned().collect(),
        wake_error: None,
    }
}
fn answered_approval(task: &mut CoordinatedTask, id: &str) {
    task.approvals.retain(|approval| approval.id != id);
    if task.approvals.is_empty() && task.state == TaskState::WaitingApproval {
        task.state = TaskState::Running;
    }
}
fn validate_text(text: &str, name: &str) -> Result<(), CoordinationError> {
    if text.trim().is_empty() {
        Err(invalid(&format!("{name} cannot be empty.")))
    } else if text.len() > TEXT_BUDGET {
        Err(invalid(&format!(
            "{name} exceeds the composer text budget."
        )))
    } else {
        Ok(())
    }
}
async fn canonical_project(path: &str) -> Result<String, CoordinationError> {
    let canonical = tokio::fs::canonicalize(path)
        .await
        .map_err(|error| invalid(&format!("Project folder could not be opened: {error}")))?;
    if !tokio::fs::metadata(&canonical)
        .await
        .map_err(storage)?
        .is_dir()
    {
        return Err(invalid("Choose a project directory."));
    }
    Ok(canonical.to_string_lossy().into_owned())
}
fn invalid(message: &str) -> CoordinationError {
    CoordinationError::Invalid(message.into())
}
fn conflict(message: &str) -> CoordinationError {
    CoordinationError::Conflict(message.into())
}
fn not_found(id: &ThreadId) -> CoordinationError {
    CoordinationError::NotFound(id.to_string())
}
fn storage(error: impl std::fmt::Display) -> CoordinationError {
    CoordinationError::Storage(error.to_string())
}

#[cfg(test)]
mod tests;
