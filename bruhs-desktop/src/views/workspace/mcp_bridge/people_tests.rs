use super::super::{
    desktop_state_backup_path, load_desktop_state_from_path, AppView, SaveStatus, Workspace,
};
use super::tests::{call, connect, fixture};
use super::WorkspaceBridge;
use bruhs_core::workspace_bridge::{BridgeRequest, BridgeResponse, WorkflowProposal};
use bruhs_models::{
    AgentAccess, AgentDomain, PersonProfile, RelationshipItem, RelationshipItemKind,
    RelationshipProfile, WorkflowOperation, WorkflowStatus,
};
use gpui::{Entity, TestAppContext, VisualTestContext};

struct Files(std::path::PathBuf);
impl Files {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("bruhs-person-reminder-{}", uuid::Uuid::new_v4())))
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
    cx.condition(workspace, |w, _| w.save_status.is_saved())
        .await;
    cx.run_until_parked();
}

fn original() -> RelationshipProfile {
    let mut profile = RelationshipProfile::new("person-1", 1);
    profile.notes = "PRIVATE prior relationship context".into();
    profile.add_item(
        RelationshipItem::new(
            RelationshipItemKind::Commitment,
            "Keep existing commitment",
            2,
        ),
        2,
    );
    profile.items[0].id = "existing-commitment".into();
    profile
}

async fn started(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
    files: &Files,
) -> String {
    workspace.update_in(cx, |w, _, cx| {
        w.persistence_test_path = Some(files.path());
        w.relationships = vec![original()];
        w.meetings_hub.update(cx, |hub, _| {
            hub.people = ["person-1", "same-name-other-id"]
                .into_iter()
                .map(|id| {
                    let mut person = PersonProfile::new("Alex");
                    person.id = id.into();
                    person
                })
                .collect();
        });
        w.automation_state
            .permissions
            .set(AgentDomain::People, AgentAccess::Read, false);
    });
    connect(workspace, cx).await;
    let reminder = WorkflowOperation::CreatePersonReminder {
        person_id: "person-1".into(),
        person: "Alex".into(),
        reminder: "Send the reviewed brief 🗒".into(),
    };
    let request = BridgeRequest::ProposeWorkflow {
        request_id: "native-person-reminder".into(),
        proposal: WorkflowProposal {
            title: "People and task".into(),
            operations: vec![
                reminder.clone(),
                WorkflowOperation::CreateTask {
                    title: "Mixed task".into(),
                },
                reminder,
            ],
        },
    };
    assert!(matches!(
        call(workspace, cx, request.clone()).await,
        BridgeResponse::Error(_)
    ));
    workspace.update_in(cx, |w, _, _| {
        assert!(w.automation_state.workflows.is_empty());
        for domain in [AgentDomain::People, AgentDomain::Tasks] {
            w.automation_state
                .permissions
                .set(domain, AgentAccess::Write, true);
        }
    });
    let BridgeResponse::Ok(response) = call(workspace, cx, request).await else {
        panic!("People proposal adapter")
    };
    assert_eq!(response["executed"], false);
    assert_eq!(response["requires_user_approval"], true);
    cx.condition(workspace, |w, _| w.save_status.is_saved())
        .await;
    workspace.update_in(cx, |w, _, _| assert_eq!(w.relationships, vec![original()]));
    click(workspace, cx, "desktop-start-workflow").await;
    response["workflow_id"]
        .as_str()
        .expect("accepted workflow ID")
        .into()
}

#[gpui::test]
async fn person_reminders_restore_checkpoint_and_mixed_undo_preserves_private_context(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let files = Files::new();
    let id = started(&workspace, cx, &files).await;
    click(&workspace, cx, "desktop-run-workflow-step").await;
    let first = load_desktop_state_from_path(&files.path())
        .unwrap()
        .unwrap();
    assert_eq!(first.relationships.len(), 1);
    assert_eq!(first.relationships[0].items.len(), 2);
    assert_eq!(
        first.relationships[0].items[0].text,
        "Send the reviewed brief 🗒"
    );
    assert_eq!(first.relationships[0].person_id, "person-1");
    assert_eq!(first.automation_state.jobs[0].next_step, 1);
    assert_eq!(
        first.automation_state.workflows[0].steps[0]
            .target_id
            .as_ref(),
        Some(&first.relationships[0].items[0].id)
    );
    assert!(!first
        .automation_state
        .permissions
        .allows(AgentDomain::People, AgentAccess::Read));
    workspace.update_in(cx, |w, _, cx| {
        w.relationships = first.relationships.clone();
        w.automation_state = first.automation_state.clone();
        w.meetings_hub
            .update(cx, |hub, _| hub.restore_people(first.people_cache.clone()));
        cx.notify();
    });
    click(&workspace, cx, "desktop-run-workflow-step").await;
    click(&workspace, cx, "desktop-run-workflow-step").await;
    let BridgeResponse::Ok(status) = call(
        &workspace,
        cx,
        BridgeRequest::Status {
            workflow_id: id.clone(),
        },
    )
    .await
    else {
        panic!("status")
    };
    assert_eq!(status["progress"]["durable_completed_steps"], 3);
    assert!(!status.to_string().contains("PRIVATE"));
    assert!(matches!(
        call(
            &workspace,
            cx,
            BridgeRequest::Read {
                domain: AgentDomain::People,
                query: String::new(),
                offset: 0
            }
        )
        .await,
        BridgeResponse::Error(_)
    ));
    let before_undo = std::fs::read(files.path()).unwrap();
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
    assert_eq!(std::fs::read(files.path()).unwrap(), before_undo);
    click(&workspace, cx, "desktop-undo-workflow").await;
    let saved = load_desktop_state_from_path(&files.path())
        .unwrap()
        .unwrap();
    assert_eq!(saved.relationships, vec![original()]);
    assert_eq!(
        saved.relationships[0].notes,
        "PRIVATE prior relationship context"
    );
    assert_eq!(saved.people_cache.records, first.people_cache.records);
    assert!(saved.work_items.is_empty());
    assert_eq!(
        saved.automation_state.workflows[0].status,
        WorkflowStatus::RolledBack
    );
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}

mod safety;

#[gpui::test]
async fn person_reminder_partial_undo_removes_new_context_but_keeps_contact(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let files = Files::new();
    started(&workspace, cx, &files).await;
    workspace.update_in(cx, |w, _, cx| {
        w.relationships.clear();
        w.persist_and_notify(cx);
    });
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    click(&workspace, cx, "desktop-run-workflow-step").await;
    let before = load_desktop_state_from_path(&files.path())
        .unwrap()
        .unwrap();
    assert_eq!(before.relationships.len(), 1);
    assert_eq!(before.relationships[0].items.len(), 1);
    click(&workspace, cx, "desktop-undo-workflow").await;
    let saved = load_desktop_state_from_path(&files.path())
        .unwrap()
        .unwrap();
    assert!(saved.relationships.is_empty());
    assert_eq!(saved.people_cache.records, before.people_cache.records);
    assert_eq!(
        saved.automation_state.workflows[0].status,
        WorkflowStatus::RolledBack
    );
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}
