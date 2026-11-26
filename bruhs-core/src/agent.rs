//! Runs agent turns through the user's selected agent runtime for a
//! thread, streams its events, tracks each thread's turn state and answers
//! the tool-permission prompts a Claude turn raises.

use crate::models::{BackgroundTask, TaskKind, TaskStatus};
use crate::registry::BackgroundTaskRegistry;
use bruhs_models::agent::{AgentProvider, PermissionMode};
use bruhs_models::crew::{
    claude_deny_rules, effective_level, gate, AccessLevel, BruhAccess, Gate, GatedAction,
};
use bruhs_models::ids::ThreadId;
use bruhs_models::thread_link::ThreadAccess;
use codex_sdk_rs::{ApprovalPolicy, CodexAccess, SandboxMode};
use futures_core::Stream;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};
use tokio::sync::{mpsc, oneshot, RwLock};

mod budget;
mod external;
pub(crate) mod model_options;
mod question_files;
pub(crate) fn opencode_executable() -> std::io::Result<std::path::PathBuf> {
    external::executable(AgentProvider::OpenCode)
}
pub use external::health;
pub(crate) use external::metadata::claude_usage_metadata;
#[cfg(unix)]
pub(crate) use external::process::ProcessGroup;
pub use external::writer_catalog::{writer_catalog, writer_catalog_for_project};

/// The models `provider`'s CLI lists for the signed-in account (Cursor's
/// `--list-models`), cached for a few minutes. Other runtimes, and a CLI that
/// cannot list, return no models; the error says why when there is one.
pub async fn list_runtime_models(
    provider: AgentProvider,
) -> bruhs_models::runtime_models::RuntimeModels {
    list_runtime_models_for_project(provider, std::path::Path::new("")).await
}

pub async fn list_runtime_models_for_project(
    provider: AgentProvider,
    project: &std::path::Path,
) -> bruhs_models::runtime_models::RuntimeModels {
    if let Err(error) =
        crate::agent_extensions::availability::require_enabled_for_project(provider, project)
    {
        return bruhs_models::runtime_models::RuntimeModels {
            models: Vec::new(),
            error: Some(error),
        };
    }
    external::models::list(provider).await
}

// ---------------------------------------------------------------------------
// ThreadStatus
// ---------------------------------------------------------------------------

/// Whether a thread has a turn in flight. A thread is `Streaming` from the
/// moment [`AgentOrchestrator::send_message`] accepts a turn until the
/// provider finishes or the caller drops the turn's event stream, whichever
/// comes first; while it is `Streaming`, another send is refused with
/// [`SendMessageError::ThreadBusy`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadStatus {
    Idle,
    Streaming,
}

// ---------------------------------------------------------------------------
// AgentTurn
// ---------------------------------------------------------------------------

/// One agent turn to run in a thread.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentTurn {
    pub thread_id: ThreadId,
    pub prompt: String,
    /// Working directory the provider CLI runs in.
    pub project_path: String,
    /// Model id passed to the provider CLI (`--model`).
    pub model: String,
    /// Which CLI runs the turn. The caller decides, normally with
    /// [`bruhs_models::agent::provider_for_model_id`]; core never infers it
    /// from the model name.
    pub provider: AgentProvider,
    pub permission_mode: PermissionMode,
    interactive_questions: bool,
    /// Provider session to resume, so earlier turns stay in the model's
    /// context. `None` resumes the session this thread's last turn with the
    /// same provider reported, if there was one.
    pub session_id: Option<String>,
    /// Drop the remembered session and start a new one. A rewind uses this
    /// so the removed turns are not resumed.
    pub fresh_session: bool,
    /// Claude `--effort` or Codex `model_reasoning_effort`. `None` keeps
    /// the provider's configured default.
    pub effort: Option<String>,
    pub model_options: bruhs_models::delivery::generation::GenerationOptions,
    pub kiro_selection: crate::agent_extensions::kiro::Selection,
    /// Claude `--autocompact`. `None` omits the flag. Codex ignores it.
    pub autocompact: Option<String>,
    /// Run with a lean context (see [`AgentTurn::lean`]).
    pub lean: bool,
    /// What the turn's agent may do with the app's other threads.
    pub thread_access: ThreadAccess,
    /// The thread's role instructions, added to its context like a meeting
    /// thread's. Keep them the same for a thread's whole life where
    /// possible: a changed prompt prefix misses the cache.
    pub instructions: Option<String>,
    /// Event handlers never inherit interactive integrations or elevated access.
    pub unattended: bool,
    /// Optional owner-chosen wall-clock budget for this complete turn.
    pub execution_timeout_seconds: Option<u64>,
    /// What an unattended turn may do, at the effective level for
    /// [`AgentTurn::provider`] (see [`AgentTurn::unattended_with`]).
    pub unattended_access: BruhAccess,
    /// The bruh tools this turn is offered on its workbench helper: its
    /// own browser and its app and mail tools (see
    /// [`AgentTurn::bruh_tools`]). Only unattended Claude turns at Ask or
    /// Full use them.
    pub bruh_tools: Option<crate::workbench_tools::BruhTools>,
    /// Folders an unattended Claude turn may never read or edit, in any
    /// tool or its sandboxed shell: the bruhs' browser profiles (see
    /// [`AgentTurn::hide_folders`]).
    pub hidden_folders: Vec<std::path::PathBuf>,
    /// The bruh has a vault key it may use, so an unattended Claude turn at
    /// Ask or Full gets the `request_secret` tool (see
    /// [`AgentTurn::bruh_secrets`]).
    pub bruh_secrets: bool,
}

impl AgentTurn {
    /// A turn that resumes this thread's last known session (see
    /// [`AgentTurn::session_id`]).
    pub fn new(
        thread_id: ThreadId,
        prompt: impl Into<String>,
        project_path: impl Into<String>,
        model: impl Into<String>,
        provider: AgentProvider,
        permission_mode: PermissionMode,
    ) -> Self {
        Self {
            thread_id,
            prompt: prompt.into(),
            project_path: project_path.into(),
            model: model.into(),
            provider,
            permission_mode,
            interactive_questions: false,
            session_id: None,
            fresh_session: false,
            effort: None,
            model_options: Default::default(),
            kiro_selection: Default::default(),
            autocompact: None,
            lean: false,
            thread_access: ThreadAccess::default(),
            instructions: None,
            unattended: false,
            execution_timeout_seconds: None,
            unattended_access: BruhAccess::default(),
            bruh_tools: None,
            hidden_folders: Vec::new(),
            bruh_secrets: false,
        }
    }

    /// The caller can display and answer a structured question for this turn.
    pub fn interactive_questions(mut self, enabled: bool) -> Self {
        self.interactive_questions = enabled;
        self
    }

    /// Bound this turn without changing model, effort or provider permissions.
    /// Missing keeps the existing unlimited-duration behavior.
    pub fn execution_timeout_seconds(mut self, seconds: Option<u64>) -> Self {
        self.execution_timeout_seconds = seconds;
        self
    }

    /// The thread's role instructions. Blank text is dropped.
    pub fn instructions(mut self, instructions: Option<String>) -> Self {
        self.instructions = instructions
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty());
        self
    }

    /// Restrict an event turn independently of the owner's interactive
    /// settings: read only (see [`AgentTurn::unattended_with`]).
    pub fn unattended(self) -> Self {
        self.unattended_with(BruhAccess::default())
    }

    /// Run an event turn at a bruh's access, independently of the owner's
    /// interactive settings: no thread or app tools, a lean context, and the
    /// effective level for this turn's provider (Codex can't ask, so Ask
    /// runs read only there). Read only plans, Ask asks before every change,
    /// Full edits its working folder without asking: Codex through its
    /// workspace-write sandbox, Claude in Default mode, so every edit still
    /// reaches the bruh's permission gate (which allows it inside the
    /// folder, symlinks resolved) instead of the CLI accepting it.
    pub fn unattended_with(mut self, access: BruhAccess) -> Self {
        let level = effective_level(access.level, self.provider);
        self.permission_mode = match level {
            AccessLevel::ReadOnly => PermissionMode::Plan,
            AccessLevel::Ask => PermissionMode::Default,
            AccessLevel::Full if self.provider == AgentProvider::Claude => PermissionMode::Default,
            AccessLevel::Full => PermissionMode::AcceptEdits,
        };
        self.unattended_access = BruhAccess { level, ..access };
        self.unattended = true;
        self.thread_access = ThreadAccess::Off;
        self.lean = true;
        self
    }

    /// Give an unattended turn its bruh's tools: the workbench helper bound
    /// to that bruh, offering its browser, app and mail tools as `tools`
    /// says, and nothing else. The caller decides each with the bruh's
    /// grants and level ([`bruhs_models::crew::computer_on`],
    /// [`bruhs_models::crew::apps_unavailable`]); only an unattended Claude
    /// turn at Ask or Full gets them (`bruh_tool_set`).
    pub fn bruh_tools(mut self, tools: Option<crate::workbench_tools::BruhTools>) -> Self {
        self.bruh_tools = tools;
        self
    }

    /// Keep an unattended Claude turn out of `folders` (absolute paths,
    /// `*` matching one name), whatever its access: Claude's deny rules
    /// cover its Read, Grep, Glob and edit tools, and its bash sandbox reads
    /// the same rules. Codex has no such rule.
    pub fn hide_folders(mut self, folders: Vec<std::path::PathBuf>) -> Self {
        self.hidden_folders = folders;
        self
    }

    /// Offer an unattended Claude turn at Ask or Full the bruh's vault keys
    /// through `request_secret` (D18): each use asks the owner, and only
    /// the approved command's environment gets the value. The caller
    /// decides with the bruh's usable keys; other turns ignore it.
    pub fn bruh_secrets(mut self, on: bool) -> Self {
        self.bruh_secrets = on;
        self
    }

    /// The user's setting for messages between threads.
    pub fn thread_access(mut self, access: ThreadAccess) -> Self {
        self.thread_access = access;
        self
    }

    /// Run a Claude turn without the user's settings files, and so without
    /// the coding plugins, skills and MCP servers they enable; claude.ai
    /// connectors still load. A new Codex session runs without plugins and
    /// the user's config otherwise intact; a resumed Codex session keeps
    /// what it started with (see `codex_lean`). For threads with no
    /// project. Keep it the same for a thread's whole life: a changed prompt
    /// prefix misses the cache. `BRUHS_FULL_CONTEXT=1` turns it off for
    /// Claude turns and new Codex sessions.
    pub fn lean(mut self, lean: bool) -> Self {
        self.lean =
            lean && lean_context_allowed(std::env::var(FULL_CONTEXT_OPT_OUT).ok().as_deref());
        self
    }

    /// Resume `session_id` (the client's copy) instead of the remembered one.
    pub fn resume(mut self, session_id: Option<String>) -> Self {
        self.session_id = session_id;
        self
    }

    /// Keep an effort level shared by Claude and current Codex reasoning
    /// models. Anything outside the app's effort list is dropped.
    pub fn effort(mut self, effort: Option<&str>) -> Self {
        self.effort = effort
            .and_then(bruhs_models::logic::run_place::claude_effort)
            .map(str::to_string);
        self
    }

    /// Keep a Claude auto-compact threshold. Anything outside the CLI range
    /// is dropped, and `None` omits the flag.
    pub fn autocompact(mut self, value: Option<&str>) -> Self {
        self.autocompact = bruhs_models::logic::compact::autocompact_flag(value);
        self
    }

    /// Start a new provider session. The remembered one is forgotten.
    pub fn fresh(mut self) -> Self {
        self.fresh_session = true;
        self.session_id = None;
        self
    }
}

/// Whether `turn` runs with the bruh's vault key tool and its broker: an
/// unattended Claude turn at Ask or Full whose bruh has a usable key. Codex
/// and every other runtime get nothing (D18).
pub(crate) fn bruh_secret_tools(turn: &AgentTurn) -> bool {
    turn.unattended
        && turn.bruh_secrets
        && turn.provider == AgentProvider::Claude
        && matches!(
            turn.unattended_access.level,
            AccessLevel::Ask | AccessLevel::Full
        )
}

/// The bruh tools `turn` runs with: an unattended Claude turn at Ask or
/// Full whose set offers at least one tool. Read only, Codex and every
/// other runtime, and attended turns, get none.
pub(crate) fn bruh_tool_set(turn: &AgentTurn) -> Option<crate::workbench_tools::BruhTools> {
    let usable = turn.unattended
        && turn.provider == AgentProvider::Claude
        && matches!(
            turn.unattended_access.level,
            AccessLevel::Ask | AccessLevel::Full
        );
    turn.bruh_tools
        .clone()
        .filter(|tools| usable && tools.any())
}

/// Why a turn was not started.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SendMessageError {
    #[error("{0}")]
    ProviderUnavailable(String),
    /// The turn's folder is a worktree bruhs removed, and checking its
    /// branch out again failed.
    #[error("{0}")]
    WorktreeUnavailable(String),
    /// The thread already has a turn in flight.
    #[error("thread {0} is already running a turn")]
    ThreadBusy(ThreadId),
    #[error("Choose a positive execution time budget within this host's timer range.")]
    InvalidExecutionBudget,
}

/// The user's answer to a tool-permission prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    /// Run the tool with the input the model proposed.
    Allow,
    /// Refuse; the model sees `reason` (or a generic refusal).
    Deny { reason: Option<String> },
}

// ---------------------------------------------------------------------------
// QueryEvent
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum QueryEvent {
    SecretRequest(bruhs_models::secrets::SecretRequest),
    SecretResolved {
        id: String,
    },
    /// The turn's agent asks about, or writes to, another thread. The app
    /// that owns the threads answers with
    /// [`crate::thread_link::ThreadLinks::resolve`].
    ThreadRequest(bruhs_models::thread_link::ThreadRequest),
    Init {
        session_id: String,
        model: Option<String>,
        tools: Vec<String>,
        mcp_servers: Vec<McpServerInfo>,
    },
    Text(String),
    Thinking(String),
    ToolCall {
        id: String,
        tool: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    Usage(UsageInfo),
    Result {
        result: String,
        session_id: String,
        duration_ms: u64,
        usage: UsageInfo,
        is_error: bool,
    },
    /// The provider is waiting for the user to allow or deny a tool call.
    /// Answer with [`AgentOrchestrator::resolve_permission`].
    PermissionRequest {
        id: String,
        tool: String,
        input: serde_json::Value,
        tool_use_id: Option<String>,
        can_answer: bool,
    },
    /// Files the user submitted with a question answer in this turn.
    QuestionFiles(Vec<bruhs_models::logic::attachments::UploadedFile>),
    /// The provider withdrew a pending permission request.
    PermissionCancelled {
        id: String,
    },
    Error(String),
    Raw(serde_json::Value),
}

/// Tokens a turn spent, plus how full the context window is.
///
/// The four counts are what was billed: per model request on a streaming
/// [`QueryEvent::Usage`], summed over the turn on [`QueryEvent::Result`].
/// `context_tokens` is the window after the latest request; a sum over
/// several requests is not a window size, so a `Result` leaves it `None`
/// and consumers keep the last value they saw.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageInfo {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u64>,
}

/// A turn total (`result.usage`): no window size, see [`UsageInfo`].
impl From<claude_sdk_rs::UsageInfo> for UsageInfo {
    fn from(usage: claude_sdk_rs::UsageInfo) -> Self {
        Self {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_read_input_tokens: usage.cache_read_input_tokens,
            cache_creation_input_tokens: usage.cache_creation_input_tokens,
            context_tokens: None,
        }
    }
}

impl UsageInfo {
    /// One Claude model request (`message_delta`): its input, cache reads
    /// and cache writes are the window the request filled.
    fn claude_request(usage: claude_sdk_rs::UsageInfo) -> Self {
        let mut usage = Self::from(usage);
        usage.context_tokens = Some(bruhs_models::logic::compact::context_tokens(
            usage.input_tokens,
            usage.cache_read_input_tokens,
            usage.cache_creation_input_tokens,
        ));
        usage
    }
}

