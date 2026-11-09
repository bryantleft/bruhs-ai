use crate::error::{Error, Result};
use crate::process::ProcessFailure;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Sandbox for model-generated shell commands (`codex exec --sandbox`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SandboxMode {
    /// Commands may read but not write (`read-only`).
    ReadOnly,
    /// Commands may write inside the workspace (`workspace-write`).
    WorkspaceWrite,
    /// No sandbox restrictions (`danger-full-access`).
    DangerFullAccess,
}

impl SandboxMode {
    /// The CLI's name for this mode.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
            Self::DangerFullAccess => "danger-full-access",
        }
    }
}

/// When Codex asks before running a command (`codex --ask-for-approval`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ApprovalPolicy {
    /// The model decides when to ask (`on-request`).
    OnRequest,
    /// Never ask; failures go straight back to the model (`never`).
    Never,
}

impl ApprovalPolicy {
    /// The CLI's name for this policy.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OnRequest => "on-request",
            Self::Never => "never",
        }
    }
}

/// How much Codex may do on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CodexAccess {
    /// Run commands inside `sandbox`, asking per `approval`.
    Sandboxed {
        /// The command sandbox.
        sandbox: SandboxMode,
        /// When to ask before running a command.
        approval: ApprovalPolicy,
    },
    /// No sandbox and no approvals
    /// (`--dangerously-bypass-approvals-and-sandbox`). Only for environments
    /// that are sandboxed externally.
    DangerouslyUnsandboxed,
}

impl Default for CodexAccess {
    /// Writes inside the workspace, never asks.
    fn default() -> Self {
        Self::Sandboxed {
            sandbox: SandboxMode::WorkspaceWrite,
            approval: ApprovalPolicy::Never,
        }
    }
}

/// Which Codex thread a query runs in, and whether it is saved.
///
/// An ephemeral thread is never written to disk, so it cannot be resumed;
/// that is why "ephemeral" and "resume" are separate variants.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Session {
    /// Start a new thread and save it so a later query can resume it.
    #[default]
    New,
    /// Start a new thread without saving it (`--ephemeral`).
    Ephemeral,
    /// Continue a saved thread (`codex exec resume <id>`). Ids starting with
    /// `-` are rejected with [`Error::InvalidSessionId`].
    Resume(String),
}

/// One `--config key=value` override. Codex parses the value as TOML and
/// falls back to the raw string.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConfigOverride {
    key: String,
    value: String,
}

impl ConfigOverride {
    /// An override of the dotted config path `key` (e.g. `model_reasoning_effort`).
    ///
    /// # Errors
    ///
    /// [`Error::InvalidConfigKey`] when `key` is empty or contains `=`: Codex
    /// splits `key=value` at the first `=`, so such a key would set a
    /// different option.
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Result<Self> {
        let key = key.into();
        if key.is_empty() || key.contains('=') {
            return Err(Error::InvalidConfigKey(key));
        }
        Ok(Self {
            key,
            value: value.into(),
        })
    }

    /// An override whose value is the literal text `text`, quoted as a TOML
    /// basic string so Codex never reads it as another type or a table.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidConfigKey`], as for [`ConfigOverride::new`].
    pub fn string(key: impl Into<String>, text: &str) -> Result<Self> {
        Self::new(key, toml_basic_string(text))
    }

    /// The dotted config path.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The value, as Codex will parse it.
    pub fn value(&self) -> &str {
        &self.value
    }
}

