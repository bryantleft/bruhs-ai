//! Local drafts and explicit reviewed sends. This module is wired only after
//! both provider adapters implement the common one-attempt transport contract.

use super::{error, runtime};
use crate::error::AppError;
use axum::{
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::{header::RETRY_AFTER, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use bruhs_core::{
    mail::{
        compose::{ComposeError, MailSendProblemKind, MailSendTransport},
        gmail::GmailClient,
        outlook::OutlookClient,
        CacheError, DraftAttachmentUpload, MailError,
    },
    AppState,
};
use bruhs_models::{connections::ServiceProvider, mail::*};
use serde::Deserialize;
use std::sync::Arc;

mod prequeue;
mod reconciliation;

pub(super) fn router() -> axum::Router<crate::config::ServerState> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route(
            "/api/mail/mailboxes/{mailbox}/drafts",
            get(list).post(create),
        )
        .route(
            "/api/mail/mailboxes/{mailbox}/drafts/{client}",
            get(read).put(save),
        )
        .route(
            "/api/mail/mailboxes/{mailbox}/drafts/{client}/review",
            post(review),
        )
        .route(
            "/api/mail/mailboxes/{mailbox}/drafts/{client}/send",
            post(send),
        )
        .route(
            "/api/mail/mailboxes/{mailbox}/drafts/{client}/files/{file}",
            post(upload)
                .delete(remove_file)
                .layer(DefaultBodyLimit::max(
                    MAIL_DRAFT_ATTACHMENT_MAX_BYTES + 65536,
                )),
        )
        .route(
            "/api/mail/mailboxes/{mailbox}/sends/{operation}",
            get(send_status),
        )
        .route(
            "/api/mail/mailboxes/{mailbox}/sends/{operation}/cancel",
            post(cancel),
        )
        .route(
            "/api/mail/mailboxes/{mailbox}/sends/{operation}/reconcile",
            post(reconciliation::reconcile),
        )
        .layer(DefaultBodyLimit::max(8 << 20))
}

pub(super) enum Error {
    App(AppError),
    Compose(ComposeError),
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
impl From<ComposeError> for Error {
    fn from(value: ComposeError) -> Self {
        Self::Compose(value)
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let error = match self {
            Self::App(error) => return error.into_response(),
            Self::Compose(error) => error,
        };
        let message = error.to_string();
        match error {
            ComposeError::Mail(error) => super::error(error).into_response(),
            ComposeError::StaleReview | ComposeError::SendPending => {
                AppError::Conflict(message).into_response()
            }
            ComposeError::Provider(problem) => {
                let status = match problem.kind {
                    MailSendProblemKind::SignIn | MailSendProblemKind::Permission => {
                        StatusCode::FORBIDDEN
                    }
                    MailSendProblemKind::RateLimited => StatusCode::TOO_MANY_REQUESTS,
                    MailSendProblemKind::AccountChanged => StatusCode::CONFLICT,
                    MailSendProblemKind::Unavailable | MailSendProblemKind::Unconfirmed => {
                        StatusCode::SERVICE_UNAVAILABLE
                    }
                    MailSendProblemKind::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
                    MailSendProblemKind::InvalidMessage => StatusCode::BAD_REQUEST,
                };
                let mut response = (
                    status,
                    Json(serde_json::json!({"error":message,"problem":problem})),
                )
                    .into_response();
                if let Some(seconds) = problem.retry_after_seconds {
                    if let Ok(value) = seconds.to_string().parse() {
                        response.headers_mut().insert(RETRY_AFTER, value);
                    }
                }
                response
            }
            ComposeError::PreparedCorrupt | ComposeError::PreparationWorker => {
                AppError::Unavailable(message).into_response()
            }
            _ => AppError::BadRequest(message).into_response(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Host {
    host_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    host_id: String,
    #[serde(default)]
    page: u64,
    expected_revision: Option<i64>,
}

fn invalid() -> AppError {
    AppError::BadRequest("The email request could not be read. No send was started.".into())
}

async fn list(
    State(state): State<Arc<AppState>>,
    Path(mailbox): Path<String>,
    query: Result<Query<Page>, axum::extract::rejection::QueryRejection>,
) -> Result<Json<MailDraftsPage>, Error> {
    let Query(query) = query.map_err(|_| invalid())?;
    Ok(Json(
        runtime(&state)?
            .compose_drafts(query.host_id, mailbox, query.page, query.expected_revision)
            .await?,
    ))
}

async fn create(
    State(state): State<Arc<AppState>>,
    Path(mailbox): Path<String>,
    body: Result<Json<MailDraftCreate>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<MailComposeDraft>, Error> {
    let Json(body) = body.map_err(|_| invalid())?;
    Ok(Json(runtime(&state)?.compose_create(mailbox, body).await?))
}

async fn read(
    State(state): State<Arc<AppState>>,
    Path((mailbox, client)): Path<(String, String)>,
    query: Result<Query<Host>, axum::extract::rejection::QueryRejection>,
) -> Result<Json<MailComposeDraft>, Error> {
    let Query(query) = query.map_err(|_| invalid())?;
    runtime(&state)?.compose_draft(query.host_id,mailbox,client).await?.map(Json).ok_or_else(||AppError::NotFound("This editable draft is no longer available. Open its send status or start a new email.".into()).into())
}

async fn save(
    State(state): State<Arc<AppState>>,
    Path((mailbox, client)): Path<(String, String)>,
    body: Result<Json<MailDraftSave>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<MailComposeDraft>, Error> {
    let Json(body) = body.map_err(|_| invalid())?;
    Ok(Json(
        runtime(&state)?.compose_save(mailbox, client, body).await?,
    ))
}

async fn provider(
    state: &AppState,
    mailbox: &str,
    gmail: Arc<GmailClient>,
    outlook: Arc<OutlookClient>,
) -> Result<Arc<dyn MailSendTransport>, Error> {
    let snapshot = runtime(state)?.mailboxes().await?;
    let selected = snapshot
        .mailboxes
        .iter()
        .find(|row| row.mailbox_id == mailbox)
        .ok_or(MailError::Cache(CacheError::UnknownMailbox))?;
    match selected.provider {
        ServiceProvider::Gmail => Ok(gmail),
        ServiceProvider::Outlook => Ok(outlook),
        _ => Err(AppError::BadRequest("Choose a Gmail or Outlook mail account.".into()).into()),
    }
}

async fn review(
    State(state): State<Arc<AppState>>,
    Extension(gmail): Extension<Arc<GmailClient>>,
    Extension(outlook): Extension<Arc<OutlookClient>>,
    Path((mailbox, client)): Path<(String, String)>,
    body: Result<Json<MailReviewRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<MailSendReview>, Error> {
    let Json(body) = body.map_err(|_| invalid())?;
    let runtime = runtime(&state)?;
    if runtime.host().as_str() != body.host_id {
        return Err(MailError::Cache(CacheError::HostChanged).into());
    }
    let provider = provider(&state, &mailbox, gmail, outlook).await?;
    let session = runtime
        .session(
            state.secrets().connections(),
            provider.provider(),
            &body.connection_id,
        )
        .await?;
    if session.authorization().mailbox().as_str() != mailbox {
        return Err(
            AppError::Conflict("Choose the account that owns this saved draft.".into()).into(),
        );
    }
    let sender = provider
        .verified_sender(&session)
        .await
        .map_err(ComposeError::from)?;
    let review = session
        .review_compose_with_limit(&sender, client, body, provider.mime_limit_bytes())
        .await?;
    drop(session);
    Ok(Json(review))
}

async fn send(
    State(state): State<Arc<AppState>>,
    Extension(gmail): Extension<Arc<GmailClient>>,
    Extension(outlook): Extension<Arc<OutlookClient>>,
    Path((mailbox, client)): Path<(String, String)>,
    body: Result<Json<MailSendRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<MailSendStatus>, Error> {
    let Json(body) = body.map_err(|_| invalid())?;
    let runtime = runtime(&state)?;
    if runtime.host().as_str() != body.host_id {
        return Err(MailError::Cache(CacheError::HostChanged).into());
    }
    let provider = provider(&state, &mailbox, gmail, outlook).await?;
    let session = match runtime
        .session(
            state.secrets().connections(),
            provider.provider(),
            &body.connection_id,
        )
        .await
    {
        Ok(session) => session,
        Err(error) => {
            prequeue::authorization_failed(runtime, &mailbox, &client, &body, &error).await;
            return Err(error.into());
        }
    };
    if session.authorization().mailbox().as_str() != mailbox {
        return Err(
            AppError::Conflict("Choose the account that owns this reviewed email.".into()).into(),
        );
    }
    Ok(Json(
        runtime
            .send_reviewed_compose(session, provider, client, body)
            .wait()
            .await?,
    ))
}

async fn send_status(
    State(state): State<Arc<AppState>>,
    Path((mailbox, operation)): Path<(String, String)>,
    query: Result<Query<Host>, axum::extract::rejection::QueryRejection>,
) -> Result<Json<MailSendStatus>, Error> {
    let Query(query) = query.map_err(|_| invalid())?;
    runtime(&state)?.compose_send_status(query.host_id,mailbox,operation).await?.map(Json).ok_or_else(||AppError::NotFound("No saved status is available for this send. Check Sent mail before starting another send.".into()).into())
}

async fn cancel(
    State(state): State<Arc<AppState>>,
    Path((mailbox, operation)): Path<(String, String)>,
    body: Result<Json<Host>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<MailSendStatus>, Error> {
    let Json(body) = body.map_err(|_| invalid())?;
    Ok(Json(
        runtime(&state)?
            .cancel_reviewed_compose(body.host_id, mailbox, operation)
            .await?,
    ))
}

async fn upload(
    State(state): State<Arc<AppState>>,
    Path((mailbox, client, file)): Path<(String, String, String)>,
    query: Result<Query<Host>, axum::extract::rejection::QueryRejection>,
    multipart: Result<Multipart, axum::extract::multipart::MultipartRejection>,
) -> Result<Json<MailDraftFile>, Error> {
    let Query(query) = query.map_err(|_| invalid())?;
    let runtime = runtime(&state)?;
    if query.host_id != runtime.host().as_str() {
        return Err(MailError::Cache(CacheError::HostChanged).into());
    }
    let mut multipart = multipart.map_err(|_| invalid())?;
    let mut field = multipart
        .next_field()
        .await
        .map_err(|_| invalid())?
        .ok_or_else(invalid)?;
    if field.name() != Some("file") {
        return Err(invalid().into());
    }
    let name = field.file_name().unwrap_or("attachment").to_owned();
    if name.len() > 4096 {
        return Err(invalid().into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = field.chunk().await.map_err(|_| invalid())? {
        if bytes
            .len()
            .checked_add(chunk.len())
            .is_none_or(|size| size > MAIL_DRAFT_ATTACHMENT_MAX_BYTES)
        {
            return Err(
                AppError::BadRequest("Keep draft files within 16 MiB combined.".into()).into(),
            );
        }
        bytes.extend_from_slice(&chunk);
    }
    drop(field);
    if multipart
        .next_field()
        .await
        .map_err(|_| invalid())?
        .is_some()
    {
        return Err(invalid().into());
    }
    Ok(Json(
        runtime
            .compose_upload(
                query.host_id,
                mailbox,
                client,
                DraftAttachmentUpload {
                    client_id: file,
                    file_name: name,
                    bytes,
                },
            )
            .await?,
    ))
}

async fn remove_file(
    State(state): State<Arc<AppState>>,
    Path((mailbox, client, file)): Path<(String, String, String)>,
    query: Result<Query<Host>, axum::extract::rejection::QueryRejection>,
) -> Result<Json<()>, Error> {
    let Query(query) = query.map_err(|_| invalid())?;
    runtime(&state)?
        .compose_remove_upload(query.host_id, mailbox, client, file)
        .await?;
    Ok(Json(()))
}

#[cfg(test)]
mod tests;
