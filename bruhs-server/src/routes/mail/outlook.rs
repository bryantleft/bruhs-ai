//! Outlook transport shares the canonical cache, not a second mailbox store.
use super::{error, runtime, AttachmentQuery};
use crate::error::AppError;
use axum::{
    extract::{Path, Query, State},
    http::{
        header::{CONTENT_DISPOSITION, CONTENT_TYPE, RETRY_AFTER, X_CONTENT_TYPE_OPTIONS},
        StatusCode,
    },
    response::{IntoResponse, Response},
    Extension, Json,
};
use bruhs_core::{
    mail::{
        outlook::{OutlookClient, OutlookError},
        CacheError, MailError,
    },
    AppState,
};
// The existing sync wire shape is shared; its Gmail name remains source-compatible.
use bruhs_models::{
    connections::ServiceProvider,
    mail::{GmailSyncRequest as SyncRequest, MailSyncResult},
};
use std::sync::Arc;

#[cfg(test)]
mod tests;

pub(super) fn router() -> axum::Router<crate::config::ServerState> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/api/mail/outlook/sync", post(sync))
        .route(
            "/api/mail/outlook/mailboxes/{mailbox}/threads/{external}/attachments/{attachment}",
            get(attachment),
        )
}

pub(super) enum Error {
    App(AppError),
    Provider(OutlookError),
}

impl From<AppError> for Error {
    fn from(value: AppError) -> Self {
        Self::App(value)
    }
}
impl From<MailError> for Error {
    fn from(value: MailError) -> Self {
        Self::App(error(value))
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let provider = match self {
            Self::App(error) => return error.into_response(),
            Self::Provider(provider) => provider,
        };
        let message = provider.to_string();
        match provider {
            OutlookError::Mail(cause) => error(cause).into_response(),
            OutlookError::RateLimited {
                retry_after_seconds,
            } => {
                let mut response = (StatusCode::TOO_MANY_REQUESTS, Json(serde_json::json!({"error":message,"retry_after_seconds":retry_after_seconds}))).into_response();
                if let Some(seconds) = retry_after_seconds {
                    if let Ok(value) = seconds.to_string().parse() {
                        response.headers_mut().insert(RETRY_AFTER, value);
                    }
                }
                response
            }
            OutlookError::SignIn | OutlookError::Permission | OutlookError::UnsupportedAccount => {
                AppError::Forbidden(message).into_response()
            }
            OutlookError::NotFound => AppError::NotFound(message).into_response(),
            OutlookError::InvalidCheckpoint | OutlookError::SyncExpired => {
                AppError::Conflict(message).into_response()
            }
            OutlookError::WrongProvider
            | OutlookError::TooLarge
            | OutlookError::UnsupportedAttachment => AppError::BadRequest(message).into_response(),
            _ => AppError::Unavailable(message).into_response(),
        }
    }
}

async fn sync(
    State(state): State<Arc<AppState>>,
    Extension(client): Extension<Arc<OutlookClient>>,
    body: Result<Json<SyncRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<MailSyncResult>, Error> {
    let Json(request) =
        body.map_err(|_| AppError::BadRequest("Invalid Outlook sync request.".into()))?;
    let runtime = runtime(&state)?;
    if request.host_id != runtime.host().as_str() {
        return Err(MailError::Cache(CacheError::HostChanged).into());
    }
    let session = runtime
        .session(
            state.secrets().connections(),
            ServiceProvider::Outlook,
            &request.connection_id,
        )
        .await?;
    let mailbox_id = session.authorization().mailbox().as_str().to_owned();
    if request
        .mailbox_id
        .as_ref()
        .is_some_and(|expected| *expected != mailbox_id)
    {
        return Err(AppError::Conflict("The connected Outlook account differs from this cached mailbox. Select the connected account before syncing.".into()).into());
    }
    let result = if request.restart {
        client.restart_sync_page(&session).await
    } else {
        client.sync_page(&session).await
    }
    .map_err(Error::Provider)?;
    drop(session);
    Ok(Json(MailSyncResult {
        host_id: runtime.host().as_str().into(),
        mailbox_id,
        revision: result.revision,
        threads_updated: result.threads_updated,
        full_scan_complete: result.full_scan_complete,
        caught_up: result.caught_up,
    }))
}

async fn attachment(
    State(state): State<Arc<AppState>>,
    Extension(client): Extension<Arc<OutlookClient>>,
    Path((mailbox, external, attachment)): Path<(String, String, String)>,
    query: Result<Query<AttachmentQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, Error> {
    let Query(query) = query.map_err(|_| {
        AppError::BadRequest(
            "Choose the server and Outlook connection that own this attachment.".into(),
        )
    })?;
    let runtime = runtime(&state)?;
    if runtime.host().as_str() != query.host_id {
        return Err(MailError::Cache(CacheError::HostChanged).into());
    }
    let session = runtime
        .session(
            state.secrets().connections(),
            ServiceProvider::Outlook,
            &query.connection_id,
        )
        .await?;
    if session.authorization().mailbox().as_str() != mailbox {
        return Err(AppError::Conflict(
            "Choose the Outlook account that owns this attachment.".into(),
        )
        .into());
    }
    let result = client
        .attachment(&session, &external, &attachment)
        .await
        .map_err(Error::Provider)?;
    drop(session);
    let mut response = (
        [
            (CONTENT_TYPE, "application/octet-stream"),
            (CONTENT_DISPOSITION, "attachment"),
            (X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        result.bytes,
    )
        .into_response();
    if let Some(size) = result.item_export_size {
        response.headers_mut().insert(
            "x-bruhs-mail-export-size",
            size.to_string().parse().map_err(|_| {
                AppError::Unavailable("The attachment export size could not be represented.".into())
            })?,
        );
    }
    Ok(response)
}
