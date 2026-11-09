//! Thin reqwest wrapper around the Google Calendar v3 `events.list`
//! endpoint. Returns the raw item JSON; the mapping module converts each
//! into our `CalendarEvent` or an `EventDeleted` signal.
//!
//! The Google Calendar API base URL is hardcoded here — tests inject a
//! custom base via the `with_base` constructor, mirroring the OAuth
//! module's `for_test()` pattern.

use serde::Deserialize;

use super::http::client;
use super::oauth::OAuthError;

const DEFAULT_BASE: &str = "https://www.googleapis.com/calendar/v3";

/// One item from `events.list`. We deserialize a permissive subset; the
/// mapping module owns the conversion to `CalendarEvent`.
#[derive(Debug, Clone, Deserialize)]
pub struct RawEvent {
    pub id: String,
    #[serde(default)]
    pub status: Option<String>, // "confirmed" | "tentative" | "cancelled"
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default, rename = "htmlLink")]
    pub html_link: Option<String>,
    #[serde(default, rename = "recurringEventId")]
    pub recurring_event_id: Option<String>,
    pub start: Option<RawEventTime>,
    pub end: Option<RawEventTime>,
    #[serde(default)]
    pub attendees: Vec<RawAttendee>,
    #[serde(default)]
    pub organizer: Option<RawAttendee>,
    #[serde(default, rename = "conferenceData")]
    pub conference_data: Option<RawConferenceData>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawEventTime {
    #[serde(default, rename = "dateTime")]
    pub date_time: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawAttendee {
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default, rename = "displayName")]
    pub display_name: Option<String>,
    #[serde(default, rename = "responseStatus")]
    pub response_status: Option<String>,
    #[serde(default)]
    pub organizer: bool,
    #[serde(default, rename = "self")]
    pub self_: bool,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawConferenceData {
    #[serde(default, rename = "conferenceSolution")]
    pub conference_solution: Option<RawConferenceSolution>,
    #[serde(default, rename = "entryPoints")]
    pub entry_points: Vec<RawEntryPoint>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawConferenceSolution {
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawEntryPoint {
    #[serde(default, rename = "entryPointType")]
    pub entry_point_type: Option<String>,
    #[serde(default)]
    pub uri: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EventsListResponse {
    #[serde(default)]
    pub items: Vec<RawEvent>,
    #[serde(default, rename = "nextSyncToken")]
    pub next_sync_token: Option<String>,
    #[serde(default, rename = "nextPageToken")]
    pub next_page_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawCalendarListItem {
    pub id: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default, rename = "summaryOverride")]
    pub summary_override: Option<String>,
    #[serde(default)]
    pub primary: bool,
    #[serde(default, rename = "backgroundColor")]
    pub background_color: Option<String>,
    #[serde(default, rename = "accessRole")]
    pub access_role: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CalendarListResponse {
    #[serde(default)]
    pub items: Vec<RawCalendarListItem>,
}

#[derive(Debug, Default, Clone)]
pub struct ListEventsRequest {
    pub calendar_id: String,
    pub time_min_rfc3339: Option<String>,
    pub time_max_rfc3339: Option<String>,
    pub sync_token: Option<String>,
    pub page_token: Option<String>,
}

pub struct CalendarClient {
    base_url: String,
}

impl CalendarClient {
    pub fn new() -> Self {
        Self {
            base_url: DEFAULT_BASE.into(),
        }
    }

    pub fn with_base(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
        }
    }

    pub async fn list_events(
        &self,
        access_token: &str,
        req: &ListEventsRequest,
    ) -> Result<EventsListResponse, OAuthError> {
        let url = format!(
            "{}/calendars/{}/events",
            self.base_url,
            urlencode(&req.calendar_id)
        );
        let mut query: Vec<(&str, String)> = vec![
            ("singleEvents", "true".into()),
            ("showDeleted", "true".into()),
        ];
        if let Some(t) = &req.sync_token {
            query.push(("syncToken", t.clone()));
        } else {
            if let Some(t) = &req.time_min_rfc3339 {
                query.push(("timeMin", t.clone()));
            }
            if let Some(t) = &req.time_max_rfc3339 {
                query.push(("timeMax", t.clone()));
            }
        }
        if let Some(p) = &req.page_token {
            query.push(("pageToken", p.clone()));
        }

        let resp = client()?
            .get(&url)
            .bearer_auth(access_token)
            .query(&query)
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(OAuthError::Status {
                status: status.as_u16(),
                body,
            });
        }
        serde_json::from_str(&body).map_err(OAuthError::json("events.list"))
    }

    pub async fn list_calendars(
        &self,
        access_token: &str,
    ) -> Result<CalendarListResponse, OAuthError> {
        let url = format!("{}/users/me/calendarList", self.base_url);
        let resp = client()?.get(&url).bearer_auth(access_token).send().await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(OAuthError::Status {
                status: status.as_u16(),
                body,
            });
        }
        serde_json::from_str(&body).map_err(OAuthError::json("calendarList"))
    }
}

