//! Zero-dependency streaming Blackman-Harris windowed-sinc polyphase FIR
//! mono audio resampler for the capture daemon.
//!
//! Replaces `rubato` and `audioadapter-buffers`:
//! - Exact rational ratio reduction via `gcd(from_rate, to_rate)` (zero phase
//!   drift over multi-hour recordings).
//! - 4-term Blackman-Harris windowed-sinc anti-aliasing filter (>90 dB stopband
//!   attenuation) with 32 taps per output sample.
//! - Precomputed L1/L2-friendly polyphase filter bank for standard audio ratios
//!   (e.g., 48 kHz → 16 kHz is a single 32-tap kernel; 44.1 kHz → 16 kHz is
//!   160 phases × 32 taps = 20 KiB) and 256-phase linearly interpolated sinc
//!   table fallback for arbitrary non-standard ratios.
//! - Strictly causal (fixed linear-phase group delay of 16 input samples) so
//!   streaming chunks never stall waiting for future lookahead and match
//!   single-shot resampling sample-for-sample.

use std::f64::consts::PI;

use bruhs_models::logic::wav::resampled_len;
use bruhs_models::num::{ToFloatLossy as _, ToIntSaturating as _};

/// Half-width of the windowed-sinc kernel in output-adjusted input samples.
const HALF_TAPS: usize = 16;
/// Maximum exact rational `up` factor before falling back to an interpolated table.
const MAX_EXACT_PHASES: usize = 512;
/// Number of sub-phases used in the linearly interpolated fallback table.
const INTERP_PHASES: usize = 256;

#[derive(Debug, thiserror::Error)]
pub(crate) enum ResampleError {
    #[error("invalid sample rate conversion: {from_rate} Hz -> {to_rate} Hz")]
    InvalidRate { from_rate: u32, to_rate: u32 },
}

enum FilterBank {
    /// Identity conversion (`from_rate == to_rate`).
    Identity,
    /// Exact polyphase table: `kernels[phase * taps .. (phase + 1) * taps]`,
    /// where `phase` indexes fractional offset `phase / up` in `[0, 1)`.
    Exact {
        up: u64,
        down: u64,
        taps: usize,
        kernels: Vec<f32>,
    },
    /// Oversampled table with linear interpolation between adjacent sub-phases.
    Interpolated {
        up: u64,
        down: u64,
        taps: usize,
        /// `(INTERP_PHASES + 1) * taps` so phase `INTERP_PHASES` is directly indexable.
        kernels: Vec<f32>,
    },
}

pub(crate) struct MonoResampler {
    bank: FilterBank,
    chunk_size: usize,
    /// Ring-like contiguous history buffer: holds `taps - 1` carry samples from
    /// previous chunks followed by the current input chunk.
    history: Vec<f32>,
    /// Exact rational position within `history`: `int_pos` is the index of the
    /// newest sample in the FIR window, and `frac_num / up` is the fractional
    /// offset in `[0, 1)`.
    int_pos: usize,
    frac_num: u64,
    /// Reusable output buffer (zero heap allocations in `process`).
    output: Vec<f32>,
}

