//! Broker-leased Microsoft Graph mail reads and resumable account-wide sync.
//! Folder removals trigger reconciliation; they never delete conversations alone.

mod attachments;
mod projection;
mod send;
mod sync;
#[cfg(test)]
mod tests;
mod wire;

pub use attachments::OutlookAttachment;
pub use sync::SyncProgress;

use super::{MailError, MailSession};
use bruhs_models::{connections::ServiceProvider, mail::ConnectedMailThread};
use futures_util::{stream, StreamExt, TryStreamExt};
use reqwest::{Client, StatusCode, Url};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::Semaphore;

// Reuse the canonical mail response/attachment budget. A whole thread or cache
// batch is bounded too, rather than accepting individually bounded pages forever.
const MAX_BYTES: usize = 64 << 20;
// Graph's Outlook limit is four concurrent requests per application/mailbox.
const CONCURRENCY: usize = 4;
const PREFER: &str =
    "IdType=\"ImmutableId\", outlook.body-content-type=\"text\", odata.maxpagesize=50";
const MESSAGE_FIELDS: &str = "id,conversationId,parentFolderId,subject,body,from,sender,toRecipients,ccRecipients,bccRecipients,replyTo,internetMessageId,internetMessageHeaders,receivedDateTime,sentDateTime,createdDateTime,isRead,isDraft,importance,categories";
const ATTACHMENT_FIELDS: &str = "id,name,contentType,size,isInline";

#[derive(Debug, thiserror::Error)]
pub enum OutlookError {
    #[error(transparent)]
    Mail(#[from] MailError),
    #[error("Choose a connected Outlook Mail account for this action.")]
    WrongProvider,
    #[error(
        "Outlook could not be reached. Saved mail and the previous sync position were retained."
    )]
    Network,
    #[error("Reconnect Outlook Mail before retrying this action.")]
    SignIn,
    #[error(
        "Outlook did not permit this read. Check mail consent and your organization's app policy."
    )]
    Permission,
    #[error("This account does not provide a mailbox supported by Microsoft Graph.")]
    UnsupportedAccount,
    #[error("Outlook is limiting requests. Wait before retrying; the previous sync position was retained.")]
    RateLimited { retry_after_seconds: Option<u64> },
    #[error("This Outlook record is no longer available.")]
    NotFound,
    #[error("Outlook's sync position expired. A new full scan is needed; saved mail and drafts are retained.")]
    SyncExpired,
    #[error("Outlook returned an invalid mail response. The sync position was not advanced.")]
    InvalidResponse,
    #[error("The saved Outlook sync position is invalid. Restart sync; saved mail and drafts are retained.")]
    InvalidCheckpoint,
    #[error("This Outlook response exceeds the connected-mail size limit. The sync position was not advanced.")]
    TooLarge,
    #[error("This is a linked or unsupported Outlook attachment. Open it in Outlook; Bruhs does not fetch its external URL.")]
    UnsupportedAttachment,
}

#[derive(Clone)]
pub struct OutlookClient {
    http: Client,
    base: Url,
    requests: Arc<Semaphore>,
}

#[derive(Clone, Serialize, Deserialize)]
struct MailboxContext {
    account: String,
    inbox: String,
    sent: String,
}

impl OutlookClient {
    pub fn new() -> Result<Self, OutlookError> {
        Self::at(
            Url::parse("https://graph.microsoft.com/v1.0/me/")
                .map_err(|_| OutlookError::InvalidResponse)?,
        )
    }

