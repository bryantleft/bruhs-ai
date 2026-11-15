//! Durable reviewed-send journal. Provider dispatch is owned by MailRuntime.
//!
//! The runtime must validate recipients/MIME/attachments and obtain current
//! broker access under its lease before claiming a provider dispatch. An id
//! comparison alone grants no access. Acceptance is distinct from delivery.

use super::{
    identity::{MailAuthorizationRef, MailCacheHostId, MailDraftRef, MailIdentityError, MailboxId},
    stored_draft, CacheError, MailCache,
};
use bruhs_models::EpochMs;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};
use std::fmt;

mod prequeue;

#[derive(Debug)]
pub enum OperationError {
    Cache(CacheError),
    InvalidReview,
    HostChanged,
    ReviewChanged,
    OperationConflict,
    DuplicateDraftOperation { operation_id: String },
    InvalidTransition,
    AcceptanceNotSaved(Box<CacheError>),
}
impl From<CacheError> for OperationError {
    fn from(error: CacheError) -> Self {
        Self::Cache(error)
    }
}
impl From<rusqlite::Error> for OperationError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Cache(error.into())
    }
}
impl From<MailIdentityError> for OperationError {
    fn from(error: MailIdentityError) -> Self {
        Self::Cache(error.into())
    }
}
impl fmt::Display for OperationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cache(error) => error.fmt(f),
            Self::InvalidReview => f.write_str("The send review is invalid. Review the draft again."),
            Self::HostChanged => f.write_str("The mail server changed. Return to the server that owns this draft."),
            Self::ReviewChanged => f.write_str("The draft changed after review. Review its current contents."),
            Self::OperationConflict => f.write_str("This send id belongs to a different reviewed action."),
            Self::DuplicateDraftOperation { .. } => f.write_str("This draft already has a pending or accepted send. Open its send status."),
            Self::InvalidTransition => f.write_str("This send cannot continue from its current state. Inspect its status before retrying."),
            Self::AcceptanceNotSaved(_) => f.write_str("The provider accepted this message, but its local receipt could not be saved. Check send status before retrying."),
        }
    }
}
impl std::error::Error for OperationError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendState {
    Queued,
    Dispatching,
    Accepted,
    Uncertain,
    NeedsReview,
    Cancelled,
    Rejected,
}
impl SendState {
    fn from_sql(value: &str) -> rusqlite::Result<Self> {
        match value {
            "queued" => Ok(Self::Queued),
            "dispatching" => Ok(Self::Dispatching),
            "accepted" => Ok(Self::Accepted),
            "uncertain" => Ok(Self::Uncertain),
            "needs_review" => Ok(Self::NeedsReview),
            "cancelled" => Ok(Self::Cancelled),
            "rejected" => Ok(Self::Rejected),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }
}

#[derive(Clone)]
pub struct SendReview {
    host: MailCacheHostId,
    authorization: MailAuthorizationRef,
    draft: MailDraftRef,
    revision: i64,
    payload: String,
    digest: String,
}
impl SendReview {
    pub fn payload(&self) -> &str {
        &self.payload
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
}
impl fmt::Debug for SendReview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SendReview")
            .field("host", &self.host)
            .field("draft_id", &self.draft.canonical_id)
            .field("revision", &self.revision)
            .field("digest", &self.digest)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct SendOperation {
    pub id: String,
    pub state: SendState,
    host_id: String,
    mailbox_id: String,
    connection_id: String,
    pub draft_id: String,
    pub draft_revision: i64,
    pub payload_digest: String,
    payload: String,
    pub provider_draft_id: Option<String>,
    pub receipt: Option<String>,
}
impl SendOperation {
    pub fn connection_id(&self) -> &str {
        &self.connection_id
    }
    pub fn payload(&self) -> &str {
        &self.payload
    }
}
impl fmt::Debug for SendOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SendOperation")
            .field("id", &self.id)
            .field("state", &self.state)
            .field("draft_id", &self.draft_id)
            .field("draft_revision", &self.draft_revision)
            .field("payload_digest", &self.payload_digest)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub enum DispatchClaim {
    Start(Box<SendOperation>),
    InProgress,
    NeedsReconciliation,
    AlreadyAccepted(Box<SendOperation>),
    NeedsReview,
    NotQueued,
}

pub struct ProviderAcceptance {
    pub message_id: Option<String>,
    pub thread_id: Option<String>,
    pub request_id: Option<String>,
}

fn canonical_payload(value: &str) -> Result<String, OperationError> {
    let mut value: Value =
        serde_json::from_str(value).map_err(|_| OperationError::InvalidReview)?;
    if !value.is_object() {
        return Err(OperationError::InvalidReview);
    }
    value.sort_all_objects();
    serde_json::to_string(&value).map_err(|_| OperationError::InvalidReview)
}
fn payload_digest(payload: &str, reply: Option<&str>) -> Result<String, OperationError> {
    let mut hash = Sha256::new();
    for part in ["bruhs.mail.send.payload.v1", reply.unwrap_or(""), payload] {
        hash.update(
            u64::try_from(part.len())
                .map_err(|_| OperationError::InvalidReview)?
                .to_be_bytes(),
        );
        hash.update(part.as_bytes());
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn clock(now: EpochMs) -> i64 {
    now.as_millis()
}
fn binding(connection: &Connection, auth: &MailAuthorizationRef) -> Result<(), OperationError> {
    let generation: Option<String> = connection
        .query_row(
            "SELECT connection_id FROM mailboxes WHERE id=?1",
            [auth.mailbox().as_str()],
            |row| row.get(0),
        )
        .optional()?;
    match generation {
        None => Err(CacheError::UnknownMailbox.into()),
        Some(value) if value != auth.connection_id() => {
            Err(CacheError::AuthorizationChanged.into())
        }
        Some(_) => Ok(()),
    }
}
fn current_operation(
    operation: &SendOperation,
    host: &MailCacheHostId,
    auth: &MailAuthorizationRef,
) -> Result<(), OperationError> {
    if operation.host_id != host.as_str() {
        return Err(OperationError::HostChanged);
    }
    if operation.mailbox_id != auth.mailbox().as_str() {
        return Err(MailIdentityError::MailboxChanged.into());
    }
    if operation.connection_id != auth.connection_id() {
        return Err(MailIdentityError::AuthorizationChanged.into());
    }
    Ok(())
}
fn load(connection: &Connection, id: &str) -> Result<Option<SendOperation>, OperationError> {
    Ok(connection.query_row("SELECT id,state,host_id,mailbox,connection_id,draft_id,draft_revision,payload_digest,payload,provider_draft_id,receipt FROM send_operations WHERE id=?1",[id],|row| Ok(SendOperation { id:row.get(0)?,state:SendState::from_sql(&row.get::<_,String>(1)?)?,host_id:row.get(2)?,mailbox_id:row.get(3)?,connection_id:row.get(4)?,draft_id:row.get(5)?,draft_revision:row.get(6)?,payload_digest:row.get(7)?,payload:row.get(8)?,provider_draft_id:row.get(9)?,receipt:row.get(10)? })).optional()?)
}

struct DraftSnapshot {
    reference: MailDraftRef,
    revision: i64,
    payload: String,
    digest: String,
}
fn draft_snapshot(
    connection: &Connection,
    mailbox: &MailboxId,
    id: &str,
) -> Result<DraftSnapshot, OperationError> {
    let row = stored_draft(connection, mailbox, id)?.ok_or(OperationError::ReviewChanged)?;
    if row.state != "editing" {
        return Err(OperationError::ReviewChanged);
    }
    let reference = match (row.reply_thread_id, row.reply_external_id) {
        (None, None) => mailbox.draft(&row.client_id, None)?,
        (Some(reply), Some(external)) => {
            let thread = mailbox.thread(&external)?;
            if thread.canonical_id != reply {
                return Err(OperationError::InvalidReview);
            }
            mailbox.draft(&row.client_id, Some(&thread))?
        }
        _ => return Err(OperationError::InvalidReview),
    };
    if reference.canonical_id != id {
        return Err(OperationError::InvalidReview);
    }
    let payload = canonical_payload(&row.payload)?;
    let digest = payload_digest(
        &payload,
        reference
            .reply_to
            .as_ref()
            .map(|item| item.canonical_id.as_str()),
    )?;
    Ok(DraftSnapshot {
        reference,
        revision: row.revision,
        payload,
        digest,
    })
}

impl MailCache {
    pub fn record_send_problem(
        &mut self,
        id: &str,
        current: &MailAuthorizationRef,
        uncertain: bool,
        problem: &crate::mail::compose::MailSendProblem,
        now: EpochMs,
    ) -> Result<(), OperationError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record = load(&tx, id)?.ok_or(OperationError::InvalidTransition)?;
        current_operation(&record, &self.host, current)?;
        binding(&tx, current)?;
        if record.state != SendState::Dispatching {
            return Err(OperationError::InvalidTransition);
        }
        let receipt = json!({"problem":problem,"delivery":"unknown"}).to_string();
        tx.execute("UPDATE send_operations SET state=?1,receipt=?2,updated_at_ms=MAX(updated_at_ms,?3) WHERE id=?4",params![if uncertain {"uncertain"} else {"rejected"},receipt,clock(now),id])?;
        tx.commit()?;
        Ok(())
    }
    /// Produces an immutable snapshot for UI review. Provider-specific address,
    /// MIME and attachment preflight is still required before network dispatch.
    pub fn review_send(
        &mut self,
        auth: &MailAuthorizationRef,
        draft: &MailDraftRef,
        expected_revision: i64,
    ) -> Result<SendReview, OperationError> {
        draft.validate()?;
        if draft.mailbox != *auth.mailbox() {
            return Err(MailIdentityError::MailboxChanged.into());
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        binding(&tx, auth)?;
        let snapshot = draft_snapshot(&tx, auth.mailbox(), &draft.canonical_id)?;
        if snapshot.revision != expected_revision || snapshot.reference != *draft {
            return Err(OperationError::ReviewChanged);
        }
        tx.commit()?;
        Ok(SendReview {
            host: self.host.clone(),
            authorization: auth.clone(),
            draft: draft.clone(),
            revision: snapshot.revision,
            payload: snapshot.payload,
            digest: snapshot.digest,
        })
    }

    pub fn queue_send(
        &mut self,
        id: &str,
        review: &SendReview,
        current: &MailAuthorizationRef,
        now: EpochMs,
    ) -> Result<SendOperation, OperationError> {
        if id.is_empty() {
            return Err(OperationError::InvalidReview);
        }
        if review.host != self.host {
            return Err(OperationError::HostChanged);
        }
        review.authorization.validate_current(current)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        binding(&tx, current)?;
        if let Some(existing) = load(&tx, id)? {
            current_operation(&existing, &self.host, current)?;
            if existing.draft_id != review.draft.canonical_id
                || existing.draft_revision != review.revision
                || existing.payload_digest != review.digest
                || existing.payload != review.payload
            {
                return Err(OperationError::OperationConflict);
            }
            tx.commit()?;
            return Ok(existing);
        }
        let duplicate: Option<String>=tx.query_row("SELECT id FROM send_operations WHERE mailbox=?1 AND draft_id=?2 AND (draft_revision=?3 OR payload_digest=?4) AND state IN ('queued','dispatching','accepted','uncertain') LIMIT 1",params![current.mailbox().as_str(),review.draft.canonical_id,review.revision,review.digest],|row|row.get(0)).optional()?;
        if let Some(operation_id) = duplicate {
            return Err(OperationError::DuplicateDraftOperation { operation_id });
        }
        let snapshot = draft_snapshot(&tx, current.mailbox(), &review.draft.canonical_id)?;
        if snapshot.revision != review.revision
            || snapshot.digest != review.digest
            || snapshot.payload != review.payload
            || snapshot.reference != review.draft
        {
            return Err(OperationError::ReviewChanged);
        }
        tx.execute("INSERT INTO send_operations(id,host_id,mailbox,connection_id,draft_id,draft_revision,payload_digest,payload,state,created_at_ms,updated_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'queued',?9,?9)",params![id,self.host.as_str(),current.mailbox().as_str(),current.connection_id(),review.draft.canonical_id,review.revision,review.digest,review.payload,clock(now)])?;
        let operation = load(&tx, id)?.ok_or(OperationError::InvalidTransition)?;
        tx.commit()?;
        Ok(operation)
    }

    /// Only a `Start` result authorizes the owning runtime to begin its single
    /// provider attempt. The runtime retains its already-validated broker lease.
    pub fn claim_send(
        &mut self,
        id: &str,
        current: &MailAuthorizationRef,
        now: EpochMs,
    ) -> Result<DispatchClaim, OperationError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let operation = load(&tx, id)?.ok_or(OperationError::InvalidTransition)?;
        current_operation(&operation, &self.host, current)?;
        binding(&tx, current)?;
        let claim = match operation.state {
            SendState::Queued => {
                let snapshot = match draft_snapshot(&tx, current.mailbox(), &operation.draft_id) {
                    Ok(value) => Some(value),
                    Err(OperationError::ReviewChanged) => None,
                    Err(error) => return Err(error),
                };
                let changed = match snapshot {
                    None => true,
                    Some(snapshot) => {
                        let missing_target = if let Some(reply) = snapshot.reference.reply_to {
                            !tx.query_row(
                                "SELECT EXISTS(SELECT 1 FROM threads WHERE mailbox=?1 AND id=?2)",
                                params![current.mailbox().as_str(), reply.canonical_id],
                                |row| row.get::<_, bool>(0),
                            )?
                        } else {
                            false
                        };
                        snapshot.revision != operation.draft_revision
                            || snapshot.digest != operation.payload_digest
                            || missing_target
                    }
                };
                if changed {
                    tx.execute("UPDATE send_operations SET state='needs_review',updated_at_ms=MAX(updated_at_ms,?1) WHERE id=?2",params![clock(now),id])?;
                    DispatchClaim::NeedsReview
                } else {
                    tx.execute("UPDATE send_operations SET state='dispatching',updated_at_ms=MAX(updated_at_ms,?1) WHERE id=?2",params![clock(now),id])?;
                    let mut claimed = operation;
                    claimed.state = SendState::Dispatching;
                    DispatchClaim::Start(Box::new(claimed))
                }
            }
            SendState::Dispatching => DispatchClaim::InProgress,
            SendState::Uncertain => DispatchClaim::NeedsReconciliation,
            SendState::Accepted => DispatchClaim::AlreadyAccepted(Box::new(operation)),
            SendState::NeedsReview => DispatchClaim::NeedsReview,
            _ => DispatchClaim::NotQueued,
        };
        tx.commit()?;
        Ok(claim)
    }

    pub fn send_operation(
        &self,
        id: &str,
        expected_host: &MailCacheHostId,
        mailbox: &MailboxId,
    ) -> Result<Option<SendOperation>, OperationError> {
        if expected_host != &self.host {
            return Err(OperationError::HostChanged);
        }
        let record = load(&self.connection, id)?;
        if record
            .as_ref()
            .is_some_and(|item| item.mailbox_id != mailbox.as_str())
        {
            return Err(MailIdentityError::MailboxChanged.into());
        }
        Ok(record)
    }

    /// Invoke only after the runtime confirms exclusive ownership following a
    /// stopped/crashed dispatch worker. Opening a second reader is insufficient.
    pub fn recover_interrupted_dispatches(
        &mut self,
        now: EpochMs,
    ) -> Result<usize, OperationError> {
        Ok(self.connection.execute("UPDATE send_operations SET state='uncertain',updated_at_ms=MAX(updated_at_ms,?1) WHERE state='dispatching'",[clock(now)])?)
    }

    pub fn cancel_queued_send(
        &mut self,
        id: &str,
        expected_host: &MailCacheHostId,
        mailbox: &MailboxId,
        now: EpochMs,
    ) -> Result<(), OperationError> {
        if expected_host != &self.host {
            return Err(OperationError::HostChanged);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record = load(&tx, id)?.ok_or(OperationError::InvalidTransition)?;
        if record.mailbox_id != mailbox.as_str() {
            return Err(MailIdentityError::MailboxChanged.into());
        }
        if !matches!(
            record.state,
            SendState::Queued | SendState::NeedsReview | SendState::Rejected | SendState::Cancelled
        ) {
            return Err(OperationError::InvalidTransition);
        }
        tx.execute("UPDATE send_operations SET state='cancelled',updated_at_ms=MAX(updated_at_ms,?1) WHERE id=?2",params![clock(now),id])?;
        tx.commit()?;
        Ok(())
    }

    /// Store only the opaque provider draft id, never an authenticated upload
    /// URL or access token. Large-attachment session secrets need broker storage.
    pub fn checkpoint_provider_draft(
        &mut self,
        id: &str,
        current: &MailAuthorizationRef,
        provider_id: &str,
        now: EpochMs,
    ) -> Result<(), OperationError> {
        if provider_id.is_empty()
            || provider_id.contains("://")
            || provider_id.chars().any(char::is_control)
        {
            return Err(OperationError::InvalidReview);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record = load(&tx, id)?.ok_or(OperationError::InvalidTransition)?;
        current_operation(&record, &self.host, current)?;
        binding(&tx, current)?;
        if record.state != SendState::Dispatching {
            return Err(OperationError::InvalidTransition);
        }
        if record
            .provider_draft_id
            .as_deref()
            .is_some_and(|stored| stored != provider_id)
        {
            return Err(OperationError::OperationConflict);
        }
        tx.execute("UPDATE send_operations SET provider_draft_id=?1,updated_at_ms=MAX(updated_at_ms,?2) WHERE id=?3",params![provider_id,clock(now),id])?;
        tx.commit()?;
        Ok(())
    }

    /// Call after a positive provider acknowledgement or a definitively
    /// correlated Sent-record lookup under the same current authorization.
    /// Reauthorization-aware reconciliation is separate integration work.
    pub fn record_send_acceptance(
        &mut self,
        id: &str,
        current: &MailAuthorizationRef,
        receipt: &ProviderAcceptance,
        now: EpochMs,
    ) -> Result<SendOperation, OperationError> {
        self.persist_send_acceptance(id, current, receipt, now)
            .map_err(|error| match error {
                OperationError::Cache(cache @ CacheError::Storage(_)) => {
                    OperationError::AcceptanceNotSaved(Box::new(cache))
                }
                other => other,
            })
    }

    fn persist_send_acceptance(
        &mut self,
        id: &str,
        current: &MailAuthorizationRef,
        receipt: &ProviderAcceptance,
        now: EpochMs,
    ) -> Result<SendOperation, OperationError> {
        for value in [&receipt.message_id, &receipt.thread_id, &receipt.request_id]
            .into_iter()
            .flatten()
        {
            if value.is_empty() || value.chars().any(char::is_control) {
                return Err(OperationError::InvalidReview);
            }
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut record = load(&tx, id)?.ok_or(OperationError::InvalidTransition)?;
        current_operation(&record, &self.host, current)?;
        binding(&tx, current)?;
        if record.state == SendState::Accepted {
            let prior: Value = serde_json::from_str(
                record
                    .receipt
                    .as_deref()
                    .ok_or(OperationError::InvalidTransition)?,
            )
            .map_err(|_| OperationError::InvalidReview)?;
            if prior.get("message_id") != Some(&json!(receipt.message_id))
                || prior.get("thread_id") != Some(&json!(receipt.thread_id))
                || prior.get("request_id") != Some(&json!(receipt.request_id))
            {
                return Err(OperationError::OperationConflict);
            }
            tx.commit()?;
            return Ok(record);
        }
        if !matches!(record.state, SendState::Dispatching | SendState::Uncertain) {
            return Err(OperationError::InvalidTransition);
        }
        let stored=json!({"message_id":receipt.message_id,"thread_id":receipt.thread_id,"request_id":receipt.request_id,"provider_draft_id":record.provider_draft_id,"accepted_at_ms":clock(now),"delivery":"unknown"}).to_string();
        tx.execute("UPDATE send_operations SET state='accepted',receipt=?1,updated_at_ms=MAX(updated_at_ms,?2) WHERE id=?3",params![stored,clock(now),id])?;
        // Preserve every newer edit. Only the exact reviewed draft revision is
        // consumed; the immutable operation retains its own reviewed payload.
        tx.execute("UPDATE drafts SET state='consumed' WHERE mailbox=?1 AND draft_id=?2 AND revision=?3 AND state='editing'",params![record.mailbox_id,record.draft_id,record.draft_revision])?;
        record.state = SendState::Accepted;
        record.receipt = Some(stored);
        tx.commit()?;
        Ok(record)
    }

    /// Only a definite provider rejection may use this transition. A timeout,
    /// dropped connection or unclassified server error must remain uncertain.
    pub fn record_definite_send_rejection(
        &mut self,
        id: &str,
        current: &MailAuthorizationRef,
        now: EpochMs,
    ) -> Result<(), OperationError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record = load(&tx, id)?.ok_or(OperationError::InvalidTransition)?;
        current_operation(&record, &self.host, current)?;
        binding(&tx, current)?;
        if record.state != SendState::Dispatching {
            return Err(OperationError::InvalidTransition);
        }
        tx.execute("UPDATE send_operations SET state='rejected',updated_at_ms=MAX(updated_at_ms,?1) WHERE id=?2",params![clock(now),id])?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
