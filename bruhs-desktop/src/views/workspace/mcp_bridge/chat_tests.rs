use super::super::{
    desktop_state_backup_path, load_desktop_state_from_path, AppView, SaveStatus, Workspace,
};
use super::tests::{call, connect, fixture};
use super::WorkspaceBridge;
use bruhs_core::workspace_bridge::{BridgeRequest, BridgeResponse, WorkflowProposal};
use bruhs_models::{AgentAccess, AgentDomain, WorkflowOperation};
use gpui::{Entity, TestAppContext, VisualTestContext};

mod safety;

struct Files(std::path::PathBuf);
impl Files {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("bruhs-chat-workflow-{}", uuid::Uuid::new_v4())))
    }
    fn path(&self) -> std::path::PathBuf {
        self.0.join("state.json")
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn click(workspace: &Entity<Workspace>, cx: &mut VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("Missing {selector}"));
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    cx.condition(workspace, |w, _| {
        !w.automation_pending && w.save_status.is_saved()
    })
    .await;
    cx.run_until_parked();
}

async fn started(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
    files: &Files,
) -> String {
    workspace.update_in(cx, |w, _, _| {
        w.persistence_test_path = Some(files.path());
        w.automation_state
            .permissions
            .set(AgentDomain::Chat, AgentAccess::Read, false);
    });
    connect(workspace, cx).await;
    let proposal = WorkflowProposal {
        title: "Chat and task".into(),
        operations: vec![
            WorkflowOperation::CreateChatThread {
                title: "Reviewed native chat 🗒".into(),
            },
            WorkflowOperation::CreateTask {
                title: "Reviewed task".into(),
            },
        ],
    };
    let request = BridgeRequest::ProposeWorkflow {
        request_id: "native-chat".into(),
        proposal,
    };
    assert!(matches!(
        call(workspace, cx, request.clone()).await,
        BridgeResponse::Error(_)
    ));
    workspace.update_in(cx, |w, _, _| {
        assert!(w.automation_state.workflows.is_empty());
        for domain in [AgentDomain::Chat, AgentDomain::Tasks] {
            w.automation_state
                .permissions
                .set(domain, AgentAccess::Write, true);
        }
    });
    let BridgeResponse::Ok(response) = call(workspace, cx, request).await else {
        panic!("chat proposal adapter")
    };
    assert_eq!(response["executed"], false);
    assert_eq!(response["requires_user_approval"], true);
    cx.condition(workspace, |w, _| w.save_status.is_saved())
        .await;
    click(workspace, cx, "desktop-start-workflow").await;
    response["workflow_id"]
        .as_str()
        .expect("accepted proposal has a workflow ID")
        .into()
}

