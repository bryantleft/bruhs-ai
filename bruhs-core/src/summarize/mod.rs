//! Meeting summarization. `summarize_segments(segments, model)` runs the
//! configured LLM with a structured-output prompt and returns a
//! `MeetingSummary` ready to persist on the Meeting record.

pub mod parser;
pub mod prompt;

use bruhs_models::{MeetingSummary, Speaker, TranscriptSegment};
use chrono::Utc;
use claude_sdk_rs::{query, QueryEvent, Tools};
use futures_core::Stream;
use tokio_stream::StreamExt;

/// Longest summary reply accepted from the CLI, in bytes.
pub const MAX_RESULT_BYTES: usize = 1024 * 1024;

/// How long one summary may take.
const SUMMARY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SummarizeError {
    #[error("transcript is empty — nothing to summarize")]
    EmptyTranscript,
    /// The private working directory for the CLI could not be prepared.
    #[error("could not prepare a working directory for the summary: {0}")]
    Workspace(#[source] std::io::Error),
    #[error("claude sdk: {0}")]
    Sdk(#[from] claude_sdk_rs::Error),
    /// The claude CLI exited unsuccessfully or could not be talked to.
    #[error("claude cli: {0}")]
    CliFailed(claude_sdk_rs::ProcessFailure),
    /// The provider reported an error (an `error` event or an error
    /// result); carries its text.
    #[error("claude: {0}")]
    Provider(String),
    /// The reply exceeded [`MAX_RESULT_BYTES`].
    #[error("summary reply is larger than {MAX_RESULT_BYTES} bytes")]
    ResultTooLarge,
    #[error("claude returned no text")]
    NoResponse,
    #[error("summary generation timed out")]
    TimedOut,
    #[error("parse: {0}")]
    Parse(#[from] parser::ParseError),
}

/// Default model id used for summarization. Phase 5 hardcodes this; a
/// per-user selector is Phase 5c work.
pub const DEFAULT_SUMMARIZE_MODEL: &str = "claude-sonnet-4-5";

/// Summarize the transcript via claude-sdk-rs. Returns a fully populated
/// `MeetingSummary` with `generated_at` = current wall clock, alongside
/// raw per-person observations parsed from the response (may be empty when
/// the model omits the field or the transcript has no clear per-person signals).
///
/// When `extract_observations` is false the per_person_observations section is
/// omitted from the prompt entirely — no Claude tokens are spent on it.
pub async fn summarize_segments(
    segments: &[TranscriptSegment],
    speakers: &[Speaker],
    model: &str,
    extract_observations: bool,
) -> Result<(MeetingSummary, Vec<parser::PersonObservationRaw>), SummarizeError> {
    if segments.is_empty() {
        return Err(SummarizeError::EmptyTranscript);
    }
    let transcript = prompt::format_labeled_transcript(segments, speakers);
    let user_prompt = prompt::build_prompt(&transcript, extract_observations);

    // The CLI runs in a fresh, empty directory: with no tools and no project
    // files around, nothing the transcript says can make it read or touch
    // the user's files.
    let workspace = tokio::task::spawn_blocking(tempfile::tempdir)
        .await
        .map_err(|e| SummarizeError::Workspace(std::io::Error::other(e)))?
        .map_err(SummarizeError::Workspace)?;

    let options = summary_options(&user_prompt, model, workspace.path());
    let reply = match query(options) {
        Ok(stream) => {
            read_reply(stream, tokio::time::Instant::now() + SUMMARY_TIMEOUT).await
            // The stream (and with it the CLI) is gone before the
            // directory is removed.
        }
        Err(e) => Err(e.into()),
    };
    remove_workspace(workspace);
    let reply = reply?;

    let generated_at = Utc::now().timestamp_millis();
    let (summary, raw_obs) = parser::parse_summary(&reply, model.to_string(), generated_at)?;
    Ok((summary, raw_obs))
}

/// Claude options for one summary run in `cwd`. Tool-free: no tools and no
/// inherited MCP servers, so the reply is a single turn without a
/// `--max-turns` cap. No partial deltas either: only the final result is
/// read.
fn summary_options(
    prompt: &str,
    model: &str,
    cwd: &std::path::Path,
) -> claude_sdk_rs::QueryOptions {
    crate::agent::claude_query_options_from_env(prompt)
        .cwd(cwd)
        .tools(Tools::None)
        .model(model)
        .include_partial_messages(false)
}

/// Delete the CLI's scratch directory off the async workers.
fn remove_workspace(workspace: tempfile::TempDir) {
    tokio::task::spawn_blocking(move || {
        if let Err(e) = workspace.close() {
            tracing::warn!("could not remove the summary working directory: {e}");
        }
    });
}

/// The final reply text from the CLI's events. Only the `result` is read,
/// never `Text`: with partial messages off no deltas arrive today, and
/// reading both would double the reply if that ever changed.
async fn read_reply<S>(
    mut stream: S,
    deadline: tokio::time::Instant,
) -> Result<String, SummarizeError>
where
    S: Stream<Item = QueryEvent> + Unpin,
{
    while let Some(event) = tokio::time::timeout_at(deadline, stream.next())
        .await
        .map_err(|_| SummarizeError::TimedOut)?
    {
        match event {
            QueryEvent::Error(message) => return Err(SummarizeError::Provider(message)),
            QueryEvent::ProcessFailed(failure) => return Err(SummarizeError::CliFailed(failure)),
            QueryEvent::Result {
                result,
                is_error: true,
                errors,
                ..
            } => {
                // Error results carry their reason in `errors`.
                let message = if result.is_empty() {
                    errors.join("\n")
                } else {
                    result
                };
                return Err(SummarizeError::Provider(message));
            }
            QueryEvent::Result { result, .. } if result.len() > MAX_RESULT_BYTES => {
                return Err(SummarizeError::ResultTooLarge);
            }
            QueryEvent::Result { result, .. } if result.trim().is_empty() => {
                return Err(SummarizeError::NoResponse);
            }
            QueryEvent::Result { result, .. } => return Ok(result),
            _ => {}
        }
    }
    Err(SummarizeError::NoResponse)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(text: &str, is_error: bool, errors: &[&str]) -> QueryEvent {
        QueryEvent::Result {
            result: text.to_string(),
            session_id: "s".into(),
            duration_ms: 1,
            usage: Default::default(),
            is_error,
            errors: errors.iter().map(|e| e.to_string()).collect(),
        }
    }

    async fn reply(events: Vec<QueryEvent>) -> Result<String, SummarizeError> {
        read_reply(
            tokio_stream::iter(events),
            tokio::time::Instant::now() + SUMMARY_TIMEOUT,
        )
        .await
    }

    #[tokio::test]
    async fn the_result_text_is_the_reply() {
        let text = reply(vec![
            QueryEvent::Text("ignored delta".into()),
            result("{\"tldr\":\"x\"}", false, &[]),
        ])
        .await
        .unwrap();
        assert_eq!(text, "{\"tldr\":\"x\"}");
    }

    /// Provider failures used to be reported as "no response", hiding why.
    #[tokio::test]
    async fn provider_errors_keep_their_text() {
        let err = reply(vec![QueryEvent::Error("rate limited".into())])
            .await
            .unwrap_err();
        assert!(matches!(&err, SummarizeError::Provider(m) if m == "rate limited"));
        let err = reply(vec![result("", true, &["max turns", "again"])])
            .await
            .unwrap_err();
        assert!(matches!(&err, SummarizeError::Provider(m) if m == "max turns\nagain"));
        let err = reply(vec![result("bad things", true, &[])])
            .await
            .unwrap_err();
        assert!(matches!(&err, SummarizeError::Provider(m) if m == "bad things"));
    }

    #[tokio::test]
    async fn oversized_replies_are_their_own_error() {
        let big = "x".repeat(MAX_RESULT_BYTES + 1);
        let err = reply(vec![result(&big, false, &[])]).await.unwrap_err();
        assert!(matches!(err, SummarizeError::ResultTooLarge));
        let at_limit = "x".repeat(MAX_RESULT_BYTES);
        assert_eq!(
            reply(vec![result(&at_limit, false, &[])])
                .await
                .unwrap()
                .len(),
            MAX_RESULT_BYTES
        );
    }

    #[tokio::test]
    async fn empty_or_missing_results_are_no_response() {
        assert!(matches!(
            reply(vec![result("  ", false, &[])]).await,
            Err(SummarizeError::NoResponse)
        ));
        assert!(matches!(
            reply(vec![]).await,
            Err(SummarizeError::NoResponse)
        ));
    }

    #[test]
    fn the_cli_runs_tool_free_in_the_given_directory() {
        let dir = tempfile::tempdir().unwrap();
        let options = summary_options("p", DEFAULT_SUMMARIZE_MODEL, dir.path());
        assert_eq!(options.cwd, dir.path());
        assert_eq!(options.tools, Tools::None);
        assert!(!options.include_partial_messages);
    }

    #[tokio::test(start_paused = true)]
    async fn a_silent_cli_times_out() {
        let err = read_reply(
            tokio_stream::pending::<QueryEvent>(),
            tokio::time::Instant::now() + SUMMARY_TIMEOUT,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, SummarizeError::TimedOut));
    }

    #[tokio::test]
    async fn summarize_segments_rejects_empty() {
        let err = summarize_segments(&[], &[], DEFAULT_SUMMARIZE_MODEL, true)
            .await
            .unwrap_err();
        assert!(matches!(err, SummarizeError::EmptyTranscript));
    }
}
