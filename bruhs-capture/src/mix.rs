//! Mix mic + system audio WAVs into a single mixed file for
//! transcription. Both inputs are 16-bit PCM (multi-channel input is
//! downmixed). Sample rates may differ (mic from cpal at device-native rate,
//! sys from SCK at 48kHz); sys is resampled to mic's rate via rubato.
//!
//! Both inputs are streamed in chunks, so memory stays bounded however long
//! the meeting was. The output is written to a temporary file and renamed into
//! place only when complete.

use bruhs_models::num::{clamp_to_i16, ToIntSaturating};
use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use rubato::{FftFixedIn, Resampler};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// Output length when `frames` samples at `from_rate` are resampled to
/// `to_rate`, rounded down (the length every resampler here truncates to).
pub(crate) fn resampled_len(frames: usize, from_rate: u32, to_rate: u32) -> usize {
    let frames = u64::try_from(frames).unwrap_or(u64::MAX);
    let out = frames.saturating_mul(u64::from(to_rate)) / u64::from(from_rate.max(1));
    usize::try_from(out).unwrap_or(usize::MAX)
}

#[derive(Debug, thiserror::Error)]
pub enum MixError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("hound: {0}")]
    Hound(#[from] hound::Error),
    #[error("unsupported format: {0:?}")]
    UnsupportedFormat(WavSpec),
    #[error("resample: {0}")]
    Resample(String),
}

/// Mix two WAVs into one mono WAV at the mic's rate. If `sys_path` doesn't
/// exist (mic-only session), copy `mic_path` to `out_path`.
pub fn mix(mic_path: &Path, sys_path: &Path, out_path: &Path) -> Result<(), MixError> {
    let mic = MonoFrames::open(mic_path)?;
    if !sys_path.exists() {
        // Mic-only — passthrough.
        drop(mic);
        let mut source = File::open(mic_path)?;
        let mut target = crate::paths::create_private_file(out_path)?;
        std::io::copy(&mut source, &mut target)?;
        target.sync_all()?;
        return Ok(());
    }
    let sys = MonoFrames::open(sys_path)?;
    let mic_spec = mic.spec;
    let sys: Box<dyn Iterator<Item = Result<i16, MixError>>> =
        if sys.spec.sample_rate == mic_spec.sample_rate {
            Box::new(sys)
        } else {
            Box::new(Resampled::new(sys, mic_spec.sample_rate)?)
        };
    write_mixed(mic, sys, mic_spec, out_path)
}

/// Equal-weight sum with clipping, until both inputs are exhausted.
fn write_mixed(
    mut mic: impl Iterator<Item = Result<i16, MixError>>,
    mut sys: impl Iterator<Item = Result<i16, MixError>>,
    spec: WavSpec,
    out_path: &Path,
) -> Result<(), MixError> {
    let parent = out_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    // Created 0600; an error before `persist` removes the partial file.
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    {
        let file = std::io::BufWriter::new(temporary.as_file_mut());
        let mut writer = WavWriter::new(file, spec)?;
        loop {
            let (m, s) = (mic.next().transpose()?, sys.next().transpose()?);
            if m.is_none() && s.is_none() {
                break;
            }
            let sum = i32::from(m.unwrap_or(0)) + i32::from(s.unwrap_or(0));
            writer.write_sample(clamp_to_i16(sum))?;
        }
        writer.finalize()?;
    }
    temporary.as_file().sync_all()?;
    temporary
        .persist(out_path)
        .map_err(|error| MixError::Io(error.error))?;
    Ok(())
}

/// A 16-bit PCM WAV read one mono frame at a time (channels averaged).
struct MonoFrames {
    spec: WavSpec,
    /// Mono frames in the file (a trailing partial frame counts as one).
    frames: usize,
    channels: usize,
    samples: hound::WavIntoSamples<BufReader<File>, i16>,
}

