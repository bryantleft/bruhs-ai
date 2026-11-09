use std::time::Duration;

use axum::extract::{rejection::PathRejection, Path};
use axum::Json;
use bruhs_models::agent::AgentProvider;
use serde_json::{json, Value};

use crate::error::{AppError, WithSuccessFlag};
use crate::routes::cli::{self, CliError};

/// The CLIs normally need a TTY/browser for the OAuth flow; give up (and
/// kill the CLI) after this long.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(60);

/// POST /api/agent/login — triggers `claude auth login` (backward compat).
pub async fn post_login() -> Result<Json<Value>, WithSuccessFlag> {
    run_login(AgentProvider::Claude).await
}

/// POST /api/agent/login/{provider} — triggers the provider's interactive
/// login: `claude auth login` or `codex login`. Unknown providers are a 400.
pub async fn post_login_for_provider(
    provider: Result<Path<AgentProvider>, PathRejection>,
) -> Result<Json<Value>, WithSuccessFlag> {
    run_login(cli::provider(provider)?).await
}

/// Runs the login CLI with a 60-second timeout and returns its stdout. In a
/// non-interactive server environment this will likely fail; the error
/// tells the user to run the login in a terminal instead. A timeout is a
/// 504.
async fn run_login(provider: AgentProvider) -> Result<Json<Value>, WithSuccessFlag> {
    let (program, args, manual): (&str, &[&str], &str) = match provider {
        AgentProvider::Claude => ("claude", &["auth", "login"], "claude auth login"),
        AgentProvider::Codex => ("codex", &["login"], "codex login"),
    };
    let name = cli::cli_name(provider);
    let output = cli::run(name, program, args, LOGIN_TIMEOUT)
        .await
        .map_err(|error| login_error(error, manual))?;
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        return Ok(Json(json!({ "success": true, "output": stdout })));
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    tracing::warn!(provider = %provider, stderr = %stderr, "agent login failed");
    Err(AppError::internal_msg(if stderr.is_empty() {
        format!(
            "{name} login failed (it normally requires a TTY — try running `{manual}` in a terminal)"
        )
    } else {
        stderr
    })
    .into())
}

/// A login CLI that did not finish: a timeout is a 504 (it used to be 408,
/// which blames the client), anything else a 500.
fn login_error(error: CliError, manual: &str) -> AppError {
    match error {
        CliError::TimedOut(name, _) => AppError::GatewayTimeout(format!(
            "{name} login timed out (try running `{manual}` in a terminal — it needs a TTY)"
        )),
        error => AppError::internal(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[test]
    fn a_login_timeout_is_a_gateway_timeout() {
        let timed_out = login_error(CliError::TimedOut("Codex", LOGIN_TIMEOUT), "codex login");
        assert_eq!(timed_out.status(), StatusCode::GATEWAY_TIMEOUT);
        assert!(timed_out.to_string().contains("codex login"));
        let missing = login_error(
            CliError::NotFound("Codex", std::io::ErrorKind::NotFound.into()),
            "codex login",
        );
        assert_eq!(missing.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            missing.to_string(),
            "Codex CLI not found. Install it first."
        );
    }
}
