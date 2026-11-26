//! Local control and the independently scoped task and keybinding sharing listener.
use crate::{error::AppError, routes::paths::HomePath};
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive},
        Html, IntoResponse, Sse,
    },
    routing::{get, post},
    Extension, Json, Router,
};
use bruhs_core::coordination::{AccessScope, BoardLinks, CoordinationError, CoordinationRuntime};
use bruhs_models::coordination::{board::TaskBoard, CoordinationCommand, CoordinationSnapshot};
use serde::Deserialize;
use std::{convert::Infallible, sync::Arc};
use tower_http::compression::{
    predicate::{DefaultPredicate, Predicate, SizeAbove},
    CompressionLayer, CompressionLevel,
};

mod provider_updates;
mod task_editor;
mod task_thread;

impl From<CoordinationError> for AppError {
    fn from(error: CoordinationError) -> Self {
        match error {
            CoordinationError::Invalid(message) => Self::BadRequest(message),
            CoordinationError::Conflict(message) => Self::Conflict(message),
            CoordinationError::NotFound(message) => Self::NotFound(message),
            CoordinationError::Unauthorized => Self::Forbidden(error.to_string()),
            error @ CoordinationError::Storage(_) => Self::internal(error),
        }
    }
}

fn runtime(value: Option<Extension<CoordinationRuntime>>) -> Result<CoordinationRuntime, AppError> {
    value.map(|Extension(runtime)| runtime).ok_or_else(|| {
        AppError::Unavailable(
            "Task workspaces are unavailable. Restart the companion server.".into(),
        )
    })
}

pub async fn snapshot(
    value: Option<Extension<CoordinationRuntime>>,
) -> Result<impl IntoResponse, AppError> {
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(runtime(value)?.snapshot().await),
    ))
}

/// The orchestration board: every task placed in its delivery lane. Branch
/// and pull request lookups refresh in the background; see `TaskBoard`.
pub async fn board(
    value: Option<Extension<CoordinationRuntime>>,
    links: Option<Extension<BoardLinks>>,
) -> Result<impl IntoResponse, AppError> {
    let snapshot = runtime(value)?.snapshot().await;
    let Some(Extension(links)) = links else {
        return Err(AppError::Unavailable(
            "The task board is unavailable. Restart the companion server.".into(),
        ));
    };
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(links.board(&snapshot).await),
    ))
}

pub async fn ui() -> impl IntoResponse {
    (
        [(header::CACHE_CONTROL, "no-store")],
        Html(include_str!("coordination/remote.html")),
    )
}

async fn checked_command(
    runtime: &CoordinationRuntime,
    command: CoordinationCommand,
) -> Result<CoordinationSnapshot, AppError> {
    match &command {
        CoordinationCommand::Create { task, .. } | CoordinationCommand::Fanout { task, .. } => {
            HomePath::resolve(&task.project_path).await?;
        }
        CoordinationCommand::SaveSpec { project_path, .. } => {
            HomePath::resolve(project_path).await?;
        }
        _ => {}
    }
    Ok(runtime.command(command).await?)
}

pub async fn command(
    value: Option<Extension<CoordinationRuntime>>,
    Json(command): Json<CoordinationCommand>,
) -> Result<Json<CoordinationSnapshot>, AppError> {
    Ok(Json(checked_command(&runtime(value)?, command).await?))
}

pub async fn prepare_work_item(
    value: Option<Extension<CoordinationRuntime>>,
    Json(request): Json<bruhs_models::workbench_handoff::WorkItemHandoffRequest>,
) -> Result<Json<bruhs_models::workbench_handoff::WorkItemHandoff>, AppError> {
    Ok(Json(runtime(value)?.prepare_work_item(request).await?))
}

pub async fn work_item_handoffs(
    value: Option<Extension<CoordinationRuntime>>,
) -> Result<impl IntoResponse, AppError> {
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(runtime(value)?.work_item_handoffs().await),
    ))
}

pub async fn work_item_handoff(
    value: Option<Extension<CoordinationRuntime>>,
    Path(id): Path<bruhs_models::ids::ThreadId>,
) -> Result<impl IntoResponse, AppError> {
    let handoff = runtime(value)?.work_item_handoff(&id).await?;
    let task = bruhs_models::workbench_handoff::work_item_draft(&handoff.request)
        .map_err(|error| AppError::BadRequest(error.into()))?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({"handoff":handoff,"task":task})),
    ))
}

pub async fn start_work_item(
    value: Option<Extension<CoordinationRuntime>>,
    Path(id): Path<bruhs_models::ids::ThreadId>,
    Json(request): Json<bruhs_models::coordination::NewTask>,
) -> Result<Json<CoordinationSnapshot>, AppError> {
    let runtime = runtime(value)?;
    if runtime.work_item_handoff(&id).await?.started.is_none() {
        HomePath::resolve(&request.project_path).await?;
    }
    runtime.start_work_item(&id, request).await?;
    Ok(Json(runtime.snapshot().await))
}

pub async fn forget_work_item_handoff(
    value: Option<Extension<CoordinationRuntime>>,
    Path(id): Path<bruhs_models::ids::ThreadId>,
) -> Result<StatusCode, AppError> {
    runtime(value)?.forget_work_item_handoff(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ChildWait {
    parent_id: bruhs_models::ids::ThreadId,
    id: bruhs_models::ids::ThreadId,
}

pub async fn wait_for_child(
    value: Option<Extension<CoordinationRuntime>>,
    Json(request): Json<ChildWait>,
) -> Result<Json<bruhs_models::coordination::CoordinatedTask>, AppError> {
    Ok(Json(
        runtime(value)?
            .wait_for_child(&request.parent_id, &request.id)
            .await?,
    ))
}

#[derive(Deserialize)]
pub struct AccessRequest {
    scope: AccessScope,
    lifetime_seconds: u32,
    #[serde(default)]
    provider_updates: bool,
}

pub async fn access_list(
    value: Option<Extension<CoordinationRuntime>>,
) -> Result<impl IntoResponse, AppError> {
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(runtime(value)?.access_grants().await),
    ))
}

pub async fn issue_access(
    value: Option<Extension<CoordinationRuntime>>,
    Json(request): Json<AccessRequest>,
) -> Result<impl IntoResponse, AppError> {
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(
            runtime(value)?
                .issue_access_with_updates(
                    request.scope,
                    request.lifetime_seconds,
                    request.provider_updates,
                )
                .await?,
        ),
    ))
}

