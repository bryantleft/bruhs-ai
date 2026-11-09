//! Shared daemon state: the current session (if any) + a tokio broadcast
//! channel that consumers subscribe to for MeetingEvents.

use bruhs_models::{MeetingEvent, WhisperModel};
use std::sync::Mutex;
use tokio::sync::broadcast;

use crate::audio::MicRecorder;
use crate::video_ocr::OcrWorker;

const CHANNEL_CAPACITY: usize = 256;

/// A `/capture/start` option outside its allowed range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StartOptionError {
    #[error("max_speakers must be between 2 and 20")]
    MaxSpeakers,
    #[error("cross_meeting_linking_threshold must be between 0.5 and 0.95")]
    LinkingThreshold,
}

/// Maximum number of sub-speaker clusters, 2–20.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaxSpeakers(u32);

impl MaxSpeakers {
    pub const DEFAULT: Self = Self(6);

    pub fn new(value: u32) -> Result<Self, StartOptionError> {
        (2..=20)
            .contains(&value)
            .then_some(Self(value))
            .ok_or(StartOptionError::MaxSpeakers)
    }

    pub fn get(self) -> usize {
        self.0 as usize
    }
}

/// Cosine-similarity threshold for cross-meeting linking, 0.5–0.95.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkingThreshold(f32);

impl LinkingThreshold {
    pub const DEFAULT: Self = Self(0.75);

    /// Rejects NaN and infinities as well as out-of-range values.
    pub fn new(value: f32) -> Result<Self, StartOptionError> {
        (0.5..=0.95)
            .contains(&value)
            .then_some(Self(value))
            .ok_or(StartOptionError::LinkingThreshold)
    }

    pub fn get(self) -> f32 {
        self.0
    }
}

#[derive(Debug, Clone)]
pub struct CurrentSession {
    /// Runtime identity, distinct even when a caller reuses a meeting ID.
    pub generation: uuid::Uuid,
    pub meeting_id: String,
    pub started_at_ms: f64,
    pub mic_wav_path: std::path::PathBuf,
    pub sys_wav_path: std::path::PathBuf,
    pub mixed_wav_path: std::path::PathBuf,
    pub whisper_model: WhisperModel,
    /// Model used by the live transcriber during recording.
    /// Falls back to `whisper_model` (the final-pass model) if absent.
    /// Stored for status inspection and future per-session reporting.
    #[expect(
        dead_code,
        reason = "recorded at start for status inspection; nothing reads it back yet"
    )]
    pub live_model: Option<WhisperModel>,
    /// When false, the post-stop pipeline skips sherpa model download
    /// and sub-speaker clustering entirely — no network traffic, no compute.
    pub subspeaker_enabled: bool,
    /// When false, the server's cross-meeting linking hook creates fresh
    /// "Speaker N" profiles instead of matching against existing ones.
    /// None → use server default (true).
    pub cross_meeting_linking_auto: Option<bool>,
    /// Cosine similarity threshold for cross-meeting linking (0.5–0.95).
    /// None → use server default (0.75).
    pub cross_meeting_linking_threshold: Option<LinkingThreshold>,
    /// Maximum number of sub-speaker clusters (2–20). None → default 6.
    pub max_speakers: Option<MaxSpeakers>,
    /// Phase Vision: when true, the SCK video output is enabled (2fps) and
    /// an OcrWorker thread accumulates participant names from OCR.
    /// Default false (opt-in, privacy).
    pub video_ocr_enabled: bool,
}

pub struct CaptureState {
    /// Where audio, transcripts, models and daemon state live.
    pub data: crate::paths::DataRoot,
    /// Verified model downloads under `data`.
    pub models: crate::models_dir::ModelStore,
    /// Screen Recording permission, probed off the async workers and cached.
    pub permission: crate::capture::PermissionCache,
    /// `/enroll`'s speaker-embedding model, loaded on first use.
    #[cfg(target_os = "macos")]
    pub speaker_extractor:
        std::sync::Arc<crate::routes::enroll::LoadOnce<sherpa_rs::speaker_id::EmbeddingExtractor>>,
    pub recovery: crate::recovery::RecoveryQueue,
    pub lifecycle: crate::capture_lifecycle::CaptureLifecycle,
    pub monitor: std::sync::Arc<crate::activity_monitor::consent::MonitorConsent>,
    current: Mutex<Option<CurrentSession>>,
    mic_recorder: Mutex<Option<MicRecorder>>,
    sys_recorder: Mutex<Option<Box<dyn crate::capture::SystemAudioRecorder>>>,
    live_transcriber: Mutex<Option<crate::live_transcribe::LiveTranscriber>>,
    /// Live transcriber for the system audio channel (sys.wav → speaker "them").
    /// Only populated when system audio capture is active.
    sys_live_transcriber: Mutex<Option<crate::live_transcribe::LiveTranscriber>>,
    /// Phase Vision: OcrWorker running on a dedicated thread while recording.
    /// `Some` only on macOS when video_ocr_enabled was true at start time.
    ocr_worker: Mutex<Option<OcrWorker>>,
    pub tx: broadcast::Sender<MeetingEvent>,
}

