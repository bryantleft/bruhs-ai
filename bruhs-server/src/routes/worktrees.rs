//! Workbench, Worktrees: every worktree bruhs created on this machine (chat,
//! Workbench tasks, issue work), what removing each frees, and removing,
//! starting again or forgetting them. A worktree a task owns goes through
//! the task's `remove_worktree` command, so task rules still apply.
use crate::error::AppError;
use axum::{
    extract::Query,
    http::header,
    response::{Html, IntoResponse},
    Extension, Json,
};
use bruhs_core::coordination::CoordinationRuntime;
use bruhs_core::worktrees as host;
use bruhs_models::coordination::worktree::{
    ManagedWorktree, WorktreeFrees, WorktreeState, WorktreeTask,
};
use bruhs_models::coordination::CoordinationCommand;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub async fn ui() -> impl IntoResponse {
    (
        [(header::CACHE_CONTROL, "no-store")],
        Html(include_str!("worktrees.html")),
    )
}

fn root_for(path: &str) -> Result<PathBuf, AppError> {
    host::locations::for_path(Path::new(path))
        .map_err(|e| AppError::Unavailable(e.to_string()))?
        .ok_or_else(|| {
            AppError::BadRequest("Choose a worktree in a registered bruhs folder.".into())
        })
}

/// A folder named in a request, confined to the worktrees folder.
fn confined(root: &Path, path: &str) -> Result<PathBuf, AppError> {
    let path = PathBuf::from(path);
    if host::inside(root, &path) {
        Ok(path)
    } else {
        Err(AppError::BadRequest(
            "Only worktrees in bruhs's worktrees folder can be managed here.".into(),
        ))
    }
}

pub async fn list(
    runtime: Option<Extension<CoordinationRuntime>>,
) -> Result<impl IntoResponse, AppError> {
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(rows(runtime).await?),
    ))
}

#[derive(Deserialize)]
pub struct PathQuery {
    path: String,
}

#[derive(Serialize)]
pub struct Measured {
    frees: Option<WorktreeFrees>,
    error: Option<String>,
}

/// What removing one worktree frees, by the configured meter.
pub async fn measure(Query(query): Query<PathQuery>) -> Result<Json<Measured>, AppError> {
    let path = confined(&root_for(&query.path)?, &query.path)?;
    let measured = match bruhs_core::worktree_meter::load(
        bruhs_core::worktree_meter::config_path().as_deref(),
    )
    .await
    {
        Ok(config) => match bruhs_core::worktree_meter::measure(&config, &path).await {
            Ok(frees) => Measured { frees, error: None },
            Err(error) => Measured {
                frees: None,
                error: Some(error),
            },
        },
        Err(error) => Measured {
            frees: None,
            error: Some(error),
        },
    };
    Ok(Json(measured))
}

#[derive(Deserialize)]
pub struct PathBody {
    path: String,
}

#[derive(Deserialize)]
pub struct Cleanup {
    path: String,
    branch: String,
    #[serde(default)]
    keep_when: bruhs_models::coordination::worktree::KeepWhen,
}

