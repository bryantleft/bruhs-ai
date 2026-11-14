use crate::error::AppError;
use axum::{
    body::Body,
    extract::{Path, Query, Request},
    http::{header, HeaderMap, HeaderValue, Method},
    response::{IntoResponse, Response},
    Json,
};
use bruhs_core::rich_responses::{export::ExportRequest, CapturedMedia, RichError, RichStore};
use bruhs_models::logic::rich_response::WidgetState;
use serde::Deserialize;
use tower_http::services::ServeFile;

pub(super) fn rich_error(error: RichError) -> AppError {
    match error {
        RichError::Invalid(message) => AppError::BadRequest(message),
        RichError::Forbidden(message) => AppError::Forbidden(message),
        RichError::Io(ref source) if source.kind() == std::io::ErrorKind::NotFound => {
            AppError::NotFound("This saved response is no longer available.".into())
        }
        error => AppError::internal_in("Rich response operation failed", error),
    }
}

#[derive(Deserialize)]
pub struct StateQuery {
    key: String,
}
pub async fn get_state(Query(query): Query<StateQuery>) -> Result<Json<WidgetState>, AppError> {
    tokio::task::spawn_blocking(move || RichStore::open_default()?.load_state(&query.key))
        .await
        .map_err(|e| AppError::internal_in("Widget state reader stopped", e))?
        .map(Json)
        .map_err(rich_error)
}
pub async fn put_state(
    Query(query): Query<StateQuery>,
    Json(state): Json<WidgetState>,
) -> Result<Json<WidgetState>, AppError> {
    tokio::task::spawn_blocking(move || {
        RichStore::open_default()?.save_state(&query.key, &state)?;
        Ok::<_, RichError>(state)
    })
    .await
    .map_err(|e| AppError::internal_in("Widget state writer stopped", e))?
    .map(Json)
    .map_err(rich_error)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureRequest {
    src: String,
    project: Option<String>,
}
pub async fn post_capture(
    Json(request): Json<CaptureRequest>,
) -> Result<Json<CapturedMedia>, AppError> {
    tokio::task::spawn_blocking(move || {
        RichStore::open_default()?.capture(&request.src, request.project.as_deref())
    })
    .await
    .map_err(|e| AppError::internal_in("Media capture worker stopped", e))?
    .map(Json)
    .map_err(rich_error)
}

#[derive(Deserialize)]
pub struct AssetGrant {
    token: String,
}
pub async fn get_asset(
    Path(name): Path<String>,
    Query(grant): Query<AssetGrant>,
    request: Request,
) -> Result<Response, AppError> {
    asset_response(
        RichStore::open_default().map_err(rich_error)?,
        name,
        grant.token,
        request,
    )
    .await
}

async fn asset_response(
    store: RichStore,
    name: String,
    token: String,
    request: Request,
) -> Result<Response, AppError> {
    let path = tokio::task::spawn_blocking(move || store.authorized_asset(&name, &token))
        .await
        .map_err(|e| AppError::internal_in("Asset grant worker stopped", e))?
        .map_err(|_| {
            AppError::Forbidden("This media grant is invalid or no longer available.".into())
        })?;
    let mime = bruhs_core::rich_responses::media_mime(&path);
    let mut response = ServeFile::new(path)
        .try_call(request)
        .await
        .map_err(|e| AppError::internal_in("Asset stream failed", e))?
        .into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    Ok(response)
}

#[cfg(test)]
mod tests;

pub async fn post_export(Json(request): Json<ExportRequest>) -> Result<Response, AppError> {
    let store = RichStore::open_default().map_err(rich_error)?;
    let bundle = bruhs_core::rich_responses::export::build(store, request)
        .await
        .map_err(rich_error)?;
    let length = bundle
        .as_file()
        .metadata()
        .map_err(|e| AppError::internal_in("Could not read bundle length", e))?
        .len();
    let stream = async_stream::try_stream! {
        use tokio::io::AsyncReadExt;
        let mut file=tokio::fs::File::open(bundle.path()).await?;
        // Match the existing ServeFile stream chunk size.
        let mut buffer=vec![0_u8;64*1024];
        loop{let read=file.read(&mut buffer).await?;if read==0{break}yield axum::body::Bytes::copy_from_slice(&buffer[..read]);}
        drop(bundle);
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/zip"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"reply.zip\""),
    );
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&length.to_string())
            .map_err(|e| AppError::internal_in("Invalid bundle length", e))?,
    );
    let stream: std::pin::Pin<
        Box<dyn futures_core::Stream<Item = Result<axum::body::Bytes, std::io::Error>> + Send>,
    > = Box::pin(stream);
    let body = Body::from_stream(stream);
    Ok((headers, body).into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DownloadRequest {
    url: String,
}

pub async fn post_remote_media(
    Json(request): Json<DownloadRequest>,
) -> Result<Json<CapturedMedia>, AppError> {
    let store = RichStore::open_default().map_err(rich_error)?;
    bruhs_core::rich_responses::remote::download(&store, &request.url)
        .await
        .map(Json)
        .map_err(rich_error)
}
/// POST only: an untrusted image URL cannot cause a proxy request.
pub async fn post_download(Json(request): Json<DownloadRequest>) -> Result<Response, AppError> {
    let store = RichStore::open_default().map_err(rich_error)?;
    let name = bruhs_core::rich_responses::remote::download_name(&request.url);
    let asset = bruhs_core::rich_responses::remote::download(&store, &request.url)
        .await
        .map_err(rich_error)?;
    let mime = bruhs_core::rich_responses::media_mime(std::path::Path::new(&asset.src));
    let mut response = ServeFile::new(&asset.src)
        .try_call(
            Request::builder()
                .method(Method::GET)
                .uri("/")
                .body(Body::empty())
                .map_err(|e| AppError::internal_in("Download request failed", e))?,
        )
        .await
        .map_err(|e| AppError::internal_in("Download stream failed", e))?
        .into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{name}\""))
            .map_err(|e| AppError::internal_in("Invalid media download name", e))?,
    );
    Ok(response)
}
