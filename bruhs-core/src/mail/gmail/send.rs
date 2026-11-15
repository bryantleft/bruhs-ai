//! One Gmail send request from the runtime's consumed dispatch capability.

use super::{forbidden, retry_after_seconds, GmailClient, GmailError};
use crate::mail::{compose::*, MailSession, ProviderAcceptance};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use bruhs_models::{connections::ServiceProvider, mail::MAIL_PREPARED_MIME_MAX_BYTES};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

const RECEIPT_MAX_BYTES: usize = 64 << 10;

fn problem(error: GmailError) -> MailSendProblem {
    let kind = match error {
        GmailError::RateLimited {
            retry_after_seconds,
        } => {
            return MailSendProblem {
                kind: MailSendProblemKind::RateLimited,
                retry_after_seconds,
            }
        }
        GmailError::SignIn => MailSendProblemKind::SignIn,
        GmailError::Permission | GmailError::DomainPolicy => MailSendProblemKind::Permission,
        GmailError::WrongProvider => MailSendProblemKind::AccountChanged,
        GmailError::InvalidResponse => MailSendProblemKind::AccountChanged,
        GmailError::TooLarge => MailSendProblemKind::TooLarge,
        _ => MailSendProblemKind::Unavailable,
    };
    MailSendProblem::new(kind)
}

fn uncertain() -> ProviderSendOutcome {
    ProviderSendOutcome::Uncertain(MailSendProblem::new(MailSendProblemKind::Unconfirmed))
}
fn rejected(kind: MailSendProblemKind) -> ProviderSendOutcome {
    ProviderSendOutcome::Rejected(MailSendProblem::new(kind))
}
fn receipt_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 4096
        && !id.chars().any(char::is_control)
        && !id.chars().any(char::is_whitespace)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SendBody<'a> {
    raw: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_id: Option<&'a str>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Receipt {
    id: String,
    thread_id: Option<String>,
}

#[async_trait::async_trait]
impl MailSendTransport for GmailClient {
    fn provider(&self) -> ServiceProvider {
        ServiceProvider::Gmail
    }
    fn mime_limit_bytes(&self) -> usize {
        MAIL_PREPARED_MIME_MAX_BYTES
    }
    async fn verified_sender(
        &self,
        session: &MailSession,
    ) -> Result<VerifiedMailSender, MailSendProblem> {
        self.sender(session).await.map_err(problem)
    }
    async fn send(&self, session: &MailSession, dispatch: MailDispatch) -> ProviderSendOutcome {
        if let Err(problem) = dispatch.validate_session(session) {
            return ProviderSendOutcome::Rejected(problem);
        }
        if session.authorization().mailbox().provider() != ServiceProvider::Gmail {
            return rejected(MailSendProblemKind::AccountChanged);
        }
        if dispatch.mime().bytes().len() > self.mime_limit_bytes() {
            return rejected(MailSendProblemKind::TooLarge);
        }
        let endpoint = match self.url(&["messages", "send"]) {
            Ok(url) => url,
            Err(_) => return rejected(MailSendProblemKind::InvalidMessage),
        };
        let body = SendBody {
            raw: URL_SAFE_NO_PAD.encode(dispatch.mime().bytes()),
            thread_id: dispatch.mime().reply_thread(),
        };
        // The client disables redirects and retry(never). Once .send() starts,
        // a network error or unclassified response is ambiguous, never replayed.
        let mut response = match self
            .http
            .post(endpoint)
            .bearer_auth(session._access.value.as_str())
            .json(&body)
            .send()
            .await
        {
            Ok(response) => response,
            Err(_) => return uncertain(),
        };
        match response.status() {
            StatusCode::UNAUTHORIZED => return rejected(MailSendProblemKind::SignIn),
            StatusCode::FORBIDDEN => {
                return ProviderSendOutcome::Rejected(problem(forbidden(response).await))
            }
            StatusCode::TOO_MANY_REQUESTS => {
                return ProviderSendOutcome::Rejected(MailSendProblem {
                    kind: MailSendProblemKind::RateLimited,
                    retry_after_seconds: retry_after_seconds(&response),
                })
            }
            StatusCode::PAYLOAD_TOO_LARGE => return rejected(MailSendProblemKind::TooLarge),
            StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND => {
                return rejected(MailSendProblemKind::InvalidMessage)
            }
            status if !status.is_success() => return uncertain(),
            _ => {}
        }
        if response
            .content_length()
            .is_some_and(|size| size > u64::try_from(RECEIPT_MAX_BYTES).unwrap_or(u64::MAX))
        {
            return uncertain();
        }
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .filter(|id| receipt_id(id))
            .map(str::to_owned);
        let mut bytes = Vec::new();
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) if bytes.len().saturating_add(chunk.len()) <= RECEIPT_MAX_BYTES => {
                    bytes.extend_from_slice(&chunk)
                }
                Ok(None) => break,
                _ => return uncertain(),
            }
        }
        let receipt: Receipt = match serde_json::from_slice(&bytes) {
            Ok(receipt) => receipt,
            Err(_) => return uncertain(),
        };
        if !receipt_id(&receipt.id) || receipt.thread_id.as_ref().is_some_and(|id| !receipt_id(id))
        {
            return uncertain();
        }
        ProviderSendOutcome::Accepted(ProviderAcceptance {
            message_id: Some(receipt.id),
            thread_id: receipt.thread_id,
            request_id,
        })
    }
}
