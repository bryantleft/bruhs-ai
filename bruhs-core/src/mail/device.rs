//! Private device copies for native email composition. This store contains no
//! OAuth capability and can never dispatch a send. Origin, server and mailbox
//! form its namespace; text and newly selected bytes commit together.
use super::ownership;
use bruhs_models::{connections::ServiceProvider, mail::*, EpochMs};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{path::Path, sync::Arc};
use tokio::sync::Mutex;

#[cfg(test)]
mod tests;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceDraftScope {
    pub origin: String,
    pub host_id: String,
    pub mailbox_id: String,
}
impl DeviceDraftScope {
    fn key(&self) -> Result<String, DeviceDraftError> {
        if self.origin.is_empty()
            || self.origin.len() > 4096
            || self.host_id.is_empty()
            || self.host_id.len() > 1024
            || self.mailbox_id.is_empty()
            || self.mailbox_id.len() > 256
        {
            return Err(DeviceDraftError::Invalid);
        }
        Ok(digest(
            &serde_json::to_vec(self).map_err(|_| DeviceDraftError::Invalid)?,
        ))
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct DeviceDraftFile {
    pub id: String,
    pub file_name: String,
    pub size_bytes: u64,
    pub server: Option<MailDraftFile>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceDraft {
    pub version: u8,
    pub scope: DeviceDraftScope,
    pub provider: ServiceProvider,
    pub client_id: String,
    pub reply: Option<MailReplyTarget>,
    pub content: MailComposeContent,
    pub server_revision: i64,
    pub files: Vec<DeviceDraftFile>,
    pub pending_remove: Vec<String>,
    pub pending_send: Option<MailSendRequest>,
    pub latest_send: Option<MailSendStatus>,
    pub dirty: bool,
    pub updated_at_ms: i64,
}
impl DeviceDraft {
    pub fn new(
        scope: DeviceDraftScope,
        provider: ServiceProvider,
        reply: Option<MailReplyTarget>,
        content: MailComposeContent,
    ) -> Self {
        Self {
            version: 1,
            scope,
            provider,
            client_id: uuid::Uuid::new_v4().to_string(),
            reply,
            content,
            server_revision: 0,
            files: Vec::new(),
            pending_remove: Vec::new(),
            pending_send: None,
            latest_send: None,
            dirty: true,
            updated_at_ms: EpochMs::now().as_millis(),
        }
    }
    pub fn from_server(
        scope: DeviceDraftScope,
        provider: ServiceProvider,
        draft: MailComposeDraft,
    ) -> Result<Self, DeviceDraftError> {
        if scope.host_id != draft.host_id || scope.mailbox_id != draft.mailbox_id {
            return Err(DeviceDraftError::Invalid);
        }
        let files = draft
            .content
            .attachments
            .iter()
            .map(|id| {
                let file = draft
                    .uploads
                    .iter()
                    .find(|file| &file.id == id)
                    .ok_or(DeviceDraftError::Corrupt)?;
                Ok(DeviceDraftFile {
                    id: id.clone(),
                    file_name: file.file_name.clone(),
                    size_bytes: file.size_bytes,
                    server: Some(file.clone()),
                })
            })
            .collect::<Result<Vec<_>, DeviceDraftError>>()?;
        let value = Self {
            version: 1,
            scope,
            provider,
            client_id: draft.client_id,
            reply: draft.reply,
            content: draft.content,
            server_revision: draft.revision,
            files,
            pending_remove: Vec::new(),
            pending_send: None,
            latest_send: draft.latest_send,
            dirty: false,
            updated_at_ms: EpochMs::now().as_millis(),
        };
        value.validate()?;
        Ok(value)
    }
    pub fn refresh_from_server(&self, remote: MailComposeDraft) -> Result<Self, DeviceDraftError> {
        let mut value = Self::from_server(self.scope.clone(), self.provider, remote)?;
        for file in &mut value.files {
            if let Some(old) = self
                .files
                .iter()
                .find(|old| old.server.is_some() && old.server == file.server)
            {
                file.id = old.id.clone();
            }
        }
        Ok(value)
    }
    pub fn validate(&self) -> Result<(), DeviceDraftError> {
        self.scope.key()?;
        self.content
            .validate_storage()
            .map_err(|_| DeviceDraftError::Invalid)?;
        if self.version != 1
            || !valid_id(&self.client_id)
            || self.client_id.len() > 128
            || self.server_revision < 0
            || !(0..=253_402_300_799_999).contains(&self.updated_at_ms)
            || !matches!(
                self.provider,
                ServiceProvider::Gmail | ServiceProvider::Outlook
            )
            || self.files.len() > MAIL_DRAFT_MAX_ATTACHMENTS
            || self.pending_remove.len() > 512
        {
            return Err(DeviceDraftError::Invalid);
        }
        let mut total = 0u64;
        let mut seen = std::collections::HashSet::new();
        for file in &self.files {
            if !valid_id(&file.id)
                || !seen.insert(&file.id)
                || file.file_name.len() > 1024
                || attachment_file_name(&file.file_name) != file.file_name
            {
                return Err(DeviceDraftError::Invalid);
            }
            total = total
                .checked_add(file.size_bytes)
                .ok_or(DeviceDraftError::Invalid)?;
            if file.server.as_ref().is_some_and(|server| {
                server.file_name != file.file_name || server.size_bytes != file.size_bytes
            }) {
                return Err(DeviceDraftError::Corrupt);
            }
        }
        if total
            > u64::try_from(MAIL_DRAFT_ATTACHMENT_MAX_BYTES)
                .map_err(|_| DeviceDraftError::Invalid)?
        {
            return Err(DeviceDraftError::Invalid);
        }
        Ok(())
    }
    pub fn consumed(&self) -> bool {
        self.latest_send.as_ref().is_some_and(|status| {
            status.state == MailDispatchState::Accepted
                && status.draft_revision == self.server_revision
                && !self.dirty
        })
    }
    pub fn listed(&self) -> bool {
        self.dirty || self.pending_send.is_some() || !self.consumed()
    }
}
fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Clone)]
pub struct DeviceDraftRecord {
    pub revision: i64,
    pub value: DeviceDraft,
}
#[derive(Clone)]
pub struct DeviceDraftRow {
    pub client_id: String,
    pub subject: String,
    pub recipients: String,
    pub updated_at_ms: i64,
    pub pending: bool,
}
pub struct DeviceFileBytes {
    pub id: String,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Copy, Debug, thiserror::Error)]
