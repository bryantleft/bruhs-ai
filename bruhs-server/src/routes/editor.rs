//! POST /api/open-in-editor — open a project or file in the user's editor.
//! POST /api/open-in-terminal — open the system terminal at a folder.

use axum::http::StatusCode;
use axum::Json;
use bruhs_core::editor::{open_in_editor, open_system_terminal, EditorError};
use bruhs_models::logic::editor::editor_by_id;
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::routes::paths::HomePath;

#[derive(Deserialize)]
pub struct OpenInEditorRequest {
    pub path: Option<String>,
    pub editor: Option<String>,
    #[serde(default)]
    pub line: Option<u32>,
}

#[derive(Serialize)]
pub struct OpenInEditorResponse {
    /// Human label of the editor that was launched.
    pub editor: String,
}

pub async fn post_open_in_editor(
    Json(body): Json<OpenInEditorRequest>,
) -> Result<Json<OpenInEditorResponse>, AppError> {
    let editor_id = body
        .editor
        .filter(|e| !e.is_empty())
        .ok_or_else(|| AppError::BadRequest("editor required".to_string()))?;
    if editor_by_id(&editor_id).is_none() {
        return Err(AppError::BadRequest(format!("unknown editor {editor_id}")));
    }
    let path = HomePath::required(body.path.as_deref()).await?;
    let editor =
        tokio::task::spawn_blocking(move || open_in_editor(&editor_id, path.as_path(), body.line))
            .await
            .map_err(|error| AppError::internal_in("editor launch failed", error))?
            .map_err(|err| match err {
                EditorError::UnknownEditor => AppError::BadRequest(err.to_string()),
                EditorError::NotInstalled => AppError::NotFound(err.to_string()),
                _ => AppError::internal(err),
            })?;
    Ok(Json(OpenInEditorResponse {
        editor: editor.label.to_string(),
    }))
}

#[derive(Deserialize)]
pub struct OpenInTerminalRequest {
    #[serde(default)]
    pub path: Option<String>,
}

/// The palette's "Open in system Terminal": the system terminal at `path`
/// inside home, or at home when the thread has no project.
pub async fn post_open_in_terminal(
    Json(body): Json<OpenInTerminalRequest>,
) -> Result<StatusCode, AppError> {
    let path = match body.path.as_deref() {
        Some(path) => HomePath::resolve(path).await?,
        None => HomePath::home().await?,
    };
    tokio::task::spawn_blocking(move || open_system_terminal(path.as_path()))
        .await
        .map_err(|error| AppError::internal_in("terminal launch failed", error))?
        .map_err(|error| AppError::internal_in("could not open Terminal", error))?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_terminal_outside_home_is_refused_before_launch() {
        let refused = post_open_in_terminal(Json(OpenInTerminalRequest {
            path: Some("/etc".to_string()),
        }))
        .await;
        assert!(matches!(refused, Err(AppError::Forbidden(_))));
        let blank = post_open_in_terminal(Json(OpenInTerminalRequest {
            path: Some(String::new()),
        }))
        .await;
        assert!(matches!(blank, Err(AppError::BadRequest(_))));
    }
}
