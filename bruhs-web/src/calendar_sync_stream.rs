//! Subscriber for the /api/calendar/events/stream SSE endpoint.
//!
//! Each `CalendarSyncEvent` triggers a write into the IDB stores plus a
//! reducer update on the in-memory signal:
//!   - `EventUpserted` — insert/replace by `google_event_id`
//!   - `EventDeleted`  — remove the matching event
//!   - `SyncStarted` / `SyncCompleted` / `SyncFailed` — update the
//!     GoogleCalendarSyncState (last_synced_at, last_error)
//!   - `Disconnected` — mark disconnected, do NOT delete events
//!     (the user explicitly clears them via "Remove all Google events"
//!     in Phase 3+)

use bruhs_models::EpochMs;
use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{EventSource, MessageEvent};

use crate::state::{Calendar, CalendarEvent, GoogleCalendarSyncState, SchedulingPreferences};
use bruhs_models::CalendarSyncEvent;

#[derive(Clone, Copy)]
struct CalendarState {
    events_read: ReadSignal<Vec<CalendarEvent>>,
    set_events: WriteSignal<Vec<CalendarEvent>>,
    sync_state_read: ReadSignal<GoogleCalendarSyncState>,
    set_sync_state: WriteSignal<GoogleCalendarSyncState>,
    set_calendars: WriteSignal<Vec<Calendar>>,
    scheduling_preferences: RwSignal<SchedulingPreferences>,
    saves: crate::task_saves::TaskSaves,
}

// Built from the shared API origin (portless-aware) at call time, since
// `concat!` can't join a non-literal const.

/// Subscribe to the stream and update the calendar signals on every
/// event. Wires once per app lifetime; the underlying EventSource
/// auto-reconnects on transient errors.
pub fn subscribe(
    events_read: ReadSignal<Vec<CalendarEvent>>,
    set_events: WriteSignal<Vec<CalendarEvent>>,
    sync_state_read: ReadSignal<GoogleCalendarSyncState>,
    set_sync_state: WriteSignal<GoogleCalendarSyncState>,
    set_calendars: WriteSignal<Vec<Calendar>>,
    scheduling_preferences: RwSignal<SchedulingPreferences>,
) {
    // Capture app-owned context before entering native EventSource callbacks.
    let state = CalendarState {
        events_read,
        set_events,
        sync_state_read,
        set_sync_state,
        set_calendars,
        scheduling_preferences,
        saves: expect_context(),
    };
    let executor = expect_context::<crate::workflow_executor::WorkflowExecutor>();
    let pending = RwSignal::new(Vec::<CalendarSyncEvent>::new());
    Effect::new(move |_| {
        // A provider update needs the hydrated original for CAS and local-edit
        // conflict detection. Keep stream ordering, including SyncCompleted,
        // while an initial load or its Retry is still pending.
        if !executor.ready.get() || pending.with(Vec::is_empty) {
            return;
        }
        for event in pending.try_update(std::mem::take).unwrap_or_default() {
            apply_event(event, state);
        }
    });
    let stream_url = format!("{}/api/calendar/events/stream", crate::api::BASE_URL);
    let src = match EventSource::new(&stream_url) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("calendar events EventSource failed to open: {e:?}");
            return;
        }
    };

    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |ev: MessageEvent| {
        let Some(data) = ev.data().as_string() else {
            return;
        };
        match serde_json::from_str::<CalendarSyncEvent>(&data) {
            Ok(event) => {
                pending.update(|events| events.push(event));
            }
            Err(e) => log::warn!("malformed calendar sync event: {e}; data: {data}"),
        }
    });
    src.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    on_message.forget();
}

