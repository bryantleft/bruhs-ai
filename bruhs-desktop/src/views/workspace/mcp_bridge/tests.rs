use super::super::{
    load_desktop_state_from_path, AppView, AutomationState, DesktopState, SaveStatus, Workspace,
};
use super::{ConnectionState, WorkspaceBridge};
use bruhs_core::workspace_bridge::{BridgeClient, BridgeRequest, BridgeResponse};
use bruhs_models::{AgentAccess, AgentDomain, WorkflowOperation};
use gpui::{AppContext, Entity, TestAppContext, VisualTestContext};
use std::{cell::RefCell, rc::Rc};

pub(super) fn fixture(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    // This fixture crosses a real Tokio Unix socket, not the deterministic
    // test executor's virtual clock.
    cx.executor().allow_parking();
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().expect("desktop state"));
    });
    let cell = Rc::new(RefCell::new(None));
    let capture = cell.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *capture.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = cell.borrow().clone().expect("workspace entity");
    workspace.update_in(cx, |workspace, _, cx| {
        workspace.work_items.clear();
        workspace.calendar_events.clear();
        workspace.captures.clear();
        workspace.knowledge_documents.clear();
        workspace.inbox_conversations.clear();
        workspace.automation_state = AutomationState::default();
        workspace.active_view = AppView::Automations;
        cx.notify();
    });
    (workspace, cx)
}

pub(super) async fn call(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
    request: BridgeRequest,
) -> BridgeResponse {
    let receiver = workspace.update_in(cx, |workspace, _, cx| {
        let ConnectionState::Connected { session, .. } = &workspace.mcp_bridge.state else {
            panic!("bridge connected")
        };
        let client =
            BridgeClient::from_file(session.connection_path()).expect("private connection file");
        let (sender, receiver) = tokio::sync::oneshot::channel();
        cx.global::<DesktopState>().runtime.spawn(async move {
            let _ = sender.send(client.call(request).await.expect("workspace IPC response"));
        });
        receiver
    });
    receiver.await.expect("IPC result channel")
}

pub(super) async fn connect(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds("desktop-mcp-connection-toggle")
        .expect("connection button");
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    cx.condition(workspace, |w, _| {
        matches!(w.mcp_bridge.state, ConnectionState::Connected { .. })
    })
    .await;
}

#[gpui::test]
async fn real_bridge_reads_current_workspace_and_rechecks_revocation(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, _| {
        assert!(matches!(w.mcp_bridge.state, ConnectionState::Disabled))
    });
    connect(&workspace, cx).await;
    let request = BridgeRequest::Read {
        domain: AgentDomain::Tasks,
        query: String::new(),
        offset: 0,
    };
    assert!(
        matches!(call(&workspace, cx, request.clone()).await, BridgeResponse::Ok(value) if value["records"] == serde_json::json!([]))
    );
    workspace.update_in(cx, |w, _, _| {
        w.automation_state
            .permissions
            .set(AgentDomain::Tasks, AgentAccess::Read, false)
    });
    assert!(matches!(
        call(&workspace, cx, request).await,
        BridgeResponse::Error(_)
    ));
    assert!(matches!(
        call(
            &workspace,
            cx,
            BridgeRequest::Read {
                domain: AgentDomain::Finance,
                query: String::new(),
                offset: 0
            }
        )
        .await,
        BridgeResponse::Error(_)
    ));
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}

#[gpui::test]
async fn real_bridge_proposals_require_grants_and_never_execute_themselves(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    connect(&workspace, cx).await;
    let operation = WorkflowOperation::CreateTask {
        title: "Actual MCP proposal".into(),
    };
    let proposal = BridgeRequest::Propose {
        request_id: "request-one".into(),
        operation: operation.clone(),
    };
    assert!(matches!(
        call(&workspace, cx, proposal.clone()).await,
        BridgeResponse::Error(_)
    ));
    workspace.update_in(cx, |w, _, _| {
        w.automation_state
            .permissions
            .set(AgentDomain::Tasks, AgentAccess::Write, true)
    });
    let response = call(&workspace, cx, proposal.clone()).await;
    let BridgeResponse::Ok(first) = response else {
        panic!("proposal should be accepted")
    };
    assert_eq!(first["executed"], false);
    assert_eq!(first["requires_user_approval"], true);
    let BridgeResponse::Ok(retry) = call(&workspace, cx, proposal).await else {
        panic!("retry")
    };
    assert_eq!(first["workflow_id"], retry["workflow_id"]);
    assert!(matches!(
        call(
            &workspace,
            cx,
            BridgeRequest::Propose {
                request_id: "second-request".into(),
                operation: operation.clone(),
            }
        )
        .await,
        BridgeResponse::Error(_)
    ));
    assert!(matches!(
        call(
            &workspace,
            cx,
            BridgeRequest::Propose {
                request_id: "request-one".into(),
                operation: WorkflowOperation::CreateTask {
                    title: "Changed payload".into()
                }
            }
        )
        .await,
        BridgeResponse::Error(_)
    ));
    workspace.update_in(cx, |w, _, _| {
        assert_eq!(w.automation_state.workflows.len(), 1);
        assert!(w.work_items.is_empty());
        assert!(w.automation_state.jobs.is_empty());
        w.mcp_bridge = WorkspaceBridge::default();
    });
}

