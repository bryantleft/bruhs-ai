use super::*;
use crate::tools::{lookup, Mode};

/// By tool name, as a client calls it. A name the bridge does not list cannot
/// be translated.
fn translate(name: &str, args: &Value) -> Result<BridgeRequest, ArgumentError> {
    super::translate(
        lookup(Mode::Bridge, name).ok_or(ArgumentError::NotBridged)?,
        args,
    )
}

#[test]
fn life_read_arguments_are_strict_and_cannot_change_privacy() {
    for (name, kind, sections, default) in [
        (
            "bruhs_read_finance",
            "read_finance",
            &[
                "accounts",
                "transactions",
                "budgets",
                "income_streams",
                "holdings",
            ][..],
            "transactions",
        ),
        (
            "bruhs_read_health",
            "read_health",
            &["workouts", "sleep", "recovery", "habits"][..],
            "workouts",
        ),
    ] {
        for empty in [Value::Null, json!({})] {
            let request = serde_json::to_value(translate(name, &empty).unwrap()).unwrap();
            assert_eq!(request["kind"], kind);
            assert_eq!(request["section"], default);
            assert_eq!(request["offset"], 0);
        }
        for section in sections {
            let args = json!({"section":section,"query":"Walk 🗒","offset":50});
            let request = serde_json::to_value(translate(name, &args).unwrap()).unwrap();
            assert_eq!(request["section"], *section);
            assert_eq!(request["query"], "Walk 🗒");
            assert_eq!(request["offset"], 50);
        }
        for args in [
            json!([]),
            json!(true),
            json!({"section":"private"}),
            json!({"section":null}),
            json!({"query":1}),
            json!({"query":"x".repeat(513)}),
            json!({"offset":-1}),
            json!({"offset":1.5}),
            json!({"offset":"50"}),
            json!({"offset":100001}),
            json!({"allowed":true}),
            json!({"allow_agent_context":true}),
            json!({"reveal_exact_amounts_to_agent":true}),
            json!({"kind":"read_finance"}),
        ] {
            assert!(translate(name, &args).is_err(), "{name}: {args}");
        }
    }
}

#[test]
fn meeting_note_tools_keep_strict_identity_and_text_contract() {
    assert!(matches!(
        translate("bruhs_read_meeting_notes", &json!({})).unwrap(),
        BridgeRequest::ReadMeetingNotes { offset: 0, .. }
    ));
    assert!(
        matches!(translate("bruhs_read_meeting_note", &json!({"note_id":"n"})).unwrap(), BridgeRequest::ReadMeetingNote { note_id } if note_id == "n")
    );
    for arguments in [
        json!({"note_id":""}),
        json!({"note_id":"n", "body":true}),
        json!({"note_id":"x".repeat(129)}),
    ] {
        assert!(translate("bruhs_read_meeting_note", &arguments).is_err());
    }
    for arguments in [
        json!({"offset":-1}),
        json!({"offset":100001}),
        json!({"query":"x".repeat(513)}),
        json!({"body":true}),
    ] {
        assert!(translate("bruhs_read_meeting_notes", &arguments).is_err());
    }
    let request = translate("bruhs_write_meetings", &json!({"operation":{"type":"create_meeting_note", "title":"Plan", "body":"  Exact 🗒\ntext  "}})).unwrap();
    assert!(
        matches!(request, BridgeRequest::Preview { operation: WorkflowOperation::CreateMeetingNote { title, body } } if title == "Plan" && body == "  Exact 🗒\ntext  ")
    );
}

