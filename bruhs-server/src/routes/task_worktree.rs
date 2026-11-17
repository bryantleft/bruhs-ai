//! A task's own worktree for the task panel: its state, whether it can be
//! removed, and (on request) what removing it frees by the configured meter.
//! Removal itself is the `remove_worktree` task command.
use crate::error::AppError;
use axum::{
    extract::{Path, Query},
    http::header,
    response::IntoResponse,
    Extension, Json,
};
use bruhs_core::coordination::CoordinationRuntime;
use bruhs_models::ids::ThreadId;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct WorktreeQuery {
    /// Measuring walks the whole folder, so it only runs when asked.
    #[serde(default)]
    measure: bool,
}

pub async fn get(
    runtime: Option<Extension<CoordinationRuntime>>,
    Path(id): Path<ThreadId>,
    Query(query): Query<WorktreeQuery>,
) -> Result<impl IntoResponse, AppError> {
    let Some(Extension(runtime)) = runtime else {
        return Err(AppError::Unavailable(
            "Task workspaces are unavailable. Restart the companion server.".into(),
        ));
    };
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(runtime.task_worktree(&id, query.measure).await?),
    ))
}
