use super::*;
use crate::state::{MessagePart, MessageStatus, ToolCallInfo, UsageInfo};
use bruhs_models::workspace_bridge::{read_chat_messages, WorkflowProposal};
use bruhs_models::{AgentAccess, WorkflowOperation};
use serde_json::json;

fn assistant() -> AgentMessage {
    let mut message = AgentMessage::assistant_placeholder("claude-opus".into(), 1_000.0);
    message.status = MessageStatus::Finished {
        at_ms: Some(1_250.0),
    };
    message.usage = Some(UsageInfo {
        input_tokens: 1,
        output_tokens: 2,
        cache_read_tokens: 3,
        cache_creation_tokens: 4,
        context_tokens: Some(5),
    });
    message.parts = vec![
        MessagePart::Thinking("Plan first.".into()),
        MessagePart::Text("Here is ".into()),
        MessagePart::ToolCall(ToolCallInfo {
            id: "c1".into(),
            tool: "Read".into(),
            input: json!({"path":"a.rs"}),
            output: Some("ok".into()),
            is_error: false,
            started_at_ms: Some(1_100.0),
            ended_at_ms: Some(1_150.0),
        }),
        MessagePart::Text("the answer.".into()),
    ];
    message
}

#[test]
fn web_messages_project_into_the_record_shape_desktop_returns() {
    let message = assistant();
    let record = serde_json::to_value(message_record(&message)).unwrap();
    assert_eq!(
        record,
        json!({
            "role":"assistant","content":"Here is the answer.","model":"claude-opus","thinking":"Plan first.",
            "usage":{"input_tokens":1,"output_tokens":2,"cache_read_input_tokens":3,"cache_creation_input_tokens":4,"context_tokens":5},
            "duration_ms":250,"started_at_ms":1000,"completed_at_ms":1250,
            "tool_calls":[{"id":"c1","tool":"Read","input":{"path":"a.rs"},"output":"ok","is_error":false}],
            "stopped":false,"error":null
        })
    );
    // The web's own per-call timings and message IDs stay out of the shape.
    assert!(record.get("id").is_none());
    assert!(record["tool_calls"][0].get("started_at_ms").is_none());
}

/// A turn the user stopped reports `stopped` like desktop's; a failed one
/// reports its error instead, never both.
#[test]
fn stopped_turns_report_stopped_and_failed_ones_their_error() {
    let mut stopped = assistant();
    stopped.finish(1_400.0, true, None);
    let record = message_record(&stopped);
    assert!(record.stopped);
    assert_eq!(record.error, None);
    assert_eq!(record.completed_at_ms, Some(1_400));
    assert_eq!(record.duration_ms, Some(400));

    let mut failed = assistant();
    failed.error = Some("provider error".into());
    failed.finish(1_400.0, true, None);
    let record = message_record(&failed);
    assert!(!record.stopped);
    assert_eq!(record.error, Some("provider error"));
}

#[test]
fn streaming_messages_and_their_offsets_are_never_counted() {
    let streaming = AgentMessage::assistant_placeholder("claude".into(), 2_000.0);
    let messages = [
        AgentMessage::user("Question".into(), 900.0),
        assistant(),
        streaming,
    ];
    let page = read_chat_messages(
        messages.iter().filter(|message| !message.is_streaming()),
        message_record,
        "t1",
        1,
    )
    .unwrap();
    assert_eq!(page["records"].as_array().unwrap().len(), 1);
    assert_eq!(page["records"][0]["content"], "Here is the answer.");
    assert!(page["next_offset"].is_null());
}

#[test]
fn unknown_or_backwards_times_have_no_duration() {
    let mut message = assistant();
    message.status = MessageStatus::Finished { at_ms: None };
    assert_eq!(message_record(&message).duration_ms, None);
    message.status = MessageStatus::Finished { at_ms: Some(500.0) };
    assert_eq!(message_record(&message).duration_ms, None);
}

#[test]
fn a_saved_undo_settles_only_its_own_pending_request() {
    let mut state = AutomationState::default();
    state
        .permissions
        .set(AgentDomain::Tasks, AgentAccess::Write, true);
    let mut reviews = BridgeReviews::default();
    let mut proposed = Vec::new();
    for request in ["r1", "r2"] {
        let proposal = WorkflowProposal {
            title: request.into(),
            operations: vec![WorkflowOperation::CreateTask {
                title: request.into(),
            }],
        };
        let Proposed::Created(id) = reviews
            .propose(&mut state, true, request.into(), proposal, 1)
            .unwrap()
        else {
            panic!("created");
        };
        let job = state.start_job(&id, 2).unwrap();
        assert!(state.complete_step(&job, format!("task-{request}"), 3));
        proposed.push(id);
    }
    for id in &proposed {
        reviews
            .rollback_reviews
            .insert(id.clone(), RollbackReview::Pending);
    }
    assert!(rolled_back_reviews(&reviews, &state).is_empty());
    assert!(state.mark_rolled_back(&proposed[0], 4));
    assert_eq!(
        rolled_back_reviews(&reviews, &state),
        vec![proposed[0].clone()]
    );
}

#[test]
fn transcripts_validate_only_when_loaded() {
    let segments = BTreeMap::new();
    assert_eq!(
        validate_segments(&segments, "m1"),
        Err(TranscriptValidationError::InvalidSegments)
    );
}
