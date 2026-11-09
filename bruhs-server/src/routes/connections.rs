//! UI-only account connection controls. Responses never contain OAuth tokens.
use crate::error::AppError;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use bruhs_core::{connections::ConnectionError, AppState};
use bruhs_models::connections::*;
use std::sync::Arc;

fn error(error: ConnectionError) -> AppError {
    match error {
        ConnectionError::Cancelled => AppError::NotFound(error.to_string()),
        ConnectionError::Storage | ConnectionError::Network => {
            AppError::Unavailable(error.to_string())
        }
        _ => AppError::BadRequest(error.to_string()),
    }
}

pub async fn list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<ServiceConnection>>, AppError> {
    state
        .secrets()
        .connections()
        .list()
        .await
        .map(Json)
        .map_err(error)
}

pub async fn get(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<ServiceProvider>,
) -> Result<Json<ServiceConnection>, AppError> {
    state
        .secrets()
        .connections()
        .connection(provider)
        .await
        .map(Json)
        .map_err(error)
}

pub async fn start(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<ServiceProvider>,
    body: Result<Json<ConnectionLogin>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<ConnectionStart>, AppError> {
    let Json(login) =
        body.map_err(|_| AppError::BadRequest("Invalid connection request.".into()))?;
    match (login.thread_id, login.request_id) {
        (None, None) => state
            .secrets()
            .connections()
            .start(provider)
            .await
            .map(Json)
            .map_err(error),
        (Some(thread), Some(request)) => {
            // The pending request selects its provider; an unrelated form cannot retarget it.
            state
                .secrets()
                .connect_request(&thread, &request, provider)
                .await
                .map(Json)
                .map_err(|e| AppError::BadRequest(e.to_string()))
        }
        _ => Err(AppError::BadRequest(
            "Both the originating thread and request are required.".into(),
        )),
    }
}

pub async fn attempt(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ConnectionAttempt>, AppError> {
    state
        .secrets()
        .connections()
        .attempt(&id)
        .map(Json)
        .map_err(error)
}

pub async fn cancel(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> StatusCode {
    state.secrets().connections().cancel(&id);
    StatusCode::NO_CONTENT
}

pub async fn disconnect(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<ServiceProvider>,
) -> Result<StatusCode, AppError> {
    state
        .secrets()
        .connections()
        .disconnect(provider)
        .await
        .map(|()| StatusCode::NO_CONTENT)
        .map_err(error)
}
