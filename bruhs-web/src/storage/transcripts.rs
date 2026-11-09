//! Selected-meeting reads and compare-and-swap cache writes with terminal acknowledgment.
use super::metadata::{finish, LocalTransaction, SaveError};
use super::{from_js, open_db, to_js, Meeting, TranscriptSegment, STORE_MEETINGS, STORE_SEGMENTS};
use idb::{Transaction, TransactionMode};
use wasm_bindgen::JsValue;

pub(crate) mod stream;

#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub meeting: Option<Meeting>,
    pub segments: Vec<TranscriptSegment>,
}

async fn read_in(tx: &Transaction, id: &str) -> Result<Snapshot, SaveError> {
    let store = tx.object_store(STORE_SEGMENTS)?;
    let index = store.index("by_meeting")?;
    let query = Some(JsValue::from_str(id).into());
    let values = index.get_all(query.clone(), Some(100_001))?;
    let keys = index.get_all_keys(query, Some(100_001))?;
    let meeting = tx
        .object_store(STORE_MEETINGS)?
        .get(JsValue::from_str(id))?;
    let (values, keys, meeting) =
        futures_util::join!(async { values.await }, async { keys.await }, async {
            meeting.await
        });
    let (values, keys) = (values?, keys?);
    if values.len() != keys.len() {
        return Err(SaveError::Corrupt);
    }
    let meeting: Option<Meeting> = meeting?.map(from_js).transpose()?;
    if meeting.as_ref().is_some_and(|meeting| meeting.id != id) {
        return Err(SaveError::Corrupt);
    }
    let mut segments = Vec::with_capacity(values.len());
    for (key, value) in keys.into_iter().zip(values) {
        let segment: TranscriptSegment = from_js(value)?;
        if key.as_string().as_deref() != Some(segment.id.as_str()) {
            return Err(SaveError::Corrupt);
        }
        segments.push(segment);
    }
    bruhs_models::validate_transcript_snapshot(id, &[], &mut segments)
        .map_err(|error| SaveError::Rejected(error.message()))?;
    Ok(Snapshot { meeting, segments })
}

pub async fn load(id: &str) -> Result<Snapshot, SaveError> {
    let db = open_db().await?;
    let tx = LocalTransaction::new(
        db.transaction(&[STORE_MEETINGS, STORE_SEGMENTS], TransactionMode::ReadOnly)?,
    );
    let result = read_in(&tx, id).await;
    finish(tx, result).await
}

pub async fn cache_meeting(meeting: Meeting) -> Result<Meeting, SaveError> {
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(&[STORE_MEETINGS], TransactionMode::ReadWrite)?);
    let result = async {
        let store = tx.object_store(STORE_MEETINGS)?;
        if let Some(raw) = store.get(JsValue::from_str(&meeting.id))?.await? {
            let current: Meeting = from_js(raw)?;
            if current.id != meeting.id {
                return Err(SaveError::Corrupt);
            }
            if current.updated_at > meeting.updated_at {
                return Ok(current);
            }
            if current.updated_at == meeting.updated_at {
                return if current == meeting {
                    Ok(current)
                } else {
                    Err(SaveError::Conflict)
                };
            }
        }
        store.put(&to_js(&meeting)?, None)?.await?;
        Ok(meeting)
    }
    .await;
    finish(tx, result).await
}

pub async fn cache(
    id: &str,
    expected: &Snapshot,
    mut incoming: Snapshot,
) -> Result<Snapshot, SaveError> {
    bruhs_models::validate_transcript_snapshot(id, &expected.segments, &mut incoming.segments)
        .map_err(|error| SaveError::Rejected(error.message()))?;
    let meeting = incoming.meeting.as_ref().ok_or(SaveError::Unavailable)?;
    if meeting.id != id
        || expected.meeting.as_ref().is_some_and(|old| {
            meeting.updated_at < old.updated_at
                || (meeting.updated_at == old.updated_at && meeting != old)
        })
    {
        return Err(SaveError::Conflict);
    }
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(
        &[STORE_MEETINGS, STORE_SEGMENTS],
        TransactionMode::ReadWrite,
    )?);
    let result = async {
        if read_in(&tx, id).await? != *expected {
            return Err(SaveError::Conflict);
        }
        let store = tx.object_store(STORE_SEGMENTS)?;
        let new_ids = incoming
            .segments
            .iter()
            .map(|segment| segment.id.as_str())
            .collect::<std::collections::HashSet<_>>();
        for old in expected
            .segments
            .iter()
            .filter(|old| old.partial && !new_ids.contains(old.id.as_str()))
        {
            store.delete(JsValue::from_str(&old.id))?.await?;
        }
        // Foreign keys are checked inside the same transaction; this meeting
        // must never overwrite another meeting's segment, even on malformed input.
        for batch in incoming.segments.chunks(64) {
            let results = futures_util::future::join_all(batch.iter().map(|segment| async {
                if let Some(raw) = store.get(JsValue::from_str(&segment.id))?.await? {
                    let saved: TranscriptSegment = from_js(raw)?;
                    if saved.id != segment.id || saved.meeting_id != id {
                        return Err(SaveError::Conflict);
                    }
                    if saved == *segment {
                        return Ok(());
                    }
                }
                store.put(&to_js(segment)?, None)?.await?;
                Ok::<_, SaveError>(())
            }))
            .await;
            for result in results {
                result?;
            }
        }
        if expected.meeting.as_ref() != Some(meeting) {
            tx.object_store(STORE_MEETINGS)?
                .put(&to_js(meeting)?, None)?
                .await?;
        }
        Ok(())
    }
    .await;
    finish(tx, result).await?;
    Ok(incoming)
}

/// Delete one cached chunk when it belongs to `meeting_id`. Missing rows are success.
pub async fn forget_segment(meeting_id: &str, segment_id: &str) -> Result<bool, SaveError> {
    if meeting_id.is_empty() || segment_id.is_empty() {
        return Err(SaveError::Rejected("Transcript chunk identity is missing."));
    }
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(&[STORE_SEGMENTS], TransactionMode::ReadWrite)?);
    let result = async {
        let store = tx.object_store(STORE_SEGMENTS)?;
        let Some(raw) = store.get(JsValue::from_str(segment_id))?.await? else {
            return Ok(false);
        };
        let saved: TranscriptSegment = from_js(raw)?;
        if saved.id != segment_id || saved.meeting_id != meeting_id {
            return Err(SaveError::Conflict);
        }
        store.delete(JsValue::from_str(segment_id))?.await?;
        Ok(true)
    }
    .await;
    finish(tx, result).await
}

/// Write one chunk back after a delete the server refused.
pub async fn put_segment(segment: &TranscriptSegment) -> Result<(), SaveError> {
    if segment.id.is_empty() || segment.meeting_id.is_empty() {
        return Err(SaveError::Rejected("Transcript chunk identity is missing."));
    }
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(&[STORE_SEGMENTS], TransactionMode::ReadWrite)?);
    let result = async {
        let store = tx.object_store(STORE_SEGMENTS)?;
        if let Some(raw) = store.get(JsValue::from_str(&segment.id))?.await? {
            let saved: TranscriptSegment = from_js(raw)?;
            if saved.id != segment.id || saved.meeting_id != segment.meeting_id {
                return Err(SaveError::Conflict);
            }
        }
        store.put(&to_js(segment)?, None)?.await?;
        Ok(())
    }
    .await;
    finish(tx, result).await
}
