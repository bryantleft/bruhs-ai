//! Durable completion hints, separate from transient SSE and audio callbacks.
//! All SQLite work runs off the async executor; receipt tokens fence stale acks.
use bruhs_models::{CaptureRecoveryReceipt, CAPTURE_RECOVERY_PAGE_SIZE};
use rusqlite::Connection;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

mod failures;
mod schema;

/// Publish only after final transcript persistence. If saving the recovery hint
/// fails, live consumers still receive completion plus an explicit warning.
pub async fn announce_speakers_assigned(state: &crate::state::CaptureState, meeting_id: String) {
    if let Err(error) = state.recovery.enqueue(meeting_id.clone()).await {
        tracing::warn!("capture completion recovery could not be saved: {error}");
        state.emit(bruhs_models::MeetingEvent::CaptureWarning {
            meeting_id: meeting_id.clone(),
            message: "Speaker recovery could not be saved. Keep this recording and reload it to check its speakers.".into(),
        });
    }
    state.emit(bruhs_models::MeetingEvent::SpeakersAssigned { meeting_id });
}

#[derive(Debug, thiserror::Error)]
pub enum RecoveryError {
    #[error("Recovery storage is unavailable")]
    Io(#[from] std::io::Error),
    #[error("Recovery database operation failed")]
    Database(#[from] rusqlite::Error),
    #[error("Recovery worker stopped")]
    Worker(#[from] tokio::task::JoinError),
    #[error("Invalid recovery request")]
    Invalid,
    #[error("Unrecognized recovery database; original file retained")]
    UnknownFormat,
}

pub struct RecoveryQueue {
    path: Option<PathBuf>,
    connection: Arc<Mutex<Option<Connection>>>,
}

impl RecoveryQueue {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            path,
            connection: Arc::new(Mutex::new(None)),
        }
    }

    async fn call<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Connection) -> Result<T, RecoveryError> + Send + 'static,
    ) -> Result<T, RecoveryError> {
        let path = self.path.clone().ok_or(RecoveryError::Invalid)?;
        let mut slot = self.connection.clone().lock_owned().await;
        tokio::task::spawn_blocking(move || {
            if slot.is_none() {
                *slot = Some(schema::open(&path)?);
            }
            operation(slot.as_mut().ok_or(RecoveryError::Invalid)?)
        })
        .await?
    }

    pub async fn enqueue(&self, meeting_id: String) -> Result<(), RecoveryError> {
        crate::paths::validate_meeting_id(&meeting_id)?;
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO recovery(meeting_id,token) VALUES(?1,?2)
                 ON CONFLICT(meeting_id) DO UPDATE SET token=excluded.token",
                (&meeting_id, uuid::Uuid::new_v4().to_string()),
            )?;
            Ok(())
        })
        .await
    }

    pub async fn page(&self, after: i64) -> Result<Vec<CaptureRecoveryReceipt>, RecoveryError> {
        if after < 0 {
            return Err(RecoveryError::Invalid);
        }
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "SELECT sequence,token,meeting_id FROM recovery WHERE sequence>?1 ORDER BY sequence LIMIT ?2"
            )?;
            let rows = statement.query_map((after, i64::try_from(CAPTURE_RECOVERY_PAGE_SIZE).unwrap_or(i64::MAX)), |row| {
                Ok(CaptureRecoveryReceipt { sequence: row.get(0)?, token: row.get(1)?, meeting_id: row.get(2)? })
            })?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        }).await
    }

    pub async fn acknowledge(
        &self,
        receipts: Vec<CaptureRecoveryReceipt>,
    ) -> Result<(), RecoveryError> {
        if receipts.len() > CAPTURE_RECOVERY_PAGE_SIZE
            || receipts.iter().any(|receipt| {
                receipt.sequence <= 0
                    || uuid::Uuid::parse_str(&receipt.token).is_err()
                    || crate::paths::validate_meeting_id(&receipt.meeting_id).is_err()
            })
        {
            return Err(RecoveryError::Invalid);
        }
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            for receipt in receipts {
                transaction.execute(
                    "DELETE FROM recovery WHERE sequence=?1 AND token=?2 AND meeting_id=?3",
                    (receipt.sequence, receipt.token, receipt.meeting_id),
                )?;
            }
            transaction.commit()?;
            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests;
