use serde::{Deserialize, Serialize};

mod edit;
pub use edit::{MeetingNoteDraft, MeetingNoteEditError};

/// Why a meeting note could not be opened or created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{}", self.message())]
pub enum MeetingNoteError {
    NotFound,
    /// Several notes share the id; none is picked.
    DuplicateIdentity,
    InvalidContent,
}

impl MeetingNoteError {
    /// The user-facing sentence.
    pub const fn message(self) -> &'static str {
        match self {
            Self::NotFound => "This local meeting note is no longer available. Return to meeting notes to choose another.",
            Self::DuplicateIdentity => "Duplicate local meeting note identities prevent opening this note safely. Original records are retained.",
            Self::InvalidContent => "Meeting notes require a title of at most 4096 bytes and a body of at most 65536 bytes.",
        }
    }
}

/// A local text note, separate from captured audio and generated summaries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "StoredMeetingNote")]
pub struct MeetingNote {
    pub id: String,
    pub title: String,
    pub body: String,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recording_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredMeetingNote {
    id: String,
    title: String,
    body: String,
    created_at: i64,
    updated_at: i64,
    #[serde(default)]
    recording_id: Option<String>,
}

impl TryFrom<StoredMeetingNote> for MeetingNote {
    type Error = &'static str;

    fn try_from(stored: StoredMeetingNote) -> Result<Self, Self::Error> {
        if stored.id.trim().is_empty()
            || stored.id.len() > 128
            || !Self::valid_content(&stored.title, &stored.body)
            || stored.updated_at < stored.created_at
            || !Self::valid_recording_id(stored.recording_id.as_deref())
        {
            return Err("Invalid stored meeting note.");
        }
        Ok(Self {
            id: stored.id,
            title: stored.title,
            body: stored.body,
            created_at: stored.created_at,
            updated_at: stored.updated_at,
            recording_id: stored.recording_id,
        })
    }
}

impl MeetingNote {
    pub fn valid_recording_id(id: Option<&str>) -> bool {
        id.is_none_or(|id| !id.trim().is_empty() && id.len() <= 128)
    }

    /// Exact identity lookup; ambiguous legacy records must not pick a winner.
    pub fn find_unique<'a>(notes: &'a [Self], id: &str) -> Result<&'a Self, MeetingNoteError> {
        let mut matches = notes.iter().filter(|note| note.id == id);
        let note = matches.next().ok_or(MeetingNoteError::NotFound)?;
        if matches.next().is_some() {
            return Err(MeetingNoteError::DuplicateIdentity);
        }
        Ok(note)
    }

    pub fn valid_content(title: &str, body: &str) -> bool {
        !title.trim().is_empty() && title.len() <= 4_096 && body.len() <= 65_536
    }

    pub fn new(title: String, body: String, now: i64) -> Result<Self, MeetingNoteError> {
        if !Self::valid_content(&title, &body) {
            return Err(MeetingNoteError::InvalidContent);
        }
        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            title,
            body,
            created_at: now,
            updated_at: now,
            recording_id: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorkflowOperation;

    #[test]
    fn meeting_note_keeps_exact_reviewed_text_without_capture_or_generation_fields() {
        let note = MeetingNote::new(
            "Planning 🗒".into(),
            "  Exact notes\n\nNext action: review.  ".into(),
            42,
        )
        .unwrap();
        assert_eq!(note.body, "  Exact notes\n\nNext action: review.  ");
        assert_eq!(
            serde_json::from_str::<MeetingNote>(&serde_json::to_string(&note).unwrap()).unwrap(),
            note
        );
        let fields = serde_json::to_value(&note).unwrap();
        assert_eq!(fields.as_object().unwrap().len(), 5);
        assert_eq!(note.created_at, 42);
    }

    #[test]
    fn meeting_note_storage_rejects_corrupt_content_and_identity() {
        let original =
            serde_json::to_value(MeetingNote::new("Title".into(), "Body".into(), 42).unwrap())
                .unwrap();
        for (field, value) in [
            ("id", serde_json::json!("")),
            ("title", serde_json::json!(" ")),
            ("body", serde_json::json!("x".repeat(65537))),
            ("updated_at", serde_json::json!(41)),
            ("unknown", serde_json::json!(true)),
        ] {
            let mut invalid = original.clone();
            invalid[field] = value;
            assert!(
                serde_json::from_value::<MeetingNote>(invalid).is_err(),
                "{field}"
            );
        }
    }

    #[test]
    fn meeting_note_content_bounds_and_legacy_empty_body_are_explicit() {
        assert!(MeetingNote::new("Title".into(), String::new(), 1).is_ok());
        assert!(MeetingNote::new(" ".into(), "Body".into(), 1).is_err());
        assert!(MeetingNote::new("T".repeat(4097), String::new(), 1).is_err());
        assert!(MeetingNote::new("Title".into(), "B".repeat(65537), 1).is_err());
        let operation: WorkflowOperation =
            serde_json::from_str(r#"{"type":"create_meeting_note","title":"Legacy title"}"#)
                .unwrap();
        assert!(operation.is_valid());
        assert_eq!(
            operation.review_fields(|_| String::new()),
            vec![("Title", "Legacy title".into()), ("Body", String::new())]
        );
    }

    #[test]
    fn exact_note_lookup_refuses_missing_and_duplicate_identities() {
        let first = MeetingNote::new("First".into(), "Original".into(), 1).unwrap();
        let mut duplicate = first.clone();
        duplicate.body = "Different original".into();
        assert!(MeetingNote::find_unique(&[first.clone(), duplicate], &first.id).is_err());
        assert!(MeetingNote::find_unique(std::slice::from_ref(&first), "missing").is_err());
        assert_eq!(
            MeetingNote::find_unique(std::slice::from_ref(&first), &first.id).unwrap(),
            &first
        );
    }

    #[test]
    fn local_note_search_keeps_full_text_and_has_a_distinct_record_kind_and_route() {
        let note = MeetingNote::new(
            "Meeting title".into(),
            "  Unique body 🗒\n\nSecond line  ".into(),
            9,
        )
        .unwrap();
        let projection = crate::SearchDocument::from(&note);
        assert_eq!(projection.kind, crate::SearchKind::MeetingNote);
        assert_eq!(projection.kind.label(), "Meeting note");
        assert_eq!(projection.body, note.body);
        assert_eq!(projection.updated_at_ms, 9);
        assert_eq!(
            crate::SearchIndex::new([projection]).search("unique 🗒", 1)[0]
                .document
                .id,
            note.id
        );
        let route = crate::AppView::MeetingNoteDetail { note_id: note.id };
        assert_eq!(route.as_str(), "meeting_note_detail");
        assert_eq!(
            serde_json::from_str::<crate::AppView>(&serde_json::to_string(&route).unwrap())
                .unwrap(),
            route
        );
    }
}
