use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationKind {
    Tone,
    Topic,
    Decision,
    ActionItemOwner,
    Quote,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersonObservation {
    pub at_meeting_id: String,
    pub kind: ObservationKind,
    pub text: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PersonProfile {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Optional voice fingerprint, populated by diarization.
    #[serde(default)]
    pub voice_embedding: Option<Vec<f32>>,
    #[serde(default)]
    pub observations: Vec<PersonObservation>,
    /// When false, new observations are silently dropped for this profile even
    /// if the global `extract_observations` toggle is on. Existing observations
    /// are preserved. Defaults to true for back-compat with legacy records that
    /// lack this field.
    #[serde(default = "default_tracked")]
    pub tracked: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A server-confirmed redirect to the currently live contact after one or more merges.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PersonMergeResolution {
    pub source_id: String,
    pub target: PersonProfile,
}

fn default_tracked() -> bool {
    true
}

impl PersonProfile {
    pub fn new(display_name: impl Into<String>) -> Self {
        let now = 0i64; // caller will overwrite via DB upsert; defaults to 0 for tests
        Self {
            id: Uuid::new_v4().to_string(),
            display_name: display_name.into(),
            aliases: Vec::new(),
            voice_embedding: None,
            observations: Vec::new(),
            tracked: true,
            created_at: now,
            updated_at: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn person_profile_new_defaults() {
        let p = PersonProfile::new("Alice");
        assert_eq!(p.display_name, "Alice");
        assert!(p.aliases.is_empty());
        assert!(p.voice_embedding.is_none());
        assert!(p.observations.is_empty());
    }

    #[test]
    fn person_profile_serde_with_observations() {
        let mut p = PersonProfile::new("Alice");
        p.observations.push(PersonObservation {
            at_meeting_id: "m-1".into(),
            kind: ObservationKind::ActionItemOwner,
            text: "Owns the launch checklist".into(),
            created_at: 100,
        });
        let json = serde_json::to_string(&p).unwrap();
        let back: PersonProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.observations.len(), 1);
        assert_eq!(back.observations[0].kind, ObservationKind::ActionItemOwner);
    }

    #[test]
    fn person_profile_tracked_defaults_true_on_legacy_json() {
        // Legacy JSON without the `tracked` field — back-compat check.
        let json = r#"{"id":"abc","display_name":"Alice","aliases":[],"voice_embedding":null,"observations":[],"created_at":0,"updated_at":0}"#;
        let p: PersonProfile = serde_json::from_str(json).unwrap();
        assert!(p.tracked, "tracked should default to true when absent");
    }

    #[test]
    fn person_profile_tracked_false_roundtrips() {
        let mut p = PersonProfile::new("Bob");
        p.tracked = false;
        let json = serde_json::to_string(&p).unwrap();
        let back: PersonProfile = serde_json::from_str(&json).unwrap();
        assert!(!back.tracked);
    }

    #[test]
    fn observation_kind_serializes_snake_case() {
        let o = PersonObservation {
            at_meeting_id: "m".into(),
            kind: ObservationKind::ActionItemOwner,
            text: "x".into(),
            created_at: 0,
        };
        let json = serde_json::to_string(&o).unwrap();
        assert!(json.contains("\"kind\":\"action_item_owner\""));
    }
}