/// `text` as a TOML basic string: `"`, `\` and every control character
/// (TOML counts DEL as one) are escaped; everything else is kept verbatim.
fn toml_basic_string(text: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => {
                // Writing into a `String` cannot fail.
                let _ = write!(out, "\\u{:04X}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Options for one [`query`](crate::query()) to the Codex CLI.
///
/// Build one with [`QueryOptions::new`] and the chainable setters; every field
/// is public for reading and adjusting afterwards.
///
/// ```
/// use codex_sdk_rs::{ApprovalPolicy, CodexAccess, QueryOptions, SandboxMode, Session};
///
/// let options = QueryOptions::new("List the files")
///     .cwd("/tmp/project")
///     .access(CodexAccess::Sandboxed {
///         sandbox: SandboxMode::ReadOnly,
///         approval: ApprovalPolicy::Never,
///     })
///     .resume("thread-id");
/// assert_eq!(options.session, Session::Resume("thread-id".into()));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct QueryOptions {
    /// The user prompt, written to the CLI's stdin.
    pub prompt: String,
    /// Workspace root (`--cd`). Defaults to `.`.
    pub cwd: PathBuf,
    /// Model (`--model`); `None` uses the configured default.
    pub model: Option<String>,
    /// Config profile (`--profile`).
    pub profile: Option<String>,
    /// Sandbox and approval policy. Defaults to [`CodexAccess::default`].
    pub access: CodexAccess,
    /// Thread to run in. Defaults to a new, saved thread.
    pub session: Session,
    /// Allow running outside a Git repository (`--skip-git-repo-check`).
    pub skip_git_repo_check: bool,
    /// Do not load `$CODEX_HOME/config.toml` (`--ignore-user-config`).
    pub ignore_user_config: bool,
    /// Do not load execpolicy `.rules` files (`--ignore-rules`).
    pub ignore_rules: bool,
    /// Give the model live web search (`--search`).
    pub enable_web_search: bool,
    /// Config overrides (`--config key=value`), applied in order.
    pub config: Vec<ConfigOverride>,
    /// Start from this process's environment. Defaults to `true`; with
    /// `false` the CLI sees only [`env`](Self::env), so pass `PATH` and
    /// `HOME` there.
    pub inherit_env: bool,
    /// Variables removed from the inherited environment.
    pub env_remove: Vec<String>,
    /// Variables set for the CLI, applied last (they win over removals).
    pub env: Vec<(String, String)>,
}

impl Default for QueryOptions {
    fn default() -> Self {
        Self {
            prompt: String::new(),
            cwd: PathBuf::from("."),
            model: None,
            profile: None,
            access: CodexAccess::default(),
            session: Session::New,
            skip_git_repo_check: false,
            ignore_user_config: false,
            ignore_rules: false,
            enable_web_search: false,
            config: Vec::new(),
            inherit_env: true,
            env_remove: Vec::new(),
            env: Vec::new(),
        }
    }
}

impl QueryOptions {
    /// Options for `prompt` with every other setting at its default.
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            ..Self::default()
        }
    }

    /// Set [`cwd`](Self::cwd).
    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = cwd.into();
        self
    }

    /// Set [`model`](Self::model).
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set [`profile`](Self::profile).
    pub fn profile(mut self, profile: impl Into<String>) -> Self {
        self.profile = Some(profile.into());
        self
    }

    /// Set [`access`](Self::access).
    pub fn access(mut self, access: CodexAccess) -> Self {
        self.access = access;
        self
    }

    /// Set [`session`](Self::session).
    pub fn session(mut self, session: Session) -> Self {
        self.session = session;
        self
    }

    /// Resume the saved thread `thread_id`; shorthand for
    /// `session(Session::Resume(..))`.
    pub fn resume(self, thread_id: impl Into<String>) -> Self {
        self.session(Session::Resume(thread_id.into()))
    }

    /// Set [`skip_git_repo_check`](Self::skip_git_repo_check).
    pub fn skip_git_repo_check(mut self, skip: bool) -> Self {
        self.skip_git_repo_check = skip;
        self
    }

    /// Set [`ignore_user_config`](Self::ignore_user_config).
    pub fn ignore_user_config(mut self, ignore: bool) -> Self {
        self.ignore_user_config = ignore;
        self
    }

    /// Set [`ignore_rules`](Self::ignore_rules).
    pub fn ignore_rules(mut self, ignore: bool) -> Self {
        self.ignore_rules = ignore;
        self
    }

    /// Set [`enable_web_search`](Self::enable_web_search).
    pub fn enable_web_search(mut self, enable: bool) -> Self {
        self.enable_web_search = enable;
        self
    }

    /// Add `text` to Codex's developer instructions for this thread
    /// (`--config developer_instructions="…"`), on top of its built-in
    /// prompt. Pass the same text on every turn of a thread: Codex re-sends
    /// its whole instruction block when the value changes between turns.
    pub fn developer_instructions(self, text: &str) -> Self {
        // A literal key with no `=`, so the `new` check cannot fail.
        self.config(ConfigOverride {
            key: "developer_instructions".to_string(),
            value: toml_basic_string(text),
        })
    }

    /// Append a [`config`](Self::config) override.
    pub fn config(mut self, entry: ConfigOverride) -> Self {
        self.config.push(entry);
        self
    }

    /// Set [`inherit_env`](Self::inherit_env).
    pub fn inherit_env(mut self, inherit: bool) -> Self {
        self.inherit_env = inherit;
        self
    }

    /// Add a variable to [`env_remove`](Self::env_remove).
    pub fn env_remove(mut self, key: impl Into<String>) -> Self {
        self.env_remove.push(key.into());
        self
    }

    /// Add a variable to [`env`](Self::env).
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }
}

