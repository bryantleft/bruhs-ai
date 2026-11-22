//! Quota signals, never inferred from context tokens. Protocol references:
//! <https://code.claude.com/docs/en/agent-sdk/typescript#sdkratelimitevent>
//! <https://developers.openai.com/codex/app-server#6-rate-limits-chatgpt>

use std::{collections::BTreeMap, process::Stdio, sync::Mutex};

use bruhs_models::agent::AgentProvider;
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::Command,
};

// Product policy: reserve the last 20% of reported plan windows for the user's
// own chats. Quota metadata has a 64 KiB frame budget, sufficient for dozens of
// provider windows; an oversized or unknown response skips optional work.
const USED_PERCENT_LIMIT: f64 = 80.0;
const MAX_FRAME_BYTES: u64 = 64 * 1024;
static CLAUDE: Mutex<ClaudeLimits> = Mutex::new(ClaudeLimits(BTreeMap::new()));

struct ClaudeLimits(BTreeMap<String, Option<i64>>);

impl ClaudeLimits {
    fn observe(&mut self, event: &Value, now: i64) {
        if event["type"] != "rate_limit_event" {
            return;
        }
        let info = &event["rate_limit_info"];
        let bucket = info["rateLimitType"].as_str().unwrap_or("account");
        let until = info["resetsAt"].as_i64();
        if until.is_some_and(|until| until <= now) {
            return;
        }
        match info["status"].as_str() {
            Some("allowed_warning" | "rejected") => {
                self.0.insert(bucket.to_string(), until);
            }
            Some("allowed") => {
                if info["utilization"]
                    .as_f64()
                    .is_some_and(|used| used * 100.0 >= USED_PERCENT_LIMIT)
                {
                    self.0.insert(bucket.to_string(), until);
                } else {
                    self.0.remove(bucket);
                }
            }
            _ => {}
        }
    }

    fn permits(&mut self, now: i64) -> bool {
        self.0
            .retain(|_, until| until.is_none_or(|until| until > now));
        self.0.is_empty()
    }
}

pub(super) fn observe_claude(event: &Value) {
    if let Ok(mut limits) = CLAUDE.lock() {
        limits.observe(event, chrono::Utc::now().timestamp());
    }
}

pub(super) async fn permits(provider: AgentProvider) -> bool {
    match provider {
        AgentProvider::Claude => CLAUDE
            .lock()
            .is_ok_and(|mut limits| limits.permits(chrono::Utc::now().timestamp())),
        AgentProvider::Cursor
        | AgentProvider::Antigravity
        | AgentProvider::Grok
        | AgentProvider::Acp
        | AgentProvider::OpenCode => false,
        AgentProvider::Codex => match read_codex_limits().await {
            Ok(result) => codex_permits(&result, chrono::Utc::now().timestamp()),
            Err(error) => {
                tracing::debug!(%error, "prompt suggestion skipped: Codex quota unavailable");
                false
            }
        },
    }
}

fn codex_permits(result: &Value, now: i64) -> bool {
    let buckets: Vec<&Value> = match result["rateLimitsByLimitId"]
        .as_object()
        .filter(|buckets| !buckets.is_empty())
    {
        Some(buckets) => buckets.values().collect(),
        None => vec![&result["rateLimits"]],
    };
    let mut known = false;
    for bucket in buckets {
        if !bucket["rateLimitReachedType"].is_null() {
            return false;
        }
        for window in ["primary", "secondary"].map(|name| &bucket[name]) {
            if window.is_null() {
                continue;
            }
            let Some(used) = window["usedPercent"]
                .as_f64()
                .filter(|used| (0.0..=100.0).contains(used))
            else {
                return false;
            };
            let Some(resets) = window["resetsAt"].as_i64() else {
                return false;
            };
            if resets <= now {
                continue;
            }
            known = true;
            if used >= USED_PERCENT_LIMIT {
                return false;
            }
        }
    }
    known
}

/// Read account metadata through the installed CLI's own authentication. No
/// thread, turn, login action, token-file read, or inference request is made.
pub(crate) async fn read_codex_limits() -> std::io::Result<Value> {
    read_codex_quota(false).await.map(|(limits, _)| limits)
}

