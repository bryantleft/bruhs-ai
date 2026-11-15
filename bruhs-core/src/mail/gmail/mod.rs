//! Gmail reads and resumable sync under the canonical runtime's broker lease.
//! Continuation tokens remain local; no provider URL is accepted from a client.

mod attachments;
mod content;
pub use attachments::GmailAttachment;
mod projection;
mod send;
#[cfg(test)]
mod tests;
mod wire;

use super::{CursorScope, MailError, MailSession, SyncBatch, ThreadWrite};
use bruhs_models::{connections::ServiceProvider, mail::ConnectedMailThread};
use reqwest::{Client, StatusCode, Url};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{collections::BTreeSet, time::Duration};

const MAX_RESPONSE_BYTES: usize = 64 << 20;
const PAGE_SIZE: usize = 50;

#[derive(Debug, thiserror::Error)]
pub enum GmailError {
    #[error(transparent)]
    Mail(#[from] MailError),
    #[error("Choose a connected Gmail account for this action.")]
    WrongProvider,
    #[error(
        "Gmail could not be reached. Saved mail and the previous sync position were retained."
    )]
    Network,
    #[error("Reconnect Gmail before syncing this mailbox.")]
    SignIn,
    #[error("Gmail did not permit this read. Check mail access and the project's Gmail API setup, then retry.")]
    Permission,
    #[error("Your Google Workspace domain does not allow this Gmail app. Ask its administrator to permit access.")]
    DomainPolicy,
    #[error("Gmail is limiting requests. Saved mail is unchanged; wait and retry sync.")]
    RateLimited { retry_after_seconds: Option<u64> },
    #[error("This Gmail record is no longer available.")]
    NotFound,
    #[error("Gmail returned an invalid mail response. The sync position was not advanced.")]
    InvalidResponse,
    #[error("This Gmail response exceeds the local size or MIME depth limit. The sync position was not advanced.")]
    TooLarge,
    #[error("This message uses an unsupported text encoding. The sync position was not advanced.")]
    UnsupportedEncoding,
    #[error("The saved Gmail sync position is invalid. Restart sync; saved drafts are retained.")]
    InvalidCheckpoint,
}

#[derive(Clone)]
pub struct GmailClient {
    http: Client,
    base: Url,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
enum Checkpoint {
    Full {
        baseline: String,
        page_token: Option<String>,
        run_id: String,
        account: String,
        generation: String,
    },
    History {
        baseline: String,
        page_token: Option<String>,
        account: String,
        generation: String,
    },
}

impl Checkpoint {
    fn generation(&self) -> &str {
        match self {
            Self::Full { generation, .. } | Self::History { generation, .. } => generation,
        }
    }

    fn valid(&self) -> bool {
        let (baseline, page_token, account, generation, run_id) = match self {
            Self::Full {
                baseline,
                page_token,
                run_id,
                account,
                generation,
            } => (baseline, page_token, account, generation, Some(run_id)),
            Self::History {
                baseline,
                page_token,
                account,
                generation,
            } => (baseline, page_token, account, generation, None),
        };
        history_id(baseline).is_some()
            && page_token.as_ref().is_none_or(|token| !token.is_empty())
            && !account.is_empty()
            && !generation.is_empty()
            && run_id.is_none_or(|id| !id.is_empty())
    }
}

fn history_id(id: &str) -> Option<&str> {
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let normalized = id.trim_start_matches('0');
    Some(if normalized.is_empty() {
        "0"
    } else {
        normalized
    })
}

fn history_not_older(current: &str, baseline: &str) -> bool {
    match (history_id(current), history_id(baseline)) {
        (Some(current), Some(baseline)) => current
            .len()
            .cmp(&baseline.len())
            .then_with(|| current.cmp(baseline))
            .is_ge(),
        _ => false,
    }
}

fn retry_after_seconds(response: &reqwest::Response) -> Option<u64> {
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

async fn forbidden(mut response: reqwest::Response) -> GmailError {
    let retry_after_seconds = retry_after_seconds(&response);
    let mut bytes = Vec::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        if bytes.len().saturating_add(chunk.len()) > 16_384 {
            break;
        }
        bytes.extend_from_slice(&chunk);
    }
    let value = serde_json::from_slice::<serde_json::Value>(&bytes).ok();
    if let Some(reasons) = value
        .as_ref()
        .and_then(|value| value.pointer("/error/errors"))
        .and_then(serde_json::Value::as_array)
    {
        for reason in reasons
            .iter()
            .filter_map(|reason| reason.get("reason").and_then(serde_json::Value::as_str))
        {
            if matches!(
                reason,
                "dailyLimitExceeded" | "rateLimitExceeded" | "userRateLimitExceeded"
            ) {
                return GmailError::RateLimited {
                    retry_after_seconds,
                };
            }
            if reason == "domainPolicy" {
                return GmailError::DomainPolicy;
            }
        }
    }
    GmailError::Permission
}

#[derive(Debug, Serialize)]
pub struct SyncProgress {
    pub revision: i64,
    pub threads_updated: usize,
    pub full_scan_complete: bool,
    pub caught_up: bool,
}

impl GmailClient {
    /// Isolated transport fixtures only; production uses the fixed Gmail origin.
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test_origin(origin: &str) -> Result<Self, GmailError> {
        let url = Url::parse(origin).map_err(|_| GmailError::InvalidResponse)?;
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
            return Err(GmailError::InvalidResponse);
        }
        Self::at(
            url.join("gmail/v1/users/me/")
                .map_err(|_| GmailError::InvalidResponse)?,
        )
    }

