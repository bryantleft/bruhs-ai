//! Provider-neutral records for the global capture surface.
//!
//! Capture preserves the user's original text and projects its first line into
//! a concise title. UI adapters decide how each typed capture becomes a task,
//! event, person, note, or message draft.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

const CAPTURE_TITLE_CHARS: usize = 120;

/// Destination selected in universal capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureKind {
    Task,
    Note,
    Event,
    Contact,
    MessageDraft,
}

impl CaptureKind {
    /// All destinations in the stable keyboard order used by both surfaces.
    pub const ALL: [Self; 5] = [
        Self::Task,
        Self::Note,
        Self::Event,
        Self::Contact,
        Self::MessageDraft,
    ];

    /// Human-readable destination label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Task => "Task",
            Self::Note => "Note",
            Self::Event => "Event",
            Self::Contact => "Contact",
            Self::MessageDraft => "Message",
        }
    }

    /// Input guidance tailored to the selected destination.
    pub const fn placeholder(self) -> &'static str {
        match self {
            Self::Task => "What needs to be done?",
            Self::Note => "Capture a thought, decision, or reference…",
            Self::Event => "What should go on your calendar?",
            Self::Contact => "Who do you want to remember?",
            Self::MessageDraft => "Draft the message you want to send…",
        }
    }
}

/// Durable audit record produced by universal capture.
///
/// Task, event, and contact entries link to the created domain record through
/// `target_id`. Notes and message drafts remain first-class capture records
/// until their richer domain modules add more fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureRecord {
    pub id: String,
    pub kind: CaptureKind,
    pub title: String,
    pub body: String,
    pub target_id: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl CaptureRecord {
    /// Validates and normalizes a capture while preserving all meaningful text.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError::Empty`] when `content` contains only whitespace.
    pub fn new(kind: CaptureKind, content: &str, now_ms: i64) -> Result<Self, CaptureError> {
        let content = content.trim();
        if content.is_empty() {
            return Err(CaptureError::Empty);
        }

        let (first_line, remainder) = content.split_once('\n').unwrap_or((content, ""));
        let first_line = first_line.trim();
        let title = truncate_chars(first_line, CAPTURE_TITLE_CHARS);
        let body = remainder.trim().to_string();

        Ok(Self {
            id: Uuid::new_v4().to_string(),
            kind,
            title,
            body,
            target_id: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        })
    }

    /// Attaches the record created by the destination adapter.
    pub fn with_target(mut self, target_id: impl Into<String>) -> Self {
        self.target_id = Some(target_id.into());
        self
    }

    /// Reconstructs the editable one-field capture text.
    pub fn content(&self) -> String {
        if self.body.is_empty() {
            self.title.clone()
        } else {
            format!("{}\n{}", self.title, self.body)
        }
    }
}

/// Validation failure returned by [`CaptureRecord::new`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureError {
    Empty,
}

impl fmt::Display for CaptureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("Capture text is required."),
        }
    }
}

impl std::error::Error for CaptureError {}

fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    let mut result = value.chars().take(limit).collect::<String>();
    result.push('…');
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_should_reject_whitespace_only_content() {
        assert_eq!(
            CaptureRecord::new(CaptureKind::Task, " \n ", 10),
            Err(CaptureError::Empty)
        );
    }

    #[test]
    fn new_should_split_title_and_body_without_losing_text() {
        let record = CaptureRecord::new(
            CaptureKind::Note,
            "Launch decision\nUse the smaller rollout cohort.",
            10,
        )
        .expect("valid capture");

        assert_eq!(record.title, "Launch decision");
        assert_eq!(record.body, "Use the smaller rollout cohort.");
    }

    #[test]
    fn new_should_truncate_unicode_titles_on_character_boundaries() {
        let content = "🫐".repeat(CAPTURE_TITLE_CHARS + 1);
        let record =
            CaptureRecord::new(CaptureKind::MessageDraft, &content, 10).expect("valid capture");

        assert_eq!(record.title.chars().count(), CAPTURE_TITLE_CHARS + 1);
    }

    #[test]
    fn content_should_round_trip_title_and_body() {
        let record = CaptureRecord::new(CaptureKind::Event, "Planning\nBring notes", 10)
            .expect("valid capture");

        assert_eq!(record.content(), "Planning\nBring notes");
    }

    #[test]
    fn with_target_should_link_the_created_domain_record() {
        let record = CaptureRecord::new(CaptureKind::Contact, "Ada", 10)
            .expect("valid capture")
            .with_target("person-1");

        assert_eq!(record.target_id.as_deref(), Some("person-1"));
    }
}
