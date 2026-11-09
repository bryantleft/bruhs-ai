use super::*;
use crate::capture::{CaptureError, SystemAudioRecorder};
use std::{sync::mpsc, time::Duration};

fn session() -> CurrentSession {
    CurrentSession {
        generation: uuid::Uuid::new_v4(),
        meeting_id: "synthetic-lifecycle".into(),
        started_at_ms: 1.0,
        mic_wav_path: "unused-synthetic-mic.wav".into(),
        sys_wav_path: "unused-synthetic-sys.wav".into(),
        mixed_wav_path: "unused-synthetic-mixed.wav".into(),
        transcription: TranscriptionJob::Whisper(WhisperModel::default()),
        live_model: None,
        subspeaker_enabled: false,
        cross_meeting_linking_auto: Some(false),
        cross_meeting_linking_threshold: None,
        max_speakers: None,
        video_ocr_enabled: false,
    }
}

#[tokio::test]
async fn duplicate_start_is_rejected_before_any_hardware_or_path_access() {
    let (_root, state) = crate::state::isolated_state();
    let state = Arc::new(state);
    state.with_current_mut(|slot| *slot = Some(session()));
    let request = serde_json::from_value(serde_json::json!({
        "meeting_id": "must-not-open-a-device", "started_at_ms": 2.0
    }))
    .unwrap();
    let error = start_capture(State(state.clone()), Ok(Json(request)))
        .await
        .unwrap_err();
    assert_eq!(error.0, StatusCode::CONFLICT);
    assert_eq!(
        state.with_current(|slot| slot.unwrap().meeting_id.clone()),
        "synthetic-lifecycle"
    );
    assert!(state.lifecycle.try_enter().is_some());
}

#[tokio::test]
async fn unrecoverable_start_time_is_rejected_before_lifecycle_work() {
    let (_root, state) = crate::state::isolated_state();
    let state = Arc::new(state);
    // Defense in depth: a regression must hit Conflict, never open real devices.
    state.with_current_mut(|slot| *slot = Some(session()));
    for started_at_ms in [-1.0, f64::NAN, f64::INFINITY, 8_640_000_000_000_001.0] {
        let mut request: StartRequest = serde_json::from_value(serde_json::json!({
            "meeting_id": "synthetic", "started_at_ms": 0.0
        }))
        .unwrap();
        request.started_at_ms = started_at_ms;
        let error = start_capture(State(state.clone()), Ok(Json(request)))
            .await
            .unwrap_err();
        assert_eq!(
            error,
            (
                StatusCode::BAD_REQUEST,
                "Invalid recording start time".into()
            )
        );
        assert!(state.lifecycle.try_enter().is_some());
    }
    assert_eq!(state.with_current(|slot| slot.unwrap().started_at_ms), 1.0);
}

#[tokio::test]
async fn idle_stop_does_not_open_hardware_and_releases_its_reservation() {
    let (_root, state) = crate::state::isolated_state();
    let state = Arc::new(state);
    assert_eq!(
        stop_capture(State(state.clone())).await.unwrap_err().0,
        StatusCode::CONFLICT
    );
    assert!(state.lifecycle.try_enter().is_some());
}

struct SyntheticSystemRecorder {
    started: tokio::sync::oneshot::Sender<()>,
    release: mpsc::Receiver<()>,
}

fn isolated_state() -> (tempfile::TempDir, Arc<CaptureState>) {
    let dir = tempfile::tempdir().unwrap();
    let mut state = CaptureState::new(crate::paths::DataRoot::at(dir.path()));
    state.remote_transcription = crate::transcribe::remote::RemoteProvider::unconfigured();
    state.recovery = crate::recovery::RecoveryQueue::new(Some(dir.path().join("recovery.sqlite3")));
    (dir, Arc::new(state))
}

