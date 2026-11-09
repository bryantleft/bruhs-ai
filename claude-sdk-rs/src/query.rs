use crate::error::{Error, Result};
use crate::process::{apply_env, CliProcess, Ending};
use crate::types::{
    McpServerInfo, PermissionDecision, PermissionMode, QueryEvent, QueryOptions, Tools, UsageInfo,
};
use futures_core::Stream;
use serde_json::json;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::AsyncWriteExt;
use tokio::process::{ChildStdin, Command};
use tokio::sync::{mpsc, Mutex};

type StdinSlot = Arc<Mutex<Option<ChildStdin>>>;

/// Spawn the `claude` CLI and return a stream of its events.
///
/// With [`QueryOptions::permission_prompts`] or [`QueryOptions::accept_input`]
/// the CLI runs in streaming-input mode: the prompt goes in over stdin.
/// Permission prompts come back as [`QueryEvent::PermissionRequest`] only
/// when `permission_prompts` is set, and must be answered through the
/// stream's [`PermissionResponder`] or the CLI waits forever. A later user
/// message uses [`PermissionResponder::push_user_message`].
///
/// Dropping the stream kills and reaps the CLI. If the CLI fails, the last
/// event is [`QueryEvent::ProcessFailed`].
///
/// # Errors
///
/// [`Error::InvalidSessionId`] for a resume id starting with `-`,
/// [`Error::CliNotFound`] when `claude` is not on `PATH` (or the working
/// directory does not exist), and [`Error::Spawn`] for other spawn failures.
///
/// # Panics
///
/// Panics when called outside a Tokio runtime.
pub fn query(options: QueryOptions) -> Result<QueryStream> {
    if let Some(session_id) = options.resume.as_ref().filter(|id| id.starts_with('-')) {
        return Err(Error::InvalidSessionId(session_id.clone()));
    }
    let streaming = options.permission_prompts || options.accept_input;
    let mut cmd = command(&options);

    // Never inherit our stdin: in text mode the CLI would read it as more input.
    cmd.stdin(if streaming {
        std::process::Stdio::piped()
    } else {
        std::process::Stdio::null()
    });

    let mut process = CliProcess::spawn("claude", &mut cmd).map_err(Error::spawn)?;
    let stdin_slot: StdinSlot = Arc::new(Mutex::new(if streaming {
        process.take_stdin()
    } else {
        None
    }));
    let responder = PermissionResponder {
        stdin: Arc::clone(&stdin_slot),
        interactive: options.permission_prompts,
    };
    let first_message = streaming.then(|| user_message_line(&options.prompt));

    let (tx, rx) = mpsc::channel(256);
    tokio::spawn(async move {
        let ending = pump(&mut process, &tx, &stdin_slot, first_message).await;
        // The turn is over either way; a CLI in streaming-input mode only
        // exits once its stdin closes.
        close_stdin(&stdin_slot).await;
        if let Some(failure) = process.finish(ending).await {
            // If the consumer is already gone nobody needs the failure.
            let _ = tx.send(QueryEvent::ProcessFailed(failure)).await;
        }
    });

    Ok(QueryStream { rx, responder })
}

/// Send the first message (streaming-input mode), then forward parsed stdout
/// lines until the CLI or the consumer is done.
async fn pump(
    process: &mut CliProcess,
    tx: &mpsc::Sender<QueryEvent>,
    stdin_slot: &StdinSlot,
    first_message: Option<String>,
) -> Ending {
    if let Some(line) = first_message {
        let written = tokio::select! {
            () = tx.closed() => return Ending::ConsumerGone,
            written = write_line(stdin_slot, &line) => written,
        };
        if let Err(e) = written {
            return Ending::Failed(format!("sending the prompt to claude failed: {e}"));
        }
    }
    loop {
        let line = match process.next_line(tx).await {
            Ok(line) => line,
            Err(ending) => return ending,
        };
        if line.trim().is_empty() {
            continue;
        }
        for event in parse_cli_line(&line) {
            let finished = matches!(event, QueryEvent::Result { .. });
            if tx.send(event).await.is_err() {
                return Ending::ConsumerGone;
            }
            if finished {
                // In streaming-input mode the CLI waits for more messages
                // until stdin closes; the turn is over (whatever the result
                // subtype), so let it exit.
                close_stdin(stdin_slot).await;
            }
        }
    }
}