pub async fn revoke_access(
    value: Option<Extension<CoordinationRuntime>>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    runtime(value)?.revoke_access(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn changes(
    value: Option<Extension<CoordinationRuntime>>,
) -> Result<impl IntoResponse, AppError> {
    let runtime = runtime(value)?;
    Ok(change_stream(runtime, None))
}

fn change_stream(runtime: CoordinationRuntime, token: Option<String>) -> impl IntoResponse {
    let mut receiver = runtime.changes();
    let stream = async_stream::stream! {
        yield Ok::<Event, Infallible>(Event::default().event("refresh").data("refresh"));
        while receiver.changed().await.is_ok() {
            if let Some(token) = &token { if runtime.authorize(token).await.is_err() { break; } }
            yield Ok(Event::default().event("refresh").data("refresh"));
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::default())
}

struct RemoteState {
    runtime: CoordinationRuntime,
    links: BoardLinks,
    desktop_state: Option<std::path::PathBuf>,
    workbench_dir: Option<std::path::PathBuf>,
    projects: tokio::sync::Mutex<
        Option<(
            std::time::Instant,
            Vec<bruhs_core::linked_projects::Checkout>,
        )>,
    >,
}

impl RemoteState {
    fn usage_root(&self) -> Result<std::path::PathBuf, AppError> {
        self.workbench_dir.clone().ok_or_else(|| {
            AppError::internal(std::io::Error::other(
                "Private workbench directory is unavailable.",
            ))
        })
    }

    async fn authorize_usage_price_editor(&self, bearer: &str) -> Result<(), AppError> {
        if self.runtime.authorize(bearer).await?.edits_usage_prices() {
            Ok(())
        } else {
            Err(CoordinationError::Unauthorized.into())
        }
    }
}

/// Bind this router only after explicit self-hosted access configuration. It
/// contains no life, capture, file, credential, or ordinary-agent API routes.
pub fn remote_router(runtime: CoordinationRuntime, links: BoardLinks) -> Router {
    remote_router_with_state(
        runtime,
        links,
        bruhs_core::delivery::writer::desktop_state_file(),
        bruhs_core::agent_extensions::directory().ok(),
    )
}

fn remote_router_with_state(
    runtime: CoordinationRuntime,
    links: BoardLinks,
    desktop_state: Option<std::path::PathBuf>,
    workbench_dir: Option<std::path::PathBuf>,
) -> Router {
    Router::new().route("/", get(|| async { Html(include_str!("coordination/remote.html")) }))
        .route("/favicon.ico", get(|| async { StatusCode::NO_CONTENT }))
        .route("/manifest.webmanifest", get(|| async {
            ([(header::CONTENT_TYPE, "application/manifest+json")], r##"{"id":"/","name":"Bruhs tasks","short_name":"Bruhs","start_url":"/","scope":"/","display":"standalone","background_color":"#141210","theme_color":"#141210","icons":[{"src":"/icon-512.png","sizes":"512x512","type":"image/png","purpose":"any"}]}"##)
        }))
        .route("/icon-512.png", get(|| async {
            ([(header::CONTENT_TYPE, "image/png")], include_bytes!("coordination/icon-512.png").as_slice())
        }))
        .merge(Router::new()
            .route("/api/projects/scheduled-tasks/{id}/deliveries", get(super::reactive::webhook_deliveries))
            .route("/api/projects/scheduled-tasks/{id}/deliveries/{delivery}", get(super::reactive::webhook_delivery))
            .route_layer(axum::middleware::from_fn_with_state(runtime.clone(), authorize_delivery_read)))
        .route("/api/projects", get(remote_projects))
        .merge(provider_updates::routes())
        .merge(task_editor::routes(runtime.clone()))
        .merge(task_thread::routes(runtime.clone()))
        .route("/api/projects/usage", get(remote_project_usage))
        .route("/api/projects/usage/history", get(remote_project_usage_history))
        .route("/api/projects/usage/model-prices", get(remote_usage_model_prices).post(edit_remote_usage_model_prices))
        .route("/api/projects/scheduled-tasks", get(remote_scheduled_tasks))
        .route("/api/projects/scheduled-tasks/{id}", axum::routing::delete(delete_remote_schedule))
        .route("/api/projects/scheduled-tasks/{id}/enabled", axum::routing::put(set_remote_schedule_enabled))
        .route("/api/projects/scheduled-tasks/{id}/runs/{request}", axum::routing::put(run_remote_schedule))
        .route("/api/projects/names", post(rename_remote_projects))
        .route("/api/keybindings/open", post(open_remote_keybindings))
        .route("/api/keybindings", get(remote_keybindings).post(edit_remote_keybinding))
        .route("/api/tasks", get(remote_snapshot))
        .route("/api/board", get(remote_board))
        .route("/api/commands", post(remote_command))
        .route("/api/proposals", post(remote_proposal))
        .route("/api/changes", get(remote_changes))
        .route("/api/tasks/{id}/worktree", get(remote_worktree))
        .layer(axum::extract::DefaultBodyLimit::max(bruhs_models::logic::composer_text::MESSAGE_CHAR_LIMIT * 4))
        .layer(tower_http::set_header::SetResponseHeaderLayer::overriding(header::CACHE_CONTROL, axum::http::HeaderValue::from_static("no-store")))
        .layer(tower_http::set_header::SetResponseHeaderLayer::overriding(header::REFERRER_POLICY, axum::http::HeaderValue::from_static("no-referrer")))
        .layer(tower_http::set_header::SetResponseHeaderLayer::overriding(header::CONTENT_SECURITY_POLICY, axum::http::HeaderValue::from_static("default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'")))
        .with_state(Arc::new(RemoteState { runtime, links, desktop_state, workbench_dir, projects: Default::default() }))
        .layer(remote_response_compression())
}

// First tested size with positive HTTP/TCP payload savings in every small-body
// fixture. See docs/performance-task-compression-2026-11-13.md.
const MIN_REMOTE_COMPRESS_BYTES: u16 = 768;

fn remote_response_compression() -> CompressionLayer<impl Predicate> {
    // Keep finite responses cheap; the default predicate leaves SSE immediate.
    // Quality tradeoffs: docs/performance-task-compression-2026-11-13.md.
    CompressionLayer::new()
        .gzip(true)
        .no_br()
        .no_deflate()
        .no_zstd()
        .quality(CompressionLevel::Fastest)
        .compress_when(
            DefaultPredicate::new()
                .and(SizeAbove::new(MIN_REMOTE_COMPRESS_BYTES))
                .and(
                    |status: StatusCode,
                     _: axum::http::Version,
                     _: &HeaderMap,
                     _: &axum::http::Extensions| { status.is_success() },
                ),
        )
}

fn token(headers: &HeaderMap) -> Result<String, AppError> {
    if let Some(origin) = headers.get(header::ORIGIN) {
        let origin = origin
            .to_str()
            .ok()
            .and_then(|origin| reqwest::Url::parse(origin).ok())
            .ok_or_else(|| AppError::Forbidden("Use the paired host's own task page.".into()))?;
        let host = headers
            .get(header::HOST)
            .and_then(|host| host.to_str().ok())
            .unwrap_or("");
        let origin = origin.origin().ascii_serialization();
        let authority = origin.split_once("://").map_or("", |(_, host)| host);
        if authority != host {
            return Err(AppError::Forbidden(
                "Cross-origin task requests are not accepted.".into(),
            ));
        }
    }
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            AppError::Forbidden("Pair this device with a local owner-issued access token.".into())
        })
}

use bruhs_models::logic::keybindings::{EnvironmentKeybindings, KeybindingEdit};

async fn remote_projects(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let bearer = token(&headers)?;
    if !matches!(
        state.runtime.authorize(&bearer).await?,
        AccessScope::Projects
            | AccessScope::ProjectUsage { .. }
            | AccessScope::ProjectUsageHistory { .. }
            | AccessScope::ProjectNames
            | AccessScope::ProjectSchedules { .. }
    ) {
        return Err(CoordinationError::Unauthorized.into());
    }
    let mut cached = state.projects.lock().await;
    if cached
        .as_ref()
        .is_none_or(|(time, _)| time.elapsed() >= std::time::Duration::from_secs(10))
    {
        let file = state
            .desktop_state
            .clone()
            .ok_or_else(|| AppError::Unavailable("The host has no desktop workspace.".into()))?;
        let projects = bruhs_core::linked_projects::inventory(file)
            .await
            .map_err(|_| {
                AppError::Unavailable("Could not read the host’s saved projects.".into())
            })?;
        *cached = Some((std::time::Instant::now(), projects));
    }
    let grant = state.runtime.authorize_grant(&bearer).await?;
    let scope = grant.scope;
    let usage = scope.shares_usage();
    let usage_history = scope.shares_usage_history();
    let edit_usage_prices = scope.edits_usage_prices();
    let conversations = matches!(
        scope,
        AccessScope::ProjectSchedules {
            edit: true,
            conversations: true,
            ..
        }
    );
    let read_threads = matches!(
        scope,
        AccessScope::ProjectSchedules {
            read_threads: true,
            ..
        }
    );
    let reply_threads = read_threads
        && matches!(
            scope,
            AccessScope::ProjectSchedules {
                reply_threads: true,
                ..
            }
        );
    let task_edit = matches!(scope, AccessScope::ProjectSchedules { edit: true, .. });
    let (editable, schedules, toggle, run, delete, deliveries) = match scope {
        AccessScope::ProjectUsage { rename } | AccessScope::ProjectUsageHistory { rename, .. } => {
            (rename, false, false, false, false, false)
        }
        AccessScope::Projects => (false, false, false, false, false, false),
        AccessScope::ProjectNames => (true, false, false, false, false, false),
        AccessScope::ProjectSchedules {
            rename,
            toggle,
            run,
            delete,
            deliveries,
            ..
        } => (rename, true, toggle, run, delete, deliveries),
        _ => return Err(CoordinationError::Unauthorized.into()),
    };
    Ok((
        [
            (
                "x-bruhs-provider-updates",
                grant.provider_updates.to_string(),
            ),
            ("x-bruhs-edit-project-names", editable.to_string()),
            ("x-bruhs-read-scheduled-tasks", schedules.to_string()),
            ("x-bruhs-toggle-scheduled-tasks", toggle.to_string()),
            ("x-bruhs-run-scheduled-tasks", run.to_string()),
            ("x-bruhs-delete-scheduled-tasks", delete.to_string()),
            ("x-bruhs-webhook-deliveries", deliveries.to_string()),
            ("x-bruhs-read-usage", usage.to_string()),
            ("x-bruhs-read-usage-history", usage_history.to_string()),
            ("x-bruhs-edit-usage-prices", edit_usage_prices.to_string()),
            ("x-bruhs-edit-scheduled-tasks", task_edit.to_string()),
            ("x-bruhs-task-conversations", conversations.to_string()),
            ("x-bruhs-read-task-threads", read_threads.to_string()),
            ("x-bruhs-reply-task-threads", reply_threads.to_string()),
        ],
        Json(
            cached
                .as_ref()
                .map(|(_, projects)| projects.clone())
                .unwrap_or_default(),
        ),
    ))
}

async fn remote_project_usage(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let bearer = token(&headers)?;
    if !state.runtime.authorize(&bearer).await?.shares_usage() {
        return Err(CoordinationError::Unauthorized.into());
    }
    let accounts = bruhs_core::usage_hubs::linked::device_report().await;
    if !state.runtime.authorize(&bearer).await?.shares_usage() {
        return Err(CoordinationError::Unauthorized.into());
    }
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(accounts)))
}

