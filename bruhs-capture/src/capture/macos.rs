#![expect(
    clippy::useless_transmute,
    reason = "cidre's `define_obj_type!` macro expands to a transmute internally"
)]

//! macOS system-audio capture via ScreenCaptureKit. Mirrors the
//! verified spike at `examples/sck_spike.rs`. The delegate writes
//! mono i16 samples (downmixed from SCK's non-interleaved float32
//! stereo) to a hound WavWriter. `stop` halts the stream and
//! finalizes the writer.
//!
//! When `video_ocr_enabled` is true a second output delegate captures
//! video frames at 2fps and pushes `cv::PixelBuf` values onto an
//! `mpsc::SyncSender` for the `OcrWorker` thread (Task T3).
//!
//! `sc::Stream` is an ObjC-bridged type that is not `Send`, so we keep it
//! owned by a dedicated recorder thread (matching `MicRecorder`'s pattern).
//! The recorder thread runs a `current_thread` tokio runtime for `start` and
//! `stop` awaits. We communicate with the outer caller via
//! `std::sync::mpsc` channels (ready signal + stop signal).

use bruhs_models::num::{ToFloatLossy, ToIntSaturating};
use std::{
    borrow::Cow,
    fs::File,
    io::{BufWriter, Seek, Write},
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
    thread::JoinHandle,
};

use cidre::{cat, cm, define_obj_type, dispatch, ns, objc, sc};
use hound::WavWriter;
use sc::stream::{Output, OutputImpl};

use super::{CaptureError, PermissionStatus};
use crate::video_ocr::{FrameChannel, OcrMsg, SendPixelBuf};

// ---- WAV writer state shared between the delegate and the recorder thread. --

struct WriterState<W: Write + Seek = BufWriter<File>> {
    wav: Option<WavWriter<W>>,
    frames_written: u64,
    sample_rate: u32,
    channels: u32,
    first_log: bool,
    failure: Option<hound::Error>,
}

impl<W: Write + Seek> WriterState<W> {
    fn write_sample(&mut self, sample: i16) -> bool {
        if self.failure.is_some() {
            return false;
        }
        let Some(wav) = self.wav.as_mut() else {
            return false;
        };
        match wav.write_sample(sample) {
            Ok(()) => {
                self.frames_written += 1;
                true
            }
            Err(error) => {
                tracing::warn!("system audio write failed: {error}");
                self.failure = Some(error);
                false
            }
        }
    }

    /// Latch a failure: later samples are dropped and stop reports it.
    fn fail(&mut self, error: hound::Error) {
        if self.failure.is_none() {
            self.failure = Some(error);
        }
    }

    fn finalize(&mut self) -> Result<(), hound::Error> {
        let finalized = self.wav.take().map_or(Ok(()), WavWriter::finalize);
        // A successful header update cannot repair missing PCM samples.
        if let Some(error) = self.failure.take() {
            return Err(error);
        }
        finalized
    }
}

// ---- Delegate inner state. --------------------------------------------------
//
// `define_obj_type!` embeds this inside an Objective-C subclass that conforms
// to `SCStreamOutput`. The delegate is created on the recorder thread, so
// thread-safety of the ObjC type is irrelevant to `Send`-ness of
// `MacSystemRecorder` itself.

