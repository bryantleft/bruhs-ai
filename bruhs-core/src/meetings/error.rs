//! `MeetingStoreError` and the `MeetingStoreResult<T>` convenience alias.

use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum MeetingStoreError {
    #[error("meeting not found: {id}")]
    MeetingNotFound { id: String },

    #[error("person not found: {id}")]
    PersonNotFound { id: String },

    #[error("speaker not found: meeting={meeting_id}, speaker={speaker_id}")]
    SpeakerNotFound {
        meeting_id: String,
        speaker_id: String,
    },

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("persistence failed: {0}")]
    PersistenceFailed(String),

    /// A SQLite operation failed. The message names only the result code:
    /// the underlying error can quote SQL or record contents, so it is kept
    /// as the [`std::error::Error::source`] for diagnostics, never shown.
    #[error("Local database operation failed ({code:?}); no success was acknowledged.")]
    Database {
        code: Option<rusqlite::ErrorCode>,
        #[source]
        source: rusqlite::Error,
    },

    /// A saved record could not be decoded; the decode error is the source.
    #[error("Saved record is malformed. Original data was preserved.")]
    Corrupt(#[source] serde_json::Error),
}

pub type MeetingStoreResult<T> = Result<T, MeetingStoreError>;
