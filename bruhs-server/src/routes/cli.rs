//! Running a provider CLI (`claude`, `codex`) for a request: never without
//! a deadline, and never leaving the child behind when the request is
//! dropped.

use std::process::{Output, Stdio};
use std::time::Duration;

use axum::extract::{rejection::PathRejection, Path};
use bruhs_models::agent::AgentProvider;
use tokio::process::Command;

use crate::error::AppError;

/// Why a CLI produced no output.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("{0} CLI not found. Install it first.")]
    NotFound(&'static str, #[source] std::io::Error),
    #[error("{0} could not be started: {1}")]
    Spawn(&'static str, #[source] std::io::Error),
    #[error("{0} did not finish within {1:?}")]
    TimedOut(&'static str, Duration),
}

/// Human name of `provider`'s CLI.
pub fn cli_name(provider: AgentProvider) -> &'static str {
    match provider {
        AgentProvider::Claude => "Claude Code",
        AgentProvider::Codex => "Codex",
    }
}

/// Run `program args` to completion with stdin closed, killing it if it
/// outlives `timeout` or the caller stops waiting.
pub async fn run(
    name: &'static str,
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<Output, CliError> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(timeout, output).await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            Err(CliError::NotFound(name, error))
        }
        Ok(Err(error)) => Err(CliError::Spawn(name, error)),
        Err(_) => Err(CliError::TimedOut(name, timeout)),
    }
}

/// The `{provider}` path segment; anything but `claude`/`codex` is a 400
/// with the usual JSON error body.
pub fn provider(
    path: Result<Path<AgentProvider>, PathRejection>,
) -> Result<AgentProvider, AppError> {
    path.map(|Path(provider)| provider).map_err(|rejection| {
        AppError::BadRequest(format!("unknown provider: {}", rejection.body_text()))
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_programs_are_not_found_by_kind() {
        let error = run(
            "Fixture",
            "/nonexistent/bruhs-fixture-cli",
            &[],
            Duration::from_secs(5),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(error, CliError::NotFound("Fixture", _)),
            "{error:?}"
        );
        assert_eq!(
            error.to_string(),
            "Fixture CLI not found. Install it first."
        );
    }

    /// A CLI that never exits is killed at the deadline instead of holding
    /// the request (and a process) forever.
    #[tokio::test(start_paused = true)]
    async fn a_hung_cli_times_out() {
        let error = run("Fixture", "sleep", &["3600"], Duration::from_secs(30))
            .await
            .unwrap_err();
        assert!(
            matches!(error, CliError::TimedOut("Fixture", _)),
            "{error:?}"
        );
    }
}
