//! Explicit positive Sent proof. No path here grants a send or an unsent verdict.

mod content;

use super::{dispatch, ComposeError, MailSendProblem, MailSendProblemKind, PreparedMime};
use crate::mail::{
    MailAuthorizationRef, MailCacheHostId, MailSession, MailboxId, ProviderAcceptance,
    SendOperation,
};
use bruhs_models::{connections::ServiceProvider, mail::MailSendStatus, EpochMs};
use sha2::{Digest as _, Sha256};
use std::{fmt, sync::Arc};

#[async_trait::async_trait]
/// A provider's lookup-only adapter. A missing result never authorizes sending.
pub trait MailSentLookup: Send + Sync {
    fn provider(&self) -> ServiceProvider;
    async fn find_sent(
        &self,
        session: &MailSession,
        target: Arc<SentLookupTarget>,
    ) -> Result<SentLookupOutcome, MailSendProblem>;
}

/// Only positive, Core-verified evidence can resolve an uncertain operation.
pub enum SentLookupOutcome {
    Unconfirmed,
    Matched(VerifiedSentCopy),
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SentBinding {
    pub host: MailCacheHostId,
    pub mailbox: MailboxId,
    pub lookup_authorization: MailAuthorizationRef,
    pub operation_id: String,
    pub original_connection_id: String,
    pub draft_id: String,
    pub draft_revision: i64,
    pub payload_digest: String,
    pub payload_sha256: String,
    pub prepared_sha256: String,
}

/// An immutable reviewed operation bound to this lookup's current broker lease.
pub struct SentLookupTarget {
    binding: SentBinding,
    prepared: PreparedMime,
}

/// An opaque positive proof; only Core adapters can construct it from raw MIME.
pub struct VerifiedSentCopy {
    inner: Box<VerifiedSentData>,
}

struct VerifiedSentData {
    binding: SentBinding,
    receipt: ProviderAcceptance,
}

impl fmt::Debug for SentLookupTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SentLookupTarget")
            .field("operation_id", &self.binding.operation_id)
            .finish_non_exhaustive()
    }
}
impl fmt::Debug for VerifiedSentCopy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerifiedSentCopy")
            .field("operation_id", &self.inner.binding.operation_id)
            .finish_non_exhaustive()
    }
}

impl SentLookupTarget {
    pub(crate) fn new(
        host: MailCacheHostId,
        authorization: MailAuthorizationRef,
        operation: SendOperation,
        prepared: PreparedMime,
    ) -> Result<Self, ComposeError> {
        if !content::matches(&prepared, prepared.bytes()) {
            return Err(ComposeError::PreparedCorrupt);
        }
        let payload_sha256 = format!("{:x}", Sha256::digest(operation.payload().as_bytes()));
        let binding = SentBinding {
            host,
            mailbox: authorization.mailbox().clone(),
            lookup_authorization: authorization,
            original_connection_id: operation.connection_id().into(),
            operation_id: operation.id,
            payload_sha256,
            draft_id: operation.draft_id,
            draft_revision: operation.draft_revision,
            payload_digest: operation.payload_digest,
            prepared_sha256: prepared.sha256().into(),
        };
        Ok(Self { binding, prepared })
    }

    pub fn message_id(&self) -> &str {
        self.prepared.message_id()
    }

    pub fn max_copy_bytes(&self) -> usize {
        self.prepared
            .bytes()
            .len()
            .saturating_add(content::MAX_HEADERS_BYTES)
    }

    pub fn validate_session(&self, session: &MailSession) -> Result<(), MailSendProblem> {
        if self.binding.host != *session.runtime.host()
            || self.binding.mailbox != *session.authorization().mailbox()
            || self
                .binding
                .lookup_authorization
                .validate_current(session.authorization())
                .is_err()
        {
            return Err(MailSendProblem::new(MailSendProblemKind::AccountChanged));
        }
        Ok(())
    }

    /// Core adapters may call this only for a unique, fetched, non-draft Sent
    /// record from this current session. No client can supply this proof type.
    pub(crate) async fn verify_sent_copy(
        self: Arc<Self>,
        session: &MailSession,
        receipt: ProviderAcceptance,
        raw_mime: Vec<u8>,
    ) -> Result<Option<VerifiedSentCopy>, MailSendProblem> {
        self.validate_session(session)?;
        if raw_mime.len() > self.max_copy_bytes()
            || receipt.message_id.as_ref().is_none_or(|id| id.is_empty())
            || [&receipt.message_id, &receipt.thread_id, &receipt.request_id]
                .into_iter()
                .flatten()
                .any(|id| id.is_empty() || id.chars().any(char::is_control))
        {
            return Ok(None);
        }
        let target = Arc::clone(&self);
        let matched = tokio::task::spawn_blocking(move || {
            content::matches(&target.prepared, &raw_mime).then(|| VerifiedSentCopy {
                inner: Box::new(VerifiedSentData {
                    binding: target.binding.clone(),
                    receipt,
                }),
            })
        })
        .await
        .map_err(|_| MailSendProblem::new(MailSendProblemKind::Unconfirmed))?;
        self.validate_session(session)?;
        Ok(matched)
    }
}

impl VerifiedSentCopy {
    pub(crate) fn into_parts(self) -> (SentBinding, ProviderAcceptance) {
        let VerifiedSentData { binding, receipt } = *self.inner;
        (binding, receipt)
    }
}

pub(crate) enum ReconciliationSource {
    Status(Box<MailSendStatus>),
    Lookup(Arc<SentLookupTarget>),
}

impl MailSession {
    /// Read-only provider lookup followed only by a positively proven receipt.
    /// Cancellation during lookup leaves uncertainty intact; a started cache
    /// commit retains the runtime's existing blocking-worker ownership.
    pub async fn reconcile_uncertain_send(
        &self,
        expected_host: String,
        operation_id: String,
        lookup: &dyn MailSentLookup,
    ) -> Result<MailSendStatus, ComposeError> {
        self.runtime.compose_host(&expected_host)?;
        if lookup.provider() != self.authorization().mailbox().provider() {
            return Err(MailSendProblem::new(MailSendProblemKind::AccountChanged).into());
        }
        let auth = self.authorization().clone();
        let source = self
            .runtime
            .compose_call(false, move |cache| {
                cache.sent_reconciliation_source(&auth, &operation_id)
            })
            .await?;
        let target = match source {
            ReconciliationSource::Status(status) => return Ok(*status),
            ReconciliationSource::Lookup(target) => target,
        };
        target.validate_session(self)?;
        let outcome = lookup.find_sent(self, Arc::clone(&target)).await?;
        target.validate_session(self)?;
        if let SentLookupOutcome::Matched(proof) = &outcome {
            if proof.inner.binding != target.binding {
                return Err(ComposeError::StaleReview);
            }
        }
        let auth = self.authorization().clone();
        let operation_id = target.binding.operation_id.clone();
        self.runtime
            .compose_call(
                matches!(&outcome, SentLookupOutcome::Matched(_)),
                move |cache| {
                    let operation = match outcome {
                        SentLookupOutcome::Matched(proof) => {
                            cache.accept_reconciled_sent(&auth, proof, EpochMs::now())?
                        }
                        SentLookupOutcome::Unconfirmed => {
                            cache.sent_reconciliation_operation(&auth, &operation_id)?
                        }
                    };
                    dispatch::status(cache.host().as_str(), auth.mailbox().as_str(), &operation)
                },
            )
            .await
    }
}
