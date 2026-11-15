use super::{
    http::{decode, send_request, url},
    segment, ApiError,
};
use bruhs_models::connections::ServiceProvider;
use bruhs_models::{mail::*, InboxFilter};
use gloo_net::http::{Method, RequestBuilder};
use serde::de::DeserializeOwned;
use wasm_bindgen::JsCast as _;

// Match the native ServerClient request policies. A sync page may fetch many
// complete Gmail threads before committing its cursor; ordinary cache reads use
// the standard 20 second local API deadline.
const READ_TIMEOUT_MS: u32 = 20_000;
const SYNC_TIMEOUT_MS: u32 = 180_000;

async fn request<T: DeserializeOwned>(
    route: &str,
    body: Option<&GmailSyncRequest>,
) -> Result<T, ApiError> {
    let abort = web_sys::AbortController::new()
        .map_err(|_| ApiError::Network("The browser cannot cancel mail requests.".into()))?;
    request_with_abort(route, body, &abort).await
}

async fn request_with_abort<T: DeserializeOwned>(
    route: &str,
    body: Option<&GmailSyncRequest>,
    abort: &web_sys::AbortController,
) -> Result<T, ApiError> {
    let timeout_ms = if body.is_some() {
        SYNC_TIMEOUT_MS
    } else {
        READ_TIMEOUT_MS
    };
    let read = async {
        let builder = RequestBuilder::new(&url(route)).abort_signal(Some(&abort.signal()));
        let request = match body {
            Some(body) => builder.method(Method::POST).json(body)?,
            None => builder.method(Method::GET).build()?,
        };
        decode(send_request(request).await?).await
    };
    futures_util::pin_mut!(read);
    let timeout = gloo_timers::future::TimeoutFuture::new(timeout_ms);
    futures_util::pin_mut!(timeout);
    match futures_util::future::select(read, timeout).await {
        futures_util::future::Either::Left((result, _)) => result,
        futures_util::future::Either::Right(_) => {
            abort.abort();
            Err(ApiError::Timeout {
                after_ms: timeout_ms,
            })
        }
    }
}

pub async fn mail_snapshot() -> Result<MailboxesSnapshot, ApiError> {
    request("/api/mail", None).await
}

pub async fn mail_connection(
    provider: ServiceProvider,
) -> Result<bruhs_models::connections::ServiceConnection, ApiError> {
    let route = match provider {
        ServiceProvider::Gmail => "/api/connections/gmail",
        ServiceProvider::Outlook => "/api/connections/outlook",
        _ => {
            return Err(ApiError::Decode(
                "Choose a Gmail or Outlook Mail connection.".into(),
            ))
        }
    };
    request(route, None).await
}

fn browse_route(mailbox: &str, query: &MailBrowseQuery) -> String {
    let filter = match query.filter {
        InboxFilter::Inbox => "inbox",
        InboxFilter::Unread => "unread",
        InboxFilter::Snoozed => "snoozed",
        InboxFilter::Archived => "archived",
        InboxFilter::All => "all",
    };
    let mut route = format!(
        "/api/mail/mailboxes/{}/threads?host_id={}&query={}&filter={filter}&page={}",
        segment(mailbox),
        segment(&query.host_id),
        segment(&query.query),
        query.page
    );
    if let Some(revision) = query.expected_revision {
        route.push_str(&format!("&expected_revision={revision}"));
    }
    route
}

pub async fn mail_browse(
    mailbox: &str,
    query: &MailBrowseQuery,
    abort: &web_sys::AbortController,
) -> Result<MailThreadsPage, ApiError> {
    request_with_abort(&browse_route(mailbox, query), None, abort).await
}

pub async fn mail_thread(
    host: &str,
    mailbox: &str,
    external: &str,
    abort: &web_sys::AbortController,
) -> Result<ConnectedMailThread, ApiError> {
    request_with_abort(
        &format!(
            "/api/mail/mailboxes/{}/threads/{}?host_id={}",
            segment(mailbox),
            segment(external),
            segment(host)
        ),
        None,
        abort,
    )
    .await
}

pub async fn mail_sync(
    provider: ServiceProvider,
    body: &GmailSyncRequest,
) -> Result<MailSyncResult, ApiError> {
    let route = match provider {
        ServiceProvider::Gmail => "/api/mail/gmail/sync",
        ServiceProvider::Outlook => "/api/mail/outlook/sync",
        _ => {
            return Err(ApiError::Decode(
                "Choose a Gmail or Outlook Mail connection.".into(),
            ))
        }
    };
    request(route, Some(body)).await
}