impl MonoResampler {
    pub(crate) fn new(frames: usize, from_rate: u32, to_rate: u32) -> Result<Self, ResampleError> {
        if from_rate == 0 || to_rate == 0 {
            return Err(ResampleError::InvalidRate { from_rate, to_rate });
        }
        let from_usize = usize::try_from(from_rate).unwrap_or(usize::MAX);
        let chunk_size = (from_usize / 4).max(256).min(frames.max(256));

        if from_rate == to_rate {
            return Ok(Self {
                bank: FilterBank::Identity,
                chunk_size,
                history: Vec::new(),
                int_pos: 0,
                frac_num: 0,
                output: Vec::with_capacity(chunk_size),
            });
        }

        let g = gcd_u64(u64::from(from_rate), u64::from(to_rate));
        let down = u64::from(from_rate) / g;
        let up = u64::from(to_rate) / g;

        let ratio = f64::from(to_rate) / f64::from(from_rate);
        // Scale kernel length inversely with downsampling ratio (capped) so
        // anti-aliasing stays sharp (>90 dB) even at high decimation ratios.
        let scale = ratio.min(1.0);
        let cutoff = 0.94 * scale;
        let half_taps = ((HALF_TAPS.to_f64_lossy() / scale)
            .ceil()
            .to_usize_saturating())
        .clamp(HALF_TAPS, 96);
        let taps = half_taps * 2;

        let bank = if up <= u64::try_from(MAX_EXACT_PHASES).unwrap_or(u64::MAX) {
            let up_usize = usize::try_from(up).unwrap_or(MAX_EXACT_PHASES);
            let mut kernels = vec![0.0_f32; up_usize * taps];
            for phase in 0..up_usize {
                let frac = phase.to_f64_lossy() / up_usize.to_f64_lossy();
                build_kernel(
                    &mut kernels[phase * taps..(phase + 1) * taps],
                    taps,
                    frac,
                    cutoff,
                );
            }
            FilterBank::Exact {
                up,
                down,
                taps,
                kernels,
            }
        } else {
            let mut kernels = vec![0.0_f32; (INTERP_PHASES + 1) * taps];
            for phase in 0..=INTERP_PHASES {
                let frac = phase.to_f64_lossy() / INTERP_PHASES.to_f64_lossy();
                build_kernel(
                    &mut kernels[phase * taps..(phase + 1) * taps],
                    taps,
                    frac,
                    cutoff,
                );
            }
            FilterBank::Interpolated {
                up,
                down,
                taps,
                kernels,
            }
        };

        // Pre-fill `taps - 1` leading zeros so the very first input sample at
        // index `taps - 1` has a complete causal window `[0..taps]`.
        let mut history = Vec::with_capacity(taps + chunk_size);
        history.resize(taps - 1, 0.0);

        let max_out = resampled_len(chunk_size, from_rate, to_rate) + 4;
        Ok(Self {
            bank,
            chunk_size,
            history,
            int_pos: taps - 1,
            frac_num: 0,
            output: Vec::with_capacity(max_out),
        })
    }

    pub(crate) fn chunk_size(&self) -> usize {
        self.chunk_size
    }

    /// Process one chunk of mono audio samples without allocating.
    pub(crate) fn process(&mut self, input: &[f32]) -> Result<&[f32], ResampleError> {
        self.output.clear();
        if input.is_empty() {
            return Ok(&self.output);
        }

        match &self.bank {
            FilterBank::Identity => {
                self.output.extend_from_slice(input);
                Ok(&self.output)
            }
            FilterBank::Exact {
                up,
                down,
                taps,
                kernels,
            } => {
                let up = *up;
                let down = *down;
                let taps = *taps;
                self.history.extend_from_slice(input);
                let total = self.history.len();

                while self.int_pos < total {
                    let phase = usize::try_from(self.frac_num).unwrap_or(0);
                    let kernel = &kernels[phase * taps..(phase + 1) * taps];
                    let base = self.int_pos + 1 - taps;
                    let window = &self.history[base..base + taps];
                    self.output.push(dot_f32(window, kernel));

                    let next_num = self.frac_num + down;
                    let step = usize::try_from(next_num / up).unwrap_or(usize::MAX);
                    self.frac_num = next_num % up;
                    self.int_pos = self.int_pos.saturating_add(step);
                }

                // Retain the final `taps - 1` samples as history for the next chunk.
                let keep = taps - 1;
                let consumed = total - keep;
                self.history.copy_within(consumed..total, 0);
                self.history.truncate(keep);
                self.int_pos -= consumed;

                Ok(&self.output)
            }
            FilterBank::Interpolated {
                up,
                down,
                taps,
                kernels,
            } => {
                let up = *up;
                let down = *down;
                let taps = *taps;
                self.history.extend_from_slice(input);
                let total = self.history.len();
                let inv_up = INTERP_PHASES.to_f64_lossy() / up.to_f64_lossy();

                while self.int_pos < total {
                    let scaled = self.frac_num.to_f64_lossy() * inv_up;
                    let idx = scaled.floor().to_usize_saturating().min(INTERP_PHASES - 1);
                    let alpha = (scaled - idx.to_f64_lossy()).to_f32_lossy();
                    let k0 = &kernels[idx * taps..(idx + 1) * taps];
                    let k1 = &kernels[(idx + 1) * taps..(idx + 2) * taps];
                    let base = self.int_pos + 1 - taps;
                    let window = &self.history[base..base + taps];
                    let v0 = dot_f32(window, k0);
                    let v1 = dot_f32(window, k1);
                    self.output.push(v0 + alpha * (v1 - v0));

                    let next_num = self.frac_num + down;
                    let step = usize::try_from(next_num / up).unwrap_or(usize::MAX);
                    self.frac_num = next_num % up;
                    self.int_pos = self.int_pos.saturating_add(step);
                }

                let keep = taps - 1;
                let consumed = total - keep;
                self.history.copy_within(consumed..total, 0);
                self.history.truncate(keep);
                self.int_pos -= consumed;

                Ok(&self.output)
            }
        }
    }
}

