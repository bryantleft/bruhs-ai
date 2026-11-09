use super::*;
use crate::state::{Project, WorkItem};

fn project() -> Record {
    Record::Project(Project {
        id: "project".into(),
        name: "Project".into(),
        color: None,
    })
}

fn task(id: &str, title: &str) -> Record {
    let mut task = WorkItem::new_at(title, 1);
    task.id = id.into();
    task.project_id = Some("project".into());
    Record::Task(task)
}

#[test]
fn new_task_joins_failed_parent_and_cannot_commit_before_retry() {
    let mut queue = Queue::default();
    queue.edit_record(Kind::Project, "project".into(), None, Some(project()));
    queue.failed_record(Kind::Project, "project", "aborted".into(), false);
    queue.edit("task".into(), None, Some(task("task", "New")));
    assert!(queue.next_batch().is_none());
    queue.retry();
    let batch = queue.next_batch().unwrap();
    assert_eq!(batch.len(), 2);
    assert!(batch.iter().all(|pending| pending.expected.is_none()));
    for pending in batch {
        assert!(queue.saved(pending));
    }
    assert!(queue.is_empty());
}

#[test]
fn cascade_keeps_original_task_baseline_and_retries_every_tombstone() {
    let mut queue = Queue::default();
    queue.edit(
        "first".into(),
        Some(task("first", "Original")),
        Some(task("first", "Edited")),
    );
    queue.failed("first", "aborted".into(), false);
    queue.edit_record(Kind::Project, "project".into(), Some(project()), None);
    queue.edit("first".into(), Some(task("first", "Edited")), None);
    queue.edit("second".into(), Some(task("second", "Second")), None);
    queue.group_records(&[
        (Kind::Project, "project".into()),
        (Kind::Task, "first".into()),
        (Kind::Task, "second".into()),
    ]);
    assert!(queue.next_batch().is_none());
    queue.retry();
    let batch = queue.next_batch().unwrap();
    assert_eq!(batch.len(), 3);
    assert!(batch.iter().all(|pending| pending.draft.is_none()));
    assert_eq!(
        batch
            .iter()
            .find(|pending| pending.id == "first")
            .unwrap()
            .expected,
        Some(task("first", "Original"))
    );
    queue.failed_record(Kind::Project, "project", "new external child".into(), true);
    queue.retry();
    assert!(queue.next_batch().is_none());
}

#[test]
fn in_flight_parent_ack_keeps_newly_joined_task_group() {
    let mut queue = Queue::default();
    queue.edit_record(Kind::Project, "project".into(), None, Some(project()));
    let issued = queue.next().unwrap();
    queue.edit("task".into(), None, Some(task("task", "New")));
    assert!(!queue.saved(issued));
    let batch = queue.next_batch().unwrap();
    assert_eq!(batch.len(), 2);
    let parent = batch
        .iter()
        .find(|pending| pending.kind == Kind::Project)
        .unwrap();
    assert_eq!(parent.expected, Some(project()));
    assert_eq!(parent.draft, parent.expected);
}

#[test]
fn failed_parent_creation_then_deletion_keeps_absent_original() {
    let mut queue = Queue::default();
    queue.edit_record(Kind::Project, "project".into(), None, Some(project()));
    queue.failed_record(Kind::Project, "project", "aborted".into(), false);
    queue.edit_record(Kind::Project, "project".into(), Some(project()), None);
    queue.retry();
    let pending = queue.next().unwrap();
    assert!(pending.expected.is_none());
    assert!(pending.draft.is_none());
}

#[test]
fn project_record_preserves_wire_shape_and_exact_color_comparison() {
    let value = serde_json::to_value(project()).unwrap();
    assert_eq!(
        value,
        serde_json::json!({"id":"project", "name":"Project", "color":null})
    );
    let mut changed: Project = serde_json::from_value(value).unwrap();
    changed.color = Some("blueberry".into());
    assert_ne!(Record::Project(changed), project());
}
