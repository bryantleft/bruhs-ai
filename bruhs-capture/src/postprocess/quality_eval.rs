//! Opt-in replay of the real post-recording pipeline without capture devices.
//! Inputs must be an explicitly synthetic/consented, hashed corpus and an
//! already-installed Base English model. Nothing is downloaded or recorded.

use super::{postprocess_meeting, PostprocessOpts};
use bruhs_models::{MeetingEvent, TranscriptSegment, WhisperModel};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

#[derive(Deserialize)]
struct Manifest {
    provenance: String,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    mic_audio: PathBuf,
    system_audio: PathBuf,
    sha256: BTreeMap<String, String>,
}

#[derive(Serialize)]
struct Measurement {
    id: String,
    /// Time from entry into mixing to delivery of the first persisted segment.
    first_segment_ms: Option<f64>,
    /// Time through mixing, model verification, ASR, channel assignment and the
    /// durable final transcript/completion receipt. Excludes device shutdown.
    finalization_ms: f64,
    segments: Vec<TranscriptSegment>,
}

fn sha256(path: &Path) -> String {
    let mut file = std::fs::File::open(path).unwrap();
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    format!("{:x}", hasher.finalize())
}

fn observe(
    event: MeetingEvent,
    started: Instant,
    first_segment_ms: &mut Option<f64>,
    completed: &mut bool,
) {
    match event {
        MeetingEvent::Segment { .. } => {
            first_segment_ms.get_or_insert_with(|| started.elapsed().as_secs_f64() * 1000.0);
        }
        MeetingEvent::TranscriptionCompleted { .. } => *completed = true,
        MeetingEvent::CaptureError { message, .. }
        | MeetingEvent::CaptureWarning { message, .. } => panic!("pipeline failed: {message}"),
        _ => {}
    }
}

#[tokio::test]
#[ignore = "offline quality replay requires explicit corpus, installed model and output paths"]
async fn offline_postprocess_quality() {
    let manifest_path =
        PathBuf::from(std::env::var_os("BRUHS_TRANSCRIPTION_EVAL_MANIFEST").unwrap());
    let model_path = PathBuf::from(std::env::var_os("BRUHS_TRANSCRIPTION_EVAL_MODEL").unwrap());
    let output_path = PathBuf::from(std::env::var_os("BRUHS_TRANSCRIPTION_EVAL_OUTPUT").unwrap());
    let manifest_hash = sha256(&manifest_path);
    let manifest: Manifest =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    assert!(matches!(
        manifest.provenance.as_str(),
        "synthetic" | "consented"
    ));
    assert!(!manifest.cases.is_empty());
    let fixture_dir = manifest_path.parent().unwrap();
    let model = WhisperModel::BaseEn;
    let descriptor = model.descriptor();
    assert_eq!(model_path.metadata().unwrap().len(), descriptor.size_bytes);
    assert_eq!(sha256(&model_path), descriptor.sha256);

    let (_root, state) = crate::state::isolated_state();
    let artifact = crate::models_dir::ModelArtifact::whisper(model);
    let cached_model = state.models.path(&artifact);
    std::fs::create_dir_all(cached_model.parent().unwrap()).unwrap();
    // Copy into an isolated cache; never chmod, link or mutate the installed model.
    std::fs::copy(&model_path, &cached_model).unwrap();
    assert_eq!(sha256(&cached_model), descriptor.sha256);
    let state = Arc::new(state);
    let mut measurements = Vec::new();
    let mut fixture_hashes = BTreeMap::new();

    // Check and copy every input before the first measured run. A mismatched
    // cache cannot enter ModelStore's download path.
    for case in &manifest.cases {
        state.data.ensure_meeting_dir(&case.id).unwrap();
        for (key, source, destination) in [
            (
                "mic_audio",
                &case.mic_audio,
                state.data.mic_wav_path(&case.id).unwrap(),
            ),
            (
                "system_audio",
                &case.system_audio,
                state.data.sys_wav_path(&case.id).unwrap(),
            ),
        ] {
            let source = fixture_dir.join(source);
            let expected = &case.sha256[key];
            assert_eq!(&sha256(&source), expected);
            std::fs::copy(&source, &destination).unwrap();
            assert_eq!(&sha256(&destination), expected);
            fixture_hashes.insert(source, expected.clone());
        }
    }

    for case in manifest.cases {
        let mut receiver = state.subscribe();
        let started = Instant::now();
        let operation = postprocess_meeting(
            state.clone(),
            PostprocessOpts {
                meeting_id: case.id.clone(),
                started_at_ms: 0.0,
                mic_wav: state.data.mic_wav_path(&case.id).unwrap(),
                sys_wav: state.data.sys_wav_path(&case.id).unwrap(),
                mixed_wav: state.data.mixed_wav_path(&case.id).unwrap(),
                transcription: crate::transcribe::TranscriptionJob::Whisper(model),
                subspeaker_enabled: false,
                max_speakers: crate::state::MaxSpeakers::new(2).unwrap(),
                video_ocr_enabled: false,
                cross_meeting_linking_auto: false,
                cross_meeting_linking_threshold: crate::state::LinkingThreshold::DEFAULT,
            },
        );
        tokio::pin!(operation);
        let mut first_segment_ms = None;
        let mut completed = false;
        loop {
            tokio::select! {
                () = &mut operation => break,
                event = receiver.recv() => observe(
                    event.unwrap(), started, &mut first_segment_ms, &mut completed,
                ),
            }
        }
        while let Ok(event) = receiver.try_recv() {
            observe(event, started, &mut first_segment_ms, &mut completed);
        }
        let finalization_ms = started.elapsed().as_secs_f64() * 1000.0;
        assert!(completed, "missing durable completion for {}", case.id);
        let segments_path = state.data.segments_jsonl_path(&case.id).unwrap();
        let segments = crate::transcribe::segments_log::load_snapshot(&segments_path).unwrap();
        assert!(segments.iter().all(|segment| {
            segment.confidence == TranscriptSegment::UNKNOWN_CONFIDENCE
                && serde_json::to_value(segment).unwrap()["confidence"].is_null()
        }));
        assert_eq!(first_segment_ms.is_some(), !segments.is_empty());
        if case.id.starts_with("silence-") {
            assert!(segments.is_empty(), "digital silence must stay empty");
        }
        measurements.push(Measurement {
            id: case.id,
            first_segment_ms,
            finalization_ms,
            segments,
        });
    }
    assert!(state.recovery.failure_page(0).await.unwrap().is_empty());
    assert_eq!(sha256(&manifest_path), manifest_hash);
    assert_eq!(sha256(&model_path), descriptor.sha256);
    for (path, expected) in fixture_hashes {
        assert_eq!(sha256(&path), expected, "input changed during replay");
    }
    let report = serde_json::json!({
        "schema_version": 1,
        "manifest_sha256": manifest_hash,
        "model_sha256": descriptor.sha256,
        "settings": {
            "model": "base.en", "language": "en", "subspeaker_enabled": false,
            "video_ocr_enabled": false, "cross_meeting_linking_auto": false,
        },
        "scope": "Post-recording mix, cache verification, final ASR, Me/Them attribution, transcript persistence and completion receipt. First case includes cold cache verification. No device shutdown, live preview, remote-speaker clustering, summary or model download.",
        "cases": measurements,
    });
    std::fs::write(output_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
}
