use super::super::{load_desktop_state_from_path, SaveStatus};
use super::tests::{call, connect, fixture};
use super::WorkspaceBridge;
use bruhs_core::workspace_bridge::{BridgeRequest, BridgeResponse, WorkflowProposal};
use bruhs_models::{AgentAccess, AgentDomain, WorkflowOperation, WorkflowStatus};
use gpui::{TestAppContext, VisualTestContext};

fn proposal() -> WorkflowProposal {
    WorkflowProposal {
        title: "One reviewed day plan".into(),
        operations: vec![
            WorkflowOperation::CreateTask {
                title: "Write the brief".into(),
            },
            WorkflowOperation::CreateCalendarEvent {
                title: "Focus".into(),
                start_at_ms: 1_000.,
                end_at_ms: 61_000.,
            },
            WorkflowOperation::AddCapture {
                content: "Decision log".into(),
            },
            WorkflowOperation::CreateKnowledgeNote {
                title: "Brief".into(),
                body: "Reviewed document".into(),
            },
            WorkflowOperation::DraftInboxMessage {
                recipient: "Ada".into(),
                body: "Reviewed draft".into(),
            },
        ],
    }
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let button = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("Missing workflow control: {selector}"));
    cx.simulate_click(button.center(), gpui::Modifiers::default());
}

#[gpui::test]
async fn batch_rejects_any_missing_grant_or_invalid_adapter_before_creating_workflow(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    connect(&workspace, cx).await;
    let BridgeResponse::Ok(preview) = call(
        &workspace,
        cx,
        BridgeRequest::PreviewWorkflow {
            proposal: proposal(),
        },
    )
    .await
    else {
        panic!("preview")
    };
    assert_eq!(preview["allowed"], false);
    assert_eq!(preview["blocked_domains"].as_array().unwrap().len(), 5);
    workspace.update_in(cx, |w, _, _| {
        for operation in proposal().operations {
            w.automation_state
                .permissions
                .set(operation.domain(), AgentAccess::Write, true);
        }
        w.automation_state
            .permissions
            .set(AgentDomain::Inbox, AgentAccess::Write, false);
    });
    assert!(matches!(
        call(
            &workspace,
            cx,
            BridgeRequest::ProposeWorkflow {
                request_id: "missing-last-grant".into(),
                proposal: proposal()
            }
        )
        .await,
        BridgeResponse::Error(_)
    ));
    let mut invalid_note = proposal();
    invalid_note
        .operations
        .push(WorkflowOperation::CreateMeetingNote {
            title: " ".into(),
            body: String::new(),
        });
    assert!(matches!(
        call(
            &workspace,
            cx,
            BridgeRequest::ProposeWorkflow {
                request_id: "invalid-note-last".into(),
                proposal: invalid_note
            }
        )
        .await,
        BridgeResponse::Error(_)
    ));
    let mut invalid = proposal();
    invalid
        .operations
        .push(WorkflowOperation::AddCapture { content: "".into() });
    assert!(matches!(
        call(
            &workspace,
            cx,
            BridgeRequest::PreviewWorkflow { proposal: invalid }
        )
        .await,
        BridgeResponse::Error(_)
    ));
    workspace.update_in(cx, |w, _, _| {
        assert!(w.automation_state.workflows.is_empty());
        assert!(w.mcp_bridge.proposals.is_empty());
        assert!(w.work_items.is_empty());
        w.mcp_bridge = WorkspaceBridge::default();
    });
}

