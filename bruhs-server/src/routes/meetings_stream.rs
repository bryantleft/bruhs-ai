//! A process-owned capture subscription feeds the shared meeting broadcast.
//! HTTP subscribers only observe events; connecting a client never starts work.

use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
};
use bruhs_core::AppState;
use bruhs_models::MeetingEvent;
use futures_core::Stream;
use std::{convert::Infallible, sync::Arc};

mod daemon;
mod linking;
mod pump;
mod recovery;

pub use pump::start;

/// GET /api/meetings/stream
pub async fn get_meetings_stream(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    Sse::new(bridge(state.subscribe_meeting_events())).keep_alive(KeepAlive::default())
}

pub(crate) fn bridge(
    mut server_rx: tokio::sync::broadcast::Receiver<MeetingEvent>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        yield Ok(Event::default().data(r#"{"kind":"stream_opened"}"#));
        loop {
            match server_rx.recv().await {
                Ok(event) => {
                    if let Ok(json) = serde_json::to_string(&event) {
                        yield Ok(Event::default().data(json));
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // Reconcile transcripts AND people after any lost capture/server event.
                    tracing::warn!("meetings SSE consumer lagged; requesting reconciliation");
                    yield Ok(Event::default().data(r#"{"kind":"stream_opened"}"#));
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    }
}

#[cfg(test)]
mod tests;
