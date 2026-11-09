use std::time::Duration;

use axum::extract::{rejection::PathRejection, Path};
use axum::Json;
use bruhs_models::agent::AgentProvider;
use serde::Serialize;
use serde_json::Value;

use crate::error::AppError;
use crate::routes::cli;

/// `claude auth status` / `codex login status` answer at once; a CLI that
/// takes longer is stuck and is killed.
const STATUS_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Serialize, Default)]
pub struct AuthStatus {
    pub authenticated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_method: Option<String>,
    /// Set when the status could not be determined (as opposed to a clear
    /// "not logged in").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl AuthStatus {
    fn unknown(error: impl Into<String>) -> Self {
        Self {
            authenticated: false,
            error: Some(error.into()),
            ..Default::default()
        }
    }
}

/// GET /api/agent/auth — Claude status (kept for backward compatibility).
pub async fn get_auth() -> Json<AuthStatus> {
    Json(auth_status(AgentProvider::Claude).await)
}

/// GET /api/agent/auth/{provider} — auth status for `claude` (`claude auth
/// status`, JSON) or `codex` (`codex login status`, plain text). Any other
/// provider is a 400.
pub async fn get_auth_for_provider(
    provider: Result<Path<AgentProvider>, PathRejection>,
) -> Result<Json<AuthStatus>, AppError> {
    Ok(Json(auth_status(cli::provider(provider)?).await))
}

async fn auth_status(provider: AgentProvider) -> AuthStatus {
    let (program, args): (&str, &[&str]) = match provider {
        AgentProvider::Claude => ("claude", &["auth", "status"]),
        AgentProvider::Codex => ("codex", &["login", "status"]),
    };
    let output = match cli::run(cli::cli_name(provider), program, args, STATUS_TIMEOUT).await {
        Ok(output) => output,
        Err(error) => return AuthStatus::unknown(error.to_string()),
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let exit_ok = output.status.success();
    match provider {
        AgentProvider::Claude => parse_claude_status(exit_ok, &stdout, &stderr),
        AgentProvider::Codex => parse_codex_status(exit_ok, &stdout, &stderr),
    }
}

// ---------------------------------------------------------------------------
// Claude
// ---------------------------------------------------------------------------

/// Text that means "logged out" in either CLI's output.
fn says_logged_out(lower: &str) -> bool {
    lower.contains("not logged in")
        || lower.contains("not authenticated")
        || lower.contains("unauthenticated")
        || lower.contains("login required")
}

/// Why the status is unknown, from whatever the CLI printed.
fn unrecognized(name: &str, exit_ok: bool, stdout: &str, stderr: &str) -> AuthStatus {
    let detail = [stderr.trim(), stdout.trim()]
        .into_iter()
        .find(|text| !text.is_empty());
    AuthStatus::unknown(match (detail, exit_ok) {
        (Some(detail), _) => format!("{name} reported: {detail}"),
        (None, true) => format!("{name} printed no login status"),
        (None, false) => format!("{name} failed without output"),
    })
}

/// Parse `claude auth status` output (JSON preferred, text fallback). Only
/// a successful run that says it is logged in counts as authenticated; a
/// failed run, empty or unrecognized output is "unknown" (with `error`).
/// Pure.
fn parse_claude_status(exit_ok: bool, stdout: &str, stderr: &str) -> AuthStatus {
    if let Ok(value) = serde_json::from_str::<Value>(stdout.trim()) {
        if value.is_object() {
            return parse_claude_json(exit_ok, &value, stderr);
        }
    }
    let lower = format!("{stdout}\n{stderr}").to_lowercase();
    if says_logged_out(&lower) {
        return AuthStatus::default();
    }
    if exit_ok && lower.contains("logged in") {
        return AuthStatus {
            authenticated: true,
            ..Default::default()
        };
    }
    unrecognized("Claude Code", exit_ok, stdout, stderr)
}

fn parse_claude_json(exit_ok: bool, value: &Value, stderr: &str) -> AuthStatus {
    let flag = |key: &str| value.get(key).and_then(Value::as_bool);
    let text = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| value.get(*key).and_then(Value::as_str))
            .map(String::from)
    };
    // An explicit flag wins over the presence of account fields.
    let logged_in = flag("loggedIn")
        .or_else(|| flag("authenticated"))
        .unwrap_or_else(|| {
            matches!(
                value.get("status").and_then(Value::as_str),
                Some("authenticated" | "active")
            ) || value.get("account").is_some()
                || value.get("email").is_some()
                || value.get("user").is_some()
        });
    if !exit_ok && logged_in {
        // Contradictory: the CLI failed but printed a logged-in record.
        return unrecognized("Claude Code", exit_ok, "", stderr);
    }
    AuthStatus {
        authenticated: logged_in,
        user: text(&["user", "email", "account"]),
        subscription_type: text(&["subscription_type", "subscriptionType", "plan"]),
        auth_method: text(&["auth_method", "authMethod"]),
        error: None,
    }
}

