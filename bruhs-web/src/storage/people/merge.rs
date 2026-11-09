//! A durable intent precedes the network; confirmed replies survive a failed local commit.

use bruhs_models::{merge_relationship_context, RelationshipProfile};
use serde::{Deserialize, Serialize};

use super::*;
use crate::relationship_store::ExpectedRelationships;
use crate::storage::{relationships, STORE_RELATIONSHIPS};
mod commit;
pub use commit::complete;
#[cfg(test)]
mod tests;

pub const KEY: &str = "bruhs.contactMerge";
const LOCK_KEY: &str = "bruhs.contactMergeLock";

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MergeLock {
    key: String,
    merge_id: String,
    source_id: String,
    target_id: String,
}
impl MergeLock {
    pub fn locks(&self, id: &str) -> bool {
        id == self.source_id || id == self.target_id
    }
}

pub async fn read_lock(tx: &idb::Transaction) -> Result<Option<MergeLock>, SaveError> {
    tx.object_store(STORE_METADATA)?
        .get(JsValue::from_str(LOCK_KEY))?
        .await?
        .map(|raw| {
            let lock: MergeLock = from_js(raw).map_err(|_| SaveError::Corrupt)?;
            if lock.key != LOCK_KEY
                || lock.merge_id.is_empty()
                || lock.source_id.is_empty()
                || lock.target_id.is_empty()
                || lock.source_id == lock.target_id
            {
                return Err(SaveError::Corrupt);
            }
            Ok(lock)
        })
        .transpose()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum Phase {
    Prepared,
    Confirmed { reply: Box<PersonProfile> },
    Completed { reply: Box<PersonProfile> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub key: String,
    pub id: String,
    pub source: PersonProfile,
    pub target: PersonProfile,
    pub before_source: Option<RelationshipProfile>,
    pub before_target: Option<RelationshipProfile>,
    pub after_relationship: Option<RelationshipProfile>,
    pub created_at_ms: i64,
    pub phase: Phase,
}

impl Record {
    fn lock(&self) -> MergeLock {
        MergeLock {
            key: LOCK_KEY.into(),
            merge_id: self.id.clone(),
            source_id: self.source.id.clone(),
            target_id: self.target.id.clone(),
        }
    }
    pub fn pending(&self) -> bool {
        !matches!(self.phase, Phase::Completed { .. })
    }
    pub fn locks(&self, id: &str) -> bool {
        self.pending() && (id == self.source.id || id == self.target.id)
    }
    fn validate(&self) -> Result<(), SaveError> {
        if self.key != KEY || self.id.is_empty() || self.source.id == self.target.id {
            return Err(SaveError::Corrupt);
        }
        let expected = merge_relationship_context(
            &self.source.id,
            &self.target.id,
            self.before_source.as_ref(),
            self.before_target.as_ref(),
            self.created_at_ms,
        )
        .map_err(|error| SaveError::Rejected(error.message()))?;
        if expected != self.after_relationship {
            return Err(SaveError::Corrupt);
        }
        if let Phase::Confirmed { reply } | Phase::Completed { reply } = &self.phase {
            if reply.id != self.target.id {
                return Err(SaveError::Corrupt);
            }
            if !replacement(&self.target, reply)? && reply.as_ref() != &self.target {
                return Err(SaveError::Corrupt);
            }
        }
        Ok(())
    }
}

pub async fn read_in(tx: &idb::Transaction) -> Result<Option<Record>, SaveError> {
    let raw = tx
        .object_store(STORE_METADATA)?
        .get(JsValue::from_str(KEY))?
        .await?;
    raw.map(|raw| {
        let record: Record = from_js(raw).map_err(|_| SaveError::Corrupt)?;
        record.validate()?;
        Ok(record)
    })
    .transpose()
}

pub async fn load() -> Result<Option<Record>, SaveError> {
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(&[STORE_METADATA], TransactionMode::ReadOnly)?);
    let result = async {
        let record = read_in(&tx).await?;
        let expected = record
            .as_ref()
            .filter(|record| record.pending())
            .map(Record::lock);
        if read_lock(&tx).await? != expected {
            return Err(SaveError::Corrupt);
        }
        Ok(record)
    }
    .await;
    finish(tx, result).await
}

pub async fn ensure_editable(tx: &idb::Transaction, id: &str) -> Result<(), SaveError> {
    // A stale tab with an empty original profile must not recreate an orphan after merge.
    if outbox::merged_ids(tx).await?.contains(id) {
        return Err(SaveError::Rejected(
            "This contact was merged. Reload People before editing its relationship.",
        ));
    }
    if read_lock(tx).await?.is_some_and(|lock| lock.locks(id)) {
        return Err(SaveError::Rejected("This relationship has a pending contact merge. Open People and retry that merge before editing it."));
    }
    Ok(())
}

async fn person_in(tx: &idb::Transaction, id: &str) -> Result<PersonProfile, SaveError> {
    let raw = tx
        .object_store(STORE_PEOPLE)?
        .get(JsValue::from_str(id))?
        .await?
        .ok_or(SaveError::Conflict)?;
    from_js(raw).map_err(|_| SaveError::Corrupt)
}

async fn check_baselines(tx: &idb::Transaction, record: &Record) -> Result<(), SaveError> {
    let (source, target, from, to) = futures_util::join!(
        person_in(tx, &record.source.id),
        person_in(tx, &record.target.id),
        relationships::read_in(tx, &record.source.id),
        relationships::read_in(tx, &record.target.id)
    );
    if source? != record.source
        || target? != record.target
        || from? != record.before_source
        || to? != record.before_target
    {
        return Err(SaveError::Conflict);
    }
    Ok(())
}

pub async fn prepare(
    source: PersonProfile,
    target: PersonProfile,
    expected: &ExpectedRelationships,
    now: i64,
) -> Result<Record, SaveError> {
    prepare_with_reply(source, target, expected, now, None).await
}

pub async fn prepare_remote(
    source: PersonProfile,
    target: PersonProfile,
    expected: &ExpectedRelationships,
    now: i64,
    reply: PersonProfile,
) -> Result<Record, SaveError> {
    prepare_with_reply(source, target, expected, now, Some(reply)).await
}

async fn prepare_with_reply(
    source: PersonProfile,
    target: PersonProfile,
    expected: &ExpectedRelationships,
    now: i64,
    reply: Option<PersonProfile>,
) -> Result<Record, SaveError> {
    let remote = reply.is_some();
    let before_source = expected
        .get(&source.id)
        .ok_or(SaveError::Unavailable)?
        .clone();
    let before_target = expected
        .get(&target.id)
        .ok_or(SaveError::Unavailable)?
        .clone();
    let after_relationship = merge_relationship_context(
        &source.id,
        &target.id,
        before_source.as_ref(),
        before_target.as_ref(),
        now,
    )
    .map_err(|error| SaveError::Rejected(error.message()))?;
    let record = Record {
        key: KEY.into(),
        id: uuid::Uuid::new_v4().to_string(),
        source,
        target,
        before_source,
        before_target,
        after_relationship,
        created_at_ms: now,
        phase: reply.map_or(Phase::Prepared, |reply| Phase::Confirmed {
            reply: Box::new(reply),
        }),
    };
    record.validate()?;
    let value = to_js(&record)?;
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(
        &[STORE_METADATA, STORE_PEOPLE, STORE_RELATIONSHIPS],
        TransactionMode::ReadWrite,
    )?);
    let result = async {
        if read_in(&tx).await?.is_some_and(|record| record.pending())
            || read_lock(&tx).await?.is_some()
        {
            return Err(SaveError::Rejected(
                "Finish the pending contact merge before starting another.",
            ));
        }
        if remote {
            outbox::ensure_remote_merge_ready(&tx, &record.source.id, &record.target.id).await?;
        } else {
            outbox::ensure_merge_ready(&tx, &record.source.id, &record.target.id).await?;
        }
        check_baselines(&tx, &record).await?;
        tx.object_store(STORE_METADATA)?.put(&value, None)?.await?;
        tx.object_store(STORE_METADATA)?
            .put(&to_js(&record.lock())?, None)?
            .await?;
        Ok(record)
    }
    .await;
    finish(tx, result).await
}

pub async fn confirm(expected: &Record, reply: PersonProfile) -> Result<Record, SaveError> {
    if reply.id != expected.target.id || !matches!(expected.phase, Phase::Prepared) {
        return Err(SaveError::Rejected(
            "Merge returned a different target identity. The original merge remains pending.",
        ));
    }
    if !replacement(&expected.target, &reply)? && reply != expected.target {
        return Err(SaveError::Rejected(
            "Merge reply is older than the saved target. The merge remains pending for recovery.",
        ));
    }
    let mut next = expected.clone();
    next.phase = Phase::Confirmed {
        reply: Box::new(reply),
    };
    let value = to_js(&next)?;
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(&[STORE_METADATA], TransactionMode::ReadWrite)?);
    let result = async {
        if read_in(&tx).await?.as_ref() != Some(expected) {
            return Err(SaveError::Conflict);
        }
        if read_lock(&tx).await? != Some(expected.lock()) {
            return Err(SaveError::Conflict);
        }
        tx.object_store(STORE_METADATA)?.put(&value, None)?.await?;
        Ok(next)
    }
    .await;
    finish(tx, result).await
}