impl MonoFrames {
    fn open(path: &Path) -> Result<Self, MixError> {
        let reader = WavReader::open(path)?;
        let spec = reader.spec();
        if spec.bits_per_sample != 16 || spec.sample_format != SampleFormat::Int {
            return Err(MixError::UnsupportedFormat(spec));
        }
        let channels = usize::from(spec.channels.max(1));
        Ok(Self {
            spec: WavSpec {
                channels: 1,
                ..spec
            },
            frames: (reader.len() as usize).div_ceil(channels),
            channels,
            samples: reader.into_samples(),
        })
    }
}

impl Iterator for MonoFrames {
    type Item = Result<i16, MixError>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut sum = 0i32;
        let mut count = 0i32;
        for _ in 0..self.channels {
            match self.samples.next() {
                Some(Ok(sample)) => {
                    sum += i32::from(sample);
                    count += 1;
                }
                Some(Err(error)) => return Some(Err(error.into())),
                None => break,
            }
        }
        (count > 0).then(|| Ok(clamp_to_i16(sum / count)))
    }
}

/// Streaming `FftFixedIn` resampling of mono i16 frames, chunk by chunk.
/// `FftFixedIn` is ~5–10× faster than `SincFixedIn` for speech-grade audio
/// at the cost of some high-frequency accuracy — imperceptible for Whisper.
/// The chunking (size, zero-padded last chunk, output truncated to the exact
/// length) matches the original whole-file resampler sample for sample.
struct Resampled {
    input: MonoFrames,
    resampler: FftFixedIn<f32>,
    chunk_size: usize,
    chunk: Vec<f32>,
    output: Vec<i16>,
    position: usize,
    /// Output samples still to produce (input frames × to / from).
    remaining: usize,
}

impl Resampled {
    fn new(input: MonoFrames, to_rate: u32) -> Result<Self, MixError> {
        let from_rate = input.spec.sample_rate;
        // FFT-friendly chunk, clamped to the input length so tiny clips are
        // not over-padded.
        let chunk_size = (from_rate as usize / 4).max(256).min(input.frames.max(256));
        let resampler = FftFixedIn::<f32>::new(
            from_rate as usize,
            to_rate as usize,
            chunk_size,
            2, // sub_chunks
            1, // channels (mono)
        )
        .map_err(|e| MixError::Resample(e.to_string()))?;
        let remaining = resampled_len(input.frames, from_rate, to_rate);
        Ok(Self {
            input,
            resampler,
            chunk_size,
            chunk: Vec::with_capacity(chunk_size),
            output: Vec::new(),
            position: 0,
            remaining,
        })
    }

    /// Resample the next input chunk. `false` when the input is exhausted.
    fn refill(&mut self) -> Result<bool, MixError> {
        self.chunk.clear();
        for sample in self.input.by_ref().take(self.chunk_size) {
            self.chunk.push(f32::from(sample?) / f32::from(i16::MAX));
        }
        if self.chunk.is_empty() {
            return Ok(false);
        }
        self.chunk.resize(self.chunk_size, 0.0);
        let resampled = self
            .resampler
            .process(std::slice::from_ref(&self.chunk), None)
            .map_err(|e| MixError::Resample(e.to_string()))?;
        self.output.clear();
        self.output
            .extend(resampled.first().into_iter().flatten().map(|&s| {
                (s * f32::from(i16::MAX))
                    .clamp(f32::from(i16::MIN), f32::from(i16::MAX))
                    .to_i16_saturating()
            }));
        self.position = 0;
        Ok(true)
    }
}

impl Iterator for Resampled {
    type Item = Result<i16, MixError>;

