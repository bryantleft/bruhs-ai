use axum::Json;
use bruhs_core::prompt_suggestion::{suggest, SuggestionError};
use bruhs_models::logic::prompt_suggestion::{SuggestionReply, SuggestionRequest};

use crate::error::AppError;

pub async fn post_suggestion(
    Json(request): Json<SuggestionRequest>,
) -> Result<Json<SuggestionReply>, AppError> {
    suggest(request)
        .await
        .map(Json)
        .map_err(|error| match error {
            SuggestionError::InvalidRequest | SuggestionError::UnsupportedProvider => {
                AppError::BadRequest(error.to_string())
            }
            SuggestionError::Busy => AppError::Unavailable(error.to_string()),
            SuggestionError::TimedOut => AppError::GatewayTimeout(error.to_string()),
            error => AppError::bad_gateway(
                "Prompt suggestion unavailable. Your draft is unchanged.",
                error,
            ),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn prompt_suggestion_rejects_invalid_context_before_starting_a_provider() {
        let result = post_suggestion(Json(SuggestionRequest {
            model: bruhs_models::agent::DEFAULT_AGENT_MODEL.into(),
            messages: Vec::new(),
        }))
        .await;
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }
}
