use std::sync::Arc;

use axum::{extract::State, Json};
use bruhs_core::{AppState, PermissionDecision, PermissionResolveError};
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
        .map_err(|e| match e {
            PermissionResolveError::UnknownRequest => AppError::NotFound(e.to_string()),
            _ => AppError::internal(e),
        })?;

    Ok(Json(ApproveResponse { success: true }))
}
