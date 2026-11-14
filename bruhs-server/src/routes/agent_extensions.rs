use crate::error::AppError;
use axum::{response::Html, Json};
use bruhs_models::agent_extensions::{
    ExtensionConfig, ImportedUsageSnapshot, UsageHistoryPreview, UsageHistoryRequest, UsageSnapshot,
};

pub async fn history_preview(
    Json(mut request): Json<UsageHistoryRequest>,
) -> Result<Json<UsageHistoryPreview>, AppError> {
    let path = bruhs_core::agent_extensions::history::source_root(&request);
    let path = crate::routes::paths::HomePath::resolve(&path.to_string_lossy()).await?;
    request.root = Some(path.as_path().to_string_lossy().into_owned());
    tokio::task::spawn_blocking(move || {
        bruhs_core::agent_extensions::history::preview_at(
            &bruhs_core::agent_extensions::directory()?,
            request,
        )
    })
    .await
    .map_err(AppError::internal)?
    .map(Json)
    .map_err(|error| AppError::BadRequest(error.to_string()))
}
pub async fn history_commit(
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<ImportedUsageSnapshot>, AppError> {
    tokio::task::spawn_blocking(move || {
        bruhs_core::agent_extensions::history::commit_at(
            &bruhs_core::agent_extensions::directory()?,
            &id,
        )
    })
    .await
    .map_err(AppError::internal)?
    .map(Json)
    .map_err(|error| AppError::BadRequest(error.to_string()))
}

pub async fn config() -> Result<Json<ExtensionConfig>, AppError> {
    tokio::task::spawn_blocking(bruhs_core::agent_extensions::load_config)
        .await
        .map_err(AppError::internal)?
        .map(Json)
        .map_err(AppError::internal)
}
pub async fn save(Json(config): Json<ExtensionConfig>) -> Result<Json<ExtensionConfig>, AppError> {
    config.validate().map_err(AppError::BadRequest)?;
    let copy = config.clone();
    tokio::task::spawn_blocking(move || {
        bruhs_core::agent_extensions::save_config_at(
            &bruhs_core::agent_extensions::directory()?,
            &copy,
        )
    })
    .await
    .map_err(AppError::internal)?
    .map_err(AppError::internal)?;
    Ok(Json(config))
}
pub async fn usage() -> Result<Json<UsageSnapshot>, AppError> {
    tokio::task::spawn_blocking(|| {
        bruhs_core::agent_extensions::snapshot_at(&bruhs_core::agent_extensions::directory()?)
    })
    .await
    .map_err(AppError::internal)?
    .map(Json)
    .map_err(AppError::internal)
}
pub async fn refresh() -> Result<Json<UsageSnapshot>, AppError> {
    bruhs_core::agent_extensions::refresh_codex()
        .await
        .map_err(|e| AppError::bad_gateway("Could not refresh Codex usage limits", e))?;
    usage().await
}
pub async fn ui() -> Html<&'static str> {
    Html(include_str!("agent_extensions.html"))
}
pub async fn workbench() -> Html<&'static str> {
    Html(include_str!("workbench.html"))
}
