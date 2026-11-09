use super::*;
use bruhs_models::{MeetingEvent, WhisperModel};

#[tokio::test]
async fn mix_failure_retains_durable_attempt_without_entering_model_download() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recovery.sqlite3");
    let (_root, mut state) = crate::state::isolated_state();
    state.recovery = crate::recovery::RecoveryQueue::new(Some(path.clone()));
    let state = Arc::new(state);
    let mut events = state.subscribe();
    // Missing input fails in the real mixer, before any model/network/device work.
    postprocess_meeting(
        state.clone(),
        PostprocessOpts {
            meeting_id: "synthetic-processing".into(),
            started_at_ms: 1200.0,
            mic_wav: dir.path().join("missing.wav"),
            sys_wav: dir.path().join("sys.wav"),
            mixed_wav: dir.path().join("mixed.wav"),
            transcription: crate::transcribe::TranscriptionJob::Whisper(WhisperModel::default()),
            subspeaker_enabled: false,
            max_speakers: crate::state::MaxSpeakers::new(2).unwrap(),
            video_ocr_enabled: false,
            cross_meeting_linking_auto: false,
            cross_meeting_linking_threshold: crate::state::LinkingThreshold::DEFAULT,
        },
    )
    .await;
    assert!(
        matches!(events.try_recv().unwrap(), MeetingEvent::CaptureError { meeting_id, message }
        if meeting_id == "synthetic-processing" && message.starts_with("mix failed:"))
    );
    assert!(
        events.try_recv().is_err(),
        "No downstream success event after failed mixing"
    );
    drop(state);
    let reopened = crate::recovery::RecoveryQueue::new(Some(path));
    let receipts = reopened.failure_page(0).await.unwrap();
    assert_eq!(
        receipts.len(),
        1,
        "Processing failure must survive disconnected consumers"
    );
    assert_eq!(receipts[0].meeting_id, "synthetic-processing");
    assert_eq!(receipts[0].started_at_ms, 1200.0);
}

#[tokio::test]
async fn corrupt_final_transcript_is_not_rewritten_or_announced_as_complete() {
    let dir = tempfile::tempdir().unwrap();
    let (_root, mut state) = crate::state::isolated_state();
    state.recovery = crate::recovery::RecoveryQueue::new(Some(dir.path().join("recovery.sqlite3")));
    let mut events = state.subscribe();
    let path = dir.path().join("segments.jsonl");
    let original = b"{incomplete synthetic transcript\n";
    std::fs::write(&path, original).unwrap();
    let input = path.clone();
    let joined = tokio::task::spawn_blocking(move || {
        let segments = crate::transcribe::segments_log::load_snapshot(&input)?;
        crate::transcribe::segments_log::write_all(&input, &segments)
    })
    .await;
    worker::finish_final(&state, "synthetic", 10.0, joined).await;
    assert_eq!(std::fs::read(path).unwrap(), original);
    assert!(matches!(
        events.try_recv().unwrap(),
        MeetingEvent::CaptureError { .. }
    ));
    assert!(events.try_recv().is_err());
    assert_eq!(state.recovery.failure_page(0).await.unwrap().len(), 1);
    assert!(state.recovery.page(0).await.unwrap().is_empty());
}
