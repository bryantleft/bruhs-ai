use axum::Json;
use bruhs_core::{meeting_suggestions::suggest, prompt_suggestion::SuggestionError};
use bruhs_models::meeting_suggestions::{MeetingSuggestionReply, MeetingSuggestionRequest};

use crate::error::AppError;

/// One live meeting suggestion round. Stateless: the app sends the newest
/// transcript and its own context, and decides what to show.
pub async fn post_round(
    Json(request): Json<MeetingSuggestionRequest>,
) -> Result<Json<MeetingSuggestionReply>, AppError> {
    suggest(request)
        .await
        .map(Json)
        .map_err(|error| match error {
            SuggestionError::InvalidRequest => {
                AppError::BadRequest("Invalid meeting suggestion request.".into())
            }
            SuggestionError::UnsupportedProvider => AppError::BadRequest(
                "Meeting suggestions run on Claude or Codex. Choose one of their models.".into(),
            ),
            SuggestionError::Busy => {
                AppError::Unavailable("Another meeting suggestion round is still running.".into())
            }
            SuggestionError::UsageReserved => AppError::Unavailable(
                "Suggestions paused to keep the rest of your usage for your own chats.".into(),
            ),
            SuggestionError::TimedOut => {
                AppError::GatewayTimeout("Meeting suggestions took too long.".into())
            }
            error => AppError::bad_gateway("Meeting suggestions are unavailable.", error),
        })
}

#[cfg(test)]
mod tests {
    use bruhs_models::meeting_suggestions::Trigger;

    use super::*;

    #[tokio::test]
    async fn a_round_without_transcript_is_refused_before_a_provider_starts() {
        let result = post_round(Json(MeetingSuggestionRequest {
            model: "claude-haiku-4-5".into(),
            trigger: Trigger::Request,
            meeting_title: "Sync".into(),
            transcript: Vec::new(),
            context: Vec::new(),
            recent: Vec::new(),
        }))
        .await;
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }
}
