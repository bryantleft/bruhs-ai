use super::super::{load_desktop_state_from_path, SaveStatus, Workspace};
use super::tests::{call, connect, fixture};
use super::{RollbackReview, WorkspaceBridge};
use bruhs_core::workspace_bridge::{BridgeRequest, BridgeResponse, WorkflowProposal};
use bruhs_models::{AgentAccess, AgentDomain, WorkflowOperation, WorkflowStatus};
use gpui::{Entity, TestAppContext, VisualTestContext};
use serde_json::Value;

mod safety;

fn ok(response: BridgeResponse) -> Value {
    match response {
        BridgeResponse::Ok(value) => value,
        error => panic!("{error:?}"),
    }
}

fn request(id: &str, dry_run: bool) -> BridgeRequest {
    BridgeRequest::RequestRollback {
        workflow_id: id.into(),
        dry_run,
    }
}

async fn click(workspace: &Entity<Workspace>, cx: &mut VisualTestContext, selector: &'static str) {
    cx.condition(workspace, |w, _| w.save_status.is_saved())
        .await;
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("Missing {selector}"));
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    cx.condition(workspace, |w, _| w.save_status.is_saved())
        .await;
    cx.run_until_parked();
}

async fn started(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
    steps: usize,
) -> String {
    workspace.update_in(cx, |w, _, _| {
        w.automation_state
            .permissions
            .set(AgentDomain::Tasks, AgentAccess::Write, true)
    });
    connect(workspace, cx).await;
    let proposal = ok(call(
        workspace,
        cx,
        BridgeRequest::ProposeWorkflow {
            request_id: "undo-plan".into(),
            proposal: WorkflowProposal {
                title: "Review requested undo".into(),
                operations: (0..steps)
                    .map(|index| WorkflowOperation::CreateTask {
                        title: format!("Undo test {index}"),
                    })
                    .collect(),
            },
        },
    )
    .await);
    click(workspace, cx, "desktop-start-workflow").await;
    proposal["workflow_id"]
        .as_str()
        .expect("accepted workflow id")
        .into()
}

#[gpui::test]
async fn requested_undo_changes_no_records_until_user_approval_and_durable_save(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let directory = std::env::temp_dir().join(format!("bruhs-mcp-undo-{}", uuid::Uuid::new_v4()));
    let path = directory.join("state.json");
    workspace.update_in(cx, |w, _, _| w.persistence_test_path = Some(path.clone()));
    let id = started(&workspace, cx, 1).await;
    click(&workspace, cx, "desktop-run-workflow-step").await;
    let saved_bytes = std::fs::read(&path).unwrap();
    let preview = ok(call(&workspace, cx, request(&id, true)).await);
    assert_eq!(preview["eligible"], true);
    assert!(preview["rollback_review"].is_null());
    assert_eq!(preview["rolled_back"], false);
    let requested = ok(call(&workspace, cx, request(&id, false)).await);
    assert_eq!(requested["rollback_review"], "pending");
    assert_eq!(requested["requires_user_approval"], true);
    assert_eq!(std::fs::read(&path).unwrap(), saved_bytes);
    workspace.update_in(cx, |w, _, _| {
        assert_eq!(w.work_items.len(), 1);
        assert_eq!(
            w.automation_state.workflows[0].status,
            WorkflowStatus::Completed
        );
    });
    assert_eq!(
        ok(call(&workspace, cx, request(&id, false)).await),
        requested
    );
    click(&workspace, cx, "desktop-undo-workflow").await;
    let saved = load_desktop_state_from_path(&path).unwrap().unwrap();
    assert!(saved.work_items.is_empty());
    assert_eq!(
        saved.automation_state.workflows[0].status,
        WorkflowStatus::RolledBack
    );
    let status = ok(call(
        &workspace,
        cx,
        BridgeRequest::Status {
            workflow_id: id.clone(),
        },
    )
    .await);
    assert_eq!(status["rolled_back"], true);
    assert_eq!(status["requires_user_approval"], false);
    assert_eq!(status["progress"]["durable_reverted_steps"], 1);
    for save_status in [
        SaveStatus::Saving,
        SaveStatus::Failed("disk full".into()),
        SaveStatus::Conflict,
    ] {
        workspace.update_in(cx, |w, _, _| w.save_status = save_status);
        let status = ok(call(
            &workspace,
            cx,
            BridgeRequest::Status {
                workflow_id: id.clone(),
            },
        )
        .await);
        assert_eq!(status["rolled_back"], false);
        assert!(status["progress"]["durable_reverted_steps"].is_null());
    }
    workspace.update_in(cx, |w, _, _| w.save_status = SaveStatus::Saved);
    let repeated = ok(call(&workspace, cx, request(&id, false)).await);
    assert_eq!(repeated["rolled_back"], true);
    assert_eq!(repeated["requires_user_approval"], false);
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
    std::fs::remove_dir_all(directory).unwrap();
}

#[gpui::test]
async fn declined_undo_cannot_be_resurfaced_by_repeating_the_request(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    let id = started(&workspace, cx, 1).await;
    click(&workspace, cx, "desktop-run-workflow-step").await;
    ok(call(&workspace, cx, request(&id, false)).await);
    click(&workspace, cx, "desktop-decline-mcp-rollback").await;
    for _ in 0..3 {
        let repeated = ok(call(&workspace, cx, request(&id, false)).await);
        assert_eq!(repeated["rollback_review"], "declined");
        assert_eq!(repeated["requires_user_approval"], false);
        assert_eq!(repeated["rolled_back"], false);
    }
    workspace.update_in(cx, |w, _, _| {
        assert_eq!(w.work_items.len(), 1);
        assert_eq!(
            w.mcp_bridge.rollback_reviews.get(&id),
            Some(&RollbackReview::Declined)
        );
        assert!(!w.mcp_bridge.pending_review(&w.automation_state));
    });
    let next = ok(call(
        &workspace,
        cx,
        BridgeRequest::Propose {
            request_id: "next-proposal".into(),
            operation: WorkflowOperation::CreateTask {
                title: "Next plan".into(),
            },
        },
    )
    .await);
    ok(call(&workspace, cx, request(&id, false)).await);
    workspace.update_in(cx, |w, _, _| {
        assert_eq!(
            w.reviewed_workflow_id.as_deref(),
            next["workflow_id"].as_str()
        );
        assert_eq!(w.work_items.len(), 1);
        w.mcp_bridge = WorkspaceBridge::default();
    });
}

#[gpui::test]
async fn reconnect_drops_undo_requests_without_deleting_saved_records(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    let id = started(&workspace, cx, 1).await;
    click(&workspace, cx, "desktop-run-workflow-step").await;
    ok(call(&workspace, cx, request(&id, false)).await);
    workspace.update_in(cx, |w, _, cx| {
        w.mcp_bridge = WorkspaceBridge::default();
        cx.notify();
    });
    connect(&workspace, cx).await;
    assert!(matches!(
        call(&workspace, cx, request(&id, false)).await,
        BridgeResponse::Error(_)
    ));
    workspace.update_in(cx, |w, _, _| {
        assert!(w.mcp_bridge.rollback_reviews.is_empty());
        assert_eq!(w.work_items.len(), 1);
        w.mcp_bridge = WorkspaceBridge::default();
    });
}
