//! GET /api/events/stream (`bruhs_models::api::live_events`) — the web app's one live connection.
//!
//! Browsers allow six HTTP/1.1 connections per host, and the plain
//! `http://localhost` server is HTTP/1.1. One stream per source (tasks,
//! calendar, meetings) took half of them per tab, so a second tab stalled
//! every other request. This stream carries all three, each frame named by
//! its source (`event: tasks|calendar|meetings`) with the same JSON the
//! single-source streams send. The desktop app still uses those.
//!
//! `?resume=true` (`RESUME_QUERY`) marks a reopen (after an error, or a page
//! restored from the back/forward cache). Google sync tokens advance once any
//! subscriber has the changes, so another tab or the desktop may have taken
//! calendar events this client missed; a reopen asks for a full calendar
//! resync, which replays the sync window (the client applies it idempotently).

use axum::{
    extract::{Query, State},
    response::sse::{Event, KeepAlive, Sse},
};
use bruhs_core::AppState;
#[cfg(unix)]
use bruhs_models::api::live_events::WORKSPACE_BRIDGE;
use bruhs_models::api::live_events::{CALENDAR, MEETINGS, REACTIVE, TASKS};
use futures_core::Stream;
use std::{convert::Infallible, sync::Arc};
use tokio_stream::StreamExt as _;

use super::{calendar_events_stream, meetings_stream, tasks};

#[derive(Debug, Default, serde::Deserialize)]
pub struct StreamQuery {
    /// This client had the stream open before and may have missed events.
    #[serde(default)]
    resume: bool,
}

/// GET /api/events/stream
pub async fn get_events_stream(
    State(state): State<Arc<AppState>>,
    #[cfg(unix)] relay: Option<axum::Extension<crate::workspace_relay::WorkspaceRelay>>,
    Query(query): Query<StreamQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let events = events(state, query.resume);
    // Every stream opens with its relay ID (`workspace_relay`); an MCP
    // workspace session bound to it ends when the stream closes.
    #[cfg(unix)]
    let events = events.merge(named(
        relay_frames(relay.map(|axum::Extension(relay)| relay)),
        WORKSPACE_BRIDGE,
    ));
    Sse::new(events).keep_alive(KeepAlive::default())
}

#[cfg(unix)]
fn relay_frames(
    relay: Option<crate::workspace_relay::WorkspaceRelay>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        let Some(frames) = relay.as_ref().and_then(crate::workspace_relay::WorkspaceRelay::attach) else {
            return;
        };
        let mut frames = std::pin::pin!(frames);
        while let Some(frame) = frames.next().await {
            match Event::default().json_data(&frame) {
                Ok(event) => yield Ok(event),
                Err(error) => tracing::warn!("A workspace relay frame could not be encoded: {error}"),
            }
        }
    }
}

fn events(state: Arc<AppState>, resume: bool) -> impl Stream<Item = Result<Event, Infallible>> {
    let tasks = named(tasks::task_snapshots(state.registry().clone()), TASKS);
    let meetings = named(
        meetings_stream::bridge(state.subscribe_meeting_events()),
        MEETINGS,
    );
    // Subscribed before the resync is requested, so the replay reaches it.
    let calendar = named(
        calendar_events_stream::event_stream(state.clone()),
        CALENDAR,
    );
    let reactive = named(reactive_signals(state.reactive().cloned()), REACTIVE);
    if resume {
        state.google_calendar().force_full_sync();
    }
    tasks.merge(calendar).merge(meetings).merge(reactive)
}

/// Agent update counts: one frame on open, then one per change (coalesced).
fn reactive_signals(
    runtime: Option<bruhs_core::reactive::ReactiveRuntime>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        let Some(runtime) = runtime else { return; };
        let mut changes = runtime.watch_changes();
        let mut previous = None;
        loop {
            match runtime.outbox_signal().await {
                Ok(signal) if previous.as_ref() != Some(&signal) => {
                    if let Ok(event) = Event::default().json_data(&signal) {
                        yield Ok(event);
                    }
                    previous = Some(signal);
                }
                Ok(_) => {}
                Err(error) => tracing::warn!("Agent update counts are unavailable: {error}"),
            }
            if changes.changed().await.is_err() {
                return;
            }
            // Many changes land together when a run finishes.
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            changes.borrow_and_update();
        }
    }
}

fn named(
    frames: impl Stream<Item = Result<Event, Infallible>>,
    name: &'static str,
) -> impl Stream<Item = Result<Event, Infallible>> {
    frames.map(move |frame| frame.map(|event| event.event(name)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;

    /// A connection opens with a task snapshot and both handshakes, each
    /// under its own name, so one client can route them.
    #[tokio::test]
    async fn one_connection_carries_every_source() {
        let state = Arc::new(AppState::new());
        let mut body = get_events_stream(
            State(state),
            #[cfg(unix)]
            None,
            Query(StreamQuery::default()),
        )
        .await
        .into_response()
        .into_body()
        .into_data_stream();
        let mut text = String::new();
        while ["tasks", "calendar", "meetings"]
            .iter()
            .any(|name| !text.contains(&format!("event: {name}\n")))
        {
            let chunk = tokio::time::timeout(std::time::Duration::from_secs(5), body.next())
                .await
                .expect("frame before timeout")
                .expect("stream still open")
                .expect("frame");
            text.push_str(&String::from_utf8_lossy(&chunk));
        }
        let frames: Vec<&str> = text.split("\n\n").collect();
        for (name, data) in [
            ("tasks", "[]"),
            ("calendar", r#"{"kind":"stream_opened"}"#),
            ("meetings", r#"{"kind":"stream_opened"}"#),
        ] {
            assert!(
                frames.iter().any(|frame| {
                    let lines: Vec<&str> = frame.lines().collect();
                    lines.contains(&format!("event: {name}").as_str())
                        && lines.contains(&format!("data: {data}").as_str())
                }),
                "no {name} frame in {text}"
            );
        }
    }

    /// Only a reopen asks for a calendar replay; a first open does not cost
    /// a full Google listing.
    #[tokio::test]
    async fn only_a_reopen_requests_a_full_calendar_resync() {
        let state = Arc::new(AppState::new());
        drop(events(state.clone(), false));
        assert!(!state.google_calendar().full_sync_pending());
        drop(events(state.clone(), true));
        assert!(state.google_calendar().full_sync_pending());
    }

    /// The query the web app sends on a reopen must parse; a rejected one
    /// fails every reopen with 400 and the tab never reconnects.
    #[test]
    fn the_client_resume_query_parses() {
        use bruhs_models::api::live_events::{PATH, RESUME_QUERY};
        let uri: axum::http::Uri = format!("{PATH}?{RESUME_QUERY}").parse().unwrap();
        let Query(query) = Query::<StreamQuery>::try_from_uri(&uri).unwrap();
        assert!(query.resume);
        let uri: axum::http::Uri = PATH.parse().unwrap();
        let Query(query) = Query::<StreamQuery>::try_from_uri(&uri).unwrap();
        assert!(!query.resume);
    }
}
