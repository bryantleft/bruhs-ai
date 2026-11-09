//! SSE bridge from the SyncEngine broadcast channel to subscribed web
//! clients. Emits one `CalendarSyncEvent`-as-JSON per event. A subscriber
//! whose receiver buffer overflowed has lost events; it asks the engine for
//! a full resync, which replays every event in the sync window (upserts and
//! deletes are idempotent on the client).

use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
};
use bruhs_core::AppState;
use futures_core::Stream;
use std::convert::Infallible;
use std::sync::Arc;

/// GET /api/calendar/events/stream
pub async fn get_events_stream(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    Sse::new(event_stream(state)).keep_alive(KeepAlive::default())
}

pub(crate) fn event_stream(state: Arc<AppState>) -> impl Stream<Item = Result<Event, Infallible>> {
    let mut rx = state.google_calendar().subscribe();
    async_stream::stream! {
        // Handshake: emit a `stream_opened` frame so subscribers can
        // confirm the connection without waiting for the next tick.
        yield Ok(Event::default().data(r#"{"kind":"stream_opened"}"#));
        loop {
            match rx.recv().await {
                Ok(event) => match Event::default().json_data(&event) {
                    Ok(frame) => yield Ok(frame),
                    // Never send an empty frame the client cannot parse.
                    Err(error) => tracing::error!("calendar event not serializable: {error}"),
                },
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(
                        "calendar SSE subscriber lagged by {skipped} events; requesting a full resync"
                    );
                    state.google_calendar().force_full_sync();
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_core::google_calendar::{
        cursor::CalendarFiles, tokens::TokenStore, GoogleCalendarHandle,
    };
    use bruhs_models::{Calendar, EventSource};
    use futures_util::StreamExt;

    /// A lagged subscriber has lost events for good unless the engine
    /// replays them; it used to get a bogus `Disconnected` instead.
    #[tokio::test]
    async fn lagged_subscriber_requests_a_full_resync() {
        let tmp = tempfile::tempdir().unwrap();
        let files = CalendarFiles::at(tmp.path());
        files
            .merge_calendars(|_| {
                vec![Calendar {
                    id: "one".into(),
                    source: EventSource::Google,
                    provider_id: Some("one".into()),
                    name: "One".into(),
                    color: None,
                    sync_enabled: false,
                    primary: false,
                    last_synced_at: None,
                }]
            })
            .await
            .unwrap();
        let app = AppState::builder()
            .google_calendar(Arc::new(GoogleCalendarHandle::new(
                None,
                TokenStore::in_memory(),
                files,
            )))
            .build();
        let state = Arc::new(app);

        let mut stream = std::pin::pin!(event_stream(state.clone()));
        // Handshake frame; the receiver is now subscribed.
        assert!(stream.next().await.is_some());
        // Overflow the 256-event broadcast buffer without reading.
        for i in 0..300 {
            state
                .google_calendar()
                .patch_calendar("one", i % 2 == 0)
                .await
                .unwrap();
        }
        assert!(!state.google_calendar().full_sync_pending());
        // The next read hits the lag and asks for a replay, then keeps
        // streaming the events that are still buffered.
        assert!(stream.next().await.is_some());
        assert!(state.google_calendar().full_sync_pending());
    }
}
