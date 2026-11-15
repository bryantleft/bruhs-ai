//! Connected-mail SQLite records, cursor transactions and revision-aware drafts.
//!
//! Opened only by the process-owned MailRuntime in production. The lower-level
//! constructor also supports isolated storage tests without provider access.

mod browse;
mod compose;
pub mod draft_attachments;
pub mod identity;
pub mod send_journal;

use bruhs_models::{connections::ServiceProvider, InboxConversation, InboxSource};
use identity::{
    MailAuthorizationRef, MailCacheHostId, MailDraftRef, MailIdentityError, MailboxId,
    RemoteThreadRef,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::Value;
use std::{collections::HashSet, fmt, path::Path};

#[derive(serde::Deserialize)]
struct CachedThread {
    #[serde(flatten)]
    conversation: InboxConversation,
    connected_mail: Option<bruhs_models::mail::ConnectedMailMetadata>,
}

fn validate_metadata(mailbox: &MailboxId, record: &CachedThread) -> Result<(), CacheError> {
    let Some(metadata) = &record.connected_mail else {
        return Ok(());
    };
    if metadata.provider != mailbox.provider()
        || metadata.mailbox_id != mailbox.as_str()
        || Some(metadata.external_thread_id.as_str()) != record.conversation.external_id.as_deref()
        || metadata.messages.len() != record.conversation.messages.len()
    {
        return Err(CacheError::InvalidInput);
    }
    let messages: std::collections::HashMap<_, _> = record
        .conversation
        .messages
        .iter()
        .map(|message| (message.id.as_str(), message))
        .collect();
    let mut seen = HashSet::new();
    let mut attachments = HashSet::new();
    for detail in &metadata.messages {
        let message = messages
            .get(detail.id.as_str())
            .ok_or(CacheError::InvalidInput)?;
        if !seen.insert(detail.id.as_str())
            || message.external_id.as_deref() != Some(detail.external_id.as_str())
            || mailbox.message(&detail.external_id)?.canonical_id != detail.id
        {
            return Err(CacheError::InvalidInput);
        }
        for attachment in &detail.attachments {
            if mailbox.attachment(&detail.external_id, &attachment.part_id)? != attachment.id
                || !attachments.insert(attachment.id.as_str())
            {
                return Err(CacheError::InvalidInput);
            }
        }
    }
    Ok(())
}

pub enum CacheError {
    Storage(rusqlite::Error),
    Identity(MailIdentityError),
    InvalidInput,
    UnsupportedSchema,
    UnknownMailbox,
    AuthorizationChanged,
    RevisionConflict,
    DraftConsumed,
    HostChanged,
    CorruptRecord,
}

impl fmt::Display for CacheError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::HostChanged => "The mail server changed. Return to the server that owns this cache.",
            Self::CorruptRecord => "Saved mail could not be read. Original records were retained; retry sync or restore a backup.",
            Self::DraftConsumed => "This draft was already sent. Start a new draft.",
            Self::Storage(_) => "Mail could not be saved. The sync cursor was not advanced.",
            Self::Identity(_) | Self::InvalidInput => {
                "The mail update is invalid. No change was saved."
            }
            Self::UnsupportedSchema => "This mail cache needs the Bruhs version that wrote it.",
            Self::UnknownMailbox => "This mailbox has not been connected to this mail store.",
            Self::AuthorizationChanged => {
                "The account connection changed. Reconnect and review the operation."
            }
            Self::RevisionConflict => {
                "Mail changed while this operation was pending. Refresh and retry."
            }
        })
    }
}

// SQLite diagnostics may include a caller-supplied value; public diagnostics
// contain only the recoverable message, never cached mail or draft content.
impl fmt::Debug for CacheError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("CacheError")
            .field(&self.to_string())
            .finish()
    }
}

impl std::error::Error for CacheError {}
impl From<rusqlite::Error> for CacheError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Storage(value)
    }
}
impl From<MailIdentityError> for CacheError {
    fn from(value: MailIdentityError) -> Self {
        Self::Identity(value)
    }
}

pub enum CursorScope {
    GmailHistory,
    GmailFullSyncPage,
    OutlookMailbox,
    OutlookFolder(String),
}

