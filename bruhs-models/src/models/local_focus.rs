//! Local foreground metadata, deliberately separate from screen-sharing evidence.
use serde::{Deserialize, Serialize};

pub const MAX_LOCAL_FOCUS_OBSERVATIONS: usize = 10_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "availability", rename_all = "snake_case", deny_unknown_fields)]
pub enum LocalFocus {
    Available {
        app_name: String,
        bundle_id: Option<String>,
    },
    Unavailable {},
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalFocusObservation {
    pub id: String,
    pub meeting_id: String,
    pub observed_at_ms: i64,
    pub focus: LocalFocus,
}

impl LocalFocusObservation {
    pub fn is_valid(&self) -> bool {
        fn text(s: &str, max: usize) -> bool {
            !s.trim().is_empty() && s.len() <= max && !s.chars().any(char::is_control)
        }
        text(&self.id, 128)
            && text(&self.meeting_id, 128)
            && self.observed_at_ms > 0
            && match &self.focus {
                LocalFocus::Available {
                    app_name,
                    bundle_id,
                } => text(app_name, 256) && bundle_id.as_deref().is_none_or(|s| text(s, 256)),
                LocalFocus::Unavailable {} => true,
            }
    }

    /// No claim about a selected window, shared app, or remote participant.
    pub fn label(&self) -> String {
        match &self.focus {
            LocalFocus::Available { app_name, .. } => format!("Local focus: {app_name}"),
            LocalFocus::Unavailable {} => "Local focus: unavailable".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_content_fields_and_malformed_metadata() {
        let mut sample = LocalFocusObservation {
            id: "observation".into(),
            meeting_id: "meeting".into(),
            observed_at_ms: 1,
            focus: LocalFocus::Available {
                app_name: "Chrome".into(),
                bundle_id: None,
            },
        };
        assert!(sample.is_valid());
        assert_eq!(sample.label(), "Local focus: Chrome");
        sample.observed_at_ms = 0;
        assert!(!sample.is_valid());
        assert!(serde_json::from_str::<LocalFocus>(
            r#"{"availability":"available","app_name":"Chrome","bundle_id":null,"url":"private"}"#
        )
        .is_err());
        assert!(serde_json::from_str::<LocalFocus>(
            r#"{"availability":"unavailable","app_name":"Chrome"}"#
        )
        .is_err());
    }
}
