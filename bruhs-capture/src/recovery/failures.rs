use super::{schema, RecoveryError, RecoveryQueue};
use bruhs_models::{CaptureFailureReceipt, CAPTURE_RECOVERY_PAGE_SIZE};

fn receipt(
    meeting_id: &str,
    started_at_ms: f64,
    failed_at_ms: f64,
) -> Result<CaptureFailureReceipt, RecoveryError> {
    let receipt = CaptureFailureReceipt {
        sequence: 1,
        token: uuid::Uuid::new_v4().to_string(),
        meeting_id: meeting_id.into(),
        started_at_ms,
        failed_at_ms,
    };
    if !receipt.is_valid() {
        return Err(RecoveryError::Invalid);
    }
    Ok(receipt)
}

fn insert(
    connection: &rusqlite::Connection,
    receipt: &CaptureFailureReceipt,
) -> Result<(), RecoveryError> {
    connection.execute(
        "INSERT INTO capture_failures(meeting_id,token,started_at_ms,failed_at_ms) VALUES(?1,?2,?3,?4)
         ON CONFLICT(meeting_id,started_at_ms) DO NOTHING",
        (&receipt.meeting_id, &receipt.token, receipt.started_at_ms, receipt.failed_at_ms),
    )?;
    Ok(())
}

impl RecoveryQueue {
    /// Persist processing failures off the async executor, before publishing them.
    pub async fn record_failure(
        &self,
        meeting_id: &str,
        started_at_ms: f64,
        failed_at_ms: f64,
    ) -> Result<(), RecoveryError> {
        let receipt = receipt(meeting_id, started_at_ms, failed_at_ms)?;
        self.call(move |connection| insert(connection, &receipt))
            .await
    }

    /// Only call from the lifecycle's blocking Stop worker, never an audio callback.
    pub fn record_failure_blocking(
        &self,
        meeting_id: &str,
        started_at_ms: f64,
        failed_at_ms: f64,
    ) -> Result<(), RecoveryError> {
        let receipt = receipt(meeting_id, started_at_ms, failed_at_ms)?;
        let path = self.path.as_ref().ok_or(RecoveryError::Invalid)?;
        let mut slot = self.connection.blocking_lock();
        if slot.is_none() {
            *slot = Some(schema::open(path)?);
        }
        insert(slot.as_ref().ok_or(RecoveryError::Invalid)?, &receipt)
    }

    pub async fn failure_page(
        &self,
        after: i64,
    ) -> Result<Vec<CaptureFailureReceipt>, RecoveryError> {
        if after < 0 {
            return Err(RecoveryError::Invalid);
        }
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "SELECT sequence,token,meeting_id,started_at_ms,failed_at_ms FROM capture_failures
                 WHERE sequence>?1 ORDER BY sequence LIMIT ?2",
            )?;
            let rows = statement.query_map(
                (
                    after,
                    i64::try_from(CAPTURE_RECOVERY_PAGE_SIZE).unwrap_or(i64::MAX),
                ),
                |row| {
                    Ok(CaptureFailureReceipt {
                        sequence: row.get(0)?,
                        token: row.get(1)?,
                        meeting_id: row.get(2)?,
                        started_at_ms: row.get(3)?,
                        failed_at_ms: row.get(4)?,
                    })
                },
            )?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .await
    }

    pub async fn acknowledge_failures(
        &self,
        receipts: Vec<CaptureFailureReceipt>,
    ) -> Result<(), RecoveryError> {
        if receipts.len() > CAPTURE_RECOVERY_PAGE_SIZE || receipts.iter().any(|r| !r.is_valid()) {
            return Err(RecoveryError::Invalid);
        }
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            for r in receipts {
                transaction.execute(
                    "DELETE FROM capture_failures WHERE sequence=?1 AND token=?2 AND meeting_id=?3 AND started_at_ms=?4 AND failed_at_ms=?5",
                    (r.sequence, r.token, r.meeting_id, r.started_at_ms, r.failed_at_ms),
                )?;
            }
            transaction.commit()?;
            Ok(())
        }).await
    }
}

#[cfg(test)]
mod tests;
