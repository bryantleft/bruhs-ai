use std::time::Duration;

use axum::extract::{rejection::PathRejection, Path};
use axum::Json;
use bruhs_models::agent::AgentProvider;
use bruhs_models::logic::cli_login::{
    codex_login_status, says_logged_out, unrecognized_status, CliLogin,
};
use bruhs_models::runtime_models::RuntimeModels;
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

/// GET /api/agent/auth/{provider}. Runtimes without a verified status probe
/// return setup guidance and an unknown status, never a guessed login success.
pub async fn get_auth_for_provider(
    provider: Result<Path<AgentProvider>, PathRejection>,
) -> Result<Json<AuthStatus>, AppError> {
    Ok(Json(auth_status(cli::provider(provider)?).await))
}

/// GET /api/agent/models/{provider}: the models the runtime's CLI lists for
/// the signed-in account (Cursor), or why it could not list them. Runtimes
/// that cannot list models answer with an empty list.
pub async fn get_models_for_provider(
    provider: Result<Path<AgentProvider>, PathRejection>,
) -> Result<Json<RuntimeModels>, AppError> {
    let provider = cli::provider(provider)?;
    Ok(Json(bruhs_core::agent::list_runtime_models(provider).await))
}

async fn auth_status(provider: AgentProvider) -> AuthStatus {
    if let Some(hint) = cli::setup_hint(provider) {
        return AuthStatus::unknown(format!(
            "{} login status is managed by its CLI. {hint}",
            provider.label()
        ));
    }
    let (program, args): (&str, &[&str]) = match provider {
        AgentProvider::Claude => ("claude", &["auth", "status"]),
        AgentProvider::Codex => ("codex", &["login", "status"]),
        AgentProvider::Cursor
        | AgentProvider::Antigravity
        | AgentProvider::Grok
        | AgentProvider::OpenCode
        | AgentProvider::Acp => {
            return AuthStatus::unknown("This runtime manages login in its CLI.")
        }
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
        AgentProvider::Cursor
        | AgentProvider::Antigravity
        | AgentProvider::Grok
        | AgentProvider::OpenCode
        | AgentProvider::Acp => AuthStatus::unknown("This runtime manages login in its CLI."),
    }
}

// ---------------------------------------------------------------------------
// Claude
// ---------------------------------------------------------------------------

/// Why the status is unknown, from whatever the CLI printed.
fn unrecognized(name: &str, exit_ok: bool, stdout: &str, stderr: &str) -> AuthStatus {
    AuthStatus::unknown(unrecognized_status(name, exit_ok, stdout, stderr))
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

/// Parse `codex login status` output with the parser the desktop app also
/// uses (`bruhs_models::logic::cli_login`).
fn parse_codex_status(exit_ok: bool, stdout: &str, stderr: &str) -> AuthStatus {
    match codex_login_status(exit_ok, stdout, stderr) {
        CliLogin::SignedIn { method } => AuthStatus {
            authenticated: true,
            auth_method: method,
            ..Default::default()
        },
        CliLogin::SignedOut => AuthStatus::default(),
        CliLogin::Unknown(error) => AuthStatus::unknown(error),
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
    async fn external_runtime_login_status_is_unknown_with_setup_guidance() {
        for provider in [
            AgentProvider::Cursor,
            AgentProvider::Antigravity,
            AgentProvider::Grok,
            AgentProvider::OpenCode,
        ] {
            let status = auth_status(provider).await;
            assert!(!status.authenticated);
            let message = status.error.unwrap();
            assert!(message.contains(provider.label()));
            assert!(message.contains("terminal"));
        }
    }

    #[tokio::test]
    async fn runtimes_that_cannot_list_models_answer_with_none() {
        for provider in [
            AgentProvider::Claude,
            AgentProvider::Codex,
            AgentProvider::OpenCode,
        ] {
            let Json(models) = get_models_for_provider(Ok(Path(provider))).await.unwrap();
            assert_eq!(models, RuntimeModels::default());
        }
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
