//! POST /api/open-in-editor — open a project or file in the user's editor.

use axum::Json;
use bruhs_core::editor::{open_in_editor, EditorError};
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
        open_in_editor(&editor_id, path.as_path(), body.line).map_err(|err| match err {
            EditorError::UnknownEditor => AppError::BadRequest(err.to_string()),
            EditorError::NotInstalled => AppError::NotFound(err.to_string()),
            _ => AppError::internal(err),
        })?;
    Ok(Json(OpenInEditorResponse {
        editor: editor.label.to_string(),
    }))
}