#[repr(C)]
struct RecorderOutputInner {
    writer: Arc<Mutex<WriterState>>,
    /// Whether the stream delivers native-endian 32-bit float PCM, checked
    /// once on the first buffer. Every later buffer is read as `f32`.
    format: FormatCheck,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FormatCheck {
    Unchecked,
    Float32,
    Unsupported,
}

/// SCK is configured for float PCM; anything else must not be reinterpreted
/// as `f32` samples.
fn is_native_f32(asbd: &cat::audio::StreamBasicDesc) -> bool {
    asbd.format == cat::audio::Format::LINEAR_PCM
        && asbd
            .format_flags
            .contains(cat::audio::FormatFlags::IS_FLOAT)
        && asbd.is_native_endian()
        && asbd.bits_per_channel == 32
}

impl RecorderOutputInner {
    fn handle_audio(&mut self, sample_buf: &mut cm::SampleBuf) {
        let Some(format_desc) = sample_buf.format_desc() else {
            return;
        };
        let Some(asbd) = format_desc.stream_basic_desc() else {
            return;
        };
        if self.format == FormatCheck::Unchecked {
            self.format = if is_native_f32(asbd) {
                FormatCheck::Float32
            } else {
                tracing::warn!(
                    "system audio is not 32-bit float PCM ({asbd:?}); recording it failed"
                );
                self.writer.lock().unwrap_or_else(|e| e.into_inner()).fail(
                    hound::Error::FormatError("system audio stream is not 32-bit float PCM"),
                );
                FormatCheck::Unsupported
            };
        }
        if self.format != FormatCheck::Float32 {
            return;
        }
        let channels = asbd.channels_per_frame as usize;
        let sample_rate = asbd.sample_rate.to_u32_saturating();
        let num_frames = usize::try_from(sample_buf.num_samples()).unwrap_or(0);
        if num_frames == 0 || channels == 0 {
            return;
        }

        // Update writer metadata on first frame.
        {
            let mut w = self.writer.lock().unwrap_or_else(|e| e.into_inner());
            if w.first_log {
                w.sample_rate = sample_rate;
                w.channels = u32::try_from(channels).unwrap_or(u32::MAX);
            }
        }

        let list = match sample_buf.audio_buf_list::<2>() {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!("audio_buf_list err: {:?}", e);
                return;
            }
        };
        let buffers = list.list().as_slice();
        convert_and_write(buffers, num_frames, channels, &self.writer);
        drop(list);
    }
}

// ---- Audio conversion helpers (promoted from the spike). --------------------

/// The `f32` samples in one CoreAudio buffer, borrowed when the data is
/// suitably aligned and copied otherwise.
///
/// # Safety
///
/// `data` must be null or point to at least `byte_len` initialized bytes that
/// stay valid and unmodified for `'a`.
unsafe fn f32_samples<'a>(data: *const u8, byte_len: usize) -> Cow<'a, [f32]> {
    const SIZE: usize = std::mem::size_of::<f32>();
    let count = byte_len / SIZE;
    if data.is_null() || count == 0 {
        return Cow::Borrowed(&[]);
    }
    if data.align_offset(std::mem::align_of::<f32>()) == 0 {
        // SAFETY: the caller guarantees `byte_len >= count * SIZE` readable
        // bytes valid for 'a; `data` is non-null and f32-aligned, and every
        // bit pattern is a valid f32.
        Cow::Borrowed(unsafe { std::slice::from_raw_parts(data.cast::<f32>(), count) })
    } else {
        // SAFETY: as above; `u8` has no alignment requirement.
        let bytes = unsafe { std::slice::from_raw_parts(data, count * SIZE) };
        Cow::Owned(
            bytes
                .chunks_exact(SIZE)
                .map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
                .collect(),
        )
    }
}

/// The samples of one buffer delivered to the delegate callback.
fn buffer_samples(buf: &cat::audio::Buf) -> Cow<'_, [f32]> {
    // SAFETY: `buf` comes from the AudioBufferList of the CMSampleBuffer passed
    // to this delegate callback. Its block buffer keeps `data_bytes_size`
    // bytes at `data` alive and unmodified until the callback returns, which
    // outlives the borrow of `buf`. The stream format was checked to be
    // native-endian f32 PCM before any buffer is read.
    unsafe { f32_samples(buf.data, buf.data_bytes_size as usize) }
}

