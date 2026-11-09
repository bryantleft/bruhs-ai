//! Pure functions: Google `events.list` item → our `CalendarEvent`, or
//! the deletion signal for cancelled items.
//!
//! All times are converted to Unix epoch milliseconds for the wire
//! shape's `f64` fields. Recurring events are exploded by `singleEvents=true`
//! upstream, so each occurrence arrives as its own item; we preserve the
//! parent id in `recurring_event_id` so the UI can label it "part of a series".

use bruhs_models::EpochMs;
use bruhs_models::{AttendeeResponse, CalendarEvent, ConferencingLink, EventAttendee, EventSource};
use chrono::{DateTime, NaiveDate, Utc};
use uuid::Uuid;

use super::client::{RawAttendee, RawConferenceData, RawEvent, RawEventTime};

#[derive(Debug, PartialEq)]
pub enum ClassifiedEvent {
    Upsert(Box<CalendarEvent>),
    Delete { google_event_id: String },
}

/// Why a Google event could not be mapped; it is skipped.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ClassifyError {
    #[error("event {0} has no start")]
    NoStart(String),
    #[error("event {0} has no end")]
    NoEnd(String),
    #[error("event {0} has unparseable start")]
    UnparseableStart(String),
    #[error("event {0} has unparseable end")]
    UnparseableEnd(String),
}

pub fn classify(
    raw: &RawEvent,
    calendar_local_id: Option<&str>,
) -> Result<ClassifiedEvent, ClassifyError> {
    if raw.status.as_deref() == Some("cancelled") {
        return Ok(ClassifiedEvent::Delete {
            google_event_id: raw.id.clone(),
        });
    }
    let start = raw
        .start
        .as_ref()
        .ok_or_else(|| ClassifyError::NoStart(raw.id.clone()))?;
    let end = raw
        .end
        .as_ref()
        .ok_or_else(|| ClassifyError::NoEnd(raw.id.clone()))?;

    let (start_ms, all_day) =
        event_time_to_ms(start).ok_or_else(|| ClassifyError::UnparseableStart(raw.id.clone()))?;
    let (end_ms, _) =
        event_time_to_ms(end).ok_or_else(|| ClassifyError::UnparseableEnd(raw.id.clone()))?;

    let now = Utc::now().timestamp_millis();
    let event = CalendarEvent {
        id: Uuid::new_v4().to_string(),
        title: raw.summary.clone().unwrap_or_else(|| "(no title)".into()),
        start_at_ms: start_ms,
        end_at_ms: end_ms,
        all_day,
        color: None,
        notes: raw.description.clone(),
        created_at: now,
        updated_at: now,
        source: EventSource::Google,
        google_event_id: Some(raw.id.clone()),
        calendar_id: calendar_local_id.map(|s| s.to_string()),
        attendees: raw.attendees.iter().map(map_attendee).collect(),
        organizer: raw.organizer.as_ref().map(map_attendee),
        location: raw.location.clone(),
        conferencing: raw.conference_data.as_ref().and_then(map_conference_data),
        recurring_event_id: raw.recurring_event_id.clone(),
        html_link: raw.html_link.clone(),
    };
    Ok(ClassifiedEvent::Upsert(Box::new(event)))
}

fn event_time_to_ms(t: &RawEventTime) -> Option<(f64, bool)> {
    if let Some(dt) = &t.date_time {
        return DateTime::parse_from_rfc3339(dt).ok().map(|d| {
            (
                EpochMs::saturating_from_i64(d.with_timezone(&Utc).timestamp_millis()).as_f64(),
                false,
            )
        });
    }
    if let Some(d) = &t.date {
        return NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .ok()
            .and_then(|nd| nd.and_hms_opt(0, 0, 0))
            .map(|ndt| {
                (
                    EpochMs::saturating_from_i64(ndt.and_utc().timestamp_millis()).as_f64(),
                    true,
                )
            });
    }
    None
}

fn map_attendee(a: &RawAttendee) -> EventAttendee {
    EventAttendee {
        email: a.email.clone().unwrap_or_default(),
        display_name: a.display_name.clone(),
        response_status: match a.response_status.as_deref() {
            Some("accepted") => AttendeeResponse::Accepted,
            Some("declined") => AttendeeResponse::Declined,
            Some("tentative") => AttendeeResponse::Tentative,
            _ => AttendeeResponse::NeedsAction,
        },
        organizer: a.organizer,
        self_: a.self_,
        optional: a.optional,
    }
}

