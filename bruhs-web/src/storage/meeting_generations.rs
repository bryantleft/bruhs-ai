//! Durable, per-meeting generation intents. No provider request is sent by this module.
use super::metadata::{finish, LocalTransaction, SaveError};
use super::{from_js, open_db, to_js, STORE_MEETING_GENERATIONS};
use bruhs_models::{SummaryGenerationReceipt, SummaryGenerationRequest, SummaryGenerationState};
use idb::{Transaction, TransactionMode};
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsValue;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    pub id: String,
    pub request: SummaryGenerationRequest,
    pub minimum_version: i64,
}

impl Checkpoint {
    pub fn validate(&self, id: &str) -> Result<(), SaveError> {
        if self.id != id
            || self.request.meeting_id != id
            || !self.request.valid()
            || uuid::Uuid::parse_str(&self.request.request_id).is_err()
            || self.minimum_version < 0
            || (self.request.replace_observations && !self.request.extract_observations)
        {
            return Err(SaveError::Corrupt);
        }
        Ok(())
    }

    pub fn confirm_terminal(&self, receipt: &SummaryGenerationReceipt) -> Result<(), SaveError> {
        self.validate(&self.id)?;
        if self.request != receipt.request {
            return Err(SaveError::Rejected("The server returned a mismatched generation receipt. The saved request is retained."));
        }
        if matches!(receipt.state, SummaryGenerationState::Pending) {
            return Err(SaveError::Rejected("This generation remains unresolved on the server. Retry loading later; no new generation will be sent."));
        }
        Ok(())
    }
}

async fn read_in(tx: &Transaction, id: &str) -> Result<Option<Checkpoint>, SaveError> {
    let value = tx
        .object_store(STORE_MEETING_GENERATIONS)?
        .get(JsValue::from_str(id))?
        .await?;
    let checkpoint: Option<Checkpoint> = value.map(from_js).transpose()?;
    if let Some(checkpoint) = &checkpoint {
        checkpoint.validate(id)?;
    }
    Ok(checkpoint)
}

pub async fn load(id: &str) -> Result<Option<Checkpoint>, SaveError> {
    let db = open_db().await?;
    let tx = LocalTransaction::new(
        db.transaction(&[STORE_MEETING_GENERATIONS], TransactionMode::ReadOnly)?,
    );
    let result = read_in(&tx, id).await;
    finish(tx, result).await
}

pub async fn load_all() -> Result<Vec<Checkpoint>, SaveError> {
    let db = open_db().await?;
    let tx = LocalTransaction::new(
        db.transaction(&[STORE_MEETING_GENERATIONS], TransactionMode::ReadOnly)?,
    );
    let result = async {
        let store = tx.object_store(STORE_MEETING_GENERATIONS)?;
        let values = store.get_all(None, Some(10_001))?;
        let keys = store.get_all_keys(None, Some(10_001))?;
        let (values, keys) = futures_util::join!(async { values.await }, async { keys.await });
        let (values, keys) = (values?, keys?);
        if values.len() != keys.len() || values.len() > 10_000 {
            return Err(SaveError::Corrupt);
        }
        values
            .into_iter()
            .zip(keys)
            .map(|(value, key)| {
                let checkpoint: Checkpoint = from_js(value)?;
                checkpoint.validate(&key.as_string().ok_or(SaveError::Corrupt)?)?;
                Ok(checkpoint)
            })
            .collect()
    }
    .await;
    finish(tx, result).await
}

pub enum Reservation {
    Started,
    Existing(Checkpoint),
}

/// Existing intents never authorize a second POST, even when the same tab retries.
pub async fn begin(checkpoint: &Checkpoint) -> Result<Reservation, SaveError> {
    checkpoint.validate(&checkpoint.id)?;
    let encoded = to_js(checkpoint)?;
    let db = open_db().await?;
    let tx = LocalTransaction::new(
        db.transaction(&[STORE_MEETING_GENERATIONS], TransactionMode::ReadWrite)?,
    );
    let result = async {
        if let Some(current) = read_in(&tx, &checkpoint.id).await? {
            return Ok(Reservation::Existing(current));
        }
        tx.object_store(STORE_MEETING_GENERATIONS)?
            .add(&encoded, None)?
            .await?;
        Ok(Reservation::Started)
    }
    .await;
    finish(tx, result).await
}

/// Call only after a successful POST or terminal receipt and acknowledged meeting caching,
/// or after a preflight rejection that provably never sent this request.
pub async fn clear(checkpoint: &Checkpoint) -> Result<(), SaveError> {
    checkpoint.validate(&checkpoint.id)?;
    let db = open_db().await?;
    let tx = LocalTransaction::new(
        db.transaction(&[STORE_MEETING_GENERATIONS], TransactionMode::ReadWrite)?,
    );
    let result = async {
        let current = read_in(&tx, &checkpoint.id).await?;
        if current
            .as_ref()
            .is_some_and(|current| current != checkpoint)
        {
            return Err(SaveError::Conflict);
        }
        tx.object_store(STORE_MEETING_GENERATIONS)?
            .delete(JsValue::from_str(&checkpoint.id))?
            .await?;
        Ok(())
    }
    .await;
    finish(tx, result).await
}

#[cfg(test)]
mod tests;