/// Codex reports cache reads but not cache writes, and its `input_tokens`
/// already includes the cached ones. bruhs follows Claude, where input and
/// cache reads are disjoint, so the cached part is taken out of input.
impl From<codex_sdk_rs::UsageInfo> for UsageInfo {
    fn from(usage: codex_sdk_rs::UsageInfo) -> Self {
        Self {
            input_tokens: usage.input_tokens.saturating_sub(usage.cached_input_tokens),
            output_tokens: usage.output_tokens,
            cache_read_input_tokens: usage.cached_input_tokens,
            cache_creation_input_tokens: 0,
            context_tokens: usage.context_input_tokens,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerInfo {
    pub name: String,
    pub status: String,
}

// ---------------------------------------------------------------------------
// OrchestratorEvent
// ---------------------------------------------------------------------------

/// Wraps QueryEvent with bruhs-specific additions.
#[derive(Debug, Clone)]
pub enum OrchestratorEvent {
    /// Forwarded from claude-sdk-rs.
    Query(QueryEvent),
    /// A background task was registered in the registry.
    BackgroundTaskRegistered(BackgroundTask),
    /// Host deadline; never a synthetic provider completion or a quota reset.
    ExecutionBudgetExceeded { seconds: u64 },
}

// ---------------------------------------------------------------------------
// OrchestratorEventStream
// ---------------------------------------------------------------------------

/// The events of one turn. Dropping it abandons the turn: the provider CLI
/// is stopped and the thread becomes [`ThreadStatus::Idle`] at once.
pub struct OrchestratorEventStream {
    rx: mpsc::Receiver<OrchestratorEvent>,
    _secrets: crate::secrets::SecretGuard,
    _threads: crate::thread_link::ThreadScopeGuard,
    _turn: TurnGuard,
    budget_event: Option<oneshot::Receiver<OrchestratorEvent>>,
    finished: bool,
    pub(crate) scratch_lease: Option<std::fs::File>,
}

impl Stream for OrchestratorEventStream {
    type Item = OrchestratorEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let stream = self.as_mut().get_mut();
        budget::poll_next(
            &mut stream.rx,
            &mut stream.budget_event,
            &mut stream.finished,
            cx,
        )
    }
}

// ---------------------------------------------------------------------------
// ThreadSession
// ---------------------------------------------------------------------------

struct ThreadSession {
    process_id: Option<u32>,
    status: ThreadStatus,
    /// Number of the turn that set `status`; a guard from an older turn
    /// never touches a newer one.
    turn: u64,
    /// Last provider session seen for this thread, and which provider owns
    /// it; the `resume` fallback when the client does not send one.
    session: Option<(AgentProvider, String)>,
}

type Sessions = Arc<Mutex<HashMap<ThreadId, ThreadSession>>>;
type Steers = Arc<Mutex<HashMap<ThreadId, claude_sdk_rs::PermissionResponder>>>;

/// Short, I/O-free critical sections only, so a std mutex is fine here (and
/// lets [`TurnGuard`] release the thread from `Drop`).
fn lock_sessions(sessions: &Sessions) -> MutexGuard<'_, HashMap<ThreadId, ThreadSession>> {
    sessions.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Returns a thread to [`ThreadStatus::Idle`] when dropped, unless a newer
/// turn has started since. One guard lives in the producer task and one in
/// the consumer's stream, so the thread is released on every exit path:
/// the provider finishing or failing to start, the task panicking or being
/// cancelled, or the caller dropping the stream.
struct TurnGuard {
    sessions: Sessions,
    thread_id: ThreadId,
    turn: u64,
}

impl Drop for TurnGuard {
    fn drop(&mut self) {
        let mut sessions = lock_sessions(&self.sessions);
        if let Some(session) = sessions.get_mut(&self.thread_id) {
            if session.turn == self.turn {
                session.status = ThreadStatus::Idle;
                session.process_id = None;
            }
        }
    }
}

/// The turn that owns a pending permission or a background task. Teardown
/// of an abandoned turn touches only what that turn created: the next turn
/// on the same thread may already be running when the old producer task
/// notices it was abandoned.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TurnOwner {
    thread_id: ThreadId,
    turn: u64,
}

/// A permission prompt the CLI is blocked on, keyed by its request id.
/// Generic over the responder only so tests can build one without a CLI.
struct PendingPermission<R = claude_sdk_rs::PermissionResponder> {
    owner: TurnOwner,
    input: serde_json::Value,
    can_answer: bool,
    question_files: Vec<question_files::QuestionFile>,
    events: mpsc::Sender<OrchestratorEvent>,
    responder: R,
}

type PermissionMap<R = claude_sdk_rs::PermissionResponder> = HashMap<String, PendingPermission<R>>;
type Permissions = Arc<RwLock<PermissionMap>>;

/// Drop the prompts `owner` raised: they can no longer be answered once its
/// CLI is gone. Prompts of a newer turn on the same thread are kept.
fn release_turn_permissions<R>(permissions: &mut PermissionMap<R>, owner: &TurnOwner) {
    permissions.retain(|_, pending| pending.owner != *owner);
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PermissionResolveError {
    /// No turn is waiting on a request with that id (already answered,
    /// cancelled, or the turn ended).
    #[error("no pending permission request with that id")]
    UnknownRequest,
    #[error("{0}")]
    Question(&'static str),
    /// The CLI's stdin was gone when the answer was written. The cause is
    /// in the message because HTTP callers only show `Display`.
    #[error("could not deliver the permission decision: {0}")]
    Transport(#[source] claude_sdk_rs::Error),
}

pub(crate) enum ProviderQueryOptions {
    Claude(claude_sdk_rs::QueryOptions),
    Codex(codex_sdk_rs::QueryOptions),
    External(external::Options),
}

impl ProviderQueryOptions {
    fn supports_mcp(&self) -> bool {
        match self {
            Self::External(options) => options.supports_mcp(),
            Self::Claude(_) | Self::Codex(_) => true,
        }
    }
}

// ---------------------------------------------------------------------------
// AgentOrchestrator
// ---------------------------------------------------------------------------

pub struct AgentOrchestrator {
    secrets: crate::secrets::SecretManager,
    thread_links: crate::thread_link::ThreadLinks,
    registry: Arc<BackgroundTaskRegistry>,
    sessions: Sessions,
    permissions: Permissions,
    /// Claude stdin for the turn that is streaming. Cleared when that turn's
    /// stream ends, including when the caller drops it.
    steers: Steers,
}

/// Drops the steer handle for one turn. Held by the Claude producer so a
/// late write cannot hit a closed stdin.
struct SteerGuard {
    steers: Steers,
    thread_id: ThreadId,
}

impl Drop for SteerGuard {
    fn drop(&mut self) {
        self.steers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.thread_id);
    }
}

/// Why a message could not be written into the running turn.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SteerError {
    /// No Claude turn is streaming for this thread.
    #[error("no live turn can take another message")]
    NoTurn,
    /// The CLI's stdin rejected the write.
    #[error("could not deliver the message: {0}")]
    Transport(String),
}

/// Reason the model sees when the user denies a tool without giving one.
const DEFAULT_DENY_REASON: &str = "Denied by user";

impl AgentOrchestrator {
    pub fn new(registry: Arc<BackgroundTaskRegistry>) -> Self {
        Self {
            secrets: crate::secrets::SecretManager::default(),
            thread_links: crate::thread_link::ThreadLinks::default(),
            registry,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            permissions: Arc::new(RwLock::new(HashMap::new())),
            steers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn secrets(&self) -> &crate::secrets::SecretManager {
        &self.secrets
    }

    /// Pending requests from turns about other threads.
    pub fn thread_links(&self) -> &crate::thread_link::ThreadLinks {
        &self.thread_links
    }

    pub(crate) fn with_connections(mut self, connections: crate::connections::Connections) -> Self {
        self.secrets = self.secrets.with_connections(connections);
        self
    }

    pub(crate) fn with_thread_waits(mut self, waits: bool) -> Self {
        self.thread_links = self.thread_links.with_waits(waits);
        self
    }

    pub(crate) fn with_secret_store(mut self, store: crate::secrets::SecretStore) -> Self {
        self.secrets = crate::secrets::SecretManager::new(store);
        self
    }

    pub(crate) fn with_one_password(mut self, one_password: crate::secrets::OnePassword) -> Self {
        self.secrets = self.secrets.with_one_password(one_password);
        self
    }

    /// Write `prompt` into the Claude turn that is streaming for `thread_id`.
    pub async fn steer(&self, thread_id: &ThreadId, prompt: &str) -> Result<(), SteerError> {
        let responder = self
            .steers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(thread_id)
            .cloned()
            .ok_or(SteerError::NoTurn)?;
        responder
            .push_user_message(prompt)
            .await
            .map_err(|error| SteerError::Transport(error.to_string()))
    }

    /// Answer a tool-permission prompt raised by a running Claude turn.
    /// Allowing re-sends the model's proposed input unchanged.
    pub async fn resolve_permission(
        &self,
        request_id: &str,
        decision: PermissionDecision,
    ) -> Result<(), PermissionResolveError> {
        if external::acp::resolve(request_id, &decision) {
            return Ok(());
        }
        let pending = {
            let mut requests = self.permissions.write().await;
            if requests
                .get(request_id)
                .is_some_and(|request| request.can_answer)
            {
                return Err(PermissionResolveError::Question(
                    "Send an answer to this question.",
                ));
            }
            requests
                .remove(request_id)
                .ok_or(PermissionResolveError::UnknownRequest)?
        };
        let decision = match decision {
            PermissionDecision::Allow => claude_sdk_rs::PermissionDecision::Allow {
                updated_input: Some(pending.input),
            },
            PermissionDecision::Deny { reason } => claude_sdk_rs::PermissionDecision::Deny {
                message: reason.unwrap_or_else(|| DEFAULT_DENY_REASON.to_string()),
            },
        };
        pending
            .responder
            .respond(request_id, decision)
            .await
            .map_err(PermissionResolveError::Transport)
    }

    /// Answer the exact structured question waiting on this thread's Claude turn.
    pub async fn answer_question(
        &self,
        thread_id: &ThreadId,
        request_id: &str,
        reply: bruhs_models::logic::user_question::QuestionReply,
    ) -> Result<(), PermissionResolveError> {
        self.answer_question_from(thread_id, request_id, reply, None)
            .await
    }

    pub(crate) async fn answer_question_from(
        &self,
        thread_id: &ThreadId,
        request_id: &str,
        reply: bruhs_models::logic::user_question::QuestionReply,
        device: Option<uuid::Uuid>,
    ) -> Result<(), PermissionResolveError> {
        use bruhs_models::logic::user_question::{answered_input, QuestionReply};
        use claude_sdk_rs::PermissionDecision::{Allow, Deny};

        let (mut pending, decision, selected) = {
            let mut requests = self.permissions.write().await;
            let request = requests
                .get(request_id)
                .filter(|pending| &pending.owner.thread_id == thread_id && pending.can_answer)
                .ok_or(PermissionResolveError::UnknownRequest)?;
            let (decision, selected) = match reply {
                QuestionReply::Answer { answers } => {
                    let input = answered_input(&request.input, &answers, |question, ids| {
                        request.files_for(question, ids, device)
                    })
                    .map_err(PermissionResolveError::Question)?;
                    let selected = answers
                        .into_iter()
                        .flat_map(|answer| answer.attachments)
                        .collect::<Vec<_>>();
                    (
                        Allow {
                            updated_input: Some(input),
                        },
                        selected,
                    )
                }
                QuestionReply::Skip => (
                    Deny {
                        message: "Question skipped by user".into(),
                    },
                    Vec::new(),
                ),
            };
            let pending = requests
                .remove(request_id)
                .ok_or(PermissionResolveError::UnknownRequest)?;
            (pending, decision, selected)
        };
        if !selected.is_empty() {
            let files = pending.selected_files(&selected);
            // Publish before waking the provider, which can finish the turn
            // as soon as it accepts the answer. The transcript owns the files
            // once it can see them, even when the provider fails to continue.
            pending
                .events
                .send(OrchestratorEvent::Query(QueryEvent::QuestionFiles(files)))
                .await
                .map_err(|_| PermissionResolveError::UnknownRequest)?;
            pending.preserve_files(&selected);
        }
        pending
            .responder
            .respond(request_id, decision)
            .await
            .map_err(PermissionResolveError::Transport)?;
        Ok(())
    }

    /// Ids of permission prompts a thread is currently blocked on.
    pub async fn pending_permissions(&self, thread_id: &ThreadId) -> Vec<String> {
        let mut ids: Vec<String> = self
            .permissions
            .read()
            .await
            .iter()
            .filter(|(_, p)| &p.owner.thread_id == thread_id)
            .map(|(id, _)| id.clone())
            .collect();
        ids.extend(external::acp::pending(thread_id.as_str()));
        ids
    }

    /// Start `turn`: spawn its provider CLI and return the turn's events.
    /// `system_prompt` is the thread's extra context: appended to Claude
    /// Code's system prompt, and Codex's developer instructions.
    ///
    /// # Errors
    ///
    /// [`SendMessageError::ThreadBusy`] when the thread already has a turn
    /// in flight; nothing is started.
    ///
    /// # Panics
    ///
    /// Outside a tokio runtime, like `tokio::spawn` (the turn runs on a
    /// spawned task). [`crate::AppState::send_message`] is the async entry.
    pub fn send_message(
        &self,
        turn: AgentTurn,
        system_prompt: Option<&str>,
    ) -> Result<OrchestratorEventStream, SendMessageError> {
        let bruh_keys = bruh_secret_tools(&turn);
        let bruh_tools = bruh_tool_set(&turn);
        let AgentTurn {
            thread_id,
            prompt,
            project_path,
            model,
            provider,
            permission_mode,
            interactive_questions,
            session_id,
            fresh_session,
            model_options,
            kiro_selection,
            effort,
            autocompact,
            lean,
            thread_access,
            instructions,
            unattended,
            execution_timeout_seconds,
            unattended_access,
            bruh_tools: _,
            hidden_folders,
            bruh_secrets: _,
        } = turn;
        crate::agent_extensions::availability::require_enabled_for_project(
            provider,
            project_path.as_ref(),
        )
        .map_err(SendMessageError::ProviderUnavailable)?;
        let deadline = budget::deadline(execution_timeout_seconds)?;
        // The thread's own context first (a meeting's transcript), then what
        // its role asks of it.
        let context = match (system_prompt, instructions.as_deref()) {
            (Some(prompt), Some(instructions)) => Some(format!("{prompt}\n\n{instructions}")),
            (Some(prompt), None) => Some(prompt.to_string()),
            (None, Some(instructions)) => Some(instructions.to_string()),
            (None, None) => None,
        };
        let system_prompt = context.as_deref();

        // Claim the thread and pick the session to resume, atomically.
        reconcile_prompt_completions(&self.registry, thread_id.as_str(), &prompt);
        let (turn_number, resume) = {
            let mut sessions = lock_sessions(&self.sessions);
            let session = sessions
                .entry(thread_id.clone())
                .or_insert_with(|| ThreadSession {
                    process_id: None,
                    status: ThreadStatus::Idle,
                    turn: 0,
                    session: None,
                });
            if session.status == ThreadStatus::Streaming {
                return Err(SendMessageError::ThreadBusy(thread_id));
            }
            session.status = ThreadStatus::Streaming;
            session.turn += 1;
            if fresh_session {
                session.session = None;
            }
            let remembered =
                (!fresh_session)
                    .then(|| {
                        session
                            .session
                            .as_ref()
                            .filter(|(owner, id)| {
                                *owner == provider && (provider != AgentProvider::Acp ||
                            bruhs_models::agent_extensions::acp_session_matches_model(id, &model))
                            })
                            .map(|(_, id)| id.clone())
                    })
                    .flatten();
            let claimed = (
                session.turn,
                session_to_resume(fresh_session, session_id, remembered),
            );
            drop(sessions);
            claimed
        };
        let guard = || TurnGuard {
            sessions: Arc::clone(&self.sessions),
            thread_id: thread_id.clone(),
            turn: turn_number,
        };
        let (consumer_guard, producer_guard) = (guard(), guard());

        let mut query_options = build_query_options(
            provider,
            &prompt,
            &project_path,
            &model,
            permission_mode,
            system_prompt,
            resume.as_deref(),
        );
        apply_effort(&mut query_options, effort.as_deref());
        if let ProviderQueryOptions::External(options) = &mut query_options {
            options.kiro_selection = kiro_selection;
        }
        apply_claude_autocompact(&mut query_options, autocompact.as_deref());
        // A Codex session keeps the plugin setting it started with.
        let lean_file = crate::codex_lean::default_file();
        let lean = match provider {
            AgentProvider::Claude => lean,
            AgentProvider::Codex => {
                crate::codex_lean::turn_is_lean(lean_file.as_deref(), resume.as_deref(), lean)
            }
            _ => false,
        };
        let record_lean = (provider == AgentProvider::Codex && lean && resume.is_none())
            .then_some(lean_file)
            .flatten();
        apply_lean_context(&mut query_options, lean);
        apply_workspace_trust(&mut query_options, &project_path, unattended);
        model_options::apply(&mut query_options, &model_options);
        // Only an unattended turn gets a bruh's tools.
        let bruh_helper = if unattended {
            restrict_bruh_turn(
                &mut query_options,
                &unattended_access,
                bruh_tools.as_ref(),
                &hidden_folders,
                bruh_keys,
            )
        } else {
            None
        };

        let artifact_project = project_path.clone();
        let (tx, rx) = mpsc::channel::<OrchestratorEvent>(256);
        let secrets = self.secrets.turn(
            thread_id.clone(),
            project_path.into(),
            tx.clone(),
            unattended,
        );
        let broker_folder = self.secrets.broker_folder();
        let consumer_secrets = secrets.guard();
        let producer_secrets = secrets.guard();
        let threads = self.thread_links.turn(
            thread_id.clone(),
            &tx,
            permission_mode == PermissionMode::BypassPermissions,
        );
        let consumer_threads = threads.guard();
        let producer_threads = threads.guard();
        // A Plan turn changes nothing, and Codex cannot ask first, so both
        // only read threads; so does a turn the user limited to reading.
        let threads_read_only = thread_access == ThreadAccess::Read
            || permission_mode == PermissionMode::Plan
            || (!matches!(provider, AgentProvider::Claude | AgentProvider::Acp)
                && permission_mode == PermissionMode::Default);
        let turn = TurnContext {
            model: model.clone(),
            requested_speed: model_options::requested_speed(provider, &model_options),
            usage_receipt: uuid::Uuid::new_v4().to_string(),
            media_project: artifact_project.clone(),
            response_text: Mutex::new(String::new()),
            _guard: producer_guard,
            tx,
            registry: Arc::clone(&self.registry),
            sessions: Arc::clone(&self.sessions),
            owner: TurnOwner {
                thread_id,
                turn: turn_number,
            },
            provider,
            tasks: Mutex::new(Vec::new()),
            tool_tasks: Mutex::new(HashMap::new()),
            status_checks: Mutex::new(std::collections::HashSet::new()),
            pending_commands: Mutex::new(HashMap::new()),
            task_status_checks: Mutex::new(HashMap::new()),
        };
        let permissions = Arc::clone(&self.permissions);
        let steers = Arc::clone(&self.steers);

        let (budget_sender, budget_event) = if deadline.is_some() {
            let (sender, receiver) = oneshot::channel();
            (Some(sender), Some(receiver))
        } else {
            (None, None)
        };
        let secret_model = model.clone();
        let budget_owner = turn.owner.clone();
        let budget_permissions = Arc::clone(&permissions);
        tokio::spawn(async move {
            let work = async move {
                let _secrets = producer_secrets;
                let _threads = producer_threads;
                if let Some(Err(error)) = bruh_helper {
                    turn.send_start_error(bruhs_models::crew::bruh_tools_failed(
                        &error.to_string(),
                    ))
                    .await;
                    return;
                }
                if !unattended {
                    match crate::secret_sources::for_turn(provider, &secret_model).await {
                        Ok(vars) => match &mut query_options {
                            ProviderQueryOptions::Claude(options) => options.env.extend(vars),
                            ProviderQueryOptions::Codex(options) => options.env.extend(vars),
                            ProviderQueryOptions::External(options) => options.secret_env = vars,
                        },
                        Err(error) => {
                            turn.send_start_error(format!("Could not start the provider. {error}"))
                                .await;
                            return;
                        }
                    }
                }
                let app_tools = query_options.supports_mcp();
                // A bruh's broker starts after its tools server is in place,
                // and joins it (`configure_bruh`).
                let mut broker = match if bruh_keys || (!unattended && app_tools) {
                    secrets.start().await.map(Some)
                } else {
                    Ok(None)
                } {
                    Ok(broker) => broker,
                    Err(_) => {
                        turn.send_start_error(
                            "Could not start the private credential broker. Retry the turn.".into(),
                        )
                        .await;
                        return;
                    }
                };
                if bruh_keys {
                    let configured = match broker.as_mut() {
                        Some(broker) => broker_folder
                            .and_then(|folder| broker.configure_bruh(&mut query_options, &folder)),
                        None => Err(std::io::Error::other("no broker")),
                    };
                    if configured.is_err() {
                        turn.send_start_error(
                            "Could not configure private credential tools. Retry the turn.".into(),
                        )
                        .await;
                        return;
                    }
                } else if !unattended && permission_mode == PermissionMode::Plan {
                    if let ProviderQueryOptions::Claude(options) = &mut query_options {
                        crate::secrets::mcp::append_claude_instructions(options);
                    }
                } else if broker
                    .as_ref()
                    .is_some_and(|broker| broker.configure(&mut query_options).is_err())
                {
                    turn.send_start_error(
                        "Could not configure private credential tools. Retry the turn.".into(),
                    )
                    .await;
                    return;
                }
                // Off in Settings: no thread tools. The turn also runs without
                // them when the broker cannot start.
                let _thread_broker =
                    if unattended || !app_tools || thread_access == ThreadAccess::Off {
                        None
                    } else {
                        match threads.start(threads_read_only).await {
                            Ok(broker) => match broker.configure(&mut query_options) {
                                Ok(()) => Some(broker),
                                Err(error) => {
                                    tracing::warn!("Thread tools could not be configured: {error}");
                                    None
                                }
                            },
                            Err(error) => {
                                tracing::warn!("Thread broker could not start: {error}");
                                None
                            }
                        }
                    };
                if let Err(error) = if unattended || !app_tools {
                    Ok(())
                } else {
                    crate::pith::configure_turn(&mut query_options).await
                } {
                    turn.send_start_error(format!("Could not connect Pith: {error}"))
                        .await;
                    return;
                }
                if !unattended && app_tools {
                    if let Err(error) = crate::website_reader::configure(&mut query_options) {
                        turn.send_start_error(format!(
                            "Could not configure website reading: {error}"
                        ))
                        .await;
                        return;
                    }
                    if let Err(error) =
                        crate::artifact_tools::configure(&mut query_options, &artifact_project)
                    {
                        turn.send_start_error(format!(
                            "Could not configure local artifact tools: {error}"
                        ))
                        .await;
                        return;
                    }
                    if let Err(error) =
                        crate::database_tools::configure(&mut query_options, &artifact_project)
                    {
                        turn.send_start_error(format!(
                            "Could not configure database tools: {error}"
                        ))
                        .await;
                        return;
                    }
                    if let Err(error) = crate::api_client_tools::configure(
                        &mut query_options,
                        &artifact_project,
                        turn.owner.thread_id.as_str(),
                        permission_mode != PermissionMode::Plan,
                    ) {
                        turn.send_start_error(format!(
                            "Could not configure API request tools: {error}"
                        ))
                        .await;
                        return;
                    }
                    if let Err(error) = crate::reactive_tools::configure(
                        &mut query_options,
                        permission_mode == PermissionMode::Plan,
                    ) {
                        turn.send_start_error(format!(
                            "Could not configure reactive automation tools: {error}"
                        ))
                        .await;
                        return;
                    }
                }
                if !unattended && app_tools {
                    if let Err(error) = crate::workbench_tools::configure(
                        &mut query_options,
                        &artifact_project,
                        turn.owner.thread_id.as_str(),
                        threads_read_only,
                        permission_mode,
                    ) {
                        turn.send_start_error(format!(
                            "Could not configure workbench tools: {error}"
                        ))
                        .await;
                        return;
                    }
                }
                match query_options {
                    ProviderQueryOptions::Claude(options) => {
                        stream_claude(
                            options,
                            turn,
                            permissions,
                            steers,
                            interactive_questions && !unattended,
                        )
                        .await;
                    }
                    ProviderQueryOptions::Codex(options) => {
                        stream_codex(options, turn, model, record_lean).await;
                    }
                    ProviderQueryOptions::External(options) => {
                        external::stream(options, turn).await
                    }
                }
            };
            match deadline {
                None => work.await,
                Some((at, seconds)) => {
                    if tokio::time::timeout_at(at, work).await.is_err() {
                        // Dropping work stops the owned provider and brokers.
                        // This path releases only the expired turn's prompts.
                        release_turn_permissions(
                            &mut *budget_permissions.write().await,
                            &budget_owner,
                        );
                        if let Some(sender) = budget_sender {
                            let _ =
                                sender.send(OrchestratorEvent::ExecutionBudgetExceeded { seconds });
                        }
                    }
                }
            }
        });

        Ok(OrchestratorEventStream {
            scratch_lease: None,
            rx,
            _secrets: consumer_secrets,
            _threads: consumer_threads,
            _turn: consumer_guard,
            budget_event,
            finished: false,
        })
    }

    /// Active local CLI, identified by turn generation and host PID.
    pub fn thread_process(&self, thread_id: &ThreadId) -> Option<(u64, u32)> {
        let sessions = lock_sessions(&self.sessions);
        let result = sessions
            .get(thread_id)
            .and_then(|session| Some((session.turn, session.process_id?)));
        drop(sessions);
        result
    }

    /// Whether `thread_id` has a turn in flight.
    pub fn thread_status(&self, thread_id: &ThreadId) -> ThreadStatus {
        lock_sessions(&self.sessions)
            .get(thread_id)
            .map_or(ThreadStatus::Idle, |s| s.status)
    }
}

/// Pending command call waiting for its `ToolResult` to see whether it spilled
/// into an asynchronous background task.
struct PendingCommand {
    label: String,
    command: String,
    is_daemon: bool,
    registered_task_id: Option<String>,
}

/// What a producer task needs to forward one turn's events.
struct TurnContext {
    model: String,
    requested_speed: bruhs_models::agent_extensions::UsageSpeed,
    usage_receipt: String,
    media_project: String,
    response_text: Mutex<String>,
    /// Declared before `tx`: by the time the caller's
    /// stream reports its end, the thread is already idle.
    _guard: TurnGuard,
    tx: mpsc::Sender<OrchestratorEvent>,
    registry: Arc<BackgroundTaskRegistry>,
    sessions: Sessions,
    owner: TurnOwner,
    provider: AgentProvider,
    /// Ids of the background tasks this turn registered; only these are
    /// settled when the turn ends.
    tasks: Mutex<Vec<String>>,
    /// Maps a tool call id to the background/subagent/swarm task ids it started.
    tool_tasks: Mutex<HashMap<String, Vec<String>>>,
    /// Tool call ids for `swarm` `status` queries, reconciled on `ToolResult`.
    status_checks: Mutex<std::collections::HashSet<String>>,
    /// Active `run_command` calls waiting to see if they backgrounded on `ToolResult`.
    pending_commands: Mutex<HashMap<String, PendingCommand>>,
    /// Tool call ids for `manage_task` `status` checks (`tool_call_id -> external_task_id`).
    task_status_checks: Mutex<HashMap<String, String>>,
}

/// `fresh` forgets the remembered session. Otherwise the client's id wins,
/// and a missing id resumes `remembered`.
fn session_to_resume(
    fresh: bool,
    requested: Option<String>,
    remembered: Option<String>,
) -> Option<String> {
    if fresh {
        None
    } else {
        requested.or(remembered)
    }
}

/// Apply an explicit effort without replacing the user's configured default
/// when the app did not select one. Other providers keep their own controls.
fn apply_effort(options: &mut ProviderQueryOptions, effort: Option<&str>) {
    let Some(level) = effort.and_then(bruhs_models::logic::run_place::claude_effort) else {
        return;
    };
    match options {
        ProviderQueryOptions::Claude(options) => options.effort = Some(level.to_string()),
        ProviderQueryOptions::Codex(options) => options
            .config
            .push(codex_sdk_rs::ConfigOverride::reasoning_effort(level)),
        ProviderQueryOptions::External(_) => {}
    }
}

/// Set by the user to give every thread their full Claude settings.
const FULL_CONTEXT_OPT_OUT: &str = "BRUHS_FULL_CONTEXT";

/// Whether lean turns are allowed, given `BRUHS_FULL_CONTEXT`'s value.
fn lean_context_allowed(opt_out: Option<&str>) -> bool {
    opt_out != Some("1")
}

/// Load no Claude settings files on a lean turn, but keep the user's
/// permission rules and hooks (see [`lean_claude_settings`]); run Codex
/// without its plugins (see [`lean_codex`]).
fn apply_lean_context(options: &mut ProviderQueryOptions, lean: bool) {
    if !lean {
        return;
    }
    match options {
        ProviderQueryOptions::Claude(options) => {
            lean_claude_settings(options, &user_claude_settings());
        }
        ProviderQueryOptions::Codex(options) => lean_codex(options),
        ProviderQueryOptions::External(_) => {}
    }
}

/// Isolate untrusted project repositories so headless agent CLIs cannot run
/// repository-local hooks, `apiKeyHelper`, `statusLine`, `.mcp.json`, or
/// project-local OpenCode/external configs before the user trusts the folder.
pub(crate) fn apply_workspace_trust(
    options: &mut ProviderQueryOptions,
    project_path: &str,
    unattended: bool,
) {
    if unattended {
        return;
    }
    let trusted = crate::workspace_trust::is_trusted(std::path::Path::new(project_path));
    apply_workspace_trust_flag(options, trusted);
}

pub(crate) fn apply_workspace_trust_flag(options: &mut ProviderQueryOptions, trusted: bool) {
    match options {
        ProviderQueryOptions::Claude(options) => {
            if !trusted {
                // Keep the user's own `~/.claude` settings when not already lean
                // (`Some([])`), while excluding project `.claude/settings*.json`.
                if options.setting_sources.is_none() {
                    options.setting_sources = Some(vec!["user".to_string()]);
                }
                // Never auto-load `.mcp.json` from an untrusted repository.
                options.strict_mcp_config = true;
            }
        }
        ProviderQueryOptions::Codex(_) => {}
        ProviderQueryOptions::External(options) => {
            options.workspace_trusted = trusted;
        }
    }
}

/// Make a Codex run lean: no plugins (`features.plugins=false`), which put
/// their tools and skill text in every request. Everything else in the
/// user's config stays: model, provider, reasoning effort, MCP servers
/// (deferred behind tool search, so they cost nothing up front) and ChatGPT
/// apps. Measured on a trivial turn (codex-cli 0.166): 24.0k → 17.9k
/// context tokens (−25.6%); `--ignore-user-config` reached only 22.8k,
/// because the built-in plugins load without it.
pub(crate) fn lean_codex(options: &mut codex_sdk_rs::QueryOptions) {
    let already = options
        .config
        .iter()
        .any(|entry| entry.key() == CODEX_PLUGINS_FEATURE);
    if !already {
        if let Ok(entry) = codex_sdk_rs::ConfigOverride::new(CODEX_PLUGINS_FEATURE, "false") {
            options.config.push(entry);
        }
    }
}

/// The Codex feature flag that loads plugins.
const CODEX_PLUGINS_FEATURE: &str = "features.plugins";

/// Tools an unattended run that may write gets without asking, so reads
/// never wait on the owner. Read only runs get `Tools::ReadOnly` instead.
const UNATTENDED_READ_TOOLS: [&str; 4] = ["Read", "Grep", "Glob", "LS"];

/// Code hosts a Full bruh's sandboxed shell may reach only when it can
/// deliver (push, merge, open pull requests); without Can deliver they are
/// refused outright rather than asked about.
const FORGE_HOSTS: [&str; 4] = ["github.com", "*.github.com", "gitlab.com", "*.gitlab.com"];

/// Whether an unattended turn at `access` on `provider` runs its shell in
/// Claude's bash sandbox, confined to the working folder (see
/// [`restrict_unattended`]). Only then may the permission gate let a Full
/// bruh's shell commands run without asking.
pub(crate) fn unattended_shell_confined(access: &BruhAccess, provider: AgentProvider) -> bool {
    provider == AgentProvider::Claude
        && effective_level(access.level, provider) == AccessLevel::Full
}

/// `--settings` for an unattended Claude turn at `access`: the deny rules
/// only (app tools, the user's own deny rules from `files`, and delivery
/// unless the gate allows it), plus the bash sandbox when the shell runs
/// without asking. Hooks and allow rules stay out: they could turn an
/// outside event into host-side actions before the gate runs.
fn unattended_claude_settings(
    access: &BruhAccess,
    files: &[serde_json::Value],
    servers: bool,
    hidden: &[std::path::PathBuf],
    secrets: bool,
) -> serde_json::Value {
    // With `servers`, the bruh's MCP servers are its workbench helper,
    // with its browser and app and mail tools (see
    // `workbench_tools::configure_bruh`), and, with `secrets`, its vault
    // keys' broker (`SecretBroker::configure_bruh`); each call reaches the
    // gate, which refuses every other app tool. Service connections are
    // never a bruh's (F27).
    let mut deny = if servers || secrets {
        Vec::new()
    } else {
        vec![serde_json::json!("mcp__*")]
    };
    if secrets {
        deny.push(serde_json::json!(crate::secrets::mcp::BRUH_CONNECTION_TOOL));
    }
    for file in files {
        deny.extend(
            file["permissions"]["deny"]
                .as_array()
                .into_iter()
                .flatten()
                .cloned(),
        );
    }
    deny.extend(
        claude_deny_rules(access, access.level)
            .into_iter()
            .map(serde_json::Value::from),
    );
    deny.extend(hidden_folder_rules(hidden).map(serde_json::Value::from));
    let mut settings = serde_json::json!({"permissions": {"deny": deny}});
    if unattended_shell_confined(access, AgentProvider::Claude) {
        // Writes outside the working folder fail and network hosts become
        // permission requests. Fail closed where the sandbox can't start,
        // never fall back to an unsandboxed command, and send every command
        // through the gate, which also catches the delivery forms the
        // prefix deny rules miss (`git -C dir push`).
        let hosts = if gate(access, access.level, GatedAction::Delivery) == Gate::Allow {
            "allowedDomains"
        } else {
            "deniedDomains"
        };
        settings["sandbox"] = serde_json::json!({
            "enabled": true,
            "failIfUnavailable": true,
            "allowUnsandboxedCommands": false,
            "autoAllowBashIfSandboxed": false,
            "network": {hosts: FORGE_HOSTS},
        });
    }
    settings
}

/// Deny rules that keep a turn out of each of `folders`. `Read` rules also
/// cover Grep and Glob, and Claude's sandbox adds `Read` and `Edit` deny
/// paths to its own read and write blocks. A leading `//` makes a rule's
/// path absolute.
pub(crate) fn hidden_folder_rules(
    folders: &[std::path::PathBuf],
) -> impl Iterator<Item = String> + '_ {
    folders.iter().flat_map(|folder| {
        let folder = folder.to_string_lossy();
        let folder = folder.trim_end_matches('/');
        ["Read", "Edit"].map(|tool| format!("{tool}(/{folder}/**)"))
    })
}

/// What an unattended turn's environment keeps, read with `get`: logins,
/// locale, temp folders, certificates and the owner's proxies (both cases,
/// as tools differ on which they read). Nothing else, so event content
/// can't reach the daemon's integration tokens through a shell.
pub(crate) fn unattended_environment(
    get: impl Fn(&str) -> Option<String>,
) -> Vec<(String, String)> {
    [
        "PATH",
        "HOME",
        // Claude's macOS keychain lookup uses USER to locate the signed-in account.
        "USER",
        "USERNAME",
        "USERPROFILE",
        "LANG",
        "LC_ALL",
        "CODEX_HOME",
        "CLAUDE_CONFIG_DIR",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "TMPDIR",
        "TEMP",
        "TMP",
        "SHELL",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "NODE_EXTRA_CA_CERTS",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "NO_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "no_proxy",
        "all_proxy",
        "SystemRoot",
        "WINDIR",
    ]
    .into_iter()
    .filter_map(|key| get(key).map(|value| (key.to_string(), value)))
    .collect()
}

/// Limit an unattended turn to `access` ([`restrict_unattended`]) and, with
/// `tools` (from [`bruh_tool_set`]), add the bruh's workbench helper as its
/// one server: `Some` with how that went, `None` when no helper is wanted.
/// The `mcp__*` deny is lifted only when a server is added.
fn restrict_bruh_turn(
    options: &mut ProviderQueryOptions,
    access: &BruhAccess,
    tools: Option<&crate::workbench_tools::BruhTools>,
    hidden: &[std::path::PathBuf],
    secrets: bool,
) -> Option<std::io::Result<()>> {
    restrict_unattended(options, access, tools.is_some(), hidden, secrets);
    tools.map(|tools| crate::workbench_tools::configure_bruh(options, tools))
}

/// Limit an unattended turn to `access` (already the effective level for
/// its provider): a filtered environment, no settings files, hooks or MCP
/// servers, and the tools, permission mode and sandbox for the level.
/// `servers` lets app tools through the deny rules, for the bruh's
/// workbench helper `workbench_tools::configure_bruh` adds next; `secrets`
/// does too, for the vault key broker, but never its connection tool.
fn restrict_unattended(
    options: &mut ProviderQueryOptions,
    access: &BruhAccess,
    servers: bool,
    hidden: &[std::path::PathBuf],
    secrets: bool,
) {
    // Event content must not gain access to the daemon's integration tokens
    // through a shell's environment. Provider logins remain on disk.
    let environment = unattended_environment(|key| std::env::var(key).ok());
    match options {
        ProviderQueryOptions::Claude(options) => {
            options.inherit_env = false;
            options.env = environment;
            options
                .env
                .extend(CLI_TELEMETRY_OPT_OUT.map(|(key, value)| (key.into(), value.into())));
            let read_tools = || {
                claude_sdk_rs::Tools::Allow(
                    UNATTENDED_READ_TOOLS
                        .iter()
                        .map(|t| t.to_string())
                        .collect(),
                )
            };
            let (tools, mode) = match access.level {
                AccessLevel::ReadOnly => (
                    claude_sdk_rs::Tools::ReadOnly,
                    claude_sdk_rs::PermissionMode::Plan,
                ),
                AccessLevel::Ask => (read_tools(), claude_sdk_rs::PermissionMode::Default),
                // Not Accept Edits: the CLI would accept in-folder edits
                // itself, and the gate's symlink check would never run.
                AccessLevel::Full => (read_tools(), claude_sdk_rs::PermissionMode::Default),
            };
            // Every ask reaches the run's permission handler, never a
            // silent denial (a turn built for bypass would have them off).
            options.permission_prompts = true;
            options.tools = tools;
            options.permission_mode = Some(mode);
            options.setting_sources = Some(Vec::new());
            options.settings = Some(
                unattended_claude_settings(
                    access,
                    &user_claude_settings(),
                    servers,
                    hidden,
                    secrets,
                )
                .to_string(),
            );
            // Start no MCP servers: not the user's, claude.ai connectors, or a
            // `.mcp.json` a Full bruh wrote into its folder, whose command
            // would run outside the shell sandbox. The bruh's workbench
            // helper and its vault key broker are the only ones added after
            // this.
            options.mcp_config = None;
            options.strict_mcp_config = true;
        }
        ProviderQueryOptions::Codex(options) => {
            options.inherit_env = false;
            options.env = environment;
            options.ignore_user_config = true;
            // Codex can't ask, so only Full writes, and only in its
            // workspace; workspace-write keeps the network off, so it can't
            // push either.
            let sandbox = match access.level {
                AccessLevel::Full => codex_sdk_rs::SandboxMode::WorkspaceWrite,
                AccessLevel::ReadOnly | AccessLevel::Ask => codex_sdk_rs::SandboxMode::ReadOnly,
            };
            options.access = codex_sdk_rs::CodexAccess::Sandboxed {
                sandbox,
                approval: codex_sdk_rs::ApprovalPolicy::Never,
            };
            options.enable_web_search = false;
            options.config.clear();
            options
                .config
                .push(codex_sdk_rs::ConfigOverride::disable_analytics());
            for (key, value) in [
                ("features.plugins", "false"),
                ("features.apps", "false"),
                ("mcp_servers", "{}"),
                ("web_search", "\"disabled\""),
            ] {
                // These are fixed valid TOML paths, checked by the unit test.
                if let Ok(entry) = codex_sdk_rs::ConfigOverride::new(key, value) {
                    options.config.push(entry);
                }
            }
        }
        ProviderQueryOptions::External(options) => options.unattended(),
    }
}

/// Make `options` lean: no settings files, so none of the plugins, skills
/// or MCP servers they enable, with the safety parts of `files` (the user's
/// settings) passed back through `--settings`. Skipping the files would
/// otherwise also drop `permissions.deny`, which Claude enforces even in
/// bypass mode, and guard hooks. `defaultMode` stays out: bruhs's permission
/// mode decides.
pub(crate) fn lean_claude_settings(
    options: &mut claude_sdk_rs::QueryOptions,
    files: &[serde_json::Value],
) {
    options.setting_sources = Some(Vec::new());
    options.settings = safety_settings(files);
}

/// The user's Claude settings files, as the CLI would read them for a turn
/// in the home directory: `settings.json` and `settings.local.json` under
/// `$CLAUDE_CONFIG_DIR` (default `~/.claude`). Unreadable files are skipped.
pub(crate) fn user_claude_settings() -> Vec<serde_json::Value> {
    let dir = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| directories::BaseDirs::new().map(|dirs| dirs.home_dir().join(".claude")));
    dir.map_or_else(Vec::new, |dir| claude_settings_in(&dir))
}

/// `settings.json` and `settings.local.json` in `dir` (a `.claude`
/// directory). Unreadable files are skipped.
pub(crate) fn claude_settings_in(dir: &std::path::Path) -> Vec<serde_json::Value> {
    ["settings.json", "settings.local.json"]
        .iter()
        .filter_map(|name| std::fs::read_to_string(dir.join(name)).ok())
        .filter_map(|text| serde_json::from_str(&text).ok())
        .collect()
}

/// Tools a read-only reviewer may never run, whatever the user allows.
const REVIEWER_DENIED_TOOLS: [&str; 3] = ["Edit", "Write", "NotebookEdit"];

/// `--settings` JSON for a read-only reviewer: only the deny rules from
/// `files` plus the write tools, and only `PreToolUse` guard hooks. The
/// user's `allow` rules stay out, since with no prompts they would let the
/// reviewer edit or commit unasked; so do side-effect hooks.
pub(crate) fn reviewer_settings(files: &[serde_json::Value]) -> String {
    let mut deny: Vec<serde_json::Value> = REVIEWER_DENIED_TOOLS
        .iter()
        .map(|tool| serde_json::Value::from(*tool))
        .collect();
    let mut guards = Vec::new();
    for file in files {
        for rule in file["permissions"]["deny"].as_array().into_iter().flatten() {
            if !deny.contains(rule) {
                deny.push(rule.clone());
            }
        }
        guards.extend(
            file["hooks"]["PreToolUse"]
                .as_array()
                .into_iter()
                .flatten()
                .cloned(),
        );
    }
    let mut settings = serde_json::json!({ "permissions": { "deny": deny } });
    if !guards.is_empty() {
        settings["hooks"] = serde_json::json!({ "PreToolUse": guards });
    }
    settings.to_string()
}

/// Permission rule lists and hooks merged from `files`, as `--settings`
/// JSON; `None` when there are none.
fn safety_settings(files: &[serde_json::Value]) -> Option<String> {
    let mut permissions = serde_json::Map::new();
    let mut hooks = serde_json::Map::new();
    for file in files {
        for key in ["allow", "ask", "deny", "additionalDirectories"] {
            let Some(rules) = file["permissions"][key].as_array() else {
                continue;
            };
            let merged = permissions
                .entry(key)
                .or_insert_with(|| serde_json::Value::Array(Vec::new()));
            if let Some(merged) = merged.as_array_mut() {
                for rule in rules {
                    if !merged.contains(rule) {
                        merged.push(rule.clone());
                    }
                }
            }
        }
        if let Some(events) = file["hooks"].as_object() {
            for (event, matchers) in events {
                let Some(matchers) = matchers.as_array() else {
                    continue;
                };
                let merged = hooks
                    .entry(event.clone())
                    .or_insert_with(|| serde_json::Value::Array(Vec::new()));
                if let Some(merged) = merged.as_array_mut() {
                    merged.extend(matchers.iter().cloned());
                }
            }
        }
    }
    let mut settings = serde_json::Map::new();
    if !permissions.is_empty() {
        settings.insert("permissions".into(), permissions.into());
    }
    if !hooks.is_empty() {
        settings.insert("hooks".into(), hooks.into());
    }
    (!settings.is_empty()).then(|| serde_json::Value::Object(settings).to_string())
}

fn apply_claude_autocompact(options: &mut ProviderQueryOptions, value: Option<&str>) {
    let Some(flag) = bruhs_models::logic::compact::autocompact_flag(value) else {
        return;
    };
    if let ProviderQueryOptions::Claude(options) = options {
        options.autocompact = Some(flag);
    }
}

fn build_query_options(
    provider: AgentProvider,
    prompt: &str,
    project_path: &str,
    model: &str,
    permission_mode: PermissionMode,
    system_prompt: Option<&str>,
    session_id: Option<&str>,
) -> ProviderQueryOptions {
    let runtime_model = provider.model_id(model);
    match provider {
        AgentProvider::Claude => {
            // No `--allowedTools` pre-approval: that silently granted Bash,
            // Edit and Write in every mode. The permission mode now decides,
            // and anything it would ask about reaches the user through
            // `PermissionRequest` events (except in bypass, which never asks).
            let mode = claude_permission_mode(permission_mode);
            let bypass = mode == claude_sdk_rs::PermissionMode::BypassPermissions;
            let mut options = claude_query_options_from_env(prompt)
                .cwd(project_path)
                .model(runtime_model)
                .permission_prompts(!bypass)
                .accept_input(true);
            options.permission_mode = Some(mode);
            options.resume = session_id.map(String::from);
            // Appended, not `--system-prompt`: replacing Claude Code's prompt
            // saves ~3.8k tokens but drops its tool-use guidance. The snapshot
            // keeps the appended text cached across resumed turns.
            let response_format = response_instructions();
            options.append_system_prompt = Some(match system_prompt {
                Some(context) => format!("{context}\n\n{response_format}"),
                None => response_format,
            });
            ProviderQueryOptions::Claude(options)
        }
        AgentProvider::Codex => {
            // The thread's context opens the first prompt of a new Codex
            // session; a resumed session already holds it. Not
            // `developer_instructions`, which would replace the user's own in
            // `~/.codex/config.toml`, and not argv, which caps one argument
            // at 128 KiB on Linux (the prompt goes over stdin).
            let prompt = match system_prompt.filter(|_| session_id.is_none()) {
                Some(context) => {
                    format!("<thread_context>\n{context}\n</thread_context>\n\n{prompt}")
                }
                None => prompt.to_string(),
            };
            // Resumed sessions may predate rich replies. Supply the small
            // format contract without replacing the user's CLI instructions.
            let prompt = format!(
                "<response_format>\n{}\n</response_format>\n\n{prompt}",
                response_instructions()
            );
            let mut options = codex_query_options(prompt)
                .cwd(project_path)
                .model(runtime_model)
                .access(codex_access(permission_mode))
                .skip_git_repo_check(true);
            if runtime_model == "default" {
                options.model = None;
            }
            // New threads are saved (not ephemeral) so the next turn can resume them.
            if let Some(thread_id) = session_id {
                options = options.resume(thread_id);
            }
            ProviderQueryOptions::Codex(options)
        }
        AgentProvider::Cursor
        | AgentProvider::Antigravity
        | AgentProvider::Grok
        | AgentProvider::OpenCode
        | AgentProvider::Acp => {
            let mut options = external::Options::new(
                provider,
                prompt,
                project_path,
                model,
                permission_mode,
                system_prompt,
                session_id,
            );
            options.append_instructions(&response_instructions());
            ProviderQueryOptions::External(options)
        }
    }
}

fn response_instructions() -> String {
    let output = match crate::uploads::media_output_directory() {
        Ok(path) => format!(
            "For project-less chats, create {} if needed and save response media there; use its absolute path in the reply.",
            path.display()
        ),
        Err(error) => format!(
            "The media output directory is unavailable: {error}. Use a project directory or explain the failure."
        ),
    };
    format!(
        "{}\n{output}\nInteractive widgets can await window.bruhs.ready, use bruhs.getState()/setState(value) for custom state, and bruhs.submit({{label,message}}) to propose a chat draft for user review. Form values save automatically except password/file fields; use stable id/name attributes. Media JSON also accepts poster, tracks:[{{src,label,language,kind:\"subtitles\"}}], and transcript text. Publish an HTML file with the publish_artifact tool for durable bundled assets and inspect_artifact for actual client preview diagnostics.",
        bruhs_models::logic::rich_response::RESPONSE_INSTRUCTIONS
    )
}

/// Opt-in that lets Claude runs keep an inherited `ANTHROPIC_API_KEY`.
const CLAUDE_API_KEY_OPT_IN: &str = "BRUHS_CLAUDE_USE_API_KEY";

/// Whether `BRUHS_CLAUDE_USE_API_KEY=1` asks Claude runs to keep an inherited
/// `ANTHROPIC_API_KEY`. The idle-compact window is five minutes when this is
/// set, and one hour on the subscription login.
pub fn claude_uses_api_key() -> bool {
    keep_anthropic_api_key()
}

/// Whether `BRUHS_CLAUDE_USE_API_KEY=1` asks Claude runs to keep an inherited
/// `ANTHROPIC_API_KEY`.
fn keep_anthropic_api_key() -> bool {
    std::env::var_os(CLAUDE_API_KEY_OPT_IN).is_some_and(|value| value == "1")
}

/// Remove inherited session and messaging variables before launching a provider.
const PARENT_SESSION_ENV: [&str; 12] = [
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_PID",
    "CLAUDE_AGENT_SDK_VERSION",
    "CLAUDE_EFFORT",
    "AI_AGENT",
];

const CLI_TELEMETRY_OPT_OUT: [(&str, &str); 2] =
    [("DISABLE_TELEMETRY", "1"), ("DISABLE_ERROR_REPORTING", "1")];

pub(crate) fn codex_query_options(prompt: impl Into<String>) -> codex_sdk_rs::QueryOptions {
    let mut options = codex_sdk_rs::QueryOptions::new(prompt)
        .config(codex_sdk_rs::ConfigOverride::disable_analytics());
    for key in crate::connections::PRIVATE_ENV {
        options = options.env_remove(key);
    }
    options
}

pub(crate) fn disable_codex_analytics(command: &mut tokio::process::Command) {
    let entry = codex_sdk_rs::ConfigOverride::disable_analytics();
    command.arg(format!("--config={}={}", entry.key(), entry.value()));
}

/// Base options for every Claude run bruhs starts. Claude Code is meant to
/// use the user's OAuth login; an `ANTHROPIC_API_KEY` inherited from whatever
/// launched bruhs would silently switch it to API billing, so it is removed
/// unless `keep_api_key` (see [`CLAUDE_API_KEY_OPT_IN`]). The variables of a
/// parent agent session ([`PARENT_SESSION_ENV`]) are always removed.
pub(crate) fn claude_query_options(
    prompt: &str,
    keep_api_key: bool,
) -> claude_sdk_rs::QueryOptions {
    let mut options = claude_sdk_rs::QueryOptions::new(prompt);
    for (key, value) in CLI_TELEMETRY_OPT_OUT {
        options = options.env(key, value);
    }
    for key in PARENT_SESSION_ENV
        .into_iter()
        .chain(crate::connections::PRIVATE_ENV)
    {
        options = options.env_remove(key);
    }
    if keep_api_key {
        options
    } else {
        options.env_remove("ANTHROPIC_API_KEY")
    }
}

/// Claude options for a run, with the API-key policy read from the
/// environment.
pub(crate) fn claude_query_options_from_env(prompt: &str) -> claude_sdk_rs::QueryOptions {
    claude_query_options(prompt, keep_anthropic_api_key())
}

/// The Claude CLI's `--permission-mode` for `mode`; the two enums match
/// one to one.
fn claude_permission_mode(mode: PermissionMode) -> claude_sdk_rs::PermissionMode {
    match mode {
        PermissionMode::Default => claude_sdk_rs::PermissionMode::Default,
        PermissionMode::AcceptEdits => claude_sdk_rs::PermissionMode::AcceptEdits,
        PermissionMode::Plan => claude_sdk_rs::PermissionMode::Plan,
        PermissionMode::BypassPermissions => claude_sdk_rs::PermissionMode::BypassPermissions,
    }
}

/// Codex's sandbox and approval policy for `mode`.
///
/// Codex runs as `codex exec --json`, which cannot ask the user anything:
/// it emits no approval requests and reads nothing back after the prompt
/// (codex-sdk-rs has no approval events or responder). So no mode can use
/// an approval-prompting policy. `Default`, which means "ask before acting"
/// on Claude, is therefore read-only on Codex rather than silently writing
/// without asking; choose `AcceptEdits` to let Codex write in the workspace.
fn codex_access(mode: PermissionMode) -> CodexAccess {
    match mode {
        PermissionMode::Default | PermissionMode::Plan => CodexAccess::Sandboxed {
            sandbox: SandboxMode::ReadOnly,
            approval: ApprovalPolicy::Never,
        },
        PermissionMode::AcceptEdits => CodexAccess::Sandboxed {
            sandbox: SandboxMode::WorkspaceWrite,
            approval: ApprovalPolicy::Never,
        },
        PermissionMode::BypassPermissions => CodexAccess::DangerouslyUnsandboxed,
    }
}

async fn stream_claude(
    options: claude_sdk_rs::QueryOptions,
    turn: TurnContext,
    permissions: Permissions,
    steers: Steers,
    interactive_questions: bool,
) {
    let mut stream = match claude_sdk_rs::query(options) {
        Ok(stream) => stream,
        Err(e) => {
            turn.send_start_error(e.to_string()).await;
            return;
        }
    };
    turn.register_process(stream.process_id);
    let responder = stream.responder();
    steers
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(turn.owner.thread_id.clone(), responder.clone());
    let _steer = SteerGuard {
        steers,
        thread_id: turn.owner.thread_id.clone(),
    };
    let mut reported_error = false;
    let mut completed = None;
    let mut consumer_alive = loop {
        let event = tokio::select! {
            // The caller dropped the stream: stop now (dropping `stream`
            // kills the CLI) instead of at the next event.
            () = turn.tx.closed() => break false,
            event = tokio_stream::StreamExt::next(&mut stream) => match event {
                Some(event) => event,
                None => break true,
            },
        };
        if matches!(event, claude_sdk_rs::QueryEvent::ProcessFailed(_)) && reported_error {
            // The turn already ended with an error; the exit status
            // that follows it adds nothing for the user.
            continue;
        }
        let cli_cost_usd = match &event {
            claude_sdk_rs::QueryEvent::Result { total_cost_usd, .. } => *total_cost_usd,
            _ => None,
        };
        let event = map_claude_event(event, interactive_questions);
        reported_error |= is_error_event(&event);
        turn.remember_session(&event);
        if let QueryEvent::Result {
            usage, session_id, ..
        } = &event
        {
            turn.record_usage(usage, Some(session_id), cli_cost_usd)
                .await;
        }
        if matches!(
            event,
            QueryEvent::Result {
                is_error: false,
                ..
            }
        ) {
            // A result can precede a failed exit. Consumers stop on success,
            // so keep it private until the SDK settles the whole process.
            completed = Some(event);
            continue;
        }
        match &event {
            QueryEvent::PermissionRequest {
                id,
                input,
                can_answer,
                ..
            } => {
                permissions.write().await.insert(
                    id.clone(),
                    PendingPermission {
                        owner: turn.owner.clone(),
                        input: input.clone(),
                        can_answer: *can_answer,
                        question_files: Vec::new(),
                        events: turn.tx.clone(),
                        responder: responder.clone(),
                    },
                );
            }
            QueryEvent::PermissionCancelled { id } => {
                let mut pending = permissions.write().await;
                if pending
                    .get(id)
                    .is_some_and(|request| request.owner == turn.owner)
                {
                    pending.remove(id);
                }
            }
            _ => {}
        }
        if !turn.forward(event).await {
            break false;
        }
    };
    // Whatever this turn still has pending can no longer be answered: the
    // CLI is gone (stream ended) or the client left (stream dropped).
    release_turn_permissions(&mut *permissions.write().await, &turn.owner);
    if consumer_alive && !reported_error {
        consumer_alive = turn
            .forward(completed.unwrap_or_else(|| {
                QueryEvent::Error(
                    "Claude ended without a turn completion receipt. Review the output and workspace before explicitly retrying.".into(),
                )
            }))
            .await;
    }
    turn.finish(consumer_alive);
}

/// `record_lean`: the lean-session list to add this turn's new session to.
async fn stream_codex(
    options: codex_sdk_rs::QueryOptions,
    turn: TurnContext,
    model: String,
    mut record_lean: Option<std::path::PathBuf>,
) {
    let started = std::time::Instant::now();
    let mut session_id = match &options.session {
        codex_sdk_rs::Session::Resume(id) => Some(id.clone()),
        _ => None,
    };
    let mut answer = String::new();
    let mut completed_usage = None;
    let mut stream = match codex_sdk_rs::query(options) {
        Ok(stream) => stream,
        Err(e) => {
            turn.send_start_error(e.to_string()).await;
            return;
        }
    };
    turn.register_process(stream.process_id);
    let mut reported_error = false;
    let mut consumer_alive = loop {
        let event = tokio::select! {
            () = turn.tx.closed() => break false,
            event = tokio_stream::StreamExt::next(&mut stream) => match event {
                Some(event) => event,
                None => break true,
            },
        };
        if matches!(event, codex_sdk_rs::QueryEvent::ProcessFailed(_)) && reported_error {
            // The turn already ended with an error; the exit status
            // that follows it adds nothing for the user.
            continue;
        }
        let event = map_codex_event(event, &model);
        match &event {
            QueryEvent::Init { session_id: id, .. } => session_id = Some(id.clone()),
            QueryEvent::Text(text) => answer.clone_from(text),
            QueryEvent::Usage(usage) => {
                completed_usage = Some(usage.clone());
                turn.record_usage(usage, session_id.as_deref(), None).await;
            }
            _ => {}
        }
        if let QueryEvent::Init { session_id, .. } = &event {
            if let Some(file) = record_lean.take() {
                let id = session_id.clone();
                // A tiny append, off the async runtime, finished before the
                // id becomes resumable; a failure only costs the session its
                // lean resumes.
                let recorded =
                    tokio::task::spawn_blocking(move || crate::codex_lean::record(&file, &id))
                        .await;
                if let Ok(Err(error)) = recorded {
                    tracing::warn!(%error, "could not record a lean Codex session");
                }
            }
        }
        reported_error |= is_error_event(&event);
        turn.remember_session(&event);
        if !turn.forward(event).await {
            break false;
        }
    };
    if consumer_alive && !reported_error {
        // The SDK emits usage after process settlement, possibly followed by a
        // failed exit. Only EOF without that failure can complete the turn.
        let completion = match (completed_usage, session_id.filter(|id| !id.is_empty())) {
            (Some(usage), Some(session_id)) => QueryEvent::Result {
                result: answer,
                session_id,
                duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                usage,
                is_error: false,
            },
            _ => QueryEvent::Error(
                "Codex ended without a complete turn completion receipt and session ID. Review the output and workspace before explicitly retrying.".into(),
            ),
        };
        turn.remember_session(&completion);
        consumer_alive = turn.forward(completion).await;
    }
    turn.finish(consumer_alive);
}

impl Drop for TurnContext {
    fn drop(&mut self) {
        // Explicit finish drains this list. Cancellation or a host deadline
        // reaches this guard with the remaining turn-owned tasks still present.
        self.finish(false);
    }
}

pub fn validate_execution_budget(seconds: Option<u64>) -> Result<(), SendMessageError> {
    budget::deadline(seconds).map(|_| ())
}

pub fn execution_budget_message(seconds: u64) -> String {
    format!("The owner-set {seconds} second execution budget was reached. This run stopped without a completion result. Files and partial output remain available; inspect them before continuing. Final usage may be unavailable.")
}

impl TurnContext {
    fn register_process(&self, pid: Option<u32>) {
        if let Some(session) = lock_sessions(&self.sessions).get_mut(&self.owner.thread_id) {
            if session.turn == self.owner.turn && session.status == ThreadStatus::Streaming {
                session.process_id = pid;
            }
        }
    }

    /// Report that the provider CLI could not be started.
    async fn send_start_error(&self, message: String) {
        // Best effort: if the caller already dropped the stream there is
        // nobody left to tell.
        let _ = self
            .tx
            .send(OrchestratorEvent::Query(QueryEvent::Error(message)))
            .await;
    }

    /// Record the provider session id announced by `init`/`result` so a
    /// later turn on this thread can resume even if the client lost its copy.
    /// An abandoned turn never overwrites what a newer turn recorded.
    fn remember_session(&self, event: &QueryEvent) {
        let id = match event {
            QueryEvent::Init { session_id, .. } | QueryEvent::Result { session_id, .. }
                if !session_id.is_empty() =>
            {
                session_id.clone()
            }
            _ => return,
        };
        if let Some(session) = lock_sessions(&self.sessions).get_mut(&self.owner.thread_id) {
            if session.turn == self.owner.turn {
                session.session = Some((self.provider, id));
            }
        }
    }

    async fn record_usage(
        &self,
        usage: &UsageInfo,
        session_id: Option<&str>,
        cli_cost_usd: Option<f64>,
    ) {
        if cfg!(test)
            || (cli_cost_usd.is_none()
                && usage.input_tokens == 0
                && usage.output_tokens == 0
                && usage.cache_read_input_tokens == 0
                && usage.cache_creation_input_tokens == 0)
        {
            return;
        }
        let record = bruhs_models::agent_extensions::UsageRecord {
            id: self.usage_receipt.clone(),
            thread_id: self.owner.thread_id.as_str().into(),
            session_id: session_id.filter(|id| !id.is_empty()).map(str::to_owned),
            model: self.model.clone(),
            project: self.media_project.clone(),
            at: chrono::Utc::now().timestamp(),
            input: usage.input_tokens,
            output: usage.output_tokens,
            cache_read: usage.cache_read_input_tokens,
            cache_write: usage.cache_creation_input_tokens,
            requested_speed: self.requested_speed,
            cli_cost_usd,
        };
        let saved = tokio::task::spawn_blocking(move || {
            crate::agent_extensions::record_at(&crate::agent_extensions::directory()?, record)
        })
        .await;
        if !matches!(saved, Ok(Ok(()))) {
            tracing::warn!(?saved, "Could not persist reported turn usage");
        }
    }

    /// Send `event` (and any background task it starts) to the caller.
    /// False when the caller has dropped the stream.
    async fn forward(&self, mut event: QueryEvent) -> bool {
        if let QueryEvent::Result {
            usage, session_id, ..
        } = &event
        {
            // Native runtimes retain reported usage independently of process
            // success. Their later completion must not record it twice.
            if !matches!(self.provider, AgentProvider::Claude | AgentProvider::Codex) {
                self.record_usage(usage, Some(session_id), None).await;
            }
        }
        if !cfg!(test) {
            if let QueryEvent::Raw(raw) = &event {
                if raw["type"] == "rate_limit_event" {
                    let raw = raw.clone();
                    let saved = tokio::task::spawn_blocking(move || {
                        crate::agent_extensions::record_claude_limit(&raw)
                    })
                    .await;
                    if !matches!(saved, Ok(Ok(()))) {
                        tracing::warn!(?saved, "Could not persist reported quota metadata");
                    }
                }
            }
        }
        match &event {
            QueryEvent::Text(text) => self
                .response_text
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push_str(text),
            QueryEvent::Result { .. } | QueryEvent::Error(_) => {
                let mut text = std::mem::take(
                    &mut *self
                        .response_text
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner),
                );
                if text.is_empty() {
                    if let QueryEvent::Result { result, .. } = &event {
                        text.clone_from(result)
                    }
                }
                if !text.is_empty() {
                    let project = self.media_project.clone();
                    // Capture valid local media before publishing completion,
                    // including replies in background or unopened threads.
                    let result = tokio::task::spawn_blocking(move || {
                        crate::rich_responses::capture_turn_media(&text, &project)
                    })
                    .await;
                    match result {
                        Ok((bindings, errors)) => {
                            if let QueryEvent::Result { result, .. } = &mut event {
                                *result =
                                    bruhs_models::logic::rich_response::rewrite_response_media(
                                        result, &bindings,
                                    );
                            }
                            if !bindings.is_empty() && self.tx.send(OrchestratorEvent::Query(QueryEvent::Raw(serde_json::json!({"type":"response_media","bindings":bindings})))).await.is_err(){return false}
                            for error in errors {
                                tracing::warn!(%error,"could not retain response media")
                            }
                        }
                        Err(error) => {
                            tracing::warn!(%error,"response media capture worker stopped")
                        }
                    }
                }
            }
            QueryEvent::ToolCall { .. } | QueryEvent::ToolResult { .. } => {
                self.response_text
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push_str("\n\n");
            }
            _ => {}
        }
        for task in self.reconcile_background_tasks(&event) {
            self.tasks
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(task.id.clone());
            if self
                .tx
                .send(OrchestratorEvent::BackgroundTaskRegistered(task))
                .await
                .is_err()
            {
                return false;
            }
        }
        self.tx.send(OrchestratorEvent::Query(event)).await.is_ok()
    }

    /// Track background shells, subagents, teamwork/swarm workers, and
    /// delegated background agents from tool calls and results.
    fn reconcile_background_tasks(&self, event: &QueryEvent) -> Vec<BackgroundTask> {
        let thread_id = self.owner.thread_id.as_str();
        match event {
            QueryEvent::ToolCall { id, tool, input } => {
                let mut created = Vec::new();
                match tool.as_str() {
                    "Bash" => {
                        if input
                            .get("run_in_background")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false)
                        {
                            let label = input
                                .get("command")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("background task")
                                .to_string();
                            let mut meta = input.clone();
                            if !meta.is_object() {
                                meta = serde_json::json!({});
                            }
                            meta["persists_after_turn"] = serde_json::json!(true);
                            meta["tool_call_id"] = serde_json::json!(id);
                            created.push(self.registry.register(
                                thread_id,
                                TaskKind::Shell,
                                label,
                                meta,
                            ));
                        }
                    }
                    "run_command" => {
                        let is_daemon = input
                            .get("IsDaemon")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false);
                        let wait_ms = input
                            .get("WaitMsBeforeAsync")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(5_000);
                        let command = input
                            .get("CommandLine")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let label = input
                            .get("toolSummary")
                            .and_then(serde_json::Value::as_str)
                            .filter(|s| !s.trim().is_empty())
                            .or_else(|| (!command.is_empty()).then_some(command.as_str()))
                            .unwrap_or("background command")
                            .to_string();
                        let mut registered_task_id = None;
                        if !is_daemon && wait_ms <= 500 {
                            let mut meta = input.clone();
                            if !meta.is_object() {
                                meta = serde_json::json!({});
                            }
                            meta["tool_call_id"] = serde_json::json!(id);
                            let task = self.registry.register(
                                thread_id,
                                TaskKind::Shell,
                                label.clone(),
                                meta,
                            );
                            registered_task_id = Some(task.id.clone());
                            created.push(task);
                        }
                        self.pending_commands
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .insert(
                                id.clone(),
                                PendingCommand {
                                    label,
                                    command,
                                    is_daemon,
                                    registered_task_id,
                                },
                            );
                    }
                    "manage_task" => {
                        match input.get("Action").and_then(serde_json::Value::as_str) {
                            Some("kill") => {
                                if let Some(ext_id) =
                                    input.get("TaskId").and_then(serde_json::Value::as_str)
                                {
                                    for task_id in self.registry.find_running_by_meta(
                                        thread_id,
                                        "external_task_id",
                                        ext_id,
                                    ) {
                                        self.registry.complete_task(
                                            &task_id,
                                            TaskStatus::Killed,
                                            None,
                                            Some("Background task killed."),
                                        );
                                    }
                                }
                            }
                            Some("status") => {
                                if let Some(ext_id) =
                                    input.get("TaskId").and_then(serde_json::Value::as_str)
                                {
                                    self.task_status_checks
                                        .lock()
                                        .unwrap_or_else(PoisonError::into_inner)
                                        .insert(id.clone(), ext_id.to_string());
                                }
                            }
                            _ => {}
                        }
                    }
                    "schedule" => {
                        let is_daemon = input
                            .get("IsDaemon")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false);
                        if !is_daemon && input.get("DurationSeconds").is_some() {
                            let prompt = input
                                .get("Prompt")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("Scheduled timer");
                            let label: String = prompt.chars().take(72).collect();
                            let meta = serde_json::json!({
                                "tool": "schedule",
                                "tool_call_id": id,
                                "role": "timer",
                                "persists_after_turn": true,
                            });
                            let task = self.registry.register(
                                thread_id,
                                TaskKind::BackgroundAgent,
                                label,
                                meta,
                            );
                            self.registry.append_log(&task.id, prompt);
                            created.push(task);
                        }
                    }
                    "Task" | "Agent" => {
                        let async_bg = input
                            .get("run_in_background")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false);
                        let kind = if async_bg {
                            TaskKind::BackgroundAgent
                        } else {
                            TaskKind::Subagent
                        };
                        let label = input
                            .get("description")
                            .or_else(|| input.get("subagent_type"))
                            .and_then(serde_json::Value::as_str)
                            .filter(|s| !s.trim().is_empty())
                            .unwrap_or(if async_bg {
                                "Background agent"
                            } else {
                                "Subagent"
                            })
                            .to_string();
                        let mut meta = serde_json::json!({
                            "tool": tool,
                            "tool_call_id": id,
                            "persists_after_turn": async_bg,
                        });
                        if let Some(sub) = input
                            .get("subagent_type")
                            .and_then(serde_json::Value::as_str)
                        {
                            meta["role"] = serde_json::json!(sub);
                        }
                        if let Some(model) = input.get("model").and_then(serde_json::Value::as_str)
                        {
                            meta["model"] = serde_json::json!(model);
                        }
                        let task = self.registry.register(thread_id, kind, label, meta);
                        if let Some(prompt) =
                            input.get("prompt").and_then(serde_json::Value::as_str)
                        {
                            self.registry.append_log(&task.id, prompt);
                        }
                        created.push(task);
                    }
                    "invoke_subagent" => {
                        let role = input
                            .get("Role")
                            .or_else(|| input.get("role"))
                            .or_else(|| input.get("TypeName"))
                            .and_then(serde_json::Value::as_str)
                            .filter(|s| !s.trim().is_empty())
                            .unwrap_or("Subagent");
                        let mut meta = serde_json::json!({
                            "tool": tool,
                            "tool_call_id": id,
                            "role": role,
                            "persists_after_turn": true,
                        });
                        if let Some(model) = input.get("Model").and_then(serde_json::Value::as_str)
                        {
                            meta["model"] = serde_json::json!(model);
                        }
                        if let Some(ws) = input.get("Workspace").and_then(serde_json::Value::as_str)
                        {
                            meta["workspace"] = serde_json::json!(ws);
                        }
                        let task =
                            self.registry
                                .register(thread_id, TaskKind::Subagent, role, meta);
                        if let Some(prompt) =
                            input.get("Prompt").and_then(serde_json::Value::as_str)
                        {
                            self.registry.append_log(&task.id, prompt);
                        }
                        created.push(task);
                    }
                    "spawn_agent" => {
                        let label = input
                            .get("role")
                            .or_else(|| input.get("description"))
                            .and_then(serde_json::Value::as_str)
                            .filter(|s| !s.trim().is_empty())
                            .unwrap_or("Subagent");
                        let meta = serde_json::json!({
                            "tool": tool,
                            "tool_call_id": id,
                            "role": label,
                            "persists_after_turn": true,
                        });
                        let task =
                            self.registry
                                .register(thread_id, TaskKind::Subagent, label, meta);
                        if let Some(prompt) = input
                            .get("prompt")
                            .or_else(|| input.get("message"))
                            .and_then(serde_json::Value::as_str)
                        {
                            self.registry.append_log(&task.id, prompt);
                        }
                        created.push(task);
                    }
                    "delegate_task" => {
                        let wait =
                            input.get("mode").and_then(serde_json::Value::as_str) == Some("wait");
                        let kind = if wait {
                            TaskKind::Subagent
                        } else {
                            TaskKind::BackgroundAgent
                        };
                        let label = input
                            .get("title")
                            .or_else(|| input.get("toolSummary"))
                            .or_else(|| input.get("role"))
                            .and_then(serde_json::Value::as_str)
                            .filter(|s| !s.trim().is_empty())
                            .unwrap_or("Delegated agent");
                        let mut meta = serde_json::json!({
                            "tool": tool,
                            "tool_call_id": id,
                            "persists_after_turn": !wait,
                        });
                        if let Some(role) = input.get("role").and_then(serde_json::Value::as_str) {
                            meta["role"] = serde_json::json!(role);
                        }
                        if let Some(model) = input
                            .pointer("/target/model")
                            .and_then(serde_json::Value::as_str)
                        {
                            meta["model"] = serde_json::json!(model);
                        }
                        let task = self.registry.register(thread_id, kind, label, meta);
                        if let Some(body) = input.get("task").and_then(serde_json::Value::as_str) {
                            self.registry.append_log(&task.id, body);
                        }
                        created.push(task);
                    }
                    "run_workflow" => {
                        let label = input
                            .get("Description")
                            .and_then(serde_json::Value::as_str)
                            .filter(|s| !s.trim().is_empty())
                            .unwrap_or("Workflow");
                        let meta = serde_json::json!({
                            "tool": tool,
                            "tool_call_id": id,
                            "persists_after_turn": true,
                        });
                        let task = self.registry.register(
                            thread_id,
                            TaskKind::BackgroundAgent,
                            label,
                            meta,
                        );
                        created.push(task);
                    }
                    "swarm" => match input.get("action").and_then(serde_json::Value::as_str) {
                        Some("add") => {
                            let base = input
                                .get("name")
                                .and_then(serde_json::Value::as_str)
                                .filter(|s| !s.trim().is_empty())
                                .unwrap_or("worker");
                            let brief = input
                                .get("brief")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("");
                            let model = input.get("model").and_then(serde_json::Value::as_str);
                            let workspace =
                                input.get("workspace").and_then(serde_json::Value::as_str);
                            let workers: Vec<(String, Option<&str>)> = if let Some(slices) =
                                input.get("tasks").and_then(serde_json::Value::as_array)
                            {
                                if slices.is_empty() {
                                    vec![(base.to_string(), None)]
                                } else {
                                    slices
                                        .iter()
                                        .enumerate()
                                        .map(|(i, slice)| {
                                            (format!("{base}-{}", i + 1), slice.as_str())
                                        })
                                        .collect()
                                }
                            } else if let Some(n) =
                                input.get("n").and_then(serde_json::Value::as_u64)
                            {
                                if n > 1 {
                                    (1..=n.min(32))
                                        .map(|i| (format!("{base}-{i}"), None))
                                        .collect()
                                } else {
                                    vec![(base.to_string(), None)]
                                }
                            } else {
                                vec![(base.to_string(), None)]
                            };
                            for (worker_name, slice) in workers {
                                let mut meta = serde_json::json!({
                                    "tool": "swarm",
                                    "tool_call_id": id,
                                    "worker_name": worker_name,
                                    "persists_after_turn": true,
                                });
                                if let Some(m) = model {
                                    meta["model"] = serde_json::json!(m);
                                }
                                if let Some(ws) = workspace {
                                    meta["workspace"] = serde_json::json!(ws);
                                }
                                let task = self.registry.register(
                                    thread_id,
                                    TaskKind::Swarm,
                                    worker_name,
                                    meta,
                                );
                                let log = match (slice, brief.is_empty()) {
                                    (Some(s), false) => format!("Target: {s}\n\n{brief}"),
                                    (Some(s), true) => format!("Target: {s}"),
                                    (None, false) => brief.to_string(),
                                    (None, true) => String::new(),
                                };
                                if !log.is_empty() {
                                    self.registry.append_log(&task.id, &log);
                                }
                                created.push(task);
                            }
                        }
                        Some("kill") => {
                            if let Some(name) =
                                input.get("name").and_then(serde_json::Value::as_str)
                            {
                                for task_id in self.registry.find_running_by_meta(
                                    thread_id,
                                    "worker_name",
                                    name,
                                ) {
                                    self.registry.complete_task(
                                        &task_id,
                                        TaskStatus::Killed,
                                        None,
                                        Some("Stopped via swarm kill."),
                                    );
                                }
                            }
                        }
                        Some("status") => {
                            self.status_checks
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .insert(id.clone());
                        }
                        Some("title") => {
                            if let Some(title) =
                                input.get("title").and_then(serde_json::Value::as_str)
                            {
                                for task in self.registry.get_by_thread(thread_id) {
                                    if task.kind == TaskKind::Swarm
                                        && task.status == TaskStatus::Running
                                    {
                                        self.registry.merge_meta(
                                            &task.id,
                                            serde_json::json!({ "swarm_title": title }),
                                        );
                                    }
                                }
                            }
                        }
                        _ => {}
                    },
                    "manage_subagents" => {
                        match input.get("Action").and_then(serde_json::Value::as_str) {
                            Some("kill_all") => {
                                self.registry.finish_thread_kind(
                                    thread_id,
                                    TaskKind::Subagent,
                                    TaskStatus::Killed,
                                );
                            }
                            Some("kill") => {
                                for cid in input
                                    .get("ConversationIds")
                                    .and_then(serde_json::Value::as_array)
                                    .into_iter()
                                    .flatten()
                                    .filter_map(serde_json::Value::as_str)
                                {
                                    for task_id in self.registry.find_running_by_meta(
                                        thread_id,
                                        "conversation_id",
                                        cid,
                                    ) {
                                        self.registry.complete_task(
                                            &task_id,
                                            TaskStatus::Killed,
                                            None,
                                            Some("Subagent killed."),
                                        );
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    "task_cancel" => {
                        if let Some(ext_id) =
                            input.get("taskId").and_then(serde_json::Value::as_str)
                        {
                            for task_id in self.registry.find_running_by_meta(
                                thread_id,
                                "external_task_id",
                                ext_id,
                            ) {
                                self.registry.complete_task(
                                    &task_id,
                                    TaskStatus::Killed,
                                    None,
                                    Some("Delegated task cancelled."),
                                );
                            }
                        }
                    }
                    "workflow_stop" => {
                        self.registry.finish_thread_kind(
                            thread_id,
                            TaskKind::BackgroundAgent,
                            TaskStatus::Killed,
                        );
                    }
                    _ => {}
                }
                if !created.is_empty() {
                    let ids: Vec<String> = created.iter().map(|t| t.id.clone()).collect();
                    self.tool_tasks
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .insert(id.clone(), ids);
                }
                created
            }
            QueryEvent::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                let mut created = Vec::new();
                if self
                    .status_checks
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(tool_use_id)
                {
                    reconcile_swarm_status(&self.registry, thread_id, content);
                }
                if let Some(ext_id) = self
                    .task_status_checks
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(tool_use_id)
                {
                    reconcile_task_status(&self.registry, thread_id, &ext_id, content, *is_error);
                }
                if let Some(pending) = self
                    .pending_commands
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(tool_use_id)
                {
                    if let Some((ext_id, log_path)) = parse_background_command_result(content) {
                        if !pending.is_daemon {
                            let mut patch = serde_json::json!({
                                "external_task_id": ext_id,
                                "persists_after_turn": true,
                            });
                            if let Some(path) = log_path {
                                patch["log_path"] = serde_json::json!(path);
                            }
                            if let Some(existing_id) = pending.registered_task_id {
                                self.registry.merge_meta(&existing_id, patch);
                                if !content.trim().is_empty() {
                                    self.registry.append_log(&existing_id, content);
                                }
                            } else {
                                if !pending.command.is_empty() {
                                    patch["command"] = serde_json::json!(pending.command);
                                }
                                let task = self.registry.register(
                                    thread_id,
                                    TaskKind::Shell,
                                    pending.label,
                                    patch,
                                );
                                if !content.trim().is_empty() {
                                    self.registry.append_log(&task.id, content);
                                }
                                created.push(task);
                            }
                        }
                        // Handled as a backgrounded command; drop any pre-registered
                        // synchronous tool_tasks mapping so it isn't immediately closed below.
                        self.tool_tasks
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .remove(tool_use_id);
                    }
                }
                let tracked = self
                    .tool_tasks
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(tool_use_id);
                if let Some(task_ids) = tracked {
                    let parsed = serde_json::from_str::<serde_json::Value>(content).ok();
                    let conversation_id = parsed
                        .as_ref()
                        .and_then(|v| {
                            v.get("conversationId")
                                .or_else(|| v.get("conversation_id"))
                                .and_then(serde_json::Value::as_str)
                        })
                        .map(str::to_string);
                    let external_task_id = parsed
                        .as_ref()
                        .and_then(|v| {
                            v.get("taskId")
                                .or_else(|| v.get("task_id"))
                                .or_else(|| v.get("shell_id"))
                                .and_then(serde_json::Value::as_str)
                        })
                        .map(str::to_string)
                        .or_else(|| parse_background_command_result(content).map(|(id, _)| id));
                    for task_id in task_ids {
                        let persists = self
                            .registry
                            .get(&task_id)
                            .and_then(|t| {
                                t.meta
                                    .get("persists_after_turn")
                                    .and_then(serde_json::Value::as_bool)
                            })
                            .unwrap_or(false);
                        if *is_error {
                            self.registry.complete_task(
                                &task_id,
                                TaskStatus::Failed,
                                Some(1),
                                Some(content),
                            );
                        } else if persists {
                            if let Some(cid) = &conversation_id {
                                self.registry.merge_meta(
                                    &task_id,
                                    serde_json::json!({ "conversation_id": cid }),
                                );
                            }
                            if let Some(tid) = &external_task_id {
                                self.registry.merge_meta(
                                    &task_id,
                                    serde_json::json!({ "external_task_id": tid }),
                                );
                            }
                        } else {
                            self.registry.complete_task(
                                &task_id,
                                TaskStatus::Exited,
                                Some(0),
                                Some(content),
                            );
                        }
                    }
                }
                created
            }
            _ => Vec::new(),
        }
    }

    /// Settle this turn's background tasks: the provider ended the turn on
    /// its own, or the caller left and the turn was abandoned. Tasks a newer
    /// turn on the thread started are left alone.
    fn finish(&self, consumer_alive: bool) {
        let tasks = std::mem::take(&mut *self.tasks.lock().unwrap_or_else(PoisonError::into_inner));
        if consumer_alive {
            self.registry.mark_tasks_ended(&tasks);
        } else {
            self.registry.mark_tasks_killed(&tasks);
        }
    }
}

/// Update running `TaskKind::Swarm` entries when `swarm` `action: "status"`
/// reports that workers have become `waiting`, `stopped`, or `killed`.
fn reconcile_swarm_status(registry: &BackgroundTaskRegistry, thread_id: &str, content: &str) {
    let running: Vec<BackgroundTask> = registry
        .get_by_thread(thread_id)
        .into_iter()
        .filter(|t| t.kind == TaskKind::Swarm && t.status == TaskStatus::Running)
        .collect();
    if running.is_empty() {
        return;
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(content) {
        let members = value
            .get("members")
            .or_else(|| value.get("workers"))
            .and_then(serde_json::Value::as_array)
            .or_else(|| value.as_array());
        if let Some(members) = members {
            for member in members {
                let Some(name) = member.get("name").and_then(serde_json::Value::as_str) else {
                    continue;
                };
                let state = member
                    .get("state")
                    .or_else(|| member.get("status"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                let terminal = match state {
                    "killed" => Some(TaskStatus::Killed),
                    "waiting" | "stopped" | "done" | "completed" => Some(TaskStatus::Exited),
                    _ => None,
                };
                if let Some(status) = terminal {
                    for task_id in registry.find_running_by_meta(thread_id, "worker_name", name) {
                        registry.complete_task(&task_id, status.clone(), None, None);
                    }
                }
            }
            return;
        }
    }
    for line in content.lines() {
        let lower = line.to_ascii_lowercase();
        for task in &running {
            let Some(name) = task
                .meta
                .get("worker_name")
                .and_then(serde_json::Value::as_str)
            else {
                continue;
            };
            if !lower.contains(&name.to_ascii_lowercase()) {
                continue;
            }
            if lower.contains("killed") {
                registry.complete_task(&task.id, TaskStatus::Killed, None, None);
            } else if lower.contains("stopped") || lower.contains("waiting") {
                registry.complete_task(&task.id, TaskStatus::Exited, None, None);
            }
        }
    }
}

fn is_error_event(event: &QueryEvent) -> bool {
    matches!(
        event,
        QueryEvent::Error(_) | QueryEvent::Result { is_error: true, .. }
    )
}

/// Extract `(external_task_id, optional_log_path)` when a shell/command tool
/// result indicates the run spilled over into a background task (e.g.
/// Antigravity's `"Tool is running as a background task with task id: ..."`).
fn parse_background_command_result(content: &str) -> Option<(String, Option<String>)> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(content) {
        let status = value
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        if status.eq_ignore_ascii_case("running")
            || value.get("background").and_then(serde_json::Value::as_bool) == Some(true)
        {
            if let Some(id) = value
                .get("taskId")
                .or_else(|| value.get("task_id"))
                .or_else(|| value.get("backgroundTaskId"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                let log = value
                    .get("logPath")
                    .or_else(|| value.get("log_path"))
                    .and_then(serde_json::Value::as_str)
                    .map(|p| p.strip_prefix("file://").unwrap_or(p).to_string());
                return Some((id.to_string(), log));
            }
        }
    }
    let lower = content.to_ascii_lowercase();
    if !lower.contains("background task") {
        return None;
    }
    let task_id = content.lines().find_map(|line| {
        let line_lower = line.to_ascii_lowercase();
        let at = line_lower.find("task id:")?;
        let rest = line[at + "task id:".len()..].trim();
        let token = rest
            .split_whitespace()
            .next()
            .map(|s| s.trim_matches(|c| matches!(c, '"' | '\'' | ',' | '.')))
            .filter(|s| !s.is_empty())?;
        Some(token.to_string())
    })?;
    let log_path = content.lines().find_map(|line| {
        let line_lower = line.to_ascii_lowercase();
        let at = line_lower.find("available at:")?;
        let raw = line[at + "available at:".len()..].trim();
        let path = raw.strip_prefix("file://").unwrap_or(raw).trim();
        (!path.is_empty()).then(|| path.to_string())
    });
    Some((task_id, log_path))
}

fn reconcile_task_status(
    registry: &BackgroundTaskRegistry,
    thread_id: &str,
    ext_id: &str,
    content: &str,
    is_error: bool,
) {
    let lower = content.to_ascii_lowercase();
    let exit_code = extract_exit_code(&lower);
    let terminal = if is_error || lower.contains("failed") || exit_code.is_some_and(|c| c != 0) {
        Some((TaskStatus::Failed, exit_code.or(Some(1))))
    } else if lower.contains("killed") || lower.contains("cancelled") || lower.contains("stopped") {
        Some((TaskStatus::Killed, exit_code))
    } else if lower.contains("exited")
        || lower.contains("finished")
        || lower.contains("completed")
        || lower.contains("not found")
    {
        Some((TaskStatus::Exited, exit_code.or(Some(0))))
    } else {
        None
    };
    if let Some((status, code)) = terminal {
        for task_id in registry.find_running_by_meta(thread_id, "external_task_id", ext_id) {
            registry.complete_task(&task_id, status.clone(), code, Some(content));
        }
    }
}

/// Complete running background tasks when a wakeup prompt announces
/// `Task id "<id>" finished` (or `Command exited with code N`).
fn reconcile_prompt_completions(registry: &BackgroundTaskRegistry, thread_id: &str, prompt: &str) {
    let running = registry.get_by_thread(thread_id);
    if !running.iter().any(|t| t.status == TaskStatus::Running) {
        return;
    }
    let lower = prompt.to_ascii_lowercase();
    if !(lower.contains("finished") || lower.contains("completed") || lower.contains("exited")) {
        return;
    }
    let exit_code = extract_exit_code(&lower);
    for task in running {
        if task.status != TaskStatus::Running {
            continue;
        }
        let Some(ext_id) = task
            .meta
            .get("external_task_id")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        if prompt.contains(ext_id) {
            let (status, code) = if exit_code.is_some_and(|c| c != 0) {
                (TaskStatus::Failed, exit_code)
            } else {
                (TaskStatus::Exited, exit_code.or(Some(0)))
            };
            registry.complete_task(&task.id, status, code, None);
        }
    }
}

fn extract_exit_code(lower: &str) -> Option<i32> {
    for needle in ["exited with code ", "exit code ", "exit_code: "] {
        if let Some(at) = lower.find(needle) {
            let rest = lower[at + needle.len()..].trim_start();
            let digits: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '-')
                .collect();
            if let Ok(code) = digits.parse::<i32>() {
                return Some(code);
            }
        }
    }
    None
}

/// An SDK event added after this mapping was written, passed on as raw data.
fn unhandled_sdk_event(event: &impl std::fmt::Debug) -> QueryEvent {
    QueryEvent::Raw(serde_json::json!({ "type": "unhandled", "event": format!("{event:?}") }))
}

fn map_claude_event(event: claude_sdk_rs::QueryEvent, interactive_questions: bool) -> QueryEvent {
    match event {
        claude_sdk_rs::QueryEvent::Init {
            session_id,
            model,
            tools,
            mcp_servers,
        } => QueryEvent::Init {
            session_id,
            model,
            tools,
            mcp_servers: mcp_servers
                .into_iter()
                .map(|server| McpServerInfo {
                    name: server.name,
                    status: server.status,
                })
                .collect(),
        },
        claude_sdk_rs::QueryEvent::Text(text) => QueryEvent::Text(text),
        claude_sdk_rs::QueryEvent::Thinking(text) => QueryEvent::Thinking(text),
        claude_sdk_rs::QueryEvent::ToolCall { id, tool, input } => {
            QueryEvent::ToolCall { id, tool, input }
        }
        claude_sdk_rs::QueryEvent::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => QueryEvent::ToolResult {
            tool_use_id,
            content,
            is_error,
        },
        claude_sdk_rs::QueryEvent::Usage(usage) => {
            QueryEvent::Usage(UsageInfo::claude_request(usage))
        }
        claude_sdk_rs::QueryEvent::Result {
            result,
            session_id,
            duration_ms,
            usage,
            is_error,
            errors,
            ..
        } => QueryEvent::Result {
            // Error results (`error_max_turns`, `error_during_execution`)
            // carry their reason in `errors` instead of `result`.
            result: if result.is_empty() && is_error {
                errors.join("\n")
            } else {
                result
            },
            session_id,
            duration_ms,
            usage: usage.into(),
            is_error,
        },
        claude_sdk_rs::QueryEvent::PermissionRequest {
            id,
            tool,
            input,
            tool_use_id,
            ..
        } => QueryEvent::PermissionRequest {
            id,
            can_answer: interactive_questions
                && tool == bruhs_models::logic::user_question::TOOL_NAME
                && bruhs_models::logic::user_question::questions(&input).is_some(),
            tool,
            input,
            tool_use_id,
        },
        claude_sdk_rs::QueryEvent::PermissionCancelled { id } => {
            QueryEvent::PermissionCancelled { id }
        }
        claude_sdk_rs::QueryEvent::Error(message) => QueryEvent::Error(message),
        claude_sdk_rs::QueryEvent::ProcessFailed(failure) => QueryEvent::Error(failure.to_string()),
        claude_sdk_rs::QueryEvent::Raw(value) => {
            crate::prompt_suggestion::observe_claude_quota(&value);
            QueryEvent::Raw(value)
        }
        other => unhandled_sdk_event(&other),
    }
}

fn map_codex_event(event: codex_sdk_rs::QueryEvent, model: &str) -> QueryEvent {
    match event {
        codex_sdk_rs::QueryEvent::Init { thread_id } => QueryEvent::Init {
            session_id: thread_id,
            model: Some(model.to_string()),
            tools: vec!["Bash".to_string()],
            mcp_servers: Vec::new(),
        },
        codex_sdk_rs::QueryEvent::Text(text) => QueryEvent::Text(text),
        codex_sdk_rs::QueryEvent::ToolCall { id, tool, input } => {
            QueryEvent::ToolCall { id, tool, input }
        }
        codex_sdk_rs::QueryEvent::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => QueryEvent::ToolResult {
            tool_use_id,
            content,
            is_error,
        },
        codex_sdk_rs::QueryEvent::Usage(usage) => QueryEvent::Usage(usage.into()),
        codex_sdk_rs::QueryEvent::Error(message) => QueryEvent::Error(message),
        // Often transient ("Reconnecting..."): pass it on as the raw event it
        // used to be rather than as an error, which consumers treat as the
        // end of the turn.
        codex_sdk_rs::QueryEvent::StreamError(message) => {
            QueryEvent::Raw(serde_json::json!({ "type": "error", "message": message }))
        }
        codex_sdk_rs::QueryEvent::ProcessFailed(failure) => QueryEvent::Error(failure.to_string()),
        codex_sdk_rs::QueryEvent::Raw(value) => QueryEvent::Raw(value),
        other => unhandled_sdk_event(&other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod claude_completion;
    mod codex_completion;

    #[test]
    fn unattended_turns_remove_interactive_integrations_and_elevated_access() {
        let turn = AgentTurn::new(
            ThreadId::new_v4(),
            "event",
            "/tmp",
            "sonnet",
            AgentProvider::Claude,
            PermissionMode::BypassPermissions,
        )
        .unattended();
        assert_eq!(turn.permission_mode, PermissionMode::Plan);
        assert_eq!(turn.thread_access, ThreadAccess::Off);
        for provider in [AgentProvider::Claude, AgentProvider::Codex] {
            let mut options = build_query_options(
                provider,
                "event",
                "/tmp",
                "test",
                PermissionMode::BypassPermissions,
                None,
                None,
            );
            restrict_unattended(&mut options, &BruhAccess::default(), false, &[], false);
            let environment = match &options {
                ProviderQueryOptions::Claude(options) => &options.env,
                ProviderQueryOptions::Codex(options) => &options.env,
                ProviderQueryOptions::External(_) => panic!("expected a managed provider"),
            };
            for key in ["USER", "USERNAME"] {
                if let Ok(value) = std::env::var(key) {
                    assert!(
                        environment
                            .iter()
                            .any(|(name, actual)| name == key && actual == &value),
                        "Unattended login must retain the account identity {key}"
                    );
                }
            }
            match options {
                ProviderQueryOptions::Claude(options) => {
                    assert!(!options.inherit_env);
                    for key in ["DISABLE_TELEMETRY", "DISABLE_ERROR_REPORTING"] {
                        assert!(options.env.contains(&(key.into(), "1".into())));
                    }
                    assert_eq!(options.tools, claude_sdk_rs::Tools::ReadOnly);
                    assert_eq!(
                        options.permission_mode,
                        Some(claude_sdk_rs::PermissionMode::Plan)
                    );
                    assert_eq!(options.setting_sources, Some(Vec::new()));
                    assert!(options.mcp_config.is_none());
                    let settings: serde_json::Value =
                        serde_json::from_str(options.settings.as_deref().unwrap()).unwrap();
                    assert!(settings.get("hooks").is_none());
                    assert!(settings["permissions"].get("allow").is_none());
                }
                ProviderQueryOptions::Codex(options) => {
                    assert!(!options.inherit_env);
                    assert!(options.ignore_user_config);
                    assert_eq!(
                        options.access,
                        codex_sdk_rs::CodexAccess::Sandboxed {
                            sandbox: codex_sdk_rs::SandboxMode::ReadOnly,
                            approval: codex_sdk_rs::ApprovalPolicy::Never
                        }
                    );
                    assert!(!options.enable_web_search);
                    assert_eq!(options.config.len(), 5);
                    assert!(options
                        .config
                        .contains(&codex_sdk_rs::ConfigOverride::disable_analytics()));
                    assert!(options
                        .config
                        .iter()
                        .any(|entry| entry.key() == "mcp_servers" && entry.value() == "{}"));
                }
                ProviderQueryOptions::External(_) => panic!("expected a managed provider"),
            }
        }
    }

    fn bruh_access(level: AccessLevel, can_deliver: bool) -> BruhAccess {
        BruhAccess {
            level,
            can_deliver,
            ..Default::default()
        }
    }

    /// The `--settings` an unattended Claude turn at `access` runs with.
    fn unattended_claude(access: &BruhAccess) -> (claude_sdk_rs::QueryOptions, serde_json::Value) {
        let mut options = options(
            AgentProvider::Claude,
            PermissionMode::BypassPermissions,
            None,
        );
        restrict_unattended(&mut options, access, false, &[], false);
        let options = claude(options);
        let settings = serde_json::from_str(options.settings.as_deref().unwrap()).unwrap();
        (options, settings)
    }

    #[test]
    fn unattended_access_sets_claude_tools_modes_and_deny_rules() {
        let reads = claude_sdk_rs::Tools::Allow(
            UNATTENDED_READ_TOOLS
                .iter()
                .map(|t| t.to_string())
                .collect(),
        );
        for (level, can_deliver, mode, tools, push_rule) in [
            (
                AccessLevel::ReadOnly,
                false,
                PermissionMode::Plan,
                claude_sdk_rs::Tools::ReadOnly,
                true,
            ),
            (
                AccessLevel::Ask,
                false,
                PermissionMode::Default,
                reads.clone(),
                true,
            ),
            // Full runs in Default mode too: every edit reaches the gate.
            (
                AccessLevel::Full,
                false,
                PermissionMode::Default,
                reads.clone(),
                true,
            ),
            (
                AccessLevel::Full,
                true,
                PermissionMode::Default,
                reads,
                false,
            ),
        ] {
            let access = bruh_access(level, can_deliver);
            let turn = AgentTurn::new(
                ThreadId::new_v4(),
                "event",
                "/tmp",
                "sonnet",
                AgentProvider::Claude,
                PermissionMode::BypassPermissions,
            )
            .unattended_with(access.clone());
            assert_eq!(turn.permission_mode, mode, "{level:?}");
            assert_eq!(turn.unattended_access, access);
            assert!(turn.unattended && turn.lean);
            assert_eq!(turn.thread_access, ThreadAccess::Off);

            let (options, settings) = unattended_claude(&access);
            assert_eq!(options.tools, tools, "{level:?}");
            assert_eq!(
                options.permission_mode,
                Some(claude_permission_mode(mode)),
                "{level:?}"
            );
            assert!(options.permission_prompts, "{level:?} must route asks");
            assert!(!options.inherit_env);
            assert_eq!(options.setting_sources, Some(Vec::new()));
            assert!(options.mcp_config.is_none());
            // No MCP servers at any level: not the user's, a `.mcp.json` the
            // bruh wrote, or claude.ai connectors.
            assert!(options.strict_mcp_config, "{level:?} starts MCP servers");
            assert!(settings.get("hooks").is_none());
            assert!(settings["permissions"].get("allow").is_none());
            let deny: Vec<&str> = settings["permissions"]["deny"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect();
            assert!(deny.contains(&"mcp__*"), "{level:?}");
            assert_eq!(
                deny.contains(&"Bash(git push:*)"),
                push_rule,
                "{level:?} can_deliver={can_deliver}"
            );
            // Only Full's shell runs without asking, so only it is sandboxed.
            assert_eq!(
                settings.get("sandbox").is_some(),
                level == AccessLevel::Full,
                "{level:?}"
            );
        }
    }

    /// F18: a Full bruh's shell runs in Claude's bash sandbox, confined to
    /// its working folder, failing closed where the sandbox can't start, and
    /// with no unsandboxed fallback. Every command still reaches the bruh's
    /// permission gate. Forge hosts are reachable only with Can deliver on.
    #[test]
    fn unattended_full_claude_confines_its_shell_to_the_working_folder() {
        for can_deliver in [false, true] {
            let access = bruh_access(AccessLevel::Full, can_deliver);
            assert!(unattended_shell_confined(&access, AgentProvider::Claude));
            let (_, settings) = unattended_claude(&access);
            let sandbox = &settings["sandbox"];
            assert_eq!(sandbox["enabled"], true);
            assert_eq!(sandbox["failIfUnavailable"], true);
            assert_eq!(sandbox["allowUnsandboxedCommands"], false);
            assert_eq!(sandbox["autoAllowBashIfSandboxed"], false);
            let (open, closed) = if can_deliver {
                ("allowedDomains", "deniedDomains")
            } else {
                ("deniedDomains", "allowedDomains")
            };
            let hosts: Vec<&str> = sandbox["network"][open]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect();
            assert_eq!(hosts, FORGE_HOSTS, "can_deliver={can_deliver}");
            assert!(sandbox["network"].get(closed).is_none());
        }
        for (level, provider) in [
            (AccessLevel::Ask, AgentProvider::Claude),
            (AccessLevel::ReadOnly, AgentProvider::Claude),
            (AccessLevel::Full, AgentProvider::Codex),
            (AccessLevel::Full, AgentProvider::Cursor),
        ] {
            assert!(
                !unattended_shell_confined(&bruh_access(level, true), provider),
                "{level:?} on {provider:?}"
            );
        }
    }

    /// The prefix deny rules miss `git -C dir push`, so delivery must not
    /// rest on them: the sandbox sends every command through the gate, which
    /// denies it, and the forge hosts stay unreachable without Can deliver.
    #[test]
    fn a_confined_full_shell_still_cannot_deliver_without_can_deliver() {
        let push = serde_json::json!({"command": "git -C repo push origin main"});
        let run = |access: &BruhAccess, input: &serde_json::Value| {
            bruhs_models::crew::permission_gate(
                access,
                AgentProvider::Claude,
                "Bash",
                input,
                "/tmp/work",
                unattended_shell_confined(access, AgentProvider::Claude),
            )
        };
        let without = bruh_access(AccessLevel::Full, false);
        let (_, settings) = unattended_claude(&without);
        assert_eq!(settings["sandbox"]["autoAllowBashIfSandboxed"], false);
        assert_eq!(
            run(&without, &push),
            Gate::Deny(bruhs_models::crew::DELIVERY_DENIAL)
        );
        assert_eq!(
            run(&without, &serde_json::json!({"command": "cargo test"})),
            Gate::Allow
        );
        assert!(settings["sandbox"]["network"]["deniedDomains"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("github.com")));
        assert_eq!(
            run(&bruh_access(AccessLevel::Full, true), &push),
            Gate::Allow
        );
    }

    /// A bruh with its Computer on starts one MCP server, the workbench
    /// helper bound to it, and nothing pre-approves its tool: every browser
    /// call reaches the permission handler and the gate.
    #[test]
    fn a_bruh_turn_with_computer_gets_only_the_browser_server_and_no_preapproval() {
        let full = bruh_access(AccessLevel::Full, true);
        let browser = crate::workbench_tools::BruhTools {
            agent_id: "a".into(),
            browser: true,
            apps: false,
            mail: false,
        };
        for level in [AccessLevel::Ask, AccessLevel::Full] {
            let access = bruh_access(level, true);
            let mut options = options(
                AgentProvider::Claude,
                PermissionMode::BypassPermissions,
                None,
            );
            restrict_unattended(&mut options, &access, true, &[], false);
            // A server already there is replaced, not merged.
            if let ProviderQueryOptions::Claude(o) = &mut options {
                o.mcp_config = Some(
                    serde_json::json!({"mcpServers":{"other":{"command":"other"}}}).to_string(),
                );
            }
            crate::workbench_tools::configure_bruh(&mut options, &browser).unwrap();
            let options = claude(options);
            let config: serde_json::Value =
                serde_json::from_str(options.mcp_config.as_deref().unwrap()).unwrap();
            let servers = config["mcpServers"].as_object().unwrap();
            assert_eq!(
                servers.keys().collect::<Vec<_>>(),
                ["bruhs_workbench"],
                "{level:?}"
            );
            let server = &servers["bruhs_workbench"];
            assert_eq!(server["env"]["BRUHS_WORKBENCH_BRUH"], "a");
            assert_eq!(server["env"]["BRUHS_WORKBENCH_BRUH_TOOLS"], "browser");
            assert_eq!(server["args"], serde_json::json!(["--workbench-mcp"]));
            // Only the endpoint, the bruh and its tools: no thread, project
            // or mode scope.
            let mut env: Vec<_> = server["env"].as_object().unwrap().keys().collect();
            env.sort();
            assert_eq!(
                env,
                [
                    "BRUHS_WORKBENCH_BRUH",
                    "BRUHS_WORKBENCH_BRUH_TOOLS",
                    "BRUHS_WORKBENCH_URL"
                ]
            );
            assert!(options.strict_mcp_config);
            // Pre-approved read tools only, never `ReadOnly`, which drops
            // every server.
            assert_eq!(
                options.tools,
                claude_sdk_rs::Tools::Allow(UNATTENDED_READ_TOOLS.map(String::from).to_vec()),
                "{level:?}"
            );
            let settings: serde_json::Value =
                serde_json::from_str(options.settings.as_deref().unwrap()).unwrap();
            assert!(!settings["permissions"]["deny"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("mcp__*")));
        }
        // A Computer needs Claude: other providers get no browser server.
        let mut codex = self::options(
            AgentProvider::Codex,
            PermissionMode::BypassPermissions,
            None,
        );
        restrict_unattended(&mut codex, &full, true, &[], false);
        let error = crate::workbench_tools::configure_bruh(&mut codex, &browser).unwrap_err();
        assert_eq!(error.to_string(), bruhs_models::crew::NEEDS_CLAUDE);
        assert!(codex_mcp_servers(&codex).is_empty());
    }

    /// The `mcp_servers.*` overrides a Codex turn carries.
    fn codex_mcp_servers(options: &ProviderQueryOptions) -> Vec<String> {
        match options {
            ProviderQueryOptions::Codex(o) => o
                .config
                .iter()
                .map(|entry| entry.key().to_string())
                .filter(|key| key.starts_with("mcp_servers."))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// A bruh never reads a browser profile, at any level and with its
    /// Computer on or off (the profile outlives Computer off): the Read and
    /// Edit deny rules hold for Read, Grep, Glob and the edit tools, and
    /// Claude's sandbox adds them to Full's shell.
    #[test]
    fn every_bruh_turn_denies_the_hidden_folders() {
        let hidden = [
            std::path::PathBuf::from("/data/bruhs/a/browser"),
            std::path::PathBuf::from("/data/bruhs/*/browser-cache/"),
        ];
        for level in [AccessLevel::ReadOnly, AccessLevel::Ask, AccessLevel::Full] {
            for computer in [false, true] {
                let mut options = options(
                    AgentProvider::Claude,
                    PermissionMode::BypassPermissions,
                    None,
                );
                restrict_unattended(
                    &mut options,
                    &bruh_access(level, true),
                    computer,
                    &hidden,
                    false,
                );
                let options = claude(options);
                let settings: serde_json::Value =
                    serde_json::from_str(options.settings.as_deref().unwrap()).unwrap();
                let deny = settings["permissions"]["deny"].as_array().unwrap();
                for rule in [
                    "Read(//data/bruhs/a/browser/**)",
                    "Edit(//data/bruhs/a/browser/**)",
                    "Read(//data/bruhs/*/browser-cache/**)",
                    "Edit(//data/bruhs/*/browser-cache/**)",
                ] {
                    assert!(
                        deny.contains(&serde_json::json!(rule)),
                        "{level:?} computer={computer}: {rule}"
                    );
                }
            }
        }
    }

    /// Every bruh turn, at every level, is denied reading and editing the
    /// vault folders it is given (a path with spaces kept whole) and running
    /// `security` or `op`, and reading `/proc`.
    #[test]
    fn every_bruh_turn_denies_the_vault_folders() {
        let hidden = [
            std::path::PathBuf::from("/Users/a/Library/Application Support/bruhs-ai/credentials"),
            std::path::PathBuf::from("/Users/a/.config/op"),
        ];
        for level in [AccessLevel::ReadOnly, AccessLevel::Ask, AccessLevel::Full] {
            for can_deliver in [false, true] {
                let mut options = options(
                    AgentProvider::Claude,
                    PermissionMode::BypassPermissions,
                    None,
                );
                restrict_unattended(
                    &mut options,
                    &bruh_access(level, can_deliver),
                    false,
                    &hidden,
                    false,
                );
                let options = claude(options);
                let settings: serde_json::Value =
                    serde_json::from_str(options.settings.as_deref().unwrap()).unwrap();
                let deny = settings["permissions"]["deny"].as_array().unwrap();
                let rules = [
                    "Read(//Users/a/Library/Application Support/bruhs-ai/credentials/**)",
                    "Edit(//Users/a/Library/Application Support/bruhs-ai/credentials/**)",
                    "Read(//Users/a/.config/op/**)",
                    "Edit(//Users/a/.config/op/**)",
                ];
                for rule in rules
                    .into_iter()
                    .chain(bruhs_models::crew::VAULT_DENY_RULES)
                {
                    assert!(
                        deny.contains(&serde_json::json!(rule)),
                        "{level:?} can_deliver={can_deliver}: {rule}"
                    );
                }
            }
        }
    }

    #[test]
    fn without_computer_a_bruh_turn_starts_no_servers_and_denies_app_tools() {
        for level in [AccessLevel::Ask, AccessLevel::Full] {
            let mut options = options(
                AgentProvider::Claude,
                PermissionMode::BypassPermissions,
                None,
            );
            restrict_unattended(&mut options, &bruh_access(level, true), false, &[], false);
            let options = claude(options);
            assert!(options.mcp_config.is_none(), "{level:?}");
            assert!(options.strict_mcp_config, "{level:?}");
            let settings: serde_json::Value =
                serde_json::from_str(options.settings.as_deref().unwrap()).unwrap();
            assert!(
                settings["permissions"]["deny"]
                    .as_array()
                    .unwrap()
                    .contains(&serde_json::json!("mcp__*")),
                "{level:?}"
            );
        }
    }

    /// D18 and ruling F27: a bruh with a vault key keeps its connection
    /// tool denied; with Computer off, every other app tool is left to the
    /// gate instead of the blanket rule, so `request_secret` can run.
    #[test]
    fn a_bruh_with_vault_keys_denies_only_the_connection_tool() {
        let deny = |computer: bool, secrets: bool| {
            let mut options = options(
                AgentProvider::Claude,
                PermissionMode::BypassPermissions,
                None,
            );
            restrict_unattended(
                &mut options,
                &bruh_access(AccessLevel::Full, true),
                computer,
                &[],
                secrets,
            );
            let settings: serde_json::Value =
                serde_json::from_str(claude(options).settings.as_deref().unwrap()).unwrap();
            settings["permissions"]["deny"].as_array().unwrap().clone()
        };
        let connection = serde_json::json!("mcp__bruhs_secrets__request_connection");
        let blanket = serde_json::json!("mcp__*");
        let proc_rule = serde_json::json!("Read(//proc/**)");
        for computer in [false, true] {
            let with = deny(computer, true);
            assert!(with.contains(&connection), "{computer}");
            assert!(!with.contains(&blanket), "{computer}");
            assert!(with.contains(&proc_rule), "{computer}");
            let without = deny(computer, false);
            assert!(!without.contains(&connection), "{computer}");
            assert_eq!(without.contains(&blanket), !computer);
            // Without keys the settings are what they were.
            let mut expected = without.clone();
            expected.retain(|rule| rule != &blanket);
            let mut keyed = with.clone();
            keyed.retain(|rule| rule != &connection);
            assert_eq!(keyed, expected, "{computer}");
        }
    }

    /// Only an unattended Claude turn at Ask or Full whose bruh has a key
    /// gets the broker.
    #[test]
    fn only_a_keyed_claude_bruh_at_ask_or_full_gets_the_key_tool() {
        let turn = |provider: AgentProvider, level: AccessLevel, keys: bool| {
            AgentTurn::new(
                ThreadId::new_v4(),
                "go",
                "/tmp",
                "model",
                provider,
                PermissionMode::Plan,
            )
            .unattended_with(bruh_access(level, false))
            .bruh_secrets(keys)
        };
        for level in [AccessLevel::Ask, AccessLevel::Full] {
            assert!(bruh_secret_tools(&turn(AgentProvider::Claude, level, true)));
            assert!(!bruh_secret_tools(&turn(
                AgentProvider::Claude,
                level,
                false
            )));
        }
        assert!(!bruh_secret_tools(&turn(
            AgentProvider::Claude,
            AccessLevel::ReadOnly,
            true
        )));
        assert!(!bruh_secret_tools(&turn(
            AgentProvider::Codex,
            AccessLevel::Full,
            true
        )));
        // An attended turn never gets a bruh's keys.
        let attended = AgentTurn::new(
            ThreadId::new_v4(),
            "go",
            "/tmp",
            "model",
            AgentProvider::Claude,
            PermissionMode::Default,
        )
        .bruh_secrets(true);
        assert!(!bruh_secret_tools(&attended));
    }

    /// A bruh turn with app tools starts the workbench helper bound to the
    /// bruh and naming its tools, lifts the blanket `mcp__*` deny for it and
    /// keeps every vault deny rule; at Read only it starts no server.
    #[test]
    fn bruh_tools_open_the_mcp_deny_only_for_the_helper() {
        use crate::workbench_tools::BruhTools;
        let apps = BruhTools {
            agent_id: "a".into(),
            browser: false,
            apps: true,
            mail: false,
        };
        let turn = |provider: AgentProvider, level: AccessLevel| {
            AgentTurn::new(
                ThreadId::new_v4(),
                "go",
                "/tmp",
                "model",
                provider,
                PermissionMode::Plan,
            )
            .unattended_with(bruh_access(level, false))
            .bruh_tools(Some(apps.clone()))
        };
        let configure = |turn: &AgentTurn| {
            let mut options = options(turn.provider, PermissionMode::BypassPermissions, None);
            let tools = bruh_tool_set(turn);
            let configured = restrict_bruh_turn(
                &mut options,
                &turn.unattended_access,
                tools.as_ref(),
                &[],
                false,
            );
            (tools, configured, options)
        };

        let full = turn(AgentProvider::Claude, AccessLevel::Full);
        let (tools, configured, options) = configure(&full);
        assert_eq!(tools.as_ref(), Some(&apps));
        assert!(matches!(configured, Some(Ok(()))));
        let options = claude(options);
        let settings: serde_json::Value =
            serde_json::from_str(options.settings.as_deref().unwrap()).unwrap();
        let deny = settings["permissions"]["deny"].as_array().unwrap();
        assert!(!deny.contains(&serde_json::json!("mcp__*")));
        for rule in bruhs_models::crew::VAULT_DENY_RULES {
            assert!(deny.contains(&serde_json::json!(rule)), "{rule}");
        }
        let config: serde_json::Value =
            serde_json::from_str(options.mcp_config.as_deref().unwrap()).unwrap();
        let servers = config["mcpServers"].as_object().unwrap();
        assert_eq!(servers.keys().collect::<Vec<_>>(), ["bruhs_workbench"]);
        let env = &servers["bruhs_workbench"]["env"];
        assert_eq!(env["BRUHS_WORKBENCH_BRUH"], "a");
        assert_eq!(env["BRUHS_WORKBENCH_BRUH_TOOLS"], "apps");
        assert!(options.strict_mcp_config);
        // Nothing pre-approves the app tools: every call reaches the gate.
        assert_eq!(
            options.tools,
            claude_sdk_rs::Tools::Allow(UNATTENDED_READ_TOOLS.map(String::from).to_vec())
        );

        // Read only, Codex or an attended turn: no helper, no lifted deny.
        let read_only = turn(AgentProvider::Claude, AccessLevel::ReadOnly);
        let (tools, configured, options) = configure(&read_only);
        assert_eq!(tools, None);
        assert!(configured.is_none());
        let options = claude(options);
        assert_eq!(options.tools, claude_sdk_rs::Tools::ReadOnly);
        assert!(options.mcp_config.is_none());
        let settings: serde_json::Value =
            serde_json::from_str(options.settings.as_deref().unwrap()).unwrap();
        assert!(settings["permissions"]["deny"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("mcp__*")));
        assert_eq!(
            bruh_tool_set(&turn(AgentProvider::Codex, AccessLevel::Full)),
            None
        );
        let attended = AgentTurn::new(
            ThreadId::new_v4(),
            "go",
            "/tmp",
            "model",
            AgentProvider::Claude,
            PermissionMode::Default,
        )
        .bruh_tools(Some(apps.clone()));
        assert_eq!(bruh_tool_set(&attended), None);
        // A set with no tool in it starts nothing.
        let empty = AgentTurn {
            bruh_tools: Some(BruhTools {
                apps: false,
                ..apps.clone()
            }),
            ..turn(AgentProvider::Claude, AccessLevel::Full)
        };
        assert_eq!(bruh_tool_set(&empty), None);
    }

    #[test]
    fn unattended_turns_keep_every_proxy_variable() {
        let env = unattended_environment(|key| {
            matches!(
                key,
                "http_proxy"
                    | "HTTPS_PROXY"
                    | "no_proxy"
                    | "ALL_PROXY"
                    | "SECRET_TOKEN"
                    | "https_proxy"
                    | "all_proxy"
                    | "HTTP_PROXY"
                    | "NO_PROXY"
                    | "PATH"
            )
            .then(|| format!("v-{key}"))
        });
        let keys: Vec<_> = env.iter().map(|(key, _)| key.as_str()).collect();
        for key in [
            "http_proxy",
            "https_proxy",
            "no_proxy",
            "all_proxy",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "NO_PROXY",
            "ALL_PROXY",
            "PATH",
        ] {
            assert!(keys.contains(&key), "{key}");
        }
        assert!(!keys.contains(&"SECRET_TOKEN"));
        assert!(env.iter().all(|(key, value)| value == &format!("v-{key}")));
        // Nothing else that looks like a proxy rides along.
        let proxies: Vec<_> = keys
            .iter()
            .filter(|key| key.to_ascii_lowercase().contains("proxy"))
            .collect();
        assert_eq!(proxies.len(), 8, "{proxies:?}");
    }

    #[test]
    fn unattended_full_codex_writes_only_in_its_workspace() {
        let sandbox = |level| {
            let mut options = options(
                AgentProvider::Codex,
                PermissionMode::BypassPermissions,
                None,
            );
            restrict_unattended(&mut options, &bruh_access(level, true), false, &[], false);
            codex(options).access
        };
        assert_eq!(
            sandbox(AccessLevel::Full),
            CodexAccess::Sandboxed {
                sandbox: SandboxMode::WorkspaceWrite,
                approval: ApprovalPolicy::Never,
            }
        );
        for level in [AccessLevel::Ask, AccessLevel::ReadOnly] {
            assert_eq!(
                sandbox(level),
                CodexAccess::Sandboxed {
                    sandbox: SandboxMode::ReadOnly,
                    approval: ApprovalPolicy::Never,
                },
                "{level:?}"
            );
        }
        let turn = |level| {
            AgentTurn::new(
                ThreadId::new_v4(),
                "event",
                "/tmp",
                "gpt-5.5",
                AgentProvider::Codex,
                PermissionMode::BypassPermissions,
            )
            .unattended_with(bruh_access(level, false))
        };
        let ask = turn(AccessLevel::Ask);
        assert_eq!(ask.permission_mode, PermissionMode::Plan);
        assert_eq!(ask.unattended_access.level, AccessLevel::ReadOnly);
        let full = turn(AccessLevel::Full);
        assert_eq!(full.permission_mode, PermissionMode::AcceptEdits);
        assert_eq!(full.unattended_access.level, AccessLevel::Full);
        let other = AgentTurn::new(
            ThreadId::new_v4(),
            "event",
            "/tmp",
            "default",
            AgentProvider::Cursor,
            PermissionMode::BypassPermissions,
        )
        .unattended_with(bruh_access(AccessLevel::Full, true));
        assert_eq!(other.permission_mode, PermissionMode::Plan);
        assert_eq!(other.unattended_access.level, AccessLevel::ReadOnly);
    }
    use tokio_stream::StreamExt;

    fn options(
        provider: AgentProvider,
        mode: PermissionMode,
        session_id: Option<&str>,
    ) -> ProviderQueryOptions {
        let model = match provider {
            AgentProvider::Claude => "claude-sonnet-4-6",
            AgentProvider::Codex => "gpt-5.5",
            _ => "default",
        };
        build_query_options(provider, "hi", "/tmp", model, mode, None, session_id)
    }

    fn claude(options: ProviderQueryOptions) -> claude_sdk_rs::QueryOptions {
        match options {
            ProviderQueryOptions::Claude(o) => o,
            ProviderQueryOptions::Codex(_) | ProviderQueryOptions::External(_) => {
                panic!("expected claude options")
            }
        }
    }

    fn codex(options: ProviderQueryOptions) -> codex_sdk_rs::QueryOptions {
        match options {
            ProviderQueryOptions::Codex(o) => o,
            ProviderQueryOptions::Claude(_) | ProviderQueryOptions::External(_) => {
                panic!("expected codex options")
            }
        }
    }

    /// A thread's context (a meeting) is added to Claude Code's own system
    /// prompt, never swapped in for it: replacing it saved ~3.8k tokens a
    /// turn but dropped the CLI's tool-use guidance.
    #[test]
    fn claude_thread_context_is_appended_not_replacing_the_cli_prompt() {
        let o = claude(build_query_options(
            AgentProvider::Claude,
            "hi",
            "/tmp",
            "opus",
            PermissionMode::Default,
            Some("meeting notes"),
            None,
        ));
        let context = o.append_system_prompt.as_deref().unwrap();
        assert!(context.starts_with("meeting notes\n\n"));
        assert!(context.contains(bruhs_models::logic::rich_response::RESPONSE_INSTRUCTIONS));
        assert_eq!(o.system_prompt, None);
    }

    /// Codex gets the context once, in the first prompt of a new session,
    /// and leaves the user's `developer_instructions` alone.
    #[test]
    fn codex_thread_context_opens_a_new_session_only() {
        let build = |session: Option<&str>| {
            codex(build_query_options(
                AgentProvider::Codex,
                "hi",
                "/tmp",
                "gpt-5.5",
                PermissionMode::Default,
                Some("meeting notes"),
                session,
            ))
        };
        let new = build(None);
        assert!(new
            .prompt
            .ends_with("<thread_context>\nmeeting notes\n</thread_context>\n\nhi"));
        assert!(new
            .prompt
            .contains(bruhs_models::logic::rich_response::RESPONSE_INSTRUCTIONS));
        assert!(new
            .config
            .iter()
            .all(|entry| entry.key() != "developer_instructions"));
        let resumed = build(Some("thr-1")).prompt;
        assert!(!resumed.contains("meeting notes"), "already in the session");
        assert!(resumed.contains(bruhs_models::logic::rich_response::RESPONSE_INSTRUCTIONS));
        assert!(resumed.ends_with("\n\nhi"));
    }

    /// Measured on CLI 2.1.319: a turn with two requests whose `result.usage`
    /// summed to 96.4k while the window held 49.2k. Only the per-request
    /// event says how full the window is.
    #[test]
    fn claude_window_comes_from_the_last_request_not_the_turn_sum() {
        let request: claude_sdk_rs::UsageInfo = serde_json::from_value(serde_json::json!({
            "input_tokens": 2, "cache_read_input_tokens": 47_261, "cache_creation_input_tokens": 1_928,
        }))
        .unwrap();
        let QueryEvent::Usage(usage) =
            map_claude_event(claude_sdk_rs::QueryEvent::Usage(request), false)
        else {
            panic!("usage event");
        };
        assert_eq!(usage.context_tokens, Some(49_191));

        let total: claude_sdk_rs::UsageInfo = serde_json::from_value(serde_json::json!({
            "input_tokens": 4, "cache_read_input_tokens": 74_003, "cache_creation_input_tokens": 22_447,
        }))
        .unwrap();
        assert_eq!(
            UsageInfo::from(total).context_tokens,
            None,
            "a sum is no window"
        );
    }

    /// A lean Claude turn loads no settings files, so none of the plugins,
    /// skills or MCP servers they enable (−31.7% on a trivial turn in the
    /// home directory, measured); claude.ai connectors still load. A lean
    /// Codex turn keeps the user's config and drops only plugins (−25.6%).
    #[test]
    fn lean_turns_skip_claude_settings_files_and_codex_plugins() {
        let mut lean = options(AgentProvider::Claude, PermissionMode::Default, None);
        apply_lean_context(&mut lean, true);
        assert_eq!(claude(lean).setting_sources, Some(Vec::new()));

        let mut full = options(AgentProvider::Claude, PermissionMode::Default, None);
        apply_lean_context(&mut full, false);
        assert_eq!(claude(full).setting_sources, None);

        let mut codex_turn = options(AgentProvider::Codex, PermissionMode::Default, None);
        apply_lean_context(&mut codex_turn, true);
        let mut codex_turn = codex(codex_turn);
        assert!(!codex_turn.ignore_user_config, "the user's config stays");
        let overrides = |o: &codex_sdk_rs::QueryOptions| {
            o.config
                .iter()
                .map(|c| format!("{}={}", c.key(), c.value()))
                .collect::<Vec<_>>()
        };
        let expected = ["analytics.enabled=false", "features.plugins=false"];
        assert_eq!(overrides(&codex_turn), expected);
        lean_codex(&mut codex_turn);
        assert_eq!(overrides(&codex_turn), expected, "once");

        let mut full_codex = options(AgentProvider::Codex, PermissionMode::Default, None);
        apply_lean_context(&mut full_codex, false);
        assert_eq!(overrides(&codex(full_codex)), ["analytics.enabled=false"]);
    }

    #[test]
    fn untrusted_workspaces_isolate_claude_project_settings_and_mcp() {
        let mut untrusted = options(AgentProvider::Claude, PermissionMode::Default, None);
        apply_workspace_trust_flag(&mut untrusted, false);
        let untrusted_opts = claude(untrusted);
        assert_eq!(untrusted_opts.setting_sources, Some(vec!["user".into()]));
        assert!(untrusted_opts.strict_mcp_config);

        // Lean turns keep `Some([])` rather than widening to `user`.
        let mut lean = options(AgentProvider::Claude, PermissionMode::Default, None);
        apply_lean_context(&mut lean, true);
        apply_workspace_trust_flag(&mut lean, false);
        let lean_opts = claude(lean);
        assert_eq!(lean_opts.setting_sources, Some(Vec::new()));
        assert!(lean_opts.strict_mcp_config);

        // Trusted workspaces keep normal settings/MCP resolution.
        let mut trusted = options(AgentProvider::Claude, PermissionMode::Default, None);
        apply_workspace_trust_flag(&mut trusted, true);
        let trusted_opts = claude(trusted);
        assert_eq!(trusted_opts.setting_sources, None);
        assert!(!trusted_opts.strict_mcp_config);
    }

    /// A lean turn skips the settings files, so the safety parts come back
    /// through `--settings`: a deny rule there held even in bypass mode
    /// (measured on CLI 2.1.319).
    #[test]
    fn lean_turns_keep_the_users_permission_rules_and_hooks() {
        let user = serde_json::json!({
            "permissions": {"defaultMode": "acceptEdits", "deny": ["Read(~/.ssh/**)"], "allow": ["Bash(ls:*)"]},
            "hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "guard"}]}]},
            "enabledPlugins": {"example-plugin@x": true},
        });
        let local =
            serde_json::json!({"permissions": {"deny": ["Read(~/.ssh/**)", "Read(./.env)"]}});
        let mut options = claude_sdk_rs::QueryOptions::new("hi");
        lean_claude_settings(&mut options, &[user, local]);
        assert_eq!(options.setting_sources, Some(Vec::new()));
        let settings: serde_json::Value =
            serde_json::from_str(options.settings.as_deref().unwrap()).unwrap();
        assert_eq!(
            settings["permissions"]["deny"],
            serde_json::json!(["Read(~/.ssh/**)", "Read(./.env)"])
        );
        assert_eq!(
            settings["permissions"]["allow"],
            serde_json::json!(["Bash(ls:*)"])
        );
        assert!(
            settings["permissions"].get("defaultMode").is_none(),
            "bruhs picks the mode"
        );
        assert!(settings.get("enabledPlugins").is_none(), "plugins stay off");
        assert_eq!(settings["hooks"]["PreToolUse"][0]["matcher"], "Bash");

        let mut bare = claude_sdk_rs::QueryOptions::new("hi");
        lean_claude_settings(
            &mut bare,
            &[serde_json::json!({"permissions": {"defaultMode": "plan"}})],
        );
        assert_eq!(bare.settings, None, "nothing to carry");
    }

    /// The reviewer runs with no prompts, so an inherited `allow` rule would
    /// let it write: it gets deny rules, the write tools and guard hooks only.
    #[test]
    fn a_reviewer_gets_deny_rules_and_guards_but_never_allow_rules() {
        let user = serde_json::json!({
            "permissions": {"allow": ["Edit", "Bash(git commit:*)"], "deny": ["Read(~/.ssh/**)"]},
            "hooks": {"PreToolUse": [{"matcher": "Bash"}], "Stop": [{"matcher": ""}]},
        });
        let project = serde_json::json!({"permissions": {"deny": ["Read(./.env)"]}});
        let settings: serde_json::Value =
            serde_json::from_str(&reviewer_settings(&[user, project])).unwrap();
        assert_eq!(
            settings["permissions"]["deny"],
            serde_json::json!([
                "Edit",
                "Write",
                "NotebookEdit",
                "Read(~/.ssh/**)",
                "Read(./.env)"
            ])
        );
        assert!(settings["permissions"].get("allow").is_none());
        assert_eq!(
            settings["hooks"]["PreToolUse"],
            serde_json::json!([{"matcher": "Bash"}])
        );
        assert!(
            settings["hooks"].get("Stop").is_none(),
            "no side-effect hooks"
        );
    }

    #[test]
    fn full_context_opt_out_reads_only_one() {
        assert!(lean_context_allowed(None));
        assert!(lean_context_allowed(Some("0")));
        assert!(!lean_context_allowed(Some("1")));
    }

    /// Real `turn.completed` numbers from codex-cli 0.166: 19,956 input of
    /// which 13,184 cached. The context is 19,956, not 33,140.
    #[test]
    fn codex_cached_tokens_are_not_counted_twice() {
        let codex: codex_sdk_rs::UsageInfo = serde_json::from_value(serde_json::json!({
            "input_tokens": 19_956,
            "cached_input_tokens": 13_184,
            "output_tokens": 10,
            "context_input_tokens": 19_956,
        }))
        .unwrap();
        let usage = UsageInfo::from(codex);
        assert_eq!(usage.input_tokens, 6_772);
        assert_eq!(usage.cache_read_input_tokens, 13_184);
        assert_eq!(
            bruhs_models::logic::compact::context_tokens(
                usage.input_tokens,
                usage.cache_read_input_tokens,
                usage.cache_creation_input_tokens,
            ),
            19_956
        );
        assert_eq!(usage.context_tokens, Some(19_956));
    }

    #[test]
    fn claude_options_carry_resume() {
        let o = claude(options(
            AgentProvider::Claude,
            PermissionMode::Default,
            Some("sess-1"),
        ));
        assert_eq!(o.resume.as_deref(), Some("sess-1"));
    }

    #[test]
    fn codex_options_carry_resume() {
        let o = codex(options(
            AgentProvider::Codex,
            PermissionMode::Default,
            Some("thr-9"),
        ));
        assert_eq!(o.session, codex_sdk_rs::Session::Resume("thr-9".into()));
    }

    /// The provider comes from the turn, never from the model name: a bare
    /// Claude alias like `opus` used to be sent to Codex.
    #[test]
    fn the_turn_picks_the_provider_not_the_model_name() {
        let o = build_query_options(
            AgentProvider::Claude,
            "hi",
            "/tmp",
            "opus",
            PermissionMode::Default,
            None,
            None,
        );
        assert_eq!(claude(o).model.as_deref(), Some("opus"));
    }

    #[test]
    fn claude_prompts_unless_bypassing() {
        for (mode, prompts) in [
            (PermissionMode::Default, true),
            (PermissionMode::AcceptEdits, true),
            (PermissionMode::Plan, true),
            (PermissionMode::BypassPermissions, false),
        ] {
            let o = claude(options(AgentProvider::Claude, mode, None));
            assert_eq!(o.permission_prompts, prompts, "mode {mode:?}");
            assert!(o.accept_input, "a Claude turn can be steered");
            assert_eq!(
                o.tools,
                claude_sdk_rs::Tools::CliDefault,
                "no blanket allow-list"
            );
        }
    }

    #[test]
    fn permission_modes_map_to_typed_provider_settings() {
        let read_only = CodexAccess::Sandboxed {
            sandbox: SandboxMode::ReadOnly,
            approval: ApprovalPolicy::Never,
        };
        for (mode, claude_mode, codex_mode) in [
            (
                PermissionMode::Default,
                claude_sdk_rs::PermissionMode::Default,
                read_only,
            ),
            (
                PermissionMode::AcceptEdits,
                claude_sdk_rs::PermissionMode::AcceptEdits,
                CodexAccess::Sandboxed {
                    sandbox: SandboxMode::WorkspaceWrite,
                    approval: ApprovalPolicy::Never,
                },
            ),
            (
                PermissionMode::Plan,
                claude_sdk_rs::PermissionMode::Plan,
                read_only,
            ),
            (
                PermissionMode::BypassPermissions,
                claude_sdk_rs::PermissionMode::BypassPermissions,
                CodexAccess::DangerouslyUnsandboxed,
            ),
        ] {
            assert_eq!(claude_permission_mode(mode), claude_mode, "{mode:?}");
            assert_eq!(
                claude(options(AgentProvider::Claude, mode, None)).permission_mode,
                Some(claude_mode),
                "{mode:?}"
            );
            assert_eq!(codex_access(mode), codex_mode, "{mode:?}");
            assert_eq!(
                codex(options(AgentProvider::Codex, mode, None)).access,
                codex_mode,
                "{mode:?}"
            );
        }
    }

    /// Codex cannot ask the user (`codex exec` has no approval channel), so
    /// "ask before acting" must not become "write without asking".
    #[test]
    fn codex_default_mode_cannot_write() {
        assert_eq!(
            codex_access(PermissionMode::Default),
            CodexAccess::Sandboxed {
                sandbox: SandboxMode::ReadOnly,
                approval: ApprovalPolicy::Never,
            }
        );
        assert_ne!(
            codex_access(PermissionMode::Default),
            CodexAccess::default()
        );
    }

    #[test]
    fn new_codex_threads_are_saved_so_they_can_be_resumed() {
        let o = codex(options(AgentProvider::Codex, PermissionMode::Default, None));
        assert_eq!(o.session, codex_sdk_rs::Session::New);
    }

    #[test]
    fn claude_runs_drop_an_inherited_api_key_unless_opted_in() {
        let oauth_only = claude_query_options("hi", false);
        assert!(oauth_only
            .env_remove
            .iter()
            .any(|key| key == "ANTHROPIC_API_KEY"));
        let with_key = claude_query_options("hi", true);
        assert!(!with_key
            .env_remove
            .iter()
            .any(|key| key == "ANTHROPIC_API_KEY"));
    }

    #[test]
    fn claude_runs_drop_a_parent_agent_session() {
        for keep_api_key in [false, true] {
            let options = claude_query_options("hi", keep_api_key);
            for key in PARENT_SESSION_ENV {
                assert!(
                    options.env_remove.iter().any(|removed| removed == key),
                    "{key}"
                );
            }
        }
    }

    #[test]
    fn provider_turns_disable_builtin_analytics() {
        let claude = claude_query_options("hi", true);
        for key in ["DISABLE_TELEMETRY", "DISABLE_ERROR_REPORTING"] {
            assert!(claude.env.contains(&(key.into(), "1".into())));
        }

        let codex = codex(build_query_options(
            AgentProvider::Codex,
            "hi",
            "/tmp",
            "model",
            PermissionMode::Default,
            None,
            None,
        ));
        assert!(codex
            .config
            .iter()
            .any(|entry| entry.key() == "analytics.enabled" && entry.value() == "false"));
    }

    #[test]
    fn both_providers_remove_service_client_secrets_from_child_environments() {
        let claude = claude_query_options("hi", true);
        let codex = codex(build_query_options(
            AgentProvider::Codex,
            "hi",
            "/tmp",
            "model",
            PermissionMode::Default,
            None,
            None,
        ));
        for key in crate::connections::PRIVATE_ENV {
            assert!(claude.env_remove.iter().any(|removed| removed == key));
            assert!(codex.env_remove.iter().any(|removed| removed == key));
        }
    }

    #[test]
    fn transient_codex_stream_errors_do_not_end_the_turn() {
        let event = map_codex_event(
            codex_sdk_rs::QueryEvent::StreamError("Reconnecting... 1/5".into()),
            "gpt-5.5",
        );
        assert!(
            matches!(&event, QueryEvent::Raw(v) if v["type"] == "error" && v["message"] == "Reconnecting... 1/5"),
            "{event:?}"
        );
        assert!(!is_error_event(&event));
    }

    #[tokio::test]
    async fn resolving_unknown_permission_is_an_error() {
        let orch = AgentOrchestrator::new(Arc::new(BackgroundTaskRegistry::new()));
        assert!(matches!(
            orch.resolve_permission("nope", PermissionDecision::Allow)
                .await,
            Err(PermissionResolveError::UnknownRequest)
        ));
        assert!(orch
            .pending_permissions(&ThreadId::new_v4())
            .await
            .is_empty());
    }

    #[test]
    fn a_fresh_session_does_not_resume_a_remembered_one() {
        assert_eq!(
            session_to_resume(true, Some("requested".into()), Some("remembered".into())),
            None
        );
        assert_eq!(
            session_to_resume(false, Some("requested".into()), Some("remembered".into()))
                .as_deref(),
            Some("requested")
        );
        assert_eq!(
            session_to_resume(false, None, Some("remembered".into())).as_deref(),
            Some("remembered")
        );
        assert_eq!(session_to_resume(false, None, None), None);
    }

    #[test]
    fn selected_effort_reaches_the_native_provider() {
        let mut claude_opts = options(AgentProvider::Claude, PermissionMode::Default, None);
        apply_effort(&mut claude_opts, Some("high"));
        assert_eq!(claude(claude_opts).effort.as_deref(), Some("high"));

        let mut unset = options(AgentProvider::Claude, PermissionMode::Default, None);
        apply_effort(&mut unset, None);
        assert_eq!(claude(unset).effort, None);

        let mut unknown = options(AgentProvider::Claude, PermissionMode::Default, None);
        apply_effort(&mut unknown, Some("minimal"));
        assert_eq!(claude(unknown).effort, None);

        for level in bruhs_models::logic::run_place::CLAUDE_EFFORTS {
            let mut codex_opts = options(AgentProvider::Codex, PermissionMode::Default, None);
            apply_effort(&mut codex_opts, Some(level));
            let ProviderQueryOptions::Codex(codex_opts) = codex_opts else {
                panic!("expected Codex options");
            };
            assert!(codex_opts
                .config
                .contains(&codex_sdk_rs::ConfigOverride::reasoning_effort(level)));
        }

        for level in [None, Some("nope"), Some("ultra")] {
            let mut codex_opts = options(AgentProvider::Codex, PermissionMode::Default, None);
            apply_effort(&mut codex_opts, level);
            let ProviderQueryOptions::Codex(codex_opts) = codex_opts else {
                panic!("expected Codex options");
            };
            assert!(codex_opts
                .config
                .iter()
                .all(|entry| entry.key() != "model_reasoning_effort"));
        }

        let turn = unstartable_turn(&ThreadId::new_v4()).effort(Some("xhigh"));
        assert_eq!(turn.effort.as_deref(), Some("xhigh"));
        assert_eq!(turn.effort(Some("nope")).effort, None);
    }

    #[test]
    fn no_session_means_no_resume() {
        let o = claude(options(
            AgentProvider::Claude,
            PermissionMode::Default,
            None,
        ));
        assert!(o.resume.is_none());
    }

    /// A Claude turn whose working directory does not exist: the CLI cannot
    /// be spawned, so these tests never run a real `claude`.
    fn unstartable_turn(thread_id: &ThreadId) -> AgentTurn {
        let missing = std::env::temp_dir()
            .join(format!("bruhs-missing-{}", uuid::Uuid::new_v4()))
            .to_string_lossy()
            .into_owned();
        AgentTurn::new(
            thread_id.clone(),
            "hi",
            missing,
            "claude-sonnet-4-6",
            AgentProvider::Claude,
            PermissionMode::Plan,
        )
    }

    #[tokio::test]
    async fn a_second_send_while_streaming_is_rejected() {
        let orch = AgentOrchestrator::new(Arc::new(BackgroundTaskRegistry::new()));
        let thread = ThreadId::new_v4();
        assert_eq!(orch.thread_status(&thread), ThreadStatus::Idle);

        let _stream = orch.send_message(unstartable_turn(&thread), None).unwrap();
        assert_eq!(orch.thread_status(&thread), ThreadStatus::Streaming);
        match orch.send_message(unstartable_turn(&thread), None) {
            Err(SendMessageError::ThreadBusy(id)) => assert_eq!(id, thread),
            Err(error) => panic!("unexpected concurrent-send refusal: {error}"),
            Ok(_) => panic!("a concurrent send must be refused"),
        }

        // Other threads are unaffected.
        let other = ThreadId::new_v4();
        assert!(orch.send_message(unstartable_turn(&other), None).is_ok());
    }

    #[tokio::test]
    async fn the_thread_is_idle_once_the_provider_ends() {
        let orch = AgentOrchestrator::new(Arc::new(BackgroundTaskRegistry::new()));
        let thread = ThreadId::new_v4();
        let mut stream = orch.send_message(unstartable_turn(&thread), None).unwrap();

        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            events.push(event);
        }
        assert!(
            matches!(
                events.as_slice(),
                [OrchestratorEvent::Query(QueryEvent::Error(_))]
            ),
            "{events:?}"
        );
        // The caller still holds the finished stream; the thread is free.
        assert_eq!(orch.thread_status(&thread), ThreadStatus::Idle);
        assert!(orch.send_message(unstartable_turn(&thread), None).is_ok());
    }

    #[tokio::test]
    async fn dropping_the_stream_frees_the_thread_at_once() {
        let orch = AgentOrchestrator::new(Arc::new(BackgroundTaskRegistry::new()));
        let thread = ThreadId::new_v4();
        let stream = orch.send_message(unstartable_turn(&thread), None).unwrap();
        drop(stream);
        assert_eq!(orch.thread_status(&thread), ThreadStatus::Idle);

        assert!(orch.send_message(unstartable_turn(&thread), None).is_ok());
    }

    /// A guard from an abandoned turn, dropped after a newer turn started,
    /// must leave the newer turn running.
    #[test]
    fn a_stale_guard_does_not_release_a_newer_turn() {
        let sessions: Sessions = Arc::new(Mutex::new(HashMap::new()));
        let thread = ThreadId::new_v4();
        lock_sessions(&sessions).insert(
            thread.clone(),
            ThreadSession {
                process_id: None,
                status: ThreadStatus::Streaming,
                turn: 2,
                session: None,
            },
        );
        let guard = |turn| TurnGuard {
            sessions: Arc::clone(&sessions),
            thread_id: thread.clone(),
            turn,
        };
        drop(guard(1));
        assert_eq!(
            lock_sessions(&sessions)[&thread].status,
            ThreadStatus::Streaming
        );
        drop(guard(2));
        assert_eq!(lock_sessions(&sessions)[&thread].status, ThreadStatus::Idle);
    }

    /// The producer of turn `turn` on `thread`, as `send_message` builds it.
    pub(super) fn turn_context(
        sessions: &Sessions,
        registry: &Arc<BackgroundTaskRegistry>,
        thread: &ThreadId,
        turn: u64,
    ) -> (TurnContext, mpsc::Receiver<OrchestratorEvent>) {
        let (tx, rx) = mpsc::channel(16);
        let context = TurnContext {
            model: "test".into(),
            requested_speed: Default::default(),
            usage_receipt: "test".into(),
            media_project: "/tmp".into(),
            response_text: Mutex::new(String::new()),
            _guard: TurnGuard {
                sessions: Arc::clone(sessions),
                thread_id: thread.clone(),
                turn,
            },
            tx,
            registry: Arc::clone(registry),
            sessions: Arc::clone(sessions),
            owner: TurnOwner {
                thread_id: thread.clone(),
                turn,
            },
            provider: AgentProvider::Claude,
            tasks: Mutex::new(Vec::new()),
            tool_tasks: Mutex::new(HashMap::new()),
            status_checks: Mutex::new(std::collections::HashSet::new()),
            pending_commands: Mutex::new(HashMap::new()),
            task_status_checks: Mutex::new(HashMap::new()),
        };
        (context, rx)
    }

    fn background_bash(id: &str) -> QueryEvent {
        QueryEvent::ToolCall {
            id: id.into(),
            tool: "Bash".into(),
            input: serde_json::json!({ "command": "sleep 60", "run_in_background": true }),
        }
    }

    /// An abandoned turn's producer may tear down after the next turn on the
    /// thread has started. It must release only what it created itself: the
    /// newer turn's permission prompt, session and background tasks stay.
    #[tokio::test]
    async fn an_abandoned_turn_tears_down_only_its_own_state() {
        let sessions: Sessions = Arc::new(Mutex::new(HashMap::new()));
        let registry = Arc::new(BackgroundTaskRegistry::new());
        let thread = ThreadId::new_v4();
        lock_sessions(&sessions).insert(
            thread.clone(),
            ThreadSession {
                process_id: None,
                status: ThreadStatus::Streaming,
                turn: 1,
                session: None,
            },
        );
        let (old, _old_rx) = turn_context(&sessions, &registry, &thread, 1);
        assert!(old.forward(background_bash("old-task")).await);

        // The client abandons turn 1 and turn 2 starts on the same thread.
        {
            let mut guard = lock_sessions(&sessions);
            let session = guard.get_mut(&thread).unwrap();
            session.turn = 2;
            session.status = ThreadStatus::Streaming;
            drop(guard);
        }
        let (new, _new_rx) = turn_context(&sessions, &registry, &thread, 2);
        new.register_process(Some(42));
        old.register_process(Some(41));
        assert_eq!(lock_sessions(&sessions)[&thread].process_id, Some(42));
        assert!(new.forward(background_bash("new-task")).await);
        new.remember_session(&QueryEvent::Init {
            session_id: "new-session".into(),
            model: None,
            tools: Vec::new(),
            mcp_servers: Vec::new(),
        });
        let mut permissions: PermissionMap<()> = HashMap::new();
        for (id, owner) in [("old-prompt", &old.owner), ("new-prompt", &new.owner)] {
            permissions.insert(
                id.into(),
                PendingPermission {
                    owner: owner.clone(),
                    input: serde_json::Value::Null,
                    can_answer: false,
                    question_files: Vec::new(),
                    events: mpsc::channel(1).0,
                    responder: (),
                },
            );
        }

        // Now the old producer notices it was abandoned and tears down.
        old.remember_session(&QueryEvent::Result {
            result: String::new(),
            session_id: "old-session".into(),
            duration_ms: 0,
            usage: UsageInfo::default(),
            is_error: false,
        });
        release_turn_permissions(&mut permissions, &old.owner);
        old.finish(false);

        assert_eq!(
            permissions.keys().collect::<Vec<_>>(),
            [&"new-prompt".to_string()]
        );
        assert_eq!(
            lock_sessions(&sessions)[&thread].session,
            Some((AgentProvider::Claude, "new-session".into()))
        );
        let statuses: HashMap<String, crate::models::TaskStatus> = registry
            .get_by_thread(thread.as_str())
            .into_iter()
            .map(|task| (task.id.clone(), task.status))
            .collect();
        let old_ids = old.tasks.lock().unwrap().clone();
        assert!(old_ids.is_empty(), "finish settles the turn's tasks once");
        let new_ids = new.tasks.lock().unwrap().clone();
        assert_eq!(new_ids.len(), 1);
        assert_eq!(statuses[&new_ids[0]], crate::models::TaskStatus::Running);
        assert_eq!(
            statuses
                .iter()
                .filter(|(id, _)| **id != new_ids[0])
                .map(|(_, status)| status.clone())
                .collect::<Vec<_>>(),
            [crate::models::TaskStatus::Killed]
        );
    }

    #[tokio::test]
    async fn swarm_and_async_subagents_stay_running_after_turn_ends_until_killed_or_idle() {
        let sessions: Sessions = Arc::new(Mutex::new(HashMap::new()));
        let registry = Arc::new(BackgroundTaskRegistry::new());
        let thread = ThreadId::new_v4();

        // Turn 1 spawns two sharded swarm workers and an async Antigravity subagent.
        let (turn1, _rx1) = turn_context(&sessions, &registry, &thread, 1);
        assert!(
            turn1
                .forward(QueryEvent::ToolCall {
                    id: "sw-1".into(),
                    tool: "swarm".into(),
                    input: serde_json::json!({
                        "action": "add",
                        "name": "builder",
                        "tasks": ["crate-a", "crate-b"],
                        "brief": "Implement features",
                        "model": "gemini-4-neon"
                    }),
                })
                .await
        );
        assert!(
            turn1
                .forward(QueryEvent::ToolCall {
                    id: "sub-1".into(),
                    tool: "invoke_subagent".into(),
                    input: serde_json::json!({
                        "Role": "Codebase Researcher",
                        "TypeName": "research",
                        "Prompt": "Survey workspace"
                    }),
                })
                .await
        );
        assert!(
            turn1
                .forward(QueryEvent::ToolResult {
                    tool_use_id: "sub-1".into(),
                    content: r#"{"conversationId":"conv-42"}"#.into(),
                    is_error: false,
                })
                .await
        );
        // Also spawn a synchronous Claude Task subagent that finishes in-turn.
        assert!(
            turn1
                .forward(QueryEvent::ToolCall {
                    id: "sync-1".into(),
                    tool: "Task".into(),
                    input: serde_json::json!({ "description": "Quick check" }),
                })
                .await
        );
        assert!(
            turn1
                .forward(QueryEvent::ToolResult {
                    tool_use_id: "sync-1".into(),
                    content: "All good".into(),
                    is_error: false,
                })
                .await
        );

        // Turn 1 ends normally while teamwork & async subagent keep running.
        turn1.finish(true);
        assert!(
            registry.has_running_for_thread(thread.as_str()),
            "thread stays Working while async swarm/subagent tasks run"
        );
        let running: Vec<_> = registry
            .get_by_thread(thread.as_str())
            .into_iter()
            .filter(|t| t.status == TaskStatus::Running)
            .collect();
        assert_eq!(running.len(), 3);
        assert_eq!(
            running.iter().filter(|t| t.kind == TaskKind::Swarm).count(),
            2
        );
        assert_eq!(
            running
                .iter()
                .filter(|t| t.kind == TaskKind::Subagent)
                .count(),
            1
        );

        // Turn 2 checks swarm status (builder-1 waiting) and kills builder-2 + subagent.
        let (turn2, _rx2) = turn_context(&sessions, &registry, &thread, 2);
        assert!(
            turn2
                .forward(QueryEvent::ToolCall {
                    id: "st-1".into(),
                    tool: "swarm".into(),
                    input: serde_json::json!({ "action": "status" }),
                })
                .await
        );
        assert!(
            turn2
                .forward(QueryEvent::ToolResult {
                    tool_use_id: "st-1".into(),
                    content: r#"{"members":[{"name":"builder-1","state":"waiting"},{"name":"builder-2","state":"working"}]}"#.into(),
                    is_error: false,
                })
                .await
        );
        assert!(
            turn2
                .forward(QueryEvent::ToolCall {
                    id: "kill-1".into(),
                    tool: "swarm".into(),
                    input: serde_json::json!({ "action": "kill", "name": "builder-2" }),
                })
                .await
        );
        assert!(
            turn2
                .forward(QueryEvent::ToolCall {
                    id: "kill-sub".into(),
                    tool: "manage_subagents".into(),
                    input: serde_json::json!({ "Action": "kill", "ConversationIds": ["conv-42"] }),
                })
                .await
        );
        turn2.finish(true);

        assert!(
            !registry.has_running_for_thread(thread.as_str()),
            "all workers and subagents have finished or been killed"
        );
    }

    #[tokio::test]
    async fn backgrounded_run_command_stays_working_until_wakeup_or_kill() {
        let sessions: Sessions = Arc::new(Mutex::new(HashMap::new()));
        let registry = Arc::new(BackgroundTaskRegistry::new());
        let thread = ThreadId::new_v4();

        // A run_command with WaitMsBeforeAsync=10000 spills over into a background task.
        let (turn1, _rx1) = turn_context(&sessions, &registry, &thread, 1);
        assert!(
            turn1
                .forward(QueryEvent::ToolCall {
                    id: "cmd-1".into(),
                    tool: "run_command".into(),
                    input: serde_json::json!({
                        "CommandLine": "cargo test -p bruhs-models",
                        "WaitMsBeforeAsync": 10000,
                        "toolSummary": "Run tests and clippy"
                    }),
                })
                .await
        );
        assert!(
            turn1
                .forward(QueryEvent::ToolResult {
                    tool_use_id: "cmd-1".into(),
                    content: "Tool is running as a background task with task id: conv-1/task-186\nTask logs are available at: file:///tmp/task-186.log".into(),
                    is_error: false,
                })
                .await
        );
        // Coordinator ends its turn while waiting for the background command.
        turn1.finish(true);

        assert!(registry.has_running_for_thread(thread.as_str()));
        let running = registry.get_by_thread(thread.as_str());
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].label, "Run tests and clippy");
        assert_eq!(running[0].status, TaskStatus::Running);
        assert_eq!(
            running[0].meta["external_task_id"].as_str(),
            Some("conv-1/task-186")
        );
        assert_eq!(
            running[0].meta["log_path"].as_str(),
            Some("/tmp/task-186.log")
        );

        // Next turn arrives with the reactive wakeup notification.
        reconcile_prompt_completions(
            &registry,
            thread.as_str(),
            r#"Task id "conv-1/task-186" finished with result: Command exited with code 0."#,
        );
        assert!(!registry.has_running_for_thread(thread.as_str()));
        let settled = registry.get_by_thread(thread.as_str());
        assert_eq!(settled[0].status, TaskStatus::Exited);
        assert_eq!(settled[0].exit_code, Some(0));
    }
}
