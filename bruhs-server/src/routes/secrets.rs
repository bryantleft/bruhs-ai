use crate::error::AppError;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use bruhs_core::{secrets::SecretError, AppState};
use bruhs_models::secrets::SecretSubmission;
use bruhs_models::secrets::{CredentialReplacement, SavedCredential};
use serde::Serialize;
use std::sync::Arc;

fn management_error(error: SecretError) -> AppError {
    match error {
        SecretError::NotSaved => {
            AppError::NotFound("Saved credential not found. Refresh the list.".into())
        }
        SecretError::Storage | SecretError::Catalog => AppError::Unavailable(error.to_string()),
        _ => AppError::BadRequest(error.to_string()),
    }
}

pub async fn list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<SavedCredential>>, AppError> {
    state
        .secrets()
        .list_credentials()
        .await
        .map(Json)
        .map_err(management_error)
}

pub async fn replace(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Result<Json<CredentialReplacement>, axum::extract::rejection::JsonRejection>,
) -> Result<StatusCode, AppError> {
    let Json(body) =
        body.map_err(|_| AppError::BadRequest("Invalid credential replacement.".into()))?;
    state
        .secrets()
        .replace_credential(&id, body.value)
        .await
        .map_err(management_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    state
        .secrets()
        .delete_credential(&id)
        .await
        .map_err(management_error)?;
    Ok(StatusCode::NO_CONTENT)
}

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
    async fn management_lists_metadata_and_never_returns_values_after_updates() {
        let store = bruhs_core::secrets::SecretStore::in_memory();
        store
            .save(
                "/synthetic/project",
                "Service key",
                String::from("private-original").into(),
            )
            .await
            .unwrap();
        let state = Arc::new(AppState::builder().secret_store(store).build());
        let Json(entries) = list(State(state.clone())).await.unwrap();
        let id = entries[0].id.clone();
        assert!(!serde_json::to_string(&entries)
            .unwrap()
            .contains("private-original"));
        let code = replace(
            State(state.clone()),
            Path(id.clone()),
            Ok(Json(CredentialReplacement {
                value: "private-replacement".into(),
            })),
        )
        .await
        .unwrap();
        assert_eq!(code, StatusCode::NO_CONTENT);
        let Json(entries) = list(State(state.clone())).await.unwrap();
        assert!(!serde_json::to_string(&entries)
            .unwrap()
            .contains("private-replacement"));
        assert_eq!(
            delete(State(state.clone()), Path(id)).await.unwrap(),
            StatusCode::NO_CONTENT
        );
        assert!(list(State(state)).await.unwrap().0.is_empty());
    }

    #[tokio::test]
    async fn malformed_replacements_do_not_echo_private_values() {
        let state = Arc::new(
            AppState::builder()
                .secret_store(bruhs_core::secrets::SecretStore::in_memory())
                .build(),
        );
        let request = Request::builder()
            .header("content-type", "application/json")
            .body(Body::from(r#"{"value":{"private-canary":true}}"#))
            .unwrap();
        let body = Json::<CredentialReplacement>::from_request(request, &()).await;
        let error = replace(State(state), Path("unknown".into()), body)
            .await
            .err()
            .unwrap();
        assert!(!error.to_string().contains("private-canary"));
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    }

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
