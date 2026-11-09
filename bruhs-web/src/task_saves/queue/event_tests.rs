use super::*;
use crate::durable_metadata::SaveState;
use crate::state::{CalendarEvent, CaptureRecord, EventSource, WorkItem};
use bruhs_models::CaptureKind;

pub(crate) fn event(title: &str) -> CalendarEvent {
    CalendarEvent {
        id: "event".into(),
        title: title.into(),
        start_at_ms: 1000.0,
        end_at_ms: 2000.0,
        all_day: false,
        color: None,
        notes: None,
        created_at: 1,
        updated_at: 2,
        source: EventSource::Local,
        google_event_id: None,
        calendar_id: None,
        attendees: Vec::new(),
        organizer: None,
        location: None,
        conferencing: None,
        recurring_event_id: None,
        html_link: None,
    }
}

fn capture_pair(queue: &mut Queue) {
    let mut capture = CaptureRecord::new(CaptureKind::Event, "Captured event", 1).unwrap();
    capture.id = "capture".into();
    capture.target_id = Some("event".into());
    queue.edit_capture(capture.id.clone(), None, Some(Record::Capture(capture)));
    queue.edit_record(
        Kind::Event,
        "event".into(),
        None,
        Some(Record::Event(event("Original"))),
    );
    queue.group_records(&[
        (Kind::Capture, "capture".into()),
        (Kind::Event, "event".into()),
    ]);
}

#[test]
fn event_capture_failure_retains_pair_and_latest_event_until_retry() {
    let mut queue = Queue::default();
    capture_pair(&mut queue);
    queue.failed_record(Kind::Event, "event", "aborted".into(), false);
    queue.edit_record(
        Kind::Event,
        "event".into(),
        Some(Record::Event(event("Original"))),
        Some(Record::Event(event("Latest"))),
    );
    assert!(queue.next_batch().is_none());
    assert!(matches!(
        queue.status(Kind::Capture, "capture"),
        SaveState::Error {
            conflict: false,
            ..
        }
    ));
    queue.retry();
    assert_eq!(queue.status(Kind::Event, "event"), SaveState::Saving);
    let batch = queue.next_batch().unwrap();
    assert_eq!(batch.len(), 2);
    let pending = batch.iter().find(|item| item.kind == Kind::Event).unwrap();
    assert!(pending.expected.is_none());
    assert_eq!(pending.draft, Some(Record::Event(event("Latest"))));
    for item in batch {
        assert!(queue.saved(item));
    }
    assert_eq!(queue.status(Kind::Event, "event"), SaveState::Saved);
}

#[test]
fn scheduled_task_and_event_share_failure_and_conflict_blocks_retry() {
    let mut queue = Queue::default();
    let mut task = WorkItem::new_at("Schedule me", 1);
    task.id = "task".into();
    let original = task.clone();
    task.planning.time_block_event_ids.push("event".into());
    queue.edit(
        task.id.clone(),
        Some(Record::Task(original)),
        Some(Record::Task(task)),
    );
    queue.edit_record(
        Kind::Event,
        "event".into(),
        None,
        Some(Record::Event(event("Time block"))),
    );
    queue.group_records(&[(Kind::Task, "task".into()), (Kind::Event, "event".into())]);
    queue.failed("task", "changed".into(), true);
    queue.retry();
    assert!(queue.next_batch().is_none());
    assert_eq!(
        queue.status(Kind::Event, "event"),
        SaveState::Error {
            message: "changed".into(),
            conflict: true
        }
    );
    let recovery: serde_json::Value = serde_json::from_str(&queue.recovery_json()).unwrap();
    assert_eq!(recovery[0]["atomic_group"], recovery[1]["atomic_group"]);
    assert_eq!(
        recovery[0]["unsaved"]["planning"]["time_block_event_ids"][0],
        "event"
    );
    assert_eq!(recovery[1]["kind"], "event");
}

#[test]
fn record_status_sees_an_older_failure_after_a_new_member_joins_group() {
    let mut queue = Queue::default();
    capture_pair(&mut queue);
    queue.failed_capture("capture", "aborted".into(), false);
    let mut next = event("Independent");
    next.id = "next".into();
    queue.edit_record(
        Kind::Event,
        next.id.clone(),
        None,
        Some(Record::Event(next)),
    );
    assert_eq!(queue.status(Kind::Event, "next"), SaveState::Saving);
    queue.group_records(&[(Kind::Event, "event".into()), (Kind::Event, "next".into())]);
    assert!(matches!(
        queue.status(Kind::Event, "next"),
        SaveState::Error { .. }
    ));
    assert!(queue.next_batch().is_none());
}

#[test]
fn event_delete_recovery_retains_original_and_retries_tombstone() {
    let mut queue = Queue::default();
    let original = Record::Event(event("Keep until committed"));
    queue.edit_record(Kind::Event, "event".into(), Some(original.clone()), None);
    queue.failed_record(Kind::Event, "event", "aborted".into(), false);
    let recovery: serde_json::Value = serde_json::from_str(&queue.recovery_json()).unwrap();
    assert_eq!(recovery[0]["operation"], "delete");
    assert_eq!(recovery[0]["original"]["title"], "Keep until committed");
    assert!(recovery[0]["unsaved"].is_null());
    queue.retry();
    let pending = queue.next().unwrap();
    assert_eq!(pending.expected, Some(original));
    assert!(pending.draft.is_none());
    assert!(queue.saved(pending));
}

#[test]
fn event_store_wire_shape_does_not_add_a_discriminator() {
    let original = event("Wire shape");
    let json = serde_json::to_value(Record::Event(original.clone())).unwrap();
    assert!(json.get("Event").is_none());
    assert_eq!(
        serde_json::from_value::<CalendarEvent>(json).unwrap(),
        original
    );
}
