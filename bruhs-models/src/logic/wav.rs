//! Pure-Rust RIFF/WAVE encoding, streaming reader, and streaming writer shared
//! across web, desktop, and capture (`bruhs-capture`).

use crate::num::ToIntSaturating;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::marker::PhantomData;
use std::path::Path;

/// Bytes in the standard 44-byte RIFF/WAVE header [`encode_wav_16bit_mono`] writes.
pub const WAV_HEADER_LEN: usize = 44;

/// Output frames after resampling, rounded down and clamped to the platform size.
pub fn resampled_len(frames: usize, from_rate: u32, to_rate: u32) -> usize {
    let frames = u64::try_from(frames).unwrap_or(u64::MAX);
    let out = frames.saturating_mul(u64::from(to_rate)) / u64::from(from_rate.max(1));
    usize::try_from(out).unwrap_or(usize::MAX)
}

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

// ---------------------------------------------------------------------------
// Streaming WAV reader & writer (replaces `hound`)
// ---------------------------------------------------------------------------

/// Sample encoding inside a RIFF/WAVE stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleFormat {
    Int,
    Float,
}

/// Stream format metadata for a RIFF/WAVE file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WavSpec {
    pub channels: u16,
    pub sample_rate: u32,
    pub bits_per_sample: u16,
    pub sample_format: SampleFormat,
}

/// Errors produced while reading or writing WAV streams.
#[derive(Debug, thiserror::Error)]
pub enum WavError {
    #[error("io error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("wav format error: {0}")]
    FormatError(&'static str),
    #[error("unsupported wav format")]
    Unsupported,
}

/// Sample type supported by [`WavReader`] and [`WavWriter`].
pub trait WavSample: Sized + Copy {
    fn read_from<R: Read>(reader: &mut R, spec: WavSpec) -> Result<Self, WavError>;
    fn write_to<W: Write>(self, writer: &mut W, spec: WavSpec) -> Result<(), WavError>;
}

impl WavSample for i16 {
    fn read_from<R: Read>(reader: &mut R, spec: WavSpec) -> Result<Self, WavError> {
        if spec.sample_format != SampleFormat::Int || spec.bits_per_sample != 16 {
            return Err(WavError::Unsupported);
        }
        let mut buf = [0u8; 2];
        reader.read_exact(&mut buf)?;
        Ok(i16::from_le_bytes(buf))
    }

    fn write_to<W: Write>(self, writer: &mut W, spec: WavSpec) -> Result<(), WavError> {
        if spec.sample_format != SampleFormat::Int || spec.bits_per_sample != 16 {
            return Err(WavError::Unsupported);
        }
        writer.write_all(&self.to_le_bytes())?;
        Ok(())
    }
}

impl WavSample for f32 {
    fn read_from<R: Read>(reader: &mut R, spec: WavSpec) -> Result<Self, WavError> {
        if spec.sample_format != SampleFormat::Float || spec.bits_per_sample != 32 {
            return Err(WavError::Unsupported);
        }
        let mut buf = [0u8; 4];
        reader.read_exact(&mut buf)?;
        Ok(f32::from_le_bytes(buf))
    }

    fn write_to<W: Write>(self, writer: &mut W, spec: WavSpec) -> Result<(), WavError> {
        if spec.sample_format != SampleFormat::Float || spec.bits_per_sample != 32 {
            return Err(WavError::Unsupported);
        }
        writer.write_all(&self.to_le_bytes())?;
        Ok(())
    }
}

/// Streaming RIFF/WAVE reader.
pub struct WavReader<R> {
    reader: R,
    spec: WavSpec,
    data_start: u64,
    data_bytes: u32,
    bytes_read: u32,
}

impl WavReader<BufReader<File>> {
    /// Open a WAV file on disk with buffered I/O.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, WavError> {
        let file = File::open(path)?;
        Self::new(BufReader::new(file))
    }
}