async fn remote_project_usage_history(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
    axum::extract::Query(period): axum::extract::Query<
        bruhs_models::agent_extensions::UsageHistoryPeriod,
    >,
) -> Result<impl IntoResponse, AppError> {
    use bruhs_core::agent_extensions::{shared_history_at, UsageHistoryRange};

    let bearer = token(&headers)?;
    if !state
        .runtime
        .authorize(&bearer)
        .await?
        .shares_usage_history()
    {
        return Err(CoordinationError::Unauthorized.into());
    }
    let range = UsageHistoryRange::from_period(period).ok_or_else(|| {
        AppError::BadRequest(
            "Choose a current rolling 24-hour period or a range from 1 to 90 days.".into(),
        )
    })?;
    let root = state.usage_root()?;
    let history =
        tokio::task::spawn_blocking(move || shared_history_at(&root, range, period.provider))
            .await
            .map_err(AppError::internal)?
            .map_err(AppError::internal)?;
    if !state
        .runtime
        .authorize(&bearer)
        .await?
        .shares_usage_history()
    {
        return Err(CoordinationError::Unauthorized.into());
    }
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(history)))
}

async fn remote_usage_model_prices(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let bearer = token(&headers)?;
    state.authorize_usage_price_editor(&bearer).await?;
    let root = state.usage_root()?;
    let rows = tokio::task::spawn_blocking(move || {
        bruhs_core::agent_extensions::usage_model_pricing_at(&root)
    })
    .await
    .map_err(AppError::internal)?
    .map_err(AppError::internal)?;
    state.authorize_usage_price_editor(&bearer).await?;
    Ok(Json(rows))
}

async fn edit_remote_usage_model_prices(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
    Json(changes): Json<Vec<bruhs_models::agent_extensions::UsageModelPricingChange>>,
) -> Result<StatusCode, AppError> {
    use std::io::ErrorKind;

    let bearer = token(&headers)?;
    state.authorize_usage_price_editor(&bearer).await?;
    let root = state.usage_root()?;
    tokio::task::spawn_blocking(move || {
        bruhs_core::agent_extensions::edit_usage_model_prices_at(&root, changes)
    })
    .await
    .map_err(AppError::internal)?
    .map_err(|error| match error.kind() {
        ErrorKind::InvalidInput => AppError::BadRequest(error.to_string()),
        ErrorKind::AlreadyExists => AppError::Conflict(error.to_string()),
        _ => AppError::internal(error),
    })?;
    Ok(StatusCode::NO_CONTENT)
}

async fn authorize_delivery_read(
    State(runtime): State<CoordinationRuntime>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, AppError> {
    let bearer = token(request.headers())?;
    let allowed = async || -> Result<(), AppError> {
        match runtime.authorize(&bearer).await? {
            AccessScope::ProjectSchedules {
                deliveries: true, ..
            } => Ok(()),
            _ => Err(CoordinationError::Unauthorized.into()),
        }
    };
    allowed().await?;
    let response = next.run(request).await;
    allowed().await?;
    Ok(response)
}

async fn remote_scheduled_tasks(
    State(state): State<Arc<RemoteState>>,
    reactive: Option<axum::Extension<bruhs_core::reactive::ReactiveRuntime>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let bearer = token(&headers)?;
    let AccessScope::ProjectSchedules { read_threads, .. } =
        state.runtime.authorize(&bearer).await?
    else {
        return Err(CoordinationError::Unauthorized.into());
    };
    let reactive = reactive
        .ok_or_else(|| AppError::Unavailable("The automation host is unavailable.".into()))?;
    let tasks = reactive
        .linked_schedule_inventory(read_threads)
        .await
        .map_err(|_| AppError::Unavailable("Could not read scheduled tasks.".into()))?;
    if !matches!(
        state.runtime.authorize(&bearer).await?,
        AccessScope::ProjectSchedules { .. }
    ) {
        return Err(CoordinationError::Unauthorized.into());
    }
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(tasks)))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ScheduleEnabled {
    enabled: bool,
}

