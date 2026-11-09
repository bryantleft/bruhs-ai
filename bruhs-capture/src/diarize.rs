//! Channel-based "Me vs Them" diarization. Reads mic.wav + sys.wav,
//! computes a per-channel energy envelope, and labels each transcript
//! segment based on which channel was active during that segment's
//! time range.

use bruhs_models::num::ToFloatLossy;
use bruhs_models::TranscriptSegment;
use hound::{SampleFormat, WavReader};
use std::path::Path;

pub const SPEAKER_ID_ME: &str = "me";
pub const SPEAKER_ID_THEM: &str = "them";

/// Per-frame RMS threshold. Below this counts as silence.
/// 1500 in i16 = ~-26dBFS, picks up normal speech, rejects mic hiss.
const ENERGY_THRESHOLD: f32 = 1500.0;
/// Window size for the RMS calculation, in milliseconds.
const WINDOW_MS: u64 = 25;

#[derive(Debug, thiserror::Error)]
pub enum DiarizeError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("hound: {0}")]
    Hound(#[from] hound::Error),
    #[error("unsupported wav format: {0}")]
    UnsupportedFormat(String),
}

/// Time-windowed activity envelope: each entry is (window_start_ms, is_active).
pub struct Envelope {
    pub window_ms: u64,
    pub active: Vec<bool>,
}

impl Envelope {
    /// Fraction of the [start_ms, end_ms) range covered by `active = true` windows.
    pub fn active_fraction(&self, start_ms: u64, end_ms: u64) -> f32 {
        let first = usize::try_from(start_ms / self.window_ms).unwrap_or(usize::MAX);
        let last = usize::try_from(end_ms.saturating_sub(1) / self.window_ms).unwrap_or(usize::MAX);
        if last < first || self.active.is_empty() {
            return 0.0;
        }
        let lo = first.min(self.active.len() - 1);
        let hi = (last + 1).min(self.active.len());
        let slice = &self.active[lo..hi];
        if slice.is_empty() {
            return 0.0;
        }
        let on = slice.iter().filter(|a| **a).count().to_f32_lossy();
        on / slice.len().to_f32_lossy()
    }
}

/// Build an activity envelope for `wav_path`. The WAV must be 16-bit
/// mono PCM (matching the recorder outputs).
pub fn compute_envelope(wav_path: &Path) -> Result<Envelope, DiarizeError> {
    let mut reader = WavReader::open(wav_path)?;
    let spec = reader.spec();
    if spec.bits_per_sample != 16 || spec.sample_format != SampleFormat::Int {
        return Err(DiarizeError::UnsupportedFormat(format!("{spec:?}")));
    }
    let channels = spec.channels as usize;
    let sample_rate = u64::from(spec.sample_rate);
    let samples_per_window =
        usize::try_from(sample_rate * WINDOW_MS / 1000).unwrap_or(usize::MAX) * channels;

    let mut active = Vec::new();
    let mut buf: Vec<i32> = Vec::with_capacity(samples_per_window);
    for sample in reader.samples::<i16>() {
        let s = i32::from(sample?);
        buf.push(s);
        if buf.len() == samples_per_window {
            let sum_sq: i64 = buf.iter().map(|&s| i64::from(s) * i64::from(s)).sum();
            let rms = (sum_sq.to_f32_lossy() / buf.len().to_f32_lossy()).sqrt();
            active.push(rms >= ENERGY_THRESHOLD);
            buf.clear();
        }
    }
    if !buf.is_empty() {
        let sum_sq: i64 = buf.iter().map(|&s| i64::from(s) * i64::from(s)).sum();
        let rms = (sum_sq.to_f32_lossy() / buf.len().to_f32_lossy()).sqrt();
        active.push(rms >= ENERGY_THRESHOLD);
    }
    Ok(Envelope {
        window_ms: WINDOW_MS,
        active,
    })
}

