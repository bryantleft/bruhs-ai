use super::*;
use crate::state::{CaptureRecord, WorkItem};
use bruhs_models::CaptureKind;

fn task() -> Record {
    let mut record = WorkItem::new_at("Task draft", 10);
    record.id = "shared-id".into();
    Record::Task(record)
}

fn capture(title: &str) -> Record {
    let mut record = CaptureRecord::new(CaptureKind::Task, title, 10).unwrap();
    record.id = "shared-id".into();
    record.target_id = Some("shared-id".into());
    Record::Capture(record)
}

fn pair(queue: &mut Queue) {
    queue.edit("shared-id".into(), None, Some(task()));
    queue.edit_capture("shared-id".into(), None, Some(capture("Capture draft")));
    queue.group_records(&[
        (Kind::Task, "shared-id".into()),
        (Kind::Capture, "shared-id".into()),
    ]);
}

#[test]
fn equal_ids_in_different_stores_do_not_collide_or_implicitly_group() {
    let mut queue = Queue::default();
    queue.edit("shared-id".into(), None, Some(task()));
    queue.edit_capture("shared-id".into(), None, Some(capture("Capture draft")));
    let task_batch = queue.next_batch().unwrap();
    assert_eq!(task_batch.len(), 1);
    assert_eq!(task_batch[0].kind, Kind::Task);
    assert!(queue.saved(task_batch.into_iter().next().unwrap()));
    let capture_batch = queue.next_batch().unwrap();
    assert_eq!(capture_batch.len(), 1);
    assert_eq!(capture_batch[0].kind, Kind::Capture);
}

#[test]
fn failed_capture_blocks_both_records_and_retains_latest_capture_edit() {
    let mut queue = Queue::default();
    pair(&mut queue);
    queue.failed_capture("shared-id", "aborted".into(), false);
    queue.edit_capture(
        "shared-id".into(),
        Some(capture("Capture draft")),
        Some(capture("Latest capture")),
    );
    assert!(queue.next_batch().is_none());
    let recovery: serde_json::Value = serde_json::from_str(&queue.recovery_json()).unwrap();
    assert_eq!(recovery[0]["kind"], "task");
    assert_eq!(recovery[1]["kind"], "capture");
    assert_eq!(recovery[0]["atomic_group"], recovery[1]["atomic_group"]);
    assert_eq!(recovery[0]["unsaved"]["title"], "Task draft");
    assert_eq!(recovery[1]["unsaved"]["title"], "Latest capture");
    assert!(recovery[1]["original"].is_null());
    queue.retry();
    let batch = queue.next_batch().unwrap();
    assert_eq!(batch.len(), 2);
    for pending in batch {
        assert!(queue.saved(pending));
    }
    assert!(queue.is_empty());
}

#[test]
fn old_capture_ack_does_not_announce_a_newer_draft_saved() {
    let mut queue = Queue::default();
    queue.edit_capture("shared-id".into(), None, Some(capture("First")));
    let issued = queue.next().unwrap();
    queue.edit_capture(
        "shared-id".into(),
        Some(capture("First")),
        Some(capture("Latest")),
    );
    assert!(!queue.saved(issued));
    let latest = queue.next().unwrap();
    assert_eq!(latest.expected, Some(capture("First")));
    assert_eq!(latest.draft, Some(capture("Latest")));
    assert!(queue.saved(latest));
}

#[test]
fn mixed_group_can_join_a_later_recurring_task_without_losing_capture() {
    let mut queue = Queue::default();
    pair(&mut queue);
    let mut next = WorkItem::new_at("Next", 20);
    next.id = "next".into();
    queue.edit(next.id.clone(), None, Some(Record::Task(next)));
    queue.group(&["shared-id".into(), "next".into()]);
    assert_eq!(queue.next_batch().unwrap().len(), 3);
    queue.failed_capture("shared-id", "conflict".into(), true);
    queue.retry();
    assert!(queue.next_batch().is_none());
}
