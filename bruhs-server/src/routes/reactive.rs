//! Local control plane and separately authenticated event ingress.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Extension, Json,
};
use bruhs_core::reactive::{ReactiveError, ReactiveRuntime};
use bruhs_models::reactive::{ReactiveCommand, ReactiveEvent, ReactiveSnapshot, MAX_PAYLOAD_BYTES};
use serde_json::{json, Value};

use crate::config::Config;
use crate::error::AppError;

pub(super) fn runtime(
    value: Option<Extension<ReactiveRuntime>>,
) -> Result<ReactiveRuntime, AppError> {
    value.map(|Extension(runtime)| runtime).ok_or_else(|| {
        AppError::Unavailable(
            "Reactive agents are not running in this server. Restart the companion server.".into(),
        )
    })
}

impl From<ReactiveError> for AppError {
    fn from(error: ReactiveError) -> Self {
        match error {
            ReactiveError::Invalid(message) => Self::BadRequest(message),
            ReactiveError::NotFound(message) => Self::NotFound(message),
            ReactiveError::Conflict(message) => Self::Conflict(message),
            ReactiveError::Unauthorized => {
                Self::Forbidden("The event source credential was not accepted.".into())
            }
            error @ ReactiveError::Storage(_) => Self::internal(error),
        }
    }
}

pub async fn snapshot(
    value: Option<Extension<ReactiveRuntime>>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let snapshot = runtime(value)?.snapshot().await?;
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(snapshot),
    ))
}

/// `POST /api/reactive/commands`. A bruh's vault grants change, its asks
/// to use a vault key are answered, and an agent is deleted (with a bruh's
/// own vault entries) only from the owner's own app, as the vault does
/// (rulings F26 and BC I2).
pub async fn command(
    value: Option<Extension<ReactiveRuntime>>,
    State(config): State<Arc<Config>>,
    headers: HeaderMap,
    Json(command): Json<ReactiveCommand>,
) -> Result<Json<ReactiveSnapshot>, AppError> {
    let runtime = runtime(value)?;
    if !config.access.owner_request(&headers) {
        if let Some(refusal) = runtime.owner_only_refusal(&command).await? {
            return Err(AppError::Forbidden(refusal.into()));
        }
    }
    Ok(Json(runtime.command(command).await?))
}

#[derive(serde::Deserialize)]
pub struct ReportPage {
    before: Option<String>,
}

pub async fn reports(
    value: Option<Extension<ReactiveRuntime>>,
    Query(page): Query<ReportPage>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let snapshot = runtime(value)?.weekly_reports(page.before).await?;
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(snapshot),
    ))
}

pub async fn report(
    value: Option<Extension<ReactiveRuntime>>,
    Path(id): Path<String>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let report = runtime(value)?.weekly_report(id).await?;
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(report),
    ))
}

pub async fn report_command(
    value: Option<Extension<ReactiveRuntime>>,
    Json(command): Json<bruhs_models::weekly_reports::WeeklyReportCommand>,
) -> Result<Json<bruhs_models::weekly_reports::WeeklyReportsSnapshot>, AppError> {
    Ok(Json(runtime(value)?.weekly_report_command(command).await?))
}

#[derive(serde::Deserialize)]
pub struct RunPage {
    /// The id of the last run on the previous page.
    before: Option<String>,
}

/// `GET /api/reactive/agents/{id}/runs?before=<run id>`: one agent's runs, newest first.
pub async fn agent_runs(
    value: Option<Extension<ReactiveRuntime>>,
    Path(id): Path<String>,
    Query(page): Query<RunPage>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let runs = runtime(value)?.agent_runs(id, page.before).await?;
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(runs),
    ))
}

/// `GET /api/reactive/rooms/{id}/messages?before=<message id>`: one room's
/// messages, newest first. An unknown room has none.
pub async fn room_messages(
    value: Option<Extension<ReactiveRuntime>>,
    Path(id): Path<String>,
    Query(page): Query<RunPage>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let messages = runtime(value)?.room_messages(id, page.before).await?;
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(messages),
    ))
}

/// `GET /api/reactive/stream`: unnamed frames of `OutboxSignal`, the same
/// producer as the combined stream's `reactive` frame.
pub async fn stream(
    value: Option<Extension<ReactiveRuntime>>,
) -> Result<
    axum::response::sse::Sse<
        impl futures_core::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
    >,
    AppError,
> {
    let runtime = runtime(value)?;
    Ok(
        axum::response::sse::Sse::new(super::events_stream::reactive_signals(Some(runtime)))
            .keep_alive(axum::response::sse::KeepAlive::default()),
    )
}

pub async fn token(
    value: Option<Extension<ReactiveRuntime>>,
    Path(id): Path<String>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let token = runtime(value)?.webhook_token(&id).await?;
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(json!({"token":token,"path":format!("/api/reactive/hooks/{id}")})),
    ))
}

pub async fn hook(
    value: Option<Extension<ReactiveRuntime>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<(StatusCode, Json<ReactiveEvent>), AppError> {
    if body.len() > MAX_PAYLOAD_BYTES {
        return Err(AppError::BadRequest(
            "Event payload exceeds the 64 KiB source budget.".into(),
        ));
    }
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|header| header.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(|| AppError::Forbidden("A source-specific Bearer token is required.".into()))?;
    // A stable delivery id makes retries idempotent. Without one, each HTTP
    // delivery is distinct; producers that retry must supply the header.
    let key = headers
        .get("x-event-id")
        .or_else(|| headers.get("x-github-delivery"))
        .and_then(|value| value.to_str().ok())
        .map_or_else(
            || bruhs_models::ids::ThreadId::new_v4().to_string(),
            str::to_string,
        );
    let payload: Value = serde_json::from_slice(&body)
        .map_err(|_| AppError::BadRequest("Webhook body must be JSON.".into()))?;
    let event = runtime(value)?
        .publish_hook(&id, token, &key, payload)
        .await?;
    Ok((StatusCode::ACCEPTED, Json(event)))
}

/// `GET /api/service`: whether automations outlive the app.
pub async fn service_status() -> impl axum::response::IntoResponse {
    // This server is the process on the port, so it reports itself and never probes.
    let asker = crate::service::Asker::Server {
        pid: std::process::id(),
    };
    let status = tokio::task::spawn_blocking(move || crate::service::status(asker))
        .await
        .unwrap_or_default();
    (
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(status),
    )
}

/// `POST /api/service`: install or remove the login service.
pub async fn service_apply(
    axum::extract::State(config): axum::extract::State<std::sync::Arc<crate::config::Config>>,
    Json(action): Json<bruhs_models::api::BackgroundServiceAction>,
) -> Result<Json<bruhs_models::api::BackgroundService>, AppError> {
    let dist = config.spa_dir.clone();
    let asker = crate::service::Asker::Server {
        pid: std::process::id(),
    };
    tokio::task::spawn_blocking(move || crate::service::apply(action, &dist, asker))
        .await
        .map_err(|_| AppError::Unavailable("The service installer stopped.".into()))?
        .map(Json)
        .map_err(|error| match error {
            crate::service::ServiceError::Unsupported(message) => AppError::BadRequest(message),
            crate::service::ServiceError::Failed(message) => AppError::Conflict(message),
        })
}

#[cfg(test)]
mod tests;
