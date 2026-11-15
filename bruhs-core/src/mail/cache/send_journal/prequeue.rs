use super::*;
use crate::mail::compose::{dispatch::check_request, record::DraftPayload, ComposeError};
use bruhs_models::mail::{MailSendProblem, MailSendRequest};

impl MailCache {
    pub(crate) fn reject_unqueued_compose(
        &mut self,
        mailbox_id: &str,
        client_id: &str,
        request: &MailSendRequest,
        problem: &MailSendProblem,
        now: EpochMs,
    ) -> Result<SendOperation, ComposeError> {
        check_request(request)?;
        if request.host_id != self.host.as_str() {
            return Err(CacheError::HostChanged.into());
        }
        if client_id.is_empty()
            || client_id.len() > 128
            || !client_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Err(ComposeError::InvalidDraft);
        }
        // Sharing queue_send's write transaction makes absence authoritative:
        // either the rejection wins, or a real attempt stays untouched.
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (provider, connection): (String, String) = tx
            .query_row(
                "SELECT provider,connection_id FROM mailboxes WHERE id=?1",
                [mailbox_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or(CacheError::UnknownMailbox)?;
        if connection != request.connection_id {
            return Err(CacheError::AuthorizationChanged.into());
        }
        let mailbox =
            MailboxId::from_cache_record(&provider, mailbox_id).map_err(CacheError::from)?;
        let reference = mailbox.draft(client_id, None).map_err(CacheError::from)?;
        if let Some(existing) =
            load(&tx, &request.operation_id).map_err(crate::mail::MailError::from)?
        {
            if existing.host_id != self.host.as_str()
                || existing.mailbox_id != mailbox_id
                || existing.connection_id != request.connection_id
                || existing.draft_id != reference.canonical_id
                || existing.draft_revision != request.revision
                || existing.payload_digest != request.digest
            {
                return Err(ComposeError::StaleReview);
            }
            tx.commit()?;
            return Ok(existing);
        }
        // A second client's negative receipt must not hide the actual pending
        // attempt in the draft's latest-send projection.
        let duplicate: Option<String> = tx.query_row(
            "SELECT id FROM send_operations WHERE mailbox=?1 AND draft_id=?2 AND (state IN ('queued','dispatching','uncertain') OR (state='accepted' AND (draft_revision=?3 OR payload_digest=?4))) LIMIT 1",
            params![mailbox_id, reference.canonical_id, request.revision, request.digest],
            |row| row.get(0),
        ).optional()?;
        if let Some(operation_id) = duplicate {
            return Err(
                crate::mail::MailError::from(OperationError::DuplicateDraftOperation {
                    operation_id,
                })
                .into(),
            );
        }
        let snapshot = draft_snapshot(&tx, &mailbox, &reference.canonical_id)
            .map_err(crate::mail::MailError::from)?;
        if snapshot.revision != request.revision || snapshot.digest != request.digest {
            return Err(ComposeError::StaleReview);
        }
        let payload = DraftPayload::decode(&snapshot.payload, &snapshot.reference)?;
        let prepared = payload
            .prepared
            .as_ref()
            .filter(|prepared| prepared.connection_id == request.connection_id)
            .ok_or(ComposeError::StaleReview)?;
        let present: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM prepared_messages WHERE mailbox=?1 AND draft_id=?2 AND id=?3 AND connection_id=?4 AND source_revision=?5 AND sha256=?6)",
            params![mailbox_id, reference.canonical_id, prepared.id, prepared.connection_id, prepared.source_revision, prepared.sha256],
            |row| row.get(0),
        )?;
        if !present {
            return Err(ComposeError::StaleReview);
        }
        let receipt = json!({"problem":problem,"delivery":"not_attempted"}).to_string();
        tx.execute(
            "INSERT INTO send_operations(id,host_id,mailbox,connection_id,draft_id,draft_revision,payload_digest,payload,state,receipt,created_at_ms,updated_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'rejected',?9,?10,?10)",
            params![request.operation_id, self.host.as_str(), mailbox_id, request.connection_id, reference.canonical_id, snapshot.revision, snapshot.digest, snapshot.payload, receipt, clock(now)],
        )?;
        let operation = load(&tx, &request.operation_id)
            .map_err(crate::mail::MailError::from)?
            .ok_or(ComposeError::StaleReview)?;
        tx.commit()?;
        Ok(operation)
    }
}