fn map_conference_data(c: &RawConferenceData) -> Option<ConferencingLink> {
    let url = c
        .entry_points
        .iter()
        .find(|ep| ep.entry_point_type.as_deref() == Some("video"))
        .and_then(|ep| ep.uri.clone())?;
    let provider = match c
        .conference_solution
        .as_ref()
        .and_then(|s| s.name.as_deref())
        .unwrap_or("")
    {
        n if n.contains("Meet") => "google_meet",
        n if n.contains("Zoom") => "zoom",
        n if n.contains("Teams") => "teams",
        _ => "other",
    };
    Some(ConferencingLink {
        provider: provider.into(),
        url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::google_calendar::client::{
        RawAttendee, RawConferenceData, RawConferenceSolution, RawEntryPoint, RawEvent,
        RawEventTime,
    };

    fn fixture(id: &str) -> RawEvent {
        RawEvent {
            id: id.into(),
            status: Some("confirmed".into()),
            summary: Some("Sync".into()),
            description: None,
            location: None,
            html_link: Some("https://calendar.google.com/event?eid=x".into()),
            recurring_event_id: None,
            start: Some(RawEventTime {
                date_time: Some("2026-05-15T09:00:00-07:00".into()),
                date: None,
            }),
            end: Some(RawEventTime {
                date_time: Some("2026-05-15T10:00:00-07:00".into()),
                date: None,
            }),
            attendees: vec![],
            organizer: None,
            conference_data: None,
        }
    }

    #[test]
    fn cancelled_event_classifies_as_delete() {
        let raw = RawEvent {
            id: "evt-1".into(),
            status: Some("cancelled".into()),
            ..fixture("evt-1")
        };
        let got = classify(&raw, None).unwrap();
        assert_eq!(
            got,
            ClassifiedEvent::Delete {
                google_event_id: "evt-1".into()
            }
        );
    }

    #[test]
    fn confirmed_event_classifies_as_upsert_with_source_google() {
        let raw = fixture("evt-1");
        let got = classify(&raw, Some("cal-local")).unwrap();
        match got {
            ClassifiedEvent::Upsert(ev) => {
                assert_eq!(ev.google_event_id.as_deref(), Some("evt-1"));
                assert_eq!(ev.source, EventSource::Google);
                assert_eq!(ev.calendar_id.as_deref(), Some("cal-local"));
                assert_eq!(ev.title, "Sync");
                assert!(!ev.all_day);
                assert_eq!(
                    ev.html_link.as_deref(),
                    Some("https://calendar.google.com/event?eid=x")
                );
            }
            other => panic!("expected Upsert, got {other:?}"),
        }
    }

    #[test]
    fn all_day_event_via_date_field() {
        let raw = RawEvent {
            start: Some(RawEventTime {
                date_time: None,
                date: Some("2026-05-15".into()),
            }),
            end: Some(RawEventTime {
                date_time: None,
                date: Some("2026-05-16".into()),
            }),
            ..fixture("evt-day")
        };
        let got = classify(&raw, None).unwrap();
        match got {
            ClassifiedEvent::Upsert(ev) => {
                assert!(ev.all_day);
            }
            _ => panic!("expected upsert"),
        }
    }

    #[test]
    fn malformed_event_returns_error() {
        let raw = RawEvent {
            start: None,
            ..fixture("evt-bad")
        };
        let err = classify(&raw, None).unwrap_err();
        assert_eq!(err, ClassifyError::NoStart("evt-bad".into()));
        assert_eq!(err.to_string(), "event evt-bad has no start");
    }

    #[test]
    fn attendees_response_status_maps() {
        let raw = RawEvent {
            attendees: vec![
                RawAttendee {
                    email: Some("a@b.c".into()),
                    display_name: None,
                    response_status: Some("accepted".into()),
                    organizer: false,
                    self_: true,
                    optional: false,
                },
                RawAttendee {
                    email: Some("d@e.f".into()),
                    display_name: None,
                    response_status: Some("declined".into()),
                    organizer: false,
                    self_: false,
                    optional: true,
                },
            ],
            ..fixture("evt-1")
        };
        let got = classify(&raw, None).unwrap();
        let ev = match got {
            ClassifiedEvent::Upsert(ev) => ev,
            _ => panic!(),
        };
        assert_eq!(ev.attendees.len(), 2);
        assert_eq!(ev.attendees[0].response_status, AttendeeResponse::Accepted);
        assert!(ev.attendees[0].self_);
        assert_eq!(ev.attendees[1].response_status, AttendeeResponse::Declined);
        assert!(ev.attendees[1].optional);
    }

    #[test]
    fn conference_data_meet_extracted() {
        let raw = RawEvent {
            conference_data: Some(RawConferenceData {
                conference_solution: Some(RawConferenceSolution {
                    name: Some("Google Meet".into()),
                }),
                entry_points: vec![RawEntryPoint {
                    entry_point_type: Some("video".into()),
                    uri: Some("https://meet.google.com/abc-def-ghi".into()),
                }],
            }),
            ..fixture("evt-1")
        };
        let got = classify(&raw, None).unwrap();
        let ev = match got {
            ClassifiedEvent::Upsert(ev) => ev,
            _ => panic!(),
        };
        let cf = ev.conferencing.unwrap();
        assert_eq!(cf.provider, "google_meet");
        assert!(cf.url.contains("meet.google.com"));
    }
}
