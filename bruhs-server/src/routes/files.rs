//! POST /api/agent/files — `@`-mention file search; the search itself is
//! `bruhs_core::files`.

use axum::Json;
use bruhs_core::files::{self, FileEntry, FileSearchError};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::routes::paths::HomePath;

#[derive(Deserialize)]
pub struct FilesRequest {
    /// Project to search; omitted for a chat without a project, which
    /// searches the home directory.
    pub path: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
}

#[derive(Serialize)]
pub struct FilesResponse {
    pub files: Vec<FileEntry>,
}

/// List files in a project (inside the home directory) matching `query`,
/// ranked by git hotspots. A failed search is an error status, never an
/// empty list that reads as "no matches".
pub async fn post_files(Json(body): Json<FilesRequest>) -> Result<Json<FilesResponse>, AppError> {
    let root = HomePath::or_home_in(body.path.as_deref(), home::home_dir().as_deref()).await?;
    let query = body.query.unwrap_or_default();
    let files = files::search(root.as_path(), &query)
        .await
        .map_err(|error| match error {
            FileSearchError::NotARepository(_) | FileSearchError::Root(_) => {
                AppError::BadRequest(error.to_string())
            }
            _ => AppError::internal(error),
        })?;
    Ok(Json(FilesResponse { files }))
}

#[derive(Deserialize)]
pub struct ChildrenRequest {
    pub path: Option<String>,
    #[serde(default)]
    pub dir: Option<String>,
}

/// POST /api/agent/files/children — one directory of the Files surface.
/// A chat without a project is an error, not the server's working directory.
pub async fn post_children(
    Json(body): Json<ChildrenRequest>,
) -> Result<Json<FilesResponse>, AppError> {
    let Some(path) = body.path.as_deref().filter(|path| !path.is_empty()) else {
        return Err(AppError::BadRequest(
            "Open a project to browse files.".into(),
        ));
    };
    let root = HomePath::resolve(path).await?;
    let dir = body.dir.unwrap_or_default();
    let files = files::list_children(root.as_path(), &dir)
        .await
        .map_err(|error| match error {
            FileSearchError::NotARepository(_) | FileSearchError::Root(_) => {
                AppError::BadRequest(error.to_string())
            }
            _ => AppError::internal(error),
        })?;
    Ok(Json(FilesResponse { files }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::paths::HomePathError;

    /// A chat without a project omits `path`; the search runs in the home
    /// directory, never in the server's working directory.
    #[tokio::test]
    async fn a_search_without_a_project_runs_in_the_home_directory() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        std::fs::create_dir_all(home.join("notes")).unwrap();
        std::fs::write(home.join("notes/todo.md"), "x").unwrap();

        let body: FilesRequest = serde_json::from_str(r#"{"query":"notes/"}"#).unwrap();
        assert_eq!(body.path, None);
        let dir = HomePath::or_home_in(body.path.as_deref(), Some(&home))
            .await
            .unwrap();
        assert_eq!(dir.as_path(), home.canonicalize().unwrap());
        let files = files::search(dir.as_path(), "notes/").await.unwrap();
        assert!(
            files.iter().any(|f| f.path.ends_with("todo.md")),
            "{files:?}"
        );
        assert!(matches!(
            HomePath::or_home_in(Some(""), Some(&home)).await,
            Err(HomePathError::Missing)
        ));
    }
}