#[test]
fn meeting_reads_translate_sections_and_reject_unknown_or_invalid_arguments() {
    assert!(matches!(
        translate("bruhs_read_meeting", &json!({"meeting_id":"m"})).unwrap(),
        BridgeRequest::ReadMeeting {
            section: MeetingReadSection::Summary,
            offset: 0,
            ..
        }
    ));
    assert!(matches!(
        translate(
            "bruhs_read_meeting",
            &json!({"meeting_id":"m","section":"transcript","offset":50})
        )
        .unwrap(),
        BridgeRequest::ReadMeeting {
            section: MeetingReadSection::Transcript,
            offset: 50,
            ..
        }
    ));
    for args in [
        json!({"meeting_id":""}),
        json!({"meeting_id":"m","section":"audio"}),
        json!({"meeting_id":"m","offset":1}),
        json!({"meeting_id":"m","section":"transcript","offset":-1}),
        json!({"meeting_id":"m","grant":true}),
        json!({"meeting_id":"m","section":"transcript","offset":100001}),
    ] {
        assert!(translate("bruhs_read_meeting", &args).is_err(), "{args}");
    }
}
use serde_json::json;

#[test]
fn person_read_parser_preserves_sections_and_rejects_ambiguous_arguments() {
    assert!(
        matches!(translate("bruhs_read_person", &json!({"person_id":"exact"})).unwrap(),
        BridgeRequest::ReadPerson { person_id, section: PersonReadSection::Profile, offset: 0 } if person_id == "exact")
    );
    assert!(matches!(
        translate(
            "bruhs_read_person",
            &json!({"person_id":"exact","section":"relationship_items","offset":50})
        )
        .unwrap(),
        BridgeRequest::ReadPerson {
            section: PersonReadSection::RelationshipItems,
            offset: 50,
            ..
        }
    ));
    for args in [
        json!({}),
        json!({"person_id":""}),
        json!({"person_id":"x".repeat(129)}),
        json!({"person_id":"exact","section":"voice_embedding"}),
        json!({"person_id":"exact","offset":1}),
        json!({"person_id":"exact","section":"observations","offset":100001}),
        json!({"person_id":"exact","offset":-1}),
        json!({"person_id":"exact","include_voice":true}),
    ] {
        assert!(translate("bruhs_read_person", &args).is_err(), "{args}");
    }
}

#[test]
fn people_summary_parser_never_ignores_a_person_id_or_unbounded_filter() {
    assert!(matches!(
        translate("bruhs_read_people", &Value::Null).unwrap(),
        BridgeRequest::Read {
            domain: AgentDomain::People,
            offset: 0,
            ..
        }
    ));
    assert!(
        matches!(translate("bruhs_read_people", &json!({"query":"Alex","offset":50})).unwrap(),
        BridgeRequest::Read { domain: AgentDomain::People, query, offset: 50 } if query == "Alex")
    );
    for args in [
        json!({"person_id":"exact"}),
        json!({"query":"x".repeat(513)}),
        json!({"offset":100001}),
        json!({"offset":-1}),
    ] {
        assert!(translate("bruhs_read_people", &args).is_err(), "{args}");
    }
}

#[test]
fn chat_history_request_is_typed_and_rejects_ambiguous_or_unbounded_arguments() {
    assert!(
        matches!(translate("bruhs_read_chat_messages", &json!({"thread_id":"local"})).unwrap(),
        BridgeRequest::ReadChatMessages { thread_id, offset: 0 } if thread_id == "local")
    );
    for args in [
        json!({"thread_id":"local","offset":-1}),
        json!({"thread_id":"local","offset":1.5}),
        json!({"thread_id":"local","offset":100001}),
        json!({"thread_id":""}),
        json!({"thread_id":"x".repeat(129)}),
        json!({"thread_id":"local","query":"ignored?"}),
        json!({"thread_id":"local","domain":"finance"}),
    ] {
        assert!(
            translate("bruhs_read_chat_messages", &args).is_err(),
            "{args}"
        );
    }
}