fn convert_and_write(
    buffers: &[cat::audio::Buf],
    num_frames: usize,
    channels: usize,
    writer_mu: &Arc<Mutex<WriterState>>,
) -> usize {
    if buffers.len() != channels {
        return write_interleaved(buffers, num_frames, channels, writer_mu);
    }

    let mut mono = vec![0.0f32; num_frames];
    for ch_buf in buffers {
        let samples = buffer_samples(ch_buf);
        for (out, &s) in mono.iter_mut().zip(samples.iter()) {
            *out += s;
        }
    }
    let inv_ch = 1.0_f32 / channels.to_f32_lossy();
    for s in mono.iter_mut() {
        *s *= inv_ch;
    }

    let mut writer = writer_mu.lock().unwrap_or_else(|e| e.into_inner());
    if writer.first_log {
        tracing::debug!(
            "first audio buf: {} frames, {} ch, {} Hz",
            num_frames,
            channels,
            writer.sample_rate
        );
        writer.first_log = false;
    }
    for &s in &mono {
        let clamped = (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)).to_i16_saturating();
        if !writer.write_sample(clamped) {
            return 0;
        }
    }
    num_frames
}

fn write_interleaved(
    buffers: &[cat::audio::Buf],
    num_frames: usize,
    channels: usize,
    writer_mu: &Arc<Mutex<WriterState>>,
) -> usize {
    let Some(buf) = buffers.first() else {
        return 0;
    };
    let samples = buffer_samples(buf);
    let n = (samples.len() / channels).min(num_frames);
    let mut writer = writer_mu.lock().unwrap_or_else(|e| e.into_inner());
    if writer.first_log {
        tracing::debug!(
            "first audio buf (interleaved): {} frames, {} ch, {} Hz",
            n,
            channels,
            writer.sample_rate
        );
        writer.first_log = false;
    }
    let inv_ch = 1.0_f32 / channels.to_f32_lossy();
    for frame in samples[..n * channels].chunks_exact(channels) {
        let sum: f32 = frame.iter().sum();
        let mono = (sum * inv_ch).clamp(-1.0, 1.0);
        let clamped = (mono * f32::from(i16::MAX)).to_i16_saturating();
        if !writer.write_sample(clamped) {
            return 0;
        }
    }
    n
}

// ---- Delegate ObjC type. ---------------------------------------------------

define_obj_type!(
    RecorderOutput + OutputImpl,
    RecorderOutputInner,
    RECORDER_OUTPUT
);

impl Output for RecorderOutput {}

#[objc::add_methods]
impl OutputImpl for RecorderOutput {
    extern "C" fn impl_stream_did_output_sample_buf(
        &mut self,
        _cmd: Option<&objc::Sel>,
        _stream: &sc::Stream,
        sample_buf: &mut cm::SampleBuf,
        kind: sc::OutputType,
    ) {
        if matches!(kind, sc::OutputType::Audio) {
            self.inner_mut().handle_audio(sample_buf);
        }
    }
}

// ---- Video delegate (for OCR frame capture) --------------------------------
//
// A separate delegate is registered for `sc::OutputType::Screen` when
// `video_ocr_enabled` is true. On each video frame it extracts the
// `cv::PixelBuf` and pushes it onto `frame_tx`. If the channel is full
// (OCR worker busy) the frame is silently dropped — better to skip than
// to back-pressure SCK.

#[repr(C)]
struct VideoOutputInner {
    /// Bounded channel to the OcrWorker thread. Capacity 1 means we drop
    /// any frame that arrives while OCR is still running on the previous one.
    /// Uses `SendPixelBuf` to satisfy `Send` for the mpsc channel.
    frame_tx: mpsc::SyncSender<OcrMsg<SendPixelBuf>>,
}

impl VideoOutputInner {
    fn handle_video(&mut self, sample_buf: &mut cm::SampleBuf) {
        let Some(pixel_buf) = sample_buf.image_buf() else {
            return;
        };
        // Non-blocking: a frame that arrives while OCR is busy (channel full)
        // or after the worker exited (disconnected) is deliberately dropped.
        if let Err(mpsc::TrySendError::Disconnected(_)) = self
            .frame_tx
            .try_send(OcrMsg::Frame(SendPixelBuf::new(pixel_buf.retained())))
        {
            tracing::trace!("OCR worker gone; dropping video frame");
        }
    }
}

define_obj_type!(VideoOutput + OutputImpl, VideoOutputInner, VIDEO_OUTPUT);

impl Output for VideoOutput {}

