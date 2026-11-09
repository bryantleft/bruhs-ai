//! Convert a recorded WAV file into Whisper's expected input: f32
//! samples, mono, 16 kHz. We do all of: read i16 PCM via hound, mix to
//! mono, resample to 16 kHz via rubato.
//!
//! rubato 0.15: `FftFixedIn::new(in_rate, out_rate, chunk_size, sub_chunks, channels)`
//! Input/output are non-interleaved `Vec<Vec<f32>>` (one inner Vec per channel).

use bruhs_models::num::ToFloatLossy;
use rubato::{FftFixedIn, Resampler};
use std::path::Path;

const TARGET_SAMPLE_RATE: u32 = 16_000;

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("hound: {0}")]
    Hound(#[from] hound::Error),
    #[error("resample: {0}")]
    Resample(String),
    #[error("unsupported wav format: {0}")]
    UnsupportedFormat(String),
}

/// Decode a WAV file starting from `mono_sample_offset` (post-mix-to-mono
/// sample index). Returns the decoded 16 kHz f32 samples from that point
/// onward, plus the new offset (old offset + samples read) for the caller to
/// persist as the next watermark.
///
/// This avoids re-reading the entire file on every tick — only the new audio
/// since the last decode is processed.
///
/// Returns `(new_samples, new_mono_offset)`. On error, returns `Err`.
pub fn decode_wav_from_sample(
    path: &Path,
    mono_sample_offset: u32,
) -> Result<(Vec<f32>, u32), DecodeError> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    if spec.bits_per_sample != 16 || spec.sample_format != hound::SampleFormat::Int {
        return Err(DecodeError::UnsupportedFormat(format!(
            "expected 16-bit PCM, got {} bits {:?}",
            spec.bits_per_sample, spec.sample_format
        )));
    }
    let channels = spec.channels as usize;
    if channels == 0 {
        return Err(DecodeError::UnsupportedFormat("0 channels".into()));
    }

    // mono_sample_offset is in post-mix mono samples; each mono sample
    // corresponds to `channels` raw interleaved PCM samples.
    let raw_sample_offset =
        mono_sample_offset.saturating_mul(u32::try_from(channels).unwrap_or(u32::MAX));
    // Seek past already-processed samples. hound seek() takes a sample offset
    // in units of single-channel samples (i.e. interleaved frames × channels).
    reader.seek(raw_sample_offset).map_err(|e| {
        DecodeError::Hound(hound::Error::IoError(std::io::Error::other(e.to_string())))
    })?;

    // Read the remaining interleaved samples and mix to mono f32.
    // channels > 0 is guaranteed by the UnsupportedFormat check above.
    let remaining_cap = (reader.len().saturating_sub(raw_sample_offset) as usize) / channels;
    let mut mono: Vec<f32> = Vec::with_capacity(remaining_cap);
    let mut chan_buf = vec![0i32; channels];
    let mut chan_idx = 0usize;
    for sample in reader.samples::<i16>() {
        let s = sample?;
        chan_buf[chan_idx] = i32::from(s);
        chan_idx += 1;
        if chan_idx == channels {
            let sum: i32 = chan_buf.iter().copied().sum();
            let avg = sum.to_f32_lossy() / channels.to_f32_lossy() / f32::from(i16::MAX);
            mono.push(avg);
            chan_idx = 0;
        }
    }

    let new_offset =
        mono_sample_offset.saturating_add(u32::try_from(mono.len()).unwrap_or(u32::MAX));

    if spec.sample_rate == TARGET_SAMPLE_RATE {
        return Ok((mono, new_offset));
    }
    let resampled = resample_mono(mono, spec.sample_rate, TARGET_SAMPLE_RATE)?;
    // The new offset is still in source-rate mono samples (not resampled),
    // because the caller tracks source-rate watermarks.
    Ok((resampled, new_offset))
}

/// Load a WAV file, mix to mono, and resample to 16 kHz f32. Returns the
/// final mono sample buffer.
pub fn decode_wav_to_whisper_input(path: &Path) -> Result<Vec<f32>, DecodeError> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    if spec.bits_per_sample != 16 || spec.sample_format != hound::SampleFormat::Int {
        return Err(DecodeError::UnsupportedFormat(format!(
            "expected 16-bit PCM, got {} bits {:?}",
            spec.bits_per_sample, spec.sample_format
        )));
    }
    let channels = spec.channels as usize;
    if channels == 0 {
        return Err(DecodeError::UnsupportedFormat("0 channels".into()));
    }

    // Read interleaved samples and average across channels into f32 mono.
    // channels > 0 is guaranteed by the UnsupportedFormat check above.
    let mut mono: Vec<f32> = Vec::with_capacity(reader.len() as usize / channels);
    let mut chan_buf = vec![0i32; channels];
    let mut chan_idx = 0usize;
    for sample in reader.samples::<i16>() {
        let s = sample?;
        chan_buf[chan_idx] = i32::from(s);
        chan_idx += 1;
        if chan_idx == channels {
            let sum: i32 = chan_buf.iter().copied().sum();
            let avg = sum.to_f32_lossy() / channels.to_f32_lossy() / f32::from(i16::MAX);
            mono.push(avg);
            chan_idx = 0;
        }
    }

    if spec.sample_rate == TARGET_SAMPLE_RATE {
        return Ok(mono);
    }
    resample_mono(mono, spec.sample_rate, TARGET_SAMPLE_RATE)
}

