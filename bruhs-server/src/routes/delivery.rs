//! The local delivery API shares one durable service across every connected surface.

use axum::{
    extract::{Path, State},
    response::Html,
    routing::{delete, get, post},
    Json, Router,
};
use bruhs_core::delivery::{DeliveryError, DeliveryService};
use bruhs_models::delivery::*;

use crate::{error::AppError, routes::paths::HomePath};

pub fn router(service: DeliveryService) -> Router {
    Router::new()
        .route("/api/delivery/ui", get(ui))
        .route("/api/delivery", get(snapshot))
        .route("/api/delivery/read", post(read))
        .route("/api/delivery/trackers/read", post(tracker_settings))
        .route("/api/delivery/trackers", post(configure_tracker))
        .route("/api/delivery/prepare", post(prepare))
        .route("/api/delivery/execute", post(execute))
        .route(
            "/api/delivery/execute-native-stack",
            post(execute_native_stack),
        )
        .route(
            "/api/delivery/native-stack-status",
            post(native_stack_status),
        )
        .route("/api/delivery/cancel", post(cancel))
        .route("/api/delivery/watches", post(watch))
        .route("/api/delivery/watches/{id}", delete(remove_watch))
        .route("/api/delivery/events/{id}/acknowledge", post(acknowledge))
        .with_state(service)
}

async fn ui() -> Html<&'static str> {
    Html(include_str!("delivery_ui.html"))
}

fn map_error(error: DeliveryError) -> AppError {
    match error {
        DeliveryError::Invalid(message) => AppError::BadRequest(message),
        DeliveryError::Conflict(message) => AppError::Conflict(message),
        DeliveryError::NotFound(message) => AppError::NotFound(message),
        DeliveryError::Command(message) => AppError::Unavailable(message),
        error @ DeliveryError::Storage(_) => AppError::internal(error),
    }
}

async fn snapshot(State(service): State<DeliveryService>) -> Json<DeliverySnapshot> {
    Json(service.snapshot().await)
}

