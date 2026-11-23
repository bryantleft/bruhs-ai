//! Local control plane and separately authenticated event ingress.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Extension, Json,
};
use bruhs_core::reactive::{ReactiveError, ReactiveRuntime};
use bruhs_models::reactive::{ReactiveCommand, ReactiveEvent, ReactiveSnapshot, MAX_PAYLOAD_BYTES};
use serde_json::json;

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

/// `GET /api/reactive`. A page that isn't the owner's gets vault key, app
/// and mail cards without their detail: the account, the call's body, an
/// email's recipients and text (final review BC I1). It gets every bruh's
/// memory blank too, in its agent and its proposals (imports B I1).
pub async fn snapshot(
    value: Option<Extension<ReactiveRuntime>>,
    State(config): State<Arc<Config>>,
    headers: HeaderMap,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let mut snapshot = runtime(value)?.snapshot().await?;
    if !config.access.owner_request(&headers) {
        snapshot = snapshot.without_owner_details();
    }
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(snapshot),
    ))
}

/// `POST /api/reactive/commands`. A bruh's vault grants change, its asks
/// to use a vault key are answered, and an agent is deleted (with a bruh's
/// own vault entries) only from the owner's own app, as the vault does
/// (rulings F26 and BC I2). So is a bruh's memory changed: another page's
/// agent save keeps the stored memory it was sent blank (imports B I1).
pub async fn command(
    value: Option<Extension<ReactiveRuntime>>,
    State(config): State<Arc<Config>>,
    headers: HeaderMap,
    Json(command): Json<ReactiveCommand>,
) -> Result<Json<ReactiveSnapshot>, AppError> {
    let runtime = runtime(value)?;
    let owner = config.access.owner_request(&headers);
    let command = if owner {
        command
    } else {
        let command = runtime.with_stored_memory(command).await?;
        if let Some(refusal) = runtime.owner_only_refusal(&command).await? {
            return Err(AppError::Forbidden(refusal.into()));
        }
        command
    };
    let snapshot = runtime.command(command).await?;
    // The answer is the snapshot, so it leaves out owner card details the
    // same way `GET /api/reactive` does (re-review N3).
    Ok(Json(if owner {
        snapshot
    } else {
        snapshot.without_owner_details()
    }))
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

/// The task host resolves availability for the same checkout that will run the task.
pub async fn task_models(
    Query(scope): Query<bruhs_models::delivery::DeliveryPreferenceScope>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let project = std::path::Path::new(scope.path.as_deref().unwrap_or_default());
    if !project.as_os_str().is_empty() && !project.is_absolute() {
        return Err(AppError::BadRequest(
            "Choose an absolute project folder.".into(),
        ));
    }
    let catalog = bruhs_core::agent::writer_catalog_for_project(project).await;
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(catalog),
    ))
}

#[derive(serde::Deserialize)]
pub struct ThreadSearch {
    project: String,
    #[serde(default)]
    desktop: bool,
    #[serde(default)]
    query: String,
}

pub async fn task_threads(
    value: Option<Extension<ReactiveRuntime>>,
    Query(search): Query<ThreadSearch>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let host = runtime(value)?;
    let threads = if search.desktop {
        host.project_conversations(search.project, search.query)
            .await?
    } else {
        host.task_threads(search.project, search.query).await?
    };
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(threads),
    ))
}

#[derive(serde::Deserialize)]
pub struct DesktopTurnQuery {
    thread: bruhs_models::ids::ThreadId,
    #[serde(default)]
    after: i64,
}

pub async fn desktop_task_turns(
    value: Option<Extension<ReactiveRuntime>>,
    Query(query): Query<DesktopTurnQuery>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let turns = runtime(value)?
        .desktop_task_turns(query.thread, query.after)
        .await?;
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(turns),
    ))
}

#[derive(serde::Deserialize)]
pub struct RunPage {
    /// The id of the last run on the previous page.
    before: Option<String>,
    thread: Option<bruhs_models::ids::ThreadId>,
}

