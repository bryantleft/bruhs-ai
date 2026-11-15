//! Typed draft CRUD, bounded metadata pages and atomic prepared-MIME storage.

use super::{identity::*, stored_draft, CacheError, MailCache};
use crate::mail::{
    compose::{drafts::ComposeSnapshot, record::*, ComposeError, PreparedMime},
    MailError,
};
use bruhs_models::{mail::*, EpochMs};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest as _, Sha256};

pub(super) const SCHEMA: &str = "
ALTER TABLE mailboxes ADD COLUMN drafts_revision INTEGER NOT NULL DEFAULT 0 CHECK(typeof(drafts_revision)='integer' AND drafts_revision>=0);
ALTER TABLE drafts ADD COLUMN compose_subject TEXT NOT NULL DEFAULT '';
ALTER TABLE drafts ADD COLUMN compose_recipients TEXT NOT NULL DEFAULT '';
ALTER TABLE drafts ADD COLUMN compose_updated_at_ms INTEGER NOT NULL DEFAULT 0;
CREATE INDEX compose_drafts_browse ON drafts(mailbox,state,compose_updated_at_ms DESC,draft_id);
CREATE TRIGGER compose_draft_insert AFTER INSERT ON drafts BEGIN UPDATE mailboxes SET drafts_revision=drafts_revision+1 WHERE id=NEW.mailbox; END;
CREATE TRIGGER compose_draft_update AFTER UPDATE ON drafts BEGIN UPDATE mailboxes SET drafts_revision=drafts_revision+1 WHERE id=NEW.mailbox; END;
CREATE TRIGGER compose_draft_delete AFTER DELETE ON drafts BEGIN UPDATE mailboxes SET drafts_revision=drafts_revision+1 WHERE id=OLD.mailbox; END;
CREATE TABLE prepared_messages(mailbox TEXT NOT NULL,draft_id TEXT NOT NULL,id TEXT NOT NULL,source_revision INTEGER NOT NULL CHECK(typeof(source_revision)='integer' AND source_revision>0),connection_id TEXT NOT NULL,sha256 TEXT NOT NULL,size_bytes INTEGER NOT NULL CHECK(typeof(size_bytes)='integer' AND size_bytes>=0 AND size_bytes<=33554432),metadata TEXT NOT NULL,data BLOB NOT NULL CHECK(length(data)=size_bytes),PRIMARY KEY(mailbox,draft_id,id),FOREIGN KEY(mailbox,draft_id) REFERENCES drafts(mailbox,draft_id) ON DELETE CASCADE);
";

// Static SQL only. The trigger derives small list fields inside the same draft
// transaction; a listing never extracts/parses complete body JSON to sort it.
const PROJECTION: &str = "compose_subject=substr(CASE WHEN json_type(CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,'$.content.subject')='text' THEN json_extract(payload,'$.content.subject') WHEN json_type(CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,'$.subject')='text' THEN json_extract(payload,'$.subject') ELSE '' END,1,500),compose_recipients=substr(CASE WHEN json_type(CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,'$.content.to')='text' THEN COALESCE(NULLIF(json_extract(payload,'$.content.to'),''),NULLIF(json_extract(payload,'$.content.cc'),''),json_extract(payload,'$.content.bcc'),'') ELSE '' END,1,500),compose_updated_at_ms=CASE WHEN json_type(CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,'$.updated_at_ms')='integer' THEN MAX(0,json_extract(payload,'$.updated_at_ms')) ELSE 0 END";

pub(super) fn migrate(connection: &Connection) -> Result<(), CacheError> {
    connection.execute_batch(SCHEMA)?;
    connection.execute_batch(&format!("UPDATE drafts SET {PROJECTION}; CREATE TRIGGER compose_projection_insert AFTER INSERT ON drafts BEGIN UPDATE drafts SET {PROJECTION} WHERE mailbox=NEW.mailbox AND draft_id=NEW.draft_id; END; CREATE TRIGGER compose_projection_update AFTER UPDATE OF payload ON drafts BEGIN UPDATE drafts SET {PROJECTION} WHERE mailbox=NEW.mailbox AND draft_id=NEW.draft_id; END;"))?;
    Ok(())
}

fn client_id(id: &str) -> Result<(), ComposeError> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(ComposeError::InvalidDraft);
    }
    Ok(())
}

struct StoredPreparation {
    revision: i64,
    connection: String,
    sha: String,
    size: i64,
    metadata: Option<String>,
    bytes: Option<Vec<u8>>,
}

