use crate::process::ProcessFailure;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// How the CLI decides whether a tool may run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PermissionMode {
    /// Ask before anything no rule allows (`--permission-mode=default`).
    Default,
    /// Approve file edits without asking (`--permission-mode=acceptEdits`).
    AcceptEdits,
    /// Plan only; nothing is changed (`--permission-mode=plan`).
    Plan,
    /// Never ask (`--dangerously-skip-permissions`). Only for sandboxes.
    BypassPermissions,
}

/// Which tools the CLI offers and pre-approves.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Tools {
    /// The CLI's own tool set and approval rules; no flags are passed.
    #[default]
    CliDefault,
    /// The CLI's tool set, with these tools or rules (`Read`,
    /// `Bash(git log:*)`) approved up front via `--allowedTools`. An empty
    /// list approves nothing extra, the same as [`Tools::CliDefault`].
    Allow(Vec<String>),
    /// No tools at all, for transcript-only generation: an empty tool
    /// inventory, `--restricted`, and no inherited MCP servers. The
    /// permission mode is not passed in this case.
    None,
}

/// Options for one [`query`](crate::query()) to the Claude CLI.
///
/// Build one with [`QueryOptions::new`] and the chainable setters; every field
/// is public for reading and adjusting afterwards.
///
/// ```
/// use claude_sdk_rs::{PermissionMode, QueryOptions, Tools};
///
/// let options = QueryOptions::new("Summarize the README")
///     .cwd("/tmp/project")
///     .model("sonnet")
///     .permission_mode(PermissionMode::Plan)
///     .tools(Tools::Allow(vec!["Read".into()]))
///     .max_turns(3);
/// assert_eq!(options.max_turns, Some(3));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct QueryOptions {
    /// The user prompt. Always passed as text, never parsed as a flag.
    pub prompt: String,
    /// Working directory of the CLI process (the CLI has no `--cwd` flag).
    /// Defaults to `.`.
    pub cwd: PathBuf,
    /// Tool inventory and pre-approvals. Defaults to [`Tools::CliDefault`].
    pub tools: Tools,
    /// Permission mode; `None` leaves the CLI's configured default.
    pub permission_mode: Option<PermissionMode>,
    /// Model alias or full name (`--model`); `None` uses the CLI default.
    pub model: Option<String>,
    /// Effort for this session (`--effort`). `None` leaves the CLI default.
    pub effort: Option<String>,
    /// Session id to resume (`--resume`). Ids starting with `-` are rejected
    /// with [`Error::InvalidSessionId`](crate::Error::InvalidSessionId).
    pub resume: Option<String>,
    /// Stop after this many agentic turns (`--max-turns`); the result then
    /// has subtype `error_max_turns`.
    pub max_turns: Option<u32>,
    /// Replace the default system prompt (`--system-prompt`).
    pub system_prompt: Option<String>,
    /// Append to the default system prompt (`--append-system-prompt`).
    pub append_system_prompt: Option<String>,
    /// Additional MCP server configuration as JSON (`--mcp-config`). Ignored with [`Tools::None`].
    pub mcp_config: Option<String>,
    /// Stream text and thinking deltas as they arrive
    /// (`--include-partial-messages`). Defaults to `true`.
    pub include_partial_messages: bool,
    /// Route tool-permission prompts to this process over stdio
    /// (`--permission-prompt-tool=stdio` plus streaming input). The prompt
    /// is then sent on stdin and every [`QueryEvent::PermissionRequest`]
    /// must be answered through
    /// [`QueryStream::responder`](crate::QueryStream::responder). Defaults to
    /// `false`.
    pub permission_prompts: bool,
    /// Keep stdin open for more user messages (`--input-format=stream-json`)
    /// even when permission prompts are off. The first prompt is still the
    /// first stdin line. Defaults to `false`.
    pub accept_input: bool,
    /// Claude `--autocompact`. `None` omits the flag so the CLI default stands.
    pub autocompact: Option<String>,
    /// Settings files to load (`--setting-sources`): any of `user`,
    /// `project`, `local`. `None` omits the flag (all of them); an empty list
    /// loads none, which also skips the plugins those files enable.
    pub setting_sources: Option<Vec<String>>,
    /// Extra settings as a JSON object (`--settings`), loaded even when
    /// [`setting_sources`](Self::setting_sources) is empty.
    pub settings: Option<String>,
    /// Start from this process's environment. Defaults to `true`; with
    /// `false` the CLI sees only [`env`](Self::env), so pass `PATH` and
    /// `HOME` there.
    pub inherit_env: bool,
    /// Variables removed from the inherited environment, e.g.
    /// `ANTHROPIC_API_KEY` to keep the CLI on its OAuth login.
    pub env_remove: Vec<String>,
    /// Variables set for the CLI, applied last (they win over removals).
    pub env: Vec<(String, String)>,
}

