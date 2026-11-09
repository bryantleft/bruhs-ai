//! Calendar-driven auto-recording: the user's thresholds and the pure
//! per-tick decision policy, shared by the web and the desktop. Settings
//! live in localStorage; per-event opt-outs are a separate
//! `HashSet<google_event_id>` alongside.

use crate::num::ToFloatLossy;
use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::{CalendarEvent, Meeting};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutoRecordSettings {
    /// Master switch. When false, the watcher is a no-op regardless of
    /// the other fields.
    pub enabled: bool,
    /// Minimum number of attendees on a calendar event for auto-record
    /// to consider it a meeting. 0 means "any event"; default is 2.
    pub min_attendees: u32,
    /// Seconds after the event's scheduled `end_at_ms` to keep recording
    /// before stopping. Default 60 — catches the tail of conversations
    /// that run a minute over.
    pub stop_delay_secs: i64,
    /// When true, the watcher shows a confirmation modal before
    /// starting recording on a matched calendar event. When false,
    /// recording starts silently. Default true: silent auto-start is
    /// opt-in.
    #[serde(default = "default_prompt_before_starting")]
    pub prompt_before_starting: bool,
    /// Phase 4c: when true, the activity monitor's `AudioActivityDetected`
    /// events can also trigger the auto-record prompt (if `enabled` is also
    /// true). Default **false** — opt-in for privacy reasons.
    #[serde(default)]
    pub audio_driven_enabled: bool,
}

fn default_prompt_before_starting() -> bool {
    true
}

impl Default for AutoRecordSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            min_attendees: 2,
            stop_delay_secs: 60,
            prompt_before_starting: true,
            audio_driven_enabled: false,
        }
    }
}

/// What the auto-record watcher should do this tick. Produced by [`decide`].
#[derive(Debug, Clone, PartialEq)]
pub enum AutoRecordDecision {
    /// No action this tick.
    Idle,
    /// The live meeting's linked event ended more than `stop_delay_secs`
    /// ago; stop the recording.
    AutoStop { meeting_id: String },
    /// A candidate event matched; start without prompting
    /// (`prompt_before_starting` is off).
    AutoStart(CalendarEvent),
    /// A candidate event matched; show the confirmation prompt.
    Prompt(CalendarEvent),
}

/// Read-only snapshot of every input the policy needs, so [`decide`] stays
/// pure: no signals, no clocks, no I/O.
#[derive(Debug, Clone, Copy)]
pub struct TickInput<'a> {
    pub now_ms: f64,
    pub settings: &'a AutoRecordSettings,
    pub capture_running: bool,
    /// Every calendar event the user can see (Google and local).
    pub events: &'a [CalendarEvent],
    pub meetings: &'a [Meeting],
    pub live_meeting_id: Option<&'a str>,
    pub pending: Option<&'a CalendarEvent>,
    /// Event keys the user opted out of permanently.
    pub suppressed: &'a HashSet<String>,
    /// Event keys skipped for this app session.
    pub session_skipped: &'a HashSet<String>,
    /// A start or stop is already in flight.
    pub in_flight: bool,
}

/// Decide what the watcher should do this tick.
pub fn decide(input: &TickInput<'_>) -> AutoRecordDecision {
    // Hard gates.
    if input.in_flight || !input.settings.enabled || !input.capture_running {
        return AutoRecordDecision::Idle;
    }

    // Auto-stop has priority: while a meeting is live, the only possible
    // action is stopping it.
    if let Some(live_id) = input.live_meeting_id {
        if let Some(live_meeting) = input.meetings.iter().find(|m| m.id == live_id) {
            if should_auto_stop(
                live_meeting,
                input.events,
                input.now_ms,
                input.settings.stop_delay_secs,
            ) {
                return AutoRecordDecision::AutoStop {
                    meeting_id: live_meeting.id.clone(),
                };
            }
        }
        return AutoRecordDecision::Idle;
    }

    // A prompt is already showing.
    if input.pending.is_some() {
        return AutoRecordDecision::Idle;
    }

    let candidate = input.events.iter().find(|e| {
        is_auto_start_candidate(
            e,
            input.now_ms,
            input.settings,
            input.meetings,
            input.suppressed,
            input.session_skipped,
        )
    });

    match candidate {
        None => AutoRecordDecision::Idle,
        Some(event) if input.settings.prompt_before_starting => {
            AutoRecordDecision::Prompt(event.clone())
        }
        Some(event) => AutoRecordDecision::AutoStart(event.clone()),
    }
}

