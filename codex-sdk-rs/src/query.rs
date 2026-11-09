use crate::error::{Error, Result};
use crate::process::{apply_env, CliProcess, Ending};
use crate::rollout;
use crate::types::{CodexAccess, QueryEvent, QueryOptions, Session, UsageInfo};
use futures_core::Stream;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::task::{Context, Poll};
use tokio::io::AsyncWriteExt;
use tokio::process::{ChildStdin, Command};
use tokio::sync::mpsc;

/// Spawn the Codex CLI and return a stream of events.
///
/// The prompt is written to the CLI's stdin (the operand is `-`): `codex
/// exec` treats a `-` operand as "read stdin", so no prompt could otherwise
/// be passed through literally.
///
/// Dropping the stream kills and reaps the CLI. If the CLI fails, the last
/// event is [`QueryEvent::ProcessFailed`].
///
/// # Errors
///
/// [`Error::InvalidSessionId`] for a resume id starting with `-`,
/// [`Error::CliNotFound`] when `codex` is not on `PATH`, and
/// [`Error::Spawn`] for other spawn failures.
///
/// # Panics
///
/// Panics when called outside a Tokio runtime.
pub fn query(options: QueryOptions) -> Result<QueryStream> {
    if let Session::Resume(thread_id) = &options.session {
        if thread_id.starts_with('-') {
            return Err(Error::InvalidSessionId(thread_id.clone()));
        }
    }
    let mut cmd = command(&options);
    cmd.stdin(Stdio::piped());
    // Where this turn's rollout lines will start: before the CLI runs, the
    // resumed thread's rollout ends at its current length.
    let mut turn = TurnRecord {
        codex_home: (options.session != Session::Ephemeral)
            .then(|| rollout::codex_home(&options.env, &options.env_remove, options.inherit_env))
            .flatten(),
        ..TurnRecord::default()
    };
    if let (Some(home), Session::Resume(thread_id)) = (&turn.codex_home, &options.session) {
        turn.rollout = rollout::find_rollout(home, thread_id).map(|path| {
            let start = std::fs::metadata(&path).map_or(0, |meta| meta.len());
            (path, start)
        });
        if turn.rollout.is_none() {
            // Without the earlier total, the reported total is all there is.
            turn.codex_home = None;
        }
    }
    let mut process = CliProcess::spawn("codex", &mut cmd).map_err(Error::spawn)?;
    let stdin = process.take_stdin();
    let prompt = options.prompt;

    let (tx, rx) = mpsc::channel(256);
    tokio::spawn(async move {
        let ending = pump(&mut process, &tx, stdin, prompt, &mut turn).await;
        let failure = process.finish(ending).await;
        // Usage waits for the exit so the rollout is complete.
        if let Some(reported) = turn.usage.take() {
            let usage = turn.rollout_usage().await.unwrap_or(reported);
            let _ = tx.send(QueryEvent::Usage(usage)).await;
        }
        if let Some(failure) = failure {
            // If the consumer is already gone nobody needs the failure.
            let _ = tx.send(QueryEvent::ProcessFailed(failure)).await;
        }
    });

    Ok(QueryStream { rx })
}

/// The `codex` invocation for `options`, minus stdio setup.
///
/// Value-taking options use the single-argument `--flag=value` form so a
/// value that starts with `-` cannot be read as another flag. The operands
/// come last, after `--`: `[SESSION_ID] -`, where `-` makes codex read the
/// prompt from stdin.
fn command(options: &QueryOptions) -> Command {
    let mut cmd = Command::new("codex");

    for entry in &options.config {
        cmd.arg(format!("--config={}={}", entry.key(), entry.value()));
    }

    match options.access {
        CodexAccess::Sandboxed { approval, .. } => {
            cmd.arg(format!("--ask-for-approval={}", approval.as_str()));
        }
        CodexAccess::DangerouslyUnsandboxed => {
            cmd.arg("--dangerously-bypass-approvals-and-sandbox");
        }
    }

    if options.enable_web_search {
        cmd.arg("--search");
    }

    cmd.arg("exec").arg("--json");

    if let Some(ref model) = options.model {
        cmd.arg(format!("--model={model}"));
    }

    if let Some(ref profile) = options.profile {
        cmd.arg(format!("--profile={profile}"));
    }

    if let CodexAccess::Sandboxed { sandbox, .. } = options.access {
        cmd.arg(format!("--sandbox={}", sandbox.as_str()));
    }

    let mut cd = OsString::from("--cd=");
    cd.push(&options.cwd);
    cmd.arg(cd);

    if options.session == Session::Ephemeral {
        cmd.arg("--ephemeral");
    }

    if options.skip_git_repo_check {
        cmd.arg("--skip-git-repo-check");
    }

    if options.ignore_user_config {
        cmd.arg("--ignore-user-config");
    }

    if options.ignore_rules {
        cmd.arg("--ignore-rules");
    }

    match &options.session {
        Session::Resume(thread_id) => {
            cmd.arg("resume").arg("--").arg(thread_id);
        }
        Session::New | Session::Ephemeral => {
            cmd.arg("--");
        }
    }
    cmd.arg("-");

    apply_env(
        &mut cmd,
        options.inherit_env,
        &options.env_remove,
        &options.env,
    );
    cmd
}

