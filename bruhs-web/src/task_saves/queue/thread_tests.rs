use super::*;
use crate::state::{AgentMessage, Thread};
use crate::storage::MessagesRecord;

fn thread(title: &str) -> Record {
    Record::Thread(Thread {
        id: "a".into(),
        title: title.into(),
        project_name: String::new(),
        updated_at: bruhs_models::EpochMs::new(1).unwrap(),
        project_path: None,
        session_id: None,
        fresh_session: false,
        branch: None,
        model: None,
        pinned: false,
        settled: false,
        pin_rank: None,
        active_rank: None,
        snoozed_until_ms: None,
        goal: None,
        worktree_path: None,
        effort: None,
        run_locked: false,
        role: None,
        instructions_stamp: None,
        import_context: None,
    })
}

fn conversation(text: &str) -> Record {
    let mut message = AgentMessage::user(text.into(), 1.0);
    message.id = "message".into();
    Record::Conversation(MessagesRecord::new("a".into(), vec![message]))
}

fn edit(queue: &mut Queue, kind: Kind, original: Option<Record>, draft: Option<Record>) {
    queue.edit_record(kind, "a".into(), original, draft);
    queue.group_records(&[(Kind::Thread, "a".into()), (Kind::Conversation, "a".into())]);
}

#[test]
fn conversation_failure_blocks_its_metadata_and_retries_only_latest_turn() {
    let mut queue = Queue::default();
    edit(
        &mut queue,
        Kind::Thread,
        Some(thread("Original")),
        Some(thread("Prompt")),
    );
    edit(
        &mut queue,
        Kind::Conversation,
        None,
        Some(conversation("Partial")),
    );
    queue.failed_record(Kind::Conversation, "a", "aborted".into(), false);
    edit(
        &mut queue,
        Kind::Conversation,
        Some(conversation("Partial")),
        Some(conversation("Finished")),
    );
    assert!(queue.next_batch().is_none());
    let json: serde_json::Value = serde_json::from_str(&queue.recovery_json()).unwrap();
    assert_eq!(json[0]["atomic_group"], json[1]["atomic_group"]);
    assert_eq!(json[0]["original"]["title"], "Original");
    assert!(json[1]["original"].is_null());
    assert_eq!(json[1]["unsaved"]["messages"][0]["content"], "Finished");
    assert_eq!(
        queue.conversation("a").unwrap().unwrap().messages()[0].text(),
        "Finished"
    );
    queue.retry();
    let retried = queue.next_batch().unwrap();
    assert_eq!(retried.len(), 2);
    assert!(retried[1].expected.is_none());
    for write in retried {
        assert!(queue.saved(write));
    }
    assert!(queue.is_empty());
}

#[test]
fn inflight_metadata_ack_advances_baseline_without_losing_new_conversation_group() {
    let mut queue = Queue::default();
    edit(&mut queue, Kind::Thread, None, Some(thread("First")));
    let issued = queue.next_batch().unwrap();
    edit(
        &mut queue,
        Kind::Thread,
        Some(thread("First")),
        Some(thread("Prompt")),
    );
    edit(
        &mut queue,
        Kind::Conversation,
        None,
        Some(conversation("Prompt")),
    );
    assert!(!queue.saved(issued[0].clone()));
    let next = queue.next_batch().unwrap();
    assert_eq!(next.len(), 2);
    assert_eq!(next[0].expected, Some(thread("First")));
    assert_eq!(next[0].draft, Some(thread("Prompt")));
    assert!(next[1].expected.is_none());
}

#[test]
fn delete_during_inflight_turn_advances_both_originals_without_resurrecting_drafts() {
    let mut queue = Queue::default();
    edit(
        &mut queue,
        Kind::Thread,
        Some(thread("Original")),
        Some(thread("Prompt")),
    );
    edit(
        &mut queue,
        Kind::Conversation,
        Some(conversation("Old")),
        Some(conversation("New")),
    );
    let issued = queue.next_batch().unwrap();
    edit(&mut queue, Kind::Thread, Some(thread("Prompt")), None);
    edit(
        &mut queue,
        Kind::Conversation,
        Some(conversation("New")),
        None,
    );
    for write in issued {
        assert!(!queue.saved(write));
    }
    assert_eq!(queue.conversation("a"), Some(None));
    let deletion = queue.next_batch().unwrap();
    assert_eq!(deletion.len(), 2);
    assert!(deletion.iter().all(|write| write.draft.is_none()));
    assert_eq!(deletion[0].expected, Some(thread("Prompt")));
    assert_eq!(deletion[1].expected, Some(conversation("New")));
}

#[test]
fn failed_turn_then_delete_keeps_committed_originals_for_retry_and_recovery() {
    let mut queue = Queue::default();
    edit(
        &mut queue,
        Kind::Thread,
        Some(thread("Original")),
        Some(thread("Prompt")),
    );
    edit(
        &mut queue,
        Kind::Conversation,
        Some(conversation("Old")),
        Some(conversation("New")),
    );
    queue.failed_record(Kind::Thread, "a", "abort".into(), false);
    edit(&mut queue, Kind::Thread, Some(thread("Prompt")), None);
    edit(
        &mut queue,
        Kind::Conversation,
        Some(conversation("New")),
        None,
    );
    assert!(queue.next_batch().is_none());
    let json: serde_json::Value = serde_json::from_str(&queue.recovery_json()).unwrap();
    assert_eq!(json[0]["original"]["title"], "Original");
    assert_eq!(json[1]["original"]["messages"][0]["content"], "Old");
    assert!(json
        .as_array()
        .unwrap()
        .iter()
        .all(|record| record["operation"] == "delete"));
    queue.retry();
    assert!(queue
        .next_batch()
        .unwrap()
        .iter()
        .all(|write| write.draft.is_none()));
}

#[test]
fn conflicting_conversation_cannot_retry_or_block_an_independent_thread() {
    let mut queue = Queue::default();
    edit(
        &mut queue,
        Kind::Thread,
        Some(thread("Original")),
        Some(thread("Prompt")),
    );
    edit(
        &mut queue,
        Kind::Conversation,
        Some(conversation("Old")),
        Some(conversation("Local")),
    );
    queue.failed_record(Kind::Conversation, "a", "conflict".into(), true);
    let Record::Thread(mut independent) = thread("Independent") else {
        unreachable!()
    };
    independent.id = "b".into();
    queue.edit_record(
        Kind::Thread,
        "b".into(),
        None,
        Some(Record::Thread(independent)),
    );
    queue.retry();
    let next = queue.next_batch().unwrap();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].id, "b");
    queue.saved(next[0].clone());
    assert!(queue.next_batch().is_none());
    assert!(queue.failure().unwrap().1);
}
