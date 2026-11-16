//! The shared answers both apps give. Transport and app wiring have their
//! own tests in bruhs-core, bruhs-server, bruhs-mcp, bruhs-desktop and
//! bruhs-web.
use super::*;
use crate::{
    AgentCapability, AutomationState, LifeContext, Meeting, MeetingNote, PersonProfile,
    RelationshipProfile, TranscriptSegment, WorkflowStatus,
};
use serde_json::{json, Value};

fn policy(grants: &[(AgentDomain, AgentAccess)]) -> AgentPermissionPolicy {
    AgentPermissionPolicy {
        grants: grants
            .iter()
            .map(|&(domain, access)| AgentCapability { domain, access })
            .collect(),
    }
}

fn person(id: &str, name: &str, tracked: bool) -> PersonProfile {
    serde_json::from_value(json!({
        "id": id, "display_name": name, "aliases": ["Alias of ".to_owned() + name],
        "voice_embedding": [0.25, 0.5],
        "observations": [{"at_meeting_id":"m1","kind":"topic","text":"Prefers mornings","created_at":3}],
        "tracked": tracked, "created_at": 1, "updated_at": 2
    }))
    .unwrap()
}

fn task(title: &str) -> WorkflowOperation {
    WorkflowOperation::CreateTask {
        title: title.into(),
    }
}

fn proposal(title: &str, operations: Vec<WorkflowOperation>) -> WorkflowProposal {
    WorkflowProposal {
        title: title.into(),
        operations,
    }
}

#[test]
fn every_read_checks_its_grant_before_resolving_an_id() {
    let none = policy(&[]);
    let write_only = policy(&[
        (AgentDomain::People, AgentAccess::Write),
        (AgentDomain::Meetings, AgentAccess::Write),
        (AgentDomain::Chat, AgentAccess::Write),
    ]);
    for policy in [&none, &write_only] {
        let missing = "no-such-id";
        assert_eq!(
            read_person(
                policy,
                [].iter(),
                [].iter(),
                missing,
                PersonReadSection::Profile,
                0
            ),
            Err(READ_NOT_GRANTED.into())
        );
        assert_eq!(
            read_meeting(
                policy,
                [].iter(),
                |_| None,
                |_| Ok(()),
                missing,
                MeetingReadSection::Summary,
                0
            ),
            Err(READ_NOT_GRANTED.into())
        );
        assert_eq!(
            read_meeting_note(policy, &[], missing),
            Err(READ_NOT_GRANTED.into())
        );
        assert_eq!(
            read_meeting_notes(policy, &[], "", 0),
            Err(READ_NOT_GRANTED.into())
        );
        assert_eq!(
            authorize_chat_messages(policy, missing, 0),
            Err(READ_NOT_GRANTED.into())
        );
        assert_eq!(
            require_read(policy, AgentDomain::Tasks),
            Err(READ_NOT_GRANTED.into())
        );
    }
}

