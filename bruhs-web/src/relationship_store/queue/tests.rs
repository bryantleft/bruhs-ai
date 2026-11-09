use super::*;

fn profile(person_id: &str, notes: &str) -> RelationshipProfile {
    let mut profile = RelationshipProfile::new(person_id, 42);
    profile.notes = notes.into();
    profile
}

fn failure(conflict: bool) -> Failure {
    Failure {
        message: "failed".into(),
        conflict,
    }
}

#[test]
fn coalesces_edits_without_advancing_unsaved_baseline() {
    let mut queue = Queue::default();
    let original = profile("a", "original");
    let first = profile("a", "first");
    let last = profile("a", "last");
    queue.edit(Some(original.clone()), first.clone());
    queue.edit(Some(first), last.clone());
    let next = queue.next().unwrap();
    assert_eq!(next.expected, Some(original));
    assert_eq!(next.draft, last);
    assert_eq!(queue.0.len(), 1);
}

#[test]
fn older_acknowledgment_keeps_later_draft_and_advances_only_its_baseline() {
    let mut queue = Queue::default();
    let first = profile("a", "first");
    let last = profile("a", "last");
    queue.edit(None, first.clone());
    let issued = queue.next().unwrap();
    queue.edit(Some(first.clone()), last.clone());
    queue.saved(issued.draft);
    let next = queue.next().unwrap();
    assert_eq!(next.expected, Some(first));
    assert_eq!(next.draft, last);
    queue.saved(last);
    assert!(queue.is_empty());
}

#[test]
fn failure_keeps_latest_draft_and_retry_does_not_duplicate_new_record() {
    let mut queue = Queue::default();
    queue.edit(None, profile("a", "first"));
    queue.edit(Some(profile("a", "first")), profile("a", "last"));
    queue.failed("a", failure(false));
    assert!(queue.next().is_none());
    assert!(queue.editable("a"));
    assert_eq!(queue.0["a"].expected, None);
    let recovery: Vec<RelationshipProfile> = serde_json::from_str(&queue.recovery_json()).unwrap();
    assert_eq!(recovery, vec![profile("a", "last")]);
    queue.retry();
    let next = queue.next().unwrap();
    assert_eq!(next.expected, None);
    assert_eq!(next.draft.notes, "last");
    queue.saved(next.draft);
    assert!(queue.is_empty());
}

#[test]
fn conflict_is_not_retryable_and_does_not_block_another_person() {
    let mut queue = Queue::default();
    queue.edit(None, profile("a", "private local draft"));
    queue.failed("a", failure(true));
    queue.edit(None, profile("b", "unrelated"));
    assert!(!queue.editable("a"));
    assert!(queue.editable("b"));
    queue.retry();
    let next = queue.next().unwrap();
    assert_eq!(next.draft.person_id, "b");
    queue.saved(next.draft);
    assert!(queue.next().is_none());
    assert!(queue.failure().unwrap().conflict);
    assert!(queue.recovery_json().contains("private local draft"));
}

#[test]
fn retry_remains_available_with_both_conflict_and_transient_errors() {
    let mut queue = Queue::default();
    queue.edit(None, profile("a", "conflict"));
    queue.failed("a", failure(true));
    queue.edit(None, profile("b", "transient"));
    queue.failed("b", failure(false));
    assert!(!queue.failure().unwrap().conflict);
    queue.retry();
    let next = queue.next().unwrap();
    assert_eq!(next.draft.person_id, "b");
    queue.saved(next.draft);
    assert!(queue.failure().unwrap().conflict);
}

#[test]
fn further_edits_do_not_silently_clear_a_failed_save() {
    let mut queue = Queue::default();
    queue.edit(None, profile("a", "first"));
    queue.failed("a", failure(false));
    queue.edit(Some(profile("a", "first")), profile("a", "latest"));
    assert!(queue.next().is_none());
    assert!(queue.failure().is_some());
    queue.retry();
    let next = queue.next().unwrap();
    assert_eq!(next.expected, None);
    assert_eq!(next.draft.notes, "latest");
}
