//! Offline evaluation of the production WAV decoder and Whisper wrapper.

#[path = "../../../bruhs-capture/src/transcribe/decode.rs"]
pub mod decode;
#[path = "../../../bruhs-capture/src/diarize.rs"]
pub mod diarize;
mod legacy_whisper;
#[path = "../../../bruhs-capture/src/transcribe/whisper.rs"]
pub mod whisper;

use bruhs_models::num::ToFloatLossy;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Instant};

#[derive(Deserialize)]
struct Manifest {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    audio: PathBuf,
    language: Option<String>,
    mic_audio: PathBuf,
    system_audio: PathBuf,
}

#[derive(Serialize)]
struct Output {
    cases: Vec<CaseOutput>,
}

#[derive(Serialize)]
struct CaseOutput {
    id: String,
    text: String,
    audio_ms: f64,
    decode_ms: f64,
    inference_ms: f64,
    first_segment_ms: Option<f64>,
    segments: Vec<SegmentOutput>,
}

#[derive(Serialize)]
struct SegmentOutput {
    start_ms: u32,
    end_ms: u32,
    text: String,
    speaker_id: Option<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let model = PathBuf::from(args.next().ok_or("expected MODEL_PATH MANIFEST_PATH")?);
    let manifest_path = PathBuf::from(args.next().ok_or("expected MANIFEST_PATH")?);
    let language = args
        .next()
        .map(|value| value.to_string_lossy().into_owned());
    let variant = args.next().unwrap_or_else(|| "candidate".into());
    if variant != "candidate" && variant != "legacy" {
        return Err("VARIANT must be candidate or legacy".into());
    }
    if args.next().is_some() {
        return Err("expected MODEL_PATH MANIFEST_PATH [MODEL_LANGUAGE] [VARIANT]".into());
    }
    let root = manifest_path.parent().ok_or("manifest has no parent")?;
    let manifest: Manifest = serde_json::from_reader(std::fs::File::open(&manifest_path)?)?;
    let mut outputs = Vec::with_capacity(manifest.cases.len());
    for case in manifest.cases {
        let decode_started = Instant::now();
        let audio = decode::decode_wav_to_whisper_input(&root.join(&case.audio))?;
        let decode_ms = decode_started.elapsed().as_secs_f64() * 1000.0;
        let started = Instant::now();
        let mut first_segment_ms = None;
        let selected_language = language.as_deref().or(case.language.as_deref());
        let selected_language = selected_language.filter(|language| *language != "auto");
        let on_segment = |_: &whisper::DecodedSegment| {
            first_segment_ms.get_or_insert_with(|| started.elapsed().as_secs_f64() * 1000.0);
        };
        let decoded = if variant == "legacy" {
            legacy_whisper::transcribe_samples(&model, &audio, selected_language, on_segment)?
        } else {
            whisper::transcribe_samples(&model, &audio, selected_language, on_segment)?
        };
        let inference_ms = started.elapsed().as_secs_f64() * 1000.0;
        let mut segments: Vec<_> = decoded
            .into_iter()
            .enumerate()
            .map(|(i, s)| bruhs_models::TranscriptSegment {
                id: format!("{}-{i}", case.id),
                meeting_id: "synthetic-evaluation".into(),
                speaker_id: None,
                start_ms: s.start_ms,
                end_ms: s.end_ms,
                text: s.text,
                confidence: bruhs_models::TranscriptSegment::UNKNOWN_CONFIDENCE,
                partial: false,
                speaker_confidence: None,
            })
            .collect();
        let mic = diarize::compute_envelope(&root.join(&case.mic_audio))?;
        let system = diarize::compute_envelope(&root.join(&case.system_audio))?;
        diarize::assign_speakers(&mut segments, &mic, Some(&system));
        outputs.push(CaseOutput {
            id: case.id,
            text: segments
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            audio_ms: audio.len().to_f64_lossy() / 16.0,
            decode_ms,
            inference_ms,
            first_segment_ms,
            segments: segments
                .into_iter()
                .map(|s| SegmentOutput {
                    start_ms: s.start_ms,
                    end_ms: s.end_ms,
                    text: s.text,
                    speaker_id: s.speaker_id,
                })
                .collect(),
        });
    }
    serde_json::to_writer(std::io::stdout(), &Output { cases: outputs })?;
    Ok(())
}