#[test]
fn life_writes_preserve_exact_amounts_and_reject_unreviewable_defaults() {
    let mut args = json!({"operation":{"type":"add_finance_transaction","merchant":"Receipt","category":"Food","amount_minor":"-9007199254740993"}});
    assert!(matches!(
        translate("bruhs_write_finance", &args).unwrap(),
        BridgeRequest::Preview {
            operation: WorkflowOperation::AddFinanceTransaction {
                amount_minor: -9_007_199_254_740_993,
                ..
            }
        }
    ));
    assert!(translate("bruhs_write_health", &args).is_err());
    args["operation"]["currency"] = "EUR".into();
    assert!(translate("bruhs_write_finance", &args).is_err());
    args["operation"]
        .as_object_mut()
        .unwrap()
        .remove("currency");
    for amount in [
        json!("9223372036854775808"),
        json!("-9223372036854775808"),
        json!("1.25"),
        json!(1.25),
    ] {
        args["operation"]["amount_minor"] = amount;
        assert!(translate("bruhs_write_finance", &args).is_err());
    }
    for minutes in [0, 1441] {
        assert!(translate(
            "bruhs_write_health",
            &json!({"operation":{"type":"log_workout","activity":"Walk","minutes":minutes}})
        )
        .is_err());
    }
}

#[test]
fn rollback_requests_default_to_preview_and_cannot_supply_approval() {
    let name = "bruhs_request_workflow_rollback";
    assert!(matches!(
        translate(name, &json!({"workflow_id":"owned"})).unwrap(),
        BridgeRequest::RequestRollback { dry_run: true, .. }
    ));
    assert!(matches!(
        translate(name, &json!({"workflow_id":"owned","dry_run":false})).unwrap(),
        BridgeRequest::RequestRollback { dry_run: false, .. }
    ));
    for args in [
        json!({"workflow_id":"owned","approved":true}),
        json!({"workflow_id":"owned","execute":true}),
        json!({"workflow_id":"owned","dry_run":"false"}),
        json!({"workflow_id":" "}),
        json!({"workflow_id":"x".repeat(129)}),
    ] {
        assert!(translate(name, &args).is_err());
    }
}

#[test]
fn batch_defaults_to_preview_and_submission_preserves_operation_order() {
    let mut args = json!({"title":"Plan", "operations":[{"type":"create_task","title":"First"},{"type":"add_capture","content":"Second"}]});
    assert!(matches!(
        translate("bruhs_propose_workflow", &args).unwrap(),
        BridgeRequest::PreviewWorkflow { .. }
    ));
    args["dry_run"] = false.into();
    assert!(translate("bruhs_propose_workflow", &args).is_err());
    args["request_id"] = "same-batch".into();
    let BridgeRequest::ProposeWorkflow {
        request_id,
        proposal,
    } = translate("bruhs_propose_workflow", &args).unwrap()
    else {
        panic!("proposal")
    };
    assert_eq!(request_id, "same-batch");
    assert_eq!(proposal.operations[0].domain(), AgentDomain::Tasks);
    assert_eq!(proposal.operations[1].domain(), AgentDomain::Capture);
}

#[test]
fn batch_rejects_invalid_late_payload_and_caller_approval_flags() {
    let mut args = json!({"title":"Plan", "operations":[{"type":"create_task","title":"First"},{"type":"add_capture","content":""}]});
    assert!(translate("bruhs_propose_workflow", &args).is_err());
    args["operations"][1]["content"] = "Valid".into();
    args["approved"] = true.into();
    assert!(translate("bruhs_propose_workflow", &args).is_err());
}

#[test]
fn write_defaults_to_preview_and_never_an_execution_command() {
    let args = json!({"operation":{"type":"create_task","title":"task"}});
    assert!(matches!(
        translate("bruhs_write_tasks", &args).unwrap(),
        BridgeRequest::Preview { .. }
    ));
}

#[test]
fn submissions_require_idempotency_id_and_correct_domain() {
    let mut args = json!({"operation":{"type":"create_task","title":"task"},"dry_run":false});
    assert!(translate("bruhs_write_tasks", &args).is_err());
    args["request_id"] = json!("one-request");
    assert!(matches!(
        translate("bruhs_write_tasks", &args).unwrap(),
        BridgeRequest::Propose { .. }
    ));
    assert!(translate("bruhs_write_calendar", &args).is_err());
}