/// The `claude` invocation for `options`, minus stdio setup.
///
/// Value-taking options use the single-argument `--flag=value` form: the
/// CLI's variadic options (`--tools`, `--mcp-config`, `--allowedTools`)
/// otherwise swallow the arguments after them, and a value starting with `-`
/// could be read as another flag. The prompt comes last, after `--`.
fn command(options: &QueryOptions) -> Command {
    let mut cmd = Command::new("claude");
    let streaming = options.permission_prompts || options.accept_input;

    cmd.arg("-p") // print mode
        .arg("--output-format=stream-json")
        .arg("--verbose")
        // The Claude CLI uses the spawned process's working directory; there is no `--cwd` flag.
        .current_dir(&options.cwd);

    if streaming {
        cmd.arg("--input-format=stream-json");
    }
    if options.permission_prompts {
        cmd.arg("--permission-prompt-tool=stdio");
    }

    if options.include_partial_messages {
        cmd.arg("--include-partial-messages");
    }

    match &options.tools {
        Tools::CliDefault => {}
        Tools::Allow(tools) if tools.is_empty() => {}
        Tools::Allow(tools) => {
            // The CLI accepts a comma- or space-separated list; commas keep
            // rules such as `Bash(git log:*)` intact.
            cmd.arg(format!("--allowedTools={}", tools.join(",")));
        }
        Tools::None => {
            // An empty approval allowlist is not an empty tool inventory. Also
            // exclude inherited MCP/config hooks from transcript-only generation.
            cmd.args([
                "--tools=",
                "--restricted",
                "--strict-mcp-config",
                "--mcp-config={\"mcpServers\":{}}",
            ]);
        }
    }

    // `--restricted` refuses bypass, and a tool-free run has nothing to permit.
    if let Some(mode) = options
        .permission_mode
        .filter(|_| options.tools != Tools::None)
    {
        cmd.arg(permission_mode_arg(mode));
    }

    if let Some(ref model) = options.model {
        cmd.arg(format!("--model={model}"));
    }

    if let Some(ref effort) = options.effort {
        cmd.arg(format!("--effort={effort}"));
    }

    if let Some(ref autocompact) = options.autocompact {
        cmd.arg(format!("--autocompact={autocompact}"));
    }

    if let Some(ref sources) = options.setting_sources {
        cmd.arg(format!("--setting-sources={}", sources.join(",")));
    }

    if let Some(ref settings) = options.settings {
        cmd.arg(format!("--settings={settings}"));
    }

    if let Some(ref session_id) = options.resume {
        cmd.arg(format!("--resume={session_id}"));
    }

    if let Some(max_turns) = options.max_turns {
        cmd.arg(format!("--max-turns={max_turns}"));
    }

    if let Some(ref sp) = options.system_prompt {
        cmd.arg(format!("--system-prompt={sp}"));
    }

    if let Some(ref asp) = options.append_system_prompt {
        cmd.arg(format!("--append-system-prompt={asp}"));
    }

    if let Some(config) = options
        .mcp_config
        .as_ref()
        .filter(|_| options.tools != Tools::None)
    {
        cmd.arg(format!("--mcp-config={config}"));
    }

    apply_env(
        &mut cmd,
        options.inherit_env,
        &options.env_remove,
        &options.env,
    );

    // The prompt: an operand in text mode, the first stdin message otherwise.
    // `--` ends option parsing, so a prompt such as `--help` or `-` is text.
    if !streaming {
        cmd.arg("--").arg(&options.prompt);
    }
    cmd
}

/// The one place a [`PermissionMode`] becomes a CLI argument.
fn permission_mode_arg(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Default => "--permission-mode=default",
        PermissionMode::AcceptEdits => "--permission-mode=acceptEdits",
        PermissionMode::Plan => "--permission-mode=plan",
        PermissionMode::BypassPermissions => "--dangerously-skip-permissions",
    }
}