/// What one turn learned along the way that its usage depends on.
#[derive(Default)]
struct TurnRecord {
    codex_home: Option<PathBuf>,
    /// The thread's rollout and its length before this turn, if resumed.
    rollout: Option<(PathBuf, u64)>,
    /// A new thread's id, from `thread.started`.
    thread_id: Option<String>,
    /// `turn.completed` usage (the thread's running total), held back.
    usage: Option<UsageInfo>,
}

impl TurnRecord {
    /// This turn's own usage from the rollout, or `None` when it cannot be
    /// read.
    async fn rollout_usage(&self) -> Option<UsageInfo> {
        let (path, start) = match (&self.rollout, &self.codex_home, &self.thread_id) {
            (Some((path, start)), _, _) => (path.clone(), *start),
            (None, Some(home), Some(thread_id)) => {
                let (home, thread_id) = (home.clone(), thread_id.clone());
                let path =
                    tokio::task::spawn_blocking(move || rollout::find_rollout(&home, &thread_id))
                        .await
                        .ok()??;
                (path, 0)
            }
            _ => return None,
        };
        tokio::task::spawn_blocking(move || {
            let contents = std::fs::read_to_string(path).ok()?;
            let start = usize::try_from(start).unwrap_or(usize::MAX);
            rollout::turn_usage(&contents, start)
        })
        .await
        .ok()?
    }
}

/// Send the prompt, then forward parsed stdout lines until the CLI or the
/// consumer is done. Usage is kept in `turn`, not forwarded.
async fn pump(
    process: &mut CliProcess,
    tx: &mpsc::Sender<QueryEvent>,
    stdin: Option<ChildStdin>,
    prompt: String,
    turn: &mut TurnRecord,
) -> Ending {
    let written = tokio::select! {
        () = tx.closed() => return Ending::ConsumerGone,
        written = write_prompt(stdin, &prompt) => written,
    };
    if let Err(e) = written {
        return Ending::Failed(format!("sending the prompt to codex failed: {e}"));
    }
    loop {
        let line = match process.next_line(tx).await {
            Ok(line) => line,
            Err(ending) => return ending,
        };
        if line.trim().is_empty() {
            continue;
        }
        let event = match parse_cli_line(&line) {
            QueryEvent::Usage(usage) => {
                turn.usage = Some(usage);
                continue;
            }
            event => event,
        };
        if let QueryEvent::Init { thread_id } = &event {
            turn.thread_id = Some(thread_id.clone());
        }
        if tx.send(event).await.is_err() {
            return Ending::ConsumerGone;
        }
    }
}

/// Write the whole prompt and close stdin so codex sees EOF.
async fn write_prompt(stdin: Option<ChildStdin>, prompt: &str) -> io::Result<()> {
    let mut stdin = stdin.ok_or_else(|| io::Error::other("stdin was not piped"))?;
    stdin.write_all(prompt.as_bytes()).await?;
    stdin.shutdown().await
}

fn parse_cli_line(line: &str) -> QueryEvent {
    let json: serde_json::Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(e) => return QueryEvent::Error(format!("JSON parse error: {e}")),
    };

    let event_type = json
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or("");

    match event_type {
        "thread.started" => QueryEvent::Init {
            thread_id: json
                .get("thread_id")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string(),
        },
        "item.started" => parse_item_started(json),
        "item.completed" => parse_item_completed(json),
        "turn.completed" => {
            let usage = json
                .get("usage")
                .and_then(|value| serde_json::from_value::<UsageInfo>(value.clone()).ok())
                .unwrap_or_default();
            QueryEvent::Usage(usage)
        }
        "turn.failed" => QueryEvent::Error(turn_failure_message(&json)),
        "error" => QueryEvent::StreamError(
            json.get("message")
                .and_then(|value| value.as_str())
                .unwrap_or("Codex reported an error")
                .to_string(),
        ),
        _ => QueryEvent::Raw(json),
    }
}

