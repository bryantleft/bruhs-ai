use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::attendee::EventSource;

/// One calendar that the user has access to. Multiple calendars can
/// coexist — the implicit "Local" calendar (no record persisted) plus
/// any number of Google calendars after the user connects.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Calendar {
    pub id: String,
    pub source: EventSource,
    /// Provider-side calendar id. For Google: `"primary"` or
    /// `"...@group.calendar.google.com"`. `None` for source=Local.
    pub provider_id: Option<String>,
    pub name: String,
    /// Bruhs color token (`"persimmon"`, `"blueberry"`, …) when known.
    /// Google's hex colors are mapped to the nearest token at sync time.
    pub color: Option<String>,
    pub sync_enabled: bool,
    /// True for the user's primary calendar (Google's `"primary"`).
    pub primary: bool,
    pub last_synced_at: Option<i64>,
}

impl Calendar {
    /// Build a fresh Google-sourced Calendar record. The local UUID is
    /// generated; provider_id is the Google calendar id.
    pub fn new_google(
        provider_id: impl Into<String>,
        name: impl Into<String>,
        primary: bool,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            source: EventSource::Google,
            provider_id: Some(provider_id.into()),
            name: name.into(),
            color: None,
            sync_enabled: primary, // primary defaults on; others opt-in
            primary,
            last_synced_at: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_google_sets_sensible_defaults() {
        let c = Calendar::new_google("primary", "Bryant", true);
        assert_eq!(c.source, EventSource::Google);
        assert_eq!(c.provider_id.as_deref(), Some("primary"));
        assert_eq!(c.name, "Bryant");
        assert!(c.primary);
        assert!(c.sync_enabled, "primary calendar defaults to sync_enabled");
        assert!(c.last_synced_at.is_none());
    }

    #[test]
    fn new_google_non_primary_is_opt_in() {
        let c = Calendar::new_google("xyz@group.calendar.google.com", "Team", false);
        assert!(!c.primary);
        assert!(!c.sync_enabled);
    }

    #[test]
    fn calendar_serde_roundtrip() {
        let c = Calendar {
            id: "cal-1".into(),
            source: EventSource::Google,
            provider_id: Some("primary".into()),
            name: "Bryant".into(),
            color: Some("persimmon".into()),
            sync_enabled: true,
            primary: true,
            last_synced_at: Some(1_700_000_000_000),
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: Calendar = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }
}
