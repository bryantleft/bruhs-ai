//! UI mail reads use the canonical host cache; provider sync retains broker access.
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
        gmail::{GmailClient, GmailError},
        CacheError, MailError, MailRuntime,
    },
    AppState,
};
use bruhs_models::{connections::ServiceProvider, mail::*};
use serde::Deserialize;
use std::sync::Arc;

mod compose;
mod outlook;
#[cfg(test)]
mod tests;

pub fn router() -> axum::Router<crate::config::ServerState> {
    use axum::routing::{get, post};
    axum::Router::new()
        .merge(compose::router())
        .merge(outlook::router())
        .route("/api/mail", get(mailboxes))
        .route("/api/mail/mailboxes/{mailbox}/threads", get(browse))
        .route(
            "/api/mail/mailboxes/{mailbox}/threads/{external}",
            get(thread),
        )
        .route("/api/mail/gmail/sync", post(sync_gmail))
        .route(
            "/api/mail/mailboxes/{mailbox}/threads/{external}/attachments/{attachment}",
            get(attachment),
        )
}

fn runtime(state: &AppState) -> Result<&MailRuntime, AppError> {
    state.mail().ok_or_else(|| {
        AppError::Unavailable(
            "Mail storage is unavailable on this server. Restart it and retry.".into(),
        )
    })
}

fn error(error: MailError) -> AppError {
    let message = error.to_string();
    match error {
        MailError::Cache(CacheError::UnknownMailbox) => AppError::NotFound(message),
        MailError::Cache(CacheError::HostChanged | CacheError::RevisionConflict) => {
            AppError::Conflict(message)
        }
        MailError::Cache(
            CacheError::Storage(_) | CacheError::UnsupportedSchema | CacheError::CorruptRecord,
        )
        | MailError::Storage
        | MailError::Worker
        | MailError::AlreadyOwned => AppError::Unavailable(message),
        MailError::Authorization(_) => AppError::Forbidden(message),
        _ => AppError::BadRequest(message),
    }
}

pub async fn mailboxes(
    State(state): State<Arc<AppState>>,
) -> Result<Json<MailboxesSnapshot>, AppError> {
    runtime(&state)?.mailboxes().await.map(Json).map_err(error)
}