impl CaptureState {
    pub fn new(data: crate::paths::DataRoot) -> Self {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            recovery: crate::recovery::RecoveryQueue::new(Some(data.recovery_db_path())),
            models: crate::models_dir::ModelStore::new(data.models_dir()),
            permission: crate::capture::PermissionCache::new(crate::capture::probe_permission),
            #[cfg(target_os = "macos")]
            speaker_extractor: std::sync::Arc::default(),
            data,
            lifecycle: crate::capture_lifecycle::CaptureLifecycle::default(),
            monitor: crate::activity_monitor::consent::MonitorConsent::new(),
            current: Mutex::new(None),
            mic_recorder: Mutex::new(None),
            sys_recorder: Mutex::new(None),
            live_transcriber: Mutex::new(None),
            sys_live_transcriber: Mutex::new(None),
            ocr_worker: Mutex::new(None),
            tx,
        }
    }

    /// Read-only access to the current session. The closure runs while the
    /// mutex is held, so it must be short and must not call back into
    /// `CaptureState` (deadlock) or `.await` (sync mutex across await point).
    pub fn with_current<F, R>(&self, f: F) -> R
    where
        F: FnOnce(Option<&CurrentSession>) -> R,
    {
        let cur = self
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(cur.as_ref())
    }

    /// Mutable access to the current session slot. Same constraints as
    /// `with_current` — no re-entrancy, no awaiting inside the closure.
    pub fn with_current_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Option<CurrentSession>) -> R,
    {
        let mut cur = self
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut cur)
    }

    pub fn emit(&self, event: MeetingEvent) {
        let _ = self.tx.send(event);
    }

    /// Call while holding the lifecycle reservation when installing resources.
    pub fn is_current_generation(&self, generation: uuid::Uuid) -> bool {
        self.with_current(|current| current.is_some_and(|session| session.generation == generation))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<MeetingEvent> {
        self.tx.subscribe()
    }

    pub fn set_mic_recorder(&self, recorder: MicRecorder) {
        *self
            .mic_recorder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(recorder);
    }

    pub fn take_mic_recorder(&self) -> Option<MicRecorder> {
        self.mic_recorder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    pub fn set_sys_recorder(&self, recorder: Box<dyn crate::capture::SystemAudioRecorder>) {
        *self
            .sys_recorder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(recorder);
    }

    pub fn take_sys_recorder(&self) -> Option<Box<dyn crate::capture::SystemAudioRecorder>> {
        self.sys_recorder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    pub fn has_sys_recorder(&self) -> bool {
        self.sys_recorder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    }

    pub fn set_live_transcriber(&self, t: crate::live_transcribe::LiveTranscriber) {
        *self
            .live_transcriber
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(t);
    }

    pub fn take_live_transcriber(&self) -> Option<crate::live_transcribe::LiveTranscriber> {
        self.live_transcriber
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    /// Install the sys-audio live transcriber (speaker "them").
    pub fn set_sys_live_transcriber(&self, t: crate::live_transcribe::LiveTranscriber) {
        *self
            .sys_live_transcriber
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(t);
    }

    /// Remove and return the sys-audio live transcriber if present.
    pub fn take_sys_live_transcriber(&self) -> Option<crate::live_transcribe::LiveTranscriber> {
        self.sys_live_transcriber
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    /// Install the OCR worker (video_ocr_enabled sessions).
    pub fn set_ocr_worker(&self, worker: OcrWorker) {
        *self
            .ocr_worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(worker);
    }

    /// Remove and return the OCR worker if present.
    pub fn take_ocr_worker(&self) -> Option<OcrWorker> {
        self.ocr_worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}

/// A state whose data root is a fresh temporary directory, so tests never
/// write to the real application directory. Keep the `TempDir` alive.
#[cfg(test)]
pub fn isolated_state() -> (tempfile::TempDir, CaptureState) {
    let dir = tempfile::tempdir().expect("temporary data root");
    let state = CaptureState::new(crate::paths::DataRoot::at(dir.path()));
    (dir, state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn subscribe_then_emit_receives() {
        let (_dir, state) = isolated_state();
        let mut rx = state.subscribe();
        state.emit(MeetingEvent::Started {
            meeting_id: "m-1".into(),
            started_at_ms: 1.0,
        });
        let got = rx.recv().await.unwrap();
        assert!(matches!(got, MeetingEvent::Started { .. }));
    }

    #[test]
    fn start_options_are_bounded() {
        assert_eq!(MaxSpeakers::new(2).unwrap().get(), 2);
        assert_eq!(MaxSpeakers::new(20).unwrap().get(), 20);
        for bad in [0, 1, 21, u32::MAX] {
            assert_eq!(MaxSpeakers::new(bad), Err(StartOptionError::MaxSpeakers));
        }
        for good in [0.5, 0.75, 0.95] {
            assert_eq!(LinkingThreshold::new(good).unwrap().get(), good);
        }
        for bad in [
            0.49,
            0.951,
            -0.75,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
        ] {
            assert_eq!(
                LinkingThreshold::new(bad),
                Err(StartOptionError::LinkingThreshold),
                "{bad}"
            );
        }
    }

    #[test]
    fn current_starts_empty() {
        let (_dir, state) = isolated_state();
        assert!(state
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_none());
    }

    #[test]
    fn mic_recorder_slot_starts_empty() {
        let (_dir, state) = isolated_state();
        assert!(state
            .mic_recorder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_none());
    }

    #[test]
    fn sys_recorder_slot_starts_empty() {
        let (_dir, state) = isolated_state();
        assert!(state
            .sys_recorder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_none());
    }

    #[test]
    fn live_transcriber_slot_starts_empty() {
        let (_dir, state) = isolated_state();
        assert!(state
            .live_transcriber
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_none());
    }

    #[test]
    fn sys_live_transcriber_slot_starts_empty() {
        let (_dir, state) = isolated_state();
        assert!(state
            .sys_live_transcriber
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_none());
    }
}