#[gpui::test]
async fn real_bridge_decline_is_user_controlled_and_terminal(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, _| {
        w.automation_state
            .permissions
            .set(AgentDomain::Tasks, AgentAccess::Write, true)
    });
    connect(&workspace, cx).await;
    let proposal = BridgeRequest::Propose {
        request_id: "decline-request".into(),
        operation: WorkflowOperation::CreateTask {
            title: "Do not create".into(),
        },
    };
    assert!(matches!(
        call(&workspace, cx, proposal.clone()).await,
        BridgeResponse::Ok(_)
    ));
    cx.run_until_parked();
    let decline = cx.debug_bounds("desktop-decline-workflow").unwrap();
    cx.simulate_click(decline.center(), gpui::Modifiers::default());
    let BridgeResponse::Ok(retry) = call(&workspace, cx, proposal).await else {
        panic!("same proposal status")
    };
    assert_eq!(retry["workflow_status"], "declined");
    assert_eq!(retry["executed"], false);
    assert_eq!(retry["requires_user_approval"], false);
    assert!(matches!(
        call(
            &workspace,
            cx,
            BridgeRequest::Propose {
                request_id: "after-decline".into(),
                operation: WorkflowOperation::CreateTask {
                    title: "New review".into()
                },
            }
        )
        .await,
        BridgeResponse::Ok(_)
    ));
    workspace.update_in(cx, |w, _, _| {
        assert_eq!(w.automation_state.workflows.len(), 2);
        assert!(w.work_items.is_empty());
        assert!(w.automation_state.jobs.is_empty());
        w.mcp_bridge = WorkspaceBridge::default();
    });
}

#[gpui::test]
async fn real_bridge_never_reports_unsaved_completion(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, _| {
        w.automation_state
            .permissions
            .set(AgentDomain::Tasks, AgentAccess::Write, true)
    });
    connect(&workspace, cx).await;
    let BridgeResponse::Ok(proposal) = call(
        &workspace,
        cx,
        BridgeRequest::Propose {
            request_id: "save-status".into(),
            operation: WorkflowOperation::CreateTask {
                title: "Provisional".into(),
            },
        },
    )
    .await
    else {
        panic!("proposal")
    };
    // Inject each acknowledgment state, independently of the file-writer tests.
    // Even an in-memory completed workflow is not durable completion.
    for status in [
        SaveStatus::Saving,
        SaveStatus::Failed("test failure".into()),
        SaveStatus::Conflict,
        SaveStatus::RecoveryRequired("test recovery".into()),
    ] {
        workspace.update_in(cx, |w, _, _| {
            w.automation_state.workflows[0].status = bruhs_models::WorkflowStatus::Completed;
            w.save_status = status;
        });
        let BridgeResponse::Ok(status) = call(
            &workspace,
            cx,
            BridgeRequest::Status {
                workflow_id: proposal["workflow_id"].as_str().unwrap().into(),
            },
        )
        .await
        else {
            panic!("status")
        };
        assert_eq!(status["executed"], false);
        assert_eq!(status["durable"], false);
        assert!(status["progress"]["durable_completed_steps"].is_null());
        assert!(status["progress"]["durable_reverted_steps"].is_null());
        assert!(matches!(
            call(
                &workspace,
                cx,
                BridgeRequest::Propose {
                    request_id: "cannot-submit".into(),
                    operation: WorkflowOperation::CreateTask {
                        title: "Blocked".into()
                    },
                }
            )
            .await,
            BridgeResponse::Error(_)
        ));
    }
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}

#[gpui::test]
async fn real_bridge_completion_requires_user_approval_and_durable_save(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    let directory =
        std::env::temp_dir().join(format!("bruhs-mcp-durability-{}", uuid::Uuid::new_v4()));
    let path = directory.join("state.json");
    workspace.update_in(cx, |w, _, _| {
        w.persistence_test_path = Some(path.clone());
        w.automation_state
            .permissions
            .set(AgentDomain::Tasks, AgentAccess::Write, true);
    });
    connect(&workspace, cx).await;
    let BridgeResponse::Ok(proposal) = call(
        &workspace,
        cx,
        BridgeRequest::Propose {
            request_id: "durable-request".into(),
            operation: WorkflowOperation::CreateTask {
                title: "MCP durable task".into(),
            },
        },
    )
    .await
    else {
        panic!("proposal")
    };
    assert_eq!(proposal["executed"], false);
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let saved = load_desktop_state_from_path(&path).unwrap().unwrap();
    assert_eq!(saved.automation_state.workflows.len(), 1);
    assert!(saved.work_items.is_empty());
    cx.run_until_parked();
    let start = cx.debug_bounds("desktop-start-workflow").unwrap();
    cx.simulate_click(start.center(), gpui::Modifiers::default());
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    cx.run_until_parked();
    let run = cx.debug_bounds("desktop-run-workflow-step").unwrap();
    cx.simulate_click(run.center(), gpui::Modifiers::default());
    workspace.update_in(cx, |w, _, _| assert_eq!(w.save_status, SaveStatus::Saving));
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let response = call(
        &workspace,
        cx,
        BridgeRequest::Status {
            workflow_id: proposal["workflow_id"].as_str().unwrap().into(),
        },
    )
    .await;
    assert!(
        matches!(response, BridgeResponse::Ok(value) if value["executed"] == true && value["durable"] == true)
    );
    let saved = load_desktop_state_from_path(&path).unwrap().unwrap();
    assert_eq!(saved.work_items[0].title, "MCP durable task");
    let read = call(
        &workspace,
        cx,
        BridgeRequest::Read {
            domain: AgentDomain::Tasks,
            query: "MCP durable".into(),
            offset: 0,
        },
    )
    .await;
    assert!(
        matches!(read, BridgeResponse::Ok(value) if value["records"][0]["title"] == "MCP durable task")
    );
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
    std::fs::remove_dir_all(directory).unwrap();
}
