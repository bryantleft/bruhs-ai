use super::super::AppView;
use super::tests::{call, connect, fixture};
use super::WorkspaceBridge;
use bruhs_core::workspace_bridge::{BridgeRequest, BridgeResponse};
use bruhs_models::{AgentAccess, AgentDomain, WorkflowOperation};
use gpui::TestAppContext;

#[gpui::test]
async fn pending_proposal_status_opens_that_review_after_history_navigation(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let earlier = workspace.update_in(cx, |w, _, _| {
        w.automation_state
            .permissions
            .set(AgentDomain::Tasks, AgentAccess::Write, true);
        let id = w.automation_state.create_workflow(
            "Older declined plan",
            vec![WorkflowOperation::CreateTask {
                title: "Old".into(),
            }],
            1,
        );
        assert!(w.automation_state.decline_workflow(&id, 2));
        id
    });
    connect(&workspace, cx).await;
    let BridgeResponse::Ok(proposal) = call(
        &workspace,
        cx,
        BridgeRequest::Propose {
            request_id: "pending-history".into(),
            operation: WorkflowOperation::CreateTask {
                title: "Pending MCP plan".into(),
            },
        },
    )
    .await
    else {
        panic!("Proposal accepted");
    };
    cx.run_until_parked();
    let previous = cx.debug_bounds("desktop-previous-workflow").unwrap();
    cx.simulate_click(previous.center(), gpui::Modifiers::default());
    workspace.update_in(cx, |w, _, cx| {
        assert_eq!(w.reviewed_workflow_id.as_deref(), Some(earlier.as_str()));
        w.active_view = AppView::Tasks;
        cx.notify();
    });
    cx.run_until_parked();
    let open = cx.debug_bounds("desktop-open-workflows").unwrap();
    cx.simulate_click(open.center(), gpui::Modifiers::default());
    workspace.update_in(cx, |w, _, _| {
        assert_eq!(
            w.reviewed_workflow_id.as_deref(),
            proposal["workflow_id"].as_str()
        );
        assert_eq!(w.active_view, AppView::Automations);
        assert!(w.work_items.is_empty());
        assert!(w.automation_state.jobs.is_empty());
        w.mcp_bridge = WorkspaceBridge::default();
    });
}
