//! Required processing stages fail closed even when their blocking worker panics
//! or is cancelled. Never forward a panic payload into a meeting event.
use crate::state::CaptureState;
use std::fmt::Display;

#[derive(Debug, thiserror::Error)]
pub(super) enum WorkerFailure<E> {
    #[error("{0}")]
    Operation(E),
    #[error("processing worker was cancelled; recording files were retained")]
    Cancelled,
    #[error("processing worker stopped unexpectedly; recording files were retained")]
    Panicked,
}

pub(super) async fn finish_required<T, E: Display>(
    state: &CaptureState,
    meeting_id: &str,
    started_at_ms: f64,
    stage: &'static str,
    joined: Result<Result<T, E>, tokio::task::JoinError>,
) -> Result<T, WorkerFailure<E>> {
    let result = match joined {
        Ok(result) => result.map_err(WorkerFailure::Operation),
        Err(error) if error.is_cancelled() => Err(WorkerFailure::Cancelled),
        Err(_) => Err(WorkerFailure::Panicked),
    };
    if let Err(error) = &result {
        tracing::warn!(stage, "required capture processing failed: {error}");
        fail(
            state,
            meeting_id,
            started_at_ms,
            format!("{stage} failed: {error}"),
        )
        .await;
    }
    result
}

pub(super) async fn fail(
    state: &CaptureState,
    meeting_id: &str,
    started_at_ms: f64,
    message: String,
) {
    let failed_at_ms = bruhs_models::EpochMs::now().as_f64().max(started_at_ms);
    if let Err(error) = state
        .recovery
        .record_failure(meeting_id, started_at_ms, failed_at_ms)
        .await
    {
        tracing::warn!("processing failure recovery could not be saved: {error}");
        state.emit(bruhs_models::MeetingEvent::CaptureWarning {
            meeting_id: meeting_id.into(),
            message: "Meeting processing failed and its recovery status could not be saved. Keep the recording files and check this meeting before closing the app.".into(),
        });
    }
    state.emit(bruhs_models::MeetingEvent::CaptureError {
        meeting_id: meeting_id.into(),
        message,
    });
}