/// Read identity from the same app-server session as its rate-limit observation.
pub(crate) async fn read_codex_quota(
    with_identity: bool,
) -> std::io::Result<(Value, Option<String>)> {
    let mut command = Command::new("codex");
    command
        .args([
            "app-server",
            "--stdio",
            "-c",
            "features.plugins=false",
            "-c",
            "features.apps=false",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    crate::connections::protect_command(&mut command);
    let mut child = command.spawn()?;
    let Some(mut stdin) = child.stdin.take() else {
        return Err(std::io::Error::other("Codex quota stdin unavailable"));
    };
    let Some(stdout) = child.stdout.take() else {
        return Err(std::io::Error::other("Codex quota stdout unavailable"));
    };
    let mut stdout = BufReader::new(stdout);
    let initialize = json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"bruhs_prompt_suggestions","version":env!("CARGO_PKG_VERSION")}}});
    stdin
        .write_all(format!("{initialize}\n").as_bytes())
        .await?;
    read_response(&mut stdout, 1).await?;
    stdin
        .write_all(
            b"{\"method\":\"initialized\"}\n{\"id\":2,\"method\":\"account/rateLimits/read\"}\n",
        )
        .await?;
    let response = read_response(&mut stdout, 2).await;
    let mut email = None;
    if with_identity && response.is_ok() {
        let account_id = 3;
        let request =
            json!({"id":account_id,"method":"account/read","params":{"refreshToken":false}});
        let account = async {
            stdin.write_all(format!("{request}\n").as_bytes()).await?;
            read_response(&mut stdout, account_id).await
        };
        if let Ok(Ok(data)) = tokio::time::timeout(std::time::Duration::from_secs(2), account).await
        {
            if data["account"]["type"] == "chatgpt" {
                email = data["account"]["email"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
            }
        }
    }
    // Older CLIs can close stdout immediately after returning limits.
    let _ = child.kill().await;
    response.map(|limits| (limits, email))
}

async fn read_response(
    reader: &mut BufReader<tokio::process::ChildStdout>,
    id: u64,
) -> std::io::Result<Value> {
    loop {
        let mut frame = Vec::new();
        let count = (&mut *reader)
            .take(MAX_FRAME_BYTES + 1)
            .read_until(b'\n', &mut frame)
            .await?;
        if count == 0 || u64::try_from(count).unwrap_or(u64::MAX) > MAX_FRAME_BYTES {
            return Err(std::io::Error::other(
                "Codex quota response missing or larger than 64 KiB",
            ));
        }
        let message: Value = serde_json::from_slice(&frame).map_err(std::io::Error::other)?;
        if message["id"].as_u64() != Some(id) {
            continue;
        }
        if message.get("error").is_some() {
            return Err(std::io::Error::other("Codex quota metadata request failed"));
        }
        return message
            .get("result")
            .cloned()
            .ok_or_else(|| std::io::Error::other("Codex quota metadata result missing"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_warnings_block_until_reset_without_treating_tokens_as_quota() {
        let mut limits = ClaudeLimits(BTreeMap::new());
        limits.observe(&json!({"type":"rate_limit_event","rate_limit_info":{"status":"allowed_warning","resetsAt":200}}), 100);
        assert!(!limits.permits(199));
        assert!(limits.permits(200));
    }

    #[test]
    fn a_healthy_claude_window_does_not_clear_a_different_warning() {
        let mut limits = ClaudeLimits(BTreeMap::new());
        limits.observe(&json!({"type":"rate_limit_event","rate_limit_info":{"status":"rejected","rateLimitType":"seven_day"}}), 100);
        limits.observe(&json!({"type":"rate_limit_event","rate_limit_info":{"status":"allowed","rateLimitType":"five_hour"}}), 100);
        assert!(!limits.permits(100));
        limits.observe(&json!({"type":"rate_limit_event","rate_limit_info":{"status":"allowed","rateLimitType":"seven_day"}}), 100);
        assert!(limits.permits(100));
    }

    #[test]
    fn codex_reserves_twenty_percent_in_every_active_window() {
        let mut result =
            json!({"rateLimits":{"primary":{"usedPercent":79,"resetsAt":200},"secondary":null}});
        assert!(codex_permits(&result, 100));
        result["rateLimits"]["primary"]["usedPercent"] = json!(80);
        assert!(!codex_permits(&result, 100));
        result["rateLimits"]["primary"]["usedPercent"] = json!(10);
        result["rateLimits"]["secondary"] = json!({"usedPercent":95,"resetsAt":500});
        assert!(!codex_permits(&result, 100));
    }

    #[test]
    fn codex_skips_unknown_expired_and_rejected_snapshots() {
        for result in [
            json!({}),
            json!({"rateLimits":{"primary":{"usedPercent":20,"resetsAt":99}}}),
            json!({"rateLimits":{"rateLimitReachedType":"usage_limit","primary":{"usedPercent":20,"resetsAt":200}}}),
        ] {
            assert!(!codex_permits(&result, 100));
        }
    }
}