#[objc::add_methods]
impl OutputImpl for VideoOutput {
    extern "C" fn impl_stream_did_output_sample_buf(
        &mut self,
        _cmd: Option<&objc::Sel>,
        _stream: &sc::Stream,
        sample_buf: &mut cm::SampleBuf,
        kind: sc::OutputType,
    ) {
        if matches!(kind, sc::OutputType::Screen) {
            self.inner_mut().handle_video(sample_buf);
        }
    }
}

// ---- Public recorder struct. -----------------------------------------------

/// System audio recorder backed by ScreenCaptureKit.
///
/// `sc::Stream` is not `Send` (ObjC-bridged), so it lives entirely on a
/// dedicated recorder thread that owns a `current_thread` tokio runtime.
/// The outer struct only holds the channel handles + join handle — all of
/// which are `Send`.
pub struct MacSystemRecorder {
    stop_tx: mpsc::SyncSender<()>,
    join: Option<JoinHandle<Result<(), CaptureError>>>,
    /// Video pixel buffers. `Some` when `video_ocr_enabled` was true at start
    /// time; `None` otherwise. Consumed by OcrWorker on start.
    video_frames: Option<FrameChannel<SendPixelBuf>>,
}

impl MacSystemRecorder {
    /// Start capturing system audio to `out_path` (WAV, 48kHz mono 16-bit).
    ///
    /// When `video_ocr_enabled` is `true`, a second SCK output delegate
    /// captures video frames at 2fps and pushes `cv::PixelBuf` values onto
    /// `self.video_frame_rx`. When `false`, no video output is registered —
    /// zero extra CPU vs. audio-only mode.
    ///
    /// This function is synchronous: it blocks until SCK has started (or
    /// returned an error), then returns. Recording continues on a background
    /// thread until `stop()` is called.
    pub fn start(out_path: &Path, video_ocr_enabled: bool) -> Result<Self, CaptureError> {
        let path: PathBuf = out_path.to_path_buf();
        let (ready_tx, ready_rx) = mpsc::sync_channel::<()>(1);
        let (stop_tx, stop_rx) = mpsc::sync_channel::<()>(1);

        // Build the video channel before spawning so we can return the rx end.
        // Capacity 1: a frame arriving while OCR runs on the previous one is dropped.
        let (video_frames, video_frame_tx_opt) = if video_ocr_enabled {
            let (tx, frames) = crate::video_ocr::frame_channel(1);
            (Some(frames), Some(tx))
        } else {
            (None, None)
        };

        let join = std::thread::spawn(move || -> Result<(), CaptureError> {
            // Each recorder thread runs its own current_thread runtime so we
            // can await cidre's async APIs without requiring an outer tokio
            // context (and without needing block_in_place).
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| CaptureError::Backend(format!("tokio build: {e}")))?;

            // Run the SCK setup + wait-for-stop inside the runtime.
            rt.block_on(async move {
                // --- WAV writer ---
                let spec = hound::WavSpec {
                    channels: 1,
                    sample_rate: 48_000,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                };
                let file = BufWriter::new(crate::paths::create_private_file(&path)?);
                let wav = hound::WavWriter::new(file, spec).map_err(CaptureError::Hound)?;

                let writer = Arc::new(Mutex::new(WriterState {
                    wav: Some(wav),
                    frames_written: 0,
                    sample_rate: 48_000,
                    channels: 1,
                    first_log: true,
                    failure: None,
                }));

                // --- SCK setup (mirrors the spike) ---
                let content = sc::ShareableContent::current().await.map_err(|e| {
                    if is_user_declined(&e) {
                        CaptureError::Permission
                    } else {
                        CaptureError::Backend(format!("ShareableContent::current: {e:?}"))
                    }
                })?;

                let displays = content.displays();
                let display = displays
                    .get(0)
                    .map_err(|e| CaptureError::Backend(format!("no display: {:?}", e)))?;

                let mut cfg = sc::StreamCfg::new();
                cfg.set_captures_audio(true);
                cfg.set_excludes_current_process_audio(true);
                cfg.set_sample_rate(48_000);
                cfg.set_channel_count(2);
                // When video OCR is on, capture at 2fps at a reduced resolution
                // (1280×720 is enough for OCR of UI text). When off, keep the
                // minimal 2×2 placeholder required by SCK for audio-only.
                if video_frame_tx_opt.is_some() {
                    cfg.set_width(1280);
                    cfg.set_height(720);
                    cfg.set_minimum_frame_interval(cm::Time::new(1, 2)); // 2fps
                } else {
                    cfg.set_width(2);
                    cfg.set_height(2);
                    cfg.set_minimum_frame_interval(cm::Time::new(1, 1)); // 1fps placeholder
                }

                let windows = ns::Array::new();
                let filter = sc::ContentFilter::with_display_excluding_windows(&display, &windows);
                let stream = sc::Stream::new(&filter, &cfg);

                let delegate = RecorderOutput::with(RecorderOutputInner {
                    writer: writer.clone(),
                    format: FormatCheck::Unchecked,
                });

                let queue = dispatch::Queue::serial_with_ar_pool();
                stream
                    .add_stream_output(delegate.as_ref(), sc::OutputType::Audio, Some(&queue))
                    .map_err(|e| {
                        CaptureError::Backend(format!("add_stream_output audio: {:?}", e))
                    })?;

                // Register video output only when video_ocr_enabled.
                if let Some(frame_tx) = video_frame_tx_opt {
                    let video_delegate = VideoOutput::with(VideoOutputInner { frame_tx });
                    let video_queue = dispatch::Queue::serial_with_ar_pool();
                    stream
                        .add_stream_output(
                            video_delegate.as_ref(),
                            sc::OutputType::Screen,
                            Some(&video_queue),
                        )
                        .map_err(|e| {
                            CaptureError::Backend(format!("add_stream_output video: {:?}", e))
                        })?;
                    tracing::debug!("SystemAudioRecorder: video OCR output registered (2fps)");
                }

                stream.start().await.map_err(|e| {
                    if is_user_declined(&e) {
                        CaptureError::Permission
                    } else {
                        CaptureError::Backend(format!("stream.start: {e:?}"))
                    }
                })?;

                tracing::info!("SystemAudioRecorder: SCK stream started → {:?}", path);

                // Signal the caller that we're live.
                let _ = ready_tx.send(());

                // Park until stop is requested.
                //
                // `stop_rx.recv()` is blocking and must not run on the async
                // executor; use `spawn_blocking` so we don't stall the runtime.
                let _ = tokio::task::spawn_blocking(move || stop_rx.recv()).await;

                // --- Tear down ---
                let stopped = stream
                    .stop()
                    .await
                    .map_err(|e| CaptureError::Backend(format!("stream.stop: {:?}", e)));

                // Even a failed stop must release the stream before finalizing
                // the partial WAV, just as the microphone recorder does.
                drop(stream);

                // Finalize WAV.
                let mut w = writer.lock().unwrap_or_else(|e| e.into_inner());
                let finalized = w.finalize().map_err(CaptureError::Hound);
                stopped.and(finalized)?;

                tracing::info!(
                    "SystemAudioRecorder: finalized ({} frames)",
                    w.frames_written
                );
                drop(w);

                Ok::<(), CaptureError>(())
            })
        });

