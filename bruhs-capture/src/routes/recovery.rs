//! Bounded local recovery reads and token-fenced acknowledgments.
use crate::{recovery::RecoveryError, state::CaptureState};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use bruhs_models::{CaptureRecoveryPage, CaptureRecoveryReceipt};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Default, Deserialize)]
pub struct Cursor {
    #[serde(default)]
    after: i64,
}

fn response_error(error: RecoveryError) -> (StatusCode, &'static str) {
    match error {
        RecoveryError::Invalid => (StatusCode::BAD_REQUEST, "Invalid recovery request."),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            "Capture recovery storage is unavailable; retry later.",
        ),
    }
}

pub async fn pending(
    State(state): State<Arc<CaptureState>>,
    Query(cursor): Query<Cursor>,
) -> Result<Json<CaptureRecoveryPage>, (StatusCode, &'static str)> {
    let receipts = state
        .recovery
        .page(cursor.after)
        .await
        .map_err(response_error)?;
    Ok(Json(CaptureRecoveryPage { receipts }))
}

pub async fn acknowledge(
    State(state): State<Arc<CaptureState>>,
    Json(receipts): Json<Vec<CaptureRecoveryReceipt>>,
) -> Result<StatusCode, (StatusCode, &'static str)> {
    state
        .recovery
        .acknowledge(receipts)
        .await
        .map_err(response_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn failures(
    State(state): State<Arc<CaptureState>>,
    Query(cursor): Query<Cursor>,
) -> Result<Json<bruhs_models::CaptureFailurePage>, (StatusCode, &'static str)> {
    let receipts = state
        .recovery
        .failure_page(cursor.after)
        .await
        .map_err(response_error)?;
    Ok(Json(bruhs_models::CaptureFailurePage { receipts }))
}

pub async fn acknowledge_failures(
    State(state): State<Arc<CaptureState>>,
    Json(receipts): Json<Vec<bruhs_models::CaptureFailureReceipt>>,
) -> Result<StatusCode, (StatusCode, &'static str)> {
    state
        .recovery
        .acknowledge_failures(receipts)
        .await
        .map_err(response_error)?;
    Ok(StatusCode::NO_CONTENT)
}
