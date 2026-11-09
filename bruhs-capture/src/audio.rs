//! Microphone capture via cpal → WAV file via hound.
//!
//! `MicRecorder::start` returns immediately; cpal runs the stream
//! callback on its own thread, which forwards i16 samples into a
//! `std::sync::Mutex<MicrophoneWriter>`. `stop` finalizes the writer.
//!
//! cpal's `Stream` is NOT `Send`, so we keep it owned by a dedicated
//! recorder thread and communicate via `mpsc::channel`. The recorder
//! thread parks on `mpsc::Receiver::recv` for a "stop" signal, then
//! drops the stream + writer and returns.
//!
//! Emits `MeetingEvent::AudioLevel` every ~250ms
//! when `event_tx` and `meeting_id` are provided. Samples are accumulated
//! in a rolling window; RMS is computed and broadcast at each interval.
//! A callback/stream failure latches writes off, emits one nonterminal warning,
//! and remains an error when Stop finalizes the recording.

use bruhs_capture::microphone::{MicrophoneGate, MicrophoneWriteError, MicrophoneWriter};
use bruhs_models::MeetingEvent;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::path::Path;
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use tokio::sync::broadcast;

/// Data and error callbacks share one failure boundary. A warning must not mark
/// the meeting stopped while the remote recorder still needs explicit cleanup.
fn process_microphone<W: std::io::Write + std::io::Seek>(
    writer: &Mutex<Option<MicrophoneWriter<W>>>,
    event_tx: Option<&broadcast::Sender<MeetingEvent>>,
    meeting_id: Option<&str>,
    operation: impl FnOnce(&mut MicrophoneWriter<W>) -> Option<f32>,
) {
    let mut guard = match writer.lock() {
        Ok(guard) => guard,
        Err(error) => {
            let mut guard = error.into_inner();
            if let Some(writer) = guard.as_mut() {
                writer.fail_stream("audio callback failed".into());
            }
            guard
        }
    };
    let Some(writer) = guard.as_mut() else {
        return;
    };
    let rms = operation(writer);
    let warning = writer.take_failure_notice().map(|error| {
        format!("Microphone recording failed: {error}. Recording is incomplete; stop and restart.")
    });
    drop(guard);
    if let Some(message) = warning {
        tracing::warn!("{message}");
        if let (Some(tx), Some(id)) = (event_tx, meeting_id) {
            let _ = tx.send(MeetingEvent::CaptureWarning {
                meeting_id: id.to_owned(),
                message,
            });
        }
    } else if let (Some(tx), Some(id), Some(rms)) = (event_tx, meeting_id, rms) {
        let _ = tx.send(MeetingEvent::AudioLevel {
            meeting_id: id.to_owned(),
            mic_rms: rms,
            sys_rms: None,
        });
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("no default input device")]
    NoDefaultInput,
    #[error("device config: {0}")]
    DeviceConfig(#[source] cpal::DefaultStreamConfigError),
    #[error("build stream: {0}")]
    BuildStream(#[source] cpal::BuildStreamError),
    #[error("play stream: {0}")]
    PlayStream(#[source] cpal::PlayStreamError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("hound: {0}")]
    Hound(#[from] hound::Error),
    #[error("microphone write: {0}")]
    MicrophoneWrite(#[from] MicrophoneWriteError),
    #[error("unsupported sample format: {0}")]
    UnsupportedSampleFormat(String),
}

pub struct MicRecorder {
    stop_tx: mpsc::Sender<()>,
    join: Option<JoinHandle<Result<(), AudioError>>>,
}

impl MicRecorder {
    #[cfg(test)]
    pub(crate) fn synthetic_completed() -> Self {
        let (stop_tx, stop_rx) = mpsc::channel();
        let join = std::thread::spawn(move || {
            stop_rx
                .recv()
                .map_err(|error| AudioError::Io(std::io::Error::other(error)))?;
            Ok(())
        });
        Self {
            stop_tx,
            join: Some(join),
        }
    }

    /// Start capturing the default input device to `wav_path`. Returns
    /// once cpal has accepted the stream; the recording runs on a
    /// background thread until `stop()` is called.
    ///
    /// If `event_tx` and `meeting_id` are provided, emits
    /// `MeetingEvent::AudioLevel` approximately every 250ms.
    pub fn start(
        wav_path: &Path,
        event_tx: Option<broadcast::Sender<MeetingEvent>>,
        meeting_id: Option<String>,
    ) -> Result<Self, AudioError> {
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let path = wav_path.to_path_buf();
        let (ready_tx, ready_rx) = mpsc::sync_channel::<()>(1);
        let join = std::thread::spawn(move || -> Result<(), AudioError> {
            let host = cpal::default_host();
            let device = host
                .default_input_device()
                .ok_or(AudioError::NoDefaultInput)?;
            let supported = device
                .default_input_config()
                .map_err(AudioError::DeviceConfig)?;
            let sample_rate = supported.sample_rate().0;
            let channels = supported.channels();
            let sample_format = supported.sample_format();

            let spec = hound::WavSpec {
                channels,
                sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let file = std::io::BufWriter::new(crate::paths::create_private_file(&path)?);
            let writer = hound::WavWriter::new(file, spec)?;
            // Automatic meeting-mute detection is not connected yet. Existing
            // starts remain ordinary capture; unsupported request flags still
            // fail validation instead of pretending to enable protection.
            let writer = MicrophoneWriter::new(writer, MicrophoneGate::ordinary())?;
            let writer = Arc::new(Mutex::new(Some(writer)));

            let config: cpal::StreamConfig = supported.into();
            let error_writer = writer.clone();
            let error_tx = event_tx.clone();
            let error_meeting = meeting_id.clone();
            let err_fn = move |err: cpal::StreamError| {
                process_microphone(
                    &error_writer,
                    error_tx.as_ref(),
                    error_meeting.as_deref(),
                    |writer| {
                        writer.fail_stream(err.to_string());
                        None
                    },
                );
            };

            let stream = match sample_format {
                cpal::SampleFormat::I16 => {
                    let w = writer.clone();
                    let tx = event_tx.clone();
                    let mid = meeting_id.clone();
                    device
                        .build_input_stream::<i16, _, _>(
                            &config,
                            move |data, _info| {
                                process_microphone(&w, tx.as_ref(), mid.as_deref(), |writer| {
                                    writer.write_i16(data, None)
                                });
                            },
                            err_fn,
                            None,
                        )
                        .map_err(AudioError::BuildStream)?
                }
                cpal::SampleFormat::F32 => {
                    let w = writer.clone();
                    let tx = event_tx.clone();
                    let mid = meeting_id.clone();
                    device
                        .build_input_stream::<f32, _, _>(
                            &config,
                            move |data, _info| {
                                process_microphone(&w, tx.as_ref(), mid.as_deref(), |writer| {
                                    writer.write_f32(data, None)
                                });
                            },
                            err_fn,
                            None,
                        )
                        .map_err(AudioError::BuildStream)?
                }
                other => {
                    return Err(AudioError::UnsupportedSampleFormat(format!("{other:?}")));
                }
            };

            stream.play().map_err(AudioError::PlayStream)?;
            let _ = ready_tx.send(());

            // Block until stop signal. cpal keeps recording on its own
            // audio thread; we just wait for the user to ask us to stop.
            let _ = stop_rx.recv();

            // Drop the stream first so no more samples land.
            drop(stream);
            if let Some(writer) = writer
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                writer.finalize()?;
            }
            Ok(())
        });

        let join = crate::recorder_start::await_ready(ready_rx, join).map_err(|error| {
            use crate::recorder_start::StartupError;
            match error {
                StartupError::Worker(error) => error,
                StartupError::Panicked => AudioError::Io(std::io::Error::other(
                    "recorder thread panicked during startup",
                )),
                StartupError::ExitedBeforeReady => {
                    AudioError::Io(std::io::Error::other("recorder thread exited before ready"))
                }
            }
        })?;
        Ok(Self {
            stop_tx,
            join: Some(join),
        })
    }

    /// Stop the recording and finalize the WAV file.
    pub fn stop(mut self) -> Result<(), AudioError> {
        let _ = self.stop_tx.send(());
        if let Some(join) = self.join.take() {
            match join.join() {
                Ok(res) => res,
                Err(_) => Err(AudioError::Io(std::io::Error::other(
                    "recorder thread panicked",
                ))),
            }
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // No test in this module opens a device.
    #[test]
    fn error_display() {
        let e = AudioError::NoDefaultInput;
        assert!(format!("{e}").contains("no default input"));
    }

    fn synthetic_writer() -> Mutex<Option<MicrophoneWriter<std::io::Cursor<Vec<u8>>>>> {
        let wav = hound::WavWriter::new(
            std::io::Cursor::new(Vec::new()),
            hound::WavSpec {
                channels: 1,
                sample_rate: 8,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        Mutex::new(Some(
            MicrophoneWriter::new(wav, MicrophoneGate::ordinary()).unwrap(),
        ))
    }

    #[test]
    fn callback_failure_warns_once_without_claiming_capture_stopped() {
        let writer = synthetic_writer();
        let (tx, mut events) = broadcast::channel(8);
        process_microphone(&writer, Some(&tx), Some("synthetic"), |writer| {
            writer.write_i16(&[123, 456], None)
        });
        assert!(matches!(
            events.try_recv().unwrap(),
            MeetingEvent::AudioLevel { .. }
        ));
        for _ in 0..3 {
            process_microphone(&writer, Some(&tx), Some("synthetic"), |writer| {
                writer.fail_stream("device disconnected".into());
                None
            });
            process_microphone(&writer, Some(&tx), Some("synthetic"), |writer| {
                writer.write_f32(&[1.0; 8], None)
            });
        }
        match events.try_recv().unwrap() {
            MeetingEvent::CaptureWarning {
                meeting_id,
                message,
            } => {
                assert_eq!(meeting_id, "synthetic");
                assert!(message.contains("device disconnected"));
                assert!(message.contains("stop and restart"));
            }
            event => panic!("expected nonterminal warning, got {event:?}"),
        }
        assert!(
            events.try_recv().is_err(),
            "must not repeat warnings, meters or terminal events"
        );
        let writer = writer.into_inner().unwrap().unwrap();
        let (stop_tx, stop_rx) = mpsc::channel();
        let join = std::thread::spawn(move || {
            stop_rx.recv().unwrap();
            writer.finalize().map_err(AudioError::from)
        });
        let recorder = MicRecorder {
            stop_tx,
            join: Some(join),
        };
        assert!(recorder
            .stop()
            .unwrap_err()
            .to_string()
            .contains("device disconnected"));
    }

    #[test]
    fn poisoned_callback_fails_closed_and_preserves_stop_error() {
        let writer = synthetic_writer();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = writer.lock().unwrap();
            panic!("synthetic callback panic");
        }));
        let (tx, mut events) = broadcast::channel(4);
        process_microphone(&writer, Some(&tx), Some("synthetic"), |writer| {
            writer.write_i16(&[123; 8], None)
        });
        assert!(matches!(
            events.try_recv().unwrap(),
            MeetingEvent::CaptureWarning { .. }
        ));
        assert!(events.try_recv().is_err());
        let writer = writer
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .unwrap();
        assert!(writer
            .finalize()
            .unwrap_err()
            .to_string()
            .contains("audio callback failed"));
    }

    #[test]
    fn absent_subscriber_or_finalized_writer_does_not_panic() {
        let writer = synthetic_writer();
        process_microphone(&writer, None, None, |writer| {
            writer.fail_stream("disconnected".into());
            None
        });
        assert!(writer.lock().unwrap().take().unwrap().finalize().is_err());
        process_microphone(&writer, None, None, |_| {
            panic!("finalized writer must not be used")
        });
    }
}
