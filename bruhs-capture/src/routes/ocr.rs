//! GET /ocr/{meeting_id} — returns the OcrObservation list written by
//! OcrWorker after recording stops. Returns an empty array when the file is
//! absent (video OCR was disabled or not yet complete).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use std::sync::Arc;

use crate::state::CaptureState;
use bruhs_models::OcrObservation;

pub async fn list_ocr(
    State(state): State<Arc<CaptureState>>,
    Path(meeting_id): Path<String>,
) -> Result<Json<Vec<OcrObservation>>, (StatusCode, String)> {
    crate::paths::validate_meeting_id(&meeting_id)
        .map_err(|error| (StatusCode::BAD_REQUEST, error.to_string()))?;
    let path = state
        .data
        .ocr_path(&meeting_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("path: {e}")))?;
    let text = match tokio::fs::read_to_string(&path).await {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Json(Vec::new())),
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("read: {e}"))),
    };
    let observations: Vec<OcrObservation> = serde_json::from_str(&text)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("parse: {e}")))?;
    Ok(Json(observations))
}
