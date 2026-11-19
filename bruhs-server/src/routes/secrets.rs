use crate::error::AppError;
use axum::{extract::State, Json};
use bruhs_core::{secrets::SecretError, AppState};
use bruhs_models::secrets::SecretSubmission;
use serde::Serialize;
use std::sync::Arc;

#[derive(Serialize)]
pub struct Resolution {
    resolved: bool,
}

/// Private UI channel. Never log the body or forward it to a provider.
pub async fn submit(
    State(state): State<Arc<AppState>>,
    body: Result<Json<SecretSubmission>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Resolution>, AppError> {
    // JSON rejection text can quote an invalid value. Keep it out of responses/logs.
    let Json(body) =
        body.map_err(|_| AppError::BadRequest("Invalid credential submission.".into()))?;
    let resolved = state
        .secrets()
        .resolve(&body.thread_id, &body.request_id, body.decision)
        .await
        .map_err(|error| match error {
            SecretError::NotPending => AppError::NotFound(error.to_string()),
            SecretError::Busy => AppError::Conflict(error.to_string()),
            SecretError::Storage | SecretError::Catalog => AppError::Unavailable(error.to_string()),
            _ => AppError::BadRequest(error.to_string()),
        })?;
    Ok(Json(Resolution { resolved }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, extract::FromRequest, http::Request};
    use bruhs_models::ids::ThreadId;

    #[tokio::test]
    async fn malformed_private_values_are_never_reflected_in_errors() {
        let request = Request::builder().header("content-type", "application/json").body(Body::from(format!(r#"{{"thread_id":"{}","request_id":"private","decision":{{"action":"provide","save":"private-canary","value":"private-canary"}}}}"#, ThreadId::new_v4()))).unwrap();
        let body = Json::<SecretSubmission>::from_request(request, &()).await;
        let error = submit(State(Arc::new(AppState::new())), body)
            .await
            .err()
            .unwrap();
        assert_eq!(error.status(), axum::http::StatusCode::BAD_REQUEST);
        assert!(!error.to_string().contains("private-canary"));
    }

    #[tokio::test]
    async fn stale_private_submissions_never_reach_storage_or_echo_the_value() {
        let submission = SecretSubmission {
            thread_id: ThreadId::new_v4(),
            request_id: "stale".into(),
            decision: bruhs_models::secrets::SecretDecision::Provide {
                value: "private-canary".into(),
                save: true,
            },
        };
        let error = submit(State(Arc::new(AppState::new())), Ok(Json(submission)))
            .await
            .err()
            .unwrap();
        assert_eq!(error.status(), axum::http::StatusCode::NOT_FOUND);
        assert!(!error.to_string().contains("private-canary"));
    }
}
