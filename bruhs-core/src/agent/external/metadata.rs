//! Read metadata from installed provider CLIs in a disposable directory.
use super::AgentProvider;
use serde_json::Value;
use std::process::Stdio;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::Command,
};

const MAX_METADATA_BYTES: u64 = 8 * 1024 * 1024;

pub(super) async fn model_list(provider: AgentProvider) -> Option<Value> {
    read(Request::Models(provider))
        .await
        .map(|(payload, _)| payload)
}

#[derive(Default)]
pub(crate) struct ClaudeAccount {
    pub email: Option<String>,
    pub plan: Option<String>,
}

pub(crate) async fn claude_usage_metadata() -> Option<(Value, ClaudeAccount)> {
    read(Request::ClaudeUsage).await
}

enum Request {
    Models(AgentProvider),
    ClaudeUsage,
}

async fn read(request: Request) -> Option<(Value, ClaudeAccount)> {
    let provider = match request {
        Request::Models(provider) => provider,
        Request::ClaudeUsage => AgentProvider::Claude,
    };

    let cwd = tempfile::tempdir().ok()?;
    let cli = crate::updates::find_on_path(provider.as_str(), std::env::var_os("PATH").as_ref())?;
    let mut cmd = Command::new(cli);
    cmd.current_dir(cwd.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    crate::connections::protect_command(&mut cmd);
    #[cfg(unix)]
    cmd.process_group(0);
    match provider {
        AgentProvider::Codex => {
            crate::agent::disable_codex_analytics(&mut cmd);
            cmd.args(["debug", "models"]).stdin(Stdio::null());
        }
        AgentProvider::Claude => {
            let options = crate::agent::claude_query_options_from_env("");
            for key in options.env_remove {
                cmd.env_remove(key);
            }
            cmd.envs(options.env);
            cmd.args([
                "-p",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
                "--no-session-persistence",
                "--setting-sources",
                "user",
                "--settings",
                r#"{"disableAllHooks":true}"#,
                "--strict-mcp-config",
                "--mcp-config",
                r#"{"mcpServers":{}}"#,
                "--tools",
                "",
                "--disable-slash-commands",
            ])
            .stdin(Stdio::piped());
        }
        _ => return None,
    }
    let mut child = cmd.spawn().ok()?;
    #[cfg(unix)]
    let mut group = super::process::ProcessGroup::of(&child);
    let stdout = child.stdout.take()?.take(MAX_METADATA_BYTES + 1);
    let mut account = ClaudeAccount::default();
    let data = if provider == AgentProvider::Codex {
        let mut bytes = Vec::new();
        let count = BufReader::new(stdout).read_to_end(&mut bytes).await.ok()?;
        if count as u64 > MAX_METADATA_BYTES {
            return None;
        }
        let status = child.wait().await.ok()?;
        #[cfg(unix)]
        group.release();
        if !status.success() {
            return None;
        }
        serde_json::from_slice(&bytes).ok()?
    } else {
        let mut input = child.stdin.take()?;
        let mut lines = BufReader::new(stdout).lines();
        let requests = [("bruhs-models", "initialize"), ("bruhs-usage", "get_usage")];
        let mut payload = Value::Null;
        for (id, kind) in requests
            .into_iter()
            .take(if matches!(request, Request::ClaudeUsage) {
                2
            } else {
                1
            })
        {
            let frame = serde_json::json!({"type":"control_request","request_id":id,"request":{"subtype":kind}});
            input
                .write_all(format!("{frame}\n").as_bytes())
                .await
                .ok()?;
            loop {
                let line = lines.next_line().await.ok()??;
                let Ok(v) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if v["type"] == "control_response" && v["response"]["request_id"] == id {
                    if v["response"]["subtype"] == "error" {
                        return None;
                    }
                    payload = v["response"].get("response")?.clone();
                    if kind == "initialize" {
                        account.plan = payload["account"]["subscriptionType"]
                            .as_str()
                            .map(str::to_owned);
                        account.email = payload["account"]["email"]
                            .as_str()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned);
                    }
                    break;
                }
            }
        }
        payload
    };
    Some((data, account))
}