fn mailbox(connection: &Connection, id: &str) -> Result<MailboxId, ComposeError> {
    let provider: String = connection
        .query_row("SELECT provider FROM mailboxes WHERE id=?1", [id], |row| {
            row.get(0)
        })
        .optional()?
        .ok_or(CacheError::UnknownMailbox)?;
    Ok(MailboxId::from_cache_record(&provider, id).map_err(CacheError::from)?)
}

fn snapshot(
    connection: &Connection,
    mailbox: &MailboxId,
    client: &str,
) -> Result<Option<ComposeSnapshot>, ComposeError> {
    client_id(client)?;
    let mut reference = mailbox.draft(client, None).map_err(CacheError::from)?;
    let Some(row) = stored_draft(connection, mailbox, &reference.canonical_id)? else {
        return Ok(None);
    };
    if row.state != "editing" {
        return Ok(None);
    }
    reference.reply_to = match (row.reply_thread_id, row.reply_external_id) {
        (None, None) => None,
        (Some(id), Some(external)) => {
            let thread = mailbox.thread(&external).map_err(CacheError::from)?;
            if thread.canonical_id != id {
                return Err(ComposeError::InvalidDraft);
            }
            Some(thread)
        }
        _ => return Err(ComposeError::InvalidDraft),
    };
    if row.client_id != client {
        return Err(ComposeError::InvalidDraft);
    }
    let payload = DraftPayload::decode(&row.payload, &reference)?;
    Ok(Some(ComposeSnapshot {
        reference,
        revision: row.revision,
        payload,
    }))
}

