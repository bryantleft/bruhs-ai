use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Query, State},
    Json,
};
use bruhs_core::{AppState, PermissionDecision, PermissionResolveError};
use bruhs_models::{
    api::{AnswerQuestionRequest, QuestionUpload},
    logic::attachments::{RetireQuestionFiles, UploadedFile},
};
use serde::{Deserialize, Serialize};

use crate::error::AppError;

#[derive(Deserialize)]
pub struct ApproveRequest {
    #[serde(rename = "requestId", alias = "request_id")]
    pub request_id: Option<String>,
    pub approved: Option<bool>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Serialize)]
pub struct ApproveResponse {
    pub success: bool,
}

/// POST /api/agent/approve — answers a tool-permission prompt that a running
/// Claude turn surfaced as a `permission_request` stream event.
pub async fn post_approve(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ApproveRequest>,
) -> Result<Json<ApproveResponse>, AppError> {
    let request_id = body
        .request_id
        .filter(|id| !id.is_empty())
        .ok_or_else(|| AppError::BadRequest("requestId and approved required".into()))?;
    let approved = body
        .approved
        .ok_or_else(|| AppError::BadRequest("requestId and approved required".into()))?;

    tracing::info!(
        request_id = %request_id,
        approved = approved,
        reason = ?body.reason,
        "permission decision"
    );

    let decision = if approved {
        PermissionDecision::Allow
    } else {
        PermissionDecision::Deny {
            reason: body.reason,
        }
    };
    state
        .resolve_permission(&request_id, decision)
        .await
        .map_err(permission_error)?;

    Ok(Json(ApproveResponse { success: true }))
}

pub async fn post_answer_question(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AnswerQuestionRequest>,
) -> Result<Json<ApproveResponse>, AppError> {
    // A dropped HTTP response must not interrupt a committed write to the provider.
    tokio::spawn(async move {
        state
            .answer_question(&body.thread_id, &body.request_id, body.reply)
            .await
    })
    .await
    .map_err(AppError::internal)?
    .map_err(permission_error)?;
    Ok(Json(ApproveResponse { success: true }))
}

pub async fn post_question_file(
    State(state): State<Arc<AppState>>,
    Query(upload): Query<QuestionUpload>,
    body: Bytes,
) -> Result<Json<UploadedFile>, AppError> {
    state
        .upload_question_file(upload, body)
        .await
        .map(Json)
        .map_err(permission_error)
}

pub async fn delete_question_file(
    State(state): State<Arc<AppState>>,
    Query(upload): Query<QuestionUpload>,
) -> Result<Json<ApproveResponse>, AppError> {
    state
        .remove_question_file(&upload)
        .await
        .map_err(permission_error)?;
    Ok(Json(ApproveResponse { success: true }))
}

pub async fn retire_question_files(
    Json(request): Json<RetireQuestionFiles>,
) -> Result<Json<ApproveResponse>, AppError> {
    tokio::task::spawn_blocking(move || bruhs_core::uploads::retire_web_question_uploads(&request))
        .await
        .map_err(AppError::internal)?
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::InvalidInput => AppError::BadRequest(error.to_string()),
            _ => AppError::internal_in("could not reclaim a discarded answer file", error),
        })?;
    Ok(Json(ApproveResponse { success: true }))
}

fn permission_error(error: PermissionResolveError) -> AppError {
    match error {
        PermissionResolveError::UnknownRequest => AppError::NotFound(error.to_string()),
        PermissionResolveError::Question(message) => AppError::BadRequest(message.into()),
        _ => AppError::internal(error),
    }
}
