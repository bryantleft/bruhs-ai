use super::super::{load_desktop_state_from_path, AppView, Workspace};
use super::tests::{call, connect, fixture};
use super::WorkspaceBridge;
use bruhs_core::workspace_bridge::{BridgeRequest, BridgeResponse, WorkflowProposal};
use bruhs_models::{AgentAccess, AgentDomain, WorkflowOperation};
use gpui::{Entity, TestAppContext, VisualTestContext};

struct Files(std::path::PathBuf);
impl Files {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("bruhs-meeting-note-{}", uuid::Uuid::new_v4())))
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

async fn completed(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
    files: &Files,
) -> String {
    workspace.update_in(cx, |w, _, _| {
        w.persistence_test_path = Some(files.path());
        w.meeting_notes.clear();
    });
    connect(workspace, cx).await;
    let request = BridgeRequest::ProposeWorkflow {
        request_id: "meeting-note-workflow".into(),
        proposal: WorkflowProposal {
            title: "Meeting note and knowledge".into(),
            operations: vec![
                WorkflowOperation::CreateMeetingNote {
                    title: "Reviewed meeting 🗒".into(),
                    body: "  Exact notes\n\nNext action: review.  ".into(),
                },
                WorkflowOperation::CreateKnowledgeNote {
                    title: "Related document".into(),
                    body: "Keep if note changes".into(),
                },
            ],
        },
    };
    assert!(matches!(
        call(workspace, cx, request.clone()).await,
        BridgeResponse::Error(_)
    ));
    workspace.update_in(cx, |w, _, _| {
        assert!(w.automation_state.workflows.is_empty());
        for domain in [AgentDomain::Meetings, AgentDomain::Knowledge] {
            w.automation_state
                .permissions
                .set(domain, AgentAccess::Write, true);
            w.automation_state
                .permissions
                .set(domain, AgentAccess::Read, false);
        }
    });
    let BridgeResponse::Ok(response) = call(workspace, cx, request.clone()).await else {
        panic!("proposal")
    };
    assert_eq!(response["executed"], false);
    cx.condition(workspace, |w, _| w.save_status.is_saved())
        .await;
    click(workspace, cx, "desktop-start-workflow").await;
    click(workspace, cx, "desktop-run-workflow-step").await;
    click(workspace, cx, "desktop-run-workflow-step").await;
    assert!(matches!(
        call(workspace, cx, request).await,
        BridgeResponse::Ok(_)
    ));
    workspace.update_in(cx, |w, _, _| assert_eq!(w.meeting_notes.len(), 1));
    response["workflow_id"]
        .as_str()
        .expect("Accepted proposal has a workflow ID")
        .into()
}

#[gpui::test]
async fn meeting_note_exact_payload_survives_file_restore_renders_and_undo_is_durable(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let files = Files::new();
    let id = completed(&workspace, cx, &files).await;
    let saved = load_desktop_state_from_path(&files.path())
        .unwrap()
        .unwrap();
    let note = saved.meeting_notes[0].clone();
    assert_eq!(note.body, "  Exact notes\n\nNext action: review.  ");
    assert_eq!(saved.automation_state.jobs[0].next_step, 2);
    assert_eq!(
        saved.automation_state.workflows[0].steps[0]
            .target_snapshot
            .as_deref(),
        Some(serde_json::to_string(&note).unwrap().as_str())
    );
    workspace.update_in(cx, |w, _, cx| {
        w.meeting_notes = saved.meeting_notes.clone();
        w.automation_state = saved.automation_state.clone();
        w.active_view = AppView::Meetings;
        cx.notify();
    });
    click(
        &workspace,
        cx,
        Box::leak(
            format!(
                "desktop-meeting-note-{}",
                super::super::element_key(&note.id)
            )
            .into_boxed_str(),
        ),
    )
    .await;
    assert!(cx.debug_bounds("desktop-meeting-note-content").is_some());
    workspace.update_in(cx, |w, _, cx| {
        assert_eq!(
            w.selected_meeting_note_id.as_deref(),
            Some(note.id.as_str())
        );
        w.active_view = AppView::Automations;
        cx.notify();
    });
    let before = std::fs::read(files.path()).unwrap();
    assert!(matches!(
        call(
            &workspace,
            cx,
            BridgeRequest::RequestRollback {
                workflow_id: id,
                dry_run: false
            }
        )
        .await,
        BridgeResponse::Ok(_)
    ));
    assert_eq!(std::fs::read(files.path()).unwrap(), before);
    click(&workspace, cx, "desktop-undo-workflow").await;
    let saved = load_desktop_state_from_path(&files.path())
        .unwrap()
        .unwrap();
    assert!(saved.meeting_notes.is_empty());
    assert!(saved.knowledge_documents.is_empty());
    assert_eq!(
        saved.automation_state.workflows[0].status,
        bruhs_models::WorkflowStatus::RolledBack
    );
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}

#[gpui::test]
async fn meeting_note_read_is_independent_and_changed_or_revoked_undo_preserves_all_domains(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let files = Files::new();
    let id = completed(&workspace, cx, &files).await;
    let note = workspace.update_in(cx, |w, _, _| w.meeting_notes[0].clone());
    let read = BridgeRequest::ReadMeetingNote {
        note_id: note.id.clone(),
    };
    assert!(matches!(
        call(&workspace, cx, read.clone()).await,
        BridgeResponse::Error(_)
    ));
    workspace.update_in(cx, |w, _, _| {
        w.automation_state
            .permissions
            .set(AgentDomain::Meetings, AgentAccess::Read, true)
    });
    let BridgeResponse::Ok(value) = call(&workspace, cx, read.clone()).await else {
        panic!("read")
    };
    assert_eq!(value["records"][0]["body"], note.body);
    let BridgeResponse::Ok(value) = call(
        &workspace,
        cx,
        BridgeRequest::ReadMeetingNotes {
            query: "Exact notes".into(),
            offset: 0,
        },
    )
    .await
    else {
        panic!("list")
    };
    assert!(value["records"].as_array().unwrap().is_empty());
    workspace.update_in(cx, |w, _, cx| {
        w.meeting_notes[0].body.push_str("Edited");
        assert!(w.automation_rollback_targets(&id, cx).is_err());
        assert_eq!(w.knowledge_documents.len(), 1);
        w.meeting_notes[0] = note.clone();
        w.meeting_notes.push(note.clone());
        assert!(w.automation_rollback_targets(&id, cx).is_err());
        w.meeting_notes.pop();
        w.automation_state
            .permissions
            .set(AgentDomain::Meetings, AgentAccess::Write, false);
        assert!(w.automation_rollback_targets(&id, cx).is_err());
        w.automation_state
            .permissions
            .set(AgentDomain::Meetings, AgentAccess::Read, false);
    });
    assert!(matches!(
        call(&workspace, cx, read).await,
        BridgeResponse::Error(_)
    ));
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}