    fn next(&mut self) -> Option<Self::Item> {
        while self.remaining > 0 {
            if let Some(&sample) = self.output.get(self.position) {
                self.position += 1;
                self.remaining -= 1;
                return Some(Ok(sample));
            }
            match self.refill() {
                Ok(true) => {}
                Ok(false) => return None,
                Err(error) => {
                    self.remaining = 0;
                    return Some(Err(error));
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::num::ToFloatLossy;
    use tempfile::TempDir;

    // The whole-file implementation this streaming mixer replaced, kept as the
    // reference its output must match.
    /// Mix two mono WAVs into one. If `sys_path` doesn't exist (mic-only
    /// session), copy `mic_path` to `out_path`.
    fn reference_mix(mic_path: &Path, sys_path: &Path, out_path: &Path) -> Result<(), MixError> {
        let mic_samples = read_pcm_i16(mic_path)?;
        if !sys_path.exists() {
            // Mic-only — passthrough.
            let mut source = std::fs::File::open(mic_path)?;
            let mut target = std::fs::File::create(out_path)?;
            std::io::copy(&mut source, &mut target)?;
            target.sync_all()?;
            return Ok(());
        }
        let sys_samples = read_pcm_i16(sys_path)?;

        // Resample sys to mic's rate if they differ.
        let sys_resampled = if sys_samples.spec.sample_rate == mic_samples.spec.sample_rate {
            sys_samples.data
        } else {
            resample(
                &sys_samples.data,
                sys_samples.spec.sample_rate,
                mic_samples.spec.sample_rate,
            )?
        };

        // Equal-weight sum with soft clip via i32 intermediate.
        let len = mic_samples.data.len().max(sys_resampled.len());
        let mut mixed = Vec::with_capacity(len);
        for i in 0..len {
            let m = i32::from(*mic_samples.data.get(i).unwrap_or(&0));
            let s = i32::from(*sys_resampled.get(i).unwrap_or(&0));
            let clipped = clamp_to_i16(m + s);
            mixed.push(clipped);
        }

        let file = std::io::BufWriter::new(std::fs::File::create(out_path)?);
        let mut writer = WavWriter::new(file, mic_samples.spec)?;
        for s in mixed {
            writer.write_sample(s)?;
        }
        writer.finalize()?;
        Ok(())
    }

    struct WavData {
        spec: WavSpec,
        data: Vec<i16>,
    }

    fn read_pcm_i16(path: &Path) -> Result<WavData, MixError> {
        let mut reader = WavReader::open(path)?;
        let spec = reader.spec();
        if spec.bits_per_sample != 16 || spec.sample_format != SampleFormat::Int {
            return Err(MixError::UnsupportedFormat(spec));
        }
        if spec.channels != 1 {
            // Downmix stereo to mono by averaging.
            let interleaved: Vec<i16> = reader.samples::<i16>().collect::<Result<_, _>>()?;
            let mono: Vec<i16> = interleaved
                .chunks(spec.channels as usize)
                .map(|c| {
                    let sum = c.iter().map(|&s| i32::from(s)).sum::<i32>();
                    clamp_to_i16(sum / i32::try_from(c.len()).unwrap())
                })
                .collect();
            let mono_spec = WavSpec {
                channels: 1,
                ..spec
            };
            return Ok(WavData {
                spec: mono_spec,
                data: mono,
            });
        }
        let data: Vec<i16> = reader.samples::<i16>().collect::<Result<_, _>>()?;
        Ok(WavData { spec, data })
    }

    /// Resample `input` from `from_rate` to `to_rate` using `FftFixedIn`.
    /// `FftFixedIn` is ~5–10× faster than `SincFixedIn` for speech-grade audio
    /// at the cost of some high-frequency accuracy — imperceptible for Whisper.
    /// Mirrors the approach used in `transcribe/decode.rs`.
    fn resample(input: &[i16], from_rate: u32, to_rate: u32) -> Result<Vec<i16>, MixError> {
        use rubato::{FftFixedIn, Resampler};

        // Choose a chunk size compatible with the FFT resampler (power-of-2 friendly,
        // clamped to the input length so we don't over-pad tiny clips).
        let chunk_size = (from_rate as usize / 4).max(256).min(input.len().max(256));
        let mut resampler = FftFixedIn::<f32>::new(
            from_rate as usize,
            to_rate as usize,
            chunk_size,
            2, // sub_chunks
            1, // channels (mono)
        )
        .map_err(|e| MixError::Resample(e.to_string()))?;

        let exact_out = resampled_len(input.len(), from_rate, to_rate);
        let mut out_f32: Vec<f32> = Vec::with_capacity(exact_out + chunk_size);

        // Convert i16 to f32 in [-1, 1].
        let input_f32: Vec<f32> = input
            .iter()
            .map(|&s| f32::from(s) / f32::from(i16::MAX))
            .collect();

        let mut cursor = 0;
        while cursor < input_f32.len() {
            let end = (cursor + chunk_size).min(input_f32.len());
            let mut chunk = input_f32[cursor..end].to_vec();
            if chunk.len() < chunk_size {
                chunk.resize(chunk_size, 0.0);
            }
            let frames = vec![chunk];
            let resampled = resampler
                .process(&frames, None)
                .map_err(|e| MixError::Resample(e.to_string()))?;
            out_f32.extend_from_slice(&resampled[0]);
            cursor += chunk_size;
        }

        // Trim to exact expected length (rubato may produce slightly more due to padding).
        out_f32.truncate(exact_out);

        let out_i16: Vec<i16> = out_f32
            .iter()
            .map(|&s| {
                (s * f32::from(i16::MAX))
                    .clamp(f32::from(i16::MIN), f32::from(i16::MAX))
                    .to_i16_saturating()
            })
            .collect();
        Ok(out_i16)
    }

    fn write_test_wav(path: &Path, rate: u32, samples: &[i16]) {
        let spec = WavSpec {
            channels: 1,
            sample_rate: rate,
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
    fn mix_passes_through_when_only_mic() {
        let dir = TempDir::new().unwrap();
        let mic = dir.path().join("mic.wav");
        let sys = dir.path().join("sys.wav"); // doesn't exist
        let out = dir.path().join("mixed.wav");
        write_test_wav(&mic, 16_000, &[100, 200, 300, 400]);
        mix(&mic, &sys, &out).unwrap();
        let reader = WavReader::open(&out).unwrap();
        let samples: Vec<i16> = reader.into_samples::<i16>().map(|s| s.unwrap()).collect();
        assert_eq!(samples, vec![100, 200, 300, 400]);
    }

    #[test]
    fn mix_sums_same_rate() {
        let dir = TempDir::new().unwrap();
        let mic = dir.path().join("mic.wav");
        let sys = dir.path().join("sys.wav");
        let out = dir.path().join("mixed.wav");
        write_test_wav(&mic, 16_000, &[100, 200, 300]);
        write_test_wav(&sys, 16_000, &[10, 20, 30]);
        mix(&mic, &sys, &out).unwrap();
        let reader = WavReader::open(&out).unwrap();
        let samples: Vec<i16> = reader.into_samples::<i16>().map(|s| s.unwrap()).collect();
        assert_eq!(samples, vec![110, 220, 330]);
    }

    #[test]
    fn mix_clips_overflow() {
        let dir = TempDir::new().unwrap();
        let mic = dir.path().join("mic.wav");
        let sys = dir.path().join("sys.wav");
        let out = dir.path().join("mixed.wav");
        write_test_wav(&mic, 16_000, &[30_000]);
        write_test_wav(&sys, 16_000, &[30_000]);
        mix(&mic, &sys, &out).unwrap();
        let reader = WavReader::open(&out).unwrap();
        let samples: Vec<i16> = reader.into_samples::<i16>().map(|s| s.unwrap()).collect();
        assert_eq!(samples, vec![i16::MAX]);
    }

    #[test]
    fn mix_resamples_different_rates() {
        let dir = TempDir::new().unwrap();
        let mic = dir.path().join("mic.wav");
        let sys = dir.path().join("sys.wav");
        let out = dir.path().join("mixed.wav");
        write_test_wav(&mic, 16_000, &vec![0; 1600]); // 100ms of silence
        write_test_wav(&sys, 48_000, &vec![100; 4800]); // 100ms at 48k
        mix(&mic, &sys, &out).unwrap();
        // Output should be ~1600 samples at 16k.
        let reader = WavReader::open(&out).unwrap();
        let samples_len = reader.duration() as usize;
        // Allow ±10% slack for resampler edge effects.
        assert!((1440..=1760).contains(&samples_len), "got {samples_len}");
    }

    fn write_wav(path: &Path, rate: u32, channels: u16, samples: &[i16]) {
        let spec = WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut w = WavWriter::create(path, spec).unwrap();
        for &s in samples {
            w.write_sample(s).unwrap();
        }
        w.finalize().unwrap();
    }

    /// Deterministic noisy speech-like signal.
    fn signal(len: usize, seed: u32) -> Vec<i16> {
        let mut state = seed.wrapping_mul(2_654_435_761).max(1);
        (0..len)
            .map(|i| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let tone = (i.to_f32_lossy() * 0.031).sin() * 12_000.0;
                let noise = i32::try_from(state % 4_000).unwrap() - 2_000;
                clamp_to_i16(tone.to_i32_saturating() + noise)
            })
            .collect()
    }

    fn read(path: &Path) -> (WavSpec, Vec<i16>) {
        let reader = WavReader::open(path).unwrap();
        let spec = reader.spec();
        (spec, reader.into_samples().map(|s| s.unwrap()).collect())
    }

    /// The streaming mixer must reproduce the whole-file mixer exactly.
    #[test]
    fn streaming_mix_matches_the_whole_file_mixer() {
        let cases: &[(u32, u16, usize, u32, u16, usize)] = &[
            // mic rate, mic channels, mic frames, sys rate, sys channels, sys frames
            (16_000, 1, 4_000, 48_000, 2, 9_000),
            (16_000, 1, 1_000, 48_000, 2, 30_000),
            (44_100, 2, 20_000, 48_000, 2, 25_000),
            (48_000, 1, 7_000, 48_000, 1, 5_000),
            (16_000, 1, 300, 48_000, 1, 100),
            (16_000, 1, 0, 48_000, 2, 0),
        ];
        for (index, &(mic_rate, mic_ch, mic_frames, sys_rate, sys_ch, sys_frames)) in
            cases.iter().enumerate()
        {
            let dir = TempDir::new().unwrap();
            let mic = dir.path().join("mic.wav");
            let sys = dir.path().join("sys.wav");
            write_wav(
                &mic,
                mic_rate,
                mic_ch,
                &signal(mic_frames * mic_ch as usize, 1),
            );
            write_wav(
                &sys,
                sys_rate,
                sys_ch,
                &signal(sys_frames * sys_ch as usize, 2),
            );
            let streamed = dir.path().join("streamed.wav");
            let reference = dir.path().join("reference.wav");
            mix(&mic, &sys, &streamed).unwrap();
            reference_mix(&mic, &sys, &reference).unwrap();
            let (streamed_spec, streamed) = read(&streamed);
            let (reference_spec, reference) = read(&reference);
            assert_eq!(streamed_spec, reference_spec, "case {index}");
            assert_eq!(streamed.len(), reference.len(), "case {index}");
            assert!(streamed == reference, "case {index}: samples differ");
        }
    }

    #[cfg(unix)]
    #[test]
    fn mixed_output_is_private_and_failure_leaves_no_partial_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let mic = dir.path().join("mic.wav");
        let sys = dir.path().join("sys.wav");
        let out = dir.path().join("mixed.wav");
        write_wav(&mic, 16_000, 1, &signal(1_000, 3));
        write_wav(&sys, 48_000, 1, &signal(3_000, 4));
        mix(&mic, &sys, &out).unwrap();
        let mode = std::fs::metadata(&out).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);

        // A truncated system recording fails without leaving temp files.
        let bytes = std::fs::read(&sys).unwrap();
        std::fs::write(&sys, &bytes[..bytes.len() - 1001]).unwrap();
        std::fs::remove_file(&out).unwrap();
        assert!(mix(&mic, &sys, &out).is_err());
        let mut names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, ["mic.wav", "sys.wav"]);
    }
}
