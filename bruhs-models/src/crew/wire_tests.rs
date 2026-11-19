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

#[test]
fn room_commands_and_triggers_have_stable_wire_names() {
    use crate::reactive::RunTrigger;
    let create: ReactiveCommand = serde_json::from_str(
        r#"{"action":"create_room","room":{"id":"r","name":"Launch","members":["a"]}}"#,
    )
    .unwrap();
    assert_eq!(
        create,
        ReactiveCommand::CreateRoom {
            room: room::Room {
                id: "r".into(),
                name: "Launch".into(),
                members: vec!["a".into()],
                lead: None
            }
        }
    );
    assert_eq!(
        serde_json::from_str::<ReactiveCommand>(
            r#"{"action":"post_room","room_id":"r","text":"@Ada hi"}"#
        )
        .unwrap(),
        ReactiveCommand::PostRoom {
            room_id: "r".into(),
            text: "@Ada hi".into()
        }
    );
    assert_eq!(
        serde_json::from_str::<ReactiveCommand>(r#"{"action":"delete_room","id":"r"}"#).unwrap(),
        ReactiveCommand::DeleteRoom { id: "r".into() }
    );
    let trigger: RunTrigger =
        serde_json::from_str(r#"{"type":"room","room_id":"r","message_id":"m","hop":2}"#).unwrap();
    assert_eq!(trigger.room_id(), Some("r"));
    assert_eq!(trigger.label(), "Room");
    assert!(matches!(trigger, RunTrigger::Room { resumed: false, .. }));
    let chat = ReactiveCommand::Chat {
        agent_id: "a".into(),
        message: "hi".into(),
        run_id: None,
    };
    assert!(!serde_json::to_string(&chat).unwrap().contains("run_id"));
    assert_eq!(
        serde_json::from_str::<ReactiveCommand>(
            r#"{"action":"chat","agent_id":"a","message":"hi"}"#
        )
        .unwrap(),
        chat
    );
}

#[test]
fn snapshots_and_signals_from_an_older_service_have_no_rooms() {
    let signal: OutboxSignal = serde_json::from_str(r#"{"unread":0,"open_questions":0}"#).unwrap();
    assert!(signal.rooms.is_empty());
    let snapshot: ReactiveSnapshot = serde_json::from_str(
        r#"{"agents":[],"sources":[],"subscriptions":[],"events":[],"runs":[],"proposals":[]}"#,
    )
    .unwrap();
    assert!(snapshot.rooms.is_empty());
}

#[test]
fn a_protocol_one_service_keeps_crew_commands_but_not_rooms() {
    assert!(crew_commands_ready(1) && !rooms_ready(1));
    assert!(rooms_ready(CREW_PROTOCOL) && crew_commands_ready(CREW_PROTOCOL));
    assert!(!crew_commands_ready(0));
    assert_eq!(service_notice(1, None), Some(ServiceNotice::ServingOld));
    assert_eq!(stop_commands(1, &[]), vec![ReactiveCommand::StopAll]);
}

#[test]
fn a_run_with_a_trigger_from_a_newer_service_still_decodes() {
    use crate::reactive::RunTrigger;
    let snapshot: ReactiveSnapshot = serde_json::from_value(serde_json::json!({
        "agents": [], "sources": [], "subscriptions": [], "events": [], "proposals": [],
        "runs": [{
            "id": "r1", "agent_id": "a", "source_id": null, "subscription_id": null,
            "event_id": null, "status": "completed", "prompt": "p", "thread_id": "t",
            "session_id": null, "created_at": 1, "updated_at": 1, "not_before": 1,
            "result": "done", "error": null,
            "trigger": {"type": "something_new", "from": "later", "hop": 3},
        }],
    }))
    .unwrap();
    let trigger = snapshot.runs[0].trigger.as_ref().unwrap();
    assert_eq!(trigger.label(), "Run");
    assert_eq!(trigger.room_id(), None);
    let bare: RunTrigger = serde_json::from_str(r#"{"type":"another"}"#).unwrap();
    assert_eq!(&bare, trigger);
    // A known trigger is still read strictly.
    assert!(serde_json::from_str::<RunTrigger>(r#"{"type":"room","room_id":"r"}"#).is_err());
}

#[test]
fn computer_is_its_own_command_and_survives_an_access_save() {
    assert_eq!(
        serde_json::from_str::<ReactiveCommand>(
            r#"{"action":"set_computer","agent_id":"a","on":true}"#
        )
        .unwrap(),
        ReactiveCommand::SetComputer {
            agent_id: "a".into(),
            on: true
        }
    );
    assert!(!serde_json::from_str::<BruhProfile>("{}").unwrap().computer);
    let state: BruhState = serde_json::from_str(
        r#"{"agent_id":"a","name":"A","look":{},"access":{},"status":"asleep"}"#,
    )
    .unwrap();
    assert!(!state.computer, "an older service's bruh reads as off");
    assert!(computer_ready(3) && !computer_ready(2) && CREW_PROTOCOL == 3);
    assert_eq!(COMPUTER_PROTOCOL, CREW_PROTOCOL);

    let agent = crate::reactive::ReactiveAgent {
        id: "a".into(),
        name: "A".into(),
        instructions: String::new(),
        memory: String::new(),
        model: "sonnet".into(),
        project_path: String::new(),
        enabled: true,
    };
    let settings = BruhSettings {
        name: "A".into(),
        instructions: String::new(),
        model: "sonnet".into(),
        effort: None,
        look: BruhLook::default(),
        access: BruhAccess::default(),
        computer: false,
    };
    let on = BruhSettings {
        computer: true,
        ..settings.clone()
    };
    assert_eq!(
        settings_commands(&agent, &state, &on),
        vec![ReactiveCommand::SetComputer {
            agent_id: "a".into(),
            on: true
        }]
    );
    let full = BruhSettings {
        access: BruhAccess {
            level: AccessLevel::Full,
            ..BruhAccess::default()
        },
        ..settings
    };
    let commands = settings_commands(&agent, &state, &full);
    assert!(matches!(
        commands.as_slice(),
        [ReactiveCommand::SetBruh { .. }]
    ));
    let both = BruhSettings {
        computer: true,
        ..full
    };
    let commands = settings_commands(&agent, &state, &both);
    assert!(matches!(
        commands.as_slice(),
        [
            ReactiveCommand::SetBruh { .. },
            ReactiveCommand::SetComputer { on: true, .. }
        ]
    ));
}
