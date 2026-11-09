use bruhs_core::{
    capture_client,
    meetings::{MeetingStoreError, MeetingStoreResult},
    AppState,
};
use bruhs_models::{CentroidsFile, MeetingEvent};
use std::{sync::Arc, time::Duration};

/// Why automatic speaker linking for a meeting did not finish this attempt.
#[derive(Debug, thiserror::Error)]
pub(super) enum LinkError {
    #[error("Capture centroids are temporarily unavailable.")]
    CentroidsUnavailable,
    #[error(transparent)]
    Store(#[from] MeetingStoreError),
}

pub(super) async fn link_centroids_for_meeting(
    app: Arc<AppState>,
    meeting_id: String,
) -> Result<(), LinkError> {
    let centroids = match tokio::time::timeout(
        Duration::from_secs(15),
        capture_client::list_centroids(&meeting_id),
    )
    .await
    {
        Ok(Ok(centroids)) => centroids,
        result => {
            tracing::warn!("centroids unavailable for {meeting_id}: {result:?}");
            return Err(LinkError::CentroidsUnavailable);
        }
    };
    if link_centroids(&app, &meeting_id, &centroids).await? {
        app.emit_meeting_event(MeetingEvent::SpeakersLinked { meeting_id });
    }
    Ok(())
}

pub(super) fn warn_failed(app: &AppState, meeting_id: String) {
    app.emit_meeting_event(MeetingEvent::CaptureWarning {
        meeting_id,
        message: "Automatic speaker linking will retry. Existing speaker choices are preserved."
            .into(),
    });
    app.emit_meeting_event(MeetingEvent::StreamOpened);
}

/// The store owns identity selection and attachment in one atomic commit.
pub(super) async fn link_centroids(
    app: &AppState,
    meeting_id: &str,
    centroids: &CentroidsFile,
) -> MeetingStoreResult<bool> {
    app.meetings()
        .link_speaker_centroids(meeting_id, centroids)
        .await
}

#[cfg(test)]
mod tests;