#[gpui::test]
async fn reviewed_chat_and_checkpoint_save_together_and_user_approved_undo_removes_only_created_records(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let files = Files::new();
    let original = workspace.update_in(cx, |w, _, _| {
        w.threads
            .iter()
            .map(|thread| thread.id.clone())
            .collect::<Vec<_>>()
    });
    let id = started(&workspace, cx, &files).await;
    click(&workspace, cx, "desktop-run-workflow-step").await;
    let first = load_desktop_state_from_path(&files.path())
        .unwrap()
        .unwrap();
    assert_eq!(first.automation_state.jobs[0].next_step, 1);
    let step = &first.automation_state.workflows[0].steps[0];
    let thread = first
        .threads
        .iter()
        .find(|thread| Some(&thread.id) == step.target_id.as_ref())
        .unwrap();
    assert_eq!(thread.title, "Reviewed native chat 🗒");
    assert!(thread.messages.is_empty());
    assert!(thread.project_name.is_empty());
    assert_eq!(
        step.target_snapshot.as_deref(),
        Some(serde_json::to_string(thread).unwrap().as_str())
    );
    assert!(!first
        .automation_state
        .permissions
        .allows(AgentDomain::Chat, AgentAccess::Read));
    // Restore through the same thread constructor used at startup, retaining
    // the saved workflow rather than proposing or replaying its first step.
    workspace.update_in(cx, |w, window, cx| {
        w.threads = first
            .threads
            .iter()
            .map(|thread| {
                Workspace::create_thread_from_persisted(
                    thread,
                    w.current_model.clone(),
                    w.permission_mode,
                    window,
                    cx,
                )
            })
            .collect();
        w.automation_state = first.automation_state.clone();
        cx.notify();
    });
    click(&workspace, cx, "desktop-run-workflow-step").await;
    let before = std::fs::read(files.path()).unwrap();
    let BridgeResponse::Ok(review) = call(
        &workspace,
        cx,
        BridgeRequest::RequestRollback {
            workflow_id: id.clone(),
            dry_run: false,
        },
    )
    .await
    else {
        panic!("chat undo review")
    };
    assert_eq!(review["completed_steps"], 2);
    assert_eq!(std::fs::read(files.path()).unwrap(), before);
    click(&workspace, cx, "desktop-undo-workflow").await;
    let saved = load_desktop_state_from_path(&files.path())
        .unwrap()
        .unwrap();
    assert_eq!(
        saved
            .threads
            .iter()
            .map(|thread| thread.id.clone())
            .collect::<Vec<_>>(),
        original
    );
    assert!(saved.work_items.is_empty());
    assert_eq!(
        saved.automation_state.workflows[0].status,
        bruhs_models::WorkflowStatus::RolledBack
    );
    let BridgeResponse::Ok(status) =
        call(&workspace, cx, BridgeRequest::Status { workflow_id: id }).await
    else {
        panic!("status")
    };
    assert_eq!(status["progress"]["durable_reverted_steps"], 2);
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}

#[gpui::test]
async fn failed_chat_save_stays_provisional_and_retry_does_not_create_another_thread(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let files = Files::new();
    started(&workspace, cx, &files).await;
    let before = std::fs::read(files.path()).unwrap();
    let count = workspace.update_in(cx, |w, _, _| w.threads.len());
    let backup = desktop_state_backup_path(&files.path());
    // Remove the fixture's previous backup so a directory can simulate failure.
    if backup.is_file() {
        std::fs::remove_file(&backup).unwrap();
    }
    std::fs::create_dir(&backup).unwrap();
    workspace.update_in(cx, |w, _, cx| {
        w.run_latest_automation_step(cx);
        w.run_latest_automation_step(cx);
        assert!(w.automation_pending);
    });
    cx.condition(&workspace, |w, _| {
        matches!(w.save_status, SaveStatus::Failed(_))
    })
    .await;
    workspace.update_in(cx, |w, _, cx| {
        assert_eq!(w.threads.len(), count + 1);
        assert_eq!(w.automation_state.jobs[0].next_step, 1);
        w.run_latest_automation_step(cx);
        assert!(w.work_items.is_empty());
    });
    assert_eq!(std::fs::read(files.path()).unwrap(), before);
    std::fs::remove_dir(&backup).unwrap();
    click(&workspace, cx, "desktop-retry-save").await;
    let saved = load_desktop_state_from_path(&files.path())
        .unwrap()
        .unwrap();
    assert_eq!(saved.threads.len(), count + 1);
    assert_eq!(saved.automation_state.jobs[0].next_step, 1);
    click(&workspace, cx, "desktop-run-workflow-step").await;
    assert_eq!(
        load_desktop_state_from_path(&files.path())
            .unwrap()
            .unwrap()
            .automation_state
            .jobs[0]
            .next_step,
        2
    );
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}

#[gpui::test]
async fn opted_in_chat_workflow_completes_after_route_change(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    let files = Files::new();
    started(&workspace, cx, &files).await;
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds("desktop-enable-workflow-background")
        .unwrap();
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    workspace.update_in(cx, |w, _, cx| {
        w.active_view = AppView::Today;
        cx.notify();
    });
    cx.condition(&workspace, |w, _| {
        w.save_status.is_saved() && w.automation_state.jobs[0].next_step == 2
    })
    .await;
    let saved = load_desktop_state_from_path(&files.path())
        .unwrap()
        .unwrap();
    assert_eq!(
        saved
            .threads
            .iter()
            .filter(|thread| thread.title == "Reviewed native chat 🗒")
            .count(),
        1
    );
    assert_eq!(saved.work_items.len(), 1);
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}