impl Default for CalendarClient {
    fn default() -> Self {
        Self::new()
    }
}

fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn list_events_happy_path_returns_items_and_next_sync_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/calendars/primary/events"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [
                    {
                        "id": "evt-1",
                        "status": "confirmed",
                        "summary": "Sync",
                        "start": { "dateTime": "2026-05-15T09:00:00-07:00" },
                        "end":   { "dateTime": "2026-05-15T10:00:00-07:00" }
                    },
                    {
                        "id": "evt-2",
                        "status": "cancelled"
                    }
                ],
                "nextSyncToken": "tok-abc"
            })))
            .mount(&server)
            .await;

        let client = CalendarClient::with_base(server.uri());
        let req = ListEventsRequest {
            calendar_id: "primary".into(),
            time_min_rfc3339: Some("2026-05-14T00:00:00Z".into()),
            time_max_rfc3339: Some("2026-05-29T00:00:00Z".into()),
            ..Default::default()
        };
        let resp = client.list_events("at", &req).await.unwrap();
        assert_eq!(resp.items.len(), 2);
        assert_eq!(resp.items[0].id, "evt-1");
        assert_eq!(resp.items[1].status.as_deref(), Some("cancelled"));
        assert_eq!(resp.next_sync_token.as_deref(), Some("tok-abc"));
    }

    #[tokio::test]
    async fn list_events_returns_status_error_on_401() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/calendars/primary/events"))
            .respond_with(ResponseTemplate::new(401).set_body_string(r#"{"error":"unauthorized"}"#))
            .mount(&server)
            .await;
        let client = CalendarClient::with_base(server.uri());
        let req = ListEventsRequest {
            calendar_id: "primary".into(),
            ..Default::default()
        };
        let err = client.list_events("at", &req).await.unwrap_err();
        match err {
            OAuthError::Status { status, .. } => assert_eq!(status, 401),
            _ => panic!("expected Status, got {err:?}"),
        }
    }

    #[tokio::test]
    async fn list_events_with_sync_token_omits_time_bounds() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/calendars/primary/events"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [], "nextSyncToken": "tok-2"
            })))
            .mount(&server)
            .await;
        let client = CalendarClient::with_base(server.uri());
        let req = ListEventsRequest {
            calendar_id: "primary".into(),
            time_min_rfc3339: Some("ignored".into()),
            sync_token: Some("tok-1".into()),
            ..Default::default()
        };
        let resp = client.list_events("at", &req).await.unwrap();
        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 1);
        let url = received[0].url.to_string();
        assert!(url.contains("syncToken=tok-1"), "actual url: {url}");
        assert!(
            !url.contains("timeMin="),
            "syncToken path must NOT include timeMin"
        );
        assert_eq!(resp.next_sync_token.as_deref(), Some("tok-2"));
    }

    #[tokio::test]
    async fn list_calendars_happy_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/me/calendarList"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [
                    {
                        "id": "primary",
                        "summary": "Bryant",
                        "primary": true,
                        "accessRole": "owner",
                        "backgroundColor": "#7986cb"
                    },
                    {
                        "id": "abc@group.calendar.google.com",
                        "summary": "Team",
                        "accessRole": "reader",
                        "backgroundColor": "#33b679"
                    }
                ]
            })))
            .mount(&server)
            .await;
        let client = CalendarClient::with_base(server.uri());
        let resp = client.list_calendars("at").await.unwrap();
        assert_eq!(resp.items.len(), 2);
        assert_eq!(resp.items[0].id, "primary");
        assert!(resp.items[0].primary);
        assert_eq!(resp.items[1].access_role.as_deref(), Some("reader"));
    }
}