async fn write_line(slot: &StdinSlot, line: &str) -> Result<()> {
    let mut guard = slot.lock().await;
    let Some(stdin) = guard.as_mut() else {
        return Err(Error::StdinClosed);
    };
    stdin
        .write_all(line.as_bytes())
        .await
        .map_err(Error::Stdin)?;
    let flushed = stdin.flush().await.map_err(Error::Stdin);
    // Held across write and flush so concurrent lines never interleave.
    drop(guard);
    flushed
}

async fn close_stdin(slot: &StdinSlot) {
    let taken = slot.lock().await.take();
    drop(taken);
}

/// One stream-json user message carrying the prompt.
fn user_message_line(prompt: &str) -> String {
    let mut line = json!({
        "type": "user",
        "message": { "role": "user", "content": prompt },
    })
    .to_string();
    line.push('\n');
    line
}

/// The `control_response` the CLI expects for a `can_use_tool` request.
fn control_response_json(request_id: &str, decision: &PermissionDecision) -> serde_json::Value {
    let response = match decision {
        PermissionDecision::Allow { updated_input } => {
            let mut v = json!({ "behavior": "allow" });
            if let Some(input) = updated_input {
                v["updatedInput"] = input.clone();
            }
            v
        }
        PermissionDecision::Deny { message } => json!({
            "behavior": "deny",
            "message": message,
        }),
    };
    json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": request_id,
            "response": response,
        },
    })
}

/// Answers [`QueryEvent::PermissionRequest`]s by writing to the CLI's stdin.
/// Cheap to clone; every clone talks to the same process.
#[derive(Debug, Clone)]
pub struct PermissionResponder {
    stdin: StdinSlot,
    interactive: bool,
}

impl PermissionResponder {
    /// False when the query was started without `permission_prompts`; such
    /// queries never emit permission requests.
    pub fn is_interactive(&self) -> bool {
        self.interactive
    }

    /// Answer the permission request `request_id`.
    ///
    /// # Errors
    ///
    /// [`Error::StdinClosed`] once the turn is over (or for a query without
    /// `permission_prompts`), and [`Error::Stdin`] if the write fails.
    pub async fn respond(&self, request_id: &str, decision: PermissionDecision) -> Result<()> {
        let mut line = control_response_json(request_id, &decision).to_string();
        line.push('\n');
        write_line(&self.stdin, &line).await
    }

    /// Write another user message into the running turn.
    ///
    /// # Errors
    ///
    /// [`Error::StdinClosed`] once the turn is over, and [`Error::Stdin`] if
    /// the write fails.
    pub async fn push_user_message(&self, prompt: &str) -> Result<()> {
        write_line(&self.stdin, &user_message_line(prompt)).await
    }
}

/// Parse one stdout line into the events it carries. Most lines map to one
/// event; an `assistant`/`user` message yields one event per `tool_use` /
/// `tool_result` block.
fn parse_cli_line(line: &str) -> Vec<QueryEvent> {
    let json: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return vec![QueryEvent::Error(format!("JSON parse error: {e}"))],
    };

    let event_type = json.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let subtype = json.get("subtype").and_then(|t| t.as_str()).unwrap_or("");

    let event = match (event_type, subtype) {
        ("system", "init") => parse_init(&json),
        ("stream_event", _) => parse_stream_event(json),
        ("assistant", _) => return content_block_events(json, "tool_use", tool_call),
        ("user", _) => {
            let events = content_block_events(json, "tool_result", tool_result);
            // A steered user line has no tool result. The client already
            // showed that message, so the echo is not a second copy.
            if events
                .iter()
                .all(|event| matches!(event, QueryEvent::Raw(_)))
            {
                return Vec::new();
            }
            return events;
        }
        ("control_request", _) => parse_control_request(json),
        ("control_cancel_request", _) => QueryEvent::PermissionCancelled {
            id: str_field(&json, "request_id"),
        },
        // Every result subtype ends the turn: `success`, `error_max_turns`,
        // `error_during_execution`, and whatever later CLIs add.
        ("result", _) => parse_result(&json, subtype),
        _ => QueryEvent::Raw(json),
    };
    vec![event]
}