#[tokio::test]
async fn invalid_remote_choice_fails_before_any_recording_or_path_access() {
    let (root, state) = isolated_state();
    // A valid request would hit Conflict, so a regression cannot open hardware.
    state.with_current_mut(|slot| *slot = Some(session()));
    for (model, expected_status) in [
        ("", StatusCode::BAD_REQUEST),
        ("synthetic-model", StatusCode::SERVICE_UNAVAILABLE),
    ] {
        let request: StartRequest = serde_json::from_value(serde_json::json!({
            "meeting_id": "synthetic-remote", "started_at_ms": 1.0,
            "transcription": {"provider": "open_ai_compatible", "remote_model": model}
        }))
        .unwrap();
        let error = start_capture(State(state.clone()), Ok(Json(request)))
            .await
            .unwrap_err();
        assert_eq!(error.0, expected_status);
        assert!(state.lifecycle.try_enter().is_some());
        assert!(!root.path().join("audio").exists());
    }
    let legacy: StartRequest = serde_json::from_value(serde_json::json!({
        "meeting_id": "synthetic-local", "started_at_ms": 1.0
    }))
    .unwrap();
    assert_eq!(
        start_capture(State(state), Ok(Json(legacy)))
            .await
            .unwrap_err()
            .0,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn successful_mic_cannot_mask_failed_system_audio_or_start_postprocessing() {
    let (_dir, state) = isolated_state();
    state.with_current_mut(|slot| *slot = Some(session()));
    state.set_mic_recorder(MicRecorder::synthetic_completed());
    let (started, _started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release) = mpsc::channel();
    release_tx.send(()).unwrap();
    state.set_sys_recorder(Box::new(SyntheticSystemRecorder { started, release }));
    let mut events = state.subscribe();
    let error = stop_capture(State(state.clone())).await.unwrap_err();
    assert_eq!(
        error,
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "sys stop: backend: synthetic system stop failure".into()
        )
    );
    assert!(state.with_current(|slot| slot.is_none()));
    assert!(matches!(
        events.try_recv().unwrap(),
        MeetingEvent::CaptureError { .. }
    ));
    assert!(
        events.try_recv().is_err(),
        "must not emit Ended or start postprocessing"
    );
    assert!(state.lifecycle.try_enter().is_some());
    let receipts = state.recovery.failure_page(0).await.unwrap();
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].meeting_id, "synthetic-lifecycle");
    assert_eq!(receipts[0].started_at_ms, 1.0);
}

