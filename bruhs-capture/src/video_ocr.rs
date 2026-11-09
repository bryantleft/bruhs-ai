//! Vision-framework OCR of conferencing-app video frames. Runs on a
//! dedicated worker thread; receives `cv::PixelBuf` values from the SCK video
//! delegate; runs `vn::RecognizeTextRequest` on each; filters to candidate
//! speaker names; accumulates with timestamps; persists to
//! `${meeting_id}.ocr.json` on stop.
//!
//! The worker loop itself is platform-independent (and tested with synthetic
//! frames); only the Vision recognizer and the pixel-buffer wrapper are
//! macOS-only.

use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

// Re-export the shared model types.
pub use bruhs_models::OcrObservation;

#[derive(Debug, thiserror::Error)]
pub enum OcrError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialize: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("thread: OcrWorker thread panicked")]
    Panicked,
}

// ─────────────────────────────────────────────────────────────────────────────
// Name-filter helpers (cross-platform, tested)
// ─────────────────────────────────────────────────────────────────────────────

/// Well-known conferencing-UI strings that are NOT participant names.
static UI_STRINGS: &[&str] = &[
    "Mute",
    "Unmute",
    "Stop",
    "Leave",
    "More",
    "Chat",
    "Participants",
    "Settings",
    "Record",
    "Reactions",
    "Share",
    "End",
];

/// Returns `true` if `text` is a plausible participant name (not a UI string,
/// right length, mostly letters).
pub fn is_candidate_name(text: &str) -> bool {
    let len = text.len();
    if !(2..=30).contains(&len) {
        return false;
    }
    // Must not be an exact UI string (case-sensitive match used by conferencing apps).
    if UI_STRINGS.contains(&text) {
        return false;
    }
    // Must start with a letter and consist of letters, spaces, hyphens, apostrophes, or dots.
    // TODO(voice-id-v3): broaden Unicode coverage — currently ASCII letters only,
    // rejects names with diacritics (Müller, Søren) or non-Latin scripts.
    let mut chars = text.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    chars.all(|c| c.is_ascii_alphabetic() || matches!(c, ' ' | '-' | '\'' | '.'))
}

// ─────────────────────────────────────────────────────────────────────────────
// Worker (cross-platform)
// ─────────────────────────────────────────────────────────────────────────────

/// What the OCR worker receives. Stop travels on the frame channel itself, so
/// stopping never waits for a frame: a static screen may produce none.
pub enum OcrMsg<T> {
    Frame(T),
    Stop,
}

/// The receiving end of a frame channel plus a sender the worker's owner uses
/// to deliver [`OcrMsg::Stop`].
pub struct FrameChannel<T> {
    frames: mpsc::Receiver<OcrMsg<T>>,
    control: mpsc::SyncSender<OcrMsg<T>>,
}

/// A bounded frame channel. The producer (the SCK video delegate) keeps the
/// returned sender and drops frames with `try_send` while OCR is busy.
pub fn frame_channel<T>(capacity: usize) -> (mpsc::SyncSender<OcrMsg<T>>, FrameChannel<T>) {
    let (sender, frames) = mpsc::sync_channel(capacity);
    let control = sender.clone();
    (sender, FrameChannel { frames, control })
}

/// OCR worker. Receives frames, runs a recognizer on each, accumulates the
/// observations with timestamps and writes them to `output_path` when it ends.
pub struct OcrWorker {
    stop: Option<Box<dyn FnOnce() + Send>>,
    join: Option<JoinHandle<Result<Vec<OcrObservation>, OcrError>>>,
}