#[test]
fn people_discovery_never_copies_histories_and_respects_opt_outs() {
    let people = [person("p1", "Ada", true), person("p2", "Grace", false)];
    let page = read_people(people.iter(), "", 0).unwrap();
    assert_eq!(
        page["records"],
        json!([
            {"id":"p1","display_name":"Ada","tracked":true,"created_at":1,"updated_at":2,"alias_count":1,"observation_count":1},
            {"id":"p2","display_name":"Grace","tracked":false,"created_at":1,"updated_at":2,"alias_count":1,"observation_count":null,"observations_withheld":true}
        ])
    );
    // Aliases are searchable identity; observation text is not.
    assert_eq!(
        read_people(people.iter(), "ALIAS OF GRACE", 0).unwrap()["records"][0]["id"],
        "p2"
    );
    assert_eq!(
        read_people(people.iter(), "mornings", 0).unwrap()["records"],
        json!([])
    );
    assert!(read_people(people.iter(), &"x".repeat(513), 0).is_err());
    assert!(read_people(people.iter(), "", 100_001).is_err());

    let read = policy(&[(AgentDomain::People, AgentAccess::Read)]);
    let withheld = read_person(
        &read,
        people.iter(),
        [].iter(),
        "p2",
        PersonReadSection::Observations,
        0,
    )
    .unwrap();
    assert_eq!(withheld["records"], json!([]));
    assert_eq!(withheld["observations_withheld"], true);
    let shared = read_person(
        &read,
        people.iter(),
        [].iter(),
        "p1",
        PersonReadSection::Observations,
        0,
    )
    .unwrap();
    assert_eq!(shared["records"][0]["text"], "Prefers mornings");
    let profile = read_person(
        &read,
        people.iter(),
        [RelationshipProfile::new("p1", 9)].iter(),
        "p1",
        PersonReadSection::Profile,
        0,
    )
    .unwrap();
    let encoded = profile.to_string();
    assert!(
        !encoded.contains("voice_embedding") && !encoded.contains("0.25"),
        "{encoded}"
    );
    assert_eq!(profile["records"][0]["relationship"]["updated_at_ms"], 9);
    assert_eq!(profile["person_id"], "p1");
    assert_eq!(profile["section"], "profile");
    assert!(read_person(
        &read,
        people.iter(),
        [].iter(),
        "p1",
        PersonReadSection::Profile,
        1
    )
    .is_err());
    let duplicates = [person("p1", "Ada", true), person("p1", "Ada again", true)];
    assert_eq!(
        read_person(
            &read,
            duplicates.iter(),
            [].iter(),
            "p1",
            PersonReadSection::Profile,
            0
        ),
        Err("Duplicate person IDs prevent an unambiguous read.".into())
    );
}

#[test]
fn meeting_reads_page_loaded_transcripts_and_never_fabricate_empty_ones() {
    let mut meeting = Meeting::new("Planning", 1_000.0);
    meeting.id = "m1".into();
    meeting.audio_path = Some("/private/audio.wav".into());
    let meetings = [meeting];
    let found = read_meetings(meetings.iter(), "plan", 0).unwrap();
    assert_eq!(found["source"], "workspace_cache");
    assert_eq!(found["records"][0]["id"], "m1");
    assert!(!found.to_string().contains("audio.wav"));

    let read = policy(&[(AgentDomain::Meetings, AgentAccess::Read)]);
    let summary = read_meeting(
        &read,
        meetings.iter(),
        |_| None,
        |_| Ok(()),
        "m1",
        MeetingReadSection::Summary,
        0,
    )
    .unwrap();
    assert_eq!(summary["meeting_id"], "m1");
    assert!(!summary.to_string().contains("audio.wav"));
    assert_eq!(
        read_meeting(&read, meetings.iter(), |_| None, |_| Ok(()), "m1", MeetingReadSection::Transcript, 0),
        Err("Transcript is not loaded in this workspace. Open the meeting in the app first; no empty transcript is claimed.".into())
    );
    let segments: Vec<TranscriptSegment> = (0..60)
        .map(|index| {
            serde_json::from_value(json!({
            "id": format!("s{index}"), "meeting_id": "m1", "start_ms": index, "end_ms": index + 1,
            "speaker_id": null, "text": format!("Line {index}"), "confidence": 0.9, "partial": false
        })).unwrap()
        })
        .collect();
    let page = read_meeting(
        &read,
        meetings.iter(),
        |_| Some(segments.as_slice()),
        |id| crate::validate_transcript_records(id, &segments),
        "m1",
        MeetingReadSection::Transcript,
        50,
    )
    .unwrap();
    assert_eq!(page["records"].as_array().unwrap().len(), 10);
    assert_eq!(page["records"][0]["text"], "Line 50");
    assert!(page["next_offset"].is_null());
    assert!(read_meeting(
        &read,
        meetings.iter(),
        |_| Some(segments.as_slice()),
        |_| Err(crate::TranscriptValidationError::InvalidSegments),
        "m1",
        MeetingReadSection::Transcript,
        0
    )
    .is_err());
}