pub async fn browse(
    State(state): State<Arc<AppState>>,
    Path(mailbox): Path<String>,
    query: Result<Query<MailBrowseQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Json<MailThreadsPage>, AppError> {
    let Query(query) = query.map_err(|_| AppError::BadRequest("Invalid mailbox query.".into()))?;
    runtime(&state)?
        .browse(mailbox, query)
        .await
        .map(Json)
        .map_err(error)
}

#[derive(Deserialize)]
pub struct HostQuery {
    host_id: String,
}

#[derive(Deserialize)]
pub struct AttachmentQuery {
    host_id: String,
    connection_id: String,
}

pub async fn attachment(
    State(state): State<Arc<AppState>>,
    Extension(client): Extension<Arc<GmailClient>>,
    Path((mailbox, external, attachment)): Path<(String, String, String)>,
    query: Result<Query<AttachmentQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, MailApiError> {
    let Query(query) = query.map_err(|_| {
        AppError::BadRequest(
            "Choose the server and Gmail connection that own this attachment.".into(),
        )
    })?;
    let runtime = runtime(&state)?;
    if runtime.host().as_str() != query.host_id {
        return Err(MailError::Cache(CacheError::HostChanged).into());
    }
    let session = runtime
        .session(
            state.secrets().connections(),
            ServiceProvider::Gmail,
            &query.connection_id,
        )
        .await?;
    if session.authorization().mailbox().as_str() != mailbox {
        return Err(AppError::Conflict(
            "Choose the Gmail account that owns this attachment.".into(),
        )
        .into());
    }
    let attachment = client
        .attachment(&session, &external, &attachment)
        .await
        .map_err(MailApiError::Provider)?;
    drop(session);
    // No MIME or filename supplied by an email can turn this response into an
    // inline document or inject headers. Clients use the saved metadata to name
    // an explicitly downloaded file, after their own destination review.
    Ok((
        [
            (CONTENT_TYPE, "application/octet-stream"),
            (CONTENT_DISPOSITION, "attachment"),
            (X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        attachment.bytes,
    )
        .into_response())
}

pub async fn thread(
    State(state): State<Arc<AppState>>,
    Path((mailbox, external)): Path<(String, String)>,
    query: Result<Query<HostQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Json<ConnectedMailThread>, AppError> {
    let Query(query) = query.map_err(|_| {
        AppError::BadRequest("Choose the mail server that owns this thread.".into())
    })?;
    runtime(&state)?
        .connected_thread(query.host_id, mailbox, external)
        .await
        .map_err(error)?
        .map(Json)
        .ok_or_else(|| {
            AppError::NotFound(
                "This cached mail thread is not available. Refresh the mailbox.".into(),
            )
        })
}

pub enum MailApiError {
    App(AppError),
    Provider(GmailError),
}
impl From<AppError> for MailApiError {
    fn from(error: AppError) -> Self {
        Self::App(error)
    }
}
impl From<MailError> for MailApiError {
    fn from(value: MailError) -> Self {
        Self::App(error(value))
    }
}

impl IntoResponse for MailApiError {
    fn into_response(self) -> Response {
        let provider = match self {
            Self::App(error) => return error.into_response(),
            Self::Provider(error) => error,
        };
        let message = provider.to_string();
        match provider {
            GmailError::RateLimited {
                retry_after_seconds,
            } => {
                let mut response=(StatusCode::TOO_MANY_REQUESTS,Json(serde_json::json!({"error":message,"retry_after_seconds":retry_after_seconds}))).into_response();
                if let Some(seconds) = retry_after_seconds {
                    if let Ok(value) = seconds.to_string().parse() {
                        response.headers_mut().insert(RETRY_AFTER, value);
                    }
                }
                response
            }
            GmailError::Mail(cause) => error(cause).into_response(),
            GmailError::SignIn | GmailError::Permission | GmailError::DomainPolicy => {
                AppError::Forbidden(message).into_response()
            }
            GmailError::NotFound => AppError::NotFound(message).into_response(),
            GmailError::InvalidCheckpoint => AppError::Conflict(message).into_response(),
            GmailError::WrongProvider | GmailError::TooLarge | GmailError::UnsupportedEncoding => {
                AppError::BadRequest(message).into_response()
            }
            _ => AppError::Unavailable(message).into_response(),
        }
    }
}

pub async fn sync_gmail(
    State(state): State<Arc<AppState>>,
    Extension(client): Extension<Arc<GmailClient>>,
    body: Result<Json<GmailSyncRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<MailSyncResult>, MailApiError> {
    let Json(request) =
        body.map_err(|_| AppError::BadRequest("Invalid Gmail sync request.".into()))?;
    let runtime = runtime(&state)?;
    if request.host_id != runtime.host().as_str() {
        return Err(MailError::Cache(CacheError::HostChanged).into());
    }
    let session = runtime
        .session(
            state.secrets().connections(),
            ServiceProvider::Gmail,
            &request.connection_id,
        )
        .await?;
    let mailbox = session.authorization().mailbox().as_str().to_owned();
    if request
        .mailbox_id
        .as_ref()
        .is_some_and(|expected| *expected != mailbox)
    {
        return Err(AppError::Conflict("The connected Gmail account differs from this cached mailbox. Select the connected account before syncing.".into()).into());
    }
    let result = if request.restart {
        client.restart_sync_page(&session).await
    } else {
        client.sync_page(&session).await
    }
    .map_err(MailApiError::Provider)?;
    drop(session);
    Ok(Json(MailSyncResult {
        host_id: runtime.host().as_str().to_owned(),
        mailbox_id: mailbox,
        revision: result.revision,
        threads_updated: result.threads_updated,
        full_scan_complete: result.full_scan_complete,
        caught_up: result.caught_up,
    }))
}