/// The plain-text reason of a `turn.failed` event: `error.message`, with
/// fallbacks for a bare `error` string or a top-level `message`.
fn turn_failure_message(json: &serde_json::Value) -> String {
    let error = json.get("error");
    error
        .and_then(|error| error.get("message"))
        .or(error)
        .or_else(|| json.get("message"))
        .and_then(|value| value.as_str())
        .unwrap_or("Codex turn failed")
        .to_string()
}

fn parse_item_started(json: serde_json::Value) -> QueryEvent {
    let Some(item) = json.get("item") else {
        return QueryEvent::Raw(json);
    };

    match item.get("type").and_then(|value| value.as_str()) {
        Some("mcp_tool_call") => QueryEvent::ToolCall {
            id: item["id"].as_str().unwrap_or_default().to_string(),
            tool: format!(
                "mcp__{}__{}",
                item["server"].as_str().unwrap_or_default(),
                item["tool"].as_str().unwrap_or_default()
            ),
            input: item["arguments"].clone(),
        },
        Some("command_execution") => {
            let id = item
                .get("id")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string();
            let command = item
                .get("command")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string();
            QueryEvent::ToolCall {
                id,
                tool: "Bash".to_string(),
                input: serde_json::json!({ "command": command }),
            }
        }
        _ => QueryEvent::Raw(json),
    }
}