        let join = crate::recorder_start::await_ready(ready_rx, join).map_err(|error| {
            use crate::recorder_start::StartupError;
            match error {
                StartupError::Worker(error) => error,
                StartupError::Panicked => {
                    CaptureError::Backend("recorder thread panicked during startup".into())
                }
                StartupError::ExitedBeforeReady => {
                    CaptureError::Backend("recorder thread exited before ready".into())
                }
            }
        })?;
        Ok(Self {
            stop_tx,
            join: Some(join),
            video_frames,
        })
    }

    /// Stop the recording and finalize the WAV file.
    pub fn stop(mut self) -> Result<(), CaptureError> {
        let _ = self.stop_tx.send(());
        if let Some(join) = self.join.take() {
            match join.join() {
                Ok(res) => res,
                Err(_) => Err(CaptureError::Backend("recorder thread panicked".into())),
            }
        } else {
            Ok(())
        }
    }
}

impl super::SystemAudioRecorder for MacSystemRecorder {
    fn stop(self: Box<Self>) -> Result<(), CaptureError> {
        (*self).stop()
    }

    fn take_video_frames(&mut self) -> Option<FrameChannel<SendPixelBuf>> {
        self.video_frames.take()
    }
}

/// Screen Recording was refused: ScreenCaptureKit's "user declined" error
/// (`SCStreamErrorUserDeclined`, -3801 in `SCStreamErrorDomain`).
fn is_user_declined(error: &ns::Error) -> bool {
    error.code() == sc::error::code::USER_DECLINED
        && error.domain().eq_ns_string(sc::error::domain())
}

