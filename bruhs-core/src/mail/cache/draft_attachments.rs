//! Immutable file uploads owned by one saved draft. Provider reads use the
//! separate Gmail/Outlook attachment adapters; these bytes never come from URLs.

use super::{identity::MailDraftRef, CacheError, MailCache};
use bruhs_models::mail::attachment_file_name;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest as _, Sha256};
use std::fmt;

pub const MAX_DRAFT_ATTACHMENT_BYTES: usize = bruhs_models::mail::MAIL_DRAFT_ATTACHMENT_MAX_BYTES;
pub const MAX_DRAFT_ATTACHMENTS: usize = bruhs_models::mail::MAIL_DRAFT_MAX_ATTACHMENTS;

#[derive(Debug, thiserror::Error)]
pub enum DraftAttachmentError {
    #[error(transparent)]
    Cache(#[from] CacheError),
    #[error("Save this draft before attaching files.")]
    UnknownDraft,
    #[error("Keep outgoing attachment files within 16 MiB combined.")]
    TooLarge,
    #[error("Keep this draft within 20 attached files.")]
    TooMany,
    #[error("The attachment upload is invalid. Choose the file again.")]
    InvalidUpload,
    #[error("This upload id belongs to another file. Choose the file again.")]
    UploadConflict,
    #[error(
        "This draft has a pending or uncertain send. Inspect its status before removing files."
    )]
    SendPending,
    #[error("Remove this file from the draft and save before deleting its stored bytes.")]
    StillAttached,
    #[error("The saved attachment could not be read. Choose the file again.")]
    CorruptRecord,
}

impl From<rusqlite::Error> for DraftAttachmentError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Cache(error.into())
    }
}

/// Content and filenames are deliberately absent from diagnostics.
pub struct DraftAttachmentUpload {
    pub client_id: String,
    pub file_name: String,
    pub bytes: Vec<u8>,
}

impl fmt::Debug for DraftAttachmentUpload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DraftAttachmentUpload")
            .field("bytes", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct DraftAttachment {
    pub id: String,
    pub file_name: String,
    pub size_bytes: u64,
    pub sha256: String,
}

impl fmt::Debug for DraftAttachment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DraftAttachment")
            .field("id", &self.id)
            .field("size_bytes", &self.size_bytes)
            .field("sha256", &self.sha256)
            .finish_non_exhaustive()
    }
}

pub struct DraftAttachmentBytes {
    pub metadata: DraftAttachment,
    pub bytes: Vec<u8>,
}

impl fmt::Debug for DraftAttachmentBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DraftAttachmentBytes")
            .field("metadata", &self.metadata)
            .finish_non_exhaustive()
    }
}

fn validate_draft(
    connection: &Connection,
    reference: &MailDraftRef,
    editing: bool,
) -> Result<(), DraftAttachmentError> {
    reference.validate().map_err(CacheError::from)?;
    let (client, reply, external, state): (String, Option<String>, Option<String>, String) = connection.query_row(
        "SELECT client_id,reply_thread_id,reply_external_id,state FROM drafts WHERE mailbox=?1 AND draft_id=?2",
        params![reference.mailbox.as_str(), reference.canonical_id],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
    ).optional()?.ok_or(DraftAttachmentError::UnknownDraft)?;
    if client != reference.client_id
        || reply.as_deref()
            != reference
                .reply_to
                .as_ref()
                .map(|reply| reply.canonical_id.as_str())
        || external.as_deref()
            != reference
                .reply_to
                .as_ref()
                .map(|reply| reply.external_id.as_str())
    {
        return Err(CacheError::RevisionConflict.into());
    }
    if editing && state != "editing" {
        return Err(CacheError::DraftConsumed.into());
    }
    Ok(())
}

fn checked_metadata(
    reference: &MailDraftRef,
    id: String,
    client: &str,
    file_name: String,
    size: i64,
    sha256: String,
) -> Result<DraftAttachment, DraftAttachmentError> {
    if reference.attachment(client).map_err(CacheError::from)? != id
        || file_name != attachment_file_name(&file_name)
        || file_name.len() > 1024
        || !(0..=16_777_216).contains(&size)
        || sha256.len() != 64
        || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(DraftAttachmentError::CorruptRecord);
    }
    Ok(DraftAttachment {
        id,
        file_name,
        size_bytes: u64::try_from(size).map_err(|_| DraftAttachmentError::CorruptRecord)?,
        sha256,
    })
}

