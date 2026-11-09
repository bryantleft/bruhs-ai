//! Compare original records and commit a logical task operation atomically.

use idb::TransactionMode;
use std::collections::BTreeSet;
use wasm_bindgen::JsValue;

use super::metadata::{finish, LocalTransaction, SaveError};
use super::{from_js, open_db, to_js, STORE_WORK_ITEMS};
use crate::state::WorkItem;
use crate::task_saves::{Kind, Record};
mod projects;
mod sources;
mod threads;

fn store_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Task => STORE_WORK_ITEMS,
        Kind::Capture => super::STORE_CAPTURES,
        Kind::Inbox => super::STORE_INBOX_CONVERSATIONS,
        Kind::Event => super::STORE_EVENTS,
        Kind::Document => super::STORE_KNOWLEDGE_DOCUMENTS,
        Kind::Attachment => super::STORE_KNOWLEDGE_ATTACHMENTS,
        Kind::SourceClip => super::STORE_SOURCE_CLIPS,
        Kind::Person => super::STORE_PEOPLE,
        Kind::Project => super::STORE_PROJECTS,
        Kind::Thread => super::STORE_THREADS,
        Kind::Conversation => super::STORE_MESSAGES,
    }
}

/// Reject an invalid snapshot rather than silently hiding damaged tasks. The
/// original storage is never changed by loading or retrying a load.
pub async fn load() -> Result<Vec<WorkItem>, SaveError> {
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(&[STORE_WORK_ITEMS], TransactionMode::ReadOnly)?);
    let result = async {
        tx.object_store(STORE_WORK_ITEMS)?
            .get_all(None, None)?
            .await?
            .into_iter()
            .map(|raw| from_js(raw).map_err(|_| SaveError::Corrupt))
            .collect()
    }
    .await;
    finish(tx, result).await
}

pub struct Write<'a> {
    pub kind: Kind,
    pub id: &'a str,
    pub expected: Option<&'a Record>,
    pub draft: Option<&'a Record>,
}

#[derive(serde::Deserialize)]
struct AttachmentOwner {
    id: String,
    document_id: String,
}

pub async fn save(writes: &[Write<'_>]) -> Result<(), SaveError> {
    let source_guard = sources::SourceGuard::new(writes);
    let project_guard = projects::ProjectGuard::new(writes);
    let mut identities = BTreeSet::new();
    for write in writes {
        if write.id.is_empty()
            || !identities.insert((write.kind, write.id))
            || write
                .expected
                .into_iter()
                .chain(write.draft)
                .any(|item| item.id() != write.id || item.kind() != write.kind)
        {
            return Err(SaveError::Rejected(
                "Record identity changed. No replacement was saved.",
            ));
        }
    }
    let values = writes
        .iter()
        .map(|write| {
            write
                .draft
                .map(|record| match record {
                    Record::Attachment(attachment) => super::attachment_to_js(attachment.as_ref()),
                    _ => to_js(record),
                })
                .transpose()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let db = open_db().await?;
    let stores = writes
        .iter()
        .flat_map(|write| {
            let cascade = write.kind == Kind::Document && write.draft.is_none();
            std::iter::once(store_name(write.kind))
                .chain(cascade.then_some(super::STORE_KNOWLEDGE_ATTACHMENTS))
                .chain((write.kind == Kind::Person).then_some(super::STORE_METADATA))
                .chain(
                    matches!(write.kind, Kind::Thread | Kind::Conversation)
                        .then_some(super::STORE_THREADS),
                )
                .chain(
                    matches!(write.kind, Kind::Thread | Kind::Conversation)
                        .then_some(super::STORE_MESSAGES),
                )
        })
        .chain(source_guard.stores())
        .chain(project_guard.stores())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let tx = LocalTransaction::new(db.transaction(&stores, TransactionMode::ReadWrite)?);
    let result = async {
        for (write, value) in writes.iter().zip(values) {
            let store = tx.object_store(store_name(write.kind))?;
            let current = store
                .get(JsValue::from_str(write.id))?
                .await?
                .map(|raw| match write.kind {
                    Kind::Task => from_js(raw)
                        .map(Record::Task)
                        .map_err(|_| SaveError::Corrupt),
                    Kind::Capture => from_js(raw)
                        .map(Record::Capture)
                        .map_err(|_| SaveError::Corrupt),
                    Kind::Inbox => from_js(raw)
                        .map(Record::Inbox)
                        .map_err(|_| SaveError::Corrupt),
                    Kind::Event => from_js(raw)
                        .map(Record::Event)
                        .map_err(|_| SaveError::Corrupt),
                    Kind::Document => from_js(raw)
                        .map(Record::Document)
                        .map_err(|_| SaveError::Corrupt),
                    Kind::Attachment => from_js(raw)
                        .map(|record| Record::Attachment(std::sync::Arc::new(record)))
                        .map_err(|_| SaveError::Corrupt),
                    Kind::SourceClip => from_js(raw)
                        .map(Record::SourceClip)
                        .map_err(|_| SaveError::Corrupt),
                    Kind::Person => from_js(raw)
                        .map(Record::Person)
                        .map_err(|_| SaveError::Corrupt),
                    Kind::Project => from_js(raw)
                        .map(Record::Project)
                        .map_err(|_| SaveError::Corrupt),
                    Kind::Thread => from_js(raw)
                        .map(Record::Thread)
                        .map_err(|_| SaveError::Corrupt),
                    Kind::Conversation => from_js(raw)
                        .map(Record::Conversation)
                        .map_err(|_| SaveError::Corrupt),
                })
                .transpose()?;
            if current.as_ref() != write.expected {
                return Err(SaveError::Conflict);
            }
            if let Some(Record::Person(person)) = write.draft {
                super::people::outbox::stage_capture(&tx, person).await?;
            }
            if let Some(value) = value {
                store.put(&value, None)?.await?;
            } else {
                store.delete(JsValue::from_str(write.id))?.await?;
            }
        }
        // Metadata is the authoritative set of immutable attachment IDs. Do
        // not load every other document's potentially large byte payloads to
        // delete this document. Pending uploads above become tombstones too.
        for write in writes {
            if let (Some(Record::Document(original)), None) = (write.expected, write.draft) {
                let store = tx.object_store(super::STORE_KNOWLEDGE_ATTACHMENTS)?;
                for attachment in &original.attachments {
                    if let Some(raw) = store.get(JsValue::from_str(&attachment.id))?.await? {
                        let owner: AttachmentOwner =
                            from_js(raw).map_err(|_| SaveError::Corrupt)?;
                        if owner.id != attachment.id || owner.document_id != original.id {
                            return Err(SaveError::Conflict);
                        }
                    }
                    store.delete(JsValue::from_str(&attachment.id))?.await?;
                }
            }
        }
        source_guard.validate(&tx).await?;
        project_guard.validate(&tx).await?;
        threads::validate_and_cascade(&tx, writes).await
    }
    .await;
    finish(tx, result).await
}