impl<R: Read> WavReader<R> {
    /// Parse the RIFF/WAVE header and position the reader at the first sample.
    pub fn new(mut reader: R) -> Result<Self, WavError> {
        let mut riff = [0u8; 12];
        reader.read_exact(&mut riff)?;
        if &riff[0..4] != b"RIFF" || &riff[8..12] != b"WAVE" {
            return Err(WavError::FormatError("not a RIFF/WAVE stream"));
        }

        let mut offset = 12u64;
        let mut spec: Option<WavSpec> = None;
        loop {
            let mut chunk_header = [0u8; 8];
            if let Err(err) = reader.read_exact(&mut chunk_header) {
                return if err.kind() == std::io::ErrorKind::UnexpectedEof {
                    Err(WavError::FormatError("missing data chunk"))
                } else {
                    Err(WavError::IoError(err))
                };
            }
            offset += 8;
            let chunk_id = &chunk_header[0..4];
            let chunk_len = u32::from_le_bytes([
                chunk_header[4],
                chunk_header[5],
                chunk_header[6],
                chunk_header[7],
            ]);

            if chunk_id == b"fmt " {
                if chunk_len < 16 {
                    return Err(WavError::FormatError("fmt chunk too short"));
                }
                let mut fmt_buf = vec![0u8; usize::try_from(chunk_len).unwrap_or(16)];
                reader.read_exact(&mut fmt_buf)?;
                offset += u64::from(chunk_len);
                if chunk_len % 2 == 1 {
                    let mut pad = [0u8; 1];
                    reader.read_exact(&mut pad)?;
                    offset += 1;
                }
                let raw_tag = u16::from_le_bytes([fmt_buf[0], fmt_buf[1]]);
                let channels = u16::from_le_bytes([fmt_buf[2], fmt_buf[3]]);
                let sample_rate =
                    u32::from_le_bytes([fmt_buf[4], fmt_buf[5], fmt_buf[6], fmt_buf[7]]);
                let bits_per_sample = u16::from_le_bytes([fmt_buf[14], fmt_buf[15]]);
                let sample_format = match raw_tag {
                    1 => SampleFormat::Int,
                    3 => SampleFormat::Float,
                    0xFFFE if fmt_buf.len() >= 40 => {
                        // WAVE_FORMAT_EXTENSIBLE: subformat GUID starts at byte 24.
                        let sub_tag = u16::from_le_bytes([fmt_buf[24], fmt_buf[25]]);
                        match sub_tag {
                            1 => SampleFormat::Int,
                            3 => SampleFormat::Float,
                            _ => return Err(WavError::Unsupported),
                        }
                    }
                    _ => return Err(WavError::Unsupported),
                };
                if channels == 0
                    || sample_rate == 0
                    || bits_per_sample == 0
                    || !bits_per_sample.is_multiple_of(8)
                {
                    return Err(WavError::FormatError("invalid fmt parameters"));
                }
                spec = Some(WavSpec {
                    channels,
                    sample_rate,
                    bits_per_sample,
                    sample_format,
                });
            } else if chunk_id == b"data" {
                let spec = spec.ok_or(WavError::FormatError("data chunk before fmt chunk"))?;
                return Ok(Self {
                    reader,
                    spec,
                    data_start: offset,
                    data_bytes: chunk_len,
                    bytes_read: 0,
                });
            } else {
                let skip = u64::from(chunk_len) + u64::from(chunk_len % 2);
                std::io::copy(&mut reader.by_ref().take(skip), &mut std::io::sink())?;
                offset += skip;
            }
        }
    }

    #[must_use]
    pub const fn spec(&self) -> WavSpec {
        self.spec
    }

    /// Total number of individual samples (frames × channels) in the `data` chunk.
    #[must_use]
    pub const fn len(&self) -> u32 {
        let bps = (self.spec.bits_per_sample / 8) as u32;
        match self.data_bytes.checked_div(bps) {
            Some(count) => count,
            None => 0,
        }
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Number of multi-channel frames in the `data` chunk.
    #[must_use]
    pub const fn duration(&self) -> u32 {
        let ch = self.spec.channels as u32;
        match self.len().checked_div(ch) {
            Some(frames) => frames,
            None => 0,
        }
    }

    /// Borrowed iterator over remaining samples in the `data` chunk.
    pub fn samples<S: WavSample>(&mut self) -> WavSamples<'_, R, S> {
        WavSamples {
            reader: self,
            _marker: PhantomData,
        }
    }

    /// Owning iterator over remaining samples in the `data` chunk.
    pub fn into_samples<S: WavSample>(self) -> WavIntoSamples<R, S> {
        WavIntoSamples {
            reader: self,
            _marker: PhantomData,
        }
    }