    /// Isolated fixtures only. Production always uses the fixed Graph origin.
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test_origin(origin: &str) -> Result<Self, OutlookError> {
        let url = Url::parse(origin).map_err(|_| OutlookError::InvalidResponse)?;
        let loopback = match url.host() {
            Some(url::Host::Ipv4(address)) => address.is_loopback(),
            Some(url::Host::Ipv6(address)) => address.is_loopback(),
            Some(url::Host::Domain(domain)) => domain == "localhost",
            _ => false,
        };
        if url.scheme() != "http"
            || !loopback
            || url.path() != "/"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(OutlookError::InvalidResponse);
        }
        Self::at(
            url.join("v1.0/me/")
                .map_err(|_| OutlookError::InvalidResponse)?,
        )
    }

    fn at(base: Url) -> Result<Self, OutlookError> {
        // Same connect/request deadlines as the existing server mail transport.
        let http = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(45))
            .build()
            .map_err(|_| OutlookError::Network)?;
        Ok(Self {
            http,
            base,
            requests: Arc::new(Semaphore::new(CONCURRENCY)),
        })
    }

    fn url(&self, segments: &[&str]) -> Result<Url, OutlookError> {
        let mut url = self.base.clone();
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|()| OutlookError::InvalidResponse)?;
            path.pop_if_empty();
            for segment in segments {
                if segment.is_empty()
                    || matches!(*segment, "." | "..")
                    || segment.chars().any(char::is_control)
                {
                    return Err(OutlookError::InvalidResponse);
                }
                path.push(segment);
            }
        }
        Ok(url)
    }

    fn queried(&self, segments: &[&str], query: &[(&str, &str)]) -> Result<Url, OutlookError> {
        let mut url = self.url(segments)?;
        url.query_pairs_mut().extend_pairs(query.iter().copied());
        Ok(url)
    }

    fn checked_link(&self, link: &str, segments: &[&str]) -> Result<Url, OutlookError> {
        let url = Url::parse(link).map_err(|_| OutlookError::InvalidResponse)?;
        if url.origin() != self.base.origin()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url.query().is_none_or(str::is_empty)
        {
            return Err(OutlookError::InvalidResponse);
        }
        let actual_path = percent_encoding::percent_decode_str(url.path())
            .decode_utf8()
            .map_err(|_| OutlookError::InvalidResponse)?;
        let mut variants = vec![segments.to_vec()];
        // Graph emits both spellings for collection literals. Opaque record ids
        // retain their exact case, including in the parenthesized OData form.
        if segments.first() == Some(&"mailFolders") {
            let mut lower = segments.to_vec();
            lower[0] = "mailfolders";
            variants.push(lower);
        }
        if segments.last() == Some(&"childFolders") {
            let lower: Vec<_> = variants
                .iter()
                .map(|variant| {
                    let mut variant = variant.clone();
                    if let Some(last) = variant.last_mut() {
                        *last = "childfolders";
                    }
                    variant
                })
                .collect();
            variants.extend(lower);
        }
        for segments in variants {
            let expected = self.url(&segments)?;
            let expected_path = percent_encoding::percent_decode_str(expected.path())
                .decode_utf8()
                .map_err(|_| OutlookError::InvalidResponse)?;
            if actual_path == expected_path {
                return Ok(url);
            }
            // Parenthesized paths and segment paths refer to the same resource.
            if segments.len() >= 2 {
                let mut alternative = self.base.path().trim_end_matches('/').to_owned();
                alternative.push('/');
                alternative.push_str(segments[0]);
                alternative.push_str("('");
                alternative.push_str(&segments[1].replace('\'', "''"));
                alternative.push_str("')");
                for segment in &segments[2..] {
                    alternative.push('/');
                    alternative.push_str(segment);
                }
                if actual_path == alternative {
                    return Ok(url);
                }
            }
        }
        Err(OutlookError::InvalidResponse)
    }

    async fn bytes(&self, session: &MailSession, url: Url) -> Result<Vec<u8>, OutlookError> {
        if session.authorization().mailbox().provider() != ServiceProvider::Outlook {
            return Err(OutlookError::WrongProvider);
        }
        if url.origin() != self.base.origin()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(OutlookError::InvalidResponse);
        }
        let _permit = self
            .requests
            .acquire()
            .await
            .map_err(|_| OutlookError::Network)?;
        let mut response = self
            .http
            .get(url)
            .header("prefer", PREFER)
            .bearer_auth(session._access.value.as_str())
            .send()
            .await
            .map_err(|_| OutlookError::Network)?;
        match response.status() {
            StatusCode::UNAUTHORIZED => return Err(OutlookError::SignIn),
            StatusCode::TOO_MANY_REQUESTS => {
                return Err(OutlookError::RateLimited {
                    retry_after_seconds: retry_after(&response),
                })
            }
            StatusCode::GONE => return Err(OutlookError::SyncExpired),
            status if !status.is_success() => return Err(response_error(response).await),
            _ => {}
        }
        if response
            .content_length()
            .is_some_and(|size| size > u64::try_from(MAX_BYTES).unwrap_or(u64::MAX))
        {
            return Err(OutlookError::TooLarge);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| OutlookError::Network)? {
            if bytes.len().saturating_add(chunk.len()) > MAX_BYTES {
                return Err(OutlookError::TooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }

    async fn get<T: DeserializeOwned>(
        &self,
        session: &MailSession,
        url: Url,
    ) -> Result<T, OutlookError> {
        serde_json::from_slice(&self.bytes(session, url).await?)
            .map_err(|_| OutlookError::InvalidResponse)
    }

    /// Read the sender from this current leased profile, never a draft request.
    /// Preparation binds this capability to the same authorization again.
    pub async fn sender(
        &self,
        session: &MailSession,
    ) -> Result<super::compose::VerifiedMailSender, OutlookError> {
        let profile: wire::Profile = self
            .get(session, self.queried(&[], &[("$select", "id,mail")])?)
            .await?;
        if profile.id.is_empty() {
            return Err(OutlookError::InvalidResponse);
        }
        // Graph's mail field is the SMTP address. A userPrincipalName is a
        // sign-in identifier, so do not promote it to a verified From address.
        let address = profile.mail.ok_or(OutlookError::InvalidResponse)?;
        super::compose::VerifiedMailSender::from_profile(
            session,
            bruhs_models::mail::MailAddress {
                name: String::new(),
                address,
            },
        )
        .map_err(|_| OutlookError::InvalidResponse)
    }

    async fn context(&self, session: &MailSession) -> Result<MailboxContext, OutlookError> {
        let profile = self.queried(&[], &[("$select", "id,mail,userPrincipalName")])?;
        let inbox = self.queried(&["mailFolders", "inbox"], &[("$select", "id")])?;
        let sent = self.queried(&["mailFolders", "sentitems"], &[("$select", "id")])?;
        let (profile, inbox, sent): (wire::Profile, wire::Id, wire::Id) = tokio::try_join!(
            self.get(session, profile),
            self.get(session, inbox),
            self.get(session, sent)
        )?;
        let account = profile
            .mail
            .filter(|value| !value.trim().is_empty())
            .or(profile.user_principal_name)
            .filter(|value| !value.trim().is_empty())
            .ok_or(OutlookError::InvalidResponse)?;
        if profile.id.is_empty() || inbox.id.is_empty() || sent.id.is_empty() {
            return Err(OutlookError::InvalidResponse);
        }
        Ok(MailboxContext {
            account,
            inbox: inbox.id,
            sent: sent.id,
        })
    }

    pub async fn read_thread(
        &self,
        session: &MailSession,
        external_id: &str,
    ) -> Result<ConnectedMailThread, OutlookError> {
        let context = self.context(session).await?;
        self.thread_with_context(session, external_id, &context)
            .await?
            .ok_or(OutlookError::NotFound)
    }

    async fn thread_with_context(
        &self,
        session: &MailSession,
        external_id: &str,
        context: &MailboxContext,
    ) -> Result<Option<ConnectedMailThread>, OutlookError> {
        if external_id.is_empty() {
            return Err(OutlookError::InvalidResponse);
        }
        let filter = format!("conversationId eq '{}'", external_id.replace('\'', "''"));
        let expand = format!("attachments($select={ATTACHMENT_FIELDS})");
        let mut url = self.queried(
            &["messages"],
            &[
                ("$filter", &filter),
                ("$select", MESSAGE_FIELDS),
                ("$expand", &expand),
            ],
        )?;
        let mut seen = BTreeSet::new();
        let mut messages = Vec::new();
        let mut budget = 0usize;
        loop {
            if !seen.insert(url.as_str().to_owned()) {
                return Err(OutlookError::InvalidResponse);
            }
            let bytes = self.bytes(session, url).await?;
            budget = budget.saturating_add(bytes.len());
            if budget > MAX_BYTES {
                return Err(OutlookError::TooLarge);
            }
            let page: wire::Page<wire::Message> =
                serde_json::from_slice(&bytes).map_err(|_| OutlookError::InvalidResponse)?;
            if page.delta.is_some()
                || page
                    .value
                    .iter()
                    .any(|message| message.conversation_id != external_id)
            {
                return Err(OutlookError::InvalidResponse);
            }
            messages.extend(page.value);
            let Some(next) = page.next else {
                break;
            };
            url = self.checked_link(&next, &["messages"])?;
        }
        if messages.is_empty() {
            // Only a successful empty conversation query confirms absence.
            // A 404 from a subordinate attachment request must remain an error.
            return Ok(None);
        }
        let budget = AtomicUsize::new(budget);
        let filled: Vec<wire::Message> = stream::iter(messages)
            .map(|mut message| {
                let budget = &budget;
                async move {
                    let mut seen = BTreeSet::new();
                    while let Some(next) = message.attachments_next.take() {
                        if !seen.insert(next.clone()) {
                            return Err(OutlookError::InvalidResponse);
                        }
                        let url =
                            self.checked_link(&next, &["messages", &message.id, "attachments"])?;
                        let bytes = self.bytes(session, url).await?;
                        budget
                            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |size| {
                                size.checked_add(bytes.len())
                                    .filter(|size| *size <= MAX_BYTES)
                            })
                            .map_err(|_| OutlookError::TooLarge)?;
                        let page: wire::Page<wire::Attachment> = serde_json::from_slice(&bytes)
                            .map_err(|_| OutlookError::InvalidResponse)?;
                        if page.delta.is_some() {
                            return Err(OutlookError::InvalidResponse);
                        }
                        message.attachments.extend(page.value);
                        message.attachments_next = page.next;
                    }
                    Ok(message)
                }
            })
            .buffer_unordered(CONCURRENCY)
            .try_collect()
            .await?;
        projection::thread(
            session.authorization().mailbox(),
            context,
            external_id,
            filled,
        )
        .map(Some)
    }
}

