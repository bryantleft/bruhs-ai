use super::*;
use crate::state::WorkItem;

fn task(title: &str) -> Record {
    Record::Task(WorkItem {
        id: "task-a".into(),
        title: title.into(),
        project_id: None,
        description: None,
        status: crate::state::WorkItemStatus::Todo,
        priority: crate::state::Priority::None,
        due_date: None,
        planning: Default::default(),
        created_at: 1,
        updated_at: 2,
    })
}

fn edit(queue: &mut Queue, original: &str, title: &str) {
    queue.edit("task-a".into(), Some(task(original)), Some(task(title)));
}

#[test]
fn coalesces_edits_while_preserving_the_first_durable_baseline() {
    let mut queue = Queue::default();
    edit(&mut queue, "original", "first");
    edit(&mut queue, "first", "latest");
    let pending = queue.next().unwrap();
    assert_eq!(pending.expected, Some(task("original")));
    assert_eq!(pending.draft, Some(task("latest")));
    assert_eq!(queue.0.len(), 1);
}

#[test]
fn older_acknowledgment_never_marks_a_newer_edit_saved() {
    let mut queue = Queue::default();
    edit(&mut queue, "original", "first");
    let issued = queue.next().unwrap();
    edit(&mut queue, "first", "latest");
    queue.saved(issued);
    let next = queue.next().unwrap();
    assert_eq!(next.expected, Some(task("first")));
    assert_eq!(next.draft, Some(task("latest")));
    queue.saved(next);
    assert!(queue.is_empty());
}

#[test]
fn further_edits_keep_failure_visible_and_retry_only_the_latest_draft() {
    let mut queue = Queue::default();
    edit(&mut queue, "original", "failed");
    queue.failed("task-a", "not saved".into(), false);
    edit(&mut queue, "failed", "latest Unicode 日本語 🦀");
    assert!(queue.next().is_none());
    assert!(queue.failure().is_some());
    let recovery: serde_json::Value = serde_json::from_str(&queue.recovery_json()).unwrap();
    assert_eq!(recovery[0]["unsaved"]["title"], "latest Unicode 日本語 🦀");
    queue.retry();
    let pending = queue.next().unwrap();
    assert_eq!(pending.expected, Some(task("original")));
    assert_eq!(pending.draft, Some(task("latest Unicode 日本語 🦀")));
}

#[test]
fn delete_after_an_inflight_save_cannot_resurrect_the_record() {
    let mut queue = Queue::default();
    edit(&mut queue, "original", "first");
    let issued = queue.next().unwrap();
    queue.edit("task-a".into(), Some(task("first")), None);
    queue.saved(issued);
    let deletion = queue.next().unwrap();
    assert_eq!(deletion.expected, Some(task("first")));
    assert!(deletion.draft.is_none());
    queue.saved(deletion);
    assert!(queue.is_empty());
}

#[test]
fn failed_creation_then_delete_preserves_absent_baseline() {
    let mut queue = Queue::default();
    queue.edit("task-a".into(), None, Some(task("new")));
    queue.failed("task-a", "not saved".into(), false);
    queue.edit("task-a".into(), Some(task("new")), None);
    queue.retry();
    let deletion = queue.next().unwrap();
    assert!(deletion.expected.is_none());
    assert!(deletion.draft.is_none());
}

#[test]
fn conflict_cannot_retry_but_does_not_block_an_independent_task() {
    let mut queue = Queue::default();
    edit(&mut queue, "original", "local");
    queue.failed("task-a", "conflict".into(), true);
    let Record::Task(mut second) = task("independent") else {
        unreachable!()
    };
    second.id = "task-b".into();
    queue.edit(second.id.clone(), None, Some(Record::Task(second)));
    queue.retry();
    let pending = queue.next().unwrap();
    assert_eq!(pending.id, "task-b");
    queue.saved(pending);
    assert!(queue.next().is_none());
    assert!(queue.failure().unwrap().1);
    assert!(queue.recovery_json().contains("local"));
}

#[test]
fn failed_deletion_recovery_includes_the_original_record() {
    let mut queue = Queue::default();
    queue.edit("task-a".into(), Some(task("recover me")), None);
    queue.failed("task-a", "delete failed".into(), false);
    let recovery: serde_json::Value = serde_json::from_str(&queue.recovery_json()).unwrap();
    assert_eq!(recovery[0]["operation"], "delete");
    assert_eq!(recovery[0]["original"]["title"], "recover me");
    assert!(recovery[0]["unsaved"].is_null());
}
