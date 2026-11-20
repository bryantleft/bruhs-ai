//! Local control and a separate, task-only, authenticated self-hosted surface.
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
                .issue_access(request.scope, request.lifetime_seconds)
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

#[derive(Clone)]
struct RemoteState {
    runtime: CoordinationRuntime,
    links: BoardLinks,
}

/// Bind this router only after explicit self-hosted access configuration. It
/// contains no life, capture, file, credential, or ordinary-agent API routes.
pub fn remote_router(runtime: CoordinationRuntime, links: BoardLinks) -> Router {
    Router::new().route("/", get(|| async { Html(include_str!("coordination/remote.html")) }))
        .route("/favicon.ico", get(|| async { StatusCode::NO_CONTENT }))
        .route("/manifest.webmanifest", get(|| async {
            ([(header::CONTENT_TYPE, "application/manifest+json")], r##"{"id":"/","name":"Bruhs tasks","short_name":"Bruhs","start_url":"/","scope":"/","display":"standalone","background_color":"#141210","theme_color":"#141210","icons":[{"src":"/icon-512.png","sizes":"512x512","type":"image/png","purpose":"any"}]}"##)
        }))
        .route("/icon-512.png", get(|| async {
            ([(header::CONTENT_TYPE, "image/png")], include_bytes!("coordination/icon-512.png").as_slice())
        }))
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
        .with_state(Arc::new(RemoteState { runtime, links }))
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
    state.runtime.authorize(&token).await?;
    Ok(change_stream(state.runtime.clone(), Some(token)))
}

#[cfg(test)]
mod tests {
    use super::*;

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
