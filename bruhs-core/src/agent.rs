//! Runs agent turns: spawns the provider CLI (Claude Code or Codex) for a
//! thread, streams its events, tracks each thread's turn state and answers
//! the tool-permission prompts a Claude turn raises.

use crate::models::{BackgroundTask, TaskKind};
use crate::registry::BackgroundTaskRegistry;
use bruhs_models::agent::{AgentProvider, PermissionMode};
use bruhs_models::ids::ThreadId;
use codex_sdk_rs::{ApprovalPolicy, CodexAccess, SandboxMode};
use futures_core::Stream;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};
use tokio::sync::{mpsc, RwLock};

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
    /// Provider session to resume, so earlier turns stay in the model's
    /// context. `None` resumes the session this thread's last turn with the
    /// same provider reported, if there was one.
    pub session_id: Option<String>,
    /// Drop the remembered session and start a new one. A rewind uses this
    /// so the removed turns are not resumed.
    pub fresh_session: bool,
    /// Claude `--effort`. `None` omits the flag. Codex ignores it.
    pub effort: Option<String>,
    /// Claude `--autocompact`. `None` omits the flag. Codex ignores it.
    pub autocompact: Option<String>,
    /// Run with a lean context (see [`AgentTurn::lean`]).
    pub lean: bool,
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
            session_id: None,
            fresh_session: false,
            effort: None,
            autocompact: None,
            lean: false,
        }
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

    /// Keep a Claude effort level. Anything outside the CLI list is dropped.
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

/// Why a turn was not started.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SendMessageError {
    /// The thread already has a turn in flight.
    #[error("thread {0} is already running a turn")]
    ThreadBusy(ThreadId),
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
    },
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
}

// ---------------------------------------------------------------------------
// OrchestratorEventStream
// ---------------------------------------------------------------------------

/// The events of one turn. Dropping it abandons the turn: the provider CLI
/// is stopped and the thread becomes [`ThreadStatus::Idle`] at once.
pub struct OrchestratorEventStream {
    rx: mpsc::Receiver<OrchestratorEvent>,
    _secrets: crate::secrets::SecretGuard,
    _turn: TurnGuard,
}

impl Stream for OrchestratorEventStream {
    type Item = OrchestratorEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

// ---------------------------------------------------------------------------
// ThreadSession
// ---------------------------------------------------------------------------

struct ThreadSession {
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
    /// The CLI's stdin was gone when the answer was written. The cause is
    /// in the message because HTTP callers only show `Display`.
    #[error("could not deliver the permission decision: {0}")]
    Transport(#[source] claude_sdk_rs::Error),
}

pub(crate) enum ProviderQueryOptions {
    Claude(claude_sdk_rs::QueryOptions),
    Codex(codex_sdk_rs::QueryOptions),
}

// ---------------------------------------------------------------------------
// AgentOrchestrator
// ---------------------------------------------------------------------------

pub struct AgentOrchestrator {
    secrets: crate::secrets::SecretManager,
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
            registry,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            permissions: Arc::new(RwLock::new(HashMap::new())),
            steers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn secrets(&self) -> &crate::secrets::SecretManager {
        &self.secrets
    }

    pub(crate) fn with_connections(mut self, connections: crate::connections::Connections) -> Self {
        self.secrets = self.secrets.with_connections(connections);
        self
    }