impl CursorScope {
    fn key(&self, mailbox: &MailboxId) -> Result<String, CacheError> {
        match (mailbox.provider(), self) {
            (ServiceProvider::Gmail, Self::GmailHistory) => Ok("gmail-history".into()),
            (ServiceProvider::Gmail, Self::GmailFullSyncPage) => Ok("gmail-full-page".into()),
            (ServiceProvider::Outlook, Self::OutlookMailbox) => Ok("outlook-mailbox".into()),
            (ServiceProvider::Outlook, Self::OutlookFolder(id)) if !id.is_empty() => {
                Ok(format!("outlook-folder:{id}"))
            }
            _ => Err(CacheError::InvalidInput),
        }
    }
}

pub struct ThreadWrite {
    pub reference: RemoteThreadRef,
    pub payload: String,
}

pub struct SyncBatch {
    pub upserts: Vec<ThreadWrite>,
    pub confirmed_deleted: Vec<RemoteThreadRef>,
    pub cursor: Option<(CursorScope, String)>,
}

pub struct MailDraftRecord {
    pub reference: MailDraftRef,
    pub revision: i64,
    pub payload: String,
}

pub struct MailCache {
    connection: Connection,
    host: MailCacheHostId,
}

struct StoredDraft {
    client_id: String,
    reply_thread_id: Option<String>,
    reply_external_id: Option<String>,
    revision: i64,
    state: String,
    payload: String,
}

fn stored_draft(
    connection: &Connection,
    mailbox: &MailboxId,
    id: &str,
) -> Result<Option<StoredDraft>, CacheError> {
    Ok(connection.query_row(
        "SELECT client_id,reply_thread_id,reply_external_id,revision,state,payload FROM drafts WHERE mailbox=?1 AND draft_id=?2",
        params![mailbox.as_str(),id],
        |row| Ok(StoredDraft {
            client_id: row.get(0)?,
            reply_thread_id: row.get(1)?,
            reply_external_id: row.get(2)?,
            revision: row.get(3)?,
            state: row.get(4)?,
            payload: row.get(5)?,
        }),
    ).optional()?)
}

fn validate_reference(reference: &RemoteThreadRef) -> Result<(), CacheError> {
    if reference.mailbox.thread(&reference.external_id)? != *reference {
        return Err(CacheError::InvalidInput);
    }
    Ok(())
}

impl MailCache {
    /// `new_host` is used only when a fresh store is initialized. The persisted
    /// host survives reopen even when a caller proposes another fresh UUID.
    pub fn open(path: &Path, new_host: &MailCacheHostId) -> Result<Self, CacheError> {
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        match version {
            0 => {
                let occupied: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%')", [], |row| row.get(0))?;
                if occupied {
                    return Err(CacheError::UnsupportedSchema);
                }
                tx.execute_batch(SCHEMA)?;
                tx.execute(
                    "INSERT INTO cache_meta(singleton,host_id) VALUES(1,?1)",
                    [new_host.as_str()],
                )?;
                tx.pragma_update(None, "user_version", 3)?;
            }
            1 => {
                tx.execute_batch(FULL_SYNC_SCHEMA)?;
                browse::migrate(&tx)?;
                tx.pragma_update(None, "user_version", 3)?;
            }
            2 => {
                browse::migrate(&tx)?;
                tx.pragma_update(None, "user_version", 3)?;
            }
            3..=5 => {}
            _ => return Err(CacheError::UnsupportedSchema),
        }
        if version < 4 {
            tx.execute_batch(draft_attachments::SCHEMA)?;
            tx.pragma_update(None, "user_version", 4)?;
        }
        if version < 5 {
            compose::migrate(&tx)?;
            tx.pragma_update(None, "user_version", 5)?;
        }
        let host: String = tx.query_row(
            "SELECT host_id FROM cache_meta WHERE singleton=1",
            [],
            |row| row.get(0),
        )?;
        let host = MailCacheHostId::from_host_record(&host)?;
        tx.commit()?;
        Ok(Self { connection, host })
    }

    pub fn host(&self) -> &MailCacheHostId {
        &self.host
    }

