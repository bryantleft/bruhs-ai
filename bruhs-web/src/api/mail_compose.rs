//! Compose-only requests. Ordinary mail navigation and its cancellation policy
//! stay in api/mail.rs; these writes belong to app-owned draft/send jobs.

use super::{
    http::{decode, send_request, url},
    segment, ApiError,
};
use bruhs_models::mail::*;
use gloo_net::http::{Method, RequestBuilder};
use serde::{de::DeserializeOwned, Serialize};

const READ_MS: u32 = 20_000;
const WORK_MS: u32 = 180_000;

async fn request<T: DeserializeOwned, B: Serialize>(
    method: Method,
    route: &str,
    body: Option<&B>,
    timeout_ms: u32,
) -> Result<T, ApiError> {
    let abort = web_sys::AbortController::new()
        .map_err(|_| ApiError::Network("The browser cannot manage this email request.".into()))?;
    let operation = async {
        let builder = RequestBuilder::new(&url(route))
            .method(method)
            .abort_signal(Some(&abort.signal()));
        let request = match body {
            Some(body) => builder.json(body)?,
            None => builder.build()?,
        };
        decode(send_request(request).await?).await
    };
    futures_util::pin_mut!(operation);
    let deadline = gloo_timers::future::TimeoutFuture::new(timeout_ms);
    futures_util::pin_mut!(deadline);
    let result = match futures_util::future::select(operation, deadline).await {
        futures_util::future::Either::Left((result, _)) => result,
        futures_util::future::Either::Right(_) => Err(ApiError::Timeout {
            after_ms: timeout_ms,
        }),
    };
    abort.abort();
    result
}

fn drafts(mailbox: &str) -> String {
    format!("/api/mail/mailboxes/{}/drafts", segment(mailbox))
}
fn draft(mailbox: &str, client: &str) -> String {
    format!("{}/{}", drafts(mailbox), segment(client))
}

pub async fn mail_draft_create(
    mailbox: &str,
    body: &MailDraftCreate,
) -> Result<MailComposeDraft, ApiError> {
    request(Method::POST, &drafts(mailbox), Some(body), READ_MS).await
}

pub async fn mail_drafts(
    host: &str,
    mailbox: &str,
    page: u64,
    expected_revision: Option<i64>,
) -> Result<MailDraftsPage, ApiError> {
    let mut route = format!("{}?host_id={}&page={page}", drafts(mailbox), segment(host));
    if let Some(revision) = expected_revision {
        route.push_str(&format!("&expected_revision={revision}"));
    }
    request::<_, ()>(Method::GET, &route, None, READ_MS).await
}

pub async fn mail_draft(
    host: &str,
    mailbox: &str,
    client: &str,
) -> Result<MailComposeDraft, ApiError> {
    request::<_, ()>(
        Method::GET,
        &format!("{}?host_id={}", draft(mailbox, client), segment(host)),
        None,
        READ_MS,
    )
    .await
}

pub async fn mail_draft_save(
    mailbox: &str,
    client: &str,
    body: &MailDraftSave,
) -> Result<MailComposeDraft, ApiError> {
    request(Method::PUT, &draft(mailbox, client), Some(body), READ_MS).await
}

pub async fn mail_draft_review(
    mailbox: &str,
    client: &str,
    body: &MailReviewRequest,
) -> Result<MailSendReview, ApiError> {
    request(
        Method::POST,
        &format!("{}/review", draft(mailbox, client)),
        Some(body),
        WORK_MS,
    )
    .await
}

pub async fn mail_draft_send(
    mailbox: &str,
    client: &str,
    body: &MailSendRequest,
) -> Result<MailSendStatus, ApiError> {
    request(
        Method::POST,
        &format!("{}/send", draft(mailbox, client)),
        Some(body),
        WORK_MS,
    )
    .await
}

pub async fn mail_send_status(
    host: &str,
    mailbox: &str,
    operation: &str,
) -> Result<MailSendStatus, ApiError> {
    request::<_, ()>(
        Method::GET,
        &format!(
            "/api/mail/mailboxes/{}/sends/{}?host_id={}",
            segment(mailbox),
            segment(operation),
            segment(host)
        ),
        None,
        READ_MS,
    )
    .await
}

pub async fn mail_send_cancel(
    host: &str,
    mailbox: &str,
    operation: &str,
) -> Result<MailSendStatus, ApiError> {
    request(
        Method::POST,
        &format!(
            "/api/mail/mailboxes/{}/sends/{}/cancel",
            segment(mailbox),
            segment(operation)
        ),
        Some(&serde_json::json!({"host_id":host})),
        READ_MS,
    )
    .await
}

pub async fn mail_draft_remove_upload(
    host: &str,
    mailbox: &str,
    client: &str,
    id: &str,
) -> Result<(), ApiError> {
    request::<_, ()>(
        Method::DELETE,
        &format!(
            "{}/files/{}?host_id={}",
            draft(mailbox, client),
            segment(id),
            segment(host)
        ),
        None,
        READ_MS,
    )
    .await
}

pub async fn mail_draft_upload(
    host: &str,
    mailbox: &str,
    client: &str,
    upload_id: &str,
    name: &str,
    file: &web_sys::Blob,
) -> Result<MailDraftFile, ApiError> {
    if file.size() > f64::from(u32::try_from(MAIL_DRAFT_ATTACHMENT_MAX_BYTES).unwrap_or(u32::MAX)) {
        return Err(ApiError::Decode(
            "Keep draft files within 16 MiB combined.".into(),
        ));
    }
    let data = web_sys::FormData::new()
        .map_err(|_| ApiError::Network("The browser could not prepare this file.".into()))?;
    data.append_with_blob_and_filename("file", file, name)
        .map_err(|_| ApiError::Network("The browser could not prepare this file.".into()))?;
    let route = format!(
        "{}/files/{}?host_id={}",
        draft(mailbox, client),
        segment(upload_id),
        segment(host)
    );
    let abort = web_sys::AbortController::new()
        .map_err(|_| ApiError::Network("The browser cannot manage this upload.".into()))?;
    let operation = async {
        let request = RequestBuilder::new(&url(&route))
            .method(Method::POST)
            .abort_signal(Some(&abort.signal()))
            .body(data)?;
        decode(send_request(request).await?).await
    };
    futures_util::pin_mut!(operation);
    let deadline = gloo_timers::future::TimeoutFuture::new(WORK_MS);
    futures_util::pin_mut!(deadline);
    let result = match futures_util::future::select(operation, deadline).await {
        futures_util::future::Either::Left((result, _)) => result,
        futures_util::future::Either::Right(_) => Err(ApiError::Timeout { after_ms: WORK_MS }),
    };
    abort.abort();
    result
}
