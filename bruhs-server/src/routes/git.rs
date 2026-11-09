//! Git routes: thin HTTP wrappers over `bruhs_core::git`. Response shapes
//! are part of the web contract and must not change.

use std::path::{Path, PathBuf};

use axum::Json;
use bruhs_core::git::{self, GitDiff, GitError};
use bruhs_models::logic::git::{is_valid_branch, GitStatus};
use bruhs_models::logic::run_place::WORKTREE_BRANCH_PREFIX;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{AppError, WithSuccessFlag};
use crate::routes::paths::{HomePath, HomePathError};

fn map_git_error(err: GitError) -> AppError {
    match err {
        GitError::Failed(msg) => AppError::BadRequest(msg),
        GitError::InvalidBranch(_) | GitError::UnknownBranch(_) => {
            AppError::BadRequest(err.to_string())
        }
        _ => AppError::internal(err),
    }
}

#[derive(Deserialize)]
pub struct BranchesRequest {
    pub path: Option<String>,
}

#[derive(Serialize)]
pub struct BranchesResponse {
    pub branches: Vec<String>,
    pub current: Option<String>,
}

impl BranchesResponse {
    fn empty() -> Self {
        Self {
            branches: vec![],
            current: None,
        }
    }
}

/// POST /api/agent/git-branches — an empty list (200) when the path is
/// missing, does not exist, is not a repo, or git fails; 403 outside the
/// home directory.
pub async fn get_branches(
    Json(body): Json<BranchesRequest>,
) -> Result<Json<BranchesResponse>, AppError> {
    let resolved = match HomePath::required(body.path.as_deref()).await {
        Ok(path) => path,
        Err(HomePathError::Missing | HomePathError::Unresolvable(_)) => {
            return Ok(Json(BranchesResponse::empty()))
        }
        Err(error) => return Err(error.into()),
    };
    match git::branches(resolved.as_path()).await {
        Ok(b) => Ok(Json(BranchesResponse {
            branches: b.branches,
            current: b.current,
        })),
        Err(error) => {
            // Not a repository is the common case; the picker shows no branches.
            tracing::debug!("git branches unavailable: {error}");
            Ok(Json(BranchesResponse::empty()))
        }
    }
}

#[derive(Deserialize)]
pub struct CheckoutRequest {
    pub path: Option<String>,
    pub branch: Option<String>,
}

/// POST /api/agent/git-checkout — `{"success": true}`, or the error with
/// `"success": false`.
pub async fn post_checkout(
    Json(body): Json<CheckoutRequest>,
) -> Result<Json<Value>, WithSuccessFlag> {
    let path = body.path.unwrap_or_default();
    let branch = body.branch.unwrap_or_default();
    if path.is_empty() || branch.is_empty() {
        return Err(AppError::BadRequest("path and branch required".into()).into());
    }
    if !is_valid_branch(&branch) {
        return Err(AppError::BadRequest("Invalid branch name".into()).into());
    }
    let resolved = HomePath::resolve(&path).await.map_err(AppError::from)?;
    git::checkout(resolved.as_path(), &branch)
        .await
        .map_err(map_git_error)?;
    Ok(Json(json!({ "success": true })))
}

#[derive(Deserialize)]
pub struct RepoRequest {
    pub path: Option<String>,
}

#[derive(Deserialize)]
pub struct WorktreeRequest {
    pub path: Option<String>,
    pub base: Option<String>,
    pub branch: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct WorktreeResponse {
    pub path: String,
    pub branch: String,
}

/// POST /api/agent/worktree — create `branch` (it must start with `bruhs/`)
/// from `base` in a worktree under the bruhs data directory. The project's
/// checkout is not switched. A repeat of the same branch returns the same path.
pub async fn post_worktree(
    Json(body): Json<WorktreeRequest>,
) -> Result<Json<WorktreeResponse>, AppError> {
    let path = body.path.unwrap_or_default();
    let base = body.base.unwrap_or_default();
    let branch = body.branch.unwrap_or_default();
    if path.is_empty() || base.is_empty() || branch.is_empty() {
        return Err(AppError::BadRequest(
            "path, base, and branch required".into(),
        ));
    }
    if !branch.starts_with(WORKTREE_BRANCH_PREFIX)
        || !is_valid_branch(&branch)
        || !is_valid_branch(&base)
    {
        return Err(AppError::BadRequest("Invalid branch name".into()));
    }
    let repo = HomePath::resolve(&path).await?;
    let dest = confined_worktree_dest(repo.as_path(), &branch).await?;
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| AppError::BadRequest(error.to_string()))?;
    }
    git::add_worktree(repo.as_path(), &base, &dest, &branch)
        .await
        .map_err(map_git_error)?;
    Ok(Json(WorktreeResponse {
        path: dest.to_string_lossy().into_owned(),
        branch,
    }))
}

