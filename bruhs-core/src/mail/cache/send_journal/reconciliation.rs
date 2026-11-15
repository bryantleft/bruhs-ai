use super::*;
use crate::mail::{
    compose::{
        dispatch::status,
        reconciliation::{ReconciliationSource, SentLookupTarget, VerifiedSentCopy},
        record::DraftPayload,
        ComposeError,
    },
    MailError,
};
use std::sync::Arc;

impl MailCache {
    // Only the dedicated Sent-read path may use a new connection generation
    // for the same stable mailbox. Ordinary send/claim checks stay unchanged.
    pub(crate) fn sent_reconciliation_operation(
        &self,
        current: &MailAuthorizationRef,
        id: &str,
    ) -> Result<SendOperation, ComposeError> {
        binding(&self.connection, current).map_err(MailError::from)?;
        let operation = load(&self.connection, id)
            .map_err(MailError::from)?
            .ok_or(ComposeError::StaleReview)?;
        if operation.host_id != self.host.as_str()
            || operation.mailbox_id != current.mailbox().as_str()
        {
            return Err(ComposeError::StaleReview);
        }
        Ok(operation)
    }

    pub(crate) fn sent_reconciliation_source(
        &self,
        current: &MailAuthorizationRef,
        id: &str,
    ) -> Result<ReconciliationSource, ComposeError> {
        let operation = self.sent_reconciliation_operation(current, id)?;
        if operation.state != SendState::Uncertain {
            return Ok(ReconciliationSource::Status(Box::new(status(
                self.host.as_str(),
                current.mailbox().as_str(),
                &operation,
            )?)));
        }
        // Client/reply identity is immutable, but the draft may have newer text.
        // The reviewed content and preparation come only from the operation.
        let row = stored_draft(&self.connection, current.mailbox(), &operation.draft_id)?
            .ok_or(ComposeError::PreparedCorrupt)?;
        let reply = match (row.reply_thread_id, row.reply_external_id) {
            (None, None) => None,
            (Some(canonical), Some(external)) => {
                let thread = current
                    .mailbox()
                    .thread(&external)
                    .map_err(CacheError::from)?;
                if thread.canonical_id != canonical {
                    return Err(ComposeError::PreparedCorrupt);
                }
                Some(thread)
            }
            _ => return Err(ComposeError::PreparedCorrupt),
        };
        let reference = current
            .mailbox()
            .draft(&row.client_id, reply.as_ref())
            .map_err(CacheError::from)?;
        if reference.canonical_id != operation.draft_id {
            return Err(ComposeError::PreparedCorrupt);
        }
        let canonical = canonical_payload(operation.payload()).map_err(MailError::from)?;
        let digest = payload_digest(
            &canonical,
            reference
                .reply_to
                .as_ref()
                .map(|thread| thread.canonical_id.as_str()),
        )
        .map_err(MailError::from)?;
        if digest != operation.payload_digest {
            return Err(ComposeError::PreparedCorrupt);
        }
        let payload = DraftPayload::decode(operation.payload(), &reference)?;
        let prepared_ref = payload
            .prepared
            .as_ref()
            .filter(|prepared| prepared.connection_id == operation.connection_id())
            .ok_or(ComposeError::PreparedCorrupt)?;
        let prepared = self.compose_prepared(&reference, prepared_ref)?;
        prepared.check_content(&payload.content)?;
        Ok(ReconciliationSource::Lookup(Arc::new(
            SentLookupTarget::new(self.host.clone(), current.clone(), operation, prepared)?,
        )))
    }

    pub(crate) fn accept_reconciled_sent(
        &mut self,
        current: &MailAuthorizationRef,
        proof: VerifiedSentCopy,
        now: EpochMs,
    ) -> Result<SendOperation, ComposeError> {
        let (proof, receipt) = proof.into_parts();
        if proof.host != self.host
            || proof.mailbox != *current.mailbox()
            || proof
                .lookup_authorization
                .validate_current(current)
                .is_err()
        {
            return Err(ComposeError::StaleReview);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        binding(&tx, current).map_err(MailError::from)?;
        let mut operation = load(&tx, &proof.operation_id)
            .map_err(MailError::from)?
            .ok_or(ComposeError::StaleReview)?;
        if operation.host_id != self.host.as_str()
            || operation.mailbox_id != current.mailbox().as_str()
            || operation.connection_id != proof.original_connection_id
            || operation.draft_id != proof.draft_id
            || operation.draft_revision != proof.draft_revision
            || operation.payload_digest != proof.payload_digest
            || format!("{:x}", Sha256::digest(operation.payload().as_bytes()))
                != proof.payload_sha256
        {
            return Err(ComposeError::StaleReview);
        }
        let prepared_sha = serde_json::from_str::<Value>(operation.payload())
            .map_err(|_| ComposeError::PreparedCorrupt)?;
        if prepared_sha
            .pointer("/prepared/sha256")
            .and_then(Value::as_str)
            != Some(proof.prepared_sha256.as_str())
        {
            return Err(ComposeError::PreparedCorrupt);
        }
        if operation.state == SendState::Accepted {
            tx.commit()?;
            return Ok(operation);
        }
        if operation.state != SendState::Uncertain {
            return Err(MailError::from(OperationError::InvalidTransition).into());
        }
        let stored = json!({"message_id":receipt.message_id,"thread_id":receipt.thread_id,
            "request_id":receipt.request_id,"provider_draft_id":operation.provider_draft_id,
            "accepted_at_ms":clock(now),"delivery":"unknown",
            "reconciled_connection_id":current.connection_id(),"correlation":"frozen_message_id_and_content"}).to_string();
        tx.execute("UPDATE send_operations SET state='accepted',receipt=?1,updated_at_ms=MAX(updated_at_ms,?2) WHERE id=?3",
            params![stored,clock(now),operation.id])?;
        tx.execute("UPDATE drafts SET state='consumed' WHERE mailbox=?1 AND draft_id=?2 AND revision=?3 AND state='editing'",
            params![operation.mailbox_id,operation.draft_id,operation.draft_revision])?;
        tx.commit()?;
        operation.state = SendState::Accepted;
        operation.receipt = Some(stored);
        Ok(operation)
    }
}