    /// Caller obtains `auth` from current broker access, not portable state.
    /// Reauthorization retains cached records but invalidates older batches.
    pub fn bind(&mut self, auth: &MailAuthorizationRef) -> Result<i64, CacheError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mailbox = auth.mailbox();
        let current: Option<(String, String, i64)> = tx
            .query_row(
                "SELECT provider,connection_id,revision FROM mailboxes WHERE id=?1",
                [mailbox.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let revision = match current {
            None => {
                tx.execute(
                    "INSERT INTO mailboxes(id,provider,connection_id,revision) VALUES(?1,?2,?3,0)",
                    params![
                        mailbox.as_str(),
                        mailbox.provider().id(),
                        auth.connection_id()
                    ],
                )?;
                0
            }
            Some((provider, generation, revision)) => {
                if provider != mailbox.provider().id() {
                    return Err(CacheError::InvalidInput);
                }
                if generation == auth.connection_id() {
                    revision
                } else {
                    let next = revision.checked_add(1).ok_or(CacheError::InvalidInput)?;
                    tx.execute(
                        "UPDATE mailboxes SET connection_id=?1,revision=?2 WHERE id=?3",
                        params![auth.connection_id(), next, mailbox.as_str()],
                    )?;
                    next
                }
            }
        };
        tx.commit()?;
        Ok(revision)
    }

    pub fn apply_sync(
        &mut self,
        auth: &MailAuthorizationRef,
        expected_revision: i64,
        batch: &SyncBatch,
    ) -> Result<i64, CacheError> {
        self.apply_sync_inner(auth, expected_revision, batch, None)
    }

    pub fn apply_full_sync_page(
        &mut self,
        auth: &MailAuthorizationRef,
        expected_revision: i64,
        batch: &SyncBatch,
        run_id: &str,
        complete: bool,
    ) -> Result<i64, CacheError> {
        if run_id.is_empty() {
            return Err(CacheError::InvalidInput);
        }
        self.apply_sync_inner(auth, expected_revision, batch, Some((run_id, complete)))
    }

    fn apply_sync_inner(
        &mut self,
        auth: &MailAuthorizationRef,
        expected_revision: i64,
        batch: &SyncBatch,
        full_sync: Option<(&str, bool)>,
    ) -> Result<i64, CacheError> {
        let mailbox = auth.mailbox();
        let mut ids = HashSet::new();
        let mut projections = Vec::with_capacity(batch.upserts.len());
        for item in &batch.upserts {
            validate_reference(&item.reference)?;
            let record: CachedThread =
                serde_json::from_str(&item.payload).map_err(|_| CacheError::InvalidInput)?;
            validate_metadata(mailbox, &record)?;
            let payload = &record.conversation;
            if item.reference.mailbox != *mailbox
                || payload.id != item.reference.canonical_id
                || payload.external_id.as_deref() != Some(item.reference.external_id.as_str())
                || payload.source != InboxSource::Email
                || !ids.insert(item.reference.canonical_id.as_str())
            {
                return Err(CacheError::InvalidInput);
            }
            projections.push(browse::projection(&record)?);
        }
        for item in &batch.confirmed_deleted {
            validate_reference(item)?;
            if item.mailbox != *mailbox || !ids.insert(item.canonical_id.as_str()) {
                return Err(CacheError::InvalidInput);
            }
        }
        let cursor = batch
            .cursor
            .as_ref()
            .map(|(scope, value)| {
                if value.is_empty() {
                    return Err(CacheError::InvalidInput);
                }
                Ok((scope.key(mailbox)?, value))
            })
            .transpose()?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (generation, revision): (String, i64) = tx
            .query_row(
                "SELECT connection_id,revision FROM mailboxes WHERE id=?1",
                [mailbox.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or(CacheError::UnknownMailbox)?;
        if generation != auth.connection_id() {
            return Err(CacheError::AuthorizationChanged);
        }
        if revision != expected_revision {
            return Err(CacheError::RevisionConflict);
        }
        let next = revision.checked_add(1).ok_or(CacheError::InvalidInput)?;
        for (item, projection) in batch.upserts.iter().zip(projections) {
            tx.execute("INSERT INTO threads(mailbox,id,external_id,payload,card,search_fields,message_at_ms,unread,archived,snoozed_until_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(mailbox,id) DO UPDATE SET external_id=excluded.external_id,payload=excluded.payload,card=excluded.card,search_fields=excluded.search_fields,message_at_ms=excluded.message_at_ms,unread=excluded.unread,archived=excluded.archived,snoozed_until_ms=excluded.snoozed_until_ms", params![mailbox.as_str(),item.reference.canonical_id,item.reference.external_id,item.payload,projection.card,projection.fields,projection.message_at_ms,projection.unread,projection.archived,projection.snoozed_until_ms])?;
        }
        for item in &batch.confirmed_deleted {
            tx.execute(
                "DELETE FROM threads WHERE mailbox=?1 AND id=?2",
                params![mailbox.as_str(), item.canonical_id],
            )?;
        }
        if let Some((run_id, complete)) = full_sync {
            tx.execute(
                "DELETE FROM full_sync_seen WHERE mailbox=?1 AND run_id<>?2",
                params![mailbox.as_str(), run_id],
            )?;
            for item in &batch.upserts {
                tx.execute("INSERT OR IGNORE INTO full_sync_seen(mailbox,run_id,external_id) VALUES(?1,?2,?3)", params![mailbox.as_str(), run_id, item.reference.external_id])?;
            }
            if complete {
                tx.execute("DELETE FROM threads WHERE mailbox=?1 AND NOT EXISTS(SELECT 1 FROM full_sync_seen AS seen WHERE seen.mailbox=threads.mailbox AND seen.run_id=?2 AND seen.external_id=threads.external_id)", params![mailbox.as_str(), run_id])?;
                tx.execute(
                    "DELETE FROM full_sync_seen WHERE mailbox=?1",
                    [mailbox.as_str()],
                )?;
            }
        }
        if let Some((key, value)) = cursor {
            tx.execute("INSERT INTO cursors(mailbox,scope,value) VALUES(?1,?2,?3) ON CONFLICT(mailbox,scope) DO UPDATE SET value=excluded.value", params![mailbox.as_str(),key,value])?;
        }
        tx.execute(
            "UPDATE mailboxes SET revision=?1 WHERE id=?2",
            params![next, mailbox.as_str()],
        )?;
        tx.commit()?;
        Ok(next)
    }

    pub fn thread(&self, reference: &RemoteThreadRef) -> Result<Option<String>, CacheError> {
        validate_reference(reference)?;
        Ok(self
            .connection
            .query_row(
                "SELECT payload FROM threads WHERE mailbox=?1 AND id=?2",
                params![reference.mailbox.as_str(), reference.canonical_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn cursor(
        &self,
        mailbox: &MailboxId,
        scope: &CursorScope,
    ) -> Result<Option<String>, CacheError> {
        Ok(self
            .connection
            .query_row(
                "SELECT value FROM cursors WHERE mailbox=?1 AND scope=?2",
                params![mailbox.as_str(), scope.key(mailbox)?],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn sync_checkpoint(
        &mut self,
        auth: &MailAuthorizationRef,
        scope: &CursorScope,
    ) -> Result<(i64, Option<String>), CacheError> {
        let key = scope.key(auth.mailbox())?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        let (generation, revision): (String, i64) = tx
            .query_row(
                "SELECT connection_id,revision FROM mailboxes WHERE id=?1",
                [auth.mailbox().as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or(CacheError::UnknownMailbox)?;
        if generation != auth.connection_id() {
            return Err(CacheError::AuthorizationChanged);
        }
        let cursor = tx
            .query_row(
                "SELECT value FROM cursors WHERE mailbox=?1 AND scope=?2",
                params![auth.mailbox().as_str(), key],
                |row| row.get(0),
            )
            .optional()?;
        tx.commit()?;
        Ok((revision, cursor))
    }

    #[cfg(test)]
    pub fn save_draft(
        &mut self,
        reference: &RemoteThreadRef,
        expected_revision: i64,
        payload: &str,
    ) -> Result<i64, CacheError> {
        validate_reference(reference)?;
        let draft = reference.mailbox.draft(
            &format!("reply:{}", reference.canonical_id),
            Some(reference),
        )?;
        self.save_mail_draft(&draft, expected_revision, payload)
    }

    #[cfg(test)]
    pub fn draft(&self, reference: &RemoteThreadRef) -> Result<Option<(i64, String)>, CacheError> {
        validate_reference(reference)?;
        let draft = reference.mailbox.draft(
            &format!("reply:{}", reference.canonical_id),
            Some(reference),
        )?;
        self.mail_draft(&draft)
    }

    pub fn save_mail_draft(
        &mut self,
        reference: &MailDraftRef,
        expected_revision: i64,
        payload: &str,
    ) -> Result<i64, CacheError> {
        reference.validate()?;
        if expected_revision < 0 {
            return Err(CacheError::InvalidInput);
        }
        let value: Value = serde_json::from_str(payload).map_err(|_| CacheError::InvalidInput)?;
        if !value.is_object() {
            return Err(CacheError::InvalidInput);
        }
        let reply_id = reference
            .reply_to
            .as_ref()
            .map(|item| item.canonical_id.as_str());
        let reply_external = reference
            .reply_to
            .as_ref()
            .map(|item| item.external_id.as_str());
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = stored_draft(&tx, &reference.mailbox, &reference.canonical_id)?;
        let revision = match current {
            Some(current) => {
                if current.state == "consumed" {
                    return Err(CacheError::DraftConsumed);
                }
                if current.client_id != reference.client_id
                    || current.reply_thread_id.as_deref() != reply_id
                    || current.reply_external_id.as_deref() != reply_external
                    || expected_revision > current.revision
                {
                    return Err(CacheError::RevisionConflict);
                }
                let previous: Value =
                    serde_json::from_str(&current.payload).map_err(|_| CacheError::InvalidInput)?;
                if previous == value {
                    tx.commit()?;
                    return Ok(current.revision);
                }
                current.revision
            }
            None => 0,
        };
        if revision != expected_revision {
            return Err(CacheError::RevisionConflict);
        }
        let next = revision.checked_add(1).ok_or(CacheError::InvalidInput)?;
        tx.execute("INSERT INTO drafts(mailbox,draft_id,client_id,reply_thread_id,reply_external_id,revision,payload) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(mailbox,draft_id) DO UPDATE SET revision=excluded.revision,payload=excluded.payload", params![reference.mailbox.as_str(),reference.canonical_id,reference.client_id,reply_id,reply_external,next,payload])?;
        tx.commit()?;
        Ok(next)
    }

    pub fn mail_draft(
        &self,
        reference: &MailDraftRef,
    ) -> Result<Option<(i64, String)>, CacheError> {
        reference.validate()?;
        let row: Option<(i64,String,Option<String>,Option<String>)> = self.connection.query_row(
            "SELECT revision,payload,reply_thread_id,reply_external_id FROM drafts WHERE mailbox=?1 AND draft_id=?2 AND state='editing'",
            params![reference.mailbox.as_str(),reference.canonical_id],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
        ).optional()?;
        row.map(|(revision, payload, reply, external)| {
            if reply.as_deref()
                != reference
                    .reply_to
                    .as_ref()
                    .map(|item| item.canonical_id.as_str())
                || external.as_deref()
                    != reference
                        .reply_to
                        .as_ref()
                        .map(|item| item.external_id.as_str())
            {
                return Err(CacheError::RevisionConflict);
            }
            Ok((revision, payload))
        })
        .transpose()
    }

    /// Storage-level recovery. The eventual UI API must page/project metadata;
    /// it must not mount every complete draft body in a list.
    pub fn drafts(&self, mailbox: &MailboxId) -> Result<Vec<MailDraftRecord>, CacheError> {
        let mut statement = self.connection.prepare("SELECT draft_id,client_id,reply_thread_id,reply_external_id,revision,payload FROM drafts WHERE mailbox=?1 AND state='editing' ORDER BY draft_id")?;
        let rows = statement.query_map([mailbox.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
            ))
        })?;
        rows.map(|row| {
            let (id, client, reply, external, revision, payload) = row?;
            let reference = match (reply, external) {
                (None, None) => mailbox.draft(&client, None)?,
                (Some(reply), Some(external)) => {
                    let thread = mailbox.thread(&external)?;
                    if thread.canonical_id != reply {
                        return Err(CacheError::InvalidInput);
                    }
                    mailbox.draft(&client, Some(&thread))?
                }
                _ => return Err(CacheError::InvalidInput),
            };
            if reference.canonical_id != id {
                return Err(CacheError::InvalidInput);
            }
            let value: Value =
                serde_json::from_str(&payload).map_err(|_| CacheError::InvalidInput)?;
            if !value.is_object() {
                return Err(CacheError::InvalidInput);
            }
            Ok(MailDraftRecord {
                reference,
                revision,
                payload,
            })
        })
        .collect()
    }
}

const FULL_SYNC_SCHEMA: &str = "CREATE TABLE full_sync_seen(mailbox TEXT NOT NULL REFERENCES mailboxes(id) ON DELETE CASCADE,run_id TEXT NOT NULL,external_id TEXT NOT NULL,PRIMARY KEY(mailbox,run_id,external_id));";

const SCHEMA: &str = "
CREATE TABLE cache_meta(singleton INTEGER PRIMARY KEY CHECK(singleton=1),host_id TEXT NOT NULL);
CREATE TABLE mailboxes(id TEXT PRIMARY KEY,provider TEXT NOT NULL CHECK(provider IN ('gmail','outlook')),connection_id TEXT NOT NULL,revision INTEGER NOT NULL CHECK(typeof(revision)='integer' AND revision>=0));
CREATE TABLE threads(mailbox TEXT NOT NULL REFERENCES mailboxes(id) ON DELETE CASCADE,id TEXT NOT NULL,external_id TEXT NOT NULL,payload TEXT NOT NULL,card TEXT NOT NULL,search_fields TEXT NOT NULL,message_at_ms INTEGER NOT NULL,unread INTEGER NOT NULL,archived INTEGER NOT NULL,snoozed_until_ms INTEGER,PRIMARY KEY(mailbox,id),UNIQUE(mailbox,external_id));
CREATE INDEX mail_threads_browse ON threads(mailbox,message_at_ms DESC,id);
CREATE TABLE cursors(mailbox TEXT NOT NULL REFERENCES mailboxes(id) ON DELETE CASCADE,scope TEXT NOT NULL,value TEXT NOT NULL,PRIMARY KEY(mailbox,scope));
CREATE TABLE drafts(mailbox TEXT NOT NULL REFERENCES mailboxes(id) ON DELETE CASCADE,draft_id TEXT NOT NULL,client_id TEXT NOT NULL,reply_thread_id TEXT,reply_external_id TEXT,state TEXT NOT NULL DEFAULT 'editing' CHECK(state IN ('editing','consumed')),revision INTEGER NOT NULL CHECK(typeof(revision)='integer' AND revision>0),payload TEXT NOT NULL,PRIMARY KEY(mailbox,draft_id),CHECK((reply_thread_id IS NULL AND reply_external_id IS NULL) OR (reply_thread_id IS NOT NULL AND reply_external_id IS NOT NULL)));
CREATE TABLE send_operations(id TEXT PRIMARY KEY,host_id TEXT NOT NULL,mailbox TEXT NOT NULL REFERENCES mailboxes(id),connection_id TEXT NOT NULL,draft_id TEXT NOT NULL,draft_revision INTEGER NOT NULL,payload_digest TEXT NOT NULL,payload TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('queued','dispatching','accepted','uncertain','needs_review','cancelled','rejected')),provider_draft_id TEXT,receipt TEXT,created_at_ms INTEGER NOT NULL,updated_at_ms INTEGER NOT NULL);
CREATE UNIQUE INDEX one_send_per_draft_revision ON send_operations(mailbox,draft_id,draft_revision) WHERE state IN ('queued','dispatching','accepted','uncertain');
CREATE UNIQUE INDEX one_send_per_draft_payload ON send_operations(mailbox,draft_id,payload_digest) WHERE state IN ('queued','dispatching','accepted','uncertain');
CREATE TABLE full_sync_seen(mailbox TEXT NOT NULL REFERENCES mailboxes(id) ON DELETE CASCADE,run_id TEXT NOT NULL,external_id TEXT NOT NULL,PRIMARY KEY(mailbox,run_id,external_id));

";

#[cfg(test)]
mod tests;