/// Pure-logic predicate: given mic + sys activity fractions for one
/// segment, decide who spoke. Tied/empty cases default to Me (mic is
/// always the user; sys-only flips to Them when the system audio is
/// meaningfully louder).
pub fn assign_speaker(mic_frac: f32, sys_frac: f32) -> &'static str {
    // Require sys to clearly dominate; small leakage from speakers into
    // the mic shouldn't flip the label.
    if sys_frac > mic_frac + 0.15 && sys_frac > 0.2 {
        SPEAKER_ID_THEM
    } else {
        SPEAKER_ID_ME
    }
}

/// Walk `segments`, populate `speaker_id` in-place by comparing mic +
/// sys envelopes over each segment's time range.
pub fn assign_speakers(
    segments: &mut [TranscriptSegment],
    mic_env: &Envelope,
    sys_env: Option<&Envelope>,
) {
    for seg in segments.iter_mut() {
        let start_ms = u64::from(seg.start_ms);
        let end_ms = u64::from(seg.end_ms);
        let mic_frac = mic_env.active_fraction(start_ms, end_ms);
        let sys_frac = sys_env.map_or(0.0, |e| e.active_fraction(start_ms, end_ms));
        let id = assign_speaker(mic_frac, sys_frac);
        seg.speaker_id = Some(id.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_from(active: Vec<bool>) -> Envelope {
        Envelope {
            window_ms: WINDOW_MS,
            active,
        }
    }

    fn make_segment(id: &str, start_ms: u32, end_ms: u32, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            id: id.into(),
            meeting_id: "m-1".into(),
            start_ms,
            end_ms,
            text: text.into(),
            speaker_id: None,
            confidence: 1.0,
            partial: false,
            speaker_confidence: None,
        }
    }

    #[test]
    fn active_fraction_full_window() {
        let env = env_from(vec![true; 10]);
        // 10 windows of 25ms = 250ms total.
        assert!((env.active_fraction(0, 250) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn active_fraction_half_active() {
        let env = env_from(vec![true, false, true, false]);
        // 4 windows, 2 active = 0.5.
        assert!((env.active_fraction(0, 100) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn assign_speaker_prefers_me_when_mic_dominates() {
        assert_eq!(assign_speaker(0.9, 0.1), SPEAKER_ID_ME);
    }

    #[test]
    fn assign_speaker_picks_them_when_sys_clearly_dominates() {
        assert_eq!(assign_speaker(0.05, 0.85), SPEAKER_ID_THEM);
    }

    #[test]
    fn assign_speaker_defaults_to_me_when_close() {
        // 0.4 vs 0.5 — both active, no clear winner — default Me.
        assert_eq!(assign_speaker(0.4, 0.5), SPEAKER_ID_ME);
    }

    #[test]
    fn assign_speaker_defaults_to_me_when_sys_weak() {
        // Sys passes the +0.15 gap but its absolute activity is too low.
        assert_eq!(assign_speaker(0.0, 0.18), SPEAKER_ID_ME);
    }

    #[test]
    fn assign_speakers_writes_speaker_id_per_segment() {
        let mut segments = vec![
            make_segment("s-1", 0, 100, "hi"),      // 4 windows (0-99ms)
            make_segment("s-2", 100, 200, "hello"), // 4 windows (100-199ms)
        ];
        let mic = env_from(vec![true, true, true, true, false, false, false, false]);
        let sys = env_from(vec![false, false, false, false, true, true, true, true]);
        assign_speakers(&mut segments, &mic, Some(&sys));
        assert_eq!(segments[0].speaker_id.as_deref(), Some(SPEAKER_ID_ME));
        assert_eq!(segments[1].speaker_id.as_deref(), Some(SPEAKER_ID_THEM));
    }

    #[test]
    fn assign_speakers_handles_missing_sys() {
        let mut segments = vec![make_segment("s-1", 0, 100, "solo")];
        let mic = env_from(vec![true; 4]);
        assign_speakers(&mut segments, &mic, None);
        assert_eq!(segments[0].speaker_id.as_deref(), Some(SPEAKER_ID_ME));
    }
}