    pub fn new() -> Result<Self, GmailError> {
        Self::at(
            Url::parse("https://gmail.googleapis.com/gmail/v1/users/me/")
                .map_err(|_| GmailError::InvalidResponse)?,
        )
    }

    fn at(base: Url) -> Result<Self, GmailError> {
        let http = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(45))
            .build()
            .map_err(|_| GmailError::Network)?;
        Ok(Self { http, base })
    }

    fn url(&self, segments: &[&str]) -> Result<Url, GmailError> {
        let mut url = self.base.clone();
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|()| GmailError::InvalidResponse)?;
            path.pop_if_empty();
            for segment in segments {
                if segment.is_empty() || matches!(*segment, "." | "..") {
                    return Err(GmailError::InvalidResponse);
                }
                path.push(segment);
            }
        }
        Ok(url)
    }

    async fn get<T: DeserializeOwned>(
        &self,
        session: &MailSession,
        segments: &[&str],
        query: &[(&str, &str)],
    ) -> Result<T, GmailError> {
        if session.authorization().mailbox().provider() != ServiceProvider::Gmail {
            return Err(GmailError::WrongProvider);
        }
        let mut response = self
            .http
            .get(self.url(segments)?)
            .query(query)
            .bearer_auth(session._access.value.as_str())
            .send()
            .await
            .map_err(|_| GmailError::Network)?;
        match response.status() {
            StatusCode::UNAUTHORIZED => return Err(GmailError::SignIn),
            StatusCode::FORBIDDEN => return Err(forbidden(response).await),
            StatusCode::TOO_MANY_REQUESTS => {
                return Err(GmailError::RateLimited {
                    retry_after_seconds: retry_after_seconds(&response),
                })
            }
            StatusCode::NOT_FOUND => return Err(GmailError::NotFound),
            StatusCode::BAD_REQUEST if query.iter().any(|(key, _)| *key == "pageToken") => {
                return Err(GmailError::InvalidCheckpoint)
            }
            status if !status.is_success() => return Err(GmailError::Network),
            _ => {}
        }
        if response
            .content_length()
            .is_some_and(|length| length > u64::try_from(MAX_RESPONSE_BYTES).unwrap_or(u64::MAX))
        {
            return Err(GmailError::TooLarge);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| GmailError::Network)? {
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(GmailError::TooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| GmailError::InvalidResponse)
    }

    /// The From address is read from this current leased profile, never a draft request.
    pub async fn sender(
        &self,
        session: &MailSession,
    ) -> Result<super::compose::VerifiedMailSender, GmailError> {
        let profile: wire::Profile = self.get(session, &["profile"], &[]).await?;
        super::compose::VerifiedMailSender::from_profile(
            session,
            bruhs_models::mail::MailAddress {
                name: String::new(),
                address: profile.email_address,
            },
        )
        .map_err(|_| GmailError::InvalidResponse)
    }

    pub async fn read_thread(
        &self,
        session: &MailSession,
        external_id: &str,
        account: &str,
    ) -> Result<ConnectedMailThread, GmailError> {
        let mut thread: wire::Thread = self
            .get(session, &["threads", external_id], &[("format", "full")])
            .await?;
        if thread.id != external_id {
            return Err(GmailError::InvalidResponse);
        }
        for message in &mut thread.messages {
            self.fill_text_parts(session, &message.id, &mut message.payload)
                .await
                .map_err(|error| match error {
                    // A missing body part does not confirm deletion of its thread.
                    GmailError::NotFound => GmailError::InvalidResponse,
                    other => other,
                })?;
        }
        projection::thread(session.authorization().mailbox(), account, thread)
    }

    async fn fill_text_parts(
        &self,
        session: &MailSession,
        message_id: &str,
        root: &mut wire::Part,
    ) -> Result<(), GmailError> {
        let mut pending = vec![(root, 0)];
        while let Some((part, depth)) = pending.pop() {
            if depth > 64 {
                return Err(GmailError::TooLarge);
            }
            if content::is_attachment(part) {
                continue;
            }
            if part.body.data.is_none() && part.body.size > 0 {
                if let Some(id) = part.body.attachment_id.as_deref() {
                    let body: wire::Body = self
                        .get(session, &["messages", message_id, "attachments", id], &[])
                        .await?;
                    if body.size != part.body.size {
                        return Err(GmailError::InvalidResponse);
                    }
                    part.body.data = body.data;
                }
            }
            pending.extend(part.parts.iter_mut().rev().map(|part| (part, depth + 1)));
        }
        Ok(())
    }

    async fn begin(&self, session: &MailSession) -> Result<Checkpoint, GmailError> {
        let profile: wire::Profile = self.get(session, &["profile"], &[]).await?;
        if profile.email_address.is_empty()
            || profile.history_id.is_empty()
            || !profile.history_id.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(GmailError::InvalidResponse);
        }
        Ok(Checkpoint::Full {
            baseline: profile.history_id,
            page_token: None,
            run_id: uuid::Uuid::new_v4().to_string(),
            account: profile.email_address,
            generation: session.authorization().connection_id().to_owned(),
        })
    }

    pub async fn sync_page(&self, session: &MailSession) -> Result<SyncProgress, GmailError> {
        let (revision, saved) = session.sync_checkpoint(CursorScope::GmailHistory).await?;
        let checkpoint = saved
            .as_deref()
            .map(serde_json::from_str::<Checkpoint>)
            .transpose()
            .map_err(|_| GmailError::InvalidCheckpoint)?;
        if checkpoint
            .as_ref()
            .is_some_and(|checkpoint| !checkpoint.valid())
        {
            return Err(GmailError::InvalidCheckpoint);
        }
        let checkpoint = match checkpoint {
            Some(checkpoint)
                if checkpoint.generation() == session.authorization().connection_id() =>
            {
                checkpoint
            }
            _ => self.begin(session).await?,
        };
        match checkpoint {
            full @ Checkpoint::Full { .. } => self.full_page(session, revision, full).await,
            history @ Checkpoint::History { .. } => {
                match self.history_page(session, revision, history).await {
                    Err(GmailError::NotFound) => {
                        self.full_page(session, revision, self.begin(session).await?)
                            .await
                    }
                    result => result,
                }
            }
        }
    }

    /// Explicit recovery retains saved mail until a fresh full scan completes.
    pub async fn restart_sync_page(
        &self,
        session: &MailSession,
    ) -> Result<SyncProgress, GmailError> {
        let (revision, _) = session.sync_checkpoint(CursorScope::GmailHistory).await?;
        self.full_page(session, revision, self.begin(session).await?)
            .await
    }

    async fn writes(
        &self,
        session: &MailSession,
        ids: BTreeSet<String>,
        account: &str,
    ) -> Result<SyncBatch, GmailError> {
        let mut batch = SyncBatch {
            upserts: Vec::new(),
            confirmed_deleted: Vec::new(),
            cursor: None,
        };
        for id in ids {
            let reference = session
                .authorization()
                .mailbox()
                .thread(&id)
                .map_err(|_| GmailError::InvalidResponse)?;
            match self.read_thread(session, &id, account).await {
                Ok(thread) => batch.upserts.push(ThreadWrite {
                    reference,
                    payload: serde_json::to_string(&thread)
                        .map_err(|_| GmailError::InvalidResponse)?,
                }),
                Err(GmailError::NotFound) => batch.confirmed_deleted.push(reference),
                Err(error) => return Err(error),
            }
        }
        Ok(batch)
    }

    async fn full_page(
        &self,
        session: &MailSession,
        revision: i64,
        checkpoint: Checkpoint,
    ) -> Result<SyncProgress, GmailError> {
        let Checkpoint::Full {
            baseline,
            page_token,
            run_id,
            account,
            generation,
        } = checkpoint
        else {
            return Err(GmailError::InvalidCheckpoint);
        };
        let page_size = PAGE_SIZE.to_string();
        let mut query = vec![
            ("maxResults", page_size.as_str()),
            ("includeSpamTrash", "true"),
        ];
        if let Some(token) = page_token.as_deref() {
            query.push(("pageToken", token));
        }
        let list: wire::ThreadList = self.get(session, &["threads"], &query).await?;
        if list
            .next_page_token
            .as_ref()
            .is_some_and(|next| next.is_empty() || Some(next) == page_token.as_ref())
        {
            return Err(GmailError::InvalidResponse);
        }
        let mut batch = self
            .writes(
                session,
                list.threads.into_iter().map(|thread| thread.id).collect(),
                &account,
            )
            .await?;
        let complete = list.next_page_token.is_none();
        let next = if complete {
            Checkpoint::History {
                baseline,
                page_token: None,
                account,
                generation,
            }
        } else {
            Checkpoint::Full {
                baseline,
                page_token: list.next_page_token,
                run_id: run_id.clone(),
                account,
                generation,
            }
        };
        batch.cursor = Some((
            CursorScope::GmailHistory,
            serde_json::to_string(&next).map_err(|_| GmailError::InvalidCheckpoint)?,
        ));
        let threads_updated = batch.upserts.len();
        let revision = session
            .apply_full_sync_page(revision, batch, run_id, complete)
            .await?;
        Ok(SyncProgress {
            revision,
            threads_updated,
            full_scan_complete: complete,
            caught_up: false,
        })
    }

    async fn history_page(
        &self,
        session: &MailSession,
        revision: i64,
        checkpoint: Checkpoint,
    ) -> Result<SyncProgress, GmailError> {
        let Checkpoint::History {
            baseline,
            page_token,
            account,
            generation,
        } = checkpoint
        else {
            return Err(GmailError::InvalidCheckpoint);
        };
        let mut query = vec![("startHistoryId", baseline.as_str()), ("maxResults", "100")];
        if let Some(token) = page_token.as_deref() {
            query.push(("pageToken", token));
        }
        let history: wire::HistoryList = self.get(session, &["history"], &query).await?;
        if !history_not_older(&history.history_id, &baseline)
            || history
                .next_page_token
                .as_ref()
                .is_some_and(|next| next.is_empty() || Some(next) == page_token.as_ref())
        {
            return Err(GmailError::InvalidResponse);
        }
        let mut ids = BTreeSet::new();
        for item in history.history {
            ids.extend(item.messages.into_iter().map(|message| message.thread_id));
            for changes in [
                item.messages_added,
                item.messages_deleted,
                item.labels_added,
                item.labels_removed,
            ] {
                ids.extend(changes.into_iter().map(|change| change.message.thread_id));
            }
        }
        let mut batch = self.writes(session, ids, &account).await?;
        let caught_up = history.next_page_token.is_none();
        let next = Checkpoint::History {
            baseline: if caught_up {
                history.history_id
            } else {
                baseline
            },
            page_token: history.next_page_token,
            account,
            generation,
        };
        batch.cursor = Some((
            CursorScope::GmailHistory,
            serde_json::to_string(&next).map_err(|_| GmailError::InvalidCheckpoint)?,
        ));
        let threads_updated = batch.upserts.len();
        let revision = session.apply_sync(revision, batch).await?;
        Ok(SyncProgress {
            revision,
            threads_updated,
            full_scan_complete: true,
            caught_up,
        })
    }
}
