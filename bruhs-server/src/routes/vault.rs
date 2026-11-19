//! The vault over HTTP (D22), for both apps' Settings, Vault page.
//!
//! - `GET /api/vault`: every entry's metadata, the bruhs' names for scope
//!   lines and the picker, and whether `op` was found. Never a value.
//! - `POST /api/vault` (a [`VaultCreate`]) and `PUT /api/vault/{id}` (a
//!   [`VaultUpdate`]): the saved [`VaultEntry`].
//! - `DELETE /api/vault/{id}`: 204, and the entry leaves every bruh's
//!   grants.
//! - `POST /api/vault/{id}/check`: whether one read worked
//!   ([`VaultCheck`]).
//!
//! Every route sits behind the shared loopback guard, and each also needs
//! a request from the owner's own app ([`Policy::owner_request`]): other
//! loopback pages may neither read the vault nor change it. Bodies
//! that aren't entries get [`INVALID_ENTRY`], never serde's text, which
//! can quote a value; errors are fixed copy, and no handler logs a body.

use std::sync::Arc;

use axum::{
    extract::{rejection::JsonRejection, Path, State},
    http::{HeaderMap, StatusCode},
    Extension, Json,
};
use bruhs_core::{reactive::ReactiveRuntime, secrets::SecretError, AppState};
use bruhs_models::vault::{
    CredentialScope, VaultBruh, VaultCheck, VaultCreate, VaultEntry, VaultListing, VaultUpdate,
    BRUH_MISSING, ENTRY_CHANGED, ENTRY_GONE, INVALID_ENTRY, OWNER_ONLY, PROJECT_MISSING,
};

use crate::config::Config;
use crate::error::AppError;
use crate::local_access::Policy;

/// The largest vault request body: a 16 KiB key value with room for its
/// name and JSON, well under the server's 1 MiB default.
pub const MAX_BODY_BYTES: usize = 64 << 10;

// Split so clippy's allow-unwrap-in-tests sees `cfg(test)`.
#[cfg(test)]
#[cfg(unix)]
mod tests;

/// The owner copy for a vault error. Every message is fixed text: core's
/// vault errors never carry a value or 1Password's own output.
fn vault_error(error: SecretError) -> AppError {
    match error {
        SecretError::NotSaved => AppError::NotFound(ENTRY_GONE.into()),
        SecretError::Invalid(message) => AppError::BadRequest(message.into()),
        SecretError::Conflict => AppError::Conflict(ENTRY_CHANGED.into()),
        error @ (SecretError::Storage | SecretError::Catalog | SecretError::OnePassword(_)) => {
            AppError::Unavailable(error.to_string())
        }
        SecretError::Connection(_) | SecretError::NotPending | SecretError::Busy => {
            AppError::Unavailable(SecretError::Storage.to_string())
        }
    }
}

/// Refuse a change from any page but the owner's own app (ruling F26).
pub(super) fn owner_only(policy: &Policy, headers: &HeaderMap) -> Result<(), AppError> {
    if policy.owner_request(headers) {
        Ok(())
    } else {
        Err(AppError::Forbidden(OWNER_ONLY.into()))
    }
}

/// A request body that must be a vault request, or [`INVALID_ENTRY`].
fn body<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, AppError> {
    body.map(|Json(body)| body)
        .map_err(|_| AppError::BadRequest(INVALID_ENTRY.into()))
}

/// Every bruh, by its id and name.
async fn bruhs(runtime: &ReactiveRuntime) -> Result<Vec<VaultBruh>, AppError> {
    Ok(runtime
        .snapshot()
        .await?
        .bruhs
        .into_iter()
        .map(|bruh| VaultBruh {
            agent_id: bruh.agent_id,
            name: bruh.name,
        })
        .collect())
}

/// `GET /api/vault`. Owner-only like the changes: usernames, sign-in
/// pages, `op://` references and project folders are the owner's to see.
/// Bruh names the runtime can't give leave `bruhs` empty, not the page.
pub async fn list(
    State(state): State<Arc<AppState>>,
    State(config): State<Arc<Config>>,
    runtime: Option<Extension<ReactiveRuntime>>,
    headers: HeaderMap,
) -> Result<Json<VaultListing>, AppError> {
    owner_only(&config.access, &headers)?;
    let mut listing = state.secrets().vault_listing().await.map_err(vault_error)?;
    if let Some(Extension(runtime)) = runtime {
        listing.bruhs = bruhs(&runtime).await.unwrap_or_else(|error| {
            tracing::warn!(%error, "The vault listing has no bruh names");
            Vec::new()
        });
    }
    Ok(Json(listing))
}