fn load(
    connection: &Connection,
    reference: &MailDraftRef,
    id: &str,
) -> Result<Option<DraftAttachmentBytes>, DraftAttachmentError> {
    let row = connection.query_row(
        "SELECT client_id,file_name,size_bytes,sha256,CASE WHEN length(data)<=16777216 THEN data END FROM draft_attachments WHERE mailbox=?1 AND draft_id=?2 AND id=?3",
        params![reference.mailbox.as_str(),reference.canonical_id,id],
        |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,i64>(2)?,row.get::<_,String>(3)?,row.get::<_,Option<Vec<u8>>>(4)?)),
    ).optional()?;
    row.map(|(client, name, size, sha256, bytes)| {
        let metadata = checked_metadata(reference, id.into(), &client, name, size, sha256)?;
        let bytes = bytes.ok_or(DraftAttachmentError::CorruptRecord)?;
        if u64::try_from(bytes.len()).ok() != Some(metadata.size_bytes)
            || format!("{:x}", Sha256::digest(&bytes)) != metadata.sha256
        {
            return Err(DraftAttachmentError::CorruptRecord);
        }
        Ok(DraftAttachmentBytes { metadata, bytes })
    })
    .transpose()
}

fn contains_attachment(value: &serde_json::Value, id: &str) -> bool {
    match value {
        serde_json::Value::String(value) => value == id,
        serde_json::Value::Array(values) => {
            values.iter().any(|value| contains_attachment(value, id))
        }
        serde_json::Value::Object(values) => {
            values.values().any(|value| contains_attachment(value, id))
        }
        _ => false,
    }
}