#[test]
fn meeting_notes_are_identity_first_and_exact_by_id() {
    let read = policy(&[(AgentDomain::Meetings, AgentAccess::Read)]);
    let notes: Vec<MeetingNote> = serde_json::from_value(json!([
        {"id":"n1","title":"Standup","body":"Private body","created_at":1,"updated_at":2},
        {"id":"n2","title":"Retro","body":"Other","created_at":1,"updated_at":3}
    ]))
    .unwrap();
    let list = read_meeting_notes(&read, &notes, "stand", 0).unwrap();
    assert_eq!(
        list["records"],
        json!([{"id":"n1","title":"Standup","updated_at":2}])
    );
    assert_eq!(list["source"], "workspace_local_notes");
    assert_eq!(
        read_meeting_note(&read, &notes, "n2").unwrap()["records"][0]["body"],
        "Other"
    );
    assert!(read_meeting_note(&read, &notes, "missing").is_err());
}

#[test]
fn life_reads_need_the_grant_and_the_opt_in_and_hide_amounts_without_consent() {
    let mut life = LifeContext::default();
    life.finance.transactions = serde_json::from_value(json!([{
        "id":"t1","account_id":null,"merchant":"Bakery","category":"Food","amount_minor":-9007199254740993_i64,
        "currency":"USD","occurred_at_ms":5,"note":"secret note"
    }]))
    .unwrap();
    let none = policy(&[]);
    let read = policy(&[(AgentDomain::Finance, AgentAccess::Read)]);
    assert_eq!(
        read_finance(&none, &life, FinanceReadSection::Transactions, "", 0),
        Err(READ_NOT_GRANTED.into())
    );
    assert!(
        read_finance(&read, &life, FinanceReadSection::Transactions, "", 0)
            .unwrap_err()
            .contains("Life agent context is disabled")
    );
    life.finance.privacy.allow_agent_context = true;
    let redacted = read_finance(&read, &life, FinanceReadSection::Transactions, "", 0).unwrap();
    assert_eq!(redacted["amounts_redacted"], true);
    assert!(redacted["records"][0].get("amount_minor").is_none());
    assert!(!redacted.to_string().contains("secret note"));
    assert_eq!(redacted["durable_receipt"], false);
    life.finance.privacy.reveal_exact_amounts_to_agent = true;
    let exact = read_finance(&read, &life, FinanceReadSection::Transactions, "bakery", 0).unwrap();
    assert_eq!(exact["records"][0]["amount_minor"], "-9007199254740993");
    assert_eq!(
        read_finance(&read, &life, FinanceReadSection::Transactions, "secret", 0).unwrap()
            ["records"],
        json!([])
    );
    assert!(read_health(
        &policy(&[(AgentDomain::Health, AgentAccess::Read)]),
        &life,
        HealthReadSection::Workouts,
        "",
        0
    )
    .unwrap_err()
    .contains("Life agent context is disabled"));
}

#[test]
fn chat_pages_share_one_record_shape() {
    let threads = [("t1", "Launch plan", "Rocket"), ("t2", "Groceries", "")];
    let summary = |thread: &&(&'static str, &'static str, &'static str)| ChatThreadSummary {
        id: thread.0,
        title: thread.1,
        project_name: thread.2,
        updated_at_ms: 7,
        pinned: false,
        settled: false,
        streaming: thread.0 == "t2",
    };
    let page = read_chat_threads(threads.iter(), summary, "rocket", 0).unwrap();
    assert_eq!(
        page["records"],
        json!([{"id":"t1","title":"Launch plan","project_name":"Rocket","updated_at_ms":7,"pinned":false,"settled":false,"streaming":false}])
    );
    let input = json!({"path":"a.rs"});
    let messages = ["first", "second"].map(|content| ChatMessageRecord {
        role: "assistant",
        content: content.into(),
        model: Some("claude"),
        thinking: "".into(),
        usage: Some(ChatUsageRecord {
            input_tokens: 1,
            output_tokens: 2,
            cache_read_input_tokens: 3,
            cache_creation_input_tokens: 4,
            context_tokens: None,
        }),
        duration_ms: Some(10),
        started_at_ms: Some(1),
        completed_at_ms: Some(11),
        tool_calls: vec![ChatToolCallRecord {
            id: "c1",
            tool: "Read",
            input: &input,
            output: Some("ok"),
            is_error: false,
        }],
        stopped: false,
        error: None,
    });
    let page = read_chat_messages(messages.into_iter(), std::convert::identity, "t1", 1).unwrap();
    assert_eq!(page["thread_id"], "t1");
    assert_eq!(
        page["records"],
        json!([{
            "role":"assistant","content":"second","model":"claude","thinking":"",
            "usage":{"input_tokens":1,"output_tokens":2,"cache_read_input_tokens":3,"cache_creation_input_tokens":4},
            "duration_ms":10,"started_at_ms":1,"completed_at_ms":11,
            "tool_calls":[{"id":"c1","tool":"Read","input":{"path":"a.rs"},"output":"ok","is_error":false}],
            "stopped":false,"error":null
        }])
    );
    assert!(authorize_chat_messages(
        &policy(&[(AgentDomain::Chat, AgentAccess::Read)]),
        &"x".repeat(129),
        0
    )
    .is_err());
}

