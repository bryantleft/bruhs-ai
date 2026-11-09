//! GET /events — SSE bridge from the daemon's broadcast channel.

use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
};
use bruhs_models::MeetingEvent;
use futures_core::Stream;
use std::convert::Infallible;
use std::sync::Arc;

use crate::state::CaptureState;

/// GET /events
pub async fn get_events(
    State(state): State<Arc<CaptureState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    Sse::new(events(state.subscribe())).keep_alive(KeepAlive::default())
}

fn events(
    mut rx: tokio::sync::broadcast::Receiver<MeetingEvent>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        yield Ok(Event::default().data(r#"{"kind":"stream_opened"}"#));
        loop {
            match rx.recv().await {
                Ok(event) => {
                    match serde_json::to_string(&event) {
                        Ok(json) => yield Ok(Event::default().data(json)),
                        Err(error) => {
                            tracing::warn!("capture event serialization failed: {error}");
                            yield Ok(Event::default().data(r#"{"kind":"stream_opened"}"#));
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // A slow observer does not mean recording failed. Existing
                    // clients use this hint to recover metadata/transcript gaps.
                    tracing::warn!("capture SSE consumer lagged; requesting reconciliation");
                    yield Ok(Event::default().data(r#"{"kind":"stream_opened"}"#));
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    }
}

#[cfg(test)]
mod tests;
