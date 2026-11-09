//! `/second-opinion`: one read-only review of a thread's last answer by the
//! other local provider. It is not a turn of the thread: no session is
//! saved, nothing can be edited, and the run is bounded, so it never changes
//! the thread's own provider session or its prompt cache.

use crate::agent::{claude_query_options_from_env, UsageInfo};
use bruhs_models::agent::AgentProvider;
use bruhs_models::logic::handoff::{reviewer_for, second_opinion_prompt, SECOND_OPINION_MAX_TURNS};
use codex_sdk_rs::{ApprovalPolicy, CodexAccess, SandboxMode, Session};
use std::time::Duration;
use tokio_stream::StreamExt;

/// Longest a review may run.
const REVIEW_TIMEOUT: Duration = Duration::from_secs(300);

/// What to review.
#[derive(Debug, Clone)]
pub struct ReviewRequest {
    /// The provider of the thread under review; the other one reviews.
    pub thread_provider: AgentProvider,
    /// Where the reviewer reads code.
    pub project_path: String,
    /// What the user asked for.
    pub task: String,
    /// The answer under review.
    pub answer: String,
    /// What to look at most closely; may be empty.
    pub focus: String,
}

/// A finished review.
#[derive(Debug, Clone)]
pub struct Review {
    pub reviewer: AgentProvider,
    pub text: String,
    pub usage: UsageInfo,
}

/// Why no review came back.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ReviewError {
    #[error("{0}")]
    Failed(String),
    #[error("The reviewer returned no text.")]
    Empty,
    #[error("The review took longer than five minutes.")]
    TimedOut,
}

/// Run the review and wait for its text.
///
/// # Errors
///
/// [`ReviewError`] when the reviewer CLI cannot start, fails, says nothing,
/// or runs past five minutes.
pub async fn second_opinion(request: ReviewRequest) -> Result<Review, ReviewError> {
    let reviewer = reviewer_for(request.thread_provider);
    let prompt = second_opinion_prompt(&request.focus, &request.task, &request.answer);
    let run = async {
        match reviewer {
            AgentProvider::Claude => review_with_claude(prompt, &request.project_path).await,
            AgentProvider::Codex => review_with_codex(prompt, &request.project_path).await,
        }
    };
    let (text, usage) = tokio::time::timeout(REVIEW_TIMEOUT, run)
        .await
        .map_err(|_| ReviewError::TimedOut)??;
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err(ReviewError::Empty);
    }
    Ok(Review {
        reviewer,
        text,
        usage,
    })
}

/// Claude options for a review: no settings files, no permission prompts
/// and only deny rules (the user's and the project's, plus the write tools),
/// so only reads and searches run.
fn claude_review_options(prompt: String, project_path: &str) -> claude_sdk_rs::QueryOptions {
    let mut options = claude_query_options_from_env(&prompt)
        .cwd(project_path)
        .permission_mode(claude_sdk_rs::PermissionMode::Default)
        .permission_prompts(false)
        .include_partial_messages(false)
        .max_turns(SECOND_OPINION_MAX_TURNS);
    // A review must not quote a file the user or the project walled off.
    let mut files = crate::agent::user_claude_settings();
    files.extend(crate::agent::claude_settings_in(
        &std::path::Path::new(project_path).join(".claude"),
    ));
    options.setting_sources = Some(Vec::new());
    options.settings = Some(crate::agent::reviewer_settings(&files));
    options.model = Some(bruhs_models::agent::DEFAULT_AGENT_MODEL.to_string());
    options
}

/// Codex options for a review: read-only sandbox, never asks, not saved,
/// and lean (no plugins: a review reads and runs commands, nothing more).
fn codex_review_options(prompt: String, project_path: &str) -> codex_sdk_rs::QueryOptions {
    let mut options = codex_sdk_rs::QueryOptions::new(prompt)
        .cwd(project_path)
        .access(CodexAccess::Sandboxed {
            sandbox: SandboxMode::ReadOnly,
            approval: ApprovalPolicy::Never,
        })
        .session(Session::Ephemeral)
        .skip_git_repo_check(true);
    for key in crate::connections::PRIVATE_ENV {
        options = options.env_remove(key);
    }
    crate::agent::lean_codex(&mut options);
    options
}

