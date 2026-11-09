use super::*;

fn original() -> MeetingNote {
    MeetingNote::new("Original".into(), "  First\n🗒  ".into(), 42).unwrap()
}

#[test]
fn manual_edit_preserves_identity_exact_text_and_advances_a_backward_clock() {
    let original = original();
    let mut draft = MeetingNoteDraft::edit(&original);
    draft.body = "  Edited\n\n🗒  ".into();
    draft.recording_id = Some("capture/one".into());
    let mut notes = vec![original.clone()];
    let saved = draft.apply(&mut notes, 1).unwrap();
    assert_eq!(saved.body, draft.body);
    assert_eq!(saved.id, original.id);
    assert_eq!(saved.created_at, 42);
    assert_eq!(saved.updated_at, 43);
    assert_eq!(saved.recording_id.as_deref(), Some("capture/one"));
    assert_eq!(notes, vec![saved]);
}

#[test]
fn manual_edit_refuses_stale_same_timestamp_deleted_and_duplicate_originals() {
    let original = original();
    let mut draft = MeetingNoteDraft::edit(&original);
    draft.title = "My draft".into();
    let mut changed = original.clone();
    changed.body = "Other user's change".into();
    for (mut notes, expected) in [
        (vec![], MeetingNoteEditError::Removed),
        (vec![changed], MeetingNoteEditError::Conflict),
        (
            vec![original.clone(), original],
            MeetingNoteEditError::Conflict,
        ),
    ] {
        let before = notes.clone();
        assert_eq!(draft.apply(&mut notes, 99), Err(expected));
        assert_eq!(notes, before);
        assert_eq!(draft.title, "My draft");
    }
    assert_ne!(
        MeetingNoteEditError::Removed.message(),
        MeetingNoteEditError::Conflict.message()
    );
}

#[test]
fn blank_oversized_and_invalid_link_drafts_cannot_mutate_notes() {
    let mut draft = MeetingNoteDraft::new(None);
    assert_eq!(draft.prepare(1), Err(MeetingNoteEditError::InvalidContent));
    draft.title = "Title".into();
    draft.body = "🗒".repeat(16385);
    assert_eq!(draft.prepare(1), Err(MeetingNoteEditError::InvalidContent));
    draft.body.clear();
    draft.recording_id = Some(" ".into());
    assert_eq!(draft.prepare(1), Err(MeetingNoteEditError::InvalidLink));
}

#[test]
fn new_draft_identity_is_stable_and_cannot_replace_an_existing_note() {
    let mut draft = MeetingNoteDraft::new(None);
    draft.title = "Manual".into();
    let first = draft.prepare(1).unwrap();
    assert_eq!(first.id, draft.prepare(2).unwrap().id);
    assert_eq!(
        draft.apply(&mut vec![first], 3),
        Err(MeetingNoteEditError::Conflict)
    );
}

#[test]
fn unchanged_edit_keeps_legacy_workflow_receipt_bytes_and_timestamp() {
    let original = original();
    let json = serde_json::to_string(&original).unwrap();
    let saved = MeetingNoteDraft::edit(&original).prepare(99).unwrap();
    assert_eq!(serde_json::to_string(&saved).unwrap(), json);
    assert!(!json.contains("recording_id"));
    assert_eq!(
        serde_json::from_str::<MeetingNote>(&json).unwrap(),
        original
    );
}

#[test]
fn linked_note_round_trips_and_invalid_stored_link_is_rejected() {
    let mut draft = MeetingNoteDraft::edit(&original());
    draft.recording_id = Some("recording-1".into());
    let saved = draft.prepare(44).unwrap();
    let mut json = serde_json::to_value(&saved).unwrap();
    assert_eq!(
        serde_json::from_value::<MeetingNote>(json.clone()).unwrap(),
        saved
    );
    json["recording_id"] = serde_json::json!(" ");
    assert!(serde_json::from_value::<MeetingNote>(json).is_err());
}

#[test]
fn exhausted_edit_timestamp_never_wraps_or_overwrites() {
    let mut original = original();
    original.updated_at = i64::MAX;
    let mut draft = MeetingNoteDraft::edit(&original);
    draft.body.push('!');
    assert_eq!(draft.prepare(0), Err(MeetingNoteEditError::InvalidTime));
}
