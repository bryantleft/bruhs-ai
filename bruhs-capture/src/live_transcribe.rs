//! Background live transcription. Loops every TICK_INTERVAL polling a WAV
//! file for new audio, runs Whisper on the tail, emits
//! `MeetingEvent::Segment { partial: true }` for new segments.
//!
//! The Whisper model is loaded once per `LiveTranscriber` instance via
//! [`whisper::PersistentContext`] and reused across ticks (~500 ms saved
//! per tick vs. reloading from disk each time).

use bruhs_models::num::{ToFloatLossy, ToIntSaturating};
use bruhs_models::{MeetingEvent, TranscriptSegment};
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;
use uuid::Uuid;

use crate::state::CaptureState;
use crate::transcribe::{decode, whisper};

/// How often to wake up and check for new audio. Should be small
/// enough that latency feels live, large enough that whisper has
/// time to finish.
const TICK_INTERVAL: Duration = Duration::from_secs(5);

/// How many seconds of audio to feed whisper per tick. The window
/// extends backwards from current-end-of-file. Larger windows give
/// whisper more context (better accuracy at boundaries) but waste
/// compute re-processing already-emitted material.
const WINDOW_SECS: f64 = 10.0;

/// Backlap before the watermark on each tick so whisper sees ~1s of
/// prior context. Output filtered against watermark.
const WATERMARK_BACKLAP_MS: u32 = 1000;

#[derive(Debug, thiserror::Error)]
pub enum LiveError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("live-transcribe thread panicked")]
    Panicked,
}

/// A position in the source WAV, counted in mono frames at the file's own
/// sample rate (often 48 kHz for system audio).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SourceSamples {
    count: u64,
    rate: u32,
}

impl SourceSamples {
    fn to_ms(self) -> u64 {
        self.count * 1000 / u64::from(self.rate.max(1))
    }

    /// One second earlier (clamped at the start of the file).
    fn back_one_second(self) -> Self {
        Self {
            count: self.count.saturating_sub(u64::from(self.rate)),
            rate: self.rate,
        }
    }
}

/// A count of decoder-output samples, which are always 16 kHz.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Samples16k(u64);

impl Samples16k {
    fn to_ms(self) -> u64 {
        self.0 * 1000 / 16_000
    }
}

/// Absolute start of the transcribed tail: the seek position in the source
/// file plus how far into the (16 kHz) decoded audio the tail starts. The two
/// counts are in different units and are converted separately.
fn tail_offset_ms(seek: SourceSamples, tail_start: Samples16k) -> u32 {
    u32::try_from(seek.to_ms() + tail_start.to_ms()).unwrap_or(u32::MAX)
}

pub struct LiveTranscriber {
    stop_tx: mpsc::Sender<()>,
    join: Option<JoinHandle<Result<(), LiveError>>>,
}

impl LiveTranscriber {
    /// Spawn the live-transcription thread. Returns immediately.
    ///
    /// - `source_wav`: WAV file to poll (mic.wav or sys.wav).
    /// - `speaker_id`: label attached to every emitted [`TranscriptSegment`].
    ///   Pass `"me"` for the microphone channel, `"them"` for system audio.
    pub fn start(
        source_wav: PathBuf,
        model_path: PathBuf,
        model: bruhs_models::WhisperModel,
        speaker_id: &'static str,
        state: Arc<CaptureState>,
        meeting_id: String,
    ) -> Result<Self, LiveError> {
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let join = std::thread::Builder::new()
            .name(format!("bruhs-live-transcribe-{speaker_id}"))
            .spawn(move || {
                run(
                    source_wav, model_path, model, speaker_id, state, meeting_id, stop_rx,
                )
            })?;
        Ok(Self {
            stop_tx,
            join: Some(join),
        })
    }

    /// Signal the thread to exit and wait for it. Errors during
    /// transcription are logged inside the thread; this returns the
    /// thread-join result.
    pub fn stop(mut self) -> Result<(), LiveError> {
        // A send error means the thread already exited; the join reports how.
        let _ = self.stop_tx.send(());
        if let Some(handle) = self.join.take() {
            handle.join().map_err(|_| LiveError::Panicked)?
        } else {
            Ok(())
        }
    }
}