async fn read(
    State(service): State<DeliveryService>,
    Json(request): Json<ReadRequest>,
) -> Result<Json<ReadResult>, AppError> {
    let path = HomePath::resolve(&request.path).await?;
    let result = match request.query {
        DeliveryQuery::Pulls => {
            let pulls = service
                .pulls_for(path.as_path(), request.forge)
                .await
                .map_err(map_error)?;
            let stacks = stack_links(&pulls);
            ReadResult::Pulls { pulls, stacks }
        }
        DeliveryQuery::Pull { number } => ReadResult::Pull {
            pull: Box::new(
                service
                    .pull_for(path.as_path(), number, request.forge)
                    .await
                    .map_err(map_error)?,
            ),
        },
        DeliveryQuery::Diff { number } => ReadResult::Diff {
            patch: service
                .diff_for(path.as_path(), number, request.forge)
                .await
                .map_err(map_error)?,
        },
        DeliveryQuery::LineComments { number } => ReadResult::LineComments {
            comments: service
                .comments_for(path.as_path(), number, request.forge)
                .await
                .map_err(map_error)?,
        },
        DeliveryQuery::Issue { number } => ReadResult::Issue {
            issue: service
                .issue_for(path.as_path(), number, request.forge)
                .await
                .map_err(map_error)?,
        },
        DeliveryQuery::TrackerIssue {
            provider,
            reference,
        } => ReadResult::Issue {
            issue: service
                .tracker_issue(path.as_path(), provider, &reference)
                .await
                .map_err(map_error)?,
        },
        DeliveryQuery::Scripts => ReadResult::Scripts {
            scripts: service.scripts(path.as_path()).await.map_err(map_error)?,
        },
    };
    Ok(Json(result))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackerPath {
    path: String,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackerConfiguration {
    path: String,
    config: TrackerConfig,
    token: Option<String>,
}

async fn tracker_settings(
    State(service): State<DeliveryService>,
    Json(request): Json<TrackerPath>,
) -> Result<Json<Vec<TrackerSetting>>, AppError> {
    let path = HomePath::resolve(&request.path).await?;
    service
        .tracker_settings(path.as_path())
        .await
        .map(Json)
        .map_err(map_error)
}
async fn configure_tracker(
    State(service): State<DeliveryService>,
    Json(request): Json<TrackerConfiguration>,
) -> Result<Json<Vec<TrackerSetting>>, AppError> {
    let path = HomePath::resolve(&request.path).await?;
    service
        .configure_tracker(
            path.as_path(),
            TrackerSetup {
                config: request.config,
                token: request.token,
            },
        )
        .await
        .map(Json)
        .map_err(map_error)
}

async fn prepare(
    State(service): State<DeliveryService>,
    Json(mut request): Json<PrepareRequest>,
) -> Result<Json<PreparedAction>, AppError> {
    request.path = HomePath::resolve(&request.path)
        .await?
        .into_path_buf()
        .to_string_lossy()
        .into_owned();
    service.prepare(request).await.map(Json).map_err(map_error)
}

async fn execute(
    State(service): State<DeliveryService>,
    Json(request): Json<ActionId>,
) -> Result<Json<ActionReceipt>, AppError> {
    service
        .execute(&request.id)
        .await
        .map(Json)
        .map_err(map_error)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeStackApproval {
    id: String,
    acknowledge_server_heads: bool,
}
async fn execute_native_stack(
    State(service): State<DeliveryService>,
    Json(request): Json<NativeStackApproval>,
) -> Result<Json<ActionReceipt>, AppError> {
    service
        .execute_native_stack(&request.id, request.acknowledge_server_heads)
        .await
        .map(Json)
        .map_err(map_error)
}
async fn native_stack_status(
    State(service): State<DeliveryService>,
    Json(request): Json<ActionId>,
) -> Result<Json<ActionReceipt>, AppError> {
    service
        .refresh_native_stack(&request.id)
        .await
        .map(Json)
        .map_err(map_error)
}

async fn cancel(
    State(service): State<DeliveryService>,
    Json(request): Json<ActionId>,
) -> Result<Json<DeliverySnapshot>, AppError> {
    service.cancel(&request.id).await.map_err(map_error)?;
    Ok(Json(service.snapshot().await))
}

async fn watch(
    State(service): State<DeliveryService>,
    Json(mut request): Json<WatchRequest>,
) -> Result<Json<PullWatch>, AppError> {
    request.path = HomePath::resolve(&request.path)
        .await?
        .into_path_buf()
        .to_string_lossy()
        .into_owned();
    service
        .watch_pull(request)
        .await
        .map(Json)
        .map_err(map_error)
}

async fn remove_watch(
    State(service): State<DeliveryService>,
    Path(id): Path<String>,
) -> Result<Json<DeliverySnapshot>, AppError> {
    service.remove_watch(&id).await.map_err(map_error)?;
    Ok(Json(service.snapshot().await))
}

async fn acknowledge(
    State(service): State<DeliveryService>,
    Path(id): Path<String>,
) -> Result<Json<DeliverySnapshot>, AppError> {
    service.acknowledge_event(&id).await.map_err(map_error)?;
    Ok(Json(service.snapshot().await))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[tokio::test]
    async fn repository_reads_and_actions_reject_paths_outside_home_before_running_clis() {
        let directory = tempfile::tempdir().unwrap();
        let service = DeliveryService::open(directory.path()).await.unwrap();
        let error = read(
            State(service.clone()),
            Json(ReadRequest {
                forge: Forge::GitHub,
                path: "/tmp".into(),
                query: DeliveryQuery::Pulls,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(error.status(), StatusCode::FORBIDDEN);
        let error = prepare(
            State(service.clone()),
            Json(PrepareRequest {
                forge: Forge::GitHub,
                path: "/tmp".into(),
                action: DeliveryAction::Comment {
                    number: 1,
                    body: "review".into(),
                },
                timeout_seconds: 5,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(error.status(), StatusCode::FORBIDDEN);
        assert!(service.snapshot().await.actions.is_empty());
    }

    #[tokio::test]
    async fn an_unknown_or_client_invented_action_id_cannot_execute() {
        let directory = tempfile::tempdir().unwrap();
        let service = DeliveryService::open(directory.path()).await.unwrap();
        let error = execute(
            State(service),
            Json(ActionId {
                id: "invented-by-client".into(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(error.status(), StatusCode::NOT_FOUND);
    }
}
