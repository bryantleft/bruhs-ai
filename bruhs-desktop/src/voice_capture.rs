//! Microphone capture for voice enrollment — the desktop's stand-in for
//! the web's `capture_voice_sample_wav` (Web Audio). Records the default
//! input device for `duration`, downmixes to mono, and returns 16-bit PCM
//! WAV bytes at the device's sample rate (the capture daemon resamples).

use bruhs_models::num::ToFloatLossy;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use bruhs_models::logic::wav::encode_wav_16bit_mono;

pub fn capture_voice_sample_wav(duration: Duration) -> Result<Vec<u8>, String> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "no input device".to_string())?;
    let supported = device
        .default_input_config()
        .map_err(|e| format!("input config: {e}"))?;
    let sample_rate = supported.sample_rate().0;
    let channels = supported.channels().max(1) as usize;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();

    let samples: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&samples);
    let on_error = |e: cpal::StreamError| tracing::warn!("input stream error: {e}");
    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_input_stream::<f32, _, _>(
            &config,
            move |data, _| push_mono(&sink, data.iter().copied(), channels),
            on_error,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream::<i16, _, _>(
            &config,
            move |data, _| {
                push_mono(
                    &sink,
                    data.iter().map(|&s| f32::from(s) / f32::from(i16::MAX)),
                    channels,
                )
            },
            on_error,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_input_stream::<u16, _, _>(
            &config,
            move |data, _| {
                push_mono(
                    &sink,
                    data.iter().map(|&s| {
                        (f32::from(s) - f32::from(u16::MAX) / 2.0) / (f32::from(u16::MAX) / 2.0)
                    }),
                    channels,
                )
            },
            on_error,
            None,
        ),
        other => return Err(format!("unsupported sample format: {other:?}")),
    }
    .map_err(|e| format!("build input stream: {e}"))?;
    stream
        .play()
        .map_err(|e| format!("start input stream: {e}"))?;
    std::thread::sleep(duration);
    drop(stream);

    let pcm = samples.lock().map_err(|_| "sample buffer poisoned")?;
    if pcm.is_empty() {
        return Err("no audio samples captured".to_string());
    }
    let encoded = encode_wav_16bit_mono(&pcm, sample_rate);
    drop(pcm);
    encoded.map_err(|error| error.to_string())
}

/// Average interleaved frames down to one channel.
fn push_mono(sink: &Arc<Mutex<Vec<f32>>>, data: impl Iterator<Item = f32>, channels: usize) {
    let Ok(mut out) = sink.lock() else { return };
    let mut frame = Vec::with_capacity(channels);
    for sample in data {
        frame.push(sample);
        if frame.len() == channels {
            out.push(frame.iter().sum::<f32>() / channels.to_f32_lossy());
            frame.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::push_mono;
    use std::sync::{Arc, Mutex};

    #[test]
    fn push_mono_averages_interleaved_channels() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        push_mono(&sink, [1.0, 0.0, 0.5, 0.5, 0.2].into_iter(), 2);
        assert_eq!(*sink.lock().unwrap(), vec![0.5, 0.5]);
    }
}
