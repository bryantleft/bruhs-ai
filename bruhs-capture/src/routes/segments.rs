//! GET /segments/:meeting_id — returns the persisted transcript segments for
//! a meeting by reading the post-diarize JSONL from disk.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use bruhs_models::TranscriptSegment;
use serde::Serialize;
use std::sync::Arc;

use crate::state::CaptureState;

#[derive(Debug, Serialize)]
pub struct SegmentsResponse {
    pub segments: Vec<TranscriptSegment>,
}

/// GET /segments/:meeting_id
pub async fn list_segments(
    State(state): State<Arc<CaptureState>>,
    Path(meeting_id): Path<String>,
) -> Result<Json<SegmentsResponse>, (StatusCode, String)> {
    crate::paths::validate_meeting_id(&meeting_id)
        .map_err(|error| (StatusCode::BAD_REQUEST, error.to_string()))?;
    let segments = tokio::task::spawn_blocking(move || {
        let path = state.data.segments_jsonl_path(&meeting_id)?;
        crate::transcribe::segments_log::load_snapshot(&path)
    })
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Transcript read could not finish.".into(),
        )
    })?
    .map_err(|_| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Transcript snapshot is unavailable or incomplete; original data was retained.".into(),
        )
    })?;
    Ok(Json(SegmentsResponse { segments }))
}