/// Returns true if the energy in `samples` is above the speech
/// threshold (reused from diarize's primitive — pure ƒ32 RMS).
fn has_speech(samples: &[f32]) -> bool {
    if samples.is_empty() {
        return false;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    let rms = (sum_sq / samples.len().to_f32_lossy()).sqrt();
    // f32 normalized samples (-1..1). 0.01 RMS ≈ -40 dBFS — picks
    // up normal mic speech, rejects mic hiss.
    rms >= 0.01
}

fn run(
    source_wav: PathBuf,
    model_path: PathBuf,
    model: bruhs_models::WhisperModel,
    speaker_id: &'static str,
    state: Arc<CaptureState>,
    meeting_id: String,
    stop_rx: mpsc::Receiver<()>,
) -> Result<(), LiveError> {
    let language = if model.descriptor().multilingual {
        None
    } else {
        Some("en")
    };

    // Load the Whisper model once for this thread's lifetime. Subsequent
    // ticks call ctx.transcribe() which reuses the resident context
    // (~500 ms saved per tick vs. loading from disk each time).
    let ctx = match whisper::PersistentContext::new(&model_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("live whisper context init failed ({speaker_id}): {e}");
            return Ok(());
        }
    };

    let mut watermark_ms: u32 = 0;
    // How many source-rate mono samples have already been decoded, so the
    // next tick seeks past them (no full re-read). The rate is read from the
    // WAV spec on first decode and cached; None means the file has not been
    // opened successfully yet.
    let mut last_source_offset: u64 = 0;
    let mut source_rate: Option<u32> = None;

    loop {
        match stop_rx.recv_timeout(TICK_INTERVAL) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }

        // Skip if file doesn't exist yet (recorder still warming, or sys
        // audio capture failed — sys.wav may never appear).
        if !source_wav.exists() {
            continue;
        }

        // Read the actual sample rate from the WAV spec on first decode
        // and cache it. Fallback to 16 kHz only if hound can't open the file
        // (handled below by the decode error path).
        let current_rate = source_rate.unwrap_or_else(|| {
            hound::WavReader::open(&source_wav).map_or(16_000, |r| r.spec().sample_rate)
        });

        // Seek-based decode: only read new audio since last tick, with a
        // 1-s backlap so Whisper has context at segment boundaries.
        let seek = SourceSamples {
            count: last_source_offset,
            rate: current_rate,
        }
        .back_one_second();
        let seek_frame = u32::try_from(seek.count).unwrap_or(u32::MAX);

        let (new_samples, new_offset) =
            match decode::decode_wav_from_sample(&source_wav, seek_frame) {
                Ok(pair) => pair,
                Err(e) => {
                    tracing::warn!("live decode failed ({speaker_id}): {e}");
                    continue;
                }
            };

        // Cache the source rate after first successful decode (it's stable
        // for the lifetime of a WAV file).
        if source_rate.is_none() {
            source_rate = Some(current_rate);
        }

        // Keep at most WINDOW_SECS of the newly-decoded audio so Whisper
        // doesn't see a multi-minute chunk on the first tick after a pause.
        // The window is in resampled 16 kHz samples (decode output is always 16 kHz).
        let window_samples = (WINDOW_SECS * 16_000.0).to_usize_saturating();
        let tail_start = new_samples.len().saturating_sub(window_samples);
        let tail = &new_samples[tail_start..];
        // Absolute position of the tail's first sample: `seek` is in source
        // samples, `tail_start` in decoded 16 kHz samples.
        let tail_offset_ms = tail_offset_ms(seek, Samples16k(tail_start as u64));

        // Update watermark to the furthest decoded point (source samples).
        last_source_offset = u64::from(new_offset);

        // VAD skip: no speech in the tail → no point running whisper.
        if !has_speech(tail) {
            continue;
        }

        let mid = meeting_id.clone();
        let state_inner = state.clone();
        let watermark_for_this_tick = watermark_ms;
        let mut new_max_end_ms = watermark_ms;

        // ctx.transcribe() reuses the resident WhisperContext — no reload.
        if let Err(e) = ctx.transcribe(tail, language, |seg| {
            // seg.start_ms / end_ms are relative to the tail; shift.
            let absolute_start = tail_offset_ms.saturating_add(seg.start_ms);
            let absolute_end = tail_offset_ms.saturating_add(seg.end_ms);

            // Drop anything fully behind the watermark (already emitted).
            // Backlap keeps ~1s of context for whisper without re-emitting.
            if absolute_end + WATERMARK_BACKLAP_MS <= watermark_for_this_tick {
                return;
            }

            let segment = TranscriptSegment {
                id: Uuid::new_v4().to_string(),
                meeting_id: mid.clone(),
                speaker_id: Some(speaker_id.into()),
                start_ms: absolute_start,
                end_ms: absolute_end,
                text: seg.text.clone(),
                confidence: 1.0,
                partial: true,
                speaker_confidence: None,
            };
            new_max_end_ms = new_max_end_ms.max(absolute_end);
            state_inner.emit(MeetingEvent::Segment {
                meeting_id: mid.clone(),
                segment,
            });
        }) {
            tracing::warn!("live whisper failed ({speaker_id}): {e}");
            continue;
        }

        watermark_ms = new_max_end_ms;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_speech_silence_is_false() {
        let samples = vec![0.0f32; 16_000];
        assert!(!has_speech(&samples));
    }

    #[test]
    fn has_speech_loud_is_true() {
        let samples: Vec<f32> = (0..16_000)
            .map(|i| if i % 2 == 0 { 0.1 } else { -0.1 })
            .collect();
        assert!(has_speech(&samples));
    }

    #[test]
    fn has_speech_empty_is_false() {
        assert!(!has_speech(&[]));
    }

    /// H14: the offset used to add 48 kHz source samples to 16 kHz decoded
    /// samples and divide the sum by 16 000, so live timestamps on system
    /// audio ran about 3x too fast.
    #[test]
    fn tail_offset_converts_each_unit_separately_for_a_48k_source() {
        let seek = SourceSamples {
            count: 10 * 48_000,
            rate: 48_000,
        };
        // The tail starts 1 s into the decoded (16 kHz) audio.
        assert_eq!(tail_offset_ms(seek, Samples16k(16_000)), 11_000);
        assert_eq!(tail_offset_ms(seek, Samples16k(0)), 10_000);
    }

    #[test]
    fn tail_offset_for_a_16k_source_matches_the_old_formula() {
        let seek = SourceSamples {
            count: 80_000,
            rate: 16_000,
        };
        assert_eq!(
            tail_offset_ms(seek, Samples16k(8_000)),
            ((80_000 + 8_000) * 1000 / 16_000) as u32
        );
    }

    #[test]
    fn backlap_is_one_second_at_the_source_rate() {
        let at = |count| SourceSamples {
            count,
            rate: 48_000,
        };
        assert_eq!(at(96_000).back_one_second(), at(48_000));
        assert_eq!(at(10).back_one_second(), at(0));
    }
}
