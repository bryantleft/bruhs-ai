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

impl CalendarEvent {
    /// Whether the event belongs to a read-only provider calendar. Its
    /// time, title and details come from the provider; bruhs never writes
    /// them back, so the apps must not offer to move or edit it.
    pub fn is_provider_read_only(&self) -> bool {
        !matches!(self.source, EventSource::Local)
    }

    /// Move a provider all-day event from the UTC midnights the sync engine
    /// carries (`date` fields are zone-less) to local midnights of the same
    /// dates, the way local all-day events are stored. Without it a
    /// 2026-05-15 event starts on May 14 west of UTC and spills into May 16
    /// east of it. `offset_at` is minutes east of UTC at an instant.
    /// Idempotent; returns whether the event changed.
    pub fn normalize_provider_all_day(&mut self, offset_at: impl Fn(i64) -> i32 + Copy) -> bool {
        use crate::calendar::zone::{is_unnormalized_provider_all_day, provider_all_day_to_local};
        use crate::EpochMs;
        if !self.all_day || !self.is_provider_read_only() {
            return false;
        }
        let start = EpochMs::saturating_from_f64(self.start_at_ms).as_millis();
        let end = EpochMs::saturating_from_f64(self.end_at_ms).as_millis();
        if !is_unnormalized_provider_all_day(start, offset_at) {
            return false;
        }
        self.start_at_ms =
            EpochMs::saturating_from_i64(provider_all_day_to_local(start, offset_at)).as_f64();
        if is_unnormalized_provider_all_day(end, offset_at) {
            self.end_at_ms =
                EpochMs::saturating_from_i64(provider_all_day_to_local(end, offset_at)).as_f64();
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::attendee::{AttendeeResponse, EventAttendee, EventSource};

    fn google_all_day(start: f64, end: f64) -> CalendarEvent {
        let mut event: CalendarEvent = serde_json::from_str(
            r#"{"id":"g","title":"Holiday","start_at_ms":0.0,"end_at_ms":0.0,"all_day":true,
                "color":null,"notes":null,"created_at":0,"updated_at":0,"source":"google",
                "google_event_id":"evt"}"#,
        )
        .unwrap();
        event.start_at_ms = start;
        event.end_at_ms = end;
        event
    }

    #[test]
    fn provider_all_day_events_move_to_local_midnight_once() {
        let pacific = |_: i64| -7 * 60;
        // 2026-05-15 and 2026-05-16 at UTC midnight.
        let mut event = google_all_day(1_778_803_200_000.0, 1_778_889_600_000.0);
        assert!(event.is_provider_read_only());
        assert!(event.normalize_provider_all_day(pacific));
        assert_eq!(event.start_at_ms, 1_778_803_200_000.0 + 7.0 * 3_600_000.0);
        assert_eq!(event.end_at_ms - event.start_at_ms, 86_400_000.0);
        let normalized = event.clone();
        assert!(!event.normalize_provider_all_day(pacific), "idempotent");
        assert_eq!(event, normalized);

        let mut timed = google_all_day(1_778_803_200_000.0, 1_778_806_800_000.0);
        timed.all_day = false;
        assert!(!timed.normalize_provider_all_day(pacific));
        let mut local = google_all_day(1_778_803_200_000.0, 1_778_889_600_000.0);
        local.source = EventSource::Local;
        assert!(!local.is_provider_read_only());
        assert!(!local.normalize_provider_all_day(pacific));
    }

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