    fn read_next_sample<S: WavSample>(&mut self) -> Option<Result<S, WavError>> {
        let bps = u32::from(self.spec.bits_per_sample / 8);
        if bps == 0 || self.bytes_read.saturating_add(bps) > self.data_bytes {
            return None;
        }
        match S::read_from(&mut self.reader, self.spec) {
            Ok(sample) => {
                self.bytes_read = self.bytes_read.saturating_add(bps);
                Some(Ok(sample))
            }
            Err(err) => Some(Err(err)),
        }
    }
}

impl<R: Read + Seek> WavReader<R> {
    /// Seek to `sample_offset` (in interleaved single-channel samples) from the
    /// start of the `data` chunk.
    pub fn seek(&mut self, sample_offset: u32) -> Result<(), WavError> {
        let bps = u64::from(self.spec.bits_per_sample / 8);
        let byte_offset = u64::from(sample_offset).saturating_mul(bps);
        let clamped = byte_offset.min(u64::from(self.data_bytes));
        self.reader
            .seek(SeekFrom::Start(self.data_start.saturating_add(clamped)))?;
        self.bytes_read = u32::try_from(clamped).unwrap_or(self.data_bytes);
        Ok(())
    }
}

/// Borrowed sample iterator returned by [`WavReader::samples`].
pub struct WavSamples<'a, R, S> {
    reader: &'a mut WavReader<R>,
    _marker: PhantomData<S>,
}

impl<R: Read, S: WavSample> Iterator for WavSamples<'_, R, S> {
    type Item = Result<S, WavError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.reader.read_next_sample()
    }
}

/// Owning sample iterator returned by [`WavReader::into_samples`].
pub struct WavIntoSamples<R, S> {
    reader: WavReader<R>,
    _marker: PhantomData<S>,
}

impl<R: Read, S: WavSample> Iterator for WavIntoSamples<R, S> {
    type Item = Result<S, WavError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.reader.read_next_sample()
    }
}

/// Streaming RIFF/WAVE writer that patches chunk lengths on [`Self::flush`],
/// [`Self::finalize`], or drop.
pub struct WavWriter<W: Write + Seek> {
    writer: W,
    spec: WavSpec,
    header_start: u64,
    data_bytes: u32,
    finalized: bool,
}

impl WavWriter<BufWriter<File>> {
    /// Create or truncate a WAV file on disk with buffered I/O.
    pub fn create<P: AsRef<Path>>(path: P, spec: WavSpec) -> Result<Self, WavError> {
        let file = File::create(path)?;
        Self::new(BufWriter::new(file), spec)
    }
}

impl<W: Write + Seek> WavWriter<W> {
    /// Write the initial 44-byte RIFF/WAVE header for `spec` and prepare to stream samples.
    pub fn new(mut writer: W, spec: WavSpec) -> Result<Self, WavError> {
        if spec.channels == 0
            || spec.sample_rate == 0
            || spec.bits_per_sample == 0
            || !spec.bits_per_sample.is_multiple_of(8)
        {
            return Err(WavError::FormatError("invalid wav spec"));
        }
        let format_tag: u16 = match spec.sample_format {
            SampleFormat::Int => 1,
            SampleFormat::Float => 3,
        };
        let bytes_per_sample = spec.bits_per_sample / 8;
        let block_align = spec.channels.saturating_mul(bytes_per_sample);
        let byte_rate = spec.sample_rate.saturating_mul(u32::from(block_align));
        let header_start = writer.stream_position()?;

        let mut header = [0u8; WAV_HEADER_LEN];
        header[0..4].copy_from_slice(b"RIFF");
        header[4..8].copy_from_slice(&36u32.to_le_bytes());
        header[8..12].copy_from_slice(b"WAVE");
        header[12..16].copy_from_slice(b"fmt ");
        header[16..20].copy_from_slice(&16u32.to_le_bytes());
        header[20..22].copy_from_slice(&format_tag.to_le_bytes());
        header[22..24].copy_from_slice(&spec.channels.to_le_bytes());
        header[24..28].copy_from_slice(&spec.sample_rate.to_le_bytes());
        header[28..32].copy_from_slice(&byte_rate.to_le_bytes());
        header[32..34].copy_from_slice(&block_align.to_le_bytes());
        header[34..36].copy_from_slice(&spec.bits_per_sample.to_le_bytes());
        header[36..40].copy_from_slice(b"data");
        header[40..44].copy_from_slice(&0u32.to_le_bytes());
        writer.write_all(&header)?;

        Ok(Self {
            writer,
            spec,
            header_start,
            data_bytes: 0,
            finalized: false,
        })
    }

