//! One pre-write boundary for microphone PCM and level metering. Muted content
//! becomes equal-length silence; downstream WAV readers never receive it.
//! The conferencing adapter and hardware timestamp mapping remain separate work.
use bruhs_models::num::{ToFloatLossy, ToIntSaturating};

mod gate;
pub use gate::{
    MicDecision, MicrophoneGate, MuteBinding, MuteSignal, SampleWindow, MUTE_SIGNAL_TTL,
};

use std::io::{Seek, Write};

#[derive(Debug, thiserror::Error)]
pub enum MicrophoneWriteError {
    #[error("microphone writer requires 16-bit PCM and a nonzero sample rate/channel count")]
    InvalidFormat,
    #[error(transparent)]
    Wav(#[from] hound::Error),
    #[error("microphone stream: {0}")]
    Stream(String),
}

/// Holds no raw audio queue. Mutating its policy requires exclusive access to
/// the same object that commits PCM, so a policy update cannot race a write.
pub struct MicrophoneWriter<W: Write + Seek> {
    writer: hound::WavWriter<W>,
    gate: MicrophoneGate,
    rms_sum: f64,
    rms_count: usize,
    rms_interval: usize,
    last_boundary: (u64, MicDecision),
    failure: Option<MicrophoneWriteError>,
    failure_reported: bool,
}

impl<W: Write + Seek> MicrophoneWriter<W> {
    pub fn new(
        writer: hound::WavWriter<W>,
        gate: MicrophoneGate,
    ) -> Result<Self, MicrophoneWriteError> {
        let spec = writer.spec();
        if spec.bits_per_sample != 16
            || spec.sample_format != hound::SampleFormat::Int
            || spec.channels == 0
            || spec.sample_rate == 0
        {
            return Err(MicrophoneWriteError::InvalidFormat);
        }
        Ok(Self {
            writer,
            gate,
            rms_sum: 0.0,
            rms_count: 0,
            rms_interval: ((u64::from(spec.sample_rate) * u64::from(spec.channels) / 4).max(1))
                as usize,
            last_boundary: (0, MicDecision::Unknown),
            failure: None,
            failure_reported: false,
        })
    }

    pub fn gate_mut(&mut self) -> &mut MicrophoneGate {
        &mut self.gate
    }

    /// A failed input stream cannot resume this recording or erase an earlier
    /// disk failure. The owner must still stop the device and finalize the WAV.
    pub fn fail_stream(&mut self, message: String) {
        if self.failure.is_none() {
            self.failure = Some(MicrophoneWriteError::Stream(message));
            self.rms_sum = 0.0;
            self.rms_count = 0;
        }
    }

    /// Report once without consuming the error needed by finalization.
    pub fn take_failure_notice(&mut self) -> Option<&MicrophoneWriteError> {
        if self.failure_reported {
            return None;
        }
        let failure = self.failure.as_ref()?;
        self.failure_reported = true;
        Some(failure)
    }

    pub fn write_i16(&mut self, data: &[i16], frame: Option<SampleWindow>) -> Option<f32> {
        self.write(data.iter().copied(), frame)
    }

    pub fn write_f32(&mut self, data: &[f32], frame: Option<SampleWindow>) -> Option<f32> {
        self.write(
            data.iter().map(|sample| {
                if sample.is_finite() {
                    (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).to_i16_saturating()
                } else {
                    0
                }
            }),
            frame,
        )
    }

    fn write(
        &mut self,
        samples: impl Iterator<Item = i16>,
        frame: Option<SampleWindow>,
    ) -> Option<f32> {
        if self.failure.is_some() {
            return None;
        }
        let decision = self.gate.decision(frame);
        let boundary = (self.gate.revision(), decision);
        if self.last_boundary != boundary {
            self.rms_sum = 0.0;
            self.rms_count = 0;
            self.last_boundary = boundary;
        }
        for sample in samples {
            let pcm = if decision == MicDecision::Capture {
                sample
            } else {
                0
            };
            if let Err(error) = self.writer.write_sample(pcm) {
                self.failure = Some(error.into());
                self.rms_sum = 0.0;
                self.rms_count = 0;
                return None;
            }
            let normalized = f64::from(pcm) / 32768.0;
            self.rms_sum += normalized * normalized;
            self.rms_count += 1;
        }
        if self.rms_count < self.rms_interval {
            return None;
        }
        let rms = (self.rms_sum / self.rms_count.to_f64_lossy())
            .sqrt()
            .clamp(0.0, 1.0)
            .to_f32_lossy();
        self.rms_sum = 0.0;
        self.rms_count = 0;
        Some(rms)
    }

    /// Finalize even after a callback write error, then report the first failure
    /// instead of silently claiming that a truncated recording was successful.
    pub fn finalize(self) -> Result<(), MicrophoneWriteError> {
        let finalized = self.writer.finalize();
        if let Some(error) = self.failure {
            return Err(error);
        }
        finalized.map_err(Into::into)
    }
}

#[cfg(test)]
mod tests;
