//! POST /api/agent/skills — `$` skill lookup from skill folders on disk.

use std::path::PathBuf;

use axum::Json;
use bruhs_core::skills::{self, SkillRef};
use bruhs_models::agent::AgentProvider;
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::routes::paths::HomePath;

#[derive(Deserialize)]
pub struct SkillsRequest {
    pub path: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    /// `"claude"` or `"codex"`. Anything else is an empty list plus `error`.
    #[serde(default)]
    pub provider: Option<String>,
}

#[derive(Serialize)]
pub struct SkillsResponse {
    pub skills: Vec<SkillRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// List skills for the selected provider. A chat without a project still
/// lists skills from the home directory.
pub async fn post_skills(
    Json(body): Json<SkillsRequest>,
) -> Result<Json<SkillsResponse>, AppError> {
    let provider = match body.provider.as_deref().unwrap_or("claude") {
        "claude" => AgentProvider::Claude,
        "codex" => AgentProvider::Codex,
        other => {
            return Ok(Json(SkillsResponse {
                skills: Vec::new(),
                error: Some(format!("Unknown provider: {other}")),
            }));
        }
    };
    let project = match body.path.as_deref().filter(|path| !path.is_empty()) {
        Some(path) => Some(HomePath::resolve(path).await?),
        None => None,
    };
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let query = body.query.unwrap_or_default();
    let skills = skills::list_skills(
        provider,
        home.as_deref(),
        project.as_ref().map(HomePath::as_path),
        &query,
    );
    Ok(Json(SkillsResponse {
        skills,
        error: None,
    }))
}
