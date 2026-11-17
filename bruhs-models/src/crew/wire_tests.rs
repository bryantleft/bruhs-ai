use super::*;
use crate::reactive::{OutboxItem, OutboxSignal, ReactiveCommand, ReactiveRun, ReactiveSnapshot};
use crate::EpochMs;

#[test]
fn new_commands_have_stable_wire_names() {
    assert_eq!(
        serde_json::from_str::<ReactiveCommand>(r#"{"action":"stop_all"}"#).unwrap(),
        ReactiveCommand::StopAll
    );
    let set: ReactiveCommand =
        serde_json::from_str(r#"{"action":"set_bruh","agent_id":"a","look":{},"access":{}}"#)
            .unwrap();
    assert_eq!(
        set,
        ReactiveCommand::SetBruh {
            agent_id: "a".into(),
            look: Default::default(),
            access: Default::default(),
            effort: None
        }
    );
    assert_eq!(
        serde_json::from_str::<ReactiveCommand>(
            r#"{"action":"decide_approval","id":"i","approve":true}"#
        )
        .unwrap(),
        ReactiveCommand::DecideApproval {
            id: "i".into(),
            approve: true
        }
    );
    for (json, want) in [
        (r#"{"action":"resume_all"}"#, ReactiveCommand::ResumeAll),
        (
            r#"{"action":"stop_bruh","agent_id":"a"}"#,
            ReactiveCommand::StopBruh {
                agent_id: "a".into(),
            },
        ),
        (
            r#"{"action":"resume_bruh","agent_id":"a"}"#,
            ReactiveCommand::ResumeBruh {
                agent_id: "a".into(),
            },
        ),
    ] {
        assert_eq!(serde_json::from_str::<ReactiveCommand>(json).unwrap(), want);
    }
}

#[test]
fn an_old_servers_snapshot_and_signal_read_as_protocol_zero() {
    let snapshot: ReactiveSnapshot = serde_json::from_str(
        r#"{"agents":[],"sources":[],"subscriptions":[],"events":[],"runs":[],"proposals":[]}"#,
    )
    .unwrap();
    assert_eq!((snapshot.protocol, snapshot.bruhs.len()), (0, 0));
    let signal: OutboxSignal = serde_json::from_str(r#"{"unread":1,"open_questions":0}"#).unwrap();
    assert_eq!((signal.protocol, signal.bruhs.len()), (0, 0));
}

const ITEM: &str = r#"{"id":"i","agent_id":"a","run_id":"r","kind":"ask","title":"t","message":"m","created_at":1"#;

#[test]
fn approvals_ride_outbox_items_and_old_items_still_read() {
    let old: OutboxItem = serde_json::from_str(&format!("{ITEM}}}")).unwrap();
    assert!(old.approval.is_none());
    assert!(!old.is_open_approval());

    let mut item: OutboxItem = serde_json::from_str(&format!(
        r#"{ITEM},"approval":{{"tool":"Bash","summary":"Run ls","fingerprint":"f"}}}}"#
    ))
    .unwrap();
    let approval = item.approval.clone().unwrap();
    assert_eq!(approval.state, ApprovalState::Waiting);
    assert_eq!(approval.tool, "Bash");
    assert!(item.is_open_approval());
    let back: OutboxItem = serde_json::from_str(&serde_json::to_string(&item).unwrap()).unwrap();
    assert_eq!(back, item);
    item.resolved_at = Some(EpochMs::saturating_from_i64(2));
    assert!(!item.is_open_approval());
}

#[test]
fn a_run_without_owner_message_still_reads() {
    let run = r#"{"id":"r","agent_id":"a","source_id":null,"subscription_id":null,"event_id":null,"status":"queued","prompt":"p","thread_id":"t","session_id":null,"created_at":1,"updated_at":1,"not_before":1,"result":null,"error":null}"#;
    let parsed: ReactiveRun = serde_json::from_str(run).unwrap();
    assert_eq!(parsed.owner_message, None);
    assert!(!serde_json::to_string(&parsed)
        .unwrap()
        .contains("owner_message"));
    let with = run.replace(r#""error":null"#, r#""error":null,"owner_message":"hi""#);
    let parsed: ReactiveRun = serde_json::from_str(&with).unwrap();
    assert_eq!(parsed.owner_message.as_deref(), Some("hi"));
}