fn resample_mono(input: Vec<f32>, in_rate: u32, out_rate: u32) -> Result<Vec<f32>, DecodeError> {
    // rubato 0.15: FftFixedIn::new(in_rate, out_rate, chunk_size, sub_chunks, nbr_channels)
    // Input is non-interleaved: &[Vec<f32>] with one Vec per channel.
    //
    // We clamp chunk_size to the input length so we never produce output for
    // zero-padded frames beyond what the actual audio contains.
    let chunk_size = (in_rate as usize / 4).max(256).min(input.len().max(256));
    let mut resampler = FftFixedIn::<f32>::new(
        in_rate as usize,
        out_rate as usize,
        chunk_size,
        2,
        1, // mono
    )
    .map_err(|e| DecodeError::Resample(format!("{e}")))?;

    // How many output samples correspond to exactly `input.len()` input frames.
    let exact_out = crate::mix::resampled_len(input.len(), in_rate, out_rate);
    let mut out: Vec<f32> = Vec::with_capacity(exact_out + chunk_size);

    let mut cursor = 0;
    while cursor < input.len() {
        let end = (cursor + chunk_size).min(input.len());
        let mut chunk: Vec<f32> = input[cursor..end].to_vec();
        if chunk.len() < chunk_size {
            chunk.resize(chunk_size, 0.0);
        }
        // Non-interleaved: one Vec per channel; we have 1 channel.
        let frames = vec![chunk];
        let resampled = resampler
            .process(&frames, None)
            .map_err(|e| DecodeError::Resample(format!("{e}")))?;
        // resampled[0] is the single mono channel output.
        out.extend_from_slice(&resampled[0]);
        cursor += chunk_size;
    }

    // Trim to the exact frame count so callers get a buffer that matches the
    // original audio duration rather than a padded one.
    out.truncate(exact_out);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::num::ToIntSaturating;
    use hound::{SampleFormat, WavSpec, WavWriter};

    fn write_test_wav(path: &Path, sample_rate: u32, channels: u16, samples: &[i16]) {
        let spec = WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut w = WavWriter::create(path, spec).unwrap();
        for &s in samples {
            w.write_sample(s).unwrap();
        }
        w.finalize().unwrap();
    }

    #[test]
    fn decode_passes_through_16k_mono() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("bruhs-decode-test-{}.wav", uuid::Uuid::new_v4()));
        write_test_wav(&path, 16_000, 1, &[1000, -1000, 2000, -2000, 0, 0]);
        let out = decode_wav_to_whisper_input(&path).unwrap();
        assert_eq!(out.len(), 6);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn decode_mixes_stereo_to_mono() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("bruhs-decode-stereo-{}.wav", uuid::Uuid::new_v4()));
        // Each stereo frame: L=+N, R=-N → average = 0
        write_test_wav(&path, 16_000, 2, &[10, -10, 100, -100, 1000, -1000, 0, 0]);
        let out = decode_wav_to_whisper_input(&path).unwrap();
        assert_eq!(out.len(), 4);
        for &s in &out {
            assert!(s.abs() < 0.001, "expected ~0 after stereo mix, got {s}");
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn decode_resamples_48k_to_16k() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("bruhs-decode-48k-{}.wav", uuid::Uuid::new_v4()));
        let samples: Vec<i16> = (0..480)
            .map(|i| ((i.to_f32_lossy() * 0.5).sin() * 1000.0).to_i16_saturating())
            .collect();
        write_test_wav(&path, 48_000, 1, &samples);
        let out = decode_wav_to_whisper_input(&path).unwrap();
        let expected = 480 / 3; // 160
        let tolerance = expected; // up to one chunk of slack
        assert!(
            out.len().abs_diff(expected) <= tolerance,
            "expected ~{expected} samples, got {}",
            out.len()
        );
        let _ = std::fs::remove_file(&path);
    }
}