#[gpui::test]
async fn one_batch_review_yields_saved_five_domain_progress_and_undo(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    let directory =
        std::env::temp_dir().join(format!("bruhs-batch-durability-{}", uuid::Uuid::new_v4()));
    let path = directory.join("state.json");
    workspace.update_in(cx, |w, _, _| {
        w.persistence_test_path = Some(path.clone());
        for operation in proposal().operations {
            w.automation_state
                .permissions
                .set(operation.domain(), AgentAccess::Write, true);
        }
    });
    connect(&workspace, cx).await;
    let request = BridgeRequest::ProposeWorkflow {
        request_id: "five-domain-plan".into(),
        proposal: proposal(),
    };
    let BridgeResponse::Ok(first) = call(&workspace, cx, request.clone()).await else {
        panic!("proposal")
    };
    assert_eq!(first["executed"], false);
    assert_eq!(first["progress"]["total_steps"], 5);
    assert!(first["job"].is_null());
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let status_request = BridgeRequest::Status {
        workflow_id: first["workflow_id"].as_str().unwrap().into(),
    };
    click(cx, "desktop-start-workflow");
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    for completed in 1..=5 {
        click(cx, "desktop-run-workflow-step");
        workspace.update_in(cx, |w, _, _| assert_eq!(w.save_status, SaveStatus::Saving));
        cx.condition(&workspace, |w, _| w.save_status.is_saved())
            .await;
        let BridgeResponse::Ok(status) = call(&workspace, cx, status_request.clone()).await else {
            panic!("saved progress")
        };
        assert_eq!(status["progress"]["durable_completed_steps"], completed);
        assert_eq!(status["job"]["next_step"], completed);
        assert_eq!(status["job"]["can_advance"], completed < 5);
        assert_eq!(status["executed"], completed == 5);
        let saved = load_desktop_state_from_path(&path).unwrap().unwrap();
        assert_eq!(
            saved.work_items.len()
                + saved.calendar_events.len()
                + saved.captures.len()
                + saved.knowledge_documents.len()
                + saved.inbox_conversations.len(),
            completed
        );
        assert_eq!(saved.automation_state.jobs[0].next_step, completed);
    }
    let BridgeResponse::Ok(retry) = call(&workspace, cx, request).await else {
        panic!("retry")
    };
    assert_eq!(retry["workflow_id"], first["workflow_id"]);
    for changed in [false, true] {
        let mut payload = proposal();
        if changed {
            payload.operations.swap(0, 1);
        } else {
            payload.title.push_str(" changed");
        }
        assert!(matches!(
            call(
                &workspace,
                cx,
                BridgeRequest::ProposeWorkflow {
                    request_id: "five-domain-plan".into(),
                    proposal: payload
                }
            )
            .await,
            BridgeResponse::Error(_)
        ));
    }
    click(cx, "desktop-undo-workflow");
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let BridgeResponse::Ok(undone) = call(&workspace, cx, status_request).await else {
        panic!("undo status")
    };
    assert_eq!(undone["workflow_status"], "rolled_back");
    assert_eq!(undone["progress"]["durable_reverted_steps"], 5);
    assert_eq!(undone["executed"], false);
    let saved = load_desktop_state_from_path(&path).unwrap().unwrap();
    assert!(
        saved.work_items.is_empty()
            && saved.calendar_events.is_empty()
            && saved.captures.is_empty()
            && saved.knowledge_documents.is_empty()
            && saved.inbox_conversations.is_empty()
    );
    assert_eq!(saved.automation_state.workflows.len(), 1);
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
    std::fs::remove_dir_all(directory).unwrap();
}

#[gpui::test]
async fn batch_rechecks_revocation_and_changed_later_payload_between_steps(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, _| {
        for operation in proposal().operations {
            w.automation_state
                .permissions
                .set(operation.domain(), AgentAccess::Write, true);
        }
    });
    connect(&workspace, cx).await;
    let BridgeResponse::Ok(first) = call(
        &workspace,
        cx,
        BridgeRequest::ProposeWorkflow {
            request_id: "revoked-plan".into(),
            proposal: proposal(),
        },
    )
    .await
    else {
        panic!("proposal")
    };
    click(cx, "desktop-start-workflow");
    click(cx, "desktop-run-workflow-step");
    workspace.update_in(cx, |w, _, _| {
        w.automation_state
            .permissions
            .set(AgentDomain::Calendar, AgentAccess::Write, false)
    });
    assert!(matches!(
        call(
            &workspace,
            cx,
            BridgeRequest::Status {
                workflow_id: first["workflow_id"].as_str().unwrap().into()
            }
        )
        .await,
        BridgeResponse::Error(_)
    ));
    click(cx, "desktop-run-workflow-step");
    workspace.update_in(cx, |w, _, _| {
        assert_eq!(w.work_items.len(), 1);
        assert!(w.calendar_events.is_empty());
        assert_eq!(w.automation_state.jobs[0].next_step, 1);
        w.automation_state
            .permissions
            .set(AgentDomain::Calendar, AgentAccess::Write, true);
        w.automation_state.workflows[0].steps[4].operation = WorkflowOperation::DraftInboxMessage {
            recipient: "Someone else".into(),
            body: "Unreviewed".into(),
        };
    });
    click(cx, "desktop-run-workflow-step");
    workspace.update_in(cx, |w, _, _| {
        assert!(w.automation_error.is_some());
        assert!(w.calendar_events.is_empty());
        assert!(w.inbox_conversations.is_empty());
        assert_eq!(
            w.automation_state.workflows[0].status,
            WorkflowStatus::Running
        );
        w.mcp_bridge = WorkspaceBridge::default();
    });
}
