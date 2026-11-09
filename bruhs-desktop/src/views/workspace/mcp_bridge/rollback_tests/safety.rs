use super::{
    call, click, fixture, ok, request, started, AgentAccess, AgentDomain, BridgeRequest,
    BridgeResponse, TestAppContext, WorkflowOperation, WorkspaceBridge,
};

#[gpui::test]
async fn unsaved_checkpoints_never_create_an_undo_review(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    let id = started(&workspace, cx, 1).await;
    click(&workspace, cx, "desktop-run-workflow-step").await;
    for status in [
        super::SaveStatus::Saving,
        super::SaveStatus::Failed("disk full".into()),
        super::SaveStatus::Conflict,
        super::SaveStatus::RecoveryRequired("recovery needed".into()),
    ] {
        workspace.update_in(cx, |w, _, _| w.save_status = status);
        assert!(matches!(
            call(&workspace, cx, request(&id, false)).await,
            BridgeResponse::Error(_)
        ));
        workspace.update_in(cx, |w, _, _| {
            assert!(w.mcp_bridge.rollback_reviews.is_empty());
            assert_eq!(w.work_items.len(), 1);
        });
    }
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}

#[gpui::test]
async fn revoked_or_edited_targets_block_requested_and_user_approved_undo(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    let id = started(&workspace, cx, 1).await;
    click(&workspace, cx, "desktop-run-workflow-step").await;
    workspace.update_in(cx, |w, _, _| {
        w.automation_state
            .permissions
            .set(AgentDomain::Tasks, AgentAccess::Write, false)
    });
    assert!(matches!(
        call(&workspace, cx, request(&id, false)).await,
        BridgeResponse::Error(_)
    ));
    workspace.update_in(cx, |w, _, _| {
        assert!(w.mcp_bridge.rollback_reviews.is_empty());
        w.automation_state
            .permissions
            .set(AgentDomain::Tasks, AgentAccess::Write, true);
    });
    ok(call(&workspace, cx, request(&id, false)).await);
    workspace.update_in(cx, |w, _, _| {
        w.automation_state
            .permissions
            .set(AgentDomain::Tasks, AgentAccess::Write, false)
    });
    click(&workspace, cx, "desktop-undo-workflow").await;
    workspace.update_in(cx, |w, _, _| {
        assert_eq!(w.work_items.len(), 1);
        assert!(w.automation_error.is_some());
        w.automation_state
            .permissions
            .set(AgentDomain::Tasks, AgentAccess::Write, true);
        w.work_items[0].title = "User edit after the request".into();
    });
    assert!(matches!(
        call(&workspace, cx, request(&id, true)).await,
        BridgeResponse::Error(_)
    ));
    click(&workspace, cx, "desktop-undo-workflow").await;
    workspace.update_in(cx, |w, _, _| {
        assert_eq!(w.work_items[0].title, "User edit after the request");
        assert!(w.automation_error.is_some());
        w.mcp_bridge = WorkspaceBridge::default();
    });
}

#[gpui::test]
async fn pending_undo_cannot_be_displaced_by_a_new_mcp_proposal(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    let id = started(&workspace, cx, 1).await;
    click(&workspace, cx, "desktop-run-workflow-step").await;
    ok(call(&workspace, cx, request(&id, false)).await);
    assert!(matches!(
        call(
            &workspace,
            cx,
            BridgeRequest::Propose {
                request_id: "displace-review".into(),
                operation: WorkflowOperation::CreateTask {
                    title: "Different plan".into()
                },
            }
        )
        .await,
        BridgeResponse::Error(_)
    ));
    workspace.update_in(cx, |w, _, _| {
        assert_eq!(w.automation_state.workflows.len(), 1);
        assert_eq!(w.reviewed_workflow_id.as_deref(), Some(id.as_str()));
        w.mcp_bridge = WorkspaceBridge::default();
    });
}

#[gpui::test]
async fn agent_cannot_pause_a_running_background_job_to_authorize_undo(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    let id = started(&workspace, cx, 2).await;
    click(&workspace, cx, "desktop-run-workflow-step").await;
    workspace.update_in(cx, |w, _, _| {
        w.automation_state.jobs[0].auto_run = true;
    });
    assert!(matches!(
        call(&workspace, cx, request(&id, false)).await,
        BridgeResponse::Error(_)
    ));
    click(&workspace, cx, "desktop-undo-workflow").await;
    workspace.update_in(cx, |w, _, _| {
        assert_eq!(w.work_items.len(), 1);
        assert_eq!(
            w.automation_state.jobs[0].status,
            bruhs_models::BackgroundJobStatus::Running
        );
        assert!(w.mcp_bridge.rollback_reviews.is_empty());
    });
    click(&workspace, cx, "desktop-pause-workflow").await;
    ok(call(&workspace, cx, request(&id, false)).await);
    click(&workspace, cx, "desktop-undo-workflow").await;
    workspace.update_in(cx, |w, _, _| {
        assert!(w.work_items.is_empty());
        assert_eq!(
            w.automation_state.jobs[0].status,
            bruhs_models::BackgroundJobStatus::RolledBack
        );
        w.mcp_bridge = WorkspaceBridge::default();
    });
}