fn parse_item_completed(json: serde_json::Value) -> QueryEvent {
    let Some(item) = json.get("item") else {
        return QueryEvent::Raw(json);
    };

    match item.get("type").and_then(|value| value.as_str()) {
        Some("mcp_tool_call") => {
            let result = &item["result"];
            let content = result["content"].as_array().map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|block| block["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            });
            QueryEvent::ToolResult {
                tool_use_id: item["id"].as_str().unwrap_or_default().to_string(),
                content: content.filter(|text| !text.is_empty()).unwrap_or_else(|| {
                    item["error"]["message"]
                        .as_str()
                        .map_or_else(|| result.to_string(), String::from)
                }),
                is_error: item["status"] == "failed"
                    || !item["error"].is_null()
                    || result["isError"] == true,
            }
        }
        Some("agent_message") => {
            let text = item
                .get("text")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string();
            QueryEvent::Text(text)
        }
        Some("command_execution") => {
            let tool_use_id = item
                .get("id")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string();
            let content = item
                .get("aggregated_output")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string();
            let exit_code = item.get("exit_code").and_then(|value| value.as_i64());
            let status = item.get("status").and_then(|value| value.as_str());
            let is_error = exit_code.unwrap_or(0) != 0 || status == Some("failed");
            QueryEvent::ToolResult {
                tool_use_id,
                content,
                is_error,
            }
        }
        _ => QueryEvent::Raw(json),
    }
}

/// The events of one [`query()`]. Dropping it kills and reaps the CLI.
#[derive(Debug)]
pub struct QueryStream {
    rx: mpsc::Receiver<QueryEvent>,
}

impl Stream for QueryStream {
    type Item = QueryEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::parse_cli_line;
    use crate::types::QueryEvent;

    #[test]
    fn parse_cli_line_should_emit_init_when_thread_starts() {
        let line = r#"{"type":"thread.started","thread_id":"thread_123"}"#;

        let event = parse_cli_line(line);

        assert!(matches!(event, QueryEvent::Init { thread_id } if thread_id == "thread_123"));
    }

    #[test]
    fn parse_cli_line_should_emit_text_when_agent_message_completes() {
        let line = r#"{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"pong"}}"#;

        let event = parse_cli_line(line);

        assert!(matches!(event, QueryEvent::Text(text) if text == "pong"));
    }

    #[test]
    fn parse_cli_line_should_emit_tool_call_when_command_starts() {
        let line = r#"{"type":"item.started","item":{"id":"item_0","type":"command_execution","command":"/bin/zsh -lc pwd","aggregated_output":"","exit_code":null,"status":"in_progress"}}"#;

        let event = parse_cli_line(line);

        assert!(
            matches!(event, QueryEvent::ToolCall { id, tool, input } if id == "item_0" && tool == "Bash" && input["command"] == "/bin/zsh -lc pwd")
        );
    }

    #[test]
    fn parse_cli_line_should_emit_tool_result_when_command_completes() {
        let line = r#"{"type":"item.completed","item":{"id":"item_0","type":"command_execution","command":"/bin/zsh -lc pwd","aggregated_output":"/repo\n","exit_code":0,"status":"completed"}}"#;

        let event = parse_cli_line(line);

        assert!(
            matches!(event, QueryEvent::ToolResult { tool_use_id, content, is_error } if tool_use_id == "item_0" && content == "/repo\n" && !is_error)
        );
    }

    #[test]
    fn mcp_calls_keep_the_server_tool_and_literal_arguments() {
        let line = r#"{"type":"item.started","item":{"id":"item_1","type":"mcp_tool_call","server":"bruhs_secrets","tool":"request_secret","arguments":{"name":"Diagnostic","env_var":"SMOKE_CREDENTIAL","command":["/usr/bin/true"]},"result":null,"error":null,"status":"in_progress"}}"#;
        assert!(
            matches!(parse_cli_line(line), QueryEvent::ToolCall { id, tool, input }
            if id == "item_1" && tool == "mcp__bruhs_secrets__request_secret"
            && input["command"] == serde_json::json!(["/usr/bin/true"]))
        );
    }

    #[test]
    fn mcp_text_results_and_operation_errors_reach_the_transcript() {
        for (status, code, is_error) in [("completed", 0, false), ("completed", 41, true)] {
            let content = serde_json::json!({"status":status,"exit_code":code}).to_string();
            let line = serde_json::json!({
                "type":"item.completed", "item": {
                    "id":"item_1", "type":"mcp_tool_call", "status":"completed", "error":null,
                    "result":{"content":[{"type":"text","text":content}],"isError":is_error}
                }
            })
            .to_string();
            assert!(
                matches!(parse_cli_line(&line), QueryEvent::ToolResult { tool_use_id, content: actual, is_error: failed }
                if tool_use_id == "item_1" && actual == content && failed == is_error)
            );
        }
    }

    #[test]
    fn mcp_approval_denials_are_tool_failures() {
        let line = r#"{"type":"item.completed","item":{"id":"item_1","type":"mcp_tool_call","server":"bruhs_secrets","tool":"request_secret","result":null,"error":{"message":"MCP tool call requires approval, but approval policy is never"},"status":"failed"}}"#;
        assert!(
            matches!(parse_cli_line(line), QueryEvent::ToolResult { tool_use_id, content, is_error }
            if tool_use_id == "item_1" && content == "MCP tool call requires approval, but approval policy is never" && is_error)
        );
    }

    #[test]
    fn turn_failed_reason_is_plain_text() {
        let nested =
            r#"{"type":"turn.failed","error":{"message":"stream disconnected before completion"}}"#;
        assert!(
            matches!(parse_cli_line(nested), QueryEvent::Error(m) if m == "stream disconnected before completion")
        );
        let bare = r#"{"type":"turn.failed","error":"quota exceeded"}"#;
        assert!(matches!(parse_cli_line(bare), QueryEvent::Error(m) if m == "quota exceeded"));
        let unknown = r#"{"type":"turn.failed"}"#;
        assert!(
            matches!(parse_cli_line(unknown), QueryEvent::Error(m) if m == "Codex turn failed")
        );
    }

    #[test]
    fn top_level_error_events_are_stream_errors() {
        let line = r#"{"type":"error","message":"Reconnecting... waiting for network"}"#;
        assert!(
            matches!(parse_cli_line(line), QueryEvent::StreamError(m) if m == "Reconnecting... waiting for network")
        );
    }

    #[test]
    fn parse_cli_line_should_emit_usage_when_turn_completes() {
        let line = r#"{"type":"turn.completed","usage":{"input_tokens":10,"cached_input_tokens":4,"output_tokens":3,"reasoning_output_tokens":1}}"#;

        let event = parse_cli_line(line);

        assert!(
            matches!(event, QueryEvent::Usage(usage) if usage.input_tokens == 10 && usage.cached_input_tokens == 4 && usage.output_tokens == 3 && usage.reasoning_output_tokens == 1)
        );
    }
}
