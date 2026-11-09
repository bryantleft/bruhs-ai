use super::linking::LinkError;
use super::{daemon, linking, recovery};
use bruhs_core::{capture_client, AppState};
use bruhs_models::MeetingEvent;
use futures_core::Stream;
use std::{future::Future, sync::Arc, time::Duration};
use tokio::sync::Notify;
use tokio_stream::StreamExt;

/// The server owns this guard, not an HTTP connection. Dropping it cancels
/// upstream retries and the linking future. An already-started SQLite worker
/// can still finish its atomic transaction; a completed replay is a no-op.
pub struct CapturePump(tokio::task::JoinHandle<()>);

impl Drop for CapturePump {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub fn start(state: Arc<AppState>) -> CapturePump {
    let endpoint = capture_client::endpoint();
    let upstream = daemon::events(endpoint.clone(), daemon::RetryPolicy::default());
    let app = state.clone();
    CapturePump(tokio::spawn(async move {
        tokio::select! {
            () = run(state.clone(), upstream, move |id| linking::link_centroids_for_meeting(app.clone(), id), wall_clock_ms) => {},
            () = recovery::run(state, endpoint) => {},
        }
    }))
}

/// Milliseconds since the epoch: when link jobs are due.
fn wall_clock_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(test)]
pub(super) fn spawn<S, F, Fut>(state: Arc<AppState>, upstream: S, link: F) -> CapturePump
where
    S: Stream<Item = String> + Send + 'static,
    F: FnMut(String) -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), LinkError>> + Send + 'static,
{
    spawn_with_clock(state, upstream, link, wall_clock_ms)
}

/// [`spawn`] with a test clock, so retry timing can run on paused time.
#[cfg(test)]
pub(super) fn spawn_with_clock<S, F, Fut, C>(
    state: Arc<AppState>,
    upstream: S,
    link: F,
    now_ms: C,
) -> CapturePump
where
    S: Stream<Item = String> + Send + 'static,
    F: FnMut(String) -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), LinkError>> + Send + 'static,
    C: Fn() -> i64 + Send + 'static,
{
    CapturePump(tokio::spawn(run(state, upstream, link, now_ms)))
}

async fn run<S, F, Fut, C>(state: Arc<AppState>, upstream: S, link: F, now_ms: C)
where
    S: Stream<Item = String>,
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<(), LinkError>>,
    C: Fn() -> i64,
{
    let wake = Notify::new();
    let mut upstream = std::pin::pin!(upstream);
    let mut worker = std::pin::pin!(work(&state, &wake, link, now_ms));
    loop {
        tokio::select! {
            () = &mut worker => return,
            payload = upstream.next() => {
                let Some(payload) = payload else { return };
                let event = match serde_json::from_str::<MeetingEvent>(&payload) {
                    Ok(event) => event,
                    Err(error) => {
                        tracing::warn!("invalid capture event: {error}");
                        state.emit_meeting_event(MeetingEvent::StreamOpened);
                        continue;
                    }
                };
                if let MeetingEvent::Segment { meeting_id, segment } = &event {
                    match state
                        .meetings()
                        .segment_removed(meeting_id, &segment.id)
                        .await
                    {
                        Ok(true) => continue,
                        Ok(false) => {}
                        Err(error) => {
                            tracing::warn!("could not check a deleted transcript chunk: {error}")
                        }
                    }
                }
                if let MeetingEvent::SpeakersAssigned { meeting_id } = &event {
                    // Persist before forwarding. Speech never waits for centroid
                    // HTTP or linking, and the worker has no in-memory backlog.
                    if let Err(error) = state.meetings().enqueue_speaker_link(meeting_id).await {
                        tracing::warn!("speaker linking could not be queued: {error}");
                        state.emit_meeting_event(MeetingEvent::CaptureWarning {
                            meeting_id: meeting_id.clone(),
                            message: "Speaker linking could not be saved for retry. Reload this recording to check its speakers.".into(),
                        });
                        state.emit_meeting_event(MeetingEvent::StreamOpened);
                    } else {
                        wake.notify_one();
                    }
                }
                state.emit_meeting_event(event);
            }
        }
    }
}

async fn work<F, Fut, C>(state: &AppState, wake: &Notify, mut link: F, now_ms: C)
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<(), LinkError>>,
    C: Fn() -> i64,
{
    loop {
        match state.meetings().next_speaker_link(now_ms()).await {
            Ok(Some(job)) => {
                let saved = match link(job.meeting_id.clone()).await {
                    Ok(()) => state.meetings().finish_speaker_link(&job).await,
                    Err(error) => {
                        tracing::warn!("speaker linking attempt failed: {error}");
                        linking::warn_failed(state, job.meeting_id.clone());
                        state.meetings().retry_speaker_link(&job, now_ms()).await
                    }
                };
                if saved.is_ok() {
                    continue;
                }
                tracing::warn!("speaker linking receipt could not be saved: {saved:?}");
                state.emit_meeting_event(MeetingEvent::StreamOpened);
            }
            Ok(None) => {}
            Err(error) => tracing::warn!("speaker linking queue unavailable: {error}"),
        }
        // Discover work from other connections too. One receipt/request is
        // active; a failed database never causes a busy retry loop.
        tokio::select! {
            () = wake.notified() => {},
            () = tokio::time::sleep(Duration::from_secs(1)) => {},
        }
    }
}

#[cfg(test)]
mod tests;