/// How long the permission probe may take before it reads as undetermined.
pub const PERMISSION_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Probe whether Screen Recording permission has been granted by asking
/// ScreenCaptureKit for shareable content: success means `Granted`, the
/// user-declined error means `Denied`, anything else (including a timeout)
/// `NotDetermined`.
///
/// Blocks for up to [`PERMISSION_PROBE_TIMEOUT`]: call it from
/// `spawn_blocking`, never on an async worker. It drives the probe with the
/// caller's tokio runtime (`handle`) instead of building a new one.
pub fn permission_status_blocking(handle: &tokio::runtime::Handle) -> PermissionStatus {
    handle.block_on(async {
        match tokio::time::timeout(PERMISSION_PROBE_TIMEOUT, sc::ShareableContent::current()).await
        {
            Ok(Ok(_)) => PermissionStatus::Granted,
            Ok(Err(e)) if is_user_declined(&e) => PermissionStatus::Denied,
            Ok(Err(_)) | Err(_) => PermissionStatus::NotDetermined,
        }
    })
}

// ---- Tests -----------------------------------------------------------------

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::time::Duration;
    use std::{
        cell::Cell,
        io::{self, Cursor},
        rc::Rc,
    };

    fn writer<W: Write + Seek>(sink: W) -> WriterState<W> {
        WriterState {
            wav: Some(
                WavWriter::new(
                    sink,
                    hound::WavSpec {
                        channels: 1,
                        sample_rate: 48_000,
                        bits_per_sample: 16,
                        sample_format: hound::SampleFormat::Int,
                    },
                )
                .expect("synthetic WAV header should be writable"),
            ),
            frames_written: 0,
            sample_rate: 48_000,
            channels: 1,
            first_log: true,
            failure: None,
        }
    }

    struct FaultSink {
        bytes: Cursor<Vec<u8>>,
        fail: Rc<Cell<bool>>,
        writes: Rc<Cell<usize>>,
    }

    impl Write for FaultSink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.writes.set(self.writes.get() + 1);
            if self.fail.get() {
                return Err(io::Error::other("synthetic system disk failure"));
            }
            self.bytes.write(bytes)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Seek for FaultSink {
        fn seek(&mut self, position: io::SeekFrom) -> io::Result<u64> {
            self.bytes.seek(position)
        }
    }

    #[test]
    fn system_pcm_failure_stops_later_writes_and_survives_header_finalization() {
        let fail = Rc::new(Cell::new(false));
        let writes = Rc::new(Cell::new(0));
        let mut state = writer(FaultSink {
            bytes: Cursor::new(Vec::new()),
            fail: fail.clone(),
            writes: writes.clone(),
        });
        assert!(state.write_sample(123));
        fail.set(true);
        assert!(!state.write_sample(456));
        let after_failure = writes.get();
        fail.set(false);
        assert!(!state.write_sample(789));
        assert_eq!(writes.get(), after_failure);
        assert_eq!(
            state.frames_written, 1,
            "count only successfully written mono frames"
        );
        assert!(state
            .finalize()
            .unwrap_err()
            .to_string()
            .contains("synthetic system disk failure"));
        assert!(
            writes.get() > after_failure,
            "partial WAV header must still finalize"
        );
        assert!(!state.write_sample(999));
    }

    #[test]
    fn system_pcm_success_round_trips_without_opening_a_device() {
        let mut bytes = Cursor::new(Vec::new());
        let mut state = writer(&mut bytes);
        for sample in [i16::MIN, 0, i16::MAX] {
            assert!(state.write_sample(sample));
        }
        assert_eq!(state.frames_written, 3);
        state.finalize().unwrap();
        assert!(!state.write_sample(123));
        drop(state);
        bytes.set_position(0);
        let pcm = hound::WavReader::new(bytes)
            .unwrap()
            .samples::<i16>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(pcm, [i16::MIN, 0, i16::MAX]);
    }

    /// The old check matched `format!("{e:?}").contains("3801")`, so any
    /// error whose text merely contained those digits read as "denied".
    #[test]
    fn only_screen_capture_user_declined_means_permission_denied() {
        let declined = ns::Error::with_domain(sc::error::domain(), -3801, None);
        assert!(is_user_declined(&declined));
        for (domain, code) in [
            (sc::error::domain(), -38010),
            (sc::error::domain(), 3801),
            (sc::error::domain(), sc::error::code::FAILED_TO_START),
            (ns::ErrorDomain::posix(), -3801),
        ] {
            let other = ns::Error::with_domain(domain, code, None);
            assert!(!is_user_declined(&other), "{code}");
        }
    }

    #[test]
    fn only_native_float_pcm_is_read_as_f32() {
        assert!(is_native_f32(&cat::audio::StreamBasicDesc::common_f32(
            48_000.0, 2, false
        )));
        assert!(is_native_f32(&cat::audio::StreamBasicDesc::common_f32(
            48_000.0, 2, true
        )));
        let mut int16 = cat::audio::StreamBasicDesc::common_f32(48_000.0, 2, true);
        int16.format_flags = cat::audio::FormatFlags::IS_SIGNED_INTEGER;
        int16.bits_per_channel = 16;
        assert!(!is_native_f32(&int16));
        let mut float64 = cat::audio::StreamBasicDesc::common_f32(48_000.0, 2, true);
        float64.bits_per_channel = 64;
        assert!(!is_native_f32(&float64));
    }

    #[test]
    fn samples_are_read_from_aligned_and_unaligned_buffers() {
        let expected = [0.25_f32, -0.5, 1.0];
        // SAFETY: points at 12 bytes of a live array.
        let aligned = unsafe { f32_samples(expected.as_ptr().cast(), 12) };
        assert!(matches!(aligned, Cow::Borrowed(_)));
        assert_eq!(&*aligned, expected);

        let mut bytes = vec![0u8; 1];
        bytes.extend(expected.iter().flat_map(|s| s.to_ne_bytes()));
        let start = bytes[1..].as_ptr();
        assert_ne!(start.align_offset(std::mem::align_of::<f32>()), 0);
        // SAFETY: points at 12 initialized bytes of a live Vec, unaligned.
        let unaligned = unsafe { f32_samples(start, 12) };
        assert!(matches!(unaligned, Cow::Owned(_)));
        assert_eq!(&*unaligned, expected);

        // SAFETY: null is allowed; a partial trailing sample is ignored.
        assert!(unsafe { f32_samples(std::ptr::null(), 12) }.is_empty());
        // SAFETY: points at 12 bytes of a live array; 7 <= 12 bytes are read.
        let partial = unsafe { f32_samples(expected.as_ptr().cast(), 7) };
        assert_eq!(&*partial, &expected[..1]);
    }

    #[test]
    fn unsupported_stream_format_fails_the_recording() {
        let mut state = writer(Cursor::new(Vec::new()));
        state.fail(hound::Error::FormatError("synthetic"));
        assert!(!state.write_sample(1));
        assert!(state.finalize().is_err());
    }

    #[test]
    #[ignore = "requires Screen Recording permission; manual only"]
    fn round_trip_writes_wav() {
        let tmp = tempfile::NamedTempFile::with_suffix(".wav").unwrap();
        let path = tmp.path().to_owned();
        let recorder = MacSystemRecorder::start(&path, false).unwrap();
        std::thread::sleep(Duration::from_millis(500));
        Box::new(recorder).stop().unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        assert!(meta.len() > 100, "WAV should have at least a header");
    }
}