fn str_field(json: &serde_json::Value, key: &str) -> String {
    json.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn parse_init(json: &serde_json::Value) -> QueryEvent {
    let tools = json
        .get("tools")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let mcp_servers = json
        .get("mcp_servers")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| serde_json::from_value::<McpServerInfo>(v.clone()).ok())
                .collect()
        })
        .unwrap_or_default();
    QueryEvent::Init {
        session_id: str_field(json, "session_id"),
        model: json.get("model").and_then(|v| v.as_str()).map(String::from),
        tools,
        mcp_servers,
    }
}

fn parse_stream_event(json: serde_json::Value) -> QueryEvent {
    let Some(event) = json.get("event") else {
        return QueryEvent::Raw(json);
    };
    match event.get("type").and_then(|t| t.as_str()) {
        Some("content_block_delta") => {
            let delta = event.get("delta");
            match delta.and_then(|d| d.get("type")).and_then(|t| t.as_str()) {
                Some("text_delta") => {
                    QueryEvent::Text(delta.map(|d| str_field(d, "text")).unwrap_or_default())
                }
                Some("thinking_delta") => QueryEvent::Thinking(
                    delta.map(|d| str_field(d, "thinking")).unwrap_or_default(),
                ),
                _ => QueryEvent::Raw(json),
            }
        }
        Some("message_delta") => match event
            .get("usage")
            .and_then(|u| serde_json::from_value::<UsageInfo>(u.clone()).ok())
        {
            Some(usage) => QueryEvent::Usage(usage),
            None => QueryEvent::Raw(json),
        },
        _ => QueryEvent::Raw(json),
    }
}

/// One event per `block_type` block in `message.content`, or the whole line
/// as [`QueryEvent::Raw`] when there is none. Text and thinking blocks are
/// skipped: they already arrived as `content_block_delta`s.
fn content_block_events(
    json: serde_json::Value,
    block_type: &str,
    to_event: fn(&serde_json::Value) -> QueryEvent,
) -> Vec<QueryEvent> {
    let events: Vec<QueryEvent> = json
        .pointer("/message/content")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(|t| t.as_str()) == Some(block_type))
        .map(to_event)
        .collect();
    if events.is_empty() {
        vec![QueryEvent::Raw(json)]
    } else {
        events
    }
}

fn tool_call(block: &serde_json::Value) -> QueryEvent {
    QueryEvent::ToolCall {
        id: str_field(block, "id"),
        tool: str_field(block, "name"),
        input: block.get("input").cloned().unwrap_or_default(),
    }
}

/// `content` is either a plain string or an array of `{type:"text", text}`
/// parts; the array case is flattened into one newline-joined string.
fn tool_result(block: &serde_json::Value) -> QueryEvent {
    let content = match block.get("content") {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => other.to_string(),
        None => String::new(),
    };
    QueryEvent::ToolResult {
        tool_use_id: str_field(block, "tool_use_id"),
        content,
        is_error: block
            .get("is_error")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    }
}

fn parse_control_request(json: serde_json::Value) -> QueryEvent {
    let Some(request) = json
        .get("request")
        .filter(|r| r.get("subtype").and_then(|s| s.as_str()) == Some("can_use_tool"))
    else {
        return QueryEvent::Raw(json);
    };
    QueryEvent::PermissionRequest {
        id: str_field(&json, "request_id"),
        tool: str_field(request, "tool_name"),
        input: request.get("input").cloned().unwrap_or_default(),
        tool_use_id: request
            .get("tool_use_id")
            .and_then(|v| v.as_str())
            .map(String::from),
        suggestions: request
            .get("permission_suggestions")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    }
}

