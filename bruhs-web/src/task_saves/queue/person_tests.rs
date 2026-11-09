use super::*;
use bruhs_models::{CaptureKind, CaptureRecord, PersonProfile};

fn person() -> Record {
    let mut person = PersonProfile::new("Synthetic contact");
    person.id = "person".into();
    person.created_at = 10;
    person.updated_at = 10;
    Record::Person(person)
}

fn capture(title: &str) -> Record {
    let mut capture = CaptureRecord::new(CaptureKind::Contact, title, 10).unwrap();
    capture.id = "receipt".into();
    capture.target_id = Some("person".into());
    Record::Capture(capture)
}

fn pair() -> Queue {
    let mut queue = Queue::default();
    queue.edit_record(Kind::Person, "person".into(), None, Some(person()));
    queue.edit_capture("receipt".into(), None, Some(capture("Original receipt")));
    queue.group_records(&[
        (Kind::Person, "person".into()),
        (Kind::Capture, "receipt".into()),
    ]);
    queue
}

#[test]
fn failed_contact_keeps_edited_receipt_in_one_retry_group() {
    let mut queue = pair();
    queue.failed_record(Kind::Person, "person", "aborted".into(), false);
    queue.edit_capture(
        "receipt".into(),
        Some(capture("Original receipt")),
        Some(capture("Latest receipt")),
    );
    assert!(queue.next_batch().is_none());
    let recovery: serde_json::Value = serde_json::from_str(&queue.recovery_json()).unwrap();
    assert_eq!(recovery.as_array().unwrap().len(), 2);
    assert_eq!(recovery[0]["kind"], "capture");
    assert_eq!(recovery[1]["kind"], "person");
    assert_eq!(recovery[0]["atomic_group"], recovery[1]["atomic_group"]);
    assert_eq!(recovery[0]["unsaved"]["title"], "Latest receipt");
    assert_eq!(recovery[0]["unsaved"]["target_id"], recovery[1]["id"]);
    assert!(recovery[0]["original"].is_null());
    assert!(recovery[1]["original"].is_null());
    queue.retry();
    let batch = queue.next_batch().unwrap();
    assert_eq!(batch.len(), 2);
    for edit in batch {
        assert!(queue.saved(edit));
    }
    assert!(queue.is_empty());
}

#[test]
fn receipt_failure_blocks_contact_and_person_conflict_blocks_retry() {
    let mut queue = pair();
    assert!(queue.saving_kind(Kind::Person));
    queue.failed_capture("receipt", "aborted".into(), false);
    assert!(queue.next_batch().is_none());
    assert!(!queue.saving_kind(Kind::Person));
    queue.retry();
    assert!(queue.saving_kind(Kind::Person));
    assert_eq!(queue.next_batch().unwrap().len(), 2);
    queue.failed_record(Kind::Person, "person", "conflict".into(), true);
    queue.retry();
    assert!(queue.next_batch().is_none());
    assert!(!queue.saving_kind(Kind::Person));
}

#[test]
fn contact_commit_does_not_discard_receipt_edited_during_save() {
    let mut queue = pair();
    let batch = queue.next_batch().unwrap();
    queue.edit_capture(
        "receipt".into(),
        Some(capture("Original receipt")),
        Some(capture("Latest receipt")),
    );
    for edit in batch {
        let kind = edit.kind;
        assert_eq!(queue.saved(edit), kind == Kind::Person);
    }
    let remaining = queue.next_batch().unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].expected, Some(capture("Original receipt")));
    assert_eq!(remaining[0].draft, Some(capture("Latest receipt")));
}

#[test]
fn person_wire_shape_preserves_profile_without_record_tag() {
    let record = person();
    let encoded = serde_json::to_value(&record).unwrap();
    assert!(encoded.get("Person").is_none());
    assert_eq!(
        Record::Person(serde_json::from_value::<PersonProfile>(encoded).unwrap()),
        record
    );
}