fn apply_event(event: CalendarSyncEvent, state: CalendarState) {
    let CalendarState {
        events_read,
        set_events,
        sync_state_read,
        set_sync_state,
        set_calendars,
        scheduling_preferences,
        saves,
    } = state;
    match event {
        CalendarSyncEvent::StreamOpened => {}
        CalendarSyncEvent::SyncStarted { at_ms: _ } => {
            set_sync_state.update(|s| {
                s.last_error = None;
            });
        }
        CalendarSyncEvent::EventUpserted { event } => {
            let event = *event;
            let google_id = event.google_event_id.clone();
            let existing_event = google_id.as_ref().and_then(|gid| {
                events_read.with_untracked(|events| {
                    events
                        .iter()
                        .find(|existing| existing.google_event_id.as_ref() == Some(gid))
                        .cloned()
                })
            });
            let locally_changed = existing_event.as_ref().is_some_and(|existing| {
                bruhs_models::should_defer_remote_event(
                    existing,
                    &event,
                    sync_state_read.get_untracked().last_synced_at,
                )
            });
            if let Some(existing) = existing_event.as_ref().filter(|_| locally_changed) {
                scheduling_preferences.update(|preferences| {
                    let already_pending = preferences.conflicts.iter().any(|conflict| {
                        conflict.event_id == existing.id && conflict.resolution.is_none()
                    });
                    if !already_pending {
                        preferences
                            .conflicts
                            .push(bruhs_models::SchedulingConflict {
                                id: uuid::Uuid::new_v4().to_string(),
                                event_id: existing.id.clone(),
                                event_title: existing.title.clone(),
                                local_start_at_ms: existing.start_at_ms,
                                local_end_at_ms: existing.end_at_ms,
                                remote_start_at_ms: event.start_at_ms,
                                remote_end_at_ms: event.end_at_ms,
                                detected_at_ms: crate::clock::now_ms(),
                                resolution: None,
                            });
                        preferences.updated_at_ms = crate::clock::now_ms();
                    }
                });
                return;
            }
            // Provider payloads receive fresh IDs during mapping. Keep the local
            // identity so a refresh cannot orphan links or create old-ID copies.
            let event = preserve_local_identity(event, existing_event.as_ref());
            set_events.update(|list| {
                let existing = list.iter().position(|e| e.id == event.id);
                match existing {
                    Some(idx) => {
                        list[idx] = event.clone();
                    }
                    None => list.push(event.clone()),
                }
            });
            saves.edit_event(event.id.clone(), existing_event, Some(event));
        }
        CalendarSyncEvent::EventDeleted { google_event_id } => {
            let original = events_read.with_untracked(|events| {
                events
                    .iter()
                    .find(|e| e.google_event_id.as_deref() == Some(&google_event_id))
                    .cloned()
            });
            if let Some(original) = original {
                set_events.update(|list| {
                    list.retain(|e| e.google_event_id.as_deref() != Some(&google_event_id));
                });
                saves.edit_event(original.id.clone(), Some(original), None);
            }
        }
        CalendarSyncEvent::CalendarsRefreshed { calendars } => {
            // Persist each one to IDB so the picker hydrates offline.
            for c in &calendars {
                crate::storage::persist_calendar(c.clone());
            }
            set_calendars.set(calendars);
        }
        CalendarSyncEvent::SyncCompleted { at_ms, .. } => {
            set_sync_state.update(|s| {
                s.last_synced_at = Some(EpochMs::saturating_from_f64(at_ms).as_millis());
                s.last_error = None;
            });
        }
        CalendarSyncEvent::SyncFailed { at_ms, message } => {
            set_sync_state.update(|s| {
                s.last_synced_at = Some(EpochMs::saturating_from_f64(at_ms).as_millis());
                s.last_error = Some(message);
            });
        }
        CalendarSyncEvent::Disconnected => {
            set_sync_state.set(GoogleCalendarSyncState::default());
        }
    }
}

fn preserve_local_identity(
    mut incoming: CalendarEvent,
    original: Option<&CalendarEvent>,
) -> CalendarEvent {
    if let Some(original) = original {
        incoming.id.clone_from(&original.id);
        incoming.created_at = original.created_at;
    }
    incoming
}

#[cfg(test)]
mod tests {
    use super::preserve_local_identity;
    use bruhs_models::{CalendarEvent, EventSource};

    fn make_google_event(google_id: &str, title: &str) -> CalendarEvent {
        CalendarEvent {
            id: format!("local-{google_id}"),
            title: title.into(),
            start_at_ms: 0.0,
            end_at_ms: 0.0,
            all_day: false,
            color: None,
            notes: None,
            created_at: 0,
            updated_at: 0,
            source: EventSource::Google,
            google_event_id: Some(google_id.into()),
            calendar_id: None,
            attendees: vec![],
            organizer: None,
            location: None,
            conferencing: None,
            recurring_event_id: None,
            html_link: None,
        }
    }

    fn reduce_upsert(list: &mut Vec<CalendarEvent>, event: CalendarEvent) {
        let existing = event.google_event_id.as_ref().and_then(|gid| {
            list.iter()
                .position(|e| e.google_event_id.as_ref() == Some(gid))
        });
        match existing {
            Some(idx) => list[idx] = preserve_local_identity(event, Some(&list[idx])),
            None => list.push(event),
        }
    }

    fn reduce_delete(list: &mut Vec<CalendarEvent>, google_event_id: &str) {
        list.retain(|e| e.google_event_id.as_deref() != Some(google_event_id));
    }

    #[test]
    fn upsert_inserts_new_event() {
        let mut list = Vec::new();
        reduce_upsert(&mut list, make_google_event("g-1", "A"));
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title, "A");
    }

    #[test]
    fn upsert_replaces_existing_by_google_event_id() {
        let mut list = vec![make_google_event("g-1", "Old")];
        list[0].created_at = 12;
        let original_id = list[0].id.clone();
        let mut incoming = make_google_event("g-1", "New");
        incoming.id = "fresh-provider-mapping-id".into();
        reduce_upsert(&mut list, incoming);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title, "New");
        assert_eq!(list[0].id, original_id);
        assert_eq!(list[0].created_at, 12);
    }

    #[test]
    fn delete_removes_matching_event() {
        let mut list = vec![
            make_google_event("g-1", "Keep"),
            make_google_event("g-2", "Drop"),
        ];
        reduce_delete(&mut list, "g-2");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].google_event_id.as_deref(), Some("g-1"));
    }

    #[test]
    fn delete_for_unknown_id_is_noop() {
        let mut list = vec![make_google_event("g-1", "Keep")];
        reduce_delete(&mut list, "g-unknown");
        assert_eq!(list.len(), 1);
    }
}