fn writable_state() -> AutomationState {
    let mut state = AutomationState::default();
    state
        .permissions
        .set(AgentDomain::Tasks, AgentAccess::Write, true);
    state
}

#[test]
fn proposals_are_admitted_once_and_retries_must_match_exactly() {
    let mut reviews = BridgeReviews::default();
    let mut state = writable_state();
    let first = proposal("Plan", vec![task("Draft")]);
    let Proposed::Created(id) = reviews
        .propose(&mut state, true, "r1".into(), first.clone(), 10)
        .unwrap()
    else {
        panic!("created");
    };
    assert_eq!(state.workflows.len(), 1);
    assert_eq!(state.workflows[0].status, WorkflowStatus::Previewed);
    assert_eq!(reviews.pending_review_id(&state), Some(id.as_str()));
    // A lost response is retried with the same ID and payload.
    assert_eq!(
        reviews.propose(&mut state, false, "r1".into(), first, 11),
        Ok(Proposed::Existing(id.clone()))
    );
    assert_eq!(
        reviews.propose(
            &mut state,
            true,
            "r1".into(),
            proposal("Changed", vec![task("Draft")]),
            12
        ),
        Err("This request_id was already used for a different title or ordered payload.".into())
    );
    // Nothing new is admitted while the first waits for review.
    assert_eq!(
        reviews.propose(
            &mut state,
            true,
            "r2".into(),
            proposal("Next", vec![task("Other")]),
            13
        ),
        Err("Review or decline the existing MCP request first. No records were changed.".into())
    );
    assert!(state.decline_workflow(&id, 14));
    assert_eq!(reviews.pending_review_id(&state), None);
    assert_eq!(
        reviews.propose(
            &mut state,
            false,
            "r2".into(),
            proposal("Next", vec![task("Other")]),
            15
        ),
        Err("Resolve the pending local save before submitting another proposal.".into())
    );
    assert!(matches!(
        reviews.propose(
            &mut state,
            true,
            "r2".into(),
            proposal("Next", vec![task("Other")]),
            16
        ),
        Ok(Proposed::Created(_))
    ));
}

#[test]
fn proposals_need_every_write_grant_and_a_bounded_request_id() {
    let mut reviews = BridgeReviews::default();
    let mut state = writable_state();
    let mixed = proposal(
        "Mixed",
        vec![
            task("Draft"),
            WorkflowOperation::AddCapture {
                content: "Note".into(),
            },
        ],
    );
    assert_eq!(
        reviews.propose(&mut state, true, "r1".into(), mixed, 1),
        Err("Write access is not granted for this domain. Review permissions in the app.".into())
    );
    assert_eq!(
        reviews.propose(
            &mut state,
            true,
            "x".repeat(129),
            proposal("Plan", vec![task("Draft")]),
            1
        ),
        Err("A request_id of 1 to 128 bytes is required.".into())
    );
    assert!(state.workflows.is_empty());
    let preview = preview_workflow(
        &state.permissions,
        &proposal(
            "Mixed",
            vec![
                task("Draft"),
                WorkflowOperation::AddCapture {
                    content: "Note".into(),
                },
            ],
        ),
    )
    .unwrap();
    assert_eq!(preview["allowed"], false);
    assert_eq!(preview["blocked_domains"], json!(["capture"]));
    assert_eq!(preview["executed"], false);
    assert_eq!(
        preview_operation(&state.permissions, &task("Draft")).unwrap()["allowed"],
        true
    );
    assert!(preview_operation(&state.permissions, &task(" ")).is_err());
}

