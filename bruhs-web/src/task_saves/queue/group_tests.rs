use super::*;
use crate::state::WorkItem;

fn task(id: &str, title: &str) -> Record {
    let mut item = WorkItem::new_at(title, 1);
    item.id = id.into();
    Record::Task(item)
}

fn create(queue: &mut Queue, id: &str) {
    queue.edit(id.into(), None, Some(task(id, "new")));
}

fn group(queue: &mut Queue, ids: &[&str]) {
    queue.group(&ids.iter().map(|id| (*id).into()).collect::<Vec<_>>());
}

#[test]
fn groups_keep_earlier_unsaved_baselines_and_latest_drafts() {
    let mut queue = Queue::default();
    queue.edit(
        "a".into(),
        Some(task("a", "original")),
        Some(task("a", "edit")),
    );
    queue.edit("a".into(), Some(task("a", "edit")), Some(task("a", "done")));
    create(&mut queue, "b");
    group(&mut queue, &["a", "b"]);
    let batch = queue.next_batch().unwrap();
    assert_eq!(batch.len(), 2);
    assert_eq!(batch[0].expected, Some(task("a", "original")));
    assert_eq!(batch[0].draft, Some(task("a", "done")));
    assert!(batch[1].expected.is_none());
}

#[test]
fn overlapping_groups_merge_all_members() {
    let mut queue = Queue::default();
    for id in ["a", "b", "c"] {
        create(&mut queue, id);
    }
    group(&mut queue, &["a", "b"]);
    group(&mut queue, &["b", "c"]);
    let batch = queue.next_batch().unwrap();
    assert_eq!(batch.len(), 3);
    for saved in batch {
        queue.saved(saved);
    }
    assert!(queue.is_empty());
}

#[test]
fn old_singleton_ack_cannot_retire_a_newly_grouped_unchanged_member() {
    let mut queue = Queue::default();
    create(&mut queue, "a");
    let issued = queue.next().unwrap();
    create(&mut queue, "b");
    group(&mut queue, &["a", "b"]);
    queue.saved(issued);
    let batch = queue.next_batch().unwrap();
    assert_eq!(batch.len(), 2);
    assert_eq!(batch[0].expected, Some(task("a", "new")));
    assert_eq!(batch[0].draft, batch[0].expected);
}

#[test]
fn failed_group_keeps_both_recovery_drafts_but_independent_work_can_save() {
    let mut queue = Queue::default();
    for id in ["a", "b", "c"] {
        create(&mut queue, id);
    }
    group(&mut queue, &["a", "b"]);
    queue.failed("b", "aborted".into(), false);
    let independent = queue.next_batch().unwrap();
    assert_eq!(independent.len(), 1);
    assert_eq!(independent[0].id, "c");
    queue.saved(independent.into_iter().next().unwrap());
    assert!(queue.next_batch().is_none());
    let recovery: serde_json::Value = serde_json::from_str(&queue.recovery_json()).unwrap();
    assert_eq!(recovery.as_array().unwrap().len(), 2);
    assert_eq!(recovery[0]["atomic_group"], recovery[1]["atomic_group"]);
    queue.retry();
    assert_eq!(queue.next_batch().unwrap().len(), 2);
}

#[test]
fn conflicted_group_blocks_newly_joined_members_even_after_retry() {
    let mut queue = Queue::default();
    create(&mut queue, "a");
    queue.failed("a", "conflict".into(), true);
    create(&mut queue, "b");
    group(&mut queue, &["a", "b"]);
    queue.retry();
    assert!(queue.next_batch().is_none());
    assert!(queue.failure().unwrap().1);
}

#[test]
fn older_group_ack_preserves_newer_edits_and_advances_their_baselines() {
    let mut queue = Queue::default();
    create(&mut queue, "a");
    create(&mut queue, "b");
    group(&mut queue, &["a", "b"]);
    let issued = queue.next_batch().unwrap();
    queue.edit(
        "b".into(),
        Some(task("b", "new")),
        Some(task("b", "latest")),
    );
    for saved in issued {
        queue.saved(saved);
    }
    let next = queue.next_batch().unwrap();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].expected, Some(task("b", "new")));
    assert_eq!(next[0].draft, Some(task("b", "latest")));
}

#[test]
fn failure_after_an_inflight_group_merge_blocks_the_entire_new_group() {
    let mut queue = Queue::default();
    for id in ["a", "b", "c"] {
        create(&mut queue, id);
    }
    group(&mut queue, &["a", "b"]);
    let issued = queue.next_batch().unwrap();
    group(&mut queue, &["b", "c"]);
    queue.failed(&issued[0].id, "aborted".into(), false);
    assert!(queue.next_batch().is_none());
    queue.retry();
    assert_eq!(queue.next_batch().unwrap().len(), 3);
}
