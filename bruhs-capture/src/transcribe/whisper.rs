//! Whisper inference wrapper.
//!
//! Two APIs:
//! - [`transcribe_samples`] — loads the model from disk each call. Used by
//!   the post-stop pipeline where per-meeting context reuse isn't needed.
//! - [`PersistentContext`] — loads the model once; reuses the
//!   `WhisperContext` across calls. Used by the live-transcription tick loop
//!   to avoid ~500 ms model-load overhead on every 5-second tick.

use std::path::Path;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

#[derive(Debug, thiserror::Error)]
pub enum WhisperError {
    #[error("whisper init: {0}")]
    Init(String),
    #[error("whisper inference: {0}")]
    Inference(String),
}

#[derive(Debug, Clone)]
pub struct DecodedSegment {
    /// Start time in milliseconds from the beginning of the audio.
    pub start_ms: u32,
    /// End time in milliseconds.
    pub end_ms: u32,
    pub text: String,
}

/// Run Whisper inference on a slice of f32 samples (mono, 16 kHz). Calls
/// `on_segment` for each segment as it's produced.
///
/// `language`: pass `Some("en")` to force English (most ggml-*.en models
/// require this — they reject auto-detect). For multilingual models like
/// large-v3-turbo, pass `None` to let Whisper detect the spoken language.
pub fn transcribe_samples<F>(
    model_path: &Path,
    samples: &[f32],
    language: Option<&str>,
    mut on_segment: F,
) -> Result<Vec<DecodedSegment>, WhisperError>
where
    F: FnMut(&DecodedSegment),
{
    let ctx = WhisperContext::new_with_params(
        &model_path.to_string_lossy(),
        WhisperContextParameters::default(),
    )
    .map_err(|e| WhisperError::Init(format!("{e}")))?;
    let mut state = ctx
        .create_state()
        .map_err(|e| WhisperError::Init(format!("create_state: {e}")))?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_translate(false);
    params.set_language(language);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);

    state
        .full(params, samples)
        .map_err(|e| WhisperError::Inference(format!("{e}")))?;

    let n = state
        .full_n_segments()
        .map_err(|e| WhisperError::Inference(format!("n_segments: {e}")))?;
    let mut out = Vec::with_capacity(usize::try_from(n).unwrap_or(0));
    for i in 0..n {
        let text = state
            .full_get_segment_text(i)
            .map_err(|e| WhisperError::Inference(format!("get_segment_text({i}): {e}")))?;
        // whisper-rs reports timestamps in centiseconds (1/100 s).
        let t0 = state
            .full_get_segment_t0(i)
            .map_err(|e| WhisperError::Inference(format!("get_segment_t0({i}): {e}")))?;
        let t1 = state
            .full_get_segment_t1(i)
            .map_err(|e| WhisperError::Inference(format!("get_segment_t1({i}): {e}")))?;
        let segment = DecodedSegment {
            start_ms: centiseconds_to_ms(t0),
            end_ms: centiseconds_to_ms(t1),
            text: text.trim().to_string(),
        };
        on_segment(&segment);
        out.push(segment);
    }
    Ok(out)
}

/// A loaded Whisper model that persists across inference calls.
///
/// Each `LiveTranscriber` instance owns one `PersistentContext` on its
/// dedicated thread. Do **not** share across threads — `WhisperContext` is
/// not `Sync`.
pub struct PersistentContext {
    ctx: WhisperContext,
}

impl PersistentContext {
    /// Load the model from `model_path`. Takes ~150–300 ms on first call
    /// (Metal init). Subsequent inference via [`Self::transcribe`] skips
    /// this overhead.
    pub fn new(model_path: &Path) -> Result<Self, WhisperError> {
        let ctx = WhisperContext::new_with_params(
            &model_path.to_string_lossy(),
            WhisperContextParameters::default(),
        )
        .map_err(|e| WhisperError::Init(format!("{e}")))?;
        Ok(Self { ctx })
    }

    /// Run Whisper inference on `samples` (mono, 16 kHz f32). Calls
    /// `on_segment` for each decoded segment. Returns all segments.
    ///
    /// The `WhisperContext` is reused from the prior call — no model reload.
    pub fn transcribe<F>(
        &self,
        samples: &[f32],
        language: Option<&str>,
        mut on_segment: F,
    ) -> Result<Vec<DecodedSegment>, WhisperError>
    where
        F: FnMut(&DecodedSegment),
    {
        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| WhisperError::Init(format!("create_state: {e}")))?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_translate(false);
        params.set_language(language);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        state
            .full(params, samples)
            .map_err(|e| WhisperError::Inference(format!("{e}")))?;

        let n = state
            .full_n_segments()
            .map_err(|e| WhisperError::Inference(format!("n_segments: {e}")))?;
        let mut out = Vec::with_capacity(usize::try_from(n).unwrap_or(0));
        for i in 0..n {
            let text = state
                .full_get_segment_text(i)
                .map_err(|e| WhisperError::Inference(format!("get_segment_text({i}): {e}")))?;
            // whisper-rs reports timestamps in centiseconds (1/100 s).
            let t0 = state
                .full_get_segment_t0(i)
                .map_err(|e| WhisperError::Inference(format!("get_segment_t0({i}): {e}")))?;
            let t1 = state
                .full_get_segment_t1(i)
                .map_err(|e| WhisperError::Inference(format!("get_segment_t1({i}): {e}")))?;
            let segment = DecodedSegment {
                start_ms: centiseconds_to_ms(t0),
                end_ms: centiseconds_to_ms(t1),
                text: text.trim().to_string(),
            };
            on_segment(&segment);
            out.push(segment);
        }
        Ok(out)
    }
}

/// whisper-rs reports segment times in centiseconds; out-of-range values
/// clamp (negative to 0) instead of wrapping.
fn centiseconds_to_ms(centiseconds: i64) -> u32 {
    let ms = centiseconds.saturating_mul(10);
    u32::try_from(ms).unwrap_or(if ms < 0 { 0 } else { u32::MAX })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centiseconds_become_milliseconds_and_clamp() {
        assert_eq!(centiseconds_to_ms(0), 0);
        assert_eq!(centiseconds_to_ms(123), 1_230);
        assert_eq!(centiseconds_to_ms(-5), 0);
        assert_eq!(centiseconds_to_ms(i64::from(u32::MAX)), u32::MAX);
        assert_eq!(centiseconds_to_ms(i64::MAX), u32::MAX);
    }

    #[test]
    fn whisper_error_display() {
        let e = WhisperError::Init("model not found".into());
        assert!(format!("{e}").contains("whisper init"));
    }
}