fn retry_after(response: &reqwest::Response) -> Option<u64> {
    let value = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?;
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(seconds.max(1));
    }
    let date = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    Some(
        u64::try_from(
            date.timestamp()
                .saturating_sub(chrono::Utc::now().timestamp()),
        )
        .unwrap_or(0)
        .max(1),
    )
}

async fn response_error(mut response: reqwest::Response) -> OutlookError {
    let status = response.status();
    let mut bytes = Vec::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        if bytes.len().saturating_add(chunk.len()) > MAX_BYTES {
            break;
        }
        bytes.extend_from_slice(&chunk);
    }
    let body = serde_json::from_slice::<serde_json::Value>(&bytes).ok();
    let code = body
        .as_ref()
        .and_then(|body| body.pointer("/error/code"))
        .and_then(serde_json::Value::as_str);
    match code {
        Some(
            "syncStateNotFound"
            | "SyncStateNotFound"
            | "ErrorInvalidSyncStateData"
            | "resyncChangesApplyDifferences"
            | "resyncChangesUploadDifferences",
        ) => OutlookError::SyncExpired,
        Some(
            "MailboxNotEnabledForRESTAPI"
            | "ErrorMailboxNotEnabledForRESTAPI"
            | "MailboxNotSupportedForRESTAPI",
        ) => OutlookError::UnsupportedAccount,
        _ if status == StatusCode::FORBIDDEN => OutlookError::Permission,
        _ if status == StatusCode::NOT_FOUND => OutlookError::NotFound,
        _ if status.is_client_error() => OutlookError::InvalidResponse,
        _ => OutlookError::Network,
    }
}
