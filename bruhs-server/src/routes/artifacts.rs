//! Publication is local filesystem work; only the UI reports actual previews.

use axum::{
    extract::{Path, Query},
    http::{header, HeaderName, HeaderValue},
    response::{Html, IntoResponse, Response},
    Json,
};
use bruhs_core::artifacts::{self, Artifact, ArtifactDiagnostics, ArtifactError, PreviewState};
use serde::Deserialize;

use crate::error::AppError;

pub async fn get_artifact(Path(id): Path<String>) -> Result<Json<Artifact>, AppError> {
    blocking(move || artifacts::load(&id)).await.map(Json)
}

pub async fn get_diagnostics(
    Path(id): Path<String>,
) -> Result<Json<ArtifactDiagnostics>, AppError> {
    blocking(move || artifacts::load_diagnostics(&id))
        .await
        .map(Json)
}

#[derive(Deserialize)]
pub struct PreviewQuery {
    #[serde(default)]
    nonce: String,
}

pub async fn get_preview(
    Path(id): Path<String>,
    Query(query): Query<PreviewQuery>,
) -> Result<Response, AppError> {
    let artifact = blocking(move || artifacts::load(&id)).await?;
    Ok(preview_response(&artifact.html, &query.nonce))
}

fn preview_response(html: &str, nonce: &str) -> Response {
    let script = bruhs_models::logic::rich_response::interactive_host_script(html, nonce);
    let mut response = Html(format!("<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"referrer\" content=\"no-referrer\"><style>html,body{{margin:0;height:100%;overflow:hidden}}iframe{{width:100%;height:100%;border:0}}</style></head><body><script>{script}</script></body></html>")).into_response();
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(super::rich_response::HOST_CSP),
    );
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response.headers_mut().insert(
        HeaderName::from_static("x-bruhs-rich-response"),
        HeaderValue::from_static("1"),
    );
    response.headers_mut().insert(HeaderName::from_static("permissions-policy"), HeaderValue::from_static("camera=(), microphone=(), geolocation=(), clipboard-read=(), clipboard-write=(), display-capture=(), autoplay=(), payment=(), usb=()"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn preview_source_cannot_break_out_of_the_trusted_host_script() {
        let response = preview_response(
            "</script><script>window.hostCompromised=true</script>",
            "</script><script>nonce()</script>",
        );
        assert_eq!(
            response.headers()[header::CONTENT_SECURITY_POLICY],
            super::super::rich_response::HOST_CSP
        );
        assert_eq!(response.headers()["x-bruhs-rich-response"], "1");
        assert!(response.headers()["permissions-policy"]
            .to_str()
            .unwrap()
            .contains("microphone=()"));
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let html = std::str::from_utf8(&body).unwrap();
        assert_eq!(html.matches("</script>").count(), 1);
        assert!(!html.contains("</script><script>window.hostCompromised"));
        assert!(html.contains("window.bruhsCreateWidget"));
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewReport {
    state: PreviewState,
    #[serde(default)]
    errors: Vec<String>,
}

pub async fn post_diagnostics(
    Path(id): Path<String>,
    Json(report): Json<PreviewReport>,
) -> Result<Json<ArtifactDiagnostics>, AppError> {
    blocking(move || artifacts::record_diagnostics(&id, report.state, report.errors))
        .await
        .map(Json)
}

async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, ArtifactError> + Send + 'static,
) -> Result<T, AppError> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| AppError::internal_in("Artifact operation failed", error))?
        .map_err(|error| match &error {
            ArtifactError::Invalid(message) => AppError::BadRequest(message.clone()),
            ArtifactError::Io(error) if error.kind() == std::io::ErrorKind::NotFound => {
                AppError::NotFound("This published artifact is no longer available.".into())
            }
            _ => AppError::internal_in("Artifact operation failed", error),
        })
}
