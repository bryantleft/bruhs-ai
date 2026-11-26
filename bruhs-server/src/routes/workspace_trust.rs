//! Passive inspection and owner-only management of host workspace trust.

use std::sync::Arc;

use axum::{extract::State, http::HeaderMap, Json};
use bruhs_models::workspace_trust::{
    WorkspaceTrustQuery, WorkspaceTrustStatus, WorkspaceTrustUpdate,
};

use crate::config::Config;
use crate::error::AppError;
use crate::routes::paths::HomePath;

/// Read the trusted-folder entries saved on this host.
pub async fn get_trusted(
    State(config): State<Arc<Config>>,
    headers: HeaderMap,
) -> Result<Json<Vec<String>>, AppError> {
    require_owner(&config, &headers)?;
    let paths = tokio::task::spawn_blocking(bruhs_core::workspace_trust::list_trusted)
        .await
        .map_err(|error| AppError::internal_in("workspace trust list task failed", error))?
        .map_err(|error| AppError::internal_in("workspace trust list failed", error))?;
    Ok(Json(paths))
}

/// Revoke the exact entry returned by the list, including missing folders.
pub async fn delete_trusted(
    State(config): State<Arc<Config>>,
    headers: HeaderMap,
    Json(body): Json<WorkspaceTrustQuery>,
) -> Result<Json<Vec<String>>, AppError> {
    require_owner(&config, &headers)?;
    let paths = tokio::task::spawn_blocking(move || {
        bruhs_core::workspace_trust::revoke_trusted(&body.path)
    })
    .await
    .map_err(|error| AppError::internal_in("workspace trust revoke task failed", error))?
    .map_err(|error| AppError::internal_in("workspace trust revoke failed", error))?;
    Ok(Json(paths))
}

fn require_owner(config: &Config, headers: &HeaderMap) -> Result<(), AppError> {
    if config.access.owner_request(headers) {
        Ok(())
    } else {
        Err(AppError::Forbidden(
            "Managing workspace trust requires the Bruhs app origin.".into(),
        ))
    }
}

/// `POST /api/agent/workspace-trust` — passively inspect a project folder
/// inside `$HOME` without executing any repository code or Git commands.
pub async fn post_inspect(
    Json(body): Json<WorkspaceTrustQuery>,
) -> Result<Json<WorkspaceTrustStatus>, AppError> {
    let resolved = HomePath::resolve(body.path.trim()).await?;
    let path = resolved.into_path_buf();
    let status = tokio::task::spawn_blocking(move || bruhs_core::workspace_trust::inspect(&path))
        .await
        .map_err(|error| AppError::internal_in("workspace trust inspection failed", error))?;
    Ok(Json(status))
}

/// `PUT /api/agent/workspace-trust` — mark a project folder inside `$HOME` as
/// trusted or untrusted. Requires the Bruhs owner origin.
pub async fn put_trust(
    State(config): State<Arc<Config>>,
    headers: HeaderMap,
    Json(body): Json<WorkspaceTrustUpdate>,
) -> Result<Json<WorkspaceTrustStatus>, AppError> {
    require_owner(&config, &headers)?;
    let resolved = HomePath::resolve(body.path.trim()).await?;
    let path = resolved.into_path_buf();
    let trusted = body.trusted;
    let status = tokio::task::spawn_blocking(move || {
        bruhs_core::workspace_trust::set_trusted(&path, trusted)
    })
    .await
    .map_err(|error| AppError::internal_in("workspace trust update failed", error))?
    .map_err(|error| AppError::BadRequest(error.to_string()))?;
    Ok(Json(status))
}
