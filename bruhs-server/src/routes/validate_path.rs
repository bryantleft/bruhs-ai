use axum::Json;
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::routes::paths::{HomePath, HomePathError};

#[derive(Deserialize)]
pub struct ValidatePathRequest {
    pub path: Option<String>,
}

#[derive(Serialize)]
pub struct ValidatePathResponse {
    pub valid: bool,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ValidatePathResponse {
    fn invalid(reason: &str) -> Self {
        Self {
            valid: false,
            name: String::new(),
            error: Some(reason.to_string()),
        }
    }
}

/// POST /api/agent/validate-path — checks that a path exists, is a
/// directory, and resolves within $HOME. 200 with `{ valid: false, error }`
/// for the soft failures the picker renders inline; 400 for a missing path
/// and 500 when the home directory itself cannot be resolved.
pub async fn post_validate(
    Json(body): Json<ValidatePathRequest>,
) -> Result<Json<ValidatePathResponse>, AppError> {
    let path = body
        .path
        .filter(|p| !p.is_empty())
        .ok_or_else(|| AppError::BadRequest("Path is required".into()))?;

    let resolved = match HomePath::resolve(&path).await {
        Ok(resolved) => resolved,
        Err(HomePathError::Unresolvable(_)) => {
            return Ok(Json(ValidatePathResponse::invalid("Directory not found")))
        }
        Err(HomePathError::OutsideHome) => {
            return Ok(Json(ValidatePathResponse::invalid(
                "Path must be within your home directory",
            )))
        }
        Err(error) => return Err(error.into()),
    };

    let is_dir = tokio::fs::metadata(resolved.as_path())
        .await
        .is_ok_and(|meta| meta.is_dir());
    if !is_dir {
        return Ok(Json(ValidatePathResponse::invalid(
            "Path is not a directory",
        )));
    }

    let name = resolved
        .as_path()
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    Ok(Json(ValidatePathResponse {
        valid: true,
        name,
        error: None,
    }))
}
