//! 16-bit mono PCM WAV encoding for voice enrollment uploads. Moved from
//! bruhs-web `components/voice_enroll.rs` and bruhs-desktop
//! `views/meetings_hub.rs` (identical copies).

use crate::num::ToIntSaturating;

/// Bytes in the RIFF/WAVE header [`encode_wav_16bit_mono`] writes.
pub const WAV_HEADER_LEN: usize = 44;

/// The samples do not fit in a WAV file (the RIFF size field is 32-bit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{samples} samples is too long for a WAV file")]
pub struct WavTooLong {
    pub samples: usize,
}

/// Encode `samples` (range `-1.0..=1.0`, clamped) as a 16-bit mono PCM WAV
/// file: the standard 44-byte header followed by little-endian `i16`s.
pub fn encode_wav_16bit_mono(samples: &[f32], sample_rate: u32) -> Result<Vec<u8>, WavTooLong> {
    let too_long = WavTooLong {
        samples: samples.len(),
    };
    // RIFF size = 36 + data bytes, and it must fit in a u32.
    let data_size = u32::try_from(samples.len())
        .ok()
        .and_then(|n| n.checked_mul(2))
        .filter(|size| size.checked_add(36).is_some())
        .ok_or(too_long)?;
    let file_size = 36 + data_size;
    let byte_rate = sample_rate.saturating_mul(2); // 1 channel × 2 bytes/sample

    let mut wav = Vec::with_capacity(WAV_HEADER_LEN + samples.len() * 2);
    // RIFF chunk
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&file_size.to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    // fmt sub-chunk: size 16, PCM (1), mono (1), sample rate, byte rate,
    // block align 2, 16 bits per sample.
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    // data sub-chunk
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    for &s in samples {
        // NaN becomes 0.
        let v = (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)).to_i16_saturating();
        wav.extend_from_slice(&v.to_le_bytes());
    }
    Ok(wav)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16_at(wav: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([wav[at], wav[at + 1]])
    }

    fn u32_at(wav: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([wav[at], wav[at + 1], wav[at + 2], wav[at + 3]])
    }

    #[test]
    fn wav_header_is_44_bytes_and_16_bit_mono() {
        // Desktop `meetings_hub` test.
        let wav = encode_wav_16bit_mono(&[0.0, 1.0, -1.0], 16_000).unwrap();
        assert_eq!(wav.len(), 44 + 6);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(u32_at(&wav, 4), 36 + 6);
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(u16_at(&wav, 20), 1, "PCM");
        assert_eq!(u16_at(&wav, 22), 1, "mono");
        assert_eq!(u32_at(&wav, 24), 16_000);
        assert_eq!(u32_at(&wav, 28), 32_000, "byte rate");
        assert_eq!(u16_at(&wav, 32), 2, "block align");
        assert_eq!(u16_at(&wav, 34), 16);
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(u32_at(&wav, 40), 6);
    }

    #[test]
    fn samples_are_clamped_little_endian_i16() {
        let wav =
            encode_wav_16bit_mono(&[0.0, 1.0, -1.0, 2.0, -2.0, 0.5, f32::NAN], 8_000).unwrap();
        let samples: Vec<i16> = wav[WAV_HEADER_LEN..]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(
            samples,
            vec![0, 32_767, -32_767, 32_767, -32_767, 16_383, 0]
        );
    }

    #[test]
    fn empty_input_is_a_bare_header() {
        let wav = encode_wav_16bit_mono(&[], 16_000).unwrap();
        assert_eq!(wav.len(), WAV_HEADER_LEN);
        assert_eq!(u32_at(&wav, 40), 0);
    }
}
