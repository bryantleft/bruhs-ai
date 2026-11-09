//! GET /centroids/{meeting_id} — returns the cluster centroids written by
//! subdiarize, wrapped in a CentroidsFile that also carries the linking
//! settings active when the recording was made.
//! Returns a default CentroidsFile (empty centroids) when the file is absent.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use bruhs_models::CentroidsFile;
use std::sync::Arc;

use crate::state::CaptureState;

pub async fn list_centroids(
    State(state): State<Arc<CaptureState>>,
    Path(meeting_id): Path<String>,
) -> Result<Json<CentroidsFile>, (StatusCode, String)> {
    crate::paths::validate_meeting_id(&meeting_id)
        .map_err(|error| (StatusCode::BAD_REQUEST, error.to_string()))?;
    let path = state
        .data
        .centroids_path(&meeting_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("path: {e}")))?;
    let text = match tokio::fs::read_to_string(&path).await {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Json(CentroidsFile::default()))
        }
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("read: {e}"))),
    };
    let file: CentroidsFile = serde_json::from_str(&text)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("parse: {e}")))?;
    Ok(Json(file))
}
