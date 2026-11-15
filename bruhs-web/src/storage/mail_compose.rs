//! Private device draft copies and selected Blob bytes. Body, file and list
//! changes commit in one existing metadata-store transaction with a CAS fence.
//! A committed local copy never authorizes a provider send.

use super::{
    from_js,
    metadata::{self, LocalTransaction, Metadata, SaveError},
    open_db, to_js, STORE_METADATA,
};
use bruhs_models::{connections::ServiceProvider, mail::*};
use idb::TransactionMode;
use serde::{Deserialize, Serialize};
use wasm_bindgen::{JsCast, JsValue};

#[derive(Clone, Serialize, Deserialize)]
pub struct LocalFile {
    pub id: String,
    pub file_name: String,
    pub size_bytes: u64,
    pub server: Option<MailDraftFile>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalDraft {
    pub version: u8,
    pub origin: String,
    pub host_id: String,
    pub mailbox_id: String,
    pub provider: ServiceProvider,
    pub client_id: String,
    pub reply: Option<MailReplyTarget>,
    pub content: MailComposeContent,
    pub server_revision: i64,
    pub files: Vec<LocalFile>,
    pub pending_remove: Vec<String>,
    pub pending_send: Option<MailSendRequest>,
    pub latest_send: Option<MailSendStatus>,
    pub dirty: bool,
    pub updated_at_ms: i64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct LocalDraftRow {
    pub key: String,
    pub client_id: String,
    pub subject: String,
    pub recipients: String,
    pub updated_at_ms: i64,
    pub dirty: bool,
    pub pending_send: Option<String>,
}

#[derive(Clone)]
pub struct Scope {
    pub origin: String,
    pub host_id: String,
    pub mailbox_id: String,
}

impl Scope {
    pub fn current(host: &str, mailbox: &str) -> Self {
        Self {
            origin: crate::api::BASE_URL.into(),
            host_id: host.into(),
            mailbox_id: mailbox.into(),
        }
    }
    fn identity(&self, client: Option<&str>) -> Result<String, SaveError> {
        serde_json::to_string(&(&self.origin, &self.host_id, &self.mailbox_id, client))
            .map_err(|_| SaveError::Corrupt)
    }
    pub(crate) fn body_key(&self, client: &str) -> Result<String, SaveError> {
        Ok(format!(
            "bruhs.mailDraft.body.{}",
            self.identity(Some(client))?
        ))
    }
    fn index_prefix(&self) -> Result<String, SaveError> {
        Ok(format!("bruhs.mailDraft.index.{}:", self.identity(None)?))
    }
    fn file_key(&self, client: &str, id: &str) -> Result<String, SaveError> {
        Ok(format!(
            "bruhs.mailDraft.file.{}:{}",
            self.identity(Some(client))?,
            serde_json::to_string(id).map_err(|_| SaveError::Corrupt)?
        ))
    }
    pub fn same(&self, other: &Self) -> bool {
        self.origin == other.origin
            && self.host_id == other.host_id
            && self.mailbox_id == other.mailbox_id
    }
    fn matches(&self, value: &LocalDraft) -> bool {
        self.origin == value.origin
            && self.host_id == value.host_id
            && self.mailbox_id == value.mailbox_id
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

impl LocalDraft {
    pub fn from_server(scope: Scope, provider: ServiceProvider, draft: MailComposeDraft) -> Self {
        let files = draft
            .content
            .attachments
            .iter()
            .filter_map(|id| draft.uploads.iter().find(|file| &file.id == id))
            .map(|file| LocalFile {
                id: file.id.clone(),
                file_name: file.file_name.clone(),
                size_bytes: file.size_bytes,
                server: Some(file.clone()),
            })
            .collect();
        Self {
            version: 1,
            origin: scope.origin,
            host_id: draft.host_id,
            mailbox_id: draft.mailbox_id,
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
            updated_at_ms: crate::clock::now_ms(),
        }
    }

    pub(crate) fn listed(&self) -> bool {
        self.dirty
            || self.pending_send.is_some()
            || self.latest_send.as_ref().is_none_or(|status| {
                status.state != MailDispatchState::Accepted
                    || status.draft_revision != self.server_revision
            })
    }
    pub fn scope(&self) -> Scope {
        Scope {
            origin: self.origin.clone(),
            host_id: self.host_id.clone(),
            mailbox_id: self.mailbox_id.clone(),
        }
    }
    pub fn validate(&self) -> Result<(), SaveError> {
        self.content.validate_storage().map_err(|_| {
            SaveError::Rejected(
                "This draft exceeds its text or file limits. Keep this tab open and shorten it.",
            )
        })?;
        if self.version != 1
            || self.origin.len() > 4096
            || self.host_id.is_empty()
            || self.host_id.len() > 1024
            || self.mailbox_id.is_empty()
            || self.mailbox_id.len() > 256
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
            return Err(SaveError::Corrupt);
        }
        let mut total = 0u64;
        let mut ids = std::collections::HashSet::new();
        for file in &self.files {
            if !valid_id(&file.id)
                || !ids.insert(&file.id)
                || file.file_name.len() > 1024
                || attachment_file_name(&file.file_name) != file.file_name
            {
                return Err(SaveError::Corrupt);
            }
            total = total
                .checked_add(file.size_bytes)
                .ok_or(SaveError::Corrupt)?;
        }
        if total > u64::try_from(MAIL_DRAFT_ATTACHMENT_MAX_BYTES).map_err(|_| SaveError::Corrupt)? {
            return Err(SaveError::Rejected(
                "Keep draft files within 16 MiB combined.",
            ));
        }
        Ok(())
    }
    fn index_key(&self) -> Result<String, SaveError> {
        let time = u64::try_from(self.updated_at_ms).map_err(|_| SaveError::Corrupt)?;
        Ok(format!(
            "{}{:020}:{}",
            self.scope().index_prefix()?,
            u64::MAX - time,
            self.client_id
        ))
    }
    pub(crate) fn row(&self) -> Result<LocalDraftRow, SaveError> {
        let recipients = [&self.content.to, &self.content.cc, &self.content.bcc]
            .into_iter()
            .find(|value| !value.is_empty())
            .map_or("", String::as_str);
        Ok(LocalDraftRow {
            key: self.index_key()?,
            client_id: self.client_id.clone(),
            subject: self.content.subject.chars().take(500).collect(),
            recipients: recipients.chars().take(500).collect(),
            updated_at_ms: self.updated_at_ms,
            dirty: self.dirty,
            pending_send: self
                .pending_send
                .as_ref()
                .map(|request| request.operation_id.clone()),
        })
    }
}

async fn read_in(
    tx: &idb::Transaction,
    scope: &Scope,
    client: &str,
) -> Result<Option<Metadata<LocalDraft>>, SaveError> {
    match metadata::read_in::<LocalDraft>(tx, &scope.body_key(client)?).await {
        Ok(record) => {
            record.value.validate()?;
            if !scope.matches(&record.value) || record.value.client_id != client {
                return Err(SaveError::Corrupt);
            }
            Ok(Some(record))
        }
        Err(SaveError::Unavailable) => Ok(None),
        Err(error) => Err(error),
    }
}

pub async fn load(scope: &Scope, client: &str) -> Result<Option<Metadata<LocalDraft>>, SaveError> {
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(&[STORE_METADATA], TransactionMode::ReadOnly)?);
    let result = read_in(&tx, scope, client).await;
    metadata::finish(tx, result).await
}

pub async fn list(scope: &Scope, after: Option<&str>) -> Result<Vec<LocalDraftRow>, SaveError> {
    let prefix = scope.index_prefix()?;
    if after.is_some_and(|key| !key.starts_with(&prefix)) {
        return Err(SaveError::Corrupt);
    }
    let lower = after.unwrap_or(&prefix);
    let upper = format!("{prefix}\u{ffff}");
    let range = idb::KeyRange::bound(
        &JsValue::from_str(lower),
        &JsValue::from_str(&upper),
        Some(after.is_some()),
        None,
    )?;
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(&[STORE_METADATA], TransactionMode::ReadOnly)?);
    let result = async {
        let rows = tx
            .object_store(STORE_METADATA)?
            .get_all(Some(range.into()), Some(50))?
            .await?;
        rows.into_iter()
            .map(|raw| {
                let row: LocalDraftRow = from_js(raw).map_err(|_| SaveError::Corrupt)?;
                if !row.key.starts_with(&prefix) || !valid_id(&row.client_id) {
                    return Err(SaveError::Corrupt);
                }
                Ok(row)
            })
            .collect()
    }
    .await;
    metadata::finish(tx, result).await
}

/// `added` is staged with its descriptor, never before/after a separate body save.
pub async fn save(
    value: &LocalDraft,
    expected: Option<u64>,
    added: &[(String, web_sys::Blob)],
) -> Result<Metadata<LocalDraft>, SaveError> {
    value.validate()?;
    let scope = value.scope();
    let key = scope.body_key(&value.client_id)?;
    for (id, blob) in added {
        let descriptor = value
            .files
            .iter()
            .find(|file| &file.id == id)
            .ok_or(SaveError::Corrupt)?;
        let size = u32::try_from(descriptor.size_bytes).map_err(|_| SaveError::Corrupt)?;
        if blob.size() != f64::from(size) {
            return Err(SaveError::Corrupt);
        }
    }
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(&[STORE_METADATA], TransactionMode::ReadWrite)?);
    let result = async {
        let current = read_in(&tx, &scope, &value.client_id).await?;
        let mut value = value.clone();
        if let Some(current) = &current {
            value.updated_at_ms = value.updated_at_ms.max(current.value.updated_at_ms);
        }
        if current.as_ref().map(|record| record.revision) != expected {
            if added.is_empty() {
                if let Some(current) = current {
                    if serde_json::to_value(&current.value).map_err(|_| SaveError::Corrupt)?
                        == serde_json::to_value(&value).map_err(|_| SaveError::Corrupt)?
                    {
                        return Ok(current);
                    }
                }
            }
            return Err(SaveError::Conflict);
        }
        let revision = expected
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(SaveError::Conflict)?;
        let record = Metadata {
            key: key.clone(),
            revision,
            value,
        };
        let encoded = metadata::encode(&record)?;
        let index = to_js(&record.value.row()?).map_err(|_| SaveError::Corrupt)?;
        let store = tx.object_store(STORE_METADATA)?;
        if let Some(current) = current {
            let previous_index = current.value.index_key()?;
            if previous_index != record.value.index_key()? {
                store.delete(JsValue::from_str(&previous_index))?.await?;
            }
            for file in current.value.files {
                if !record.value.files.iter().any(|next| next.id == file.id) {
                    store
                        .delete(JsValue::from_str(
                            &scope.file_key(&record.value.client_id, &file.id)?,
                        ))?
                        .await?;
                }
            }
        }
        for (id, blob) in added {
            let file = js_sys::Object::new();
            js_sys::Reflect::set(
                &file,
                &JsValue::from_str("key"),
                &JsValue::from_str(&scope.file_key(&record.value.client_id, id)?),
            )
            .map_err(|_| SaveError::Corrupt)?;
            js_sys::Reflect::set(&file, &JsValue::from_str("blob"), blob.as_ref())
                .map_err(|_| SaveError::Corrupt)?;
            store.put(&file, None)?.await?;
        }
        store.put(&encoded, None)?.await?;
        if record.value.listed() {
            store.put(&index, None)?.await?;
        } else {
            store
                .delete(JsValue::from_str(&record.value.index_key()?))?
                .await?;
        }
        Ok(record)
    }
    .await;
    metadata::finish(tx, result).await
}

pub async fn file(
    scope: &Scope,
    client: &str,
    id: &str,
) -> Result<Option<web_sys::Blob>, SaveError> {
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(&[STORE_METADATA], TransactionMode::ReadOnly)?);
    let result = async {
        let record = read_in(&tx, scope, client)
            .await?
            .ok_or(SaveError::Unavailable)?;
        let descriptor = record
            .value
            .files
            .iter()
            .find(|file| file.id == id)
            .ok_or(SaveError::Corrupt)?;
        let Some(raw) = tx
            .object_store(STORE_METADATA)?
            .get(JsValue::from_str(&scope.file_key(client, id)?))?
            .await?
        else {
            return Ok(None);
        };
        let blob = js_sys::Reflect::get(&raw, &JsValue::from_str("blob"))
            .map_err(|_| SaveError::Corrupt)?
            .dyn_into::<web_sys::Blob>()
            .map_err(|_| SaveError::Corrupt)?;
        if blob.size()
            != f64::from(u32::try_from(descriptor.size_bytes).map_err(|_| SaveError::Corrupt)?)
        {
            return Err(SaveError::Corrupt);
        }
        Ok(Some(blob))
    }
    .await;
    metadata::finish(tx, result).await
}