#[test]
fn unsupported_aliases_and_self_approval_are_rejected() {
    for name in [
        "bruhs_set_agent_capability",
        "bruhs_approve",
        "bruhs_rollback_workflow",
        "bruhs_create_work_item",
        "bruhs_send_message",
    ] {
        assert!(translate(name, &json!({})).is_err());
    }
}

#[test]
fn generic_reads_enforce_the_same_caps_as_the_specific_readers() {
    for name in [
        "bruhs_read_tasks",
        "bruhs_read_calendar",
        "bruhs_read_capture",
        "bruhs_read_knowledge",
        "bruhs_read_inbox",
        "bruhs_read_chat",
        "bruhs_read_meetings",
    ] {
        assert!(matches!(
            translate(name, &json!({"query":"x".repeat(512),"offset":100_000})).unwrap(),
            BridgeRequest::Read {
                offset: 100_000,
                ..
            }
        ));
        for args in [
            json!({"query":"x".repeat(513)}),
            json!({"offset":100_001}),
            json!({"offset":-1}),
            json!({"offset":1.5}),
            json!({"query":7}),
        ] {
            assert!(translate(name, &args).is_err(), "{name}: {args}");
        }
        // The schema of the generic reads does not forbid extra fields.
        assert!(translate(name, &json!({"limit":5})).is_ok());
    }
}

#[test]
fn request_ids_and_workflow_ids_are_capped_at_the_advertised_128_bytes() {
    let mut write = json!({"operation":{"type":"create_task","title":"task"},"dry_run":false});
    write["request_id"] = "r".repeat(128).into();
    assert!(translate("bruhs_write_tasks", &write).is_ok());
    for bad in [json!("r".repeat(129)), json!(" "), json!("")] {
        write["request_id"] = bad;
        assert!(translate("bruhs_write_tasks", &write).is_err());
    }
    let mut batch = json!({"title":"Plan","operations":[{"type":"create_task","title":"First"}],"dry_run":false});
    batch["request_id"] = "r".repeat(129).into();
    assert!(translate("bruhs_propose_workflow", &batch).is_err());
    batch["request_id"] = "r".repeat(128).into();
    assert!(translate("bruhs_propose_workflow", &batch).is_ok());

    assert!(matches!(
        translate(
            "bruhs_workflow_status",
            &json!({"workflow_id":"w".repeat(128)})
        )
        .unwrap(),
        BridgeRequest::Status { .. }
    ));
    for args in [
        json!({}),
        json!({"workflow_id":""}),
        json!({"workflow_id":"w".repeat(129)}),
        json!({"workflow_id":"w","approved":true}),
    ] {
        assert!(translate("bruhs_workflow_status", &args).is_err(), "{args}");
    }
}

#[test]
fn argument_errors_name_the_tool_and_the_problem() {
    let missing = translate("bruhs_read_chat_messages", &json!({}))
        .unwrap_err()
        .to_string();
    assert!(missing.contains("bruhs_read_chat_messages") && missing.contains("thread_id"));
    let malformed = translate("bruhs_read_chat_messages", &json!({"thread_id": ""}))
        .unwrap_err()
        .to_string();
    assert!(malformed.contains("thread_id") || malformed.contains("non-blank"));
}

#[test]
fn every_listed_bridge_tool_translates_to_something_other_than_not_connected() {
    for tool in crate::tools::list_tools(Mode::Bridge) {
        let outcome = translate(&tool.name, &json!({}));
        assert!(
            !matches!(outcome, Err(ArgumentError::NotBridged)),
            "{} is listed but not connected",
            tool.name
        );
    }
}

#[test]
fn standalone_only_tools_are_unknown_to_the_bridge() {
    for name in [
        "bruhs_list_threads",
        "bruhs_list_tasks",
        "bruhs_agent_audit",
    ] {
        assert!(lookup(Mode::Bridge, name).is_none(), "{name}");
    }
}