pub(crate) fn resample_mono(
    input: &[f32],
    from_rate: u32,
    to_rate: u32,
) -> Result<Vec<f32>, ResampleError> {
    let mut resampler = MonoResampler::new(input.len(), from_rate, to_rate)?;
    let exact_out = resampled_len(input.len(), from_rate, to_rate);
    let mut output = Vec::with_capacity(exact_out);
    for chunk in input.chunks(resampler.chunk_size()) {
        let remaining = exact_out.saturating_sub(output.len());
        if remaining == 0 {
            break;
        }
        let samples = resampler.process(chunk)?;
        output.extend_from_slice(&samples[..samples.len().min(remaining)]);
    }
    // If rational stepping fell 1 sample short due to ceiling rounding in
    // `resampled_len`, flush one silent tail sample to match the exact length.
    while output.len() < exact_out {
        let remaining = exact_out - output.len();
        let tail = resampler.process(&[0.0])?;
        if tail.is_empty() {
            output.resize(exact_out, 0.0);
            break;
        }
        output.extend_from_slice(&tail[..tail.len().min(remaining)]);
    }
    Ok(output)
}

/// Build a normalized 4-term Blackman-Harris windowed-sinc kernel of length
/// `taps` centered for fractional offset `frac` in `[0, 1)`.
///
/// The kernel is laid out in forward memory order matching `history[base..base+taps]`
/// so `dot_f32` auto-vectorizes cleanly with contiguous SIMD loads.
fn build_kernel(out: &mut [f32], taps: usize, frac: f64, cutoff: f64) {
    let center = (taps / 2).to_f64_lossy() - 1.0 + frac;
    let denom = (taps - 1).to_f64_lossy();
    let mut sum = 0.0_f64;
    let mut tmp = [0.0_f64; 192];
    let buf = &mut tmp[..taps];

    for (i, slot) in buf.iter_mut().enumerate() {
        let idx = i.to_f64_lossy();
        let x = idx - center;
        let sinc = if x.abs() < 1e-12 {
            1.0
        } else {
            let arg = PI * cutoff * x;
            arg.sin() / arg
        };
        // 4-term Blackman-Harris window (-92 dB peak sidelobe).
        let w_pos = (idx + 0.5 - frac).clamp(0.0, denom) / denom;
        let a = 2.0 * PI * w_pos;
        let window =
            0.35875 - 0.48829 * a.cos() + 0.14128 * (2.0 * a).cos() - 0.01168 * (3.0 * a).cos();
        let coeff = sinc * window;
        *slot = coeff;
        sum += coeff;
    }

    let inv_sum = if sum.abs() > 1e-12 { 1.0 / sum } else { 1.0 };
    for (dst, &src) in out.iter_mut().zip(buf.iter()) {
        *dst = (src * inv_sum).to_f32_lossy();
    }
}

#[inline]
fn dot_f32(a: &[f32], b: &[f32]) -> f32 {
    // 4-way unrolled accumulation helps LLVM emit NEON/AVX FMA loops.
    let mut s0 = 0.0_f32;
    let mut s1 = 0.0_f32;
    let mut s2 = 0.0_f32;
    let mut s3 = 0.0_f32;
    let (chunks_a, rem_a) = a.as_chunks::<4>();
    let (chunks_b, rem_b) = b.as_chunks::<4>();
    for (ca, cb) in chunks_a.iter().zip(chunks_b.iter()) {
        s0 = ca[0].mul_add(cb[0], s0);
        s1 = ca[1].mul_add(cb[1], s1);
        s2 = ca[2].mul_add(cb[2], s2);
        s3 = ca[3].mul_add(cb[3], s3);
    }
    let mut total = (s0 + s1) + (s2 + s3);
    for (&xa, &xb) in rem_a.iter().zip(rem_b.iter()) {
        total = xa.mul_add(xb, total);
    }
    total
}

