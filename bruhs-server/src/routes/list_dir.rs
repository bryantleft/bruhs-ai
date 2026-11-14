use axum::Json;
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::routes::paths::HomePath;

#[derive(Deserialize)]
pub struct ListDirRequest {
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Serialize)]
pub struct DirEntryOut {
    pub name: String,
    pub path: String,
}

#[derive(Serialize)]
pub struct ListDirResponse {
    pub current: String,
    pub name: String,
    pub parent: Option<String>,
    pub dirs: Vec<DirEntryOut>,
}

/// POST /api/agent/list-dir — list immediate child directories under a
/// path within the user's home folder (the home folder when no path is
/// given; never `/`).
pub async fn post_list_dir(
    Json(body): Json<ListDirRequest>,
) -> Result<Json<ListDirResponse>, AppError> {
    let resolved = match body.path.as_deref().filter(|p| !p.is_empty()) {
        Some(path) => HomePath::resolve(path).await?,
        None => HomePath::home().await?,
    };

    // A finite scan needs one worker job rather than Tokio's repeated
    // directory-batch handoffs. Keep filesystem I/O and sorting off the runtime.
    let listing = tokio::task::spawn_blocking(move || read_directory(&resolved))
        .await
        .map_err(|error| AppError::internal_in("read directory worker", error))??;
    Ok(Json(listing))
}

fn read_directory(resolved: &HomePath) -> Result<ListDirResponse, AppError> {
    let entries = std::fs::read_dir(resolved.as_path())
        .map_err(|error| AppError::BadRequest(format!("Cannot read directory: {error}")))?;

    let mut dirs: Vec<DirEntryOut> = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => return Err(AppError::internal_in("read directory", error)),
        };
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        if !is_dir {
            continue;
        }
        dirs.push(DirEntryOut {
            name: name.clone(),
            path: resolved.as_path().join(&name).to_string_lossy().to_string(),
        });
    }
    dirs.sort_by(|a, b| a.name.cmp(&b.name));

    let current = resolved.as_path().to_string_lossy().to_string();
    let name = resolved
        .as_path()
        .file_name()
        .map_or_else(|| current.clone(), |n| n.to_string_lossy().to_string());
    let parent = resolved.parent().map(|p| p.to_string_lossy().to_string());

    Ok(ListDirResponse {
        current,
        name,
        parent,
        dirs,
    })
}
