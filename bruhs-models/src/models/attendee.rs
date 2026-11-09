use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EventSource {
    #[default]
    Local,
    Google,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AttendeeResponse {
    #[default]
    NeedsAction,
    Declined,
    Tentative,
    Accepted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventAttendee {
    pub email: String,
    pub display_name: Option<String>,
    #[serde(default)]
    pub response_status: AttendeeResponse,
    #[serde(default)]
    pub organizer: bool,
    /// True when this attendee is the connected user. Renamed on the wire
    /// to avoid the Rust `self` keyword on the field.
    #[serde(default, rename = "is_self")]
    pub self_: bool,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConferencingLink {
    /// Lowercase snake_case provider tag: `"google_meet"`, `"zoom"`,
    /// `"teams"`, `"other"`.
    pub provider: String,
    pub url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_source_default_is_local() {
        assert_eq!(EventSource::default(), EventSource::Local);
    }

    #[test]
    fn attendee_response_default_is_needs_action() {
        assert_eq!(AttendeeResponse::default(), AttendeeResponse::NeedsAction);
    }

    #[test]
    fn attendee_serde_roundtrip_with_all_fields() {
        let a = EventAttendee {
            email: "you@example.com".into(),
            display_name: Some("You".into()),
            response_status: AttendeeResponse::Accepted,
            organizer: true,
            self_: true,
            optional: false,
        };
        let json = serde_json::to_string(&a).unwrap();
        let back: EventAttendee = serde_json::from_str(&json).unwrap();
        assert_eq!(a, back);
    }

    #[test]
    fn attendee_self_field_serializes_as_is_self() {
        let a = EventAttendee {
            email: "x@y.z".into(),
            display_name: None,
            response_status: AttendeeResponse::NeedsAction,
            organizer: false,
            self_: true,
            optional: false,
        };
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains("\"is_self\":true"));
        assert!(!json.contains("\"self_\""));
    }

    #[test]
    fn attendee_deserializes_from_minimal_json() {
        // Defaults populate the optional fields.
        let json = r#"{"email":"x@y.z","display_name":null}"#;
        let a: EventAttendee = serde_json::from_str(json).unwrap();
        assert_eq!(a.email, "x@y.z");
        assert_eq!(a.response_status, AttendeeResponse::NeedsAction);
        assert!(!a.organizer);
        assert!(!a.self_);
        assert!(!a.optional);
    }

    #[test]
    fn conferencing_link_roundtrip() {
        let c = ConferencingLink {
            provider: "google_meet".into(),
            url: "https://meet.google.com/abc-defg-hij".into(),
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: ConferencingLink = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }
}
