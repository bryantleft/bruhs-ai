//! An in-memory ten-second microphone test with resampling for the output device.
use super::*;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

#[derive(Default)]
pub struct Meter {
    pub elapsed_ms: AtomicU32,
    pub peak_percent: AtomicU32,
}

pub fn record(
    order: &[Option<String>],
    stop: Receiver<bool>,
    meter: &Meter,
) -> Result<Clip, String> {
    let wav = capture_with_stop(order, |buffer, rate| {
        let started = Instant::now();
        loop {
            if let Ok(pcm) = buffer.lock() {
                let ms = pcm.len() as u64 * 1000 / u64::from(rate);
                meter
                    .elapsed_ms
                    .store(ms.min(10_000) as u32, Ordering::Relaxed);
                let count = (rate / 12) as usize;
                let peak = pcm[pcm.len().saturating_sub(count)..]
                    .iter()
                    .fold(0.0_f32, |acc, v| acc.max(v.abs()));
                meter
                    .peak_percent
                    .store((peak.clamp(0.0, 1.0) * 100.) as u32, Ordering::Relaxed);
            }
            if started.elapsed() >= Duration::from_secs(10) {
                return true;
            }
            match stop.recv_timeout(Duration::from_millis(75)) {
                Ok(save) => return save,
                Err(mpsc::RecvTimeoutError::Disconnected) => return false,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    })?;
    Clip::from_capture(&wav)
}

#[derive(Clone)]
pub struct Clip {
    samples: Arc<[i16]>,
    rate: u32,
}

impl Clip {
    // Input is the canonical mono PCM16 header from encode_wav_16bit_mono.
    pub(crate) fn from_capture(wav: &[u8]) -> Result<Self, String> {
        let rate = wav
            .get(24..28)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_le_bytes)
            .filter(|rate| *rate > 0)
            .ok_or("No microphone sample was recorded.")?;
        let audio = wav
            .get(bruhs_models::logic::wav::WAV_HEADER_LEN..)
            .ok_or("Incomplete microphone recording.")?;
        let samples: Vec<_> = audio
            .chunks_exact(2)
            .take(rate as usize * 10)
            .map(|v| i16::from_le_bytes([v[0], v[1]]))
            .collect();
        if samples.is_empty() {
            return Err("No microphone sample was recorded.".into());
        }
        Ok(Self {
            samples: samples.into(),
            rate,
        })
    }
    pub fn duration_ms(&self) -> u32 {
        (self.samples.len() as u64 * 1000 / u64::from(self.rate)) as u32
    }
}

#[derive(Default)]
pub struct Cursor {
    pub ms: AtomicU32,
    seek: AtomicU32,
}
impl Cursor {
    pub fn seek(&self, ms: u32) {
        self.ms.store(ms, Ordering::Relaxed);
        self.seek.store(ms + 1, Ordering::Release);
    }
}

pub fn play(clip: Clip, cursor: Arc<Cursor>, stop: Receiver<()>) -> Result<(), String> {
    let out = cpal::default_host()
        .default_output_device()
        .ok_or("No audio output device is available.")?;
    let supported = out.default_output_config().map_err(|e| e.to_string())?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let (done, finish) = mpsc::channel();
    fn build<T: cpal::SizedSample + cpal::FromSample<f32>>(
        out: &cpal::Device,
        config: &cpal::StreamConfig,
        clip: Clip,
        cursor: Arc<Cursor>,
        done: mpsc::Sender<Result<Instant, String>>,
    ) -> Result<cpal::Stream, cpal::BuildStreamError> {
        let output_rate = u64::from(config.sample_rate.0);
        let ratio = u64::from(clip.rate);
        let mut position =
            u64::from(cursor.ms.load(Ordering::Relaxed)) * u64::from(clip.rate) * output_rate
                / 1000;
        let channels = usize::from(config.channels);
        let mut complete = false;
        let errors = done.clone();
        out.build_output_stream(
            config,
            move |buffer: &mut [T], info| {
                let requested = cursor.seek.swap(0, Ordering::AcqRel);
                if requested > 0 {
                    position = u64::from(requested - 1) * ratio * output_rate / 1000;
                }
                let end = clip.samples.len() as u64 * output_rate;
                for frame in buffer.chunks_mut(channels) {
                    let index = (position / output_rate) as usize;
                    let value = clip
                        .samples
                        .get(index)
                        .map_or(0., |&s| f32::from(s) / 32768.);
                    frame.fill(T::from_sample(value));
                    position = (position + ratio).min(end);
                }
                cursor.ms.store(
                    (position * 1000 / (ratio * output_rate)) as u32,
                    Ordering::Relaxed,
                );
                if position == end && !complete {
                    complete = true;
                    let stamp = info.timestamp();
                    let latency = stamp
                        .playback
                        .duration_since(&stamp.callback)
                        .unwrap_or_default();
                    let span = Duration::from_secs_f64(
                        (buffer.len() / channels) as f64 / output_rate as f64,
                    );
                    let _ = done.send(Ok(Instant::now() + latency + span));
                }
            },
            move |e| {
                let _ = errors.send(Err(e.to_string()));
            },
            None,
        )
    }
    let stream = match format {
        cpal::SampleFormat::F32 => build::<f32>(&out, &config, clip, cursor, done),
        cpal::SampleFormat::I16 => build::<i16>(&out, &config, clip, cursor, done),
        cpal::SampleFormat::U16 => build::<u16>(&out, &config, clip, cursor, done),
        other => return Err(format!("Unsupported playback format {other:?}.")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    loop {
        if !matches!(stop.try_recv(), Err(mpsc::TryRecvError::Empty)) {
            return Ok(());
        }
        match finish.recv_timeout(Duration::from_millis(20)) {
            Ok(result) => {
                let remaining = result?.saturating_duration_since(Instant::now());
                let _ = stop.recv_timeout(remaining);
                return Ok(());
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("The audio output disconnected.".into())
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires a working host audio output"]
    fn microphone_sample_plays_synthetic_silence_from_a_seek_position() {
        let bytes = encode_wav_16bit_mono(&vec![0.; 32_000], 16_000).unwrap();
        let clip = Clip::from_capture(&bytes).unwrap();
        let cursor = Arc::new(Cursor::default());
        cursor.seek(1500);
        let (_keep, stop) = mpsc::channel();
        play(clip, cursor.clone(), stop).unwrap();
        assert_eq!(cursor.ms.load(Ordering::Relaxed), 2000);
    }

    #[test]
    fn microphone_clip_retains_pcm_and_bounds_duration() {
        let bytes = encode_wav_16bit_mono(&vec![0.5; 176_000], 16_000).unwrap();
        let clip = Clip::from_capture(&bytes).unwrap();
        assert_eq!(clip.duration_ms(), 10_000);
        assert!(clip.samples.iter().all(|v| *v > 16_000));
    }
}