/// `GET /api/reactive/agents/{id}/runs?before=<run id>`: one agent's runs, newest first.
pub async fn agent_runs(
    value: Option<Extension<ReactiveRuntime>>,
    Path(id): Path<String>,
    Query(page): Query<RunPage>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let runs = runtime(value)?
        .agent_thread_runs(id, page.before, page.thread)
        .await?;
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
/// producer as the combined stream's `reactive` frame, without owner card
/// details for a page that isn't the owner's (final review BC I1).
pub async fn stream(
    value: Option<Extension<ReactiveRuntime>>,
    State(config): State<Arc<Config>>,
    headers: HeaderMap,
) -> Result<
    axum::response::sse::Sse<
        impl futures_core::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
    >,
    AppError,
> {
    let runtime = runtime(value)?;
    let owner = config.access.owner_request(&headers);
    Ok(
        axum::response::sse::Sse::new(super::events_stream::reactive_signals(Some(runtime), owner))
            .keep_alive(axum::response::sse::KeepAlive::default()),
    )
}

pub async fn webhook_deliveries(
    value: Option<Extension<ReactiveRuntime>>,
    Path(task): Path<String>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let deliveries = runtime(value)?.webhook_deliveries(task).await?;
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(deliveries),
    ))
}

pub async fn webhook_delivery(
    value: Option<Extension<ReactiveRuntime>>,
    Path((task, delivery)): Path<(String, String)>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let detail = runtime(value)?.webhook_delivery(task, delivery).await?;
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(detail),
    ))
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

/// Local owner endpoints keep the credential out of snapshots and browser caches.
pub async fn task_token(
    value: Option<Extension<ReactiveRuntime>>,
    Path(id): Path<String>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let token = runtime(value)?.task_webhook_token(id, None).await?;
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(token),
    ))
}

pub async fn rotate_task_token(
    value: Option<Extension<ReactiveRuntime>>,
    Path(id): Path<String>,
    Json(previous): Json<String>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let token = runtime(value)?
        .task_webhook_token(id, Some(previous))
        .await?;
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(token),
    ))
}

#[derive(serde::Deserialize)]
pub struct HookPath {
    id: String,
    token: Option<String>,
}

pub struct HookArrival(bruhs_models::EpochMs);

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for HookArrival {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        _: &mut axum::http::request::Parts,
        _: &S,
    ) -> Result<Self, Self::Rejection> {
        Ok(Self(bruhs_models::EpochMs::now()))
    }
}

pub async fn hook(
    HookArrival(received): HookArrival,
    value: Option<Extension<ReactiveRuntime>>,
    Path(path): Path<HookPath>,
    uri: axum::http::Uri,
    Query(query): Query<Vec<(String, String)>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<(StatusCode, Json<ReactiveEvent>), AppError> {
    if body.len() > MAX_PAYLOAD_BYTES {
        return Err(AppError::BadRequest(
            "Event payload exceeds the 64 KiB source budget.".into(),
        ));
    }
    use bruhs_core::reactive::WebhookTarget;
    let (target, token, receipt_uri) = if let Some(token) = path.token {
        // Templates and delivery history may display the URI. Omit the capability segment.
        let mut safe = uri
            .path()
            .rsplit_once('/')
            .map_or("", |(prefix, _)| prefix)
            .to_owned();
        safe.push_str("/[redacted]");
        if let Some(query) = uri.query() {
            safe.push('?');
            safe.push_str(query);
        }
        (WebhookTarget::Task(path.id), token, safe)
    } else {
        let token = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|header| header.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or_else(|| {
                AppError::Forbidden("A source-specific Bearer token is required.".into())
            })?;
        (
            WebhookTarget::Source(path.id),
            token.to_owned(),
            uri.to_string(),
        )
    };
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
    let mut request = bruhs_models::reactive::WebhookMetadata {
        raw_body: Some(body.to_vec()),
        uri: Some(receipt_uri),
        ..Default::default()
    };
    for (name, value) in &headers {
        if matches!(
            name.as_str(),
            "authorization" | "proxy-authorization" | "cookie"
        ) {
            continue;
        }
        // Keep each field occurrence so an unreadable second signature cannot disappear.
        request
            .headers
            .entry(name.as_str().into())
            .or_default()
            .push(value.to_str().unwrap_or_default().into());
    }
    for (name, value) in query {
        request.query.entry(name).or_default().push(value);
    }
    let mut event = runtime(value)?
        .receive_addressed_webhook(target, &token, &key, None, request, received)
        .await?;
    // Subscription identities belong to the host. The sender receives its delivery receipt.
    event.filtered_subscriptions.clear();
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