impl OcrWorker {
    /// Spawn the worker thread. `recognize` gets each frame and the
    /// milliseconds since `start_instant`.
    pub fn spawn<T: Send + 'static>(
        channel: FrameChannel<T>,
        output_path: PathBuf,
        start_instant: Instant,
        mut recognize: impl FnMut(&T, u32) -> Vec<OcrObservation> + Send + 'static,
    ) -> Result<Self, OcrError> {
        let FrameChannel { frames, control } = channel;
        // Accumulator shared across the unwind boundary so that a panic in the
        // OCR loop doesn't silently discard already-processed frames.
        let accumulated = Arc::new(Mutex::new(Vec::<OcrObservation>::new()));
        let join = std::thread::Builder::new().name("bruhs-ocr".into()).spawn(
            move || -> Result<Vec<OcrObservation>, OcrError> {
                let unwind_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    // Ends on Stop, or when every sender is gone.
                    while let Ok(OcrMsg::Frame(frame)) = frames.recv() {
                        let timestamp_ms =
                            u32::try_from(start_instant.elapsed().as_millis()).unwrap_or(u32::MAX);
                        let observations = recognize(&frame, timestamp_ms);
                        accumulated
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .extend(observations);
                    }
                }));
                drop(frames);
                let observations = std::mem::take(
                    &mut *accumulated
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner),
                );
                if unwind_result.is_err() {
                    tracing::error!(
                        "OcrWorker panicked; persisting {} partial observations",
                        observations.len()
                    );
                }
                let json = serde_json::to_string(&observations)?;
                crate::paths::write_private_atomic(&output_path, json.as_bytes())?;
                tracing::info!(
                    "OcrWorker: wrote {} observations → {:?}",
                    observations.len(),
                    output_path
                );
                Ok(observations)
            },
        )?;
        Ok(Self {
            stop: Some(Box::new(move || {
                // Blocks at most until the worker takes the frame it is about
                // to receive. An error means the worker already exited.
                if control.send(OcrMsg::Stop).is_err() {
                    tracing::debug!("OcrWorker exited before stop");
                }
            })),
            join: Some(join),
        })
    }

    /// Signal the worker to stop and wait for it to persist its observations.
    pub fn stop(mut self) -> Result<Vec<OcrObservation>, OcrError> {
        if let Some(stop) = self.stop.take() {
            stop();
        }
        match self.join.take() {
            Some(join) => join.join().map_err(|_| OcrError::Panicked)?,
            None => Ok(Vec::new()),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// macOS implementation
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
mod macos_impl {
    use super::{is_candidate_name, FrameChannel, OcrError, OcrObservation, OcrWorker};
    use bruhs_models::OcrBoundingBox;
    use cidre::{arc, cv, ns, vn};
    use std::path::PathBuf;
    use std::time::Instant;

    /// A retained `CVPixelBuffer` that can move to the OCR thread.
    pub struct SendPixelBuf(arc::R<cv::PixelBuf>);

    // SAFETY: CVPixelBuffer is a reference-counted CoreFoundation object whose
    // retain/release are thread-safe, and CoreVideo documents pixel buffers as
    // usable from any thread. The wrapper owns one retain and only reads the
    // buffer (Vision OCR) on the receiving thread; the SCK delegate keeps no
    // reference that it mutates, so moving it cannot create aliased mutation.
    unsafe impl Send for SendPixelBuf {}

    impl SendPixelBuf {
        pub fn new(pixel_buf: arc::R<cv::PixelBuf>) -> Self {
            Self(pixel_buf)
        }

        pub fn pixel_buf(&self) -> &cv::PixelBuf {
            &self.0
        }
    }

    /// Minimum confidence threshold for an OCR candidate to be kept.
    const MIN_CONFIDENCE: f32 = 0.8;

    impl OcrWorker {
        /// Run Vision OCR on frames from the SCK video delegate.
        pub fn start(
            channel: FrameChannel<SendPixelBuf>,
            output_path: PathBuf,
            start_instant: Instant,
        ) -> Result<Self, OcrError> {
            Self::spawn(
                channel,
                output_path,
                start_instant,
                |frame, timestamp_ms| ocr_pixel_buf(frame.pixel_buf(), timestamp_ms),
            )
        }
    }

    /// Run `VNRecognizeTextRequest` on a single `CVPixelBuffer` and return
    /// filtered `OcrObservation` values.
    fn ocr_pixel_buf(pixel_buf: &cv::PixelBuf, timestamp_ms: u32) -> Vec<OcrObservation> {
        // Build a VNImageRequestHandler from the CVPixelBuffer.
        // Returns None if the pixel buffer is invalid.
        let Some(handler) = vn::ImageRequestHandler::with_cv_pixel_buf(pixel_buf, None) else {
            return Vec::new();
        };

        let mut request = vn::RecognizeTextRequest::new();
        request.set_recognition_level(vn::RequestTextRecognitionLevel::Accurate);
        request.set_uses_lang_correction(true);
        let en = ns::String::with_str("en-US");
        let langs = ns::Array::<ns::String>::from_slice(&[&en]);
        request.set_recognition_langs(&langs);

        let requests = ns::Array::<vn::Request>::from_slice(&[&request]);
        if handler.perform(&requests).is_err() {
            return Vec::new();
        }

        let Some(results) = request.results() else {
            return Vec::new();
        };

        let count = results.len();
        let mut out = Vec::new();

        for i in 0..count {
            let obs = match results.get(i) {
                Ok(o) => o,
                Err(_) => continue,
            };

            let candidates = obs.top_candidates(1);
            let Ok(top) = candidates.get(0) else {
                continue;
            };

            let text = top.string().to_string();
            let conf = top.confidence();

            if conf < MIN_CONFIDENCE {
                continue;
            }
            if !is_candidate_name(&text) {
                continue;
            }

            let r = obs.bounding_box();
            out.push(OcrObservation {
                text,
                bbox: OcrBoundingBox {
                    x: r.origin.x,
                    y: r.origin.y,
                    w: r.size.width,
                    h: r.size.height,
                },
                confidence: conf,
                timestamp_ms,
            });
        }

        out
    }
}

#[cfg(target_os = "macos")]
pub use macos_impl::SendPixelBuf;

// ─────────────────────────────────────────────────────────────────────────────
// Tests (pure, no Vision needed)
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn observation(text: &str, timestamp_ms: u32) -> OcrObservation {
        OcrObservation {
            text: text.into(),
            bbox: bruhs_models::OcrBoundingBox {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
            confidence: 1.0,
            timestamp_ms,
        }
    }

    /// Run `stop` on another thread so a regression fails instead of hanging.
    fn stop_within(worker: OcrWorker, limit: Duration) -> Result<Vec<OcrObservation>, OcrError> {
        let (done_tx, done_rx) = mpsc::channel();
        std::thread::spawn(move || done_tx.send(worker.stop()).unwrap());
        done_rx
            .recv_timeout(limit)
            .expect("OcrWorker::stop must not wait for a video frame")
    }

    /// H9: on a static screen SCK sends no frames. The worker used to block in
    /// `recv()` until the next frame and only then look for a stop signal, so
    /// `/capture/stop` hung while it still held the frame sender alive.
    #[test]
    fn stop_never_waits_for_a_frame() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("m.ocr.json");
        // The fake SCK delegate stays alive and never sends.
        let (_delegate, channel) = frame_channel::<u32>(1);
        let worker =
            OcrWorker::spawn(channel, output.clone(), Instant::now(), |_, _| Vec::new()).unwrap();
        let observations = stop_within(worker, Duration::from_secs(10)).unwrap();
        assert!(observations.is_empty());
        assert_eq!(std::fs::read_to_string(&output).unwrap(), "[]");
    }

    #[test]
    fn frames_before_stop_are_recognized_and_persisted_privately() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("m.ocr.json");
        let (delegate, channel) = frame_channel::<&'static str>(4);
        let worker = OcrWorker::spawn(channel, output.clone(), Instant::now(), |name, _| {
            vec![observation(name, 7)]
        })
        .unwrap();
        delegate.send(OcrMsg::Frame("Alice")).unwrap();
        delegate.send(OcrMsg::Frame("Bob")).unwrap();
        let observations = stop_within(worker, Duration::from_secs(10)).unwrap();
        let names: Vec<_> = observations.iter().map(|o| o.text.as_str()).collect();
        assert_eq!(names, ["Alice", "Bob"]);
        let saved: Vec<OcrObservation> =
            serde_json::from_str(&std::fs::read_to_string(&output).unwrap()).unwrap();
        assert_eq!(saved, observations);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&output).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn worker_ends_when_the_frame_source_goes_away() {
        let dir = tempfile::tempdir().unwrap();
        let (delegate, channel) = frame_channel::<u32>(1);
        let worker = OcrWorker::spawn(
            channel,
            dir.path().join("m.ocr.json"),
            Instant::now(),
            |_, _| Vec::new(),
        )
        .unwrap();
        drop(delegate);
        assert!(stop_within(worker, Duration::from_secs(10)).is_ok());
    }

    #[test]
    fn accepts_simple_names() {
        assert!(is_candidate_name("Alice"));
        assert!(is_candidate_name("Bob Smith"));
        assert!(is_candidate_name("Jean-Luc"));
        assert!(is_candidate_name("O'Brien"));
    }

    #[test]
    fn rejects_ui_strings() {
        assert!(!is_candidate_name("Mute"));
        assert!(!is_candidate_name("Leave"));
        assert!(!is_candidate_name("Participants"));
        assert!(!is_candidate_name("Settings"));
        assert!(!is_candidate_name("Chat"));
        assert!(!is_candidate_name("Record"));
    }

    #[test]
    fn rejects_too_short_or_too_long() {
        assert!(!is_candidate_name("A"));
        let long: String = "A".repeat(31);
        assert!(!is_candidate_name(&long));
    }

    #[test]
    fn rejects_non_ascii_start() {
        assert!(!is_candidate_name("1Alice"));
        assert!(!is_candidate_name(" Alice"));
    }

    #[test]
    fn rejects_digits_in_name() {
        assert!(!is_candidate_name("Alice123"));
        assert!(!is_candidate_name("Room 42"));
    }

    #[test]
    fn accepts_two_char_names() {
        assert!(is_candidate_name("Li"));
        assert!(is_candidate_name("Bo"));
    }
}
