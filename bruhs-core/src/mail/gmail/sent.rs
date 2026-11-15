//! A unique current-session Sent copy can supply positive evidence only.

use super::{content, GmailClient, GmailError};
use crate::mail::{
    compose::{
        reconciliation::{MailSentLookup, SentLookupOutcome, SentLookupTarget},
        MailSendProblem, MailSendProblemKind,
    },
    MailSession, ProviderAcceptance,
};
use bruhs_models::connections::ServiceProvider;
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Candidates {
    #[serde(default)]
    messages: Vec<Candidate>,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Candidate {
    id: String,
    thread_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SentMessage {
    id: String,
    thread_id: Option<String>,
    label_ids: Vec<String>,
    raw: String,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 4096
        && !matches!(id, "." | "..")
        && !id
            .chars()
            .any(|value| value.is_control() || value.is_whitespace())
}

fn problem(error: GmailError) -> MailSendProblem {
    let kind = match error {
        GmailError::SignIn => MailSendProblemKind::SignIn,
        GmailError::Permission | GmailError::DomainPolicy => MailSendProblemKind::Permission,
        GmailError::WrongProvider => MailSendProblemKind::AccountChanged,
        GmailError::RateLimited {
            retry_after_seconds,
        } => {
            return MailSendProblem {
                kind: MailSendProblemKind::RateLimited,
                retry_after_seconds,
            }
        }
        _ => MailSendProblemKind::Unconfirmed,
    };
    MailSendProblem::new(kind)
}

#[async_trait::async_trait]
impl MailSentLookup for GmailClient {
    fn provider(&self) -> ServiceProvider {
        ServiceProvider::Gmail
    }

    async fn find_sent(
        &self,
        session: &MailSession,
        target: Arc<SentLookupTarget>,
    ) -> Result<SentLookupOutcome, MailSendProblem> {
        target.validate_session(session)?;
        if session.authorization().mailbox().provider() != ServiceProvider::Gmail {
            return Err(MailSendProblem::new(MailSendProblemKind::AccountChanged));
        }
        let query = format!("rfc822msgid:<{}>", target.message_id());
        // A second candidate proves ambiguity. A continuation never proves a
        // unique match, even if this page contains just one record.
        let mut candidates: Candidates = self
            .get(
                session,
                &["messages"],
                &[
                    ("q", query.as_str()),
                    ("labelIds", "SENT"),
                    ("maxResults", "2"),
                    ("includeSpamTrash", "true"),
                ],
            )
            .await
            .map_err(problem)?;
        if candidates.next_page_token.is_some() || candidates.messages.len() != 1 {
            return Ok(SentLookupOutcome::Unconfirmed);
        }
        let candidate = candidates.messages.remove(0);
        if !valid_id(&candidate.id)
            || candidate
                .thread_id
                .as_deref()
                .is_some_and(|id| !valid_id(id))
        {
            return Ok(SentLookupOutcome::Unconfirmed);
        }
        let message: SentMessage = self
            .get(session, &["messages", &candidate.id], &[("format", "raw")])
            .await
            .map_err(problem)?;
        if message.id != candidate.id
            || !valid_id(&message.id)
            || message.thread_id.as_deref().is_some_and(|id| !valid_id(id))
            || candidate
                .thread_id
                .as_ref()
                .is_some_and(|id| message.thread_id.as_ref() != Some(id))
            || !message.label_ids.iter().any(|label| label == "SENT")
            || message.label_ids.iter().any(|label| label == "DRAFT")
            || message.raw.len() > target.max_copy_bytes().saturating_add(2) / 3 * 4
        {
            return Ok(SentLookupOutcome::Unconfirmed);
        }
        let raw = message.raw;
        let bytes = tokio::task::spawn_blocking(move || content::decode(&raw))
            .await
            .map_err(|_| MailSendProblem::new(MailSendProblemKind::Unconfirmed))?
            .map_err(problem)?;
        let receipt = ProviderAcceptance {
            message_id: Some(message.id),
            thread_id: message.thread_id,
            request_id: None,
        };
        Ok(
            match target.verify_sent_copy(session, receipt, bytes).await? {
                Some(proof) => SentLookupOutcome::Matched(proof),
                None => SentLookupOutcome::Unconfirmed,
            },
        )
    }
}
