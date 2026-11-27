//! Owner-only local SSH profile controls; not mounted on paired task listeners.
use crate::error::AppError;
use axum::{
    extract::State,
    http::header,
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use bruhs_core::remote_hosts::{RemoteHostError, RemoteHostManager};
use bruhs_models::remote_hosts::RemoteHostCommand;

pub fn router(manager: RemoteHostManager) -> Router {
    let router = Router::new()
        .route("/api/remote-hosts", get(snapshot))
        .route("/api/remote-hosts/commands", post(command))
        .route("/api/remote-hosts/ui", get(ui));
    #[cfg(unix)]
    let router = router.route("/api/remote-hosts/previews", post(forward_preview));
    router.with_state(manager)
}

#[cfg(unix)]
async fn forward_preview(
    State(manager): State<RemoteHostManager>,
    Json(request): Json<bruhs_models::remote_hosts::ForwardPreview>,
) -> Result<impl IntoResponse, AppError> {
    let forwarded = tokio::spawn(async move { manager.forward_preview(request).await })
        .await
        .map_err(AppError::internal)?
        .map_err(map_error)?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(forwarded)))
}
fn map_error(error: RemoteHostError) -> AppError {
    match error {
        RemoteHostError::Invalid(message) => AppError::BadRequest(message),
        RemoteHostError::Conflict(message) => AppError::Conflict(message),
        RemoteHostError::NotFound => AppError::NotFound(error.to_string()),
        error @ RemoteHostError::Storage(_) => AppError::internal(error),
    }
}
async fn snapshot(State(manager): State<RemoteHostManager>) -> impl IntoResponse {
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(manager.snapshot().await),
    )
}
async fn command(
    State(manager): State<RemoteHostManager>,
    Json(command): Json<RemoteHostCommand>,
) -> Result<impl IntoResponse, AppError> {
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(manager.command(command).await.map_err(map_error)?),
    ))
}
async fn ui() -> impl IntoResponse {
    (
        [(header::CACHE_CONTROL, "no-store")],
        Html(include_str!("remote_hosts.html")),
    )
}