async fn review_with_claude(
    prompt: String,
    project_path: &str,
) -> Result<(String, UsageInfo), ReviewError> {
    let mut stream = claude_sdk_rs::query(claude_review_options(prompt, project_path))
        .map_err(|error| ReviewError::Failed(error.to_string()))?;
    let mut failure = None;
    while let Some(event) = stream.next().await {
        match event {
            claude_sdk_rs::QueryEvent::Result {
                result,
                usage,
                is_error,
                errors,
                ..
            } => {
                if is_error && result.trim().is_empty() {
                    let reason = errors.join("; ");
                    return Err(ReviewError::Failed(if reason.is_empty() {
                        "The Claude review failed.".into()
                    } else {
                        reason
                    }));
                }
                return Ok((result, usage.into()));
            }
            claude_sdk_rs::QueryEvent::ProcessFailed(error) => failure = Some(error.to_string()),
            claude_sdk_rs::QueryEvent::Error(message) => failure = Some(message),
            _ => {}
        }
    }
    Err(ReviewError::Failed(failure.unwrap_or_else(|| {
        "The Claude review ended without an answer.".into()
    })))
}

async fn review_with_codex(
    prompt: String,
    project_path: &str,
) -> Result<(String, UsageInfo), ReviewError> {
    let mut stream = codex_sdk_rs::query(codex_review_options(prompt, project_path))
        .map_err(|error| ReviewError::Failed(error.to_string()))?;
    let (mut text, mut usage, mut failure) = (String::new(), UsageInfo::default(), None);
    while let Some(event) = stream.next().await {
        match event {
            // Each agent message is complete; the last one is the answer.
            codex_sdk_rs::QueryEvent::Text(message) => text = message,
            codex_sdk_rs::QueryEvent::Usage(reported) => usage = reported.into(),
            codex_sdk_rs::QueryEvent::Error(message) => failure = Some(message),
            codex_sdk_rs::QueryEvent::ProcessFailed(error) => failure = Some(error.to_string()),
            _ => {}
        }
    }
    match failure {
        Some(message) if text.trim().is_empty() => Err(ReviewError::Failed(message)),
        _ => Ok((text, usage)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_claude_review_is_lean_bounded_and_cannot_ask_for_writes() {
        let options = claude_review_options("p".into(), "/tmp");
        assert!(!options.permission_prompts, "nothing can be approved");
        assert_eq!(
            options.permission_mode,
            Some(claude_sdk_rs::PermissionMode::Default),
            "never bypass"
        );
        assert_eq!(options.max_turns, Some(SECOND_OPINION_MAX_TURNS));
        assert_eq!(options.setting_sources, Some(Vec::new()));
        let settings: serde_json::Value =
            serde_json::from_str(options.settings.as_deref().unwrap()).unwrap();
        let deny = settings["permissions"]["deny"].as_array().unwrap();
        assert!(deny.contains(&"Edit".into()) && deny.contains(&"Write".into()));
        assert!(settings["permissions"].get("allow").is_none());
        assert_eq!(options.resume, None);
    }

    #[test]
    fn a_codex_review_is_read_only_lean_and_not_saved() {
        let options = codex_review_options("p".into(), "/tmp");
        assert_eq!(options.session, Session::Ephemeral);
        assert!(options
            .config
            .iter()
            .any(|c| c.key() == "features.plugins" && c.value() == "false"));
        assert_eq!(
            options.access,
            CodexAccess::Sandboxed {
                sandbox: SandboxMode::ReadOnly,
                approval: ApprovalPolicy::Never,
            }
        );
    }
}
