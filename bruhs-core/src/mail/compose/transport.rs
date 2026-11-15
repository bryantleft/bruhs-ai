//! The provider boundary consumes one durable dispatch capability. It cannot be
//! created from HTTP input, cloned, or reconstructed from a reviewed MIME alone.

use super::{ComposeError, PreparedMime, VerifiedMailSender};
use crate::mail::{
    MailAuthorizationRef, MailCacheHostId, MailSession, ProviderAcceptance, SendOperation,
    SendState,
};
use bruhs_models::connections::ServiceProvider;
use std::fmt;

pub use bruhs_models::mail::{MailSendProblem, MailSendProblemKind};

#[must_use]
pub enum ProviderSendOutcome {
    /// A positive provider acknowledgement, never recipient delivery.
    Accepted(ProviderAcceptance),
    /// The adapter knows that no send was accepted. No automatic retry follows.
    Rejected(MailSendProblem),
    /// The request may have been accepted. Never automatically replay it.
    Uncertain(MailSendProblem),
}

impl fmt::Debug for ProviderSendOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Accepted(_) => f.write_str("Accepted"),
            Self::Rejected(problem) => f.debug_tuple("Rejected").field(problem).finish(),
            Self::Uncertain(problem) => f.debug_tuple("Uncertain").field(problem).finish(),
        }
    }
}

pub struct MailDispatch {
    host: MailCacheHostId,
    authorization: MailAuthorizationRef,
    operation_id: String,
    mime: PreparedMime,
}

impl fmt::Debug for MailDispatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MailDispatch")
            .field("operation_id", &self.operation_id)
            .field("mime", &self.mime)
            .finish_non_exhaustive()
    }
}

impl MailDispatch {
    pub(super) fn new(
        host: MailCacheHostId,
        authorization: MailAuthorizationRef,
        operation: SendOperation,
        mime: PreparedMime,
    ) -> Result<Self, ComposeError> {
        if operation.state != SendState::Dispatching
            || operation.connection_id() != authorization.connection_id()
        {
            return Err(ComposeError::StaleReview);
        }
        Ok(Self {
            host,
            authorization,
            operation_id: operation.id,
            mime,
        })
    }

    pub fn validate_session(&self, session: &MailSession) -> Result<(), MailSendProblem> {
        if self.host != *session.runtime.host()
            || self
                .authorization
                .validate_current(session.authorization())
                .is_err()
        {
            return Err(MailSendProblem::new(MailSendProblemKind::AccountChanged));
        }
        Ok(())
    }

    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }
    pub fn mime(&self) -> &PreparedMime {
        &self.mime
    }
}

/// Implementations validate the session and fixed provider origin, disable
/// redirects and transport retries, and issue at most one provider send request.
/// Profile reads are separate and never authorize a second send attempt.
#[async_trait::async_trait]
pub trait MailSendTransport: Send + Sync {
    fn provider(&self) -> ServiceProvider;
    /// Raw MIME budget, accounting for any outer encoding in the provider body.
    fn mime_limit_bytes(&self) -> usize;
    async fn verified_sender(
        &self,
        session: &MailSession,
    ) -> Result<VerifiedMailSender, MailSendProblem>;
    async fn send(&self, session: &MailSession, dispatch: MailDispatch) -> ProviderSendOutcome;
}
