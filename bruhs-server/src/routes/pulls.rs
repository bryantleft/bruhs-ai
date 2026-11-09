//! POST /api/agent/pulls — `#` pull-request lookup via `gh`.

use axum::Json;
use bruhs_core::pulls::{self, PullRef};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::routes::paths::HomePath;

#[derive(Deserialize)]
pub struct PullsRequest {
    pub path: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
}

#[derive(Serialize)]
pub struct PullsResponse {
    pub pulls: Vec<PullRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// List pull requests for the project at `path`. A chat without a project
/// and a missing `gh` both return an empty list plus `error`, not a forged
/// pull request.
pub async fn post_pulls(Json(body): Json<PullsRequest>) -> Result<Json<PullsResponse>, AppError> {
    let Some(path) = body.path.as_deref().filter(|path| !path.is_empty()) else {
        return Ok(Json(PullsResponse {
            pulls: Vec::new(),
            error: Some("Open a project to attach a pull request.".into()),
        }));
    };
    let root = HomePath::resolve(path).await?;
    let query = body.query.unwrap_or_default();
    let listed = pulls::list_pulls(root.as_path(), &query).await;
    Ok(Json(PullsResponse {
        pulls: listed.pulls,
        error: listed.error,
    }))
}

#[derive(Deserialize)]
pub struct BranchPullRequest {
    pub path: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
}

#[derive(Serialize)]
pub struct BranchPullResponse {
    pub signed_in: bool,
    pub pull: Option<PullRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// POST /api/agent/branch-pull — the GitHub pull request for this thread's
/// branch. Signed-out `gh` is `signed_in: false` and no pull request.
pub async fn post_branch_pull(
    Json(body): Json<BranchPullRequest>,
) -> Result<Json<BranchPullResponse>, AppError> {
    let Some(path) = body.path.as_deref().filter(|path| !path.is_empty()) else {
        return Ok(Json(BranchPullResponse {
            signed_in: false,
            pull: None,
            error: Some("Open a project to see its pull request.".into()),
        }));
    };
    let root = HomePath::resolve(path).await?;
    let branch = body.branch.unwrap_or_default();
    let listed = pulls::pull_for_branch(root.as_path(), &branch).await;
    Ok(Json(BranchPullResponse {
        signed_in: listed.signed_in,
        pull: listed.pull,
        error: listed.error,
    }))
}