impl MailCache {
    /// An upload does not advance the draft or mailbox revision. A later draft
    /// edit explicitly links its immutable id, under that draft's revision check.
    pub fn store_draft_attachment(
        &mut self,
        reference: &MailDraftRef,
        upload: DraftAttachmentUpload,
    ) -> Result<DraftAttachment, DraftAttachmentError> {
        if upload.bytes.len() > MAX_DRAFT_ATTACHMENT_BYTES {
            return Err(DraftAttachmentError::TooLarge);
        }
        if upload.client_id.is_empty()
            || upload.client_id.len() > 128
            || !upload
                .client_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(DraftAttachmentError::InvalidUpload);
        }
        let file_name = attachment_file_name(&upload.file_name);
        if file_name.len() > 1024 {
            return Err(DraftAttachmentError::InvalidUpload);
        }
        let metadata = DraftAttachment {
            id: reference
                .attachment(&upload.client_id)
                .map_err(CacheError::from)?,
            file_name,
            size_bytes: u64::try_from(upload.bytes.len())
                .map_err(|_| DraftAttachmentError::TooLarge)?,
            sha256: format!("{:x}", Sha256::digest(&upload.bytes)),
        };
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_draft(&tx, reference, true)?;
        if let Some(existing) = load(&tx, reference, &metadata.id)? {
            if existing.metadata != metadata || existing.bytes != upload.bytes {
                return Err(DraftAttachmentError::UploadConflict);
            }
            tx.commit()?;
            return Ok(metadata);
        }
        let (count,total): (i64,i64) = tx.query_row(
            "SELECT count(*),COALESCE(sum(size_bytes),0) FROM draft_attachments WHERE mailbox=?1 AND draft_id=?2",
            params![reference.mailbox.as_str(),reference.canonical_id],
            |row|Ok((row.get(0)?,row.get(1)?)),
        )?;
        if usize::try_from(count).map_err(|_| DraftAttachmentError::CorruptRecord)?
            >= MAX_DRAFT_ATTACHMENTS
        {
            return Err(DraftAttachmentError::TooMany);
        }
        if u64::try_from(total)
            .ok()
            .and_then(|total| total.checked_add(metadata.size_bytes))
            .is_none_or(|total| total > 16_777_216)
        {
            return Err(DraftAttachmentError::TooLarge);
        }
        let stored_size =
            i64::try_from(metadata.size_bytes).map_err(|_| DraftAttachmentError::TooLarge)?;
        tx.execute(
            "INSERT INTO draft_attachments(mailbox,draft_id,id,client_id,file_name,size_bytes,sha256,data) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![reference.mailbox.as_str(),reference.canonical_id,metadata.id,upload.client_id,metadata.file_name,stored_size,metadata.sha256,upload.bytes],
        )?;
        tx.commit()?;
        Ok(metadata)
    }

    pub fn draft_attachments(
        &self,
        reference: &MailDraftRef,
    ) -> Result<Vec<DraftAttachment>, DraftAttachmentError> {
        validate_draft(&self.connection, reference, false)?;
        let mut query=self.connection.prepare("SELECT id,client_id,file_name,size_bytes,sha256 FROM draft_attachments WHERE mailbox=?1 AND draft_id=?2 ORDER BY id LIMIT 21")?;
        let rows = query.query_map(
            params![reference.mailbox.as_str(), reference.canonical_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )?;
        let mut attachments = Vec::new();
        for row in rows {
            let (id, client, name, size, sha256) = row?;
            attachments.push(checked_metadata(
                reference, id, &client, name, size, sha256,
            )?);
        }
        if attachments.len() > MAX_DRAFT_ATTACHMENTS {
            return Err(DraftAttachmentError::CorruptRecord);
        }
        Ok(attachments)
    }

    pub fn draft_attachment(
        &self,
        reference: &MailDraftRef,
        id: &str,
    ) -> Result<Option<DraftAttachmentBytes>, DraftAttachmentError> {
        validate_draft(&self.connection, reference, false)?;
        load(&self.connection, reference, id)
    }

    /// A queued/ambiguous send keeps its file inputs. The user first inspects
    /// or cancels that operation; removing a UI chip alone does not delete bytes.
    pub fn remove_draft_attachment(
        &mut self,
        reference: &MailDraftRef,
        id: &str,
    ) -> Result<bool, DraftAttachmentError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_draft(&tx, reference, true)?;
        // Pending/uncertain attempts retain originals. An accepted typed send
        // already owns frozen MIME; keep the conservative legacy protection
        // only when that operation has no corresponding prepared-message blob.
        let pending:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM send_operations AS operation WHERE mailbox=?1 AND draft_id=?2 AND (state IN ('queued','dispatching','uncertain') OR (state='accepted' AND NOT EXISTS(SELECT 1 FROM prepared_messages AS prepared WHERE prepared.mailbox=operation.mailbox AND prepared.draft_id=operation.draft_id AND prepared.id=json_extract(operation.payload,'$.prepared.id') AND prepared.sha256=json_extract(operation.payload,'$.prepared.sha256')))))",params![reference.mailbox.as_str(),reference.canonical_id],|row|row.get(0))?;
        if pending {
            return Err(DraftAttachmentError::SendPending);
        }
        let payload: String = tx.query_row(
            "SELECT payload FROM drafts WHERE mailbox=?1 AND draft_id=?2",
            params![reference.mailbox.as_str(), reference.canonical_id],
            |row| row.get(0),
        )?;
        let payload: serde_json::Value =
            serde_json::from_str(&payload).map_err(|_| DraftAttachmentError::CorruptRecord)?;
        if contains_attachment(&payload, id) {
            return Err(DraftAttachmentError::StillAttached);
        }
        let removed = tx.execute(
            "DELETE FROM draft_attachments WHERE mailbox=?1 AND draft_id=?2 AND id=?3",
            params![reference.mailbox.as_str(), reference.canonical_id, id],
        )?;
        tx.commit()?;
        Ok(removed == 1)
    }
}

pub(super) const SCHEMA: &str = "CREATE TABLE draft_attachments(mailbox TEXT NOT NULL,draft_id TEXT NOT NULL,id TEXT NOT NULL,client_id TEXT NOT NULL,file_name TEXT NOT NULL,size_bytes INTEGER NOT NULL CHECK(typeof(size_bytes)='integer' AND size_bytes>=0 AND size_bytes<=16777216),sha256 TEXT NOT NULL CHECK(length(sha256)=64),data BLOB NOT NULL CHECK(length(data)=size_bytes),PRIMARY KEY(mailbox,draft_id,id),UNIQUE(mailbox,draft_id,client_id),FOREIGN KEY(mailbox,draft_id) REFERENCES drafts(mailbox,draft_id) ON DELETE CASCADE);";

#[cfg(test)]
mod tests;
