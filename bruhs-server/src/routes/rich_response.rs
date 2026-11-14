//! Inline media is confined to the current project or uploaded attachments.
//! Interactive HTML runs in a separate opaque-origin document.

use std::path::PathBuf;

use axum::{
    extract::{Query, Request},
    http::{header, HeaderValue},
    response::{Html, IntoResponse, Response},
};
use bruhs_models::logic::rich_response::{INTERACTIVE_HOST, MEDIA_HOST};
use serde::Deserialize;
use tower_http::services::ServeFile;

use crate::error::AppError;

mod lifecycle;
pub use lifecycle::*;

pub const HOST_CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src 'self' http: https: data: blob:; media-src 'self' http: https: data: blob:; frame-src about:; connect-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";
const MEDIA_HOST_CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src 'self' http: https: data: blob:; media-src 'self' http: https: data: blob:; frame-src 'self' http: https:; connect-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";

pub async fn get_host() -> Response {
    host_response(INTERACTIVE_HOST, HOST_CSP)
}

/// Controlled media views need HTTP(S) resources. The fragment is typed
/// media data, never assistant-supplied HTML or JavaScript.
pub async fn get_media_host() -> Response {
    host_response(MEDIA_HOST, MEDIA_HOST_CSP)
}

fn host_response(html: &'static str, policy: &'static str) -> Response {
    let mut response = Html(html).into_response();
    response.headers_mut().insert(
        header::HeaderName::from_static("x-bruhs-rich-response"),
        HeaderValue::from_static("1"),
    );
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(policy),
    );
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response.headers_mut().insert(
        header::HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("camera=(), microphone=(), geolocation=(), clipboard-read=(), clipboard-write=(), display-capture=(), autoplay=(), payment=(), usb=()"),
    );
    response
}

#[derive(Deserialize)]
pub struct MediaQuery {
    path: String,
    project: Option<String>,
    #[serde(default)]
    download: bool,
}

pub async fn get_media(
    Query(query): Query<MediaQuery>,
    request: Request,
) -> Result<Response, AppError> {
    let download = query.download;
    let path =
        tokio::task::spawn_blocking(move || resolve_media(&query.path, query.project.as_deref()))
            .await
            .map_err(|error| AppError::internal_in("Could not resolve inline media", error))??;
    let mime = bruhs_core::rich_responses::media_mime(&path);
    let download_name = download.then(|| {
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    });
    // ServeFile streams bytes and implements HTTP Range/HEAD, so seeking
    // does not buffer an entire recording or duplicate it for each viewer.
    let mut response = ServeFile::new(&path)
        .try_call(request)
        .await
        .map_err(|error| AppError::internal_in("Could not stream inline media", error))?
        .into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    // SVG files must not become scripts if opened as a document directly.
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    if let Some(name) = download_name {
        response.headers_mut().insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_str(&format!("attachment; filename=\"{name}\""))
                .map_err(|error| AppError::internal_in("Invalid media download name", error))?,
        );
    }
    Ok(response)
}

fn resolve_media(raw: &str, project: Option<&str>) -> Result<PathBuf, AppError> {
    bruhs_core::rich_responses::RichStore::open_default()
        .and_then(|store| store.capture(raw, project))
        .map(|asset| PathBuf::from(asset.src))
        .map_err(lifecycle::rich_error)
}

#[cfg(test)]
mod tests;