#[tokio::test]
async fn failed_recovery_storage_warns_but_does_not_skip_cleanup_or_terminal_error() {
    let (_dir, mut_state) = isolated_state();
    let mut state = Arc::try_unwrap(mut_state).ok().unwrap();
    state.recovery = crate::recovery::RecoveryQueue::new(None);
    let state = Arc::new(state);
    state.with_current_mut(|slot| *slot = Some(session()));
    let mut events = state.subscribe();
    assert_eq!(
        stop_capture(State(state.clone())).await.unwrap_err().0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert!(state.with_current(|slot| slot.is_none()));
    assert!(matches!(
        events.try_recv().unwrap(),
        MeetingEvent::CaptureWarning { .. }
    ));
    assert!(matches!(
        events.try_recv().unwrap(),
        MeetingEvent::CaptureError { .. }
    ));
    assert!(events.try_recv().is_err());
}

#[tokio::test]
async fn failure_recovery_http_is_bounded_and_rejects_foreign_acknowledgments() {
    let (_dir, state) = isolated_state();
    state.with_current_mut(|slot| *slot = Some(session()));
    assert!(stop_capture(State(state.clone())).await.is_err());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (app, authorized) = crate::test_router(state.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = authorized;
    let response = client
        .get(format!("{base}/recovery/failures"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let page = response
        .json::<bruhs_models::CaptureFailurePage>()
        .await
        .unwrap();
    assert_eq!(page.receipts.len(), 1);
    let url = format!("{base}/recovery/failures/ack");
    assert_eq!(
        client
            .post(&url)
            .header("Origin", "https://foreign.test")
            .json(&page.receipts)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(state.recovery.failure_page(0).await.unwrap(), page.receipts);
    assert_eq!(
        client
            .post(&url)
            .header("content-type", "application/json")
            .body(" ".repeat(33 * 1024))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        client
            .post(&url)
            .json(&page.receipts)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert!(state.recovery.failure_page(0).await.unwrap().is_empty());
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

impl SystemAudioRecorder for SyntheticSystemRecorder {
    fn stop(self: Box<Self>) -> Result<(), CaptureError> {
        self.started.send(()).unwrap();
        self.release.recv_timeout(Duration::from_secs(3)).unwrap();
        Err(CaptureError::Backend(
            "synthetic system stop failure".into(),
        ))
    }
}

#[tokio::test(flavor = "current_thread")]
async fn mic_failure_still_stops_system_audio_before_clearing_recording_status() {
    let (_dir, state) = isolated_state();
    state.with_current_mut(|slot| *slot = Some(session()));
    let (started, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release) = mpsc::channel();
    state.set_sys_recorder(Box::new(SyntheticSystemRecorder { started, release }));
    let mut events = state.subscribe();
    let stopping = state.clone();
    let stop = tokio::spawn(async move { stop_capture(State(stopping)).await });
    tokio::time::timeout(Duration::from_secs(3), started_rx)
        .await
        .unwrap()
        .unwrap();
    assert!(
        state.with_current(|slot| slot.is_some()),
        "reported stopped before hardware cleanup"
    );
    assert!(state.lifecycle.try_enter().is_none());
    release_tx.send(()).unwrap();
    let error = stop.await.unwrap().unwrap_err();
    assert_eq!(
        error,
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "mic recorder missing; sys stop: backend: synthetic system stop failure".into()
        )
    );
    assert!(state.with_current(|slot| slot.is_none()));
    assert!(matches!(
        events.try_recv().unwrap(),
        MeetingEvent::CaptureError { .. }
    ));
    assert!(
        events.try_recv().is_err(),
        "failure must not emit a successful Ended event"
    );
    assert!(state.lifecycle.try_enter().is_some());
}

#[test]
fn repeated_meeting_id_does_not_authorize_an_old_model_resolution() {
    let (_root, state) = crate::state::isolated_state();
    let old = session();
    let old_generation = old.generation;
    state.with_current_mut(|slot| *slot = Some(old));
    assert!(state.is_current_generation(old_generation));
    state.with_current_mut(|slot| *slot = None);
    assert!(!state.is_current_generation(old_generation));
    let replacement = session();
    let new_generation = replacement.generation;
    state.with_current_mut(|slot| *slot = Some(replacement));
    assert!(!state.is_current_generation(old_generation));
    assert!(state.is_current_generation(new_generation));
}

#[tokio::test(flavor = "current_thread")]
async fn production_http_health_responds_during_a_synthetic_hardware_stop() {
    let (_dir, state) = isolated_state();
    state.with_current_mut(|slot| *slot = Some(session()));
    let (started, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release) = mpsc::channel();
    state.set_sys_recorder(Box::new(SyntheticSystemRecorder { started, release }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    // Production handlers, but never main() or its hardware worker.
    let (app, authorized) = crate::test_router(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = authorized;
    let stop = client.post(format!("{base}/capture/stop"));
    let stopping = tokio::spawn(async move { stop.send().await.unwrap() });
    tokio::time::timeout(Duration::from_secs(2), started_rx)
        .await
        .unwrap()
        .unwrap();
    let health = client.get(format!("{base}/health")).send().await.unwrap();
    assert_eq!(health.status(), StatusCode::OK);
    assert!(
        !stopping.is_finished(),
        "synthetic hardware should still be held"
    );
    assert!(state.with_current(|slot| slot.is_some()));
    release_tx.send(()).unwrap();
    let response = stopping.await.unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        response.text().await.unwrap(),
        "mic recorder missing; sys stop: backend: synthetic system stop failure"
    );
    assert!(state.with_current(|slot| slot.is_none()));
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn production_capture_boundaries_reject_paths_and_unsupported_privacy_options() {
    let (_root, state) = crate::state::isolated_state();
    let state = Arc::new(state);
    // A sentinel session also makes a failed validation regression safe: it
    // returns Conflict instead of opening a physical device in this test.
    state.with_current_mut(|slot| *slot = Some(session()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (app, authorized) = crate::test_router(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = authorized;
    for id in ["", "../outside", "/absolute", "a\\b", "a\0b", "a b"] {
        let response = client
            .post(format!("{base}/capture/start"))
            .header("content-type", "application/json")
            .body(serde_json::json!({"meeting_id": id, "started_at_ms": 1.0}).to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{id:?}");
    }
    let fixture_dir = tempfile::tempdir().unwrap();
    let outside = fixture_dir.path().join("must-not-be-created");
    let encoded_path = outside.to_str().unwrap().replace('/', "%2F");
    for route in ["centroids", "ocr", "segments"] {
        let response = client
            .get(format!("{base}/{route}/{encoded_path}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{route}");
        assert!(!outside.exists());
    }
    // Neither future mute protection nor a misspelled privacy option may be
    // silently discarded while ordinary microphone capture starts.
    for option in ["respect_meeting_mute", "video_ocr_enable"] {
        let body =
            serde_json::json!({"meeting_id": "synthetic", "started_at_ms": 1.0, option: true});
        let response = client
            .post(format!("{base}/capture/start"))
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{option}");
        assert!(response.text().await.unwrap().contains(option));
    }
    assert_eq!(
        state.with_current(|slot| slot.unwrap().meeting_id.clone()),
        "synthetic-lifecycle"
    );
    assert!(!state.monitor.status().enabled);
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

#[test]
fn current_client_capture_options_still_deserialize() {
    let request: StartRequest = serde_json::from_value(serde_json::json!({
        "meeting_id": "synthetic", "started_at_ms": 1.0,
        "model": null, "live_model": null, "subspeaker_enabled": false,
        "cross_meeting_linking_auto": false, "cross_meeting_linking_threshold": 0.75,
        "max_speakers": 6, "video_ocr_enabled": false
    }))
    .unwrap();
    assert_eq!(request.video_ocr_enabled, Some(false));
    assert_eq!(request.cross_meeting_linking_auto, Some(false));
}

/// Reads used to create the meeting directory as a side effect of building
/// the path, so polling an unknown meeting littered the audio directory.
#[tokio::test]
async fn reading_an_unknown_meeting_creates_no_directories() {
    let (root, state) = isolated_state();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (app, authorized) = crate::test_router(state.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = authorized;
    for route in ["ocr", "centroids", "segments"] {
        let response = client
            .get(format!("{base}/{route}/never-recorded"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{route}");
    }
    assert!(!state.data.audio_dir().exists());
    assert!(!root.path().join("audio").exists());
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

/// Holds the video frame sender like the SCK stream does, never sends a frame
/// (static screen), and releases it only when stopped.
struct StaticScreenRecorder {
    _frames: mpsc::SyncSender<crate::video_ocr::OcrMsg<u32>>,
}

impl SystemAudioRecorder for StaticScreenRecorder {
    fn stop(self: Box<Self>) -> Result<(), CaptureError> {
        Ok(())
    }
}

/// H9: `/capture/stop` used to join the OCR worker before stopping the system
/// recorder; with no frames arriving the join never returned and the stop held
/// the lifecycle lock forever.
#[tokio::test]
async fn stop_with_video_ocr_and_a_static_screen_completes() {
    let (root, state) = isolated_state();
    let mut current = session();
    current.video_ocr_enabled = true;
    state.with_current_mut(|slot| *slot = Some(current));
    state
        .data
        .ensure_meeting_dir("synthetic-lifecycle")
        .unwrap();
    let (frames, channel) = crate::video_ocr::frame_channel::<u32>(1);
    let ocr_path = state.data.ocr_path("synthetic-lifecycle").unwrap();
    state.set_ocr_worker(
        crate::video_ocr::OcrWorker::spawn(
            channel,
            ocr_path.clone(),
            std::time::Instant::now(),
            |_, _| Vec::new(),
        )
        .unwrap(),
    );
    state.set_sys_recorder(Box::new(StaticScreenRecorder { _frames: frames }));
    // No mic recorder: the stop fails, so no postprocessing starts.
    let stopped = tokio::time::timeout(Duration::from_secs(10), stop_capture(State(state.clone())))
        .await
        .expect("stop must not wait for a video frame");
    assert_eq!(stopped.unwrap_err().1, "mic recorder missing");
    assert_eq!(std::fs::read_to_string(&ocr_path).unwrap(), "[]");
    assert!(state.lifecycle.try_enter().is_some());
    drop(root);
}

#[tokio::test]
async fn out_of_range_start_options_are_bad_requests() {
    let (_root, state) = isolated_state();
    // Valid requests reach the lifecycle and stop at the sentinel session.
    state.with_current_mut(|slot| *slot = Some(session()));
    let start = |options: serde_json::Value| {
        let mut body = serde_json::json!({"meeting_id": "synthetic", "started_at_ms": 1.0});
        body.as_object_mut()
            .unwrap()
            .extend(options.as_object().unwrap().clone());
        let request: StartRequest = serde_json::from_value(body).unwrap();
        start_capture(State(state.clone()), Ok(Json(request)))
    };
    for options in [
        serde_json::json!({"max_speakers": 1}),
        serde_json::json!({"max_speakers": 21}),
        serde_json::json!({"cross_meeting_linking_threshold": 0.49}),
        serde_json::json!({"cross_meeting_linking_threshold": 0.96}),
        serde_json::json!({"cross_meeting_linking_threshold": -1.0}),
    ] {
        let error = start(options.clone()).await.unwrap_err();
        assert_eq!(error.0, StatusCode::BAD_REQUEST, "{options}");
    }
    // A wrong type is a 400 too, like an out-of-range value (not axum's 422).
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (app, client) = crate::test_router(state.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    for options in [
        serde_json::json!({"max_speakers": "six"}),
        serde_json::json!({"max_speakers": -1}),
        serde_json::json!({"cross_meeting_linking_threshold": "high"}),
    ] {
        let mut body = serde_json::json!({"meeting_id": "synthetic", "started_at_ms": 1.0});
        body.as_object_mut()
            .unwrap()
            .extend(options.as_object().unwrap().clone());
        let response = client
            .post(format!("{base}/capture/start"))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{options}");
    }
    server.abort();
    for options in [
        serde_json::json!({"max_speakers": 2, "cross_meeting_linking_threshold": 0.5}),
        serde_json::json!({"max_speakers": 20, "cross_meeting_linking_threshold": 0.95}),
    ] {
        let error = start(options.clone()).await.unwrap_err();
        assert_eq!(error.0, StatusCode::CONFLICT, "{options}");
    }
}