/// The stable key a meeting links to an event by: the Google event id, so a
/// sync pass that mints a fresh local UUID does not re-trigger recording,
/// else the local id for events that never came from Google.
pub fn auto_record_event_key(event: &CalendarEvent) -> &str {
    event
        .google_event_id
        .as_deref()
        .unwrap_or(event.id.as_str())
}

/// Whether `event` should trigger an auto-start at `now_ms`: not opted out,
/// enough attendees, in progress, and not already recorded.
pub fn is_auto_start_candidate(
    event: &CalendarEvent,
    now_ms: f64,
    cfg: &AutoRecordSettings,
    meetings: &[Meeting],
    suppressed: &HashSet<String>,
    session_skipped: &HashSet<String>,
) -> bool {
    let key = auto_record_event_key(event);
    if suppressed.contains(key) || session_skipped.contains(key) {
        return false;
    }
    u32::try_from(event.attendees.len()).unwrap_or(u32::MAX) >= cfg.min_attendees
        && event.start_at_ms <= now_ms
        && now_ms < event.end_at_ms
        && !meetings
            .iter()
            .any(|m| m.calendar_event_id.as_deref() == Some(key))
}

/// Whether `meeting` should be auto-stopped at `now_ms`: its linked event
/// (matched by Google id, then local id) ended `stop_delay_secs` ago or more.
/// A meeting whose event no longer exists (deleted upstream) is never
/// auto-stopped.
pub fn should_auto_stop(
    meeting: &Meeting,
    events: &[CalendarEvent],
    now_ms: f64,
    stop_delay_secs: i64,
) -> bool {
    let Some(linked) = meeting.calendar_event_id.as_deref() else {
        return false;
    };
    let Some(event) = events
        .iter()
        .find(|e| e.google_event_id.as_deref() == Some(linked) || e.id == linked)
    else {
        return false;
    };
    now_ms >= event.end_at_ms + stop_delay_secs.saturating_mul(1000).to_f64_lossy()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_disabled_with_sensible_thresholds() {
        let s = AutoRecordSettings::default();
        assert!(!s.enabled);
        assert_eq!(s.min_attendees, 2);
        assert_eq!(s.stop_delay_secs, 60);
        assert!(s.prompt_before_starting);
        assert!(
            !s.audio_driven_enabled,
            "audio_driven_enabled must default to false (opt-in)"
        );
    }

    #[test]
    fn serde_roundtrip() {
        let s = AutoRecordSettings {
            enabled: true,
            min_attendees: 3,
            stop_delay_secs: 30,
            prompt_before_starting: false,
            audio_driven_enabled: true,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: AutoRecordSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn deserialize_legacy_settings_without_prompt_field() {
        let json = r#"{"enabled":true,"min_attendees":2,"stop_delay_secs":60}"#;
        let s: AutoRecordSettings = serde_json::from_str(json).unwrap();
        assert!(s.prompt_before_starting);
        // Phase 4c: audio_driven_enabled also defaults to false on legacy records.
        assert!(!s.audio_driven_enabled);
    }

    #[test]
    fn deserialize_legacy_settings_without_audio_driven_field() {
        // Records written before Phase 4c have no audio_driven_enabled.
        let json = r#"{"enabled":true,"min_attendees":2,"stop_delay_secs":60,"prompt_before_starting":true}"#;
        let s: AutoRecordSettings = serde_json::from_str(json).unwrap();
        assert!(
            !s.audio_driven_enabled,
            "audio_driven_enabled should default false on pre-4c records"
        );
    }

    // ── Policy ───────────────────────────────────────────────────────────

    use crate::{AttendeeResponse, EventAttendee, EventSource};

    fn make_event(id: &str, start_ms: f64, end_ms: f64, attendees: usize) -> CalendarEvent {
        CalendarEvent {
            id: id.into(),
            title: "Sync".into(),
            start_at_ms: start_ms,
            end_at_ms: end_ms,
            all_day: false,
            color: None,
            notes: None,
            created_at: 0,
            updated_at: 0,
            source: EventSource::Google,
            // google_event_id == id mirrors the stable key.
            google_event_id: Some(id.into()),
            calendar_id: None,
            attendees: (0..attendees)
                .map(|i| EventAttendee {
                    email: format!("a{i}@x"),
                    display_name: None,
                    response_status: AttendeeResponse::NeedsAction,
                    organizer: false,
                    self_: false,
                    optional: false,
                })
                .collect(),
            organizer: None,
            location: None,
            conferencing: None,
            recurring_event_id: None,
            html_link: None,
        }
    }

    fn enabled() -> AutoRecordSettings {
        AutoRecordSettings {
            enabled: true,
            ..AutoRecordSettings::default()
        }
    }

    fn none() -> HashSet<String> {
        HashSet::new()
    }

    #[test]
    fn candidate_match_basic() {
        let event = make_event("e-1", 100.0, 1000.0, 3);
        assert!(is_auto_start_candidate(
            &event,
            500.0,
            &enabled(),
            &[],
            &none(),
            &none()
        ));
    }

    #[test]
    fn candidate_skips_low_attendee_count() {
        let event = make_event("e-1", 100.0, 1000.0, 1);
        assert!(!is_auto_start_candidate(
            &event,
            500.0,
            &enabled(),
            &[],
            &none(),
            &none()
        ));
        let any = AutoRecordSettings {
            min_attendees: 0,
            ..enabled()
        };
        let solo = make_event("e-2", 100.0, 1000.0, 0);
        assert!(is_auto_start_candidate(
            &solo,
            500.0,
            &any,
            &[],
            &none(),
            &none()
        ));
    }

    #[test]
    fn candidate_skips_event_already_recorded() {
        let event = make_event("e-1", 100.0, 1000.0, 3);
        let mut prior = Meeting::new("Sync", 0.0);
        // Linked by google_event_id, so a re-sync with a new local UUID
        // does not re-trigger recording.
        prior.calendar_event_id = Some("e-1".into());
        assert!(!is_auto_start_candidate(
            &event,
            500.0,
            &enabled(),
            &[prior],
            &none(),
            &none()
        ));
    }

    #[test]
    fn candidate_skips_event_outside_window() {
        let event = make_event("e-1", 100.0, 1000.0, 3);
        assert!(!is_auto_start_candidate(
            &event,
            50.0,
            &enabled(),
            &[],
            &none(),
            &none()
        ));
        assert!(!is_auto_start_candidate(
            &event,
            1000.0,
            &enabled(),
            &[],
            &none(),
            &none()
        ));
        assert!(is_auto_start_candidate(
            &event,
            100.0,
            &enabled(),
            &[],
            &none(),
            &none()
        ));
    }

    #[test]
    fn candidate_skips_suppressed_and_session_skipped_events() {
        let event = make_event("g-1", 100.0, 1000.0, 3);
        let mut keys = HashSet::new();
        keys.insert("g-1".to_string());
        assert!(!is_auto_start_candidate(
            &event,
            500.0,
            &enabled(),
            &[],
            &keys,
            &none()
        ));
        assert!(!is_auto_start_candidate(
            &event,
            500.0,
            &enabled(),
            &[],
            &none(),
            &keys
        ));
    }

    #[test]
    fn local_events_key_by_their_own_id() {
        let mut event = make_event("local-1", 100.0, 1000.0, 3);
        event.google_event_id = None;
        assert_eq!(auto_record_event_key(&event), "local-1");
        let mut prior = Meeting::new("Sync", 0.0);
        prior.calendar_event_id = Some("local-1".into());
        assert!(!is_auto_start_candidate(
            &event,
            500.0,
            &enabled(),
            &[prior],
            &none(),
            &none()
        ));
    }

    #[test]
    fn auto_stop_fires_at_end_plus_delay() {
        let event = make_event("g-1", 0.0, 1000.0, 2);
        let mut meeting = Meeting::new("Sync", 0.0);
        meeting.calendar_event_id = Some("g-1".into());
        // delay = 60s → stop_at = 1000 + 60_000 = 61_000
        let events = std::slice::from_ref(&event);
        assert!(!should_auto_stop(&meeting, events, 60_999.0, 60));
        assert!(should_auto_stop(&meeting, events, 61_000.0, 60));
    }

    #[test]
    fn auto_stop_no_op_without_link_or_event() {
        let event = make_event("g-1", 0.0, 1000.0, 2);
        let unlinked = Meeting::new("Sync", 0.0);
        assert!(!should_auto_stop(
            &unlinked,
            std::slice::from_ref(&event),
            999_999.0,
            0
        ));
        let mut orphan = Meeting::new("Sync", 0.0);
        orphan.calendar_event_id = Some("g-1".into());
        assert!(!should_auto_stop(&orphan, &[], 999_999.0, 0));
        assert!(should_auto_stop(&orphan, &[event], 999_999.0, 0));
    }

    #[test]
    fn auto_stop_matches_a_local_event_by_id() {
        let mut event = make_event("local-1", 0.0, 1000.0, 0);
        event.google_event_id = None;
        let mut meeting = Meeting::new("Sync", 0.0);
        meeting.calendar_event_id = Some("local-1".into());
        assert!(should_auto_stop(&meeting, &[event], 1000.0, 0));
    }

    #[test]
    fn decide_prompts_for_a_live_event_and_idles_when_gated() {
        let events = [make_event("e-1", 100.0, 1000.0, 3)];
        let cfg = enabled();
        let none = none();
        let base = TickInput {
            now_ms: 500.0,
            settings: &cfg,
            capture_running: true,
            events: &events,
            meetings: &[],
            live_meeting_id: None,
            pending: None,
            suppressed: &none,
            session_skipped: &none,
            in_flight: false,
        };
        assert!(matches!(decide(&base), AutoRecordDecision::Prompt(_)));
        let no_daemon = TickInput {
            capture_running: false,
            ..base
        };
        assert_eq!(decide(&no_daemon), AutoRecordDecision::Idle);
        let busy = TickInput {
            in_flight: true,
            ..base
        };
        assert_eq!(decide(&busy), AutoRecordDecision::Idle);
        let pending = TickInput {
            pending: Some(&events[0]),
            ..base
        };
        assert_eq!(decide(&pending), AutoRecordDecision::Idle);
        let disabled = AutoRecordSettings::default();
        let off = TickInput {
            settings: &disabled,
            ..base
        };
        assert_eq!(decide(&off), AutoRecordDecision::Idle);
        let silent = AutoRecordSettings {
            prompt_before_starting: false,
            ..enabled()
        };
        let auto = TickInput {
            settings: &silent,
            ..base
        };
        assert!(matches!(decide(&auto), AutoRecordDecision::AutoStart(_)));
    }

    #[test]
    fn decide_auto_stops_after_the_event_plus_delay() {
        let events = [make_event("g-1", 0.0, 1000.0, 2)];
        let mut meeting = Meeting::new("Sync", 0.0);
        meeting.calendar_event_id = Some("g-1".into());
        let meetings = [meeting];
        let cfg = enabled();
        let none = none();
        let input = TickInput {
            now_ms: 61_000.0,
            settings: &cfg,
            capture_running: true,
            events: &events,
            meetings: &meetings,
            live_meeting_id: Some(meetings[0].id.as_str()),
            pending: None,
            suppressed: &none,
            session_skipped: &none,
            in_flight: false,
        };
        assert_eq!(
            decide(&input),
            AutoRecordDecision::AutoStop {
                meeting_id: meetings[0].id.clone()
            }
        );
        let early = TickInput {
            now_ms: 60_999.0,
            ..input
        };
        assert_eq!(decide(&early), AutoRecordDecision::Idle);
        // A live meeting never starts another one.
        let other_live = TickInput {
            now_ms: 500.0,
            live_meeting_id: Some("someone-else"),
            ..input
        };
        assert_eq!(decide(&other_live), AutoRecordDecision::Idle);
    }
}
