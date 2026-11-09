//! POST /enroll/{person_id} — voice enrollment endpoint.
//!
//! Accepts a raw WAV body (16kHz mono 16-bit PCM, produced by the web
//! frontend via AudioContext → WAV encoding). Extracts a sherpa-onnx
//! speaker embedding and returns it for the server to store on the
//! PersonProfile.
//!
//! Tolerant to slight format variation: resamples to 16kHz if needed.

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use bruhs_models::num::ToFloatLossy;
use serde::Serialize;
use std::sync::Arc;

use crate::state::CaptureState;

#[derive(Debug, Serialize)]
pub struct EnrollResponse {
    pub embedding: Vec<f32>,
}

/// POST /enroll/{person_id}
///
/// Body: raw WAV bytes (16kHz mono 16-bit PCM preferred; other rates are
/// resampled). Person ID is ignored by the daemon — it's forwarded by the
/// server which handles the DB upsert.
pub async fn enroll_voice(
    Path(_person_id): Path<String>,
    State(state): State<Arc<CaptureState>>,
    body: Bytes,
) -> Result<Json<EnrollResponse>, (StatusCode, String)> {
    #[cfg(target_os = "macos")]
    {
        // 1. Ensure the sherpa speaker model is available.
        let model_path = state
            .models
            .ensure_sherpa_speaker(|d, t| {
                tracing::info!(
                    "enroll: sherpa model download {d} / {} bytes",
                    t.map_or_else(|| "?".into(), |t| t.to_string())
                );
            })
            .await
            .map_err(|e| {
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    format!("sherpa model unavailable: {e}"),
                )
            })?;

        // 2. Decode, resample and run the model off the async workers.
        let extractor = Arc::clone(&state.speaker_extractor);
        let embedding = tokio::task::spawn_blocking(move || {
            let samples = decode_wav_bytes_to_f32_mono_16k(&body)?;
            if samples.is_empty() {
                return Err(EnrollError::Empty);
            }
            extractor.with(&model_path, load_extractor, |extractor| {
                compute_embedding(extractor, samples)
            })?
        })
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("embedding worker: {e}"),
            )
        })?
        .map_err(|e| (e.status(), e.to_string()))?;

        Ok(Json(EnrollResponse { embedding }))
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (state, body);
        Err((
            StatusCode::NOT_IMPLEMENTED,
            "voice enrollment is only supported on macOS".into(),
        ))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EnrollError {
    #[error("wav decode: {0}")]
    Decode(#[from] hound::Error),
    #[error("wav decode: unsupported WAV format: {format:?} {bits}-bit")]
    UnsupportedFormat {
        format: hound::SampleFormat,
        bits: u16,
    },
    #[error("wav decode: resample: {0}")]
    Resample(String),
    #[error("empty audio after decode")]
    Empty,
    #[error("embedding: {0}")]
    Embedding(String),
}

impl EnrollError {
    fn status(&self) -> StatusCode {
        match self {
            Self::Decode(_) | Self::UnsupportedFormat { .. } | Self::Empty => {
                StatusCode::BAD_REQUEST
            }
            Self::Resample(_) | Self::Embedding(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// A value loaded on first use and kept for later calls: the speaker
/// embedding extractor takes a model load per construction. Only used from
/// blocking threads, so a std mutex is fine.
pub struct LoadOnce<T> {
    slot: std::sync::Mutex<Option<(std::path::PathBuf, T)>>,
}

impl<T> Default for LoadOnce<T> {
    fn default() -> Self {
        Self {
            slot: std::sync::Mutex::new(None),
        }
    }
}

impl<T> LoadOnce<T> {
    /// Run `use_value` on the value for `path`, loading it first if absent
    /// (or if it was loaded from another path). A failed load is retried on
    /// the next call.
    pub fn with<R, E>(
        &self,
        path: &std::path::Path,
        load: impl FnOnce(&std::path::Path) -> Result<T, E>,
        use_value: impl FnOnce(&mut T) -> R,
    ) -> Result<R, E> {
        let mut slot = self
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = match slot.take() {
            Some((loaded, value)) if loaded == path => slot.insert((loaded, value)),
            _ => slot.insert((path.to_path_buf(), load(path)?)),
        };
        let result = use_value(&mut entry.1);
        drop(slot);
        Ok(result)
    }
}

// ── helpers (macOS-only) ────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
const TARGET_SAMPLE_RATE: u32 = 16_000;

/// Decode a WAV body (as bytes) to f32 mono samples at 16kHz.
/// Resamples if the input sample rate differs from 16kHz.
#[cfg(target_os = "macos")]
fn decode_wav_bytes_to_f32_mono_16k(bytes: &[u8]) -> Result<Vec<f32>, EnrollError> {
    use hound::{SampleFormat, WavReader};
    use rubato::{FftFixedIn, Resampler};
    use std::io::Cursor;

    let cursor = Cursor::new(bytes);
    let mut reader = WavReader::new(cursor)?;
    let spec = reader.spec();

    let channels = spec.channels as usize;
    let mut mono: Vec<f32> = Vec::with_capacity(reader.len() as usize / channels.max(1));

    match (spec.sample_format, spec.bits_per_sample) {
        (SampleFormat::Int, 16) => {
            let mut chan_buf = vec![0i32; channels];
            let mut chan_idx = 0usize;
            for sample in reader.samples::<i16>() {
                let s = i32::from(sample?);
                chan_buf[chan_idx] = s;
                chan_idx += 1;
                if chan_idx == channels {
                    let sum: i32 = chan_buf.iter().sum();
                    mono.push(sum.to_f32_lossy() / channels.to_f32_lossy() / f32::from(i16::MAX));
                    chan_idx = 0;
                }
            }
        }
        (SampleFormat::Float, 32) => {
            let mut chan_buf = vec![0.0f32; channels];
            let mut chan_idx = 0usize;
            for sample in reader.samples::<f32>() {
                let s = sample?;
                chan_buf[chan_idx] = s;
                chan_idx += 1;
                if chan_idx == channels {
                    let sum: f32 = chan_buf.iter().sum();
                    mono.push(sum / channels.to_f32_lossy());
                    chan_idx = 0;
                }
            }
        }
        (format, bits) => return Err(EnrollError::UnsupportedFormat { format, bits }),
    }

    if spec.sample_rate == TARGET_SAMPLE_RATE {
        return Ok(mono);
    }

    // Resample to 16kHz.
    let in_rate = spec.sample_rate as usize;
    let out_rate = TARGET_SAMPLE_RATE as usize;
    let chunk_size = (in_rate / 4).max(256).min(mono.len().max(256));
    let exact_out = crate::mix::resampled_len(mono.len(), spec.sample_rate, TARGET_SAMPLE_RATE);

    let mut resampler = FftFixedIn::<f32>::new(in_rate, out_rate, chunk_size, 2, 1)
        .map_err(|e| EnrollError::Resample(e.to_string()))?;

    let mut out: Vec<f32> = Vec::with_capacity(exact_out + chunk_size);
    let mut cursor = 0;
    while cursor < mono.len() {
        let end = (cursor + chunk_size).min(mono.len());
        let mut chunk = mono[cursor..end].to_vec();
        if chunk.len() < chunk_size {
            chunk.resize(chunk_size, 0.0);
        }
        let resampled = resampler
            .process(&[chunk], None)
            .map_err(|e| EnrollError::Resample(e.to_string()))?;
        out.extend_from_slice(&resampled[0]);
        cursor += chunk_size;
    }
    out.truncate(exact_out);
    Ok(out)
}

#[cfg(target_os = "macos")]
fn load_extractor(
    model_path: &std::path::Path,
) -> Result<sherpa_rs::speaker_id::EmbeddingExtractor, EnrollError> {
    let model = model_path
        .to_str()
        .ok_or_else(|| EnrollError::Embedding("model path is not valid UTF-8".into()))?;
    let config = sherpa_rs::speaker_id::ExtractorConfig {
        model: model.into(),
        ..Default::default()
    };
    sherpa_rs::speaker_id::EmbeddingExtractor::new(config)
        .map_err(|e| EnrollError::Embedding(format!("{e:?}")))
}

/// Extract an L2-normalized speaker embedding from f32 mono 16kHz samples.
#[cfg(target_os = "macos")]
fn compute_embedding(
    extractor: &mut sherpa_rs::speaker_id::EmbeddingExtractor,
    samples: Vec<f32>,
) -> Result<Vec<f32>, EnrollError> {
    let raw = extractor
        .compute_speaker_embedding(samples, TARGET_SAMPLE_RATE)
        .map_err(|e| EnrollError::Embedding(format!("{e:?}")))?;
    Ok(crate::subdiarize::l2_normalize(&raw))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::num::ToIntSaturating;
    use std::path::Path;

    #[test]
    fn value_is_loaded_once_per_path() {
        let cache = LoadOnce::<u32>::default();
        let loads = std::cell::Cell::new(0);
        let load = |_: &Path| -> Result<u32, ()> {
            loads.set(loads.get() + 1);
            Ok(41)
        };
        for _ in 0..3 {
            let got = cache.with(Path::new("/model-a"), load, |value| {
                *value += 1;
                *value
            });
            assert!(got.is_ok());
        }
        assert_eq!(loads.get(), 1, "loaded on the first call only");
        assert_eq!(cache.with(Path::new("/model-a"), load, |v| *v), Ok(44));
        assert_eq!(cache.with(Path::new("/model-b"), load, |v| *v), Ok(41));
        assert_eq!(loads.get(), 2);
    }

    #[test]
    fn failed_load_is_retried() {
        let cache = LoadOnce::<u32>::default();
        assert_eq!(
            cache.with(Path::new("/m"), |_| Err("offline"), |v| *v),
            Err("offline")
        );
        assert_eq!(
            cache.with(Path::new("/m"), |_| Ok::<_, &str>(7), |v| *v),
            Ok(7)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn wav_body_is_decoded_and_resampled_to_16k_mono() {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut bytes = std::io::Cursor::new(Vec::new());
        let mut writer = hound::WavWriter::new(&mut bytes, spec).unwrap();
        for i in 0..48_000 {
            let s = ((i.to_f32_lossy() * 0.05).sin() * 8_000.0).to_i16_saturating();
            writer.write_sample(s).unwrap();
            writer.write_sample(s).unwrap();
        }
        writer.finalize().unwrap();
        let samples = decode_wav_bytes_to_f32_mono_16k(bytes.get_ref()).unwrap();
        assert_eq!(samples.len(), 16_000);

        let error = decode_wav_bytes_to_f32_mono_16k(b"not a wav").unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    }
}
