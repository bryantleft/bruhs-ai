use crate::error::AppError;
use axum::{http::header::CACHE_CONTROL, response::IntoResponse, Json};
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fingerprint {
    project_path: String,
    path: String,
}
pub async fn fingerprint(Json(body): Json<Fingerprint>) -> Result<impl IntoResponse, AppError> {
    let digest = tokio::task::spawn_blocking(move || {
        bruhs_core::whiteboard::source_fingerprint(
            std::path::Path::new(&body.project_path),
            &body.path,
        )
    })
    .await
    .map_err(|e| AppError::Unavailable(e.to_string()))?
    .map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok((
        [(CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({"sha256":digest})),
    ))
}
