//! `POST /api/agent/uploads` stores one attachment.
//! `GET /api/agent/uploads/open` reads a file for the viewer.
//!
//! `/api/agent/files` stays the `@` search. These routes do not replace it.

use axum::body::Body;
use axum::extract::Query;
use axum::http::{header, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::Json;
use bruhs_core::{open_for_viewer, require_uploaded, write_upload, OpenError, OpenedFile};
use bruhs_models::logic::attachments::{OpenedText, UploadedFile};
use serde::Deserialize;

use crate::error::AppError;

#[derive(Deserialize)]
pub struct UploadQuery {
    pub name: String,
}

#[derive(Deserialize)]
pub struct OpenQuery {
    pub path: String,
    #[serde(default)]
    pub project: Option<String>,
}

/// Store the raw body. The route's body limit is 50 MiB.
pub async fn post_upload(
    Query(query): Query<UploadQuery>,
    body: axum::body::Bytes,
) -> Result<Json<UploadedFile>, AppError> {
    let uploaded = write_upload(&query.name, &body)
        .map_err(|message| AppError::BadRequest(message.to_string()))?;
    Ok(Json(uploaded))
}

/// Read a file for the viewer. Relative paths join `project`. Absolute paths
/// may sit outside that project.
pub async fn get_open(Query(query): Query<OpenQuery>) -> Result<Response, AppError> {
    let project =
        match query.project.as_deref().filter(|path| !path.is_empty()) {
            Some(path) => Some(std::fs::canonicalize(path).map_err(|_| {
                AppError::BadRequest("Open a project to read a relative file.".into())
            })?),
            None => None,
        };
    let opened = open_for_viewer(&query.path, project.as_deref()).map_err(open_error)?;
    if opened.bytes.is_some() {
        return binary_response(opened);
    }
    let text = opened.text.unwrap_or_default();
    Ok(Json(OpenedText {
        name: opened.name,
        path: opened.path,
        kind: opened.kind,
        mime: opened.mime.to_string(),
        truncated: opened.truncated,
        text,
    })
    .into_response())
}

/// Reject a turn that names a path this app did not write.
pub fn reject_foreign_uploads(files: &[UploadedFile]) -> Result<(), AppError> {
    require_uploaded(files).map_err(|message| AppError::BadRequest(message.to_string()))
}

fn open_error(error: OpenError) -> AppError {
    match error {
        OpenError::NotFound => AppError::NotFound(error.message().to_string()),
        _ => AppError::BadRequest(error.message().to_string()),
    }
}

fn binary_response(opened: OpenedFile) -> Result<Response, AppError> {
    let bytes = opened.bytes.unwrap_or_default();
    let mime = HeaderValue::from_static(opened.mime);
    Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .body(Body::from(bytes))
        .map_err(|_| AppError::internal_msg("could not build the file response"))
}