pub(super) async fn finish_final<E: Display>(
    state: &CaptureState,
    meeting_id: &str,
    started_at_ms: f64,
    joined: Result<Result<(), E>, tokio::task::JoinError>,
) {
    if finish_required(state, meeting_id, started_at_ms, "final transcript", joined)
        .await
        .is_ok()
    {
        crate::recovery::announce_speakers_assigned(state, meeting_id.into()).await;
        state.emit(bruhs_models::MeetingEvent::TranscriptionCompleted {
            meeting_id: meeting_id.into(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::MeetingEvent;

    fn isolated_state() -> (tempfile::TempDir, CaptureState) {
        let dir = tempfile::tempdir().unwrap();
        let mut state = CaptureState::new(crate::paths::DataRoot::at(dir.path()));
        state.recovery =
            crate::recovery::RecoveryQueue::new(Some(dir.path().join("recovery.sqlite3")));
        (dir, state)
    }

    #[tokio::test]
    async fn successful_worker_preserves_value_without_failure_event() {
        let (_dir, state) = isolated_state();
        let mut receiver = state.subscribe();
        let joined = tokio::task::spawn_blocking(|| Ok::<_, std::io::Error>(42)).await;
        assert_eq!(
            finish_required(&state, "synthetic", 10.0, "mix", joined)
                .await
                .unwrap(),
            42
        );
        assert!(receiver.try_recv().is_err());
    }

    #[tokio::test]
    async fn operation_failure_is_returned_and_published_once() {
        let (_dir, state) = isolated_state();
        let mut receiver = state.subscribe();
        let joined = tokio::task::spawn_blocking(|| {
            Err::<(), _>(std::io::Error::other("synthetic disk error"))
        })
        .await;
        assert!(matches!(
            finish_required(&state, "synthetic", 10.0, "mix", joined).await,
            Err(WorkerFailure::Operation(_))
        ));
        assert!(
            matches!(receiver.recv().await.unwrap(), MeetingEvent::CaptureError { meeting_id, message }
            if meeting_id == "synthetic" && message == "mix failed: synthetic disk error")
        );
        assert!(receiver.try_recv().is_err());
        assert_eq!(state.recovery.failure_page(0).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn panic_stops_each_required_stage_without_exposing_payload() {
        for stage in ["mix", "transcription", "final transcript"] {
            let (_dir, state) = isolated_state();
            let mut receiver = state.subscribe();
            let joined = tokio::task::spawn_blocking(|| -> Result<(), std::io::Error> {
                panic!("synthetic-private-panic-payload")
            })
            .await;
            assert!(matches!(
                finish_required(&state, "synthetic", 10.0, stage, joined).await,
                Err(WorkerFailure::Panicked)
            ));
            assert!(
                matches!(receiver.recv().await.unwrap(), MeetingEvent::CaptureError { meeting_id, message }
                if meeting_id == "synthetic" && message.starts_with(stage) && message.contains("stopped unexpectedly") && !message.contains("synthetic-private"))
            );
            assert!(receiver.try_recv().is_err());
            assert_eq!(state.recovery.failure_page(0).await.unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn cancelled_join_is_a_failure_not_a_successful_stage() {
        let (_dir, state) = isolated_state();
        let mut receiver = state.subscribe();
        let task = tokio::spawn(std::future::pending::<Result<(), std::io::Error>>());
        task.abort();
        assert!(matches!(
            finish_required(&state, "synthetic", 10.0, "transcription", task.await).await,
            Err(WorkerFailure::Cancelled)
        ));
        assert!(
            matches!(receiver.recv().await.unwrap(), MeetingEvent::CaptureError { message, .. } if message.contains("cancelled"))
        );
        assert_eq!(state.recovery.failure_page(0).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn unavailable_recovery_warns_without_losing_original_processing_error() {
        let (_root, mut state) = crate::state::isolated_state();
        state.recovery = crate::recovery::RecoveryQueue::new(None);
        let mut receiver = state.subscribe();
        fail(
            &state,
            "synthetic",
            10.0,
            "model download failed: synthetic outage".into(),
        )
        .await;
        assert!(
            matches!(receiver.try_recv().unwrap(), MeetingEvent::CaptureWarning { message, .. }
            if message.contains("recovery status could not be saved"))
        );
        assert!(
            matches!(receiver.try_recv().unwrap(), MeetingEvent::CaptureError { message, .. }
            if message == "model download failed: synthetic outage")
        );
        assert!(receiver.try_recv().is_err());
    }

    #[tokio::test]
    async fn final_write_failure_is_durable_without_speakers_or_completion() {
        let (dir, state) = isolated_state();
        let mut receiver = state.subscribe();
        let path = dir.path().join("not-a-directory");
        std::fs::write(&path, "synthetic retained fixture").unwrap();
        let joined = tokio::task::spawn_blocking(move || {
            crate::transcribe::segments_log::write_all(&path.join("segments.jsonl"), &[])
        })
        .await;
        finish_final(&state, "synthetic", 10.0, joined).await;
        assert!(
            matches!(receiver.try_recv().unwrap(), MeetingEvent::CaptureError { message, .. }
            if message.starts_with("final transcript failed:"))
        );
        assert!(
            receiver.try_recv().is_err(),
            "No successful completion after failed write"
        );
        assert!(state.recovery.page(0).await.unwrap().is_empty());
        assert_eq!(state.recovery.failure_page(0).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn successful_final_stage_announces_durable_speakers_then_completion() {
        let (_dir, state) = isolated_state();
        let mut receiver = state.subscribe();
        finish_final(&state, "synthetic", 10.0, Ok(Ok::<(), std::io::Error>(()))).await;
        assert!(
            matches!(receiver.try_recv().unwrap(), MeetingEvent::SpeakersAssigned { meeting_id }
            if meeting_id == "synthetic")
        );
        assert!(
            matches!(receiver.try_recv().unwrap(), MeetingEvent::TranscriptionCompleted { meeting_id }
            if meeting_id == "synthetic")
        );
        assert!(receiver.try_recv().is_err());
        assert_eq!(state.recovery.page(0).await.unwrap().len(), 1);
        assert!(state.recovery.failure_page(0).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn processing_failure_is_saved_before_event_and_replay_does_not_replace_receipt() {
        let (dir, state) = isolated_state();
        let mut receiver = state.subscribe();
        fail(
            &state,
            "synthetic",
            10.0,
            "model download failed: private fixture".into(),
        )
        .await;
        let reopened =
            crate::recovery::RecoveryQueue::new(Some(dir.path().join("recovery.sqlite3")));
        let original = reopened.failure_page(0).await.unwrap();
        assert_eq!(original.len(), 1);
        assert_eq!(original[0].started_at_ms, 10.0);
        assert!(matches!(
            receiver.try_recv().unwrap(),
            MeetingEvent::CaptureError { .. }
        ));
        fail(
            &state,
            "synthetic",
            10.0,
            "transcription failed: other private fixture".into(),
        )
        .await;
        assert_eq!(reopened.failure_page(0).await.unwrap(), original);
        let serialized = serde_json::to_string(&original).unwrap();
        assert!(!serialized.contains("private"));
    }
}
