use super::*;
use axum::{body::Bytes, http::StatusCode, routing::post, Json, Router};
use bruhs_models::MeetingEvent;

type Request = (header::HeaderMap, Bytes);

async fn stub(
    status: StatusCode,
    response: serde_json::Value,
) -> (
    String,
    tokio::task::JoinHandle<()>,
    tokio::sync::mpsc::UnboundedReceiver<Request>,
) {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    let app = Router::new().route(
        "/v1/audio/transcriptions",
        post(move |headers: header::HeaderMap, body: Bytes| {
            let (sender, response) = (sender.clone(), response.clone());
            async move {
                sender.send((headers, body)).unwrap();
                (
                    status,
                    [("location", "http://127.0.0.1:9/must-not-follow")],
                    Json(response),
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, task, receiver)
}

fn wav(path: &Path, frames: usize) {
    let mut writer = bruhs_models::logic::wav::WavWriter::create(
        path,
        bruhs_models::logic::wav::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: bruhs_models::logic::wav::SampleFormat::Int,
        },
    )
    .unwrap();
    for _ in 0..frames {
        writer.write_sample(1000i16).unwrap();
    }
    writer.finalize().unwrap();
}

#[tokio::test]
async fn selected_model_reaches_provider_and_final_segments_reach_disk_and_events() {
    let (base, server, mut requests) = stub(StatusCode::OK, serde_json::json!({
        "text": "Synthetic speech", "segments": [{"start": 0.1, "end": 0.8, "text": "Synthetic speech"}]
    })).await;
    let (_root, state) = crate::state::isolated_state();
    state.data.ensure_meeting_dir("synthetic-remote").unwrap();
    let path = state.data.mic_wav_path("synthetic-remote").unwrap();
    wav(&path, 16_000);
    let state = Arc::new(state);
    let mut events = state.subscribe();
    let job = RemoteProvider::synthetic(&base, Some("synthetic-provider-key"))
        .prepare("chosen-model")
        .unwrap();
    assert!(!format!("{job:?}").contains("synthetic-provider-key"));
    // Exercise the full post-stop pipeline, including mixing and diarization.
    crate::postprocess::postprocess_meeting(
        state.clone(),
        crate::postprocess::PostprocessOpts {
            meeting_id: "synthetic-remote".into(),
            started_at_ms: 1.0,
            mic_wav: path,
            sys_wav: state.data.sys_wav_path("synthetic-remote").unwrap(),
            mixed_wav: state.data.mixed_wav_path("synthetic-remote").unwrap(),
            transcription: super::super::TranscriptionJob::Compatible(job),
            subspeaker_enabled: false,
            max_speakers: crate::state::MaxSpeakers::DEFAULT,
            video_ocr_enabled: false,
            cross_meeting_linking_auto: false,
            cross_meeting_linking_threshold: crate::state::LinkingThreshold::DEFAULT,
        },
    )
    .await;
    let (headers, body) = requests.try_recv().unwrap();
    assert_eq!(
        headers[header::AUTHORIZATION],
        "Bearer synthetic-provider-key"
    );
    let body = String::from_utf8_lossy(&body);
    assert!(body.contains("name=\"model\"\r\n\r\nchosen-model"));
    assert!(body.contains("verbose_json"));
    assert!(body.contains("timestamp_granularities[]"));
    assert!(body.contains("filename=\"meeting.wav\""));
    let saved = super::super::segments_log::load_snapshot(
        &state.data.segments_jsonl_path("synthetic-remote").unwrap(),
    )
    .unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!((saved[0].start_ms, saved[0].end_ms), (100, 800));
    assert_eq!(saved[0].text, "Synthetic speech");
    assert_eq!(saved[0].speaker_id.as_deref(), Some("me"));
    assert!(!saved[0].partial);
    let mut published = false;
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(event, MeetingEvent::CaptureError { .. }));
        published |= matches!(event, MeetingEvent::Segment { .. });
    }
    assert!(published);
    assert!(!state.data.models_dir().exists());
    server.abort();
}

#[tokio::test]
async fn provider_errors_and_redirects_do_not_echo_private_responses_or_switch_providers() {
    for status in [StatusCode::UNAUTHORIZED, StatusCode::TEMPORARY_REDIRECT] {
        let (base, server, mut requests) = stub(
            status,
            serde_json::json!({"error": "synthetic-private-echo"}),
        )
        .await;
        let (_root, state) = crate::state::isolated_state();
        state.data.ensure_meeting_dir("synthetic-error").unwrap();
        let path = state.data.mic_wav_path("synthetic-error").unwrap();
        wav(&path, 16_000);
        let state = Arc::new(state);
        let mut events = state.subscribe();
        let error = RemoteProvider::synthetic(&base, Some("synthetic-provider-key"))
            .prepare("chosen-model")
            .unwrap()
            .transcribe(state.clone(), "synthetic-error".into(), path.clone())
            .await
            .unwrap_err();
        assert!(matches!(error, RemoteError::Status(actual) if actual == status));
        assert!(!error.to_string().contains("synthetic-private-echo"));
        assert!(!error.to_string().contains("synthetic-provider-key"));
        assert!(path.exists());
        assert!(!state
            .data
            .segments_jsonl_path("synthetic-error")
            .unwrap()
            .exists());
        assert!(events.try_recv().is_err());
        requests.try_recv().unwrap();
        assert!(requests.try_recv().is_err(), "must not retry automatically");
        server.abort();
    }
}

#[test]
fn unsupported_formats_and_invalid_timestamps_cannot_become_successful_transcripts() {
    assert!(serde_json::from_value::<VerboseTranscript>(
        serde_json::json!({"text":"no timestamps"})
    )
    .is_err());
    for response in [
        serde_json::json!({"text":"no timestamps", "segments": []}),
        serde_json::json!({"text":"backwards", "segments": [{"start": 0.8,"end":0.1,"text":"invalid"}]}),
        serde_json::json!({"text":"outside", "segments": [{"start": 0.1,"end":2.0,"text":"invalid"}]}),
        serde_json::json!({"text":"negative", "segments": [{"start": -0.1,"end":0.1,"text":"invalid"}]}),
    ] {
        assert!(serde_json::from_value::<VerboseTranscript>(response)
            .unwrap()
            .decode(0, 1000)
            .is_err());
    }
    let response: VerboseTranscript =
        serde_json::from_value(serde_json::json!({"text":"", "segments":[]})).unwrap();
    assert!(response.decode(0, 1000).unwrap().is_empty());
    let response: VerboseTranscript = serde_json::from_value(
        serde_json::json!({"text":"later", "segments":[{"start":0.1,"end":0.8,"text":"later"}]}),
    )
    .unwrap();
    let decoded = response.decode(10_000, 1000).unwrap();
    assert_eq!((decoded[0].start_ms, decoded[0].end_ms), (10_100, 10_800));
}

#[tokio::test]
async fn provider_failure_is_durable_and_keeps_original_audio_without_local_fallback() {
    let (base, server, mut requests) = stub(
        StatusCode::BAD_GATEWAY,
        serde_json::json!({"error":"synthetic-private-response"}),
    )
    .await;
    let (root, mut state) = crate::state::isolated_state();
    state.recovery =
        crate::recovery::RecoveryQueue::new(Some(root.path().join("recovery.sqlite3")));
    state.data.ensure_meeting_dir("synthetic-failure").unwrap();
    let path = state.data.mic_wav_path("synthetic-failure").unwrap();
    wav(&path, 16_000);
    let original = std::fs::read(&path).unwrap();
    let mut events = state.subscribe();
    let state = Arc::new(state);
    let job = RemoteProvider::synthetic(&base, None)
        .prepare("selected-model")
        .unwrap();
    crate::postprocess::postprocess_meeting(
        state.clone(),
        crate::postprocess::PostprocessOpts {
            meeting_id: "synthetic-failure".into(),
            started_at_ms: 1.0,
            mic_wav: path.clone(),
            sys_wav: state.data.sys_wav_path("synthetic-failure").unwrap(),
            mixed_wav: state.data.mixed_wav_path("synthetic-failure").unwrap(),
            transcription: super::super::TranscriptionJob::Compatible(job),
            subspeaker_enabled: false,
            max_speakers: crate::state::MaxSpeakers::DEFAULT,
            video_ocr_enabled: false,
            cross_meeting_linking_auto: false,
            cross_meeting_linking_threshold: crate::state::LinkingThreshold::DEFAULT,
        },
    )
    .await;
    let MeetingEvent::CaptureError { message, .. } = events.try_recv().unwrap() else {
        panic!("provider failure must produce a terminal capture error");
    };
    assert!(message.contains("502"));
    assert!(!message.contains("synthetic-private-response"));
    assert!(events.try_recv().is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(state.recovery.failure_page(0).await.unwrap().len(), 1);
    assert!(!state.data.models_dir().exists());
    requests.try_recv().unwrap();
    server.abort();
}

#[test]
fn chunking_stays_within_upload_limit_and_preserves_source_time() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic-long.wav");
    let first_frames = (MAX_UPLOAD_BYTES - WAV_HEADER_BYTES) / 2;
    wav(&path, first_frames + 16_000);
    let mut chunks = WavChunks::open(&path).unwrap();
    let first = chunks.next_chunk().unwrap().unwrap();
    assert!(first.bytes.len() <= MAX_UPLOAD_BYTES);
    assert_eq!(first.start_ms, 0);
    let first_reader =
        bruhs_models::logic::wav::WavReader::new(std::io::Cursor::new(first.bytes)).unwrap();
    assert_eq!(
        usize::try_from(first_reader.duration()).unwrap(),
        first_frames
    );
    let next = chunks.next_chunk().unwrap().unwrap();
    assert_eq!(
        next.start_ms,
        u32::try_from(first_frames * 1000 / 16_000).unwrap()
    );
    assert_eq!(next.duration_ms, 1000);
    let reader =
        bruhs_models::logic::wav::WavReader::new(std::io::Cursor::new(next.bytes)).unwrap();
    assert_eq!(reader.duration(), 16_000);
    assert!(chunks.next_chunk().unwrap().is_none());
}

#[test]
fn endpoint_validation_keeps_keys_bound_to_a_secure_configured_service() {
    for base in [
        "http://remote.test/v1",
        "https://user:pass@remote.test/v1",
        "https://remote.test/v1?key=secret",
        "https://remote.test/#fragment",
        "file:///tmp/audio",
    ] {
        assert!(remote_client(base, None, Duration::from_secs(5)).is_err());
    }
    let client = remote_client("http://[::1]:1234/v1/", None, Duration::from_secs(5)).unwrap();
    assert_eq!(
        client.endpoint.as_str(),
        "http://[::1]:1234/v1/audio/transcriptions"
    );
    assert!(remote_client(
        "https://remote.test/v1",
        Some("invalid\nkey"),
        Duration::from_secs(5)
    )
    .is_err());
}

#[tokio::test]
async fn worker_panic_payload_is_not_forwarded_into_meeting_errors() {
    let joined = tokio::spawn(async { panic!("synthetic-private-worker-payload") }).await;
    let error = RemoteError::from(joined.unwrap_err());
    assert!(matches!(error, RemoteError::WorkerPanicked));
    assert!(!error
        .to_string()
        .contains("synthetic-private-worker-payload"));
}

#[tokio::test]
async fn cloud_destinations_use_the_selected_model_language_and_transport() {
    let (base, server, mut received) = stub(
        StatusCode::OK,
        serde_json::json!({"text":"synthetic result"}),
    )
    .await;
    for (provider, expected_url) in [
        (
            CloudProvider::OpenAi,
            "https://api.openai.com/v1/audio/transcriptions",
        ),
        (CloudProvider::Xai, "https://api.x.ai/v1/stt"),
        (
            CloudProvider::OpenRouter,
            "https://openrouter.ai/api/v1/audio/transcriptions",
        ),
    ] {
        let model = provider.fixed_model().unwrap_or("scratch/test");
        let hints = SpeechHints {
            terms: vec!["Bruhs".into(), "Crème brûlée".into(), "<Scratch>".into()],
            preserve_correction_words: true,
        };
        let mut job =
            RemoteJob::cloud(provider, model.into(), "private-fixture-key", hints).unwrap();
        assert_eq!(job.remote.endpoint.as_str(), expected_url);
        job.remote.endpoint = Url::parse(&format!("{base}/audio/transcriptions")).unwrap();
        let mut options = DictationOptions {
            client_filters_text: false,
            model: Default::default(),
            language: Some("es".into()),
            remove_filler_words: true,
            silence_filtering: false,
        };
        assert_eq!(
            job.dictate(vec![1, 2, 3], &options).await.unwrap(),
            "synthetic result"
        );
        let (headers, bytes) = received.recv().await.unwrap();
        assert_eq!(headers[header::AUTHORIZATION], "Bearer private-fixture-key");
        if provider == CloudProvider::OpenRouter {
            let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(json["model"], model);
            assert_eq!(
                json["input_audio"],
                serde_json::json!({"data":"AQID","format":"wav"})
            );
            assert_eq!(json["provider"]["zdr"], true);
            assert_eq!(json["language"], "es");
            assert_eq!(json.as_object().unwrap().len(), 4);
        } else {
            let body = String::from_utf8_lossy(&bytes);
            assert!(body.contains(&format!("name=\"model\"\r\n\r\n{model}")));
            let field = if provider == CloudProvider::OpenAi {
                "languages[]"
            } else {
                "language"
            };
            assert!(body.contains(&format!("name=\"{field}\"\r\n\r\nes")));
            assert!(body.contains("filename=\"dictation.wav\""));
            let field = if provider == CloudProvider::OpenAi {
                "keywords[]"
            } else {
                "keyterm"
            };
            for term in ["Bruhs", "Crème brûlée"] {
                assert!(body.contains(&format!("name=\"{field}\"\r\n\r\n{term}\r\n")));
            }
            assert_eq!(body.contains("<Scratch>"), provider == CloudProvider::Xai);
            assert_eq!(
                body.contains("name=\"vad_threshold\"\r\n\r\n0"),
                provider == CloudProvider::Xai
            );
            assert_eq!(
                body.contains("name=\"filler_words\"\r\n\r\ntrue"),
                provider == CloudProvider::Xai
            );
        }
        options.language = Some("cy".into());
        options.silence_filtering = true;
        job.hints.preserve_correction_words = false;
        job.dictate(vec![4], &options).await.unwrap();
        let (_, bytes) = received.recv().await.unwrap();
        let body = String::from_utf8_lossy(&bytes);
        assert!(!body.contains("language"));
        assert!(!body.contains("vad_threshold"));
        if provider == CloudProvider::Xai {
            assert!(body.contains("name=\"filler_words\"\r\n\r\nfalse"));
            assert!(!body.contains("name=\"format\""));
        }
    }
    server.abort();
}