    #[must_use]
    pub const fn spec(&self) -> WavSpec {
        self.spec
    }

    /// Append one sample in little-endian wire format.
    pub fn write_sample<S: WavSample>(&mut self, sample: S) -> Result<(), WavError> {
        let bps = u32::from(self.spec.bits_per_sample / 8);
        let next_data = self
            .data_bytes
            .checked_add(bps)
            .filter(|&size| size.checked_add(36).is_some())
            .ok_or(WavError::FormatError("wav file exceeds 4 GiB RIFF limit"))?;
        sample.write_to(&mut self.writer, self.spec)?;
        self.data_bytes = next_data;
        Ok(())
    }

    /// Update the RIFF and `data` chunk sizes in-place and flush the underlying writer.
    pub fn flush(&mut self) -> Result<(), WavError> {
        let end_pos = self.writer.stream_position()?;
        let riff_size = 36u32.saturating_add(self.data_bytes);
        self.writer
            .seek(SeekFrom::Start(self.header_start.saturating_add(4)))?;
        self.writer.write_all(&riff_size.to_le_bytes())?;
        self.writer
            .seek(SeekFrom::Start(self.header_start.saturating_add(40)))?;
        self.writer.write_all(&self.data_bytes.to_le_bytes())?;
        self.writer.seek(SeekFrom::Start(end_pos))?;
        self.writer.flush()?;
        Ok(())
    }

    /// Finalize header sizes, flush buffered bytes, and consume the writer.
    pub fn finalize(mut self) -> Result<(), WavError> {
        self.finalized = true;
        self.flush()
    }
}

impl<W: Write + Seek> Drop for WavWriter<W> {
    fn drop(&mut self) {
        if !self.finalized {
            let _ = self.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampled_frame_counts_keep_the_capture_rounding_contract() {
        assert_eq!(resampled_len(48_000, 48_000, 16_000), 16_000);
        assert_eq!(resampled_len(16_000, 16_000, 48_000), 48_000);
        assert_eq!(resampled_len(1, 3, 2), 0);
        assert_eq!(resampled_len(0, 48_000, 16_000), 0);
    }

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
            .as_chunks::<2>()
            .0
            .iter()
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

    #[test]
    fn streaming_reader_and_writer_round_trip_int16_float32_and_seek() {
        let spec = WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut writer = WavWriter::new(&mut cursor, spec).unwrap();
            for s in [10i16, -10, 200, -200, 3000, -3000] {
                writer.write_sample(s).unwrap();
            }
            writer.finalize().unwrap();
        }
        let mut reader = WavReader::new(std::io::Cursor::new(cursor.into_inner())).unwrap();
        assert_eq!(reader.spec(), spec);
        assert_eq!(reader.len(), 6);
        assert_eq!(reader.duration(), 3);
        reader.seek(2).unwrap();
        let tail: Vec<i16> = reader.samples::<i16>().map(Result::unwrap).collect();
        assert_eq!(tail, vec![200, -200, 3000, -3000]);

        let fspec = WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 32,
            sample_format: SampleFormat::Float,
        };
        let mut fcursor = std::io::Cursor::new(Vec::new());
        {
            let mut writer = WavWriter::new(&mut fcursor, fspec).unwrap();
            for s in [0.0f32, 0.5, -0.75] {
                writer.write_sample(s).unwrap();
            }
            writer.finalize().unwrap();
        }
        let reader = WavReader::new(std::io::Cursor::new(fcursor.into_inner())).unwrap();
        let floats: Vec<f32> = reader.into_samples::<f32>().map(Result::unwrap).collect();
        assert_eq!(floats, vec![0.0, 0.5, -0.75]);

        // Truncated data chunk must surface an error rather than ending silently.
        let mut truncated = encode_wav_16bit_mono(&[0.25, -0.25, 0.5], 16_000).unwrap();
        truncated.truncate(truncated.len() - 3);
        let reader = WavReader::new(std::io::Cursor::new(truncated)).unwrap();
        assert!(reader.into_samples::<i16>().any(|item| item.is_err()));
    }
}