/// `worktree_dest` does not exist yet, so it cannot be canonicalized whole.
/// The existing ancestor has to land inside the home directory, and the
/// remainder we join is only path components this server generated.
async fn confined_worktree_dest(project: &Path, branch: &str) -> Result<PathBuf, AppError> {
    let dest = git::worktree_dest(project, branch).map_err(map_git_error)?;
    let home = HomePath::home().await?;
    let mut ancestor = dest.as_path();
    while !ancestor.exists() {
        ancestor = ancestor.parent().ok_or(HomePathError::OutsideHome)?;
    }
    let canonical = tokio::fs::canonicalize(ancestor)
        .await
        .map_err(HomePathError::Unresolvable)?;
    if !canonical.starts_with(home.as_path()) {
        return Err(HomePathError::OutsideHome.into());
    }
    let rest = dest.strip_prefix(ancestor).unwrap_or(Path::new(""));
    if rest
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(HomePathError::OutsideHome.into());
    }
    let confined = canonical.join(rest);
    if !confined.starts_with(home.as_path()) {
        return Err(HomePathError::OutsideHome.into());
    }
    Ok(confined)
}

#[derive(Deserialize)]
pub struct SnapshotRequest {
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SnapshotResponse {
    pub tree: String,
}

#[derive(Deserialize)]
pub struct RestoreRequest {
    pub path: Option<String>,
    pub tree: Option<String>,
}

/// The worktree must already exist under the bruhs worktrees root. `/tmp`
/// is refused even when that root cannot be created.
async fn confined_existing_worktree(path: Option<&str>) -> Result<PathBuf, AppError> {
    let path = path.unwrap_or("").trim();
    if path.is_empty() {
        return Err(AppError::BadRequest("path required".into()));
    }
    git::ensure_bruhs_worktree(Path::new(path)).map_err(map_git_error)
}

/// POST /api/agent/worktree-snapshot — `{ tree }` for a bruhs worktree.
pub async fn post_snapshot(
    Json(body): Json<SnapshotRequest>,
) -> Result<Json<SnapshotResponse>, AppError> {
    let path = confined_existing_worktree(body.path.as_deref()).await?;
    let tree = git::snapshot_worktree(&path).await.map_err(map_git_error)?;
    Ok(Json(SnapshotResponse { tree }))
}

/// POST /api/agent/worktree-restore — put that worktree back to `tree`.
pub async fn post_restore(Json(body): Json<RestoreRequest>) -> Result<Json<Value>, AppError> {
    let path = confined_existing_worktree(body.path.as_deref()).await?;
    let tree = body.tree.unwrap_or_default();
    if tree.trim().is_empty() {
        return Err(AppError::BadRequest("path and tree required".into()));
    }
    git::restore_worktree(&path, &tree)
        .await
        .map_err(map_git_error)?;
    Ok(Json(json!({ "success": true })))
}

/// POST /api/agent/git-status
pub async fn get_status(Json(body): Json<RepoRequest>) -> Result<Json<GitStatus>, AppError> {
    let dir = HomePath::required(body.path.as_deref()).await?;
    git::status(dir.as_path())
        .await
        .map(Json)
        .map_err(map_git_error)
}

/// POST /api/agent/git-diff — `git diff HEAD` plus untracked files rendered
/// as additions, as one unified diff.
pub async fn get_diff(Json(body): Json<RepoRequest>) -> Result<Json<GitDiff>, AppError> {
    let dir = HomePath::required(body.path.as_deref()).await?;
    git::diff(dir.as_path())
        .await
        .map(Json)
        .map_err(map_git_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    /// Branch errors are client errors (400) whatever route raised them;
    /// checkout used to answer 500 for a failed `git switch`.
    #[tokio::test]
    async fn checkout_reports_errors_with_the_success_flag() {
        let response = post_checkout(Json(CheckoutRequest {
            path: Some("/tmp".into()),
            branch: Some("bad..name".into()),
        }))
        .await
        .unwrap_err()
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            json!({ "success": false, "error": "Invalid branch name" })
        );
        let rejected = post_worktree(Json(WorktreeRequest {
            path: Some("/tmp/repo".into()),
            base: Some("main".into()),
            branch: Some("feature/x".into()),
        }))
        .await
        .unwrap_err()
        .into_response();
        assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);

        for (error, status) in [
            (
                GitError::Failed("dirty tree".into()),
                StatusCode::BAD_REQUEST,
            ),
            (
                GitError::UnknownBranch("nope".into()),
                StatusCode::BAD_REQUEST,
            ),
            (GitError::TimedOut, StatusCode::INTERNAL_SERVER_ERROR),
        ] {
            assert_eq!(map_git_error(error).status(), status);
        }
    }

    #[tokio::test]
    async fn snapshot_and_restore_refuse_a_path_outside_the_worktrees_root() {
        for path in [
            Some("/tmp".to_string()),
            Some("/tmp/repo".to_string()),
            None,
        ] {
            let snapshot = post_snapshot(Json(SnapshotRequest { path: path.clone() }))
                .await
                .unwrap_err();
            assert_eq!(snapshot.status(), StatusCode::BAD_REQUEST);
            let restore = post_restore(Json(RestoreRequest {
                path,
                tree: Some("4b825dc642cb6eb9a060e54bf8d69288fbee4904".into()),
            }))
            .await
            .unwrap_err();
            assert_eq!(restore.status(), StatusCode::BAD_REQUEST);
        }
    }
}
