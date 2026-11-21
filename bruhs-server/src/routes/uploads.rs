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
    let uploaded = tokio::task::spawn_blocking(move || write_upload(&query.name, &body))
        .await
        .map_err(|error| AppError::internal_in("attachment writer failed", error))?
        .map_err(|message| AppError::BadRequest(message.to_string()))?;
    Ok(Json(uploaded))
}

/// Read a file for the viewer. Relative paths join `project`. Absolute paths
/// may sit outside that project.
pub async fn get_open(Query(query): Query<OpenQuery>) -> Result<Response, AppError> {
    let opened = tokio::task::spawn_blocking(move || {
        let project = match query.project.as_deref().filter(|path| !path.is_empty()) {
            Some(path) => Some(std::fs::canonicalize(path).map_err(|_| {
                AppError::BadRequest("Open a project to read a relative file.".into())
            })?),
            None => None,
        };
        open_for_viewer(&query.path, project.as_deref()).map_err(open_error)
    })
    .await
    .map_err(|error| AppError::internal_in("file viewer failed", error))??;
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
pub async fn reject_foreign_uploads(files: &[UploadedFile]) -> Result<(), AppError> {
    if files.is_empty() {
        return Ok(());
    }
    let files = files.to_vec();
    tokio::task::spawn_blocking(move || require_uploaded(&files))
        .await
        .map_err(|error| AppError::internal_in("attachment validation failed", error))?
        .map_err(|message| AppError::BadRequest(message.to_string()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{to_bytes, Bytes},
        http::StatusCode,
    };

    #[tokio::test(flavor = "current_thread")]
    async fn uploaded_files_round_trip_and_foreign_paths_stay_rejected() {
        let Json(file) = post_upload(
            Query(UploadQuery {
                name: "performance.txt".into(),
            }),
            Bytes::from_static(b"hello from an attachment"),
        )
        .await
        .unwrap();
        reject_foreign_uploads(std::slice::from_ref(&file))
            .await
            .unwrap();
        let response = get_open(Query(OpenQuery {
            path: file.path.clone(),
            project: None,
        }))
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(text["text"], "hello from an attachment");
        assert_eq!(text["truncated"], false);
        tokio::fs::remove_file(&file.path).await.unwrap();

        let foreign = UploadedFile {
            name: "foreign.txt".into(),
            path: "/not/an/upload".into(),
        };
        assert!(matches!(
            reject_foreign_uploads(&[foreign]).await,
            Err(AppError::BadRequest(_))
        ));
        reject_foreign_uploads(&[]).await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn viewer_keeps_relative_paths_and_missing_project_errors() {
        let directory = tempfile::tempdir().unwrap();
        tokio::fs::write(directory.path().join("file.txt"), b"project file")
            .await
            .unwrap();
        let response = get_open(Query(OpenQuery {
            path: "file.txt".into(),
            project: Some(directory.path().to_string_lossy().into_owned()),
        }))
        .await
        .unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(text["text"], "project file");
        assert!(matches!(
            get_open(Query(OpenQuery {
                path: "file.txt".into(),
                project: None
            }))
            .await,
            Err(AppError::BadRequest(_))
        ));
    }
}

async fn capture_store<T: Send + 'static>(
    operation: impl FnOnce() -> std::io::Result<T> + Send + 'static,
) -> Result<Json<T>, AppError> {
    let result = tokio::task::spawn_blocking(operation)
        .await
        .map_err(AppError::internal)?;
    result.map(Json).map_err(|err| match err.kind() {
        std::io::ErrorKind::InvalidInput => AppError::BadRequest(err.to_string()),
        _ => AppError::internal_in("browser capture retention", err),
    })
}
pub async fn capture_retention() -> Result<Json<u16>, AppError> {
    capture_store(bruhs_core::uploads::browser_captures::days).await
}
pub async fn set_capture_retention(Json(days): Json<u16>) -> Result<Json<u16>, AppError> {
    capture_store(move || bruhs_core::uploads::browser_captures::set_days(days)).await
}
pub async fn prune_captures() -> Result<Json<usize>, AppError> {
    capture_store(bruhs_core::uploads::browser_captures::prune).await
}

pub fn schedule_capture_cleanup() {
    tokio::spawn(async {
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(24 * 60 * 60));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            timer.tick().await;
            if let Err(error) = prune_captures().await {
                tracing::warn!(%error, "Browser capture cleanup could not finish");
            }
        }
    });
}
