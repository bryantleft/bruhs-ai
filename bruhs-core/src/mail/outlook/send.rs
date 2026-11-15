//! One fixed-origin POST of an already frozen and durably claimed MIME message.
//! An ambiguous response never authorizes an automatic retry.

use super::{retry_after, MailSession, OutlookClient, OutlookError};
use crate::mail::{
    compose::{
        MailDispatch, MailSendProblem, MailSendProblemKind, MailSendTransport, ProviderSendOutcome,
        VerifiedMailSender,
    },
    ProviderAcceptance,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use bruhs_models::connections::ServiceProvider;
use reqwest::{header, StatusCode};

// Graph write requests are capped at 4 MB. Reserve the outer base64 expansion:
// a frozen MIME of at most 3,000,000 bytes encodes to at most 4,000,000 bytes.
// The common review path enforces this limit before freezing or claiming a send.
const MAX_SEND_MIME_BYTES: usize = 3_000_000;

fn problem(kind: MailSendProblemKind) -> MailSendProblem {
    MailSendProblem::new(kind)
}

fn sender_problem(error: OutlookError) -> MailSendProblem {
    match error {
        OutlookError::SignIn | OutlookError::NotFound | OutlookError::InvalidResponse => {
            problem(MailSendProblemKind::SignIn)
        }
        OutlookError::Permission | OutlookError::UnsupportedAccount => {
            problem(MailSendProblemKind::Permission)
        }
        OutlookError::WrongProvider | OutlookError::Mail(_) => {
            problem(MailSendProblemKind::AccountChanged)
        }
        OutlookError::RateLimited {
            retry_after_seconds,
        } => MailSendProblem {
            kind: MailSendProblemKind::RateLimited,
            retry_after_seconds,
        },
        OutlookError::TooLarge => problem(MailSendProblemKind::TooLarge),
        _ => problem(MailSendProblemKind::Unavailable),
    }
}

#[async_trait::async_trait]
impl MailSendTransport for OutlookClient {
    fn provider(&self) -> ServiceProvider {
        ServiceProvider::Outlook
    }

    fn mime_limit_bytes(&self) -> usize {
        MAX_SEND_MIME_BYTES
    }

    async fn verified_sender(
        &self,
        session: &MailSession,
    ) -> Result<VerifiedMailSender, MailSendProblem> {
        self.sender(session).await.map_err(sender_problem)
    }

    async fn send(&self, session: &MailSession, dispatch: MailDispatch) -> ProviderSendOutcome {
        use MailSendProblemKind as Kind;
        use ProviderSendOutcome::{Accepted, Rejected, Uncertain};

        if let Err(problem) = dispatch.validate_session(session) {
            return Rejected(problem);
        }
        if session.authorization().mailbox().provider() != ServiceProvider::Outlook {
            return Rejected(problem(Kind::AccountChanged));
        }
        if dispatch.mime().bytes().len() > self.mime_limit_bytes() {
            return Rejected(problem(Kind::TooLarge));
        }
        let Ok(url) = self.url(&["sendMail"]) else {
            return Rejected(problem(Kind::Unavailable));
        };
        // Build before acquiring the permit or attempting transport. A malformed
        // credential cannot be sent and is never included in a stored problem.
        let request = self
            .http
            .post(url)
            .header(header::CONTENT_TYPE, "text/plain")
            .header("prefer", "IdType=\"ImmutableId\"")
            .bearer_auth(session._access.value.as_str())
            .body(STANDARD.encode(dispatch.mime().bytes()))
            .build();
        let Ok(request) = request else {
            return Rejected(problem(Kind::SignIn));
        };
        let Ok(_permit) = self.requests.acquire().await else {
            return Rejected(problem(Kind::Unavailable));
        };
        // The client disables redirects and transport retries. There is exactly
        // one execution; all failures after it begins remain potentially sent.
        let Ok(response) = self.http.execute(request).await else {
            return Uncertain(problem(Kind::Unconfirmed));
        };
        match response.status() {
            StatusCode::ACCEPTED => {
                // Graph returns no message/thread ids here. Keep them absent:
                // the frozen RFC Message-ID is not a Graph resource identifier.
                let request_id = response
                    .headers()
                    .get("request-id")
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| uuid::Uuid::parse_str(value).ok())
                    .map(|value| value.to_string());
                Accepted(ProviderAcceptance {
                    message_id: None,
                    thread_id: None,
                    request_id,
                })
            }
            StatusCode::UNAUTHORIZED => Rejected(problem(Kind::SignIn)),
            StatusCode::FORBIDDEN => Rejected(problem(Kind::Permission)),
            StatusCode::NOT_FOUND => Rejected(problem(Kind::Unavailable)),
            StatusCode::PAYLOAD_TOO_LARGE => Rejected(problem(Kind::TooLarge)),
            StatusCode::TOO_MANY_REQUESTS => Rejected(MailSendProblem {
                kind: Kind::RateLimited,
                retry_after_seconds: retry_after(&response),
            }),
            status if status.is_client_error() && status != StatusCode::REQUEST_TIMEOUT => {
                Rejected(problem(Kind::InvalidMessage))
            }
            // Only 202 is a documented positive acknowledgement. A redirect,
            // timeout, server error or unexpected success is not proof of rejection.
            _ => Uncertain(problem(Kind::Unconfirmed)),
        }
    }
}

#[cfg(test)]
mod tests;
