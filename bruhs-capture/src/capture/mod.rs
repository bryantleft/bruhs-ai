//! Cross-platform system-audio capture. macOS uses ScreenCaptureKit;
//! Linux and Windows are no-ops.

use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("Screen Recording permission not granted")]
    Permission,
    #[cfg_attr(
        target_os = "macos",
        expect(dead_code, reason = "constructed on non-macOS targets only")
    )]
    #[error("platform does not support system audio capture")]
    Unsupported,
    #[error("backend: {0}")]
    Backend(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("hound: {0}")]
    Hound(#[from] hound::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionStatus {
    Granted,
    Denied,
    NotDetermined,
    #[cfg_attr(
        target_os = "macos",
        expect(dead_code, reason = "constructed on non-macOS targets only")
    )]
    Unsupported,
}

pub trait SystemAudioRecorder: Send {
    fn stop(self: Box<Self>) -> Result<(), CaptureError>;

    /// Extract the video frame channel if this recorder was started with
    /// `video_ocr_enabled = true`. Default impl returns `None`.
    #[cfg(target_os = "macos")]
    fn take_video_frames(
        &mut self,
    ) -> Option<crate::video_ocr::FrameChannel<crate::video_ocr::SendPixelBuf>> {
        None
    }
}

/// Probe system-audio permission. Blocking: run it on a blocking thread
/// (`PermissionCache` does).
pub fn probe_permission() -> PermissionStatus {
    #[cfg(target_os = "macos")]
    {
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => macos::permission_status_blocking(&handle),
            Err(_) => PermissionStatus::NotDetermined,
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        PermissionStatus::Unsupported
    }
}

/// `/status` answers from a cached permission probe. The probe can take
/// seconds (ScreenCaptureKit round trip), so it runs on a blocking thread,
/// at most once per [`PermissionCache::TTL`], and is abandoned after
/// [`PermissionCache::TIMEOUT`].
pub struct PermissionCache {
    probe: std::sync::Arc<dyn Fn() -> PermissionStatus + Send + Sync>,
    entry: tokio::sync::Mutex<Option<(tokio::time::Instant, PermissionStatus)>>,
}

impl PermissionCache {
    pub const TTL: std::time::Duration = std::time::Duration::from_secs(5);
    pub const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(4);

    pub fn new(probe: impl Fn() -> PermissionStatus + Send + Sync + 'static) -> Self {
        Self {
            probe: std::sync::Arc::new(probe),
            entry: tokio::sync::Mutex::new(None),
        }
    }

    /// The cached status, probing first if it is missing or stale.
    /// Concurrent callers share one probe.
    pub async fn status(&self) -> PermissionStatus {
        let mut entry = self.entry.lock().await;
        if let Some((probed_at, status)) = *entry {
            if probed_at.elapsed() < Self::TTL {
                return status;
            }
        }
        let probe = std::sync::Arc::clone(&self.probe);
        let status =
            match tokio::time::timeout(Self::TIMEOUT, tokio::task::spawn_blocking(move || probe()))
                .await
            {
                Ok(Ok(status)) => status,
                Ok(Err(error)) => {
                    tracing::warn!("permission probe failed: {error}");
                    PermissionStatus::NotDetermined
                }
                Err(_) => {
                    tracing::warn!("permission probe timed out");
                    PermissionStatus::NotDetermined
                }
            };
        *entry = Some((tokio::time::Instant::now(), status));
        status
    }
}

pub fn start(out_path: &Path) -> Result<Box<dyn SystemAudioRecorder>, CaptureError> {
    start_with_opts(out_path, false)
}

/// Like [`start`] but with explicit video-OCR control. When
/// `video_ocr_enabled` is `true` the recorder captures video frames at 2fps
/// in addition to audio and exposes them via `SystemAudioRecorder::take_video_frames`.
pub fn start_with_opts(
    out_path: &Path,
    video_ocr_enabled: bool,
) -> Result<Box<dyn SystemAudioRecorder>, CaptureError> {
    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(macos::MacSystemRecorder::start(
            out_path,
            video_ocr_enabled,
        )?))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (out_path, video_ocr_enabled);
        Err(CaptureError::Unsupported)
    }
}

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc, Mutex};

    #[tokio::test(start_paused = true)]
    async fn permission_is_probed_at_most_once_per_ttl() {
        let probes = Arc::new(AtomicUsize::new(0));
        let seen = probes.clone();
        let cache = PermissionCache::new(move || {
            seen.fetch_add(1, Ordering::SeqCst);
            PermissionStatus::Denied
        });
        for _ in 0..3 {
            assert_eq!(cache.status().await, PermissionStatus::Denied);
        }
        assert_eq!(probes.load(Ordering::SeqCst), 1);
        tokio::time::advance(PermissionCache::TTL).await;
        assert_eq!(cache.status().await, PermissionStatus::Denied);
        assert_eq!(probes.load(Ordering::SeqCst), 2);
    }

    /// H10: `/status` used to build a runtime on a new thread and `join()` it
    /// inside the async handler, parking a tokio worker for up to 3 s on every
    /// poll. The probe now runs on a blocking thread and is abandoned at the
    /// timeout while the async task stays free.
    #[tokio::test(start_paused = true)]
    async fn hung_probe_reads_as_not_determined_after_the_timeout() {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let (started_tx, release_rx) = (Mutex::new(started_tx), Mutex::new(release_rx));
        let cache = Arc::new(PermissionCache::new(move || {
            started_tx.lock().unwrap().send(()).unwrap();
            // Released only at the end of the test.
            release_rx.lock().unwrap().recv().unwrap();
            PermissionStatus::Granted
        }));
        let waiting = tokio::spawn({
            let cache = cache.clone();
            async move { cache.status().await }
        });
        tokio::task::spawn_blocking(move || started_rx.recv().unwrap())
            .await
            .unwrap();
        assert!(!waiting.is_finished());
        tokio::time::advance(PermissionCache::TIMEOUT).await;
        assert_eq!(waiting.await.unwrap(), PermissionStatus::NotDetermined);
        // The timeout result is cached, so a hung probe is not re-spawned on
        // every poll.
        assert_eq!(cache.status().await, PermissionStatus::NotDetermined);
        release_tx.send(()).unwrap();
    }
}
