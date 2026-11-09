use super::super::{
    desktop_state_backup_path, load_desktop_state_from_path, AppView, DesktopState, OpenCapture,
    SaveStatus, Workspace,
};
use super::tests::fixture;
use bruhs_models::PersonProfile;
use gpui::TestAppContext;
use gpui::{Entity, VisualTestContext};
mod server;

#[test]
fn legacy_workspace_defaults_people_cache_but_malformed_contacts_are_rejected() {
    let legacy: super::super::PersistedWorkspaceState = serde_json::from_str("{}").unwrap();
    assert!(legacy.people_cache.records.is_empty());
    assert!(legacy.people_cache.pending_creates.is_empty());
    assert!(
        serde_json::from_value::<super::super::PersistedWorkspaceState>(
            serde_json::json!({"people_cache":{"records":[{"id":"broken"}]}})
        )
        .is_err()
    );
}

struct Files(std::path::PathBuf);
impl Files {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("bruhs-contact-{}", uuid::Uuid::new_v4())))
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

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("Missing {selector}"));
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
}

fn capture(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    workspace.update_in(cx, |w, window, cx| {
        w.open_capture_dialog(&OpenCapture, window, cx)
    });
    click(cx, "desktop-capture-kind-contact");
    workspace.update_in(cx, |w, window, cx| {
        w.capture_input
            .update(cx, |input, cx| input.set_value("Offline Ada", window, cx))
    });
    click(cx, "desktop-capture-submit");
}

#[gpui::test]
async fn workspace_snapshot_retains_contact_records_for_offline_restore(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        let mut person = PersonProfile::new("Offline contact");
        person.id = "owned-contact".into();
        w.meetings_hub
            .update(cx, |hub, _| hub.people.push(person.clone()));
        let saved = serde_json::to_value(w.snapshot(cx)).unwrap();
        assert_eq!(
            saved["people_cache"]["records"][0],
            serde_json::to_value(person).unwrap()
        );
    });
}

#[gpui::test]
async fn captured_contact_and_pending_sync_survive_file_restore_and_retry_uses_same_id(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let files = Files::new();
    let server = workspace.update_in(cx, |w, _, cx| {
        let server = server::Server::new(&cx.global::<DesktopState>().runtime);
        w.persistence_test_path = Some(files.path());
        w.meetings_hub
            .update(cx, |hub, _| hub.set_people_test_client(server.client()));
        server
    });
    capture(&workspace, cx);
    cx.condition(&workspace, |w, cx| {
        w.save_status.is_saved() && w.meetings_hub.read(cx).notice.is_some()
    })
    .await;
    let saved = load_desktop_state_from_path(&files.path())
        .unwrap()
        .unwrap();
    assert_eq!(saved.people_cache.records.len(), 1);
    let id = saved.people_cache.records[0].id.clone();
    assert_eq!(saved.captures[0].target_id.as_ref(), Some(&id));
    assert!(saved.people_cache.pending_creates.contains(&id));
    workspace.update_in(cx, |w, _, cx| {
        w.meetings_hub.update(cx, |hub, cx| {
            hub.people.clear();
            hub.restore_people(saved.people_cache.clone());
            cx.notify();
        });
        w.active_view = AppView::People;
        cx.notify();
    });
    server.go_online();
    // The existing top-centre notice overlays the People header. Dismiss it
    // through its actual control before clicking the recovery action beneath.
    click(cx, "desktop-hub-notice-dismiss");
    click(cx, "desktop-retry-contact-sync");
    cx.condition(&workspace, |w, cx| {
        w.save_status.is_saved() && w.meetings_hub.read(cx).pending_people_count() == 0
    })
    .await;
    let saved = load_desktop_state_from_path(&files.path())
        .unwrap()
        .unwrap();
    assert_eq!(saved.people_cache.records.len(), 1);
    assert_eq!(saved.people_cache.records[0].id, id);
    assert!(saved.people_cache.pending_creates.is_empty());
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["id"], id);
    assert_eq!(requests[1]["id"], id);
}

#[gpui::test]
async fn failed_contact_file_save_keeps_draft_and_global_retry_persists_contact_and_capture(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let files = Files::new();
    let _server = workspace.update_in(cx, |w, _, cx| {
        let server = server::Server::new(&cx.global::<DesktopState>().runtime);
        w.persistence_test_path = Some(files.path());
        w.meetings_hub
            .update(cx, |hub, _| hub.set_people_test_client(server.client()));
        w.persist_and_notify(cx);
        server
    });
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let before = std::fs::read(files.path()).unwrap();
    let backup = desktop_state_backup_path(&files.path());
    if backup.is_file() {
        std::fs::remove_file(&backup).unwrap();
    }
    std::fs::create_dir(&backup).unwrap();
    capture(&workspace, cx);
    cx.condition(&workspace, |w, cx| {
        matches!(w.save_status, SaveStatus::Failed(_)) && w.meetings_hub.read(cx).notice.is_some()
    })
    .await;
    assert_eq!(std::fs::read(files.path()).unwrap(), before);
    workspace.update_in(cx, |w, _, cx| {
        assert_eq!(w.meetings_hub.read(cx).people.len(), 1)
    });
    std::fs::remove_dir(&backup).unwrap();
    click(cx, "desktop-retry-save");
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let saved = load_desktop_state_from_path(&files.path())
        .unwrap()
        .unwrap();
    assert_eq!(saved.people_cache.records.len(), 1);
    assert_eq!(saved.captures.len(), 1);
    assert_eq!(
        saved.captures[0].target_id.as_ref(),
        Some(&saved.people_cache.records[0].id)
    );
    assert_eq!(saved.people_cache.pending_creates.len(), 1);
}