    pub(crate) fn with_secret_store(mut self, store: crate::secrets::SecretStore) -> Self {
        self.secrets = crate::secrets::SecretManager::new(store);
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
        let pending = self
            .permissions
            .write()
            .await
            .remove(request_id)
            .ok_or(PermissionResolveError::UnknownRequest)?;
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

    /// Ids of permission prompts a thread is currently blocked on.
    pub async fn pending_permissions(&self, thread_id: &ThreadId) -> Vec<String> {
        self.permissions
            .read()
            .await
            .iter()
            .filter(|(_, p)| &p.owner.thread_id == thread_id)
            .map(|(id, _)| id.clone())
            .collect()
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
        let AgentTurn {
            thread_id,
            prompt,
            project_path,
            model,
            provider,
            permission_mode,
            session_id,
            fresh_session,
            effort,
            autocompact,
            lean,
        } = turn;

        // Claim the thread and pick the session to resume, atomically.
        let (turn_number, resume) = {
            let mut sessions = lock_sessions(&self.sessions);
            let session = sessions
                .entry(thread_id.clone())
                .or_insert_with(|| ThreadSession {
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
            let remembered = (!fresh_session)
                .then(|| {
                    session
                        .session
                        .as_ref()
                        .filter(|(owner, _)| *owner == provider)
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
        apply_claude_effort(&mut query_options, effort.as_deref());
        apply_claude_autocompact(&mut query_options, autocompact.as_deref());
        // A Codex session keeps the plugin setting it started with.
        let lean_file = crate::codex_lean::default_file();
        let lean = match provider {
            AgentProvider::Claude => lean,
            AgentProvider::Codex => {
                crate::codex_lean::turn_is_lean(lean_file.as_deref(), resume.as_deref(), lean)
            }
        };
        let record_lean = (provider == AgentProvider::Codex && lean && resume.is_none())
            .then_some(lean_file)
            .flatten();
        apply_lean_context(&mut query_options, lean);

        let (tx, rx) = mpsc::channel::<OrchestratorEvent>(256);
        let secrets = self
            .secrets
            .turn(thread_id.clone(), project_path.into(), tx.clone());
        let consumer_secrets = secrets.guard();
        let producer_secrets = secrets.guard();
        let turn = TurnContext {
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
        };
        let permissions = Arc::clone(&self.permissions);
        let steers = Arc::clone(&self.steers);

        tokio::spawn(async move {
            let _secrets = producer_secrets;
            let broker = match secrets.start().await {
                Ok(broker) => broker,
                Err(_) => {
                    turn.send_start_error(
                        "Could not start the private credential broker. Retry the turn.".into(),
                    )
                    .await;
                    return;
                }
            };
            if permission_mode == PermissionMode::Plan {
                if let ProviderQueryOptions::Claude(options) = &mut query_options {
                    crate::secrets::mcp::append_claude_instructions(options);
                }
            } else if broker.configure(&mut query_options).is_err() {
                turn.send_start_error(
                    "Could not configure private credential tools. Retry the turn.".into(),
                )
                .await;
                return;
            }
            match query_options {
                ProviderQueryOptions::Claude(options) => {
                    stream_claude(options, turn, permissions, steers).await;
                }
                ProviderQueryOptions::Codex(options) => {
                    stream_codex(options, turn, model, record_lean).await;
                }
            }
        });

        Ok(OrchestratorEventStream {
            rx,
            _secrets: consumer_secrets,
            _turn: consumer_guard,
        })
    }

    /// Whether `thread_id` has a turn in flight.
    pub fn thread_status(&self, thread_id: &ThreadId) -> ThreadStatus {
        lock_sessions(&self.sessions)
            .get(thread_id)
            .map_or(ThreadStatus::Idle, |s| s.status)
    }
}

/// What a producer task needs to forward one turn's events.
struct TurnContext {
    /// Declared first so it drops before `tx`: by the time the caller's
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

/// Put a Claude effort level on `options`. Codex and unknown levels are left alone,
/// so an unset effort does not change the CLI arguments.
fn apply_claude_effort(options: &mut ProviderQueryOptions, effort: Option<&str>) {
    let Some(level) = effort.and_then(bruhs_models::logic::run_place::claude_effort) else {
        return;
    };
    if let ProviderQueryOptions::Claude(options) = options {
        options.effort = Some(level.to_string());
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
                .model(model)
                .permission_prompts(!bypass)
                .accept_input(true);
            options.permission_mode = Some(mode);
            options.resume = session_id.map(String::from);
            // Appended, not `--system-prompt`: replacing Claude Code's prompt
            // saves ~3.8k tokens but drops its tool-use guidance. The snapshot
            // keeps the appended text cached across resumed turns.
            options.append_system_prompt = system_prompt.map(String::from);
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
            let mut options = codex_sdk_rs::QueryOptions::new(prompt)
                .cwd(project_path)
                .model(model)
                .access(codex_access(permission_mode))
                .skip_git_repo_check(true);
            // New threads are saved (not ephemeral) so the next turn can resume them.
            for key in crate::connections::PRIVATE_ENV {
                options = options.env_remove(key);
            }
            if let Some(thread_id) = session_id {
                options = options.resume(thread_id);
            }
            ProviderQueryOptions::Codex(options)
        }
    }
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

/// Variables an agent host sets for the Claude Code session it runs, which a
/// bruhs started from that session's terminal (T3 Code, Claude Code, Claude
/// desktop) would otherwise pass on. They describe the parent, not bruhs:
/// `CLAUDE_CODE_ENTRYPOINT=t3-code` alone turns on the host's Artifact tools
/// and skills, 32k -> 46k tokens on every turn (measured, Claude Code
/// 2.1.319); the messaging socket and token would point the child at the
/// parent session, and `CLAUDE_EFFORT` would set the child's effort.
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
) {
    let mut stream = match claude_sdk_rs::query(options) {
        Ok(stream) => stream,
        Err(e) => {
            turn.send_start_error(e.to_string()).await;
            return;
        }
    };
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
    let consumer_alive = loop {
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
        let event = map_claude_event(event);
        reported_error |= is_error_event(&event);
        turn.remember_session(&event);
        match &event {
            QueryEvent::PermissionRequest { id, input, .. } => {
                permissions.write().await.insert(
                    id.clone(),
                    PendingPermission {
                        owner: turn.owner.clone(),
                        input: input.clone(),
                        responder: responder.clone(),
                    },
                );
            }
            QueryEvent::PermissionCancelled { id } => {
                permissions.write().await.remove(id);
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
    turn.finish(consumer_alive);
}

/// `record_lean`: the lean-session list to add this turn's new session to.
async fn stream_codex(
    options: codex_sdk_rs::QueryOptions,
    turn: TurnContext,
    model: String,
    mut record_lean: Option<std::path::PathBuf>,
) {
    let mut stream = match codex_sdk_rs::query(options) {
        Ok(stream) => stream,
        Err(e) => {
            turn.send_start_error(e.to_string()).await;
            return;
        }
    };
    let mut reported_error = false;
    let consumer_alive = loop {
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
    turn.finish(consumer_alive);
}

impl TurnContext {
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

    /// Send `event` (and any background task it starts) to the caller.
    /// False when the caller has dropped the stream.
    async fn forward(&self, event: QueryEvent) -> bool {
        if let Some(task) =
            maybe_register_background_task(&self.registry, &self.owner.thread_id, &event)
        {
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

fn is_error_event(event: &QueryEvent) -> bool {
    matches!(
        event,
        QueryEvent::Error(_) | QueryEvent::Result { is_error: true, .. }
    )
}

fn maybe_register_background_task(
    registry: &BackgroundTaskRegistry,
    thread_id: &ThreadId,
    event: &QueryEvent,
) -> Option<BackgroundTask> {
    let QueryEvent::ToolCall { tool, input, .. } = event else {
        return None;
    };
    if tool != "Bash" {
        return None;
    }
    if !input
        .get("run_in_background")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return None;
    }

    let label = input
        .get("command")
        .and_then(|value| value.as_str())
        .unwrap_or("background task")
        .to_string();

    Some(registry.register(thread_id.as_str(), TaskKind::Shell, label, input.clone()))
}

/// An SDK event added after this mapping was written, passed on as raw data.
fn unhandled_sdk_event(event: &impl std::fmt::Debug) -> QueryEvent {
    QueryEvent::Raw(serde_json::json!({ "type": "unhandled", "event": format!("{event:?}") }))
}

fn map_claude_event(event: claude_sdk_rs::QueryEvent) -> QueryEvent {
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
            tool,
            input,
            tool_use_id,
        },
        claude_sdk_rs::QueryEvent::PermissionCancelled { id } => {
            QueryEvent::PermissionCancelled { id }
        }
        claude_sdk_rs::QueryEvent::Error(message) => QueryEvent::Error(message),
        claude_sdk_rs::QueryEvent::ProcessFailed(failure) => QueryEvent::Error(failure.to_string()),
        claude_sdk_rs::QueryEvent::Raw(value) => QueryEvent::Raw(value),
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
    use tokio_stream::StreamExt;

    fn options(
        provider: AgentProvider,
        mode: PermissionMode,
        session_id: Option<&str>,
    ) -> ProviderQueryOptions {
        let model = match provider {
            AgentProvider::Claude => "claude-sonnet-4-6",
            AgentProvider::Codex => "gpt-5.5",
        };
        build_query_options(provider, "hi", "/tmp", model, mode, None, session_id)
    }

    fn claude(options: ProviderQueryOptions) -> claude_sdk_rs::QueryOptions {
        match options {
            ProviderQueryOptions::Claude(o) => o,
            ProviderQueryOptions::Codex(_) => panic!("expected claude options"),
        }
    }

    fn codex(options: ProviderQueryOptions) -> codex_sdk_rs::QueryOptions {
        match options {
            ProviderQueryOptions::Codex(o) => o,
            ProviderQueryOptions::Claude(_) => panic!("expected codex options"),
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
        assert_eq!(o.append_system_prompt.as_deref(), Some("meeting notes"));
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
        assert_eq!(
            new.prompt,
            "<thread_context>\nmeeting notes\n</thread_context>\n\nhi"
        );
        assert!(new.config.is_empty(), "no developer_instructions override");
        assert_eq!(build(Some("thr-1")).prompt, "hi", "already in the session");
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
        let QueryEvent::Usage(usage) = map_claude_event(claude_sdk_rs::QueryEvent::Usage(request))
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
        assert_eq!(overrides(&codex_turn), ["features.plugins=false"]);
        lean_codex(&mut codex_turn);
        assert_eq!(overrides(&codex_turn), ["features.plugins=false"], "once");

        let mut full_codex = options(AgentProvider::Codex, PermissionMode::Default, None);
        apply_lean_context(&mut full_codex, false);
        assert!(codex(full_codex).config.is_empty());
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

    /// A bruhs launched from a T3 Code or Claude Code terminal must not run
    /// Claude as that host's session (T3's entrypoint alone costs +14k per
    /// turn).
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
    fn claude_effort_reaches_only_the_claude_cli() {
        let mut claude_opts = options(AgentProvider::Claude, PermissionMode::Default, None);
        apply_claude_effort(&mut claude_opts, Some("high"));
        assert_eq!(claude(claude_opts).effort.as_deref(), Some("high"));

        let mut unset = options(AgentProvider::Claude, PermissionMode::Default, None);
        apply_claude_effort(&mut unset, None);
        assert_eq!(claude(unset).effort, None);

        let mut unknown = options(AgentProvider::Claude, PermissionMode::Default, None);
        apply_claude_effort(&mut unknown, Some("minimal"));
        assert_eq!(claude(unknown).effort, None);

        let mut codex_opts = options(AgentProvider::Codex, PermissionMode::Default, None);
        apply_claude_effort(&mut codex_opts, Some("max"));
        assert!(matches!(codex_opts, ProviderQueryOptions::Codex(_)));

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
    fn turn_context(
        sessions: &Sessions,
        registry: &Arc<BackgroundTaskRegistry>,
        thread: &ThreadId,
        turn: u64,
    ) -> (TurnContext, mpsc::Receiver<OrchestratorEvent>) {
        let (tx, rx) = mpsc::channel(16);
        let context = TurnContext {
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
}
