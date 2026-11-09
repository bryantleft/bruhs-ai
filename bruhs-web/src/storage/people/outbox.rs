use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::*;
use crate::storage::STORE_METADATA;

const KEY: &str = "bruhs.pendingContactCreates";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Outbox {
    key: String,
    ids: BTreeSet<String>,
    #[serde(default)]
    merged: BTreeSet<String>,
}

async fn read(tx: &idb::Transaction) -> Result<Outbox, SaveError> {
    match tx
        .object_store(STORE_METADATA)?
        .get(JsValue::from_str(KEY))?
        .await?
    {
        Some(raw) => from_js(raw).map_err(|_| SaveError::Corrupt),
        None => Ok(Outbox {
            key: KEY.into(),
            ids: BTreeSet::new(),
            merged: BTreeSet::new(),
        }),
    }
}

/// Stage the sync identity in the caller's contact/receipt transaction. Never
/// make a contact eligible for HTTP sync before its local capture commits.
pub(crate) async fn stage_capture(
    tx: &idb::Transaction,
    person: &PersonProfile,
) -> Result<(), SaveError> {
    if person.id.is_empty() || person.display_name.trim().is_empty() {
        return Err(SaveError::Rejected(
            "Contact ID and name are required. No contact was saved.",
        ));
    }
    let mut outbox = read(tx).await?;
    if outbox.merged.contains(&person.id) {
        return Err(SaveError::Conflict);
    }
    outbox.ids.insert(person.id.clone());
    tx.object_store(STORE_METADATA)?
        .put(&to_js(&outbox)?, None)?
        .await?;
    Ok(())
}

pub async fn pending() -> Result<Vec<PersonProfile>, SaveError> {
    let db = open_db().await?;
    let tx = LocalTransaction::new(
        db.transaction(&[STORE_PEOPLE, STORE_METADATA], TransactionMode::ReadOnly)?,
    );
    let result = async {
        let outbox = read(&tx).await?;
        let ids = outbox.ids.iter().collect::<Vec<_>>();
        let mut people = Vec::with_capacity(ids.len());
        for batch in ids.chunks(64) {
            let loaded = join_all(batch.iter().map(|id| async {
                let raw = tx
                    .object_store(STORE_PEOPLE)?
                    .get(JsValue::from_str(id))?
                    .await?
                    .ok_or(SaveError::Corrupt)?;
                from_js(raw).map_err(|_| SaveError::Corrupt)
            }))
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;
            people.extend(loaded);
        }
        Ok(people)
    }
    .await;
    finish(tx, result).await
}

pub(super) async fn merged_ids(tx: &idb::Transaction) -> Result<BTreeSet<String>, SaveError> {
    read(tx).await.map(|outbox| outbox.merged)
}

pub(super) async fn ensure_merge_ready(
    tx: &idb::Transaction,
    source: &str,
    target: &str,
) -> Result<(), SaveError> {
    let outbox = read(tx).await?;
    if [source, target]
        .iter()
        .any(|id| outbox.ids.contains(*id) || outbox.merged.contains(*id))
    {
        return Err(SaveError::Rejected("Finish syncing these contacts before merging them, and select contacts that have not already been merged."));
    }
    Ok(())
}

pub(super) async fn mark_merged_in(tx: &idb::Transaction, source: &str) -> Result<(), SaveError> {
    let mut outbox = read(tx).await?;
    outbox.ids.remove(source);
    outbox.merged.insert(source.into());
    tx.object_store(STORE_METADATA)?
        .put(&to_js(&outbox)?, None)?
        .await?;
    Ok(())
}

pub(super) async fn ensure_remote_merge_ready(
    tx: &idb::Transaction,
    source: &str,
    target: &str,
) -> Result<(), SaveError> {
    let outbox = read(tx).await?;
    if outbox.ids.contains(target)
        || outbox.merged.contains(source)
        || outbox.merged.contains(target)
    {
        return Err(SaveError::Rejected("Sync the surviving contact and reload merge recovery before reconciling this remote merge."));
    }
    Ok(())
}

pub async fn acknowledge(sent: &PersonProfile, reply: &PersonProfile) -> Result<(), SaveError> {
    if sent.id != reply.id {
        return Err(SaveError::Rejected(
            "Contact sync returned a different identity. The original contact remains pending.",
        ));
    }
    let db = open_db().await?;
    let tx = LocalTransaction::new(
        db.transaction(&[STORE_PEOPLE, STORE_METADATA], TransactionMode::ReadWrite)?,
    );
    let result = async {
        let mut outbox = read(&tx).await?;
        let raw = tx
            .object_store(STORE_PEOPLE)?
            .get(JsValue::from_str(&sent.id))?
            .await?
            .ok_or(SaveError::Conflict)?;
        let current: PersonProfile = from_js(raw).map_err(|_| SaveError::Corrupt)?;
        if current != *sent {
            return Err(SaveError::Rejected(
                "This contact changed during sync. The newer local contact remains pending.",
            ));
        }
        outbox.ids.remove(&sent.id);
        tx.object_store(STORE_METADATA)?
            .put(&to_js(&outbox)?, None)?
            .await?;
        Ok(())
    }
    .await;
    finish(tx, result).await
}