// ---------------------------------------------------------------------------
// Codex
// ---------------------------------------------------------------------------

/// Parse `codex login status` output. Codex prints plain text like
/// `Logged in using ChatGPT` (exit 0) when authenticated, or a "not logged in"
/// message otherwise. Pure so it can be unit-tested without spawning the CLI.
fn parse_codex_status(exit_ok: bool, stdout: &str, stderr: &str) -> AuthStatus {
    let lower = format!("{stdout}\n{stderr}").to_lowercase();
    if says_logged_out(&lower) {
        return AuthStatus::default();
    }
    if !(exit_ok && lower.contains("logged in")) {
        return unrecognized("Codex", exit_ok, stdout, stderr);
    }
    // "Logged in using ChatGPT" → auth_method = "ChatGPT".
    let auth_method = stdout
        .lines()
        .find_map(|l| l.trim().strip_prefix("Logged in using "))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    AuthStatus {
        authenticated: true,
        auth_method,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_logged_in_chatgpt() {
        let s = parse_codex_status(true, "Logged in using ChatGPT\n", "");
        assert!(s.authenticated);
        assert_eq!(s.auth_method.as_deref(), Some("ChatGPT"));
    }

    #[test]
    fn codex_logged_in_api_key() {
        let s = parse_codex_status(true, "Logged in using an API key\n", "");
        assert!(s.authenticated);
        assert_eq!(s.auth_method.as_deref(), Some("an API key"));
    }

    #[test]
    fn codex_not_logged_in() {
        let s = parse_codex_status(false, "Not logged in\n", "");
        assert!(!s.authenticated);
        assert!(s.auth_method.is_none());
    }

    #[test]
    fn codex_not_logged_in_even_if_exit_ok() {
        // Defensive: a "not logged in" body must win even on a 0 exit.
        let s = parse_codex_status(true, "Not logged in. Run `codex login`.\n", "");
        assert!(!s.authenticated);
    }

    #[test]
    fn claude_json_logged_in() {
        let s = parse_claude_status(
            true,
            r#"{"loggedIn":true,"subscriptionType":"max","email":"a@b.c"}"#,
            "",
        );
        assert!(s.authenticated);
        assert_eq!(s.subscription_type.as_deref(), Some("max"));
        assert_eq!(s.user.as_deref(), Some("a@b.c"));
        assert!(s.error.is_none());
    }

    /// A failed run or output the parser does not recognize used to read
    /// as authenticated. It is now "unknown": not authenticated, with why.
    #[test]
    fn claude_failure_or_unrecognized_output_is_never_authenticated() {
        for (exit_ok, stdout, stderr) in [
            (false, "", ""),
            (true, "", ""),
            (false, "", "error: something broke"),
            (true, "Some new output format", ""),
            (false, r#"{"loggedIn":true,"email":"a@b.c"}"#, "crashed"),
        ] {
            let s = parse_claude_status(exit_ok, stdout, stderr);
            assert!(!s.authenticated, "{exit_ok} {stdout:?} {stderr:?}");
            assert!(s.error.is_some(), "{exit_ok} {stdout:?} {stderr:?}");
        }
        // A clear "logged out" is not an error.
        for (exit_ok, stdout) in [
            (false, "Not logged in. Run claude auth login."),
            (false, r#"{"loggedIn":false}"#),
            (true, r#"{"loggedIn":false,"email":"a@b.c"}"#),
        ] {
            let s = parse_claude_status(exit_ok, stdout, "");
            assert!(!s.authenticated, "{stdout}");
            assert!(s.error.is_none(), "{stdout}");
        }
        assert!(parse_claude_status(true, "Logged in as a@b.c", "").authenticated);
    }

    #[test]
    fn codex_unrecognized_output_is_unknown() {
        let s = parse_codex_status(false, "", "panic: boom");
        assert!(!s.authenticated);
        assert_eq!(s.error.as_deref(), Some("Codex reported: panic: boom"));
        assert!(parse_codex_status(true, "", "").error.is_some());
        assert!(parse_codex_status(false, "Not logged in\n", "")
            .error
            .is_none());
    }

    #[tokio::test]
    async fn unknown_providers_are_rejected() {
        let app = axum::Router::new().route(
            "/api/agent/auth/{provider}",
            axum::routing::get(get_auth_for_provider),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/api/agent/auth/gateway",
            listener.local_addr().unwrap()
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(url)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        let body: Value = response.json().await.unwrap();
        assert!(body["error"].as_str().unwrap().contains("unknown provider"));
        server.abort();
    }
}
