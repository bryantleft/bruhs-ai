use super::*;
use crate::state::{CaptureRecord, InboxConversation};
use bruhs_models::CaptureKind;

fn conversation(body: &str) -> Record {
    let mut item = InboxConversation::local_draft("Recipient", body, 10);
    item.id = "conversation".into();
    // Stable fixtures let exact baseline comparisons detect unexpected changes.
    item.participants[0].id = "recipient".into();
    item.messages[0].id = "message".into();
    Record::Inbox(item)
}

fn pair(queue: &mut Queue) {
    let mut capture = CaptureRecord::new(CaptureKind::MessageDraft, "Draft\nOriginal", 10).unwrap();
    capture.id = "receipt".into();
    capture.target_id = Some("conversation".into());
    queue.edit_record(
        Kind::Inbox,
        "conversation".into(),
        None,
        Some(conversation("Original")),
    );
    queue.edit_capture("receipt".into(), None, Some(Record::Capture(capture)));
    queue.group_records(&[
        (Kind::Inbox, "conversation".into()),
        (Kind::Capture, "receipt".into()),
    ]);
}

#[test]
fn failed_conversation_keeps_latest_draft_in_atomic_capture_group() {
    let mut queue = Queue::default();
    pair(&mut queue);
    queue.failed_record(Kind::Inbox, "conversation", "aborted".into(), false);
    queue.edit_record(
        Kind::Inbox,
        "conversation".into(),
        Some(conversation("Original")),
        Some(conversation("Latest")),
    );
    assert!(queue.next_batch().is_none());
    queue.retry();
    let batch = queue.next_batch().unwrap();
    assert_eq!(batch.len(), 2);
    let inbox = batch.iter().find(|item| item.kind == Kind::Inbox).unwrap();
    assert_eq!(inbox.expected, None);
    assert_eq!(inbox.draft, Some(conversation("Latest")));
    assert_eq!(batch[0].group, batch[1].group);
    for item in batch {
        assert!(queue.saved(item));
    }
    assert!(queue.is_empty());
}

#[test]
fn conversation_conflict_blocks_both_members_but_not_an_independent_import() {
    let mut queue = Queue::default();
    pair(&mut queue);
    queue.failed_record(Kind::Inbox, "conversation", "conflict".into(), true);
    let Record::Inbox(mut other) = conversation("Other") else {
        unreachable!()
    };
    other.id = "other".into();
    queue.edit_record(
        Kind::Inbox,
        other.id.clone(),
        None,
        Some(Record::Inbox(other)),
    );
    queue.retry();
    let batch = queue.next_batch().unwrap();
    assert_eq!(batch.len(), 1);
    assert_eq!(batch[0].id, "other");
    assert!(queue.saved(batch.into_iter().next().unwrap()));
    assert!(queue.next_batch().is_none());
    let recovery: serde_json::Value = serde_json::from_str(&queue.recovery_json()).unwrap();
    assert_eq!(recovery.as_array().unwrap().len(), 2);
    assert_eq!(recovery[1]["kind"], "inbox");
    assert_eq!(recovery[1]["unsaved"]["messages"][0]["body"], "Original");
}

#[test]
fn failed_delete_preserves_durable_original_and_tombstone_for_retry() {
    let mut queue = Queue::default();
    queue.edit_record(
        Kind::Inbox,
        "conversation".into(),
        Some(conversation("Original")),
        None,
    );
    queue.failed_record(Kind::Inbox, "conversation", "aborted".into(), false);
    let recovery: serde_json::Value = serde_json::from_str(&queue.recovery_json()).unwrap();
    assert_eq!(recovery[0]["operation"], "delete");
    assert_eq!(recovery[0]["original"]["messages"][0]["body"], "Original");
    assert!(recovery[0]["unsaved"].is_null());
    queue.retry();
    let pending = queue.next().unwrap();
    assert_eq!(pending.expected, Some(conversation("Original")));
    assert!(pending.draft.is_none());
    assert!(queue.saved(pending));
}

#[test]
fn older_save_acknowledgment_cannot_retire_a_later_delete() {
    let mut queue = Queue::default();
    queue.edit_record(
        Kind::Inbox,
        "conversation".into(),
        None,
        Some(conversation("Original")),
    );
    let issued = queue.next().unwrap();
    queue.edit_record(
        Kind::Inbox,
        "conversation".into(),
        Some(conversation("Original")),
        None,
    );
    assert!(!queue.saved(issued));
    let deletion = queue.next().unwrap();
    assert_eq!(deletion.expected, Some(conversation("Original")));
    assert!(deletion.draft.is_none());
    assert!(queue.saved(deletion));
}