pub async fn mail_attachment(
    provider: ServiceProvider,
    host: &str,
    mailbox: &str,
    thread: &str,
    connection: &str,
    attachment: &MailAttachment,
) -> Result<web_sys::Blob, ApiError> {
    if attachment.size_bytes > MAIL_ATTACHMENT_MAX_BYTES {
        return Err(ApiError::Decode(
            "Attachment exceeds the local buffer limit.".into(),
        ));
    }
    let expected = u32::try_from(attachment.size_bytes)
        .map_err(|_| ApiError::Decode("Attachment exceeds the local buffer limit.".into()))?;
    let maximum = u32::try_from(MAIL_ATTACHMENT_MAX_BYTES)
        .map_err(|_| ApiError::Decode("Attachment exceeds the local buffer limit.".into()))?;
    let prefix = match provider {
        ServiceProvider::Gmail => "/api/mail",
        ServiceProvider::Outlook => "/api/mail/outlook",
        _ => {
            return Err(ApiError::Decode(
                "Choose a Gmail or Outlook Mail connection.".into(),
            ))
        }
    };
    let route = format!(
        "{prefix}/mailboxes/{}/threads/{}/attachments/{}?host_id={}&connection_id={}",
        segment(mailbox),
        segment(thread),
        segment(&attachment.id),
        segment(host),
        segment(connection)
    );
    let abort = web_sys::AbortController::new()
        .map_err(|_| ApiError::Network("The browser cannot cancel mail requests.".into()))?;
    let read = async {
        let response = send_request(
            RequestBuilder::new(&url(&route))
                .abort_signal(Some(&abort.signal()))
                .build()?,
        )
        .await?;
        let response: web_sys::Response = response.into();
        let expected = if provider == ServiceProvider::Outlook {
            response
                .headers()
                .get("x-bruhs-mail-export-size")
                .map_err(|_| {
                    ApiError::Decode("The attachment export size could not be read.".into())
                })?
                .map(|size| {
                    size.parse::<u32>()
                        .ok()
                        .filter(|size| *size <= maximum)
                        .ok_or_else(|| ApiError::Decode("Invalid attachment export size.".into()))
                })
                .transpose()?
                .unwrap_or(expected)
        } else {
            expected
        };
        let promise = response.blob().map_err(|_| {
            ApiError::Decode("The browser could not receive this attachment.".into())
        })?;
        // Keep the buffer in the browser rather than copying it through WASM.
        let blob = wasm_bindgen_futures::JsFuture::from(promise)
            .await
            .map_err(|_| ApiError::Decode("The attachment did not finish downloading.".into()))?
            .dyn_into::<web_sys::Blob>()
            .map_err(|_| {
                ApiError::Decode("The browser could not package this attachment.".into())
            })?;
        if blob.size() > f64::from(maximum) || blob.size() != f64::from(expected) {
            return Err(ApiError::Decode(
                "The attachment did not match its saved size.".into(),
            ));
        }
        Ok(blob)
    };
    futures_util::pin_mut!(read);
    let timeout = gloo_timers::future::TimeoutFuture::new(SYNC_TIMEOUT_MS);
    futures_util::pin_mut!(timeout);
    let result = match futures_util::future::select(read, timeout).await {
        futures_util::future::Either::Left((result, _)) => result,
        futures_util::future::Either::Right(_) => Err(ApiError::Timeout {
            after_ms: SYNC_TIMEOUT_MS,
        }),
    };
    abort.abort();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_search_and_provider_ids_cannot_change_route_or_query_boundaries() {
        let route = browse_route(
            "mail/box",
            &MailBrowseQuery {
                host_id: "host & other".into(),
                query: "CAFÉ 世界 & page=9".into(),
                filter: InboxFilter::All,
                page: 2,
                expected_revision: Some(8),
            },
        );
        assert_eq!(route, "/api/mail/mailboxes/mail%2Fbox/threads?host_id=host%20%26%20other&query=CAF%C3%89%20%E4%B8%96%E7%95%8C%20%26%20page%3D9&filter=all&page=2&expected_revision=8");
    }
}
