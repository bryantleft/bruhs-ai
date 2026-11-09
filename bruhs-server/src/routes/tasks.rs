use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    Json,
};
use bruhs_core::{AppState, BackgroundTask, BackgroundTaskRegistry, RemoveTaskError};
use futures_core::Stream;
use serde::Deserialize;
use std::convert::Infallible;
use std::sync::Arc;

use crate::error::AppError;

#[derive(Deserialize)]
pub struct ThreadQuery {
    thread_id: Option<String>,
}

/// `GET /api/tasks?thread_id=<id>`
/// Returns background tasks, optionally filtered by thread.
pub async fn get_tasks(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ThreadQuery>,
) -> Json<Vec<BackgroundTask>> {
    let tasks = match &query.thread_id {
        Some(id) => state.registry().get_by_thread(id),
        None => state.registry().get_all(),
    };
    Json(tasks)
}

/// DELETE /api/tasks/:id — removes a finished task: 204, 404 for an unknown
/// id, 409 while the task is still running.
pub async fn delete_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    match state.registry().remove(&id) {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(error @ RemoveTaskError::StillRunning(_)) => Err(AppError::Conflict(error.to_string())),
        Err(error) => Err(AppError::NotFound(error.to_string())),
    }
}

/// GET /api/tasks/stream — the full task list as one JSON SSE event, sent
/// on connect and again after every change (not on a timer).
pub async fn get_tasks_stream(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    Sse::new(task_snapshots(state.registry().clone())).keep_alive(KeepAlive::default())
}

fn task_snapshots(
    registry: Arc<BackgroundTaskRegistry>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    let mut changes = registry.watch_changes();
    async_stream::stream! {
        loop {
            // Mark the current version seen before reading, so a change made
            // while this snapshot is built triggers another one.
            changes.borrow_and_update();
            match Event::default().json_data(registry.get_all()) {
                Ok(event) => yield Ok(event),
                // Never send an empty frame the client cannot parse.
                Err(error) => tracing::error!("task snapshot not serializable: {error}"),
            }
            if changes.changed().await.is_err() {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_core::TaskKind;
    use tokio_stream::StreamExt;

    #[tokio::test]
    async fn deleting_a_running_task_is_a_conflict() {
        let state = Arc::new(AppState::new());
        let task =
            state
                .registry()
                .register("thread", TaskKind::Shell, "sleep", serde_json::json!({}));
        assert!(matches!(
            delete_task(State(state.clone()), Path(task.id.clone())).await,
            Err(AppError::Conflict(_))
        ));
        assert!(matches!(
            delete_task(State(state.clone()), Path("missing".into())).await,
            Err(AppError::NotFound(_))
        ));
        state.registry().mark_stream_ended("thread");
        assert_eq!(
            delete_task(State(state), Path(task.id)).await.unwrap(),
            StatusCode::NO_CONTENT
        );
    }

    /// One snapshot on connect, then one per change; nothing while idle
    /// (the stream used to resend the whole registry every 500 ms).
    #[tokio::test(start_paused = true)]
    async fn snapshots_are_pushed_on_change_only() {
        let registry = Arc::new(BackgroundTaskRegistry::new());
        let mut stream = std::pin::pin!(task_snapshots(registry.clone()));
        assert!(stream.next().await.is_some(), "initial snapshot");
        let idle = tokio::time::timeout(std::time::Duration::from_secs(60), stream.next()).await;
        assert!(idle.is_err(), "no snapshot without a change");

        registry.register("thread", TaskKind::Shell, "echo", serde_json::json!({}));
        assert!(stream.next().await.is_some(), "snapshot after a change");
        let idle = tokio::time::timeout(std::time::Duration::from_secs(60), stream.next()).await;
        assert!(idle.is_err());
    }
}