/// One event from `codex exec --json`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum QueryEvent {
    /// A thread was started or resumed.
    Init {
        /// Thread id; pass it to [`QueryOptions::resume`] to continue.
        thread_id: String,
    },
    /// A completed assistant message (Codex does not stream deltas).
    Text(String),
    /// A shell command started.
    ToolCall {
        /// Item id, matched by [`QueryEvent::ToolResult::tool_use_id`].
        id: String,
        /// Always `Bash` for shell commands.
        tool: String,
        /// `{"command": "..."}`.
        input: serde_json::Value,
    },
    /// A shell command finished.
    ToolResult {
        /// The [`QueryEvent::ToolCall::id`] this answers.
        tool_use_id: String,
        /// Combined command output.
        content: String,
        /// Whether the command failed (non-zero exit or failed status).
        is_error: bool,
    },
    /// Token usage of the completed turn.
    Usage(UsageInfo),
    /// The turn failed (`turn.failed`), or a stdout line was not valid JSON.
    Error(String),
    /// A top-level `error` event. Codex uses these both for transient notices
    /// (such as `Reconnecting...`) and for fatal errors; a fatal one is
    /// followed by `turn.failed` or a failed exit, so this alone does not end
    /// the turn.
    StreamError(String),
    /// The CLI process failed: it exited unsuccessfully, its output could not
    /// be read, or it could not be sent the prompt. Always the last event.
    ProcessFailed(ProcessFailure),
    /// A well-formed line this SDK does not interpret.
    Raw(serde_json::Value),
}

/// Token usage of one turn, sent once the CLI has exited.
///
/// Codex's `turn.completed` reports the whole thread's running total; this
/// SDK subtracts the total recorded before the turn (from the thread's
/// rollout file) so the numbers are this turn's own. When the rollout cannot
/// be read the CLI's numbers are passed on unchanged and
/// [`context_input_tokens`](Self::context_input_tokens) is `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct UsageInfo {
    /// Input tokens, including the cached ones.
    #[serde(default)]
    pub input_tokens: u64,
    /// Input tokens served from the prompt cache (part of `input_tokens`).
    #[serde(default)]
    pub cached_input_tokens: u64,
    /// Output tokens.
    #[serde(default)]
    pub output_tokens: u64,
    /// Reasoning output tokens.
    #[serde(default)]
    pub reasoning_output_tokens: u64,
    /// Input of the turn's last model request: how full the context window
    /// is now. `None` when unknown.
    #[serde(default)]
    pub context_input_tokens: Option<u64>,
}
