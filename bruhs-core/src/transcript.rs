//! A meeting's transcript as the server serves it: the capture daemon's
//! current copy, committed to the store before anyone sees it, or the saved
//! copy when capture is unavailable or its reply is rejected.

use crate::meetings::MeetingStoreError;
use crate::store::AppState;
use bruhs_models::ids::MeetingId;
use bruhs_models::TranscriptSegment;

/// Where the returned segments came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptSource {
    /// Capture's reply, now committed to the store.
    Capture,
    /// The store's copy.
    Saved,
}

impl TranscriptSource {
    /// Wire value of the `x-bruhs-transcript-source` header.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Capture => "capture",
            Self::Saved => "saved",
        }
    }
}

/// What happened to the refresh from capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptRefresh {
    /// Capture answered and its copy was committed.
    Current,
    /// Capture did not answer.
    Unavailable,
    /// Capture answered but its copy was not committed (stale, or it would
    /// have dropped or rewritten saved speech).
    Rejected,
}

impl TranscriptRefresh {
    /// Wire value of the `x-bruhs-transcript-refresh` header.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Unavailable => "unavailable",
            Self::Rejected => "rejected",
        }
    }
}

/// The transcript and how fresh it is.
#[derive(Debug, Clone, PartialEq)]
pub struct RefreshedTranscript {
    pub segments: Vec<TranscriptSegment>,
    pub source: TranscriptSource,
    pub refresh: TranscriptRefresh,
}

/// Why no transcript can be served.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TranscriptError {
    #[error("Meeting no longer exists.")]
    MeetingNotFound,
    /// Capture did not answer and nothing is saved. An empty list would
    /// claim the meeting had no speech.
    #[error("Capture transcript is unavailable and no saved transcript exists. Retry when capture is connected.")]
    Unavailable(#[source] crate::CaptureClientError),
    /// Reading the store failed, or capture's copy was rejected and nothing
    /// is saved to fall back on.
    #[error(transparent)]
    Store(#[from] MeetingStoreError),
}

impl AppState {
    /// The transcript of `meeting_id`, refreshed from capture first.
    /// Capture's copy is validated and committed before it is returned, so
    /// a reply the store refuses is never published; the saved copy is
    /// served instead, and a failed refresh never reads as an empty meeting.
    pub async fn refreshed_transcript(
        &self,
        meeting_id: &MeetingId,
    ) -> Result<RefreshedTranscript, TranscriptError> {
        let id = meeting_id.as_str();
        let store = self.meetings();
        if store.get_meeting(id).await?.is_none() {
            return Err(TranscriptError::MeetingNotFound);
        }
        let saved = store.list_segments(id).await?;
        let incoming = match crate::capture_client::list_segments(id).await {
            Ok(incoming) => incoming,
            Err(error) if saved.is_empty() => return Err(TranscriptError::Unavailable(error)),
            Err(error) => {
                tracing::debug!("capture transcript unavailable for {id}: {error}");
                return Ok(RefreshedTranscript {
                    segments: saved,
                    source: TranscriptSource::Saved,
                    refresh: TranscriptRefresh::Unavailable,
                });
            }
        };
        match store.cache_transcript(id, saved, incoming).await {
            Ok(segments) => Ok(RefreshedTranscript {
                segments,
                source: TranscriptSource::Capture,
                refresh: TranscriptRefresh::Current,
            }),
            Err(error) => {
                // A competing fetch may have committed a newer snapshot. Never
                // publish the uncommitted reply or mistake a failed refresh
                // for empty.
                let current = store.list_segments(id).await?;
                if current.is_empty() {
                    return Err(error.into());
                }
                tracing::debug!("capture transcript rejected for {id}: {error}");
                Ok(RefreshedTranscript {
                    segments: current,
                    source: TranscriptSource::Saved,
                    refresh: TranscriptRefresh::Rejected,
                })
            }
        }
    }
}
