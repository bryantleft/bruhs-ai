//! Settings, Apps over HTTP (D21, D23), for both apps.
//!
//! - `GET /api/apps`: every app account and connected mailbox, value free,
//!   and each bruh with its grants and why it can't use apps now
//!   ([`AppsListing`]).
//! - `POST /api/apps/accounts` (an [`AppConnect`]) and
//!   `PUT /api/apps/accounts/{id}` (an [`AppReplace`]): the saved
//!   [`AppAccount`], once the token's test call passed.
//! - `POST /api/apps/accounts/{id}/test`: the saved token's test call
//!   ([`AppTestResult`]).
//!
//! Disconnect is the vault's `DELETE /api/vault/{id}`, which also takes the
//! account out of every bruh's grants.
//!
//! Every route sits behind the shared loopback guard and takes only the
//! owner's own app ([`Policy::owner_request`]), GET included: account
//! names and grants are the owner's to see. Bodies that aren't one get
//! [`INVALID_APP_ACCOUNT`], never serde's text, which can quote a token; a
//! test call that fails is 422 with its copy. Errors are fixed copy, and no
//! handler logs a body.

use std::sync::Arc;

use axum::{
    extract::{rejection::JsonRejection, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use bruhs_core::apps::{AppsError, AppsHost};
use bruhs_core::reactive::{ReactiveError, ReactiveRuntime};
use bruhs_core::secrets::SecretError;
use bruhs_models::agent::AgentProvider;
use bruhs_models::apps::{
    AppAccount, AppConnect, AppReplace, AppTestResult, AppsBruh, AppsListing, INVALID_APP_ACCOUNT,
};
use bruhs_models::crew::{apps_unavailable, CREW_PROTOCOL, KEYCHAIN_NEEDS_OWNER, OWNER_ONLY_APPS};
use bruhs_models::vault::{
    CredentialScope, VaultCreate, VaultEntry, VaultKind, VaultSecretInput, BRUH_MISSING,
};
use serde_json::json;

use crate::config::Config;
use crate::error::AppError;
use crate::local_access::Policy;

/// The largest Apps request body: a token and key of 4 KiB each with the
/// account's values, as the vault allows.
pub const MAX_BODY_BYTES: usize = super::vault::MAX_BODY_BYTES;

// Split so clippy's allow-unwrap-in-tests sees `cfg(test)`; the bruh
// route's tests share its fixture.
#[cfg(test)]
#[cfg(unix)]
pub(super) mod tests;

/// A refusal with a status [`AppError`] has no variant for (413, 422), or
/// any [`AppError`]. Its text is fixed copy.
#[derive(Debug)]
pub enum RouteError {
    App(AppError),
    Status(StatusCode, String),
}

impl RouteError {
    pub(super) fn status(status: StatusCode, message: impl Into<String>) -> Self {
        Self::Status(status, message.into())
    }

    pub(super) fn forbidden(message: impl Into<String>) -> Self {
        Self::App(AppError::Forbidden(message.into()))
    }

    pub(super) fn unavailable(message: impl Into<String>) -> Self {
        Self::App(AppError::Unavailable(message.into()))
    }
}

impl From<AppError> for RouteError {
    fn from(error: AppError) -> Self {
        Self::App(error)
    }
}

impl From<ReactiveError> for RouteError {
    fn from(error: ReactiveError) -> Self {
        Self::App(error.into())
    }
}

impl IntoResponse for RouteError {
    fn into_response(self) -> Response {
        match self {
            Self::App(error) => error.into_response(),
            Self::Status(status, message) => {
                (status, Json(json!({ "error": message }))).into_response()
            }
        }
    }
}

/// The status and copy for an app error that keeps its own fixed copy.
pub(super) fn app_status(error: &AppsError) -> Option<(StatusCode, &'static str)> {
    Some(match error {
        AppsError::Refused(message) => (StatusCode::FORBIDDEN, message),
        AppsError::Invalid(message) => (StatusCode::BAD_REQUEST, message),
        AppsError::Unavailable(message) => (StatusCode::SERVICE_UNAVAILABLE, message),
        AppsError::Unreachable(message) => (StatusCode::BAD_GATEWAY, message),
        AppsError::TooLarge(message) => (StatusCode::PAYLOAD_TOO_LARGE, message),
        AppsError::TimedOut(message) => (StatusCode::GATEWAY_TIMEOUT, message),
        AppsError::TestFailed(_) | AppsError::MailSync(_) | AppsError::Vault(_) => return None,
    })
}

/// The owner copy for an app error. A keychain that needs the owner gets
/// the vault's own copy, not the bruh's (ruling A Task 8), and an account
/// the vault no longer has is the vault's [`ENTRY_GONE`](bruhs_models::vault::ENTRY_GONE).
fn owner_error(error: AppsError) -> RouteError {
    match error {
        AppsError::Unavailable(KEYCHAIN_NEEDS_OWNER) => {
            RouteError::unavailable(SecretError::Storage.to_string())
        }
        AppsError::TestFailed(message) => {
            RouteError::status(StatusCode::UNPROCESSABLE_ENTITY, message)
        }
        AppsError::MailSync(message) => RouteError::unavailable(message),
        AppsError::Vault(error) => super::vault::vault_error(error).into(),
        error => match app_status(&error) {
            Some((status, message)) => RouteError::status(status, message),
            None => RouteError::unavailable(SecretError::Storage.to_string()),
        },
    }
}

/// Refuse any page but the owner's own app ([`OWNER_ONLY_APPS`]).
fn owner_apps(policy: &Policy, headers: &HeaderMap) -> Result<(), RouteError> {
    if policy.owner_request(headers) {
        Ok(())
    } else {
        Err(RouteError::forbidden(OWNER_ONLY_APPS))
    }
}

/// The process's apps host, which `main` attaches.
fn host(apps: Option<Extension<AppsHost>>) -> Result<AppsHost, RouteError> {
    apps.map(|Extension(apps)| apps)
        .ok_or_else(|| RouteError::unavailable(bruhs_models::apps::APPS_NOT_ON_SERVER))
}

/// An app account as the page lists it, from its saved entry.
fn account(entry: VaultEntry) -> Result<AppAccount, RouteError> {
    match entry.kind {
        VaultKind::AppToken(meta) => Ok(AppAccount {
            id: entry.id,
            app: meta.app,
            name: entry.name,
            account: meta.account,
            scope: entry.scope,
            state: entry.state,
            attention: entry.attention,
        }),
        _ => Err(AppError::BadRequest(INVALID_APP_ACCOUNT.into()).into()),
    }
}

/// Wipes a connect body the route refused, as the store wipes a create it
/// refuses.
fn discard_connect(connect: AppConnect) {
    let AppConnect {
        app,
        params,
        token,
        key,
        ..
    } = connect;
    bruhs_core::secrets::discard(VaultCreate {
        name: String::new(),
        scope: CredentialScope::Global,
        secret: VaultSecretInput::AppToken {
            app,
            account: String::new(),
            params,
            token: Some(token),
            key,
        },
    });
}

/// Every bruh with its grants, and why it can't use apps at its stored
/// access and model (D2).
async fn bruhs(runtime: &ReactiveRuntime) -> Result<Vec<AppsBruh>, ReactiveError> {
    let snapshot = runtime.snapshot().await?;
    Ok(snapshot
        .bruhs
        .into_iter()
        .map(|bruh| {
            let provider = snapshot
                .agents
                .iter()
                .find(|agent| agent.id == bruh.agent_id)
                .map_or(AgentProvider::Claude, |agent| {
                    AgentProvider::for_model(&agent.model)
                });
            AppsBruh {
                unavailable: apps_unavailable(bruh.access.level, provider).map(str::to_owned),
                agent_id: bruh.agent_id,
                name: bruh.name,
                apps: bruh.apps,
                mail: bruh.mail,
            }
        })
        .collect())
}

/// `GET /api/apps`. Bruhs the runtime can't list leave `bruhs` empty, not
/// the page.
pub async fn list(
    State(config): State<Arc<Config>>,
    apps: Option<Extension<AppsHost>>,
    runtime: Option<Extension<ReactiveRuntime>>,
    headers: HeaderMap,
) -> Result<Json<AppsListing>, RouteError> {
    owner_apps(&config.access, &headers)?;
    let apps = host(apps)?;
    let accounts = apps.accounts().await.map_err(owner_error)?;
    let mailboxes = apps.mailboxes().await.map_err(owner_error)?;
    let bruhs = match runtime {
        Some(Extension(runtime)) => bruhs(&runtime).await.unwrap_or_else(|error| {
            tracing::warn!(%error, "The apps listing has no bruhs");
            Vec::new()
        }),
        None => Vec::new(),
    };
    Ok(Json(AppsListing {
        accounts,
        mailboxes,
        bruhs,
        protocol: CREW_PROTOCOL,
    }))
}

/// A Bruh scope's bruh, found in the runtime before the test call runs.
async fn bruh_exists(
    scope: &CredentialScope,
    runtime: Option<&Extension<ReactiveRuntime>>,
) -> Result<(), RouteError> {
    let CredentialScope::Bruh { agent_id } = scope else {
        return Ok(());
    };
    // An empty one is left for the host's own copy.
    if agent_id.trim().is_empty() {
        return Ok(());
    }
    let runtime = super::reactive::runtime(runtime.cloned())?;
    let snapshot = runtime.snapshot().await?;
    if snapshot.bruhs.iter().any(|bruh| &bruh.agent_id == agent_id) {
        Ok(())
    } else {
        Err(AppError::BadRequest(BRUH_MISSING.into()).into())
    }
}

/// `POST /api/apps/accounts`: tests the token, then saves it. A Bruh
/// account needs a bruh the runtime has, before and after the save.
pub async fn connect(
    State(config): State<Arc<Config>>,
    apps: Option<Extension<AppsHost>>,
    runtime: Option<Extension<ReactiveRuntime>>,
    headers: HeaderMap,
    body: Result<Json<AppConnect>, JsonRejection>,
) -> Result<Json<AppAccount>, RouteError> {
    owner_apps(&config.access, &headers)?;
    let Json(connect) = body.map_err(|_| AppError::BadRequest(INVALID_APP_ACCOUNT.into()))?;
    let apps = match host(apps) {
        Ok(apps) => apps,
        Err(refused) => {
            discard_connect(connect);
            return Err(refused);
        }
    };
    if let Err(refused) = bruh_exists(&connect.scope, runtime.as_ref()).await {
        discard_connect(connect);
        return Err(refused);
    }
    let entry = apps.connect(connect).await.map_err(owner_error)?;
    // The bruh may have been deleted during the test call: then the token
    // goes with its others (as the vault's create does).
    if let (CredentialScope::Bruh { agent_id }, Some(Extension(runtime))) = (&entry.scope, &runtime)
    {
        match runtime.keep_bruh_entry(agent_id).await {
            Ok(true) => {}
            Ok(false) => return Err(AppError::BadRequest(BRUH_MISSING.into()).into()),
            Err(error) => {
                tracing::warn!(%error, "A new bruh app account's bruh could not be checked again");
            }
        }
    }
    account(entry).map(Json)
}

/// `PUT /api/apps/accounts/{id}`: a new token for the same account, once
/// it passes the test call. Name, scope, values and grants stay.
pub async fn replace(
    State(config): State<Arc<Config>>,
    apps: Option<Extension<AppsHost>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Result<Json<AppReplace>, JsonRejection>,
) -> Result<Json<AppAccount>, RouteError> {
    owner_apps(&config.access, &headers)?;
    let Json(replace) = body.map_err(|_| AppError::BadRequest(INVALID_APP_ACCOUNT.into()))?;
    // `replace` moves into `Zeroizing` inside the host, refused or not.
    let apps = host(apps)?;
    let entry = apps.replace(&id, replace).await.map_err(owner_error)?;
    account(entry).map(Json)
}

/// `POST /api/apps/accounts/{id}/test`: the saved token's test call. A
/// refused token is `ok: false` with its copy; an entry the vault can't
/// read is an error.
pub async fn test(
    State(config): State<Arc<Config>>,
    apps: Option<Extension<AppsHost>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<AppTestResult>, RouteError> {
    owner_apps(&config.access, &headers)?;
    host(apps)?
        .test_saved(&id)
        .await
        .map(Json)
        .map_err(owner_error)
}

/// `POST /api/vault/{id}/check` on an app token: its test call, as the
/// vault's [`VaultCheck`](bruhs_models::vault::VaultCheck) shows it.
pub(super) async fn check_saved(
    apps: &AppsHost,
    id: &str,
) -> Result<bruhs_models::vault::VaultCheck, RouteError> {
    let result = apps.test_saved(id).await.map_err(owner_error)?;
    Ok(bruhs_models::vault::VaultCheck {
        ok: result.ok,
        message: result.message,
    })
}