pub enum DeviceDraftError {
    #[error("The email draft could not be saved on this device. Keep the editor open and retry.")]
    Storage,
    #[error("Another app instance changed this device draft. Your current edits remain here; open a separate copy before replacing them.")]
    Conflict,
    #[error("The saved device draft or file could not be read. The original was retained.")]
    Corrupt,
    #[error("The draft exceeds its text or file limits, or belongs to another server.")]
    Invalid,
    #[error(
        "Another app instance owns the device email store. Close that instance before retrying."
    )]
    Busy,
    #[error("The device email storage worker stopped. Keep the editor open and retry.")]
    Worker,
}
impl From<rusqlite::Error> for DeviceDraftError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}
#[derive(Clone)]
pub struct MailDeviceStore {
    db: Arc<Mutex<Connection>>,
    owner: Arc<ownership::RuntimeLock>,
}
impl MailDeviceStore {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, DeviceDraftError> {
        let path = path.as_ref().to_owned();
        let (db,owner)=tokio::task::spawn_blocking(move||{
            let (path,owner)=ownership::acquire(&path).map_err(|error|if matches!(error,super::MailError::AlreadyOwned){DeviceDraftError::Busy}else{DeviceDraftError::Storage})?;
            let db=Connection::open(path)?;
            db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")?;
            let version:i64=db.query_row("PRAGMA user_version",[],|row|row.get(0))?;
            if version==0 {db.execute_batch("BEGIN IMMEDIATE; CREATE TABLE device_drafts(scope TEXT NOT NULL,client TEXT NOT NULL,revision INTEGER NOT NULL,payload TEXT NOT NULL,subject TEXT NOT NULL,recipients TEXT NOT NULL,updated INTEGER NOT NULL,listed INTEGER NOT NULL,pending INTEGER NOT NULL,PRIMARY KEY(scope,client)); CREATE INDEX device_draft_list ON device_drafts(scope,listed,updated DESC,client); CREATE TABLE device_files(scope TEXT NOT NULL,client TEXT NOT NULL,id TEXT NOT NULL,bytes BLOB NOT NULL,sha256 TEXT NOT NULL,PRIMARY KEY(scope,client,id),FOREIGN KEY(scope,client) REFERENCES device_drafts(scope,client) ON DELETE CASCADE); PRAGMA user_version=1; COMMIT;")?;} else if version!=1{return Err(DeviceDraftError::Corrupt);}
            Ok::<_,DeviceDraftError>((db,owner))
        }).await.map_err(|_|DeviceDraftError::Worker)??;
        Ok(Self {
            db: Arc::new(Mutex::new(db)),
            owner: Arc::new(owner),
        })
    }
    async fn call<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Connection) -> Result<T, DeviceDraftError> + Send + 'static,
    ) -> Result<T, DeviceDraftError> {
        let mut db = self.db.clone().lock_owned().await;
        let owner = Arc::clone(&self.owner);
        tokio::task::spawn_blocking(move || {
            let result = work(&mut db);
            drop(db);
            drop(owner);
            result
        })
        .await
        .map_err(|_| DeviceDraftError::Worker)?
    }
    pub async fn load(
        &self,
        scope: DeviceDraftScope,
        client: String,
    ) -> Result<Option<DeviceDraftRecord>, DeviceDraftError> {
        self.call(move |db| read(db, &scope, &client)).await
    }
    pub async fn list(
        &self,
        scope: DeviceDraftScope,
        page: u64,
    ) -> Result<Vec<DeviceDraftRow>, DeviceDraftError> {
        self.call(move|db|{
        let scope=scope.key()?;let offset=page.checked_mul(50).and_then(|v|i64::try_from(v).ok()).ok_or(DeviceDraftError::Invalid)?;
        let mut query=db.prepare("SELECT client,subject,recipients,updated,pending FROM device_drafts WHERE scope=?1 AND listed=1 ORDER BY updated DESC,client LIMIT 50 OFFSET ?2")?;
        let rows=query.query_map(params![scope,offset],|row|Ok(DeviceDraftRow{client_id:row.get(0)?,subject:row.get(1)?,recipients:row.get(2)?,updated_at_ms:row.get(3)?,pending:row.get(4)?}))?;
        rows.collect::<Result<Vec<_>,_>>().map_err(Into::into)
    }).await
    }
    pub async fn save(
        &self,
        value: DeviceDraft,
        expected: Option<i64>,
        added: Vec<DeviceFileBytes>,
    ) -> Result<DeviceDraftRecord, DeviceDraftError> {
        value.validate()?;
        self.call(move |db| save_record(db, value, expected, added))
            .await
    }
    /// The UI has stopped its writers and supplies its final authoritative
    /// snapshot. Read and compare-and-save under the same owned mutex so an
    /// older queued writer cannot interleave or overwrite this final version.
    pub async fn save_on_shutdown(
        &self,
        value: DeviceDraft,
        added: Vec<DeviceFileBytes>,
    ) -> Result<DeviceDraftRecord, DeviceDraftError> {
        value.validate()?;
        self.call(move |db| {
            let expected = read(db, &value.scope, &value.client_id)?.map(|record| record.revision);
            save_record(db, value, expected, added)
        })
        .await
    }
    pub async fn file(
        &self,
        scope: DeviceDraftScope,
        client: String,
        id: String,
    ) -> Result<Vec<u8>, DeviceDraftError> {
        self.call(move |db| {
            let record = read(db, &scope, &client)?.ok_or(DeviceDraftError::Corrupt)?;
            let descriptor = record
                .value
                .files
                .iter()
                .find(|file| file.id == id)
                .ok_or(DeviceDraftError::Corrupt)?;
            let (bytes, hash): (Vec<u8>, String) = db
                .query_row(
                    "SELECT bytes,sha256 FROM device_files WHERE scope=?1 AND client=?2 AND id=?3",
                    params![scope.key()?, client, id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?
                .ok_or(DeviceDraftError::Corrupt)?;
            if u64::try_from(bytes.len()).ok() != Some(descriptor.size_bytes)
                || digest(&bytes) != hash
            {
                return Err(DeviceDraftError::Corrupt);
            }
            Ok(bytes)
        })
        .await
    }
}
fn save_record(
    db: &mut Connection,
    value: DeviceDraft,
    expected: Option<i64>,
    added: Vec<DeviceFileBytes>,
) -> Result<DeviceDraftRecord, DeviceDraftError> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let scope = value.scope.key()?;
    let current = read(&tx, &value.scope, &value.client_id)?;
    let payload = serde_json::to_string(&value).map_err(|_| DeviceDraftError::Invalid)?;
    if current.as_ref().map(|r| r.revision) != expected {
        if added.is_empty() {
            if let Some(current) = current {
                if serde_json::to_string(&current.value).map_err(|_| DeviceDraftError::Corrupt)?
                    == payload
                {
                    return Ok(current);
                }
            }
        }
        return Err(DeviceDraftError::Conflict);
    }
    for file in &added {
        let descriptor = value
            .files
            .iter()
            .find(|d| d.id == file.id)
            .ok_or(DeviceDraftError::Invalid)?;
        if descriptor.size_bytes
            != u64::try_from(file.bytes.len()).map_err(|_| DeviceDraftError::Invalid)?
        {
            return Err(DeviceDraftError::Invalid);
        }
        let previous: Option<String> = tx
            .query_row(
                "SELECT sha256 FROM device_files WHERE scope=?1 AND client=?2 AND id=?3",
                params![scope, value.client_id, file.id],
                |row| row.get(0),
            )
            .optional()?;
        if previous.is_some_and(|hash| hash != digest(&file.bytes)) {
            return Err(DeviceDraftError::Conflict);
        }
    }
    let revision = expected
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(DeviceDraftError::Conflict)?;
    let recipients = [&value.content.to, &value.content.cc, &value.content.bcc]
        .into_iter()
        .find(|v| !v.is_empty())
        .map_or("", String::as_str)
        .chars()
        .take(500)
        .collect::<String>();
    let subject = value.content.subject.chars().take(500).collect::<String>();
    tx.execute("INSERT INTO device_drafts(scope,client,revision,payload,subject,recipients,updated,listed,pending) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(scope,client) DO UPDATE SET revision=excluded.revision,payload=excluded.payload,subject=excluded.subject,recipients=excluded.recipients,updated=excluded.updated,listed=excluded.listed,pending=excluded.pending",params![scope,value.client_id,revision,payload,subject,recipients,value.updated_at_ms,value.listed(),value.pending_send.is_some()])?;
    if let Some(current) = current {
        for file in &current.value.files {
            if !value.files.iter().any(|next| next.id == file.id) {
                tx.execute(
                    "DELETE FROM device_files WHERE scope=?1 AND client=?2 AND id=?3",
                    params![scope, value.client_id, file.id],
                )?;
            }
        }
    }
    for file in added {
        tx.execute("INSERT INTO device_files(scope,client,id,bytes,sha256) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(scope,client,id) DO NOTHING",params![scope,value.client_id,file.id,file.bytes,digest(&file.bytes)])?;
    }
    for file in value.files.iter().filter(|file| file.server.is_none()) {
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM device_files WHERE scope=?1 AND client=?2 AND id=?3)",
            params![scope, value.client_id, file.id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(DeviceDraftError::Corrupt);
        }
    }
    tx.commit()?;
    Ok(DeviceDraftRecord { revision, value })
}

fn read(
    db: &Connection,
    scope: &DeviceDraftScope,
    client: &str,
) -> Result<Option<DeviceDraftRecord>, DeviceDraftError> {
    if !valid_id(client) {
        return Err(DeviceDraftError::Invalid);
    }
    let row: Option<(i64, String)> = db
        .query_row(
            "SELECT revision,payload FROM device_drafts WHERE scope=?1 AND client=?2",
            params![scope.key()?, client],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    row.map(|(revision, payload)| {
        let value: DeviceDraft =
            serde_json::from_str(&payload).map_err(|_| DeviceDraftError::Corrupt)?;
        value.validate()?;
        if value.scope != *scope || value.client_id != client {
            return Err(DeviceDraftError::Corrupt);
        }
        Ok(DeviceDraftRecord { revision, value })
    })
    .transpose()
}