#[test]
fn status_is_scoped_to_the_session_and_provisional_until_saved() {
    let mut reviews = BridgeReviews::default();
    let mut state = writable_state();
    let Proposed::Created(id) = reviews
        .propose(
            &mut state,
            true,
            "r1".into(),
            proposal("Plan", vec![task("Draft")]),
            1,
        )
        .unwrap()
    else {
        panic!("created");
    };
    assert_eq!(
        BridgeReviews::default().status(&state, true, &id),
        Err("Unknown proposal for this connection session.".into())
    );
    let job = state.start_job(&id, 2).unwrap();
    assert!(state.complete_step(&job, "task-1", 3));
    let provisional = reviews.status(&state, false, &id).unwrap();
    assert_eq!(provisional["executed"], false);
    assert_eq!(provisional["progress"]["completed_steps"], 1);
    assert_eq!(
        provisional["progress"]["durable_completed_steps"],
        Value::Null
    );
    let saved = reviews.status(&state, true, &id).unwrap();
    assert_eq!(saved["executed"], true);
    assert_eq!(saved["progress"]["durable_completed_steps"], 1);
    assert!(!saved.to_string().contains("target_snapshot"));
    state
        .permissions
        .set(AgentDomain::Tasks, AgentAccess::Write, false);
    assert_eq!(
        reviews.status(&state, true, &id),
        Err("Write access was revoked. Review this workflow in the app.".into())
    );
}

#[test]
fn undo_requests_open_one_review_and_a_decline_sticks() {
    let mut reviews = BridgeReviews::default();
    let mut state = writable_state();
    let Proposed::Created(id) = reviews
        .propose(
            &mut state,
            true,
            "r1".into(),
            proposal("Plan", vec![task("Draft")]),
            1,
        )
        .unwrap()
    else {
        panic!("created");
    };
    let job = state.start_job(&id, 2).unwrap();
    assert!(state.complete_step(&job, "task-1", 3));
    assert_eq!(
        reviews.begin_rollback_request(&state, false, &id),
        Err("Wait for a durable local save before requesting undo.".into())
    );
    let request = reviews.begin_rollback_request(&state, true, &id).unwrap();
    assert!(request.needs_targets());
    let (preview, opened) = reviews
        .finish_rollback_request(&state, request, &id, true, 1)
        .unwrap();
    assert!(!opened);
    assert_eq!(preview["eligible"], true);
    assert_eq!(preview["rollback_review"], Value::Null);
    let (asked, opened) = reviews
        .finish_rollback_request(&state, request, &id, false, 1)
        .unwrap();
    assert!(opened);
    assert_eq!(asked["rollback_review"], "pending");
    assert_eq!(reviews.pending_review_id(&state), Some(id.as_str()));
    assert!(reviews.decline_rollback(&id));
    assert!(!reviews.decline_rollback(&id));
    let request = reviews.begin_rollback_request(&state, true, &id).unwrap();
    assert!(!request.needs_targets());
    let (repeat, opened) = reviews
        .finish_rollback_request(&state, request, &id, false, 0)
        .unwrap();
    assert!(!opened);
    assert_eq!(repeat["rollback_review"], "declined");
    assert_eq!(repeat["requires_user_approval"], false);
}

#[test]
fn the_copied_config_names_the_file_and_never_a_token() {
    let config = mcp_client_config("/tmp/bruhs-bridge-x/connection.json");
    assert_eq!(
        config["mcpServers"]["bruhs"]["env"]["BRUHS_WORKSPACE_BRIDGE"],
        "/tmp/bruhs-bridge-x/connection.json"
    );
    assert_eq!(config["mcpServers"]["bruhs"]["command"], "bruhs-mcp");
    assert!(!config.to_string().contains("token"));
}

#[test]
fn responses_carry_app_refusals_as_tool_errors() {
    assert_eq!(
        BridgeResponse::from(Err::<Value, _>("Nope".into())),
        BridgeResponse::Error("Nope".into())
    );
    assert_eq!(
        serde_json::to_value(BridgeResponse::Ok(json!({"a":1}))).unwrap(),
        json!({"status":"ok","value":{"a":1}})
    );
}
