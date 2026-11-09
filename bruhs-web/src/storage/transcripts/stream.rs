//! Stream writes never roll back finalized words or canonical speaker annotations.
use super::*;
use bruhs_models::TranscriptValidationError;

pub(crate) fn should_write(
    saved: Option<&TranscriptSegment>,
    incoming: &TranscriptSegment,
) -> Result<bool, &'static str> {
    bruhs_models::validate_transcript_records(&incoming.meeting_id, std::slice::from_ref(incoming))
        .map_err(TranscriptValidationError::message)?;
    let Some(saved) = saved else { return Ok(true) };
    bruhs_models::validate_transcript_records(&saved.meeting_id, std::slice::from_ref(saved))
        .map_err(TranscriptValidationError::message)?;
    if saved.id != incoming.id || saved.meeting_id != incoming.meeting_id {
        return Err("A stream segment cannot replace another recording's speech.");
    }
    if !saved.partial {
        if incoming.partial {
            return Ok(false);
        }
        if incoming.text != saved.text
            || incoming.start_ms != saved.start_ms
            || incoming.end_ms != saved.end_ms
            || incoming.confidence != saved.confidence
        {
            return Err("A stream replay cannot rewrite finalized speech.");
        }
        // Only an authoritative snapshot may revise finalized annotations.
        return Ok(false);
    }
    Ok(saved != incoming)
}

pub async fn cache_stream(batch: &[std::sync::Arc<TranscriptSegment>]) -> Result<(), SaveError> {
    if batch.len() > 256 {
        return Err(SaveError::Rejected("Live transcript batch limit exceeded."));
    }
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(&[STORE_SEGMENTS], TransactionMode::ReadWrite)?);
    let result = async {
        let store = tx.object_store(STORE_SEGMENTS)?;
        // Requests share one terminally acknowledged transaction, with bounded fan-out.
        for group in batch.chunks(64) {
            let results = futures_util::future::join_all(group.iter().map(|segment| async {
                let saved: Option<TranscriptSegment> = store
                    .get(JsValue::from_str(&segment.id))?
                    .await?
                    .map(from_js)
                    .transpose()?;
                if should_write(saved.as_ref(), segment).map_err(SaveError::Rejected)? {
                    store.put(&to_js(segment.as_ref())?, None)?.await?;
                }
                Ok::<_, SaveError>(())
            }))
            .await;
            for result in results {
                result?;
            }
        }
        Ok(())
    }
    .await;
    finish(tx, result).await
}
