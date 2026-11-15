//! Positive Sent evidence from one current-account Graph folder copy.

use super::{retry_after, MailSession, OutlookClient, MAX_BYTES, PREFER};
use crate::mail::{
    compose::{
        reconciliation::{MailSentLookup, SentLookupOutcome, SentLookupTarget},
        MailSendProblem, MailSendProblemKind,
    },
    ProviderAcceptance,
};
use bruhs_models::connections::ServiceProvider;
use reqwest::{StatusCode, Url};
use serde::{de::DeserializeOwned, Deserialize};
use std::sync::Arc;

const FIELDS: &str = "id,conversationId,parentFolderId,internetMessageId,isDraft";

#[derive(Deserialize)]
struct Folder {
    id: String,
}

#[derive(Deserialize)]
struct Candidates {
    value: Vec<Candidate>,
    #[serde(rename = "@odata.nextLink")]
    next: Option<String>,
    #[serde(rename = "@odata.deltaLink")]
    delta: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Candidate {
    id: String,
    conversation_id: Option<String>,
    parent_folder_id: Option<String>,
    internet_message_id: Option<String>,
    is_draft: Option<bool>,
}

fn problem(kind: MailSendProblemKind) -> MailSendProblem {
    MailSendProblem::new(kind)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 4096
        && !matches!(id, "." | "..")
        && !id
            .chars()
            .any(|value| value.is_control() || value.is_whitespace())
}

impl OutlookClient {
    async fn sent_bytes(
        &self,
        session: &MailSession,
        target: &SentLookupTarget,
        url: Url,
        limit: usize,
        accept: &str,
    ) -> Result<Option<Vec<u8>>, MailSendProblem> {
        target.validate_session(session)?;
        if session.authorization().mailbox().provider() != ServiceProvider::Outlook
            || url.origin() != self.base.origin()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(problem(MailSendProblemKind::AccountChanged));
        }
        let _permit = self
            .requests
            .acquire()
            .await
            .map_err(|_| problem(MailSendProblemKind::Unconfirmed))?;
        target.validate_session(session)?;
        let mut response = self
            .http
            .get(url)
            .header("prefer", PREFER)
            .header("accept", accept)
            .bearer_auth(session._access.value.as_str())
            .send()
            .await
            .map_err(|_| problem(MailSendProblemKind::Unconfirmed))?;
        match response.status() {
            StatusCode::OK => {}
            StatusCode::NOT_FOUND => return Ok(None),
            StatusCode::UNAUTHORIZED => return Err(problem(MailSendProblemKind::SignIn)),
            StatusCode::FORBIDDEN => return Err(problem(MailSendProblemKind::Permission)),
            StatusCode::TOO_MANY_REQUESTS => {
                return Err(MailSendProblem {
                    kind: MailSendProblemKind::RateLimited,
                    retry_after_seconds: retry_after(&response),
                })
            }
            // Partial content, redirects and unexpected successes cannot prove
            // a complete unique Sent record, even if their body looks valid.
            _ => return Ok(None),
        }
        let limit = limit.min(MAX_BYTES);
        if response
            .content_length()
            .is_some_and(|bytes| bytes > u64::try_from(limit).unwrap_or(u64::MAX))
        {
            return Err(problem(MailSendProblemKind::TooLarge));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| problem(MailSendProblemKind::Unconfirmed))?
        {
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err(problem(MailSendProblemKind::TooLarge));
            }
            bytes.extend_from_slice(&chunk);
        }
        target.validate_session(session)?;
        Ok(Some(bytes))
    }

    async fn sent_json<T: DeserializeOwned + Send + 'static>(
        &self,
        session: &MailSession,
        target: &SentLookupTarget,
        url: Url,
    ) -> Result<Option<T>, MailSendProblem> {
        let Some(bytes) = self
            .sent_bytes(session, target, url, MAX_BYTES, "application/json")
            .await?
        else {
            return Ok(None);
        };
        let value = tokio::task::spawn_blocking(move || serde_json::from_slice::<T>(&bytes))
            .await
            .map_err(|_| problem(MailSendProblemKind::Unconfirmed))?
            .map_err(|_| problem(MailSendProblemKind::Unconfirmed))?;
        target.validate_session(session)?;
        Ok(Some(value))
    }
}

#[async_trait::async_trait]
impl MailSentLookup for OutlookClient {
    fn provider(&self) -> ServiceProvider {
        ServiceProvider::Outlook
    }

    async fn find_sent(
        &self,
        session: &MailSession,
        target: Arc<SentLookupTarget>,
    ) -> Result<SentLookupOutcome, MailSendProblem> {
        target.validate_session(session)?;
        let url = self
            .queried(&["mailFolders", "sentitems"], &[("$select", "id")])
            .map_err(|_| problem(MailSendProblemKind::Unconfirmed))?;
        let Some(folder): Option<Folder> = self.sent_json(session, &target, url).await? else {
            return Ok(SentLookupOutcome::Unconfirmed);
        };
        if !valid_id(&folder.id) {
            return Ok(SentLookupOutcome::Unconfirmed);
        }
        let message_id = format!("<{}>", target.message_id());
        let filter = format!("internetMessageId eq '{}'", message_id.replace('\'', "''"));
        let url = self
            .queried(
                &["mailFolders", &folder.id, "messages"],
                &[("$filter", &filter), ("$top", "2"), ("$select", FIELDS)],
            )
            .map_err(|_| problem(MailSendProblemKind::Unconfirmed))?;
        let Some(mut page): Option<Candidates> = self.sent_json(session, &target, url).await?
        else {
            return Ok(SentLookupOutcome::Unconfirmed);
        };
        // A second result proves ambiguity; a nextLink cannot prove uniqueness.
        // Do not follow continuations or infer acceptance from a partial page.
        if page.next.is_some() || page.delta.is_some() || page.value.len() != 1 {
            return Ok(SentLookupOutcome::Unconfirmed);
        }
        let candidate = page.value.remove(0);
        if !valid_id(&candidate.id)
            || candidate.parent_folder_id.as_deref() != Some(folder.id.as_str())
            || candidate.is_draft != Some(false)
            || candidate.internet_message_id.as_deref() != Some(message_id.as_str())
            || candidate
                .conversation_id
                .as_deref()
                .is_some_and(|id| !valid_id(id))
        {
            return Ok(SentLookupOutcome::Unconfirmed);
        }
        // Folder-scoped MIME retrieval also fails if the message moved out of
        // Sent between the metadata read and this request.
        let url = self
            .url(&[
                "mailFolders",
                &folder.id,
                "messages",
                &candidate.id,
                "$value",
            ])
            .map_err(|_| problem(MailSendProblemKind::Unconfirmed))?;
        let Some(raw) = self
            .sent_bytes(
                session,
                &target,
                url,
                target.max_copy_bytes(),
                "message/rfc822",
            )
            .await?
        else {
            return Ok(SentLookupOutcome::Unconfirmed);
        };
        let receipt = ProviderAcceptance {
            message_id: Some(candidate.id),
            thread_id: candidate.conversation_id,
            request_id: None,
        };
        Ok(
            match target.verify_sent_copy(session, receipt, raw).await? {
                Some(proof) => SentLookupOutcome::Matched(proof),
                None => SentLookupOutcome::Unconfirmed,
            },
        )
    }
}
