//! Transcription pipeline. `transcribe_wav` loads a recorded meeting,
//! runs Whisper, and emits `MeetingEvent::Segment` for each result.

pub mod decode;
pub mod remote;
mod segment_sink;
pub mod segments_log;
pub mod whisper;

use bruhs_models::TranscriptSegment;
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

use crate::state::CaptureState;

#[derive(Debug, thiserror::Error)]
pub enum TranscribeError {
    #[error("decode: {0}")]
    Decode(#[source] decode::DecodeError),
    #[error("whisper: {0}")]
    Whisper(#[source] whisper::WhisperError),
    #[error("log: {0}")]
    Log(#[source] segments_log::LogError),
    #[error("io: {0}")]
    Io(#[source] std::io::Error),
    #[error("remote: {0}")]
    Remote(#[source] remote::RemoteError),
}

#[derive(Debug, Clone)]
pub enum TranscriptionJob {
    Whisper(bruhs_models::WhisperModel),
    Compatible(remote::RemoteJob),
}

/// Transcribe `wav_path` to JSONL alongside it. Broadcasts a
/// `MeetingEvent::Segment` for each Whisper segment as it's produced.
///
/// Blocking: this fn calls into whisper.cpp which is CPU-heavy and can
/// take seconds-to-minutes. Call from a `tokio::task::spawn_blocking`
/// with the model path pre-resolved (see `ModelStore::ensure_whisper`).
pub fn transcribe_wav(
    state: Arc<CaptureState>,
    meeting_id: String,
    wav_path: &Path,
    model_path: &Path,
    model: bruhs_models::WhisperModel,
) -> Result<(), TranscribeError> {
    let jsonl_path = state
        .data
        .segments_jsonl_path(&meeting_id)
        .map_err(TranscribeError::Io)?;
    state.data.ensure_audio_dir().map_err(TranscribeError::Io)?;
    let samples = decode::decode_wav_to_whisper_input(wav_path).map_err(TranscribeError::Decode)?;

    // English-only models reject auto-detect; multilingual models like
    // large-v3-turbo want None so Whisper picks the spoken language.
    let language = if model.descriptor().multilingual {
        None
    } else {
        Some("en")
    };
    let mut sink = segment_sink::SegmentSink::new(&state, &jsonl_path);
    let transcription = whisper::transcribe_samples(model_path, &samples, language, |seg| {
        let segment = TranscriptSegment {
            id: Uuid::new_v4().to_string(),
            meeting_id: meeting_id.clone(),
            speaker_id: None,
            start_ms: seg.start_ms,
            end_ms: seg.end_ms,
            text: seg.text.clone(),
            confidence: 1.0,
            partial: false,
            speaker_confidence: None,
        };
        sink.push(segment);
    });
    // Report the first save failure even if inference also failed afterwards.
    sink.finish().map_err(TranscribeError::Log)?;
    transcription.map_err(TranscribeError::Whisper)?;

    Ok(())
}
