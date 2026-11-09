use serde::{Deserialize, Serialize};

use crate::models::calendar::Calendar;
use crate::models::calendar_event::CalendarEvent;

/// Whether bruhs holds Google tokens, and for which account. Only a
/// connection has an account, so "disconnected as you@example.com" cannot
/// be represented.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum GoogleConnection {
    #[default]
    Disconnected,
    Connected {
        /// The account's address, when Google reported one.
        email: Option<String>,
    },
}

/// Public, secret-free view of the Google Calendar connection. Safe to
/// serialize over the wire. Tokens live in the OS keychain, never here.
///
/// The sync fields are kept whatever the connection: clients learn sync
/// times from the event stream before they fetch the connection.
///
/// On the wire this is the flat object it has always been:
/// `{connected, email, last_synced_at, last_error, calendars}`. Reading a
/// disconnected object that still names an email drops the email.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    from = "GoogleCalendarSyncStateWire",
    into = "GoogleCalendarSyncStateWire"
)]
pub struct GoogleCalendarSyncState {
    pub connection: GoogleConnection,
    pub last_synced_at: Option<i64>,
    pub last_error: Option<String>,
    pub calendars: Vec<Calendar>,
}

impl GoogleCalendarSyncState {
    /// Connected as `email`, with nothing synced yet.
    pub fn connected(email: Option<String>) -> Self {
        Self {
            connection: GoogleConnection::Connected { email },
            ..Self::default()
        }
    }

    pub fn is_connected(&self) -> bool {
        matches!(self.connection, GoogleConnection::Connected { .. })
    }

    /// The connected account's address.
    pub fn email(&self) -> Option<&str> {
        match &self.connection {
            GoogleConnection::Connected { email } => email.as_deref(),
            GoogleConnection::Disconnected => None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct GoogleCalendarSyncStateWire {
    #[serde(default)]
    connected: bool,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    last_synced_at: Option<i64>,
    #[serde(default)]
    last_error: Option<String>,
    #[serde(default)]
    calendars: Vec<Calendar>,
}

impl From<GoogleCalendarSyncStateWire> for GoogleCalendarSyncState {
    fn from(wire: GoogleCalendarSyncStateWire) -> Self {
        Self {
            connection: if wire.connected {
                GoogleConnection::Connected { email: wire.email }
            } else {
                GoogleConnection::Disconnected
            },
            last_synced_at: wire.last_synced_at,
            last_error: wire.last_error,
            calendars: wire.calendars,
        }
    }
}

impl From<GoogleCalendarSyncState> for GoogleCalendarSyncStateWire {
    fn from(state: GoogleCalendarSyncState) -> Self {
        let (connected, email) = match state.connection {
            GoogleConnection::Connected { email } => (true, email),
            GoogleConnection::Disconnected => (false, None),
        };
        Self {
            connected,
            email,
            last_synced_at: state.last_synced_at,
            last_error: state.last_error,
            calendars: state.calendars,
        }
    }
}

/// Sync-channel event broadcast from `bruhs-server` to subscribed
/// clients over `GET /api/calendar/events/stream`. All variants are
/// idempotent — replaying the stream from the beginning produces the
/// same local state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CalendarSyncEvent {
    StreamOpened,
    SyncStarted {
        at_ms: f64,
    },
    EventUpserted {
        event: Box<CalendarEvent>,
    },
    EventDeleted {
        google_event_id: String,
    },
    CalendarsRefreshed {
        calendars: Vec<Calendar>,
    },
    SyncCompleted {
        at_ms: f64,
        upserted: usize,
        deleted: usize,
    },
    SyncFailed {
        at_ms: f64,
        message: String,
    },
    Disconnected,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::attendee::EventSource;

    #[test]
    fn stream_handshake_is_a_valid_noop_event() {
        let event: CalendarSyncEvent =
            serde_json::from_str(r#"{"kind":"stream_opened"}"#).expect("handshake");
        assert_eq!(event, CalendarSyncEvent::StreamOpened);
    }

    #[test]
    fn default_is_disconnected() {
        let s = GoogleCalendarSyncState::default();
        assert!(!s.is_connected());
        assert!(s.email().is_none());
        assert!(s.calendars.is_empty());
    }

    #[test]
    fn sync_state_keeps_its_flat_wire_shape() {
        let state = GoogleCalendarSyncState {
            last_synced_at: Some(5),
            ..GoogleCalendarSyncState::connected(Some("you@example.com".into()))
        };
        assert_eq!(
            serde_json::to_value(&state).unwrap(),
            serde_json::json!({
                "connected": true,
                "email": "you@example.com",
                "last_synced_at": 5,
                "last_error": null,
                "calendars": [],
            })
        );
        assert_eq!(
            serde_json::to_value(GoogleCalendarSyncState::default()).unwrap(),
            serde_json::json!({
                "connected": false,
                "email": null,
                "last_synced_at": null,
                "last_error": null,
                "calendars": [],
            })
        );
    }

    #[test]
    fn old_sync_state_shapes_still_read() {
        // What the server has always sent (no `calendars`).
        let old: GoogleCalendarSyncState = serde_json::from_str(
            r#"{"connected":true,"email":"a@b.c","last_synced_at":7,"last_error":"boom"}"#,
        )
        .unwrap();
        assert_eq!(old.email(), Some("a@b.c"));
        assert_eq!(old.last_synced_at, Some(7));
        assert_eq!(old.last_error.as_deref(), Some("boom"));
        // A disconnected object naming an account: the account is dropped,
        // the sync fields are kept.
        let stale: GoogleCalendarSyncState = serde_json::from_str(
            r#"{"connected":false,"email":"a@b.c","last_synced_at":7,"last_error":null}"#,
        )
        .unwrap();
        assert_eq!(stale.connection, GoogleConnection::Disconnected);
        assert_eq!(stale.email(), None);
        assert_eq!(stale.last_synced_at, Some(7));
        assert_eq!(
            serde_json::from_str::<GoogleCalendarSyncState>("{}").unwrap(),
            GoogleCalendarSyncState::default()
        );
    }

    #[test]
    fn sync_state_roundtrip() {
        let s = GoogleCalendarSyncState {
            connection: GoogleConnection::Connected {
                email: Some("you@example.com".into()),
            },
            last_synced_at: Some(1_700_000_000_000),
            last_error: None,
            calendars: vec![Calendar {
                id: "cal-1".into(),
                source: EventSource::Google,
                provider_id: Some("primary".into()),
                name: "Bryant".into(),
                color: None,
                sync_enabled: true,
                primary: true,
                last_synced_at: Some(1_700_000_000_000),
            }],
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: GoogleCalendarSyncState = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn sync_event_variants_serialize_with_kind_tag() {
        let e = CalendarSyncEvent::SyncStarted { at_ms: 1.0 };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"kind\":\"sync_started\""));

        let e = CalendarSyncEvent::EventDeleted {
            google_event_id: "abc".into(),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"kind\":\"event_deleted\""));
        assert!(json.contains("\"google_event_id\":\"abc\""));

        let e = CalendarSyncEvent::Disconnected;
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(json, "{\"kind\":\"disconnected\"}");
    }

    #[test]
    fn sync_event_roundtrip_for_completed() {
        let e = CalendarSyncEvent::SyncCompleted {
            at_ms: 100.0,
            upserted: 7,
            deleted: 1,
        };
        let json = serde_json::to_string(&e).unwrap();
        let back: CalendarSyncEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
    }
}
