use super::*;
use std::collections::HashSet;

fn run<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

fn standalone(name: &str, args: Value) -> ToolResult {
    let state = Arc::new(AppState::new());
    let tool = lookup(Mode::Standalone, name).unwrap_or_else(|| panic!("{name} is not listed"));
    run(call_standalone(tool, &args, &state))
}

fn text(result: &ToolResult) -> &str {
    &result.content[0].text
}

#[test]
fn every_listed_tool_is_dispatchable_in_the_mode_that_lists_it() {
    for mode in [Mode::Standalone, Mode::Bridge] {
        let listed = list_tools(mode);
        let names: HashSet<_> = listed.iter().map(|tool| tool.name.as_str()).collect();
        assert_eq!(names.len(), listed.len(), "{mode:?} lists a name twice");
        for name in names {
            assert!(
                lookup(mode, name).is_some(),
                "{mode:?} lists {name} but cannot dispatch it"
            );
        }
    }
}

#[test]
fn a_tool_unlisted_in_a_mode_is_unknown_in_that_mode() {
    for spec in registry() {
        for mode in [Mode::Standalone, Mode::Bridge] {
            let listed = list_tools(mode)
                .iter()
                .any(|tool| tool.name == spec.info.name);
            assert_eq!(
                lookup(mode, &spec.info.name).is_some(),
                listed,
                "{mode:?} {}",
                spec.info.name
            );
        }
    }
}

#[test]
fn standalone_dispatch_never_reaches_the_bridge_only_fallback() {
    for info in list_tools(Mode::Standalone) {
        let result = standalone(&info.name, json!({}));
        assert!(
            !text(&result).contains("requires the native workspace connection"),
            "{}: {}",
            info.name,
            text(&result)
        );
        assert!(!text(&result).contains("Unknown tool"), "{}", info.name);
    }
}

#[test]
fn the_catalog_agrees_with_the_modes_that_can_answer() {
    let standalone: HashSet<_> = list_tools(Mode::Standalone)
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    let bridge: HashSet<_> = list_tools(Mode::Bridge)
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    // Only the app has these domains' data.
    for domain in AgentDomain::ALL {
        let read = format!("bruhs_read_{}", domain.key());
        assert!(bridge.contains(&read));
        assert_eq!(standalone.contains(&read), domain == AgentDomain::Chat);
        let write = format!("bruhs_write_{}", domain.key());
        assert!(bridge.contains(&write) && standalone.contains(&write));
    }
    for name in [
        "bruhs_propose_workflow",
        "bruhs_workflow_status",
        "bruhs_request_workflow_rollback",
        "bruhs_read_meeting",
        "bruhs_read_chat_messages",
    ] {
        assert!(
            bridge.contains(name) && !standalone.contains(name),
            "{name}"
        );
    }
    for name in [
        "bruhs_list_threads",
        "bruhs_list_tasks",
        "bruhs_agent_audit",
    ] {
        assert!(
            standalone.contains(name) && !bridge.contains(name),
            "{name}"
        );
    }
}

#[test]
fn registry_names_match_the_shared_tool_definitions() {
    let listed: HashSet<_> = list_tools(Mode::Bridge)
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    for definition in agent_tool_definitions() {
        assert!(listed.contains(&definition.name), "{}", definition.name);
    }
}

#[test]
fn legacy_unreachable_tools_are_gone_in_both_modes() {
    // Their Chat/Tasks Write grant can never be given (the default policy
    // grants no Write and nothing can change it), so they could only fail.
    for name in [
        "bruhs_create_thread",
        "bruhs_send_message",
        "bruhs_create_work_item",
        "bruhs_rollback_workflow",
        "bruhs_set_agent_capability",
    ] {
        for mode in [Mode::Standalone, Mode::Bridge] {
            assert!(lookup(mode, name).is_none(), "{mode:?} {name}");
            assert!(!list_tools(mode).iter().any(|tool| tool.name == name));
        }
    }
}

#[test]
fn the_default_policy_grants_no_write_so_no_chat_or_task_write_tool_could_work() {
    let policy = bruhs_models::AgentPermissionPolicy::default();
    for domain in AgentDomain::ALL {
        assert!(!policy.allows(domain, AgentAccess::Write), "{domain:?}");
    }
}

#[test]
fn unattached_write_never_returns_a_completed_receipt() {
    let result = standalone(
        "bruhs_write_tasks",
        json!({
            "operation": {"type":"create_task", "title":"Must not pretend"},
            "dry_run":false
        }),
    );
    assert_eq!(result.is_error, Some(true));
    assert!(!text(&result).contains("target_id"));
}

#[test]
fn a_standalone_write_preview_reports_the_missing_grant() {
    let result = standalone(
        "bruhs_write_tasks",
        json!({"operation": {"type":"create_task", "title":"Preview"}}),
    );
    assert_eq!(result.is_error, None);
    let body: Value = serde_json::from_str(text(&result)).unwrap();
    assert_eq!(body["dry_run"], true);
    assert_eq!(body["allowed"], false);
    assert_eq!(body["blocked_domains"], json!(["tasks"]));
}

#[test]
fn typed_write_rejects_an_operation_for_another_domain() {
    let result = standalone(
        "bruhs_write_calendar",
        json!({
            "operation": { "type": "create_task", "title": "Mismatch" },
            "dry_run": true
        }),
    );
    assert_eq!(result.is_error, Some(true));
    assert!(text(&result).contains("not Calendar"));
}

#[test]
fn typed_write_rejects_invalid_content_before_creating_a_workflow() {
    let result = standalone(
        "bruhs_write_tasks",
        json!({
            "operation": { "type": "create_task", "title": "  " },
            "dry_run": true
        }),
    );
    assert_eq!(result.is_error, Some(true));
    assert!(text(&result).contains("No workflow was created"));
}

#[test]
fn argument_validation_failures_are_uniformly_tool_errors() {
    // Missing, wrong-typed and malformed arguments are all `isError` results
    // (the transport maps only a missing name or unknown tool to -32602).
    for args in [
        json!({}),
        json!({"thread_id": 3}),
        json!({"thread_id": ""}),
        json!({"thread_id": "x".repeat(129)}),
    ] {
        let result = standalone("bruhs_list_tasks", args.clone());
        assert_eq!(result.is_error, Some(true), "{args}");
        assert!(
            text(&result).contains("bruhs_list_tasks"),
            "{args}: {}",
            text(&result)
        );
    }
    let result = standalone("bruhs_list_tasks", json!({"thread_id": "t-1"}));
    assert_eq!(result.is_error, None);
    assert_eq!(text(&result), "[]");
}

#[test]
fn standalone_reads_keep_the_caps_of_the_specific_readers() {
    for args in [
        json!({"query": "x".repeat(513)}),
        json!({"offset": 100_001}),
        json!({"offset": -1}),
    ] {
        let result = standalone("bruhs_read_chat", args.clone());
        assert_eq!(result.is_error, Some(true), "{args}");
    }
    let result = standalone("bruhs_read_chat", json!({"query": "x", "offset": 100_000}));
    assert_eq!(result.is_error, None);
}

#[test]
fn listing_threads_reports_an_empty_workspace() {
    let result = standalone("bruhs_list_threads", Value::Null);
    assert_eq!(result.is_error, None);
    assert_eq!(text(&result), "[]");
}

#[test]
fn the_audit_timeline_is_readable() {
    let result = standalone("bruhs_agent_audit", Value::Null);
    assert_eq!(result.is_error, None);
    let body: Value = serde_json::from_str(text(&result)).unwrap();
    assert!(body["permissions"]["grants"].is_array());
}