fn parse_result(json: &serde_json::Value, subtype: &str) -> QueryEvent {
    // `is_error` is authoritative (a `success` result can still carry an API
    // error); older CLIs without the field signal errors by subtype.
    let is_error = json
        .get("is_error")
        .and_then(|v| v.as_bool())
        .unwrap_or(subtype != "success");
    QueryEvent::Result {
        result: str_field(json, "result"),
        session_id: str_field(json, "session_id"),
        duration_ms: json
            .get("duration_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        usage: json
            .get("usage")
            .and_then(|u| serde_json::from_value::<UsageInfo>(u.clone()).ok())
            .unwrap_or_default(),
        is_error,
        errors: json
            .get("errors")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// The events of one [`query()`]. Dropping it kills and reaps the CLI.
#[derive(Debug)]
pub struct QueryStream {
    rx: mpsc::Receiver<QueryEvent>,
    responder: PermissionResponder,
}

impl QueryStream {
    /// Handle for answering permission requests raised by this query.
    pub fn responder(&self) -> PermissionResponder {
        self.responder.clone()
    }
}

impl Stream for QueryStream {
    type Item = QueryEvent;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only(mut events: Vec<QueryEvent>) -> QueryEvent {
        assert_eq!(events.len(), 1, "{events:?}");
        events.remove(0)
    }

    #[test]
    fn every_content_block_becomes_its_own_event() {
        let assistant = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"two calls"},{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"a"}},{"type":"tool_use","id":"t2","name":"Grep","input":{"pattern":"b"}}]}}"#;
        let calls: Vec<(String, String)> = parse_cli_line(assistant)
            .into_iter()
            .map(|e| match e {
                QueryEvent::ToolCall { id, tool, .. } => (id, tool),
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(
            calls,
            [("t1".into(), "Read".into()), ("t2".into(), "Grep".into())]
        );

        let user = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"a"},{"type":"tool_result","tool_use_id":"t2","content":[{"type":"text","text":"b"},{"type":"text","text":"c"}],"is_error":true}]}}"#;
        let results: Vec<(String, String, bool)> = parse_cli_line(user)
            .into_iter()
            .map(|e| match e {
                QueryEvent::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } => (tool_use_id, content, is_error),
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(
            results,
            [
                ("t1".into(), "a".into(), false),
                ("t2".into(), "b\nc".into(), true)
            ]
        );

        let text_only =
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]}}"#;
        assert!(matches!(
            only(parse_cli_line(text_only)),
            QueryEvent::Raw(_)
        ));
    }

    #[test]
    fn every_result_subtype_is_a_result_and_is_error_comes_from_the_field() {
        let cases = [
            (
                r#"{"type":"result","subtype":"success","is_error":false,"result":"done"}"#,
                false,
            ),
            // A success subtype can still report an API error.
            (
                r#"{"type":"result","subtype":"success","is_error":true,"result":"API Error"}"#,
                true,
            ),
            (
                r#"{"type":"result","subtype":"error_max_turns","is_error":true,"errors":["Reached maximum number of turns (1)"]}"#,
                true,
            ),
            (
                r#"{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["boom"]}"#,
                true,
            ),
            // Without the field, any non-success subtype is an error.
            (
                r#"{"type":"result","subtype":"error_during_execution"}"#,
                true,
            ),
            (r#"{"type":"result","subtype":"success"}"#, false),
        ];
        for (line, expected) in cases {
            match only(parse_cli_line(line)) {
                QueryEvent::Result { is_error, .. } => assert_eq!(is_error, expected, "{line}"),
                other => panic!("{line} parsed as {other:?}"),
            }
        }
        match only(parse_cli_line(cases[2].0)) {
            QueryEvent::Result { errors, result, .. } => {
                assert_eq!(errors, ["Reached maximum number of turns (1)"]);
                assert!(result.is_empty());
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn can_use_tool_control_request_becomes_permission_request() {
        let line = r#"{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"touch x"},"tool_use_id":"toolu_1","permission_suggestions":[{"type":"addRules"}]}}"#;
        match only(parse_cli_line(line)) {
            QueryEvent::PermissionRequest {
                id,
                tool,
                input,
                tool_use_id,
                suggestions,
            } => {
                assert_eq!(id, "req-1");
                assert_eq!(tool, "Bash");
                assert_eq!(input["command"], "touch x");
                assert_eq!(tool_use_id.as_deref(), Some("toolu_1"));
                assert!(suggestions.is_array());
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn other_control_requests_stay_raw_and_cancels_map() {
        let raw =
            r#"{"type":"control_request","request_id":"r","request":{"subtype":"hook_callback"}}"#;
        assert!(matches!(only(parse_cli_line(raw)), QueryEvent::Raw(_)));
        let cancel = r#"{"type":"control_cancel_request","request_id":"req-1"}"#;
        assert!(
            matches!(only(parse_cli_line(cancel)), QueryEvent::PermissionCancelled { id } if id == "req-1")
        );
    }

    #[test]
    fn control_response_shapes() {
        let allow = control_response_json(
            "req-1",
            &PermissionDecision::Allow {
                updated_input: Some(json!({"command": "ls"})),
            },
        );
        assert_eq!(allow["type"], "control_response");
        assert_eq!(allow["response"]["subtype"], "success");
        assert_eq!(allow["response"]["request_id"], "req-1");
        assert_eq!(allow["response"]["response"]["behavior"], "allow");
        assert_eq!(
            allow["response"]["response"]["updatedInput"]["command"],
            "ls"
        );

        let deny = control_response_json(
            "req-2",
            &PermissionDecision::Deny {
                message: "no".into(),
            },
        );
        assert_eq!(deny["response"]["response"]["behavior"], "deny");
        assert_eq!(deny["response"]["response"]["message"], "no");
        assert!(deny["response"]["response"].get("updatedInput").is_none());
    }

    fn argv_of(options: &QueryOptions) -> Vec<String> {
        command(options)
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn private_mcp_config_is_one_argument_and_tool_free_runs_exclude_it() {
        let mut options = QueryOptions::new("hello");
        let config =
            r#"{"mcpServers":{"private":{"command":"a path/host","args":["--secret-mcp"]}}}"#;
        options.mcp_config = Some(config.into());
        assert!(argv_of(&options).contains(&format!("--mcp-config={config}")));
        options.tools = Tools::None;
        let args = argv_of(&options);
        assert!(!args.contains(&format!("--mcp-config={config}")));
        assert!(args.contains(&"--mcp-config={\"mcpServers\":{}}".into()));
    }

    #[test]
    fn accept_input_streams_without_a_permission_tool_or_a_bare_prompt() {
        let args = argv_of(&QueryOptions::new("hello").accept_input(true));
        assert!(args.iter().any(|arg| arg == "--input-format=stream-json"));
        assert!(!args
            .iter()
            .any(|arg| arg == "--permission-prompt-tool=stdio"));
        assert!(!args.iter().any(|arg| arg == "--" || arg == "hello"));
    }

    #[test]
    fn setting_sources_are_one_argument_and_empty_means_none() {
        let default = argv_of(&QueryOptions::new("hello"));
        assert!(!default
            .iter()
            .any(|arg| arg.starts_with("--setting-sources")));
        let none = argv_of(&QueryOptions::new("hello").setting_sources(Vec::<String>::new()));
        assert!(none.contains(&"--setting-sources=".into()), "{none:?}");
        let some = argv_of(&QueryOptions::new("hello").setting_sources(["project", "local"]));
        assert!(some.contains(&"--setting-sources=project,local".into()));
        let mut extra = QueryOptions::new("hello");
        extra.settings = Some(r#"{"permissions":{"deny":["Read(./.env)"]}}"#.into());
        assert!(argv_of(&extra)
            .contains(&r#"--settings={"permissions":{"deny":["Read(./.env)"]}}"#.into()));
    }

    #[test]
    fn autocompact_is_a_flag_only_when_set() {
        let set = argv_of(&QueryOptions::new("hello").autocompact("300000"));
        assert!(set.iter().any(|arg| arg == "--autocompact=300000"));
        let unset = argv_of(&QueryOptions::new("hello"));
        assert!(!unset.iter().any(|arg| arg.starts_with("--autocompact")));
        assert!(unset.iter().any(|arg| arg == "hello"));
    }

    #[test]
    fn a_user_text_echo_is_not_an_event() {
        let line = user_message_line("steer this");
        assert!(parse_cli_line(line.trim_end()).is_empty());
    }

    #[test]
    fn user_message_line_is_one_json_line() {
        let line = user_message_line("hi\nthere");
        assert!(line.ends_with('\n'));
        let v: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(v["type"], "user");
        assert_eq!(v["message"]["content"], "hi\nthere");
        assert_eq!(line.matches('\n').count(), 1);
    }
}
