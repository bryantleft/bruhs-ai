use super::super::super::{load_desktop_state_from_path, Workspace};
use super::*;
use bruhs_models::WorkflowOperation;
use gpui::{Entity, VisualTestContext};

struct Files(std::path::PathBuf);
impl Files {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("bruhs-people-read-flow-{}", uuid::Uuid::new_v4())))
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
    let bounds = cx.debug_bounds(selector).expect("workflow control");
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    cx.condition(workspace, |w, _| w.save_status.is_saved())
        .await;
}

#[gpui::test]
async fn discovered_person_identity_flows_through_user_approved_durable_reminder_and_read_back(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let files = Files::new();
    workspace.update_in(cx, |w, _, cx| {
        w.persistence_test_path = Some(files.path());
        w.relationships.clear();
        w.automation_state
            .permissions
            .set(AgentDomain::People, AgentAccess::Read, true);
        w.automation_state
            .permissions
            .set(AgentDomain::People, AgentAccess::Write, true);
        let person = PersonProfile::new("Discovered Alex");
        w.meetings_hub
            .update(cx, |hub, _| hub.people = vec![person]);
    });
    connect(&workspace, cx).await;
    let discovered = value(
        call(
            &workspace,
            cx,
            BridgeRequest::Read {
                domain: AgentDomain::People,
                query: "Discovered Alex".into(),
                offset: 0,
            },
        )
        .await,
    );
    let id = discovered["records"][0]["id"].as_str().unwrap();
    let name = discovered["records"][0]["display_name"].as_str().unwrap();
    let proposed = value(
        call(
            &workspace,
            cx,
            BridgeRequest::Propose {
                request_id: "discovered-person".into(),
                operation: WorkflowOperation::CreatePersonReminder {
                    person_id: id.into(),
                    person: name.into(),
                    reminder: "Send the next brief".into(),
                },
            },
        )
        .await,
    );
    assert_eq!(proposed["executed"], false);
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    assert_eq!(
        value(
            call(
                &workspace,
                cx,
                details(id, PersonReadSection::RelationshipItems, 0)
            )
            .await
        )["records"],
        json!([])
    );
    click(&workspace, cx, "desktop-start-workflow").await;
    click(&workspace, cx, "desktop-run-workflow-step").await;
    let persisted = load_desktop_state_from_path(&files.path())
        .unwrap()
        .unwrap();
    let read_back = value(
        call(
            &workspace,
            cx,
            details(id, PersonReadSection::RelationshipItems, 0),
        )
        .await,
    );
    assert_eq!(
        read_back["records"],
        serde_json::to_value(&persisted.relationships[0].items).unwrap()
    );
    assert_eq!(read_back["records"][0]["text"], "Send the next brief");
    assert_eq!(persisted.relationships[0].person_id, id);
    assert_eq!(persisted.automation_state.jobs[0].next_step, 1);
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}