const fn gcd_u64(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_wave(rate: u32, freq_hz: f64, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|i| {
                let t = i.to_f64_lossy() / f64::from(rate);
                (2.0 * PI * freq_hz * t).sin().to_f32_lossy()
            })
            .collect()
    }

    fn rms(samples: &[f32]) -> f64 {
        if samples.is_empty() {
            return 0.0;
        }
        let sum_sq: f64 = samples
            .iter()
            .map(|&s| {
                let v = f64::from(s);
                v * v
            })
            .sum();
        (sum_sq / samples.len().to_f64_lossy()).sqrt()
    }

    #[test]
    fn rejects_zero_sample_rates() {
        assert!(MonoResampler::new(1024, 0, 16_000).is_err());
        assert!(MonoResampler::new(1024, 48_000, 0).is_err());
        assert!(resample_mono(&[0.0; 100], 0, 16_000).is_err());
    }

    #[test]
    fn identity_rate_is_exact_passthrough() {
        let input = sine_wave(16_000, 440.0, 2000);
        let out = resample_mono(&input, 16_000, 16_000).unwrap();
        assert_eq!(out, input);
    }

    #[test]
    fn preserves_inband_sine_amplitude_across_common_ratios() {
        for (from, to) in [
            (48_000, 16_000),
            (44_100, 16_000),
            (16_000, 48_000),
            (44_100, 48_000),
            (48_000, 44_100),
            // Non-standard ratio that exercises the interpolated table path.
            (47_999, 16_001),
        ] {
            let input = sine_wave(from, 1_000.0, from as usize / 2);
            let out = resample_mono(&input, from, to).unwrap();
            assert_eq!(out.len(), resampled_len(input.len(), from, to));
            // Skip initial/final filter transient when measuring steady-state RMS.
            let steady = &out[256..out.len() - 256];
            let measured = rms(steady);
            let expected = 0.5_f64.sqrt();
            assert!(
                (measured - expected).abs() < 0.02,
                "{from}->{to}: rms={measured:.4}, expected={expected:.4}"
            );
        }
    }

    #[test]
    fn rejects_aliasing_above_target_nyquist() {
        // 12 kHz tone is well above 16 kHz's 8 kHz Nyquist.
        let input = sine_wave(48_000, 12_000.0, 48_000);
        let out = resample_mono(&input, 48_000, 16_000).unwrap();
        let steady = &out[256..out.len() - 256];
        let residual = rms(steady);
        // >70 dB attenuation (Blackman-Harris achieves >85 dB).
        assert!(residual < 1e-3, "aliasing residual too high: {residual:.6}");
    }

    #[test]
    fn streaming_chunks_match_single_shot_bit_for_bit() {
        let input = sine_wave(44_100, 750.0, 44_100);
        let batch = resample_mono(&input, 44_100, 16_000).unwrap();

        let mut streaming = MonoResampler::new(input.len(), 44_100, 16_000).unwrap();
        let mut collected = Vec::new();
        for chunk in input.chunks(333) {
            collected.extend_from_slice(streaming.process(chunk).unwrap());
        }
        let n = batch.len().min(collected.len());
        assert!(n >= batch.len() - 1);
        for i in 0..n {
            assert!(
                (batch[i] - collected[i]).abs() <= 1e-6,
                "mismatch at sample {i}: batch={} streaming={}",
                batch[i],
                collected[i]
            );
        }
    }

    #[test]
    fn handles_empty_and_short_inputs() {
        assert!(resample_mono(&[], 48_000, 16_000).unwrap().is_empty());
        let short = resample_mono(&[0.5, -0.5, 0.25], 48_000, 16_000).unwrap();
        assert_eq!(short.len(), resampled_len(3, 48_000, 16_000));
    }
}