async fn set_remote_schedule_enabled(
    State(state): State<Arc<RemoteState>>,
    reactive: Option<axum::Extension<bruhs_core::reactive::ReactiveRuntime>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(edit): Json<ScheduleEnabled>,
) -> Result<impl IntoResponse, AppError> {
    let bearer = token(&headers)?;
    if !matches!(
        state.runtime.authorize(&bearer).await?,
        AccessScope::ProjectSchedules { toggle: true, .. }
    ) {
        return Err(CoordinationError::Unauthorized.into());
    }
    let reactive = reactive
        .ok_or_else(|| AppError::Unavailable("The automation host is unavailable.".into()))?;
    let task = reactive
        .set_linked_schedule_enabled(id, edit.enabled)
        .await
        .map_err(|err| match err {
            bruhs_core::reactive::ReactiveError::Storage(_) => {
                AppError::Unavailable("The host could not save the scheduled task.".into())
            }
            other => AppError::from(other),
        })?;
    if !matches!(
        state.runtime.authorize(&bearer).await?,
        AccessScope::ProjectSchedules { toggle: true, .. }
    ) {
        return Err(CoordinationError::Unauthorized.into());
    }
    Ok(Json(task))
}

async fn delete_remote_schedule(
    State(state): State<Arc<RemoteState>>,
    reactive: Option<axum::Extension<bruhs_core::reactive::ReactiveRuntime>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, AppError> {
    let scope = state.runtime.authorize(&token(&headers)?).await?;
    if !matches!(scope, AccessScope::ProjectSchedules { delete: true, .. }) {
        return Err(CoordinationError::Unauthorized.into());
    }
    let reactive = reactive
        .ok_or_else(|| AppError::Unavailable("The automation host is unavailable.".into()))?;
    reactive
        .delete_linked_schedule(id)
        .await
        .map_err(|err| match err {
            bruhs_core::reactive::ReactiveError::Storage(_) => {
                AppError::Unavailable("The host could not delete the scheduled task.".into())
            }
            other => AppError::from(other),
        })?;
    Ok(StatusCode::NO_CONTENT)
}

async fn run_remote_schedule(
    State(state): State<Arc<RemoteState>>,
    reactive: Option<axum::Extension<bruhs_core::reactive::ReactiveRuntime>>,
    Path((id, request)): Path<(String, uuid::Uuid)>,
    headers: HeaderMap,
) -> Result<StatusCode, AppError> {
    let scope = state.runtime.authorize(&token(&headers)?).await?;
    if !matches!(scope, AccessScope::ProjectSchedules { run: true, .. }) {
        return Err(CoordinationError::Unauthorized.into());
    }
    let reactive = reactive
        .ok_or_else(|| AppError::Unavailable("The automation host is unavailable.".into()))?;
    reactive
        .run_linked_schedule(id, request)
        .await
        .map_err(|err| match err {
            bruhs_core::reactive::ReactiveError::Storage(_) => {
                AppError::Unavailable("The host could not queue the scheduled task.".into())
            }
            other => AppError::from(other),
        })?;
    Ok(StatusCode::NO_CONTENT)
}

async fn rename_remote_projects(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
    Json(edit): Json<bruhs_core::linked_projects::names::Rename>,
) -> Result<StatusCode, AppError> {
    let bearer = token(&headers)?;
    let mut cache = state.projects.lock().await;
    if !matches!(
        state.runtime.authorize(&bearer).await?,
        AccessScope::ProjectNames
            | AccessScope::ProjectUsage { rename: true }
            | AccessScope::ProjectUsageHistory { rename: true, .. }
            | AccessScope::ProjectSchedules { rename: true, .. }
    ) {
        return Err(CoordinationError::Unauthorized.into());
    }
    let path = state
        .desktop_state
        .clone()
        .ok_or_else(|| AppError::Unavailable("The host has no desktop workspace.".into()))?;
    tokio::task::spawn_blocking(move || {
        bruhs_core::linked_projects::names::rename(&path, &edit, false)
    })
    .await
    .map_err(AppError::internal)?
    .map_err(|err| match err.kind() {
        std::io::ErrorKind::InvalidInput => AppError::BadRequest(err.to_string()),
        std::io::ErrorKind::WouldBlock => AppError::Conflict(err.to_string()),
        std::io::ErrorKind::NotFound => {
            AppError::NotFound("This checkout is no longer available.".into())
        }
        _ => AppError::Unavailable("Could not save the project name on this host.".into()),
    })?;
    *cache = None;
    drop(cache);
    Ok(StatusCode::NO_CONTENT)
}

async fn open_remote_keybindings(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
) -> Result<StatusCode, AppError> {
    if state.runtime.authorize(&token(&headers)?).await?
        != (AccessScope::Keybindings { editable: true })
    {
        return Err(CoordinationError::Unauthorized.into());
    }
    let path = state.desktop_state.clone().ok_or_else(|| {
        AppError::Unavailable("Desktop settings are unavailable on this host.".into())
    })?;
    tokio::task::spawn_blocking(move || bruhs_core::keybinding_file::open_environment_file(&path))
        .await
        .map_err(AppError::internal)?
        .map_err(|_| {
            AppError::Unavailable(
                "The host could not open its keybindings file in its configured editor.".into(),
            )
        })?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remote_keybindings(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
) -> Result<Json<EnvironmentKeybindings>, AppError> {
    host_keybindings(state, headers, None).await
}
async fn edit_remote_keybinding(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
    Json(edit): Json<KeybindingEdit>,
) -> Result<Json<EnvironmentKeybindings>, AppError> {
    host_keybindings(state, headers, Some(edit)).await
}
async fn host_keybindings(
    state: Arc<RemoteState>,
    headers: HeaderMap,
    edit: Option<KeybindingEdit>,
) -> Result<Json<EnvironmentKeybindings>, AppError> {
    let AccessScope::Keybindings { editable } = state.runtime.authorize(&token(&headers)?).await?
    else {
        return Err(CoordinationError::Unauthorized.into());
    };
    if edit.is_some() && !editable {
        return Err(CoordinationError::Unauthorized.into());
    }
    let path = state.desktop_state.clone().ok_or_else(|| {
        AppError::Unavailable("Desktop settings are unavailable on this host.".into())
    })?;
    let rules = tokio::task::spawn_blocking(move || {
        bruhs_core::keybinding_file::environment_rules(&path, edit.as_ref())
    })
    .await
    .map_err(AppError::internal)?
    .map_err(|e| match e.kind() {
        std::io::ErrorKind::InvalidInput => AppError::BadRequest(e.to_string()),
        std::io::ErrorKind::WouldBlock => AppError::Conflict(e.to_string()),
        _ => AppError::internal(e),
    })?;
    Ok(Json(EnvironmentKeybindings { editable, rules }))
}

async fn remote_snapshot(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
) -> Result<Json<CoordinationSnapshot>, AppError> {
    let scope = state.runtime.authorize(&token(&headers)?).await?;
    Ok(Json(state.runtime.scoped_snapshot(&scope).await?))
}

/// A guest's scoped snapshot carries no paths, so its board never reads a
/// branch or pull request; the owner's board matches the local one.
async fn remote_board(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
) -> Result<Json<TaskBoard>, AppError> {
    let scope = state.runtime.authorize(&token(&headers)?).await?;
    let snapshot = state.runtime.scoped_snapshot(&scope).await?;
    Ok(Json(state.links.board(&snapshot).await))
}

async fn remote_command(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
    Json(command): Json<CoordinationCommand>,
) -> Result<Json<CoordinationSnapshot>, AppError> {
    if state.runtime.authorize(&token(&headers)?).await? != AccessScope::Owner {
        return Err(CoordinationError::Unauthorized.into());
    }
    Ok(Json(checked_command(&state.runtime, command).await?))
}

/// The owner's task panel on a paired device: the same worktree view as
/// the local one, sizes included. Guests have no worktree.
async fn remote_worktree(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
    Path(id): Path<bruhs_models::ids::ThreadId>,
    axum::extract::Query(query): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<impl IntoResponse, AppError> {
    if state.runtime.authorize(&token(&headers)?).await? != AccessScope::Owner {
        return Err(CoordinationError::Unauthorized.into());
    }
    let measure = query.get("measure").is_some_and(|value| value == "true");
    Ok(Json(state.runtime.task_worktree(&id, measure).await?))
}

#[derive(Deserialize)]
struct ProposalRequest {
    text: String,
}
async fn remote_proposal(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
    Json(request): Json<ProposalRequest>,
) -> Result<impl IntoResponse, AppError> {
    let scope = state.runtime.authorize(&token(&headers)?).await?;
    Ok(Json(state.runtime.propose(&scope, request.text).await?))
}

async fn remote_changes(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let token = token(&headers)?;
    if matches!(
        state.runtime.authorize(&token).await?,
        AccessScope::Keybindings { .. }
            | AccessScope::Projects
            | AccessScope::ProjectUsage { .. }
            | AccessScope::ProjectUsageHistory { .. }
            | AccessScope::ProjectNames
            | AccessScope::ProjectSchedules { .. }
    ) {
        return Err(CoordinationError::Unauthorized.into());
    }
    Ok(change_stream(state.runtime.clone(), Some(token)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn remote_model_prices_require_an_explicit_edit_grant() {
        use bruhs_models::agent_extensions::UsageModelPricing;
        use serde_json::json;

        let root = tempfile::tempdir().unwrap();
        let workbench = root.path().join("workbench");
        let desktop = root.path().join("desktop.json");
        std::fs::write(&desktop, "{}").unwrap();
        let runtime = CoordinationRuntime::open(
            root.path().join("tasks.json"),
            Arc::new(bruhs_core::AppState::new()),
        )
        .await
        .unwrap();
        let legacy = serde_json::from_value(json!({"kind":"project_usage_history"})).unwrap();
        let reader = runtime.issue_access(legacy, 60).await.unwrap();
        let writer = runtime
            .issue_access(
                AccessScope::ProjectUsageHistory {
                    rename: false,
                    limits: false,
                    edit_prices: true,
                },
                60,
            )
            .await
            .unwrap();
        let delivery = bruhs_core::delivery::DeliveryService::open(root.path().join("delivery"))
            .await
            .unwrap();
        let app = remote_router_with_state(
            runtime.clone(),
            BoardLinks::new(delivery),
            Some(desktop),
            Some(workbench.clone()),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let projects = format!("http://{}/api/projects", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = reqwest::Client::new();
        let url = format!("{projects}/usage/model-prices");
        let request = |token: &str, body: serde_json::Value| {
            client.post(&url).bearer_auth(token).json(&body).send()
        };
        let empty = json!({"price":null,"alias":null});
        let changes = json!([
            {"model":"codex/canonical","expected":empty,"edit":{"price":{
                "input_per_million":3.0,"output_per_million":4.0
            }}},
            {"model":"codex/preview","expected":empty,"edit":{"alias":"codex/canonical"}}
        ]);
        for (grant, can_edit) in [(&reader, "false"), (&writer, "true")] {
            let inventory = client
                .get(&projects)
                .bearer_auth(&grant.token)
                .send()
                .await
                .unwrap();
            assert_eq!(inventory.status(), StatusCode::OK);
            assert_eq!(inventory.headers()["x-bruhs-edit-usage-prices"], can_edit);
        }
        assert_eq!(
            client
                .get(&url)
                .bearer_auth(&reader.token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            request(&reader.token, changes.clone())
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert!(!workbench.join("agents.json").exists());
        for _ in 0..2 {
            assert_eq!(
                request(&writer.token, changes.clone())
                    .await
                    .unwrap()
                    .status(),
                StatusCode::NO_CONTENT
            );
        }
        let read = client
            .get(&url)
            .bearer_auth(&writer.token)
            .send()
            .await
            .unwrap();
        assert_eq!(read.headers()[header::CACHE_CONTROL], "no-store");
        let rows: std::collections::BTreeMap<String, UsageModelPricing> =
            read.json().await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows["codex/preview"].alias.as_deref(),
            Some("codex/canonical")
        );
        let stale = json!([{"model":"codex/preview","expected":empty,"edit":{"alias":"other"}}]);
        assert_eq!(
            request(&writer.token, stale).await.unwrap().status(),
            StatusCode::CONFLICT
        );
        runtime.revoke_access(&writer.grant.id).await.unwrap();
        drop(runtime);
        assert_eq!(
            request(&writer.token, changes).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
        server.abort();
    }

    #[tokio::test]
    #[expect(
        clippy::significant_drop_tightening,
        reason = "test runtime stays alive until the server task is aborted"
    )]
    async fn paired_projects_only_return_saved_checkout_metadata() {
        let root = tempfile::tempdir().unwrap();
        let checkout = root.path().join("source");
        let legacy = root.path().join("legacy");
        std::fs::create_dir(&checkout).unwrap();
        for args in [
            vec!["init", "-q"],
            vec![
                "remote",
                "add",
                "origin",
                "https://bob:secret@example.org/group/app.git?q=private",
            ],
        ] {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(&checkout)
                .status()
                .unwrap()
                .success());
        }
        let saved = root.path().join("desktop.json");
        std::fs::write(&saved, serde_json::to_vec(&serde_json::json!({
            "project_list":{"entries":[{"name":"Workbench","path":checkout}],"titles":{(checkout.to_str().unwrap()):"Studio editor","/private-folder":"Private name"}},
            "recent_projects":[{"name":"Duplicated","path":checkout}],
            "threads":[
                {"project_name":"Legacy","project_settings_root":legacy,"project_path":"/private-worktree","messages":["private message"]},
                {"project_name":"","project_path":"/private-chat"},
            ],
        })).unwrap()).unwrap();
        let runtime = CoordinationRuntime::open(
            root.path().join("tasks.json"),
            Arc::new(bruhs_core::AppState::new()),
        )
        .await
        .unwrap();
        let access = runtime
            .issue_access(AccessScope::Projects, 60)
            .await
            .unwrap();
        let writer = runtime
            .issue_access(AccessScope::ProjectNames, 60)
            .await
            .unwrap();
        let other = runtime
            .issue_access(AccessScope::Keybindings { editable: true }, 60)
            .await
            .unwrap();
        let delivery = bruhs_core::delivery::DeliveryService::open(root.path().join("delivery"))
            .await
            .unwrap();
        let reactive =
            bruhs_core::reactive::ReactiveRuntime::open(root.path().join("automations.db"))
                .await
                .unwrap();
        let workbench = root.path().join("workbench");
        bruhs_core::agent_extensions::record_at(
            &workbench,
            bruhs_models::agent_extensions::UsageRecord {
                id: "private-receipt".into(),
                thread_id: "private-thread".into(),
                session_id: Some("private-session".into()),
                model: "gpt-5.5".into(),
                project: "private-project".into(),
                at: chrono::Utc::now().timestamp(),
                input: 25,
                output: 8,
                cache_read: 0,
                cache_write: 0,
                requested_speed: Default::default(),
                cli_cost_usd: None,
            },
        )
        .unwrap();
        let app = remote_router_with_state(
            runtime.clone(),
            BoardLinks::new(delivery),
            Some(saved),
            Some(workbench),
        )
        .layer(axum::Extension(reactive));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/api", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = reqwest::Client::new();
        let url = format!("{base}/projects");
        let schedules_url = format!("{url}/scheduled-tasks");
        for old in [&access.token, &writer.token, &other.token] {
            assert_eq!(
                client
                    .get(&schedules_url)
                    .bearer_auth(old)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
        let editor_url = format!("{base}/projects/task-editor/missing");
        for restricted in [&access.token, &writer.token, &other.token] {
            assert_eq!(
                client
                    .get(&editor_url)
                    .bearer_auth(restricted)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
        let editor = runtime
            .issue_access(
                AccessScope::ProjectSchedules {
                    rename: false,
                    toggle: false,
                    run: false,
                    delete: false,
                    deliveries: false,
                    usage: false,
                    usage_history: false,
                    edit_usage_prices: false,
                    edit: true,
                    conversations: false,
                    read_threads: false,
                    reply_threads: false,
                },
                60,
            )
            .await
            .unwrap();
        assert_eq!(
            client
                .get(&url)
                .bearer_auth(&editor.token)
                .send()
                .await
                .unwrap()
                .headers()["x-bruhs-edit-scheduled-tasks"],
            "true"
        );
        assert_eq!(
            client
                .get(&editor_url)
                .bearer_auth(&editor.token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            client
                .get(format!("{url}/task-models"))
                .query(&[("path", "/private-folder")])
                .bearer_auth(&editor.token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
        let search = format!("{url}/task-conversations");
        let thread_url = format!("{schedules_url}/missing/thread/owner-thread");
        assert_eq!(
            client
                .get(&thread_url)
                .bearer_auth(&editor.token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        let reader = runtime
            .issue_access(
                AccessScope::ProjectSchedules {
                    rename: false,
                    toggle: false,
                    run: false,
                    delete: false,
                    deliveries: false,
                    usage: false,
                    usage_history: false,
                    edit_usage_prices: false,
                    edit: false,
                    conversations: false,
                    read_threads: true,
                    reply_threads: false,
                },
                60,
            )
            .await
            .unwrap();
        assert_eq!(
            client
                .get(&thread_url)
                .bearer_auth(&reader.token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            client
                .post(&thread_url)
                .bearer_auth(&reader.token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        let project = checkout.to_str().unwrap();
        assert_eq!(
            client
                .get(&search)
                .query(&[("path", project)])
                .bearer_auth(&editor.token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        let picker = runtime
            .issue_access(
                AccessScope::ProjectSchedules {
                    rename: false,
                    toggle: false,
                    run: false,
                    delete: false,
                    deliveries: false,
                    usage: false,
                    usage_history: false,
                    edit_usage_prices: false,
                    edit: true,
                    conversations: true,
                    read_threads: false,
                    reply_threads: false,
                },
                60,
            )
            .await
            .unwrap();
        assert_eq!(
            client
                .get(&url)
                .bearer_auth(&picker.token)
                .send()
                .await
                .unwrap()
                .headers()["x-bruhs-task-conversations"],
            "true"
        );
        let private = client
            .get(&search)
            .query(&[("path", "/private-folder")])
            .bearer_auth(&picker.token)
            .send()
            .await
            .unwrap();
        assert_eq!(private.status(), StatusCode::BAD_REQUEST);
        runtime.revoke_access(&picker.grant.id).await.unwrap();
        assert_eq!(
            client
                .get(&search)
                .query(&[("path", project)])
                .bearer_auth(&picker.token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        runtime.revoke_access(&editor.grant.id).await.unwrap();
        assert_eq!(
            client
                .get(&editor_url)
                .bearer_auth(&editor.token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        let usage_url = format!("{url}/usage");
        let history_url = format!("{usage_url}/history");
        let get_history = |token: &str, days: u16| {
            client
                .get(&history_url)
                .query(&[("days", days)])
                .bearer_auth(token)
                .send()
        };
        for token in [&access.token, &writer.token, &other.token] {
            assert_eq!(
                client
                    .get(&usage_url)
                    .bearer_auth(token)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
        for rename in [false, true] {
            let grant = runtime
                .issue_access(AccessScope::ProjectUsage { rename }, 60)
                .await
                .unwrap();
            let inventory = client
                .get(&url)
                .bearer_auth(&grant.token)
                .send()
                .await
                .unwrap();
            assert_eq!(inventory.headers()["x-bruhs-read-usage"], "true");
            assert_eq!(inventory.headers()["x-bruhs-read-usage-history"], "false");
            assert_eq!(
                get_history(&grant.token, 30).await.unwrap().status(),
                StatusCode::FORBIDDEN
            );
            assert_eq!(
                inventory.headers()["x-bruhs-edit-project-names"],
                rename.to_string().as_str()
            );
            assert_eq!(inventory.headers()["x-bruhs-read-scheduled-tasks"], "false");
            for route in [
                &schedules_url,
                &format!("{base}/keybindings"),
                &format!("{base}/tasks"),
            ] {
                assert_eq!(
                    client
                        .get(route)
                        .bearer_auth(&grant.token)
                        .send()
                        .await
                        .unwrap()
                        .status(),
                    StatusCode::FORBIDDEN
                );
            }
            runtime.revoke_access(&grant.grant.id).await.unwrap();
            assert_eq!(
                client
                    .get(&usage_url)
                    .bearer_auth(&grant.token)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
        let history_grant = runtime
            .issue_access(
                AccessScope::ProjectUsageHistory {
                    rename: false,
                    limits: false,
                    edit_prices: false,
                },
                60,
            )
            .await
            .unwrap();
        let inventory = client
            .get(&url)
            .bearer_auth(&history_grant.token)
            .send()
            .await
            .unwrap();
        assert_eq!(inventory.headers()["x-bruhs-read-usage"], "false");
        assert_eq!(inventory.headers()["x-bruhs-read-usage-history"], "true");
        assert_eq!(
            get_history(&history_grant.token, 91)
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
        let history = get_history(&history_grant.token, 30).await.unwrap();
        assert_eq!(history.status(), StatusCode::OK);
        assert_eq!(history.headers()[header::CACHE_CONTROL], "no-store");
        let history = history.text().await.unwrap();
        assert!(!history.contains("private-"));
        let history: bruhs_models::agent_extensions::SharedUsageHistory =
            serde_json::from_str(&history).unwrap();
        assert_eq!(history.by_model["gpt-5.5"].input, 25);
        assert_eq!(history.breakdown.sessions_by_provider["codex"], 1);
        let hourly = client
            .get(&history_url)
            .query(&bruhs_models::agent_extensions::UsageHistoryPeriod {
                days: None,
                last_24h_ending_at: Some(chrono::Utc::now().timestamp() + 1),
                provider: Some(bruhs_models::agent::AgentProvider::Codex),
            })
            .bearer_auth(&history_grant.token)
            .send()
            .await
            .unwrap();
        assert_eq!(hourly.status(), StatusCode::OK);
        let hourly: bruhs_models::agent_extensions::SharedUsageHistory =
            hourly.json().await.unwrap();
        assert!(hourly.by_day.is_empty());
        assert_eq!(
            hourly.by_hour.values().map(|hour| hour.input).sum::<u64>(),
            25
        );
        assert_eq!(
            client
                .get(&usage_url)
                .bearer_auth(&history_grant.token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        runtime
            .revoke_access(&history_grant.grant.id)
            .await
            .unwrap();
        assert_eq!(
            get_history(&history_grant.token, 30)
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        let run_url = format!("{schedules_url}/missing/runs/{}", uuid::Uuid::new_v4());
        let run_request = |token: &str| client.put(&run_url).bearer_auth(token).send();
        let toggle_request = |token: &str| {
            client
                .put(format!("{schedules_url}/missing/enabled"))
                .bearer_auth(token)
                .json(&serde_json::json!({"enabled":true}))
                .send()
        };
        for old in [&access.token, &writer.token, &other.token] {
            assert_eq!(
                toggle_request(old).await.unwrap().status(),
                StatusCode::FORBIDDEN
            );
            assert_eq!(
                run_request(old).await.unwrap().status(),
                StatusCode::FORBIDDEN
            );
        }
        let delete_request = |token: &str| {
            client
                .delete(format!("{schedules_url}/missing"))
                .bearer_auth(token)
                .send()
        };
        for (rename, toggle, run, delete, deliveries) in [
            (false, false, false, false, false),
            (true, false, false, false, false),
            (false, true, false, false, false),
            (true, true, false, false, false),
            (false, false, true, false, false),
            (true, true, true, false, false),
            (false, false, false, true, false),
            (true, true, true, true, false),
            (false, false, false, false, true),
            (true, true, true, true, true),
        ] {
            let grant = runtime
                .issue_access(
                    AccessScope::ProjectSchedules {
                        rename,
                        toggle,
                        run,
                        delete,
                        deliveries,
                        usage: false,
                        usage_history: false,
                        edit_usage_prices: false,
                        edit: false,
                        conversations: false,
                        read_threads: false,
                        reply_threads: false,
                    },
                    60,
                )
                .await
                .unwrap();
            let response = client
                .get(&url)
                .bearer_auth(&grant.token)
                .send()
                .await
                .unwrap();
            assert_eq!(response.headers()["x-bruhs-edit-scheduled-tasks"], "false");
            assert_eq!(
                client
                    .get(&editor_url)
                    .bearer_auth(&grant.token)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::FORBIDDEN
            );
            assert_eq!(response.headers()["x-bruhs-read-usage"], "false");
            assert_eq!(response.headers()["x-bruhs-read-scheduled-tasks"], "true");
            assert_eq!(
                response.headers()["x-bruhs-toggle-scheduled-tasks"],
                toggle.to_string().as_str()
            );
            assert_eq!(
                response.headers()["x-bruhs-run-scheduled-tasks"],
                run.to_string().as_str()
            );
            assert_eq!(
                response.headers()["x-bruhs-webhook-deliveries"],
                deliveries.to_string().as_str()
            );
            for suffix in ["deliveries", "deliveries/one"] {
                let response = client
                    .get(format!("{schedules_url}/missing/{suffix}"))
                    .bearer_auth(&grant.token)
                    .send()
                    .await
                    .unwrap();
                assert_eq!(
                    response.status(),
                    if deliveries {
                        StatusCode::NOT_FOUND
                    } else {
                        StatusCode::FORBIDDEN
                    }
                );
            }
            assert_eq!(
                response.headers()["x-bruhs-delete-scheduled-tasks"],
                delete.to_string().as_str()
            );
            assert_eq!(
                delete_request(&grant.token).await.unwrap().status(),
                if delete {
                    StatusCode::NOT_FOUND
                } else {
                    StatusCode::FORBIDDEN
                }
            );
            assert_eq!(
                run_request(&grant.token).await.unwrap().status(),
                if run {
                    StatusCode::NOT_FOUND
                } else {
                    StatusCode::FORBIDDEN
                }
            );
            assert_eq!(
                toggle_request(&grant.token).await.unwrap().status(),
                if toggle {
                    StatusCode::NOT_FOUND
                } else {
                    StatusCode::FORBIDDEN
                }
            );
            assert_eq!(
                response.headers()["x-bruhs-edit-project-names"],
                if rename { "true" } else { "false" }
            );
            let response = client
                .get(&schedules_url)
                .bearer_auth(&grant.token)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response
                    .json::<Vec<bruhs_models::reactive::ScheduledTaskSummary>>()
                    .await
                    .unwrap(),
                vec![]
            );
            for endpoint in ["tasks", "changes", "keybindings"] {
                assert_eq!(
                    client
                        .get(format!("{base}/{endpoint}"))
                        .bearer_auth(&grant.token)
                        .send()
                        .await
                        .unwrap()
                        .status(),
                    StatusCode::FORBIDDEN
                );
            }
            runtime.revoke_access(&grant.grant.id).await.unwrap();
            assert_eq!(
                client
                    .get(format!("{schedules_url}/missing/deliveries/one"))
                    .bearer_auth(&grant.token)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::FORBIDDEN
            );
            assert_eq!(
                delete_request(&grant.token).await.unwrap().status(),
                StatusCode::FORBIDDEN
            );
            assert_eq!(
                run_request(&grant.token).await.unwrap().status(),
                StatusCode::FORBIDDEN
            );
            assert_eq!(
                toggle_request(&grant.token).await.unwrap().status(),
                StatusCode::FORBIDDEN
            );
            assert_eq!(
                client
                    .get(&schedules_url)
                    .bearer_auth(&grant.token)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::FORBIDDEN
            );
        }

        let response = client
            .get(&url)
            .bearer_auth(&access.token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-bruhs-edit-project-names"], "false");
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(
            body,
            serde_json::json!([
                {"name":"Legacy","path":legacy,"identity":null},
                {"name":"Studio editor","title":"Studio editor","path":checkout,"identity":{"label":"example.org/group/app","repository":"remote:example.org/group/app","relative_path":""}}
            ])
        );
        for path in ["tasks", "changes", "board", "keybindings"] {
            assert_eq!(
                client
                    .get(format!("{base}/{path}"))
                    .bearer_auth(&access.token)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
        assert_eq!(
            client
                .post(&url)
                .bearer_auth(&access.token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
        let read = |bearer: &str| client.get(&url).bearer_auth(bearer).send();
        let post = |bearer: &str, expected: serde_json::Value, title: Option<&str>| {
            client
                .post(format!("{url}/names"))
                .bearer_auth(bearer)
                .json(&serde_json::json!({"expected": expected, "title": title}))
                .send()
        };
        let expected = serde_json::json!({(checkout.to_str().unwrap()):"Studio editor",(legacy.to_str().unwrap()):null});
        assert_eq!(
            read(&writer.token).await.unwrap().headers()["x-bruhs-edit-project-names"],
            "true"
        );
        for token in [&access.token, &other.token] {
            assert_eq!(
                post(token, expected.clone(), Some("Editor"))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
        for path in ["tasks", "keybindings", "changes"] {
            assert_eq!(
                client
                    .get(format!("{base}/{path}"))
                    .bearer_auth(&writer.token)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
        for _ in 0..2 {
            assert_eq!(
                post(&writer.token, expected.clone(), Some("Editor"))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::NO_CONTENT
            );
        }
        let stale = serde_json::json!({(checkout.to_str().unwrap()):"Studio editor",(legacy.to_str().unwrap()):"Editor"});
        assert_eq!(
            post(&writer.token, stale, Some("New"))
                .await
                .unwrap()
                .status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            post(
                &writer.token,
                serde_json::json!({"/private-folder":null}),
                Some("Private")
            )
            .await
            .unwrap()
            .status(),
            StatusCode::NOT_FOUND
        );
        let renamed: Vec<bruhs_core::linked_projects::Checkout> =
            read(&access.token).await.unwrap().json().await.unwrap();
        assert_eq!(renamed.len(), 2);
        assert!(renamed
            .iter()
            .all(|p| p.name == "Editor" && p.title.as_deref() == Some("Editor")));
        let both = serde_json::json!({(checkout.to_str().unwrap()):"Editor",(legacy.to_str().unwrap()):"Editor"});
        assert_eq!(
            post(&writer.token, both, None).await.unwrap().status(),
            StatusCode::NO_CONTENT
        );
        let reset: Vec<bruhs_core::linked_projects::Checkout> =
            read(&access.token).await.unwrap().json().await.unwrap();
        assert_eq!(
            reset.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            ["Legacy", "Workbench"]
        );
        assert!(reset.iter().all(|p| p.title.is_none()));
        runtime.revoke_access(&writer.grant.id).await.unwrap();
        assert_eq!(
            post(&writer.token, expected, Some("Editor"))
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        runtime.revoke_access(&access.grant.id).await.unwrap();
        for token in [&access.token, &other.token] {
            assert_eq!(
                client
                    .get(&url)
                    .bearer_auth(token)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
        server.abort();
    }

    #[tokio::test]
    #[expect(
        clippy::significant_drop_tightening,
        reason = "test runtime stays alive until the worker task is aborted"
    )]
    async fn paired_shortcut_grants_preserve_local_rules_and_enforce_access() {
        use bruhs_models::logic::keybindings::Keybinding;
        let root = tempfile::tempdir().unwrap();
        let runtime = CoordinationRuntime::open(
            root.path().join("tasks.json"),
            Arc::new(bruhs_core::AppState::new()),
        )
        .await
        .unwrap();
        let owner = runtime.issue_access(AccessScope::Owner, 60).await.unwrap();
        let read = runtime
            .issue_access(AccessScope::Keybindings { editable: false }, 60)
            .await
            .unwrap();
        let write = runtime
            .issue_access(AccessScope::Keybindings { editable: true }, 60)
            .await
            .unwrap();
        let delivery = bruhs_core::delivery::DeliveryService::open(root.path().join("delivery"))
            .await
            .unwrap();
        let saved = root.path().join("state.json");
        let original = Keybinding {
            command: "settings.open".into(),
            key: "alt+8".into(),
            when: None,
        };
        std::fs::write(
            &saved,
            serde_json::to_vec(
                &serde_json::json!({"keybindings":[original],"keybinding_schema":1}),
            )
            .unwrap(),
        )
        .unwrap();
        let app = remote_router_with_state(
            runtime.clone(),
            BoardLinks::new(delivery),
            Some(saved.clone()),
            None,
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let worker = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let http = reqwest::Client::new();
        let url = format!("{base}/api/keybindings");
        let snapshot = http
            .get(&url)
            .bearer_auth(&read.token)
            .send()
            .await
            .unwrap()
            .json::<EnvironmentKeybindings>()
            .await
            .unwrap();
        assert!(!snapshot.editable && snapshot.rules.contains(&original));
        let changed = Keybinding {
            key: "alt+9".into(),
            ..original.clone()
        };
        let edit = KeybindingEdit::Upsert {
            rule: changed.clone(),
            replace: Some(original.clone()),
        };
        for bearer in [&read.token, &owner.token] {
            assert_eq!(
                http.post(&url)
                    .bearer_auth(bearer)
                    .json(&edit)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::FORBIDDEN
            );
            assert_eq!(
                http.post(format!("{url}/open"))
                    .bearer_auth(bearer)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
        for path in ["tasks", "changes", "board"] {
            assert_eq!(
                http.get(format!("{base}/api/{path}"))
                    .bearer_auth(&write.token)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
        for _ in 0..2 {
            let saved = http
                .post(&url)
                .bearer_auth(&write.token)
                .json(&edit)
                .send()
                .await
                .unwrap()
                .json::<EnvironmentKeybindings>()
                .await
                .unwrap();
            assert!(saved.editable && !saved.rules.contains(&original));
            assert_eq!(saved.rules.iter().filter(|r| *r == &changed).count(), 1);
        }
        let disk = saved.with_file_name("keybindings.json");
        let saved = std::fs::read(&disk).unwrap();
        assert!(serde_json::from_slice::<Vec<Keybinding>>(&saved)
            .unwrap()
            .contains(&changed));
        runtime.revoke_access(&write.grant.id).await.unwrap();
        let remove = KeybindingEdit::Remove { rule: changed };
        assert_eq!(
            http.post(&url)
                .bearer_auth(&write.token)
                .json(&remove)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(std::fs::read(disk).unwrap(), saved);
        worker.abort();
    }

    #[tokio::test]
    async fn remote_task_api_requires_auth_and_has_no_personal_or_credential_routes() {
        let root = tempfile::tempdir().unwrap();
        let runtime = CoordinationRuntime::open(
            root.path().join("tasks.json"),
            Arc::new(bruhs_core::AppState::new()),
        )
        .await
        .unwrap();
        let access = runtime.issue_access(AccessScope::Owner, 60).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let delivery = bruhs_core::delivery::DeliveryService::open(root.path().join("delivery"))
            .await
            .unwrap();
        let app = remote_router(runtime.clone(), BoardLinks::new(delivery));
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = reqwest::Client::new();
        let manifest = client
            .get(format!("{base}/manifest.webmanifest"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            manifest.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/manifest+json"
        );
        assert_eq!(
            manifest.json::<serde_json::Value>().await.unwrap()["display"],
            "standalone"
        );
        let icon = client
            .get(format!("{base}/icon-512.png"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            icon.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/png"
        );
        assert_eq!(
            client
                .get(format!("{base}/api/tasks"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            client
                .get(format!("{base}/api/tasks"))
                .bearer_auth(&access.token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            client
                .get(format!("{base}/api/tasks"))
                .bearer_auth(&access.token)
                .header(header::ORIGIN, "https://attacker.example")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            client
                .get(format!("{base}/api/board"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        let board = client
            .get(format!("{base}/api/board"))
            .bearer_auth(&access.token)
            .send()
            .await
            .unwrap();
        assert_eq!(board.status(), StatusCode::OK);
        let board: TaskBoard = board.json().await.unwrap();
        assert!(board.cards.is_empty() && !board.refreshing);
        for path in [
            "/api/meetings",
            "/api/secrets",
            "/api/coordination/access",
            "/api/agent",
        ] {
            assert_eq!(
                client
                    .get(format!("{base}{path}"))
                    .bearer_auth(&access.token)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::NOT_FOUND
            );
        }
        runtime.revoke_access(&access.grant.id).await.unwrap();
        drop(runtime);
        assert_eq!(
            client
                .get(format!("{base}/api/tasks"))
                .bearer_auth(&access.token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        server.abort();
    }
}