/// `POST /api/vault`. A Project folder is kept by its canonical path, and
/// a Bruh entry needs a bruh the runtime has.
pub async fn create(
    State(state): State<Arc<AppState>>,
    State(config): State<Arc<Config>>,
    runtime: Option<Extension<ReactiveRuntime>>,
    headers: HeaderMap,
    create: Result<Json<VaultCreate>, JsonRejection>,
) -> Result<Json<VaultEntry>, AppError> {
    owner_only(&config.access, &headers)?;
    let mut create = body(create)?;
    if let Err(refused) = scope_checked(&mut create.scope, runtime).await {
        // Its values are wiped as the store wipes a create it refuses.
        bruhs_core::secrets::discard(create);
        return Err(refused);
    }
    state
        .secrets()
        .vault_create(create)
        .await
        .map(Json)
        .map_err(vault_error)
}

/// A Project folder made canonical, or a Bruh scope's bruh found in the
/// runtime. An empty folder or bruh is left for the store's own copy.
async fn scope_checked(
    scope: &mut CredentialScope,
    runtime: Option<Extension<ReactiveRuntime>>,
) -> Result<(), AppError> {
    match scope {
        CredentialScope::Project { path } if !path.trim().is_empty() => {
            *path = project_folder(path).await?;
        }
        CredentialScope::Bruh { agent_id } if !agent_id.trim().is_empty() => {
            let runtime = super::reactive::runtime(runtime)?;
            if !bruhs(&runtime)
                .await?
                .iter()
                .any(|bruh| &bruh.agent_id == agent_id)
            {
                return Err(AppError::BadRequest(BRUH_MISSING.into()));
            }
        }
        CredentialScope::Global
        | CredentialScope::Project { .. }
        | CredentialScope::Bruh { .. } => {}
    }
    Ok(())
}

/// `path` as the canonical folder thread requests match against, or
/// [`PROJECT_MISSING`] when it isn't a folder.
async fn project_folder(path: &str) -> Result<String, AppError> {
    let missing = || AppError::BadRequest(PROJECT_MISSING.into());
    let canonical = tokio::fs::canonicalize(path).await.map_err(|_| missing())?;
    let folder = tokio::fs::metadata(&canonical)
        .await
        .is_ok_and(|metadata| metadata.is_dir());
    if !folder {
        return Err(missing());
    }
    canonical
        .into_os_string()
        .into_string()
        .map_err(|_| missing())
}

/// `PUT /api/vault/{id}`: new values or sign-in page; name, scope, kind
/// and a login's origin stay.
pub async fn update(
    State(state): State<Arc<AppState>>,
    State(config): State<Arc<Config>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    update: Result<Json<VaultUpdate>, JsonRejection>,
) -> Result<Json<VaultEntry>, AppError> {
    owner_only(&config.access, &headers)?;
    let update = body(update)?;
    state
        .secrets()
        .vault_update(&id, update)
        .await
        .map(Json)
        .map_err(vault_error)
}

/// `DELETE /api/vault/{id}`: the entry, its keychain item, and its id in
/// every bruh's grants.
pub async fn delete(
    State(state): State<Arc<AppState>>,
    State(config): State<Arc<Config>>,
    runtime: Option<Extension<ReactiveRuntime>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, AppError> {
    owner_only(&config.access, &headers)?;
    state
        .secrets()
        .vault_delete(&id)
        .await
        .map_err(vault_error)?;
    if let Some(Extension(runtime)) = runtime {
        forget_grants(&runtime, &id).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Take a deleted entry's id out of every bruh's grants in one store
/// transaction, so it doesn't count toward their limit and a grant change
/// made meanwhile stays. A grant of a deleted entry grants nothing, so a
/// failure here is only logged.
async fn forget_grants(runtime: &ReactiveRuntime, id: &str) {
    if let Err(error) = runtime.forget_vault_entry(id).await {
        tracing::warn!(%error, "A deleted vault entry's grants could not be removed");
    }
}

/// `POST /api/vault/{id}/check`: reads each value once and drops it.
pub async fn check(
    State(state): State<Arc<AppState>>,
    State(config): State<Arc<Config>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<VaultCheck>, AppError> {
    owner_only(&config.access, &headers)?;
    state
        .secrets()
        .vault_check(&id)
        .await
        .map(Json)
        .map_err(vault_error)
}
