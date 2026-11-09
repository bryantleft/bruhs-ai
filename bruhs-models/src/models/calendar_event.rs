use serde::{Deserialize, Serialize};

use crate::models::attendee::{ConferencingLink, EventAttendee, EventSource};

/// Calendar event — single record persisted to IndexedDB.
///
/// Times are stored as Unix epoch milliseconds (browser-native via `Date.now()`),
/// not chrono types, so the wire shape stays trivially serializable into a
/// `JsValue` for IDB. UI converts to local-time strings on render.
///
/// New fields below the original block are additive and either
/// `Option<T>` or `#[serde(default)]`, so records written by DB_VERSION 2
/// continue to deserialize unchanged.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CalendarEvent {
    pub id: String,
    pub title: String,
    pub start_at_ms: f64,
    pub end_at_ms: f64,
    /// All-day events ignore the time portion of start/end and span the
    /// whole day from local midnight to local midnight.
    pub all_day: bool,
    /// Bruhs token name for the event color (persimmon, blueberry, pandan…).
    /// `None` falls back to a neutral chrome tone in the UI.
    pub color: Option<String>,
    pub notes: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,

    // ── Added in Phase 0 of Calendar v1 Google Sync ──
    #[serde(default)]
    pub source: EventSource,
    #[serde(default)]
    pub google_event_id: Option<String>,
    #[serde(default)]
    pub calendar_id: Option<String>,
    #[serde(default)]
    pub attendees: Vec<EventAttendee>,
    #[serde(default)]
    pub organizer: Option<EventAttendee>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub conferencing: Option<ConferencingLink>,
    #[serde(default)]
    pub recurring_event_id: Option<String>,
    #[serde(default)]
    pub html_link: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::attendee::{AttendeeResponse, EventAttendee, EventSource};

    /// Records written by DB_VERSION 2 had no source/google/attendees
    /// fields. Verify they still deserialize with defaults.
    #[test]
    fn legacy_v2_event_deserializes_with_defaults() {
        let legacy = r#"{
            "id":"abc",
            "title":"Sync",
            "start_at_ms":1700000000000.0,
            "end_at_ms":1700003600000.0,
            "all_day":false,
            "color":"persimmon",
            "notes":null,
            "created_at":1700000000000,
            "updated_at":1700000000000
        }"#;
        let ev: CalendarEvent = serde_json::from_str(legacy).unwrap();
        assert_eq!(ev.id, "abc");
        assert_eq!(ev.source, EventSource::Local);
        assert!(ev.google_event_id.is_none());
        assert!(ev.calendar_id.is_none());
        assert!(ev.attendees.is_empty());
        assert!(ev.organizer.is_none());
        assert!(ev.location.is_none());
        assert!(ev.conferencing.is_none());
        assert!(ev.recurring_event_id.is_none());
        assert!(ev.html_link.is_none());
    }

    #[test]
    fn google_event_roundtrip() {
        let ev = CalendarEvent {
            id: "local-uuid".into(),
            title: "Sync".into(),
            start_at_ms: 1_700_000_000_000.0,
            end_at_ms: 1_700_003_600_000.0,
            all_day: false,
            color: None,
            notes: None,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
            source: EventSource::Google,
            google_event_id: Some("google-evt-id".into()),
            calendar_id: Some("cal-1".into()),
            attendees: vec![EventAttendee {
                email: "you@example.com".into(),
                display_name: Some("You".into()),
                response_status: AttendeeResponse::Accepted,
                organizer: false,
                self_: true,
                optional: false,
            }],
            organizer: None,
            location: Some("Meet".into()),
            conferencing: None,
            recurring_event_id: None,
            html_link: Some("https://calendar.google.com/event?eid=…".into()),
        };
        let json = serde_json::to_string(&ev).unwrap();
        let back: CalendarEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.google_event_id, ev.google_event_id);
        assert_eq!(back.attendees.len(), 1);
        assert_eq!(back.attendees[0].email, "you@example.com");
        assert_eq!(back.source, EventSource::Google);
    }
}