fn latest_status(
    connection: &Connection,
    host: &MailCacheHostId,
    reference: &MailDraftRef,
) -> Result<Option<MailSendStatus>, ComposeError> {
    let row: Option<(String, i64, String, Option<String>)> = connection.query_row(
        "SELECT id,draft_revision,state,receipt FROM send_operations WHERE mailbox=?1 AND draft_id=?2 ORDER BY created_at_ms DESC,rowid DESC LIMIT 1",
        params![reference.mailbox.as_str(),reference.canonical_id],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
    ).optional()?;
    row.map(|(operation_id, draft_revision, state, receipt)| {
        let state = match state.as_str() {
            "queued" => MailDispatchState::Queued,
            "dispatching" => MailDispatchState::Dispatching,
            "accepted" => MailDispatchState::Accepted,
            "uncertain" => MailDispatchState::Uncertain,
            "needs_review" => MailDispatchState::NeedsReview,
            "cancelled" => MailDispatchState::Cancelled,
            "rejected" => MailDispatchState::Rejected,
            _ => return Err(ComposeError::InvalidDraft),
        };
        let receipt: Option<serde_json::Value> = receipt
            .map(|value| serde_json::from_str(&value))
            .transpose()
            .map_err(|_| ComposeError::InvalidDraft)?;
        Ok(MailSendStatus {
            host_id: host.as_str().into(),
            mailbox_id: reference.mailbox.as_str().into(),
            operation_id,
            draft_id: reference.canonical_id.clone(),
            draft_revision,
            state,
            provider_message_id: receipt
                .as_ref()
                .and_then(|value| value.get("message_id"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            problem: receipt
                .as_ref()
                .and_then(|value| value.get("problem"))
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| ComposeError::PreparedCorrupt)?,
            provider_thread_id: receipt
                .as_ref()
                .and_then(|value| value.get("thread_id"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
        })
    })
    .transpose()
}

fn bare_recipients(values: &[MailAddress]) -> String {
    // A source envelope is untrusted input too. Keep its exact safe addresses;
    // malformed source entries remain absent rather than becoming new headers.
    values
        .iter()
        .filter_map(|address| {
            let content = MailComposeContent {
                to: address.address.clone(),
                ..Default::default()
            };
            let parsed = content.validate_send().ok()?;
            (parsed.to.len() == 1
                && parsed.to[0].name.is_empty()
                && parsed.to[0].address == address.address)
                .then(|| address.address.clone())
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn prepared_id(
    reference: &MailDraftRef,
    source_revision: i64,
    connection: &str,
    metadata: &str,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"bruhs.prepared-mail.v1");
    for value in [
        reference.mailbox.as_str(),
        reference.canonical_id.as_str(),
        &source_revision.to_string(),
        connection,
        metadata,
    ] {
        digest.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
        digest.update(value.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

impl MailCache {
    pub(crate) fn compose_cancel_send(
        &mut self,
        mailbox_id: &str,
        id: &str,
    ) -> Result<MailSendStatus, ComposeError> {
        let mailbox = mailbox(&self.connection, mailbox_id)?;
        let host = self.host().clone();
        self.cancel_queued_send(id, &host, &mailbox, EpochMs::now())
            .map_err(MailError::from)?;
        self.compose_send_status(mailbox_id, id)?
            .ok_or(ComposeError::StaleReview)
    }
    pub(crate) fn compose_send_status(
        &self,
        mailbox_id: &str,
        id: &str,
    ) -> Result<Option<MailSendStatus>, ComposeError> {
        let mailbox = mailbox(&self.connection, mailbox_id)?;
        self.send_operation(id, self.host(), &mailbox)
            .map_err(MailError::from)?
            .map(|operation| {
                crate::mail::compose::dispatch::status(self.host().as_str(), mailbox_id, &operation)
            })
            .transpose()
    }
    pub(crate) fn compose_snapshot(
        &self,
        mailbox_id: &str,
        client: &str,
    ) -> Result<ComposeSnapshot, ComposeError> {
        snapshot(
            &self.connection,
            &mailbox(&self.connection, mailbox_id)?,
            client,
        )?
        .ok_or(ComposeError::InvalidDraft)
    }

    pub(crate) fn compose_draft(
        &self,
        mailbox_id: &str,
        client: &str,
    ) -> Result<Option<MailComposeDraft>, ComposeError> {
        snapshot(
            &self.connection,
            &mailbox(&self.connection, mailbox_id)?,
            client,
        )?
        .map(|snapshot| self.compose_view(snapshot))
        .transpose()
    }

    fn compose_view(&self, snapshot: ComposeSnapshot) -> Result<MailComposeDraft, ComposeError> {
        let uploads = self
            .draft_attachments(&snapshot.reference)?
            .into_iter()
            .map(|file| MailDraftFile {
                id: file.id,
                file_name: file.file_name,
                size_bytes: file.size_bytes,
                sha256: file.sha256,
            })
            .collect();
        let latest_send = latest_status(&self.connection, self.host(), &snapshot.reference)?;
        Ok(MailComposeDraft {
            host_id: self.host.as_str().into(),
            mailbox_id: snapshot.reference.mailbox.as_str().into(),
            client_id: snapshot.reference.client_id,
            draft_id: snapshot.reference.canonical_id,
            revision: snapshot.revision,
            reply: snapshot.payload.reply,
            content: snapshot.payload.content,
            uploads,
            latest_send,
        })
    }

    pub(crate) fn compose_create(
        &mut self,
        mailbox_id: &str,
        request: &MailDraftCreate,
    ) -> Result<MailComposeDraft, ComposeError> {
        client_id(&request.client_id)?;
        let mailbox = mailbox(&self.connection, mailbox_id)?;
        if let Some(current) = snapshot(&self.connection, &mailbox, &request.client_id)? {
            if current.payload.reply != request.reply {
                return Err(ComposeError::InvalidReply);
            }
            return self.compose_view(current);
        }
        let mut content = MailComposeContent::default();
        let thread = request
            .reply
            .as_ref()
            .map(|target| {
                if target.external_thread_id.len() > 16384 || target.message_id.len() > 256 {
                    return Err(ComposeError::InvalidReply);
                }
                let reference = mailbox
                    .thread(&target.external_thread_id)
                    .map_err(CacheError::from)?;
                let payload = self.thread(&reference)?.ok_or(ComposeError::InvalidReply)?;
                let thread: ConnectedMailThread =
                    serde_json::from_str(&payload).map_err(|_| ComposeError::InvalidReply)?;
                let detail = thread
                    .connected_mail
                    .messages
                    .iter()
                    .find(|detail| detail.id == target.message_id)
                    .ok_or(ComposeError::InvalidReply)?;
                let message = thread
                    .conversation
                    .messages
                    .iter()
                    .find(|message| message.id == target.message_id)
                    .ok_or(ComposeError::InvalidReply)?;
                content.subject = detail.envelope.subject.clone();
                let addresses = if message.direction == bruhs_models::MessageDirection::Outgoing {
                    &detail.envelope.to
                } else if detail.envelope.reply_to.is_empty() {
                    &detail.envelope.from
                } else {
                    &detail.envelope.reply_to
                };
                content.to = bare_recipients(addresses);
                Ok(reference)
            })
            .transpose()?;
        let reference = mailbox
            .draft(&request.client_id, thread.as_ref())
            .map_err(CacheError::from)?;
        let payload = DraftPayload::new(content, request.reply.clone(), EpochMs::now())?;
        self.save_mail_draft(&reference, 0, &payload.encode()?)?;
        self.compose_view(self.compose_snapshot(mailbox_id, &request.client_id)?)
    }

    pub(crate) fn compose_save(
        &mut self,
        mailbox_id: &str,
        client: &str,
        request: &MailDraftSave,
    ) -> Result<MailComposeDraft, ComposeError> {
        request.content.validate_storage()?;
        let current = self.compose_snapshot(mailbox_id, client)?;
        if request.expected_revision < 1 || request.expected_revision > current.revision {
            return Err(CacheError::RevisionConflict.into());
        }
        if current.payload.content == request.content {
            return self.compose_view(current);
        }
        if current.revision != request.expected_revision {
            return Err(CacheError::RevisionConflict.into());
        }
        let known = self.draft_attachments(&current.reference)?;
        if request
            .content
            .attachments
            .iter()
            .any(|id| !known.iter().any(|file| &file.id == id))
        {
            return Err(ComposeError::AttachmentChanged);
        }
        let mut payload = current.payload;
        payload.edit(request.content.clone(), EpochMs::now())?;
        self.save_mail_draft(
            &current.reference,
            request.expected_revision,
            &payload.encode()?,
        )?;
        self.compose_view(self.compose_snapshot(mailbox_id, client)?)
    }

    pub(crate) fn compose_drafts(
        &mut self,
        mailbox_id: &str,
        requested: u64,
        expected: Option<i64>,
    ) -> Result<MailDraftsPage, ComposeError> {
        const PAGE: u64 = 50;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        let checked = mailbox(&tx, mailbox_id)?;
        let revision: i64 = tx.query_row(
            "SELECT drafts_revision FROM mailboxes WHERE id=?1",
            [mailbox_id],
            |row| row.get(0),
        )?;
        if expected.is_some_and(|value| value != revision) {
            return Err(CacheError::RevisionConflict.into());
        }
        // Project only list metadata. Complete bodies and prepared blobs never
        // become mounted rows, even with thousands of saved drafts.
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM drafts WHERE mailbox=?1 AND state='editing'",
            [mailbox_id],
            |row| row.get(0),
        )?;
        let count = u64::try_from(count).map_err(|_| CacheError::CorruptRecord)?;
        let page = requested.min(count.saturating_sub(1) / PAGE);
        let offset = i64::try_from(page.checked_mul(PAGE).ok_or(ComposeError::InvalidDraft)?)
            .map_err(|_| ComposeError::InvalidDraft)?;
        let mut statement = tx.prepare("SELECT client_id,draft_id,revision,compose_subject,compose_recipients,compose_updated_at_ms FROM drafts WHERE mailbox=?1 AND state='editing' ORDER BY compose_updated_at_ms DESC,draft_id LIMIT 50 OFFSET ?2")?;
        let rows = statement
            .query_map(params![mailbox_id, offset], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        let mut result = Vec::with_capacity(rows.len());
        for (client_id, draft_id, revision, subject, recipients, updated_at_ms) in rows {
            let reference = checked.draft(&client_id, None).map_err(CacheError::from)?;
            if reference.canonical_id != draft_id {
                return Err(ComposeError::InvalidDraft);
            }
            let pending_send =
                latest_status(&tx, &self.host, &reference)?.map(|status| status.state);
            result.push(MailDraftSummary {
                client_id,
                draft_id,
                revision,
                subject,
                recipients,
                updated_at_ms,
                pending_send,
            });
        }
        tx.commit()?;
        Ok(MailDraftsPage {
            host_id: self.host.as_str().into(),
            mailbox_id: mailbox_id.into(),
            revision,
            page,
            total_matches: count,
            rows: result,
        })
    }

    pub(crate) fn compose_freeze(
        &mut self,
        auth: &MailAuthorizationRef,
        source: ComposeSnapshot,
        prepared: PreparedMime,
    ) -> Result<ComposeSnapshot, ComposeError> {
        if source.reference.mailbox != *auth.mailbox() {
            return Err(ComposeError::InvalidDraft);
        }
        prepared.check_content(&source.payload.content)?;
        self.compose_check_reply(&source, &prepared)?;
        let (metadata, bytes) = prepared.into_stored()?;
        let size = i64::try_from(bytes.len()).map_err(|_| ComposeError::PreparedTooLarge)?;
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        let id = prepared_id(
            &source.reference,
            source.revision,
            auth.connection_id(),
            &metadata,
        );
        let mut payload = source.payload.clone();
        payload.prepared = Some(PreparedReference {
            id: id.clone(),
            source_revision: source.revision,
            connection_id: auth.connection_id().into(),
            sha256: sha256.clone(),
        });
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let generation: String = tx.query_row(
            "SELECT connection_id FROM mailboxes WHERE id=?1",
            [auth.mailbox().as_str()],
            |row| row.get(0),
        )?;
        if generation != auth.connection_id() {
            return Err(ComposeError::StaleReview);
        }
        let current = snapshot(&tx, auth.mailbox(), &source.reference.client_id)?
            .ok_or(ComposeError::StaleReview)?;
        if current.revision != source.revision
            || current.reference != source.reference
            || current.payload.encode()? != source.payload.encode()?
        {
            return Err(ComposeError::StaleReview);
        }
        let next = source
            .revision
            .checked_add(1)
            .ok_or(ComposeError::InvalidDraft)?;
        tx.execute("INSERT INTO prepared_messages(mailbox,draft_id,id,source_revision,connection_id,sha256,size_bytes,metadata,data) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![auth.mailbox().as_str(),source.reference.canonical_id,id,source.revision,auth.connection_id(),sha256,size,metadata,bytes])?;
        tx.execute(
            "UPDATE drafts SET revision=?1,payload=?2 WHERE mailbox=?3 AND draft_id=?4",
            params![
                next,
                payload.encode()?,
                auth.mailbox().as_str(),
                source.reference.canonical_id
            ],
        )?;
        tx.commit()?;
        Ok(ComposeSnapshot {
            reference: source.reference,
            revision: next,
            payload,
        })
    }

    pub(crate) fn compose_prepared(
        &self,
        draft: &MailDraftRef,
        prepared: &PreparedReference,
    ) -> Result<PreparedMime, ComposeError> {
        let row: Option<StoredPreparation> = self.connection.query_row("SELECT source_revision,connection_id,sha256,size_bytes,CASE WHEN length(CAST(metadata AS BLOB))<=262144 THEN metadata END,CASE WHEN length(data)<=33554432 THEN data END FROM prepared_messages WHERE mailbox=?1 AND draft_id=?2 AND id=?3",params![draft.mailbox.as_str(),draft.canonical_id,prepared.id],|row|Ok(StoredPreparation { revision:row.get(0)?,connection:row.get(1)?,sha:row.get(2)?,size:row.get(3)?,metadata:row.get(4)?,bytes:row.get(5)? })).optional()?;
        let StoredPreparation {
            revision,
            connection,
            sha,
            size,
            metadata,
            bytes,
        } = row.ok_or(ComposeError::PreparedCorrupt)?;
        let metadata = metadata.ok_or(ComposeError::PreparedCorrupt)?;
        let bytes = bytes.ok_or(ComposeError::PreparedCorrupt)?;
        if revision != prepared.source_revision
            || connection != prepared.connection_id
            || sha != prepared.sha256
            || usize::try_from(size).ok() != Some(bytes.len())
            || prepared_id(draft, revision, &connection, &metadata) != prepared.id
        {
            return Err(ComposeError::PreparedCorrupt);
        }
        let result = PreparedMime::from_stored(&metadata, bytes)?;
        if result.sha256() != sha {
            return Err(ComposeError::PreparedCorrupt);
        }
        Ok(result)
    }

    pub(crate) fn compose_has_pending(
        &self,
        reference: &MailDraftRef,
    ) -> Result<bool, ComposeError> {
        Ok(self.connection.query_row("SELECT EXISTS(SELECT 1 FROM send_operations WHERE mailbox=?1 AND draft_id=?2 AND state IN ('queued','dispatching','uncertain'))",params![reference.mailbox.as_str(),reference.canonical_id],|row|row.get(0))?)
    }

    pub(crate) fn compose_check_reply(
        &self,
        snapshot: &ComposeSnapshot,
        prepared: &PreparedMime,
    ) -> Result<(), ComposeError> {
        let thread: Option<ConnectedMailThread> = snapshot
            .reference
            .reply_to
            .as_ref()
            .map(|reference| {
                self.thread(reference)?
                    .ok_or(ComposeError::InvalidReply)
                    .and_then(|payload| {
                        serde_json::from_str(&payload).map_err(|_| ComposeError::InvalidReply)
                    })
            })
            .transpose()?;
        prepared.check_reply_source(
            &snapshot.reference,
            &snapshot.payload.content,
            snapshot.payload.reply.as_ref(),
            thread.as_ref(),
        )
    }
}

impl From<CacheError> for ComposeError {
    fn from(error: CacheError) -> Self {
        Self::Mail(MailError::Cache(error))
    }
}
impl From<rusqlite::Error> for ComposeError {
    fn from(error: rusqlite::Error) -> Self {
        CacheError::from(error).into()
    }
}
impl From<crate::mail::DraftAttachmentError> for ComposeError {
    fn from(error: crate::mail::DraftAttachmentError) -> Self {
        Self::Mail(MailError::DraftAttachment(error))
    }
}