pub async fn clean_deleted_scratch(
    Extension(retention): Extension<host::retention::RetentionHost>,
    Json(body): Json<bruhs_models::coordination::worktree::ScratchCleanup>,
) -> Result<axum::http::StatusCode, AppError> {
    detached(async move { retention.remove_deleted_scratch(&body).await }).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Release the deleted thread’s checkout when its branch, owners and local
/// files still satisfy the policy captured at deletion.
pub async fn clean_deleted_thread(
    Extension(runtime): Extension<CoordinationRuntime>,
    Json(body): Json<Cleanup>,
) -> Result<axum::http::StatusCode, AppError> {
    let root = root_for(&body.path)?;
    let path = confined(&root, &body.path)?;
    if !path.exists() {
        return Ok(axum::http::StatusCode::NO_CONTENT);
    }
    detached(async move {
        let _lock = host::lock(&root).await?;
        if !path.exists() {
            return Ok(());
        }
        if runtime.task_in_worktree(&path).await.is_some() {
            return Err("A Workbench task uses this worktree.".into());
        }
        let project = bruhs_core::git::main_checkout(&path)
            .await
            .ok_or("Git could not identify the main checkout.")?;
        host::cleanup_thread_checkout(&root, &project, &path, &body.branch, body.keep_when).await
    })
    .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub async fn remove(
    runtime: Option<Extension<CoordinationRuntime>>,
    Json(body): Json<PathBody>,
) -> Result<Json<Vec<ManagedWorktree>>, AppError> {
    let root = root_for(&body.path)?;
    let path = confined(&root, &body.path)?;
    let task = match &runtime {
        Some(Extension(runtime)) => runtime.task_in_worktree(&path).await,
        None => None,
    };
    if let (Some(task), Some(Extension(runtime))) = (task, &runtime) {
        runtime
            .command(CoordinationCommand::RemoveWorktree { id: task.id })
            .await?;
    } else {
        let project = bruhs_core::git::main_checkout(&path).await.ok_or_else(|| {
            AppError::Conflict("Git cannot find the checkout this worktree belongs to.".into())
        })?;
        detached(async move {
            let _lock = host::lock(&root).await?;
            let branch = host::check(&root, &project, &path).await?;
            host::unregister(&root, &project, &path, &branch).await
        })
        .await?;
    }
    Ok(Json(rows(runtime).await?))
}

#[derive(Deserialize)]
pub struct StartAgain {
    path: String,
    /// The branch to start from when the recorded branch is gone.
    base: String,
}

/// Check a removed worktree out again now; from `base` when its branch was
/// deleted (the branch name is kept).
pub async fn start_again(
    runtime: Option<Extension<CoordinationRuntime>>,
    Json(body): Json<StartAgain>,
) -> Result<Json<Vec<ManagedWorktree>>, AppError> {
    let root = root_for(&body.path)?;
    let path = confined(&root, &body.path)?;
    if bruhs_models::logic::git::BranchName::try_from(body.base.as_str()).is_err() {
        return Err(AppError::BadRequest("Choose a valid base branch.".into()));
    }
    let (target, base) = (path.clone(), body.base);
    detached(async move {
        let _lock = host::lock(&root).await?;
        let record = host::record(&root, &target)
            .await?
            .ok_or_else(|| "bruhs has no record of removing this worktree.".to_string())?;
        host::restore_branch(
            &root,
            Path::new(&record.project),
            &target,
            &record.branch,
            Some(&base),
        )
        .await
    })
    .await?;
    if let Some(Extension(runtime)) = &runtime {
        runtime.worktree_restored(&path).await?;
    }
    Ok(Json(rows(runtime).await?))
}

/// Drop the record of a removed chat or issue worktree: its next turn no
/// longer checks it out again. A task's worktree starts again instead,
/// since the task cannot run without one.
pub async fn forget(
    runtime: Option<Extension<CoordinationRuntime>>,
    Json(body): Json<PathBody>,
) -> Result<Json<Vec<ManagedWorktree>>, AppError> {
    let root = root_for(&body.path)?;
    let path = confined(&root, &body.path)?;
    if let Some(Extension(runtime)) = &runtime {
        if runtime.task_in_worktree(&path).await.is_some() {
            return Err(AppError::Conflict(
                "A Workbench task works here. Start it again from another branch, or delete the task.".into(),
            ));
        }
    }
    detached(async move {
        let _lock = host::lock(&root).await?;
        host::forget(&root, &path).await
    })
    .await?;
    Ok(Json(rows(runtime).await?))
}

async fn rows(
    runtime: Option<Extension<CoordinationRuntime>>,
) -> Result<Vec<ManagedWorktree>, AppError> {
    let mut rows = Vec::new();
    for root in host::locations::all().map_err(|e| AppError::Unavailable(e.to_string()))? {
        rows.extend(host::list(&root).await);
    }
    rows.sort_by(|a, b| a.path.cmp(&b.path));
    if let Some(Extension(runtime)) = runtime {
        let owners = runtime.worktree_owners().await;
        for row in &mut rows {
            let Some(task) = owners.get(&host::resolve(Path::new(&row.path))) else {
                continue;
            };
            if row.state != WorktreeState::Removed {
                if let Some(reason) = runtime.task_refusal(task).await {
                    row.blocked = Some(reason);
                }
            }
            row.task = Some(WorktreeTask {
                id: task.id.clone(),
                title: task.title.clone(),
            });
        }
    }
    Ok(rows)
}

/// Run a removal, restore or forget to completion even if the request that
/// asked is dropped midway (the page closed), so it never stops between its
/// steps.
async fn detached<F>(work: F) -> Result<(), AppError>
where
    F: std::future::Future<Output = Result<(), String>> + Send + 'static,
{
    tokio::spawn(work)
        .await
        .map_err(AppError::internal)?
        .map_err(AppError::Conflict)
}