impl Default for QueryOptions {
    fn default() -> Self {
        Self {
            prompt: String::new(),
            cwd: PathBuf::from("."),
            tools: Tools::CliDefault,
            permission_mode: None,
            model: None,
            effort: None,
            resume: None,
            max_turns: None,
            system_prompt: None,
            append_system_prompt: None,
            mcp_config: None,
            include_partial_messages: true,
            permission_prompts: false,
            accept_input: false,
            autocompact: None,
            setting_sources: None,
            settings: None,
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

    /// Set [`tools`](Self::tools).
    pub fn tools(mut self, tools: Tools) -> Self {
        self.tools = tools;
        self
    }

    /// Set [`permission_mode`](Self::permission_mode).
    pub fn permission_mode(mut self, mode: PermissionMode) -> Self {
        self.permission_mode = Some(mode);
        self
    }

    /// Set [`model`](Self::model).
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set [`effort`](Self::effort).
    pub fn effort(mut self, effort: impl Into<String>) -> Self {
        self.effort = Some(effort.into());
        self
    }

    /// Set [`resume`](Self::resume).
    pub fn resume(mut self, session_id: impl Into<String>) -> Self {
        self.resume = Some(session_id.into());
        self
    }

    /// Set [`max_turns`](Self::max_turns).
    pub fn max_turns(mut self, max_turns: u32) -> Self {
        self.max_turns = Some(max_turns);
        self
    }

    /// Set [`system_prompt`](Self::system_prompt).
    pub fn system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Set [`append_system_prompt`](Self::append_system_prompt).
    pub fn append_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.append_system_prompt = Some(prompt.into());
        self
    }

    /// Set [`include_partial_messages`](Self::include_partial_messages).
    pub fn include_partial_messages(mut self, include: bool) -> Self {
        self.include_partial_messages = include;
        self
    }

    /// Set [`permission_prompts`](Self::permission_prompts).
    pub fn permission_prompts(mut self, enabled: bool) -> Self {
        self.permission_prompts = enabled;
        self
    }

    /// Set [`accept_input`](Self::accept_input).
    pub fn accept_input(mut self, enabled: bool) -> Self {
        self.accept_input = enabled;
        self
    }

    /// Set [`autocompact`](Self::autocompact).
    pub fn autocompact(mut self, tokens: impl Into<String>) -> Self {
        self.autocompact = Some(tokens.into());
        self
    }

    /// Set [`setting_sources`](Self::setting_sources).
    pub fn setting_sources<S: Into<String>>(
        mut self,
        sources: impl IntoIterator<Item = S>,
    ) -> Self {
        self.setting_sources = Some(sources.into_iter().map(Into::into).collect());
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

/// One event from the Claude CLI's `stream-json` output.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum QueryEvent {
    /// The session started.
    Init {
        /// Session id; pass it to [`QueryOptions::resume`] to continue.
        session_id: String,
        /// The model in use, when reported.
        model: Option<String>,
        /// Names of the tools available to the model.
        tools: Vec<String>,
        /// MCP servers and their connection status.
        mcp_servers: Vec<McpServerInfo>,
    },
    /// A streamed text delta (with `include_partial_messages`).
    Text(String),
    /// A streamed thinking delta (with `include_partial_messages`).
    Thinking(String),
    /// The model called a tool. One event per `tool_use` block.
    ToolCall {
        /// Tool-use id, matched by [`QueryEvent::ToolResult::tool_use_id`].
        id: String,
        /// Tool name, e.g. `Bash`.
        tool: String,
        /// The tool's input as the model wrote it.
        input: serde_json::Value,
    },
    /// A tool finished. One event per `tool_result` block.
    ToolResult {
        /// The [`QueryEvent::ToolCall::id`] this answers.
        tool_use_id: String,
        /// The result text (multi-part results are joined with newlines).
        content: String,
        /// Whether the tool reported failure.
        is_error: bool,
    },
    /// Token usage reported mid-stream.
    Usage(UsageInfo),
    /// The turn ended. Emitted for every result subtype (`success`,
    /// `error_max_turns`, `error_during_execution`, ...).
    Result {
        /// The final text; empty for most error results (see `errors`).
        result: String,
        /// Session id; pass it to [`QueryOptions::resume`] to continue.
        session_id: String,
        /// Wall-clock duration of the turn.
        duration_ms: u64,
        /// Total token usage of the turn.
        usage: UsageInfo,
        /// Whether the turn failed.
        is_error: bool,
        /// Why the turn failed, for error subtypes.
        errors: Vec<String>,
    },
    /// The CLI is asking whether a tool may run (only with
    /// [`QueryOptions::permission_prompts`]). Answer it through the stream's
    /// [`PermissionResponder`](crate::PermissionResponder) or the CLI waits
    /// forever.
    PermissionRequest {
        /// Request id to answer with.
        id: String,
        /// Tool name, e.g. `Bash`.
        tool: String,
        /// The input the model proposed.
        input: serde_json::Value,
        /// The tool-use id the request belongs to, when reported.
        tool_use_id: Option<String>,
        /// `permission_suggestions` from the CLI (rule updates it would accept).
        suggestions: serde_json::Value,
    },
    /// The CLI withdrew a pending permission request.
    PermissionCancelled {
        /// The withdrawn request's id.
        id: String,
    },
    /// A stdout line that was not valid JSON.
    Error(String),
    /// The CLI process failed: it exited unsuccessfully, its output could not
    /// be read, or it could not be sent the prompt. Always the last event.
    ProcessFailed(ProcessFailure),
    /// A well-formed line this SDK does not interpret.
    Raw(serde_json::Value),
}

/// Answer to a [`QueryEvent::PermissionRequest`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum PermissionDecision {
    /// Let the tool run.
    Allow {
        /// Input to run with; `None` keeps what the model proposed.
        updated_input: Option<serde_json::Value>,
    },
    /// Refuse; the model sees `message`.
    Deny {
        /// Why the tool may not run.
        message: String,
    },
}

/// Token usage, as reported by the CLI.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct UsageInfo {
    /// Uncached input tokens.
    #[serde(default)]
    pub input_tokens: u64,
    /// Output tokens.
    #[serde(default)]
    pub output_tokens: u64,
    /// Input tokens read from the prompt cache.
    #[serde(default)]
    pub cache_read_input_tokens: u64,
    /// Input tokens written to the prompt cache.
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
}

/// An MCP server listed in [`QueryEvent::Init`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct McpServerInfo {
    /// Server name.
    pub name: String,
    /// Connection status, e.g. `connected` or `failed`.
    pub status: String,
}
