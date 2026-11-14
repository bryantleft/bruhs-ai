//! Frozen final-pass control from a6f72da6, before the digital-silence guard.
//! Used only by this offline benchmark; never linked into bruhs-capture.
//! Original source SHA-256: 07de68a783fa81901b233f2eebbe4ad02484c19d12217375c5f608f39e351c87

use crate::whisper::{DecodedSegment, WhisperError};
use std::path::Path;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

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

fn centiseconds_to_ms(centiseconds: i64) -> u32 {
    let ms = centiseconds.saturating_mul(10);
    u32::try_from(ms).unwrap_or(if ms < 0 { 0 } else { u32::MAX })
}
