use super::*;

#[gpui::test]
async fn person_reminder_rechecks_identity_and_write_grant_at_execution(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    let files = Files::new();
    started(&workspace, cx, &files).await;
    let before = std::fs::read(files.path()).unwrap();
    workspace.update_in(cx, |w, _, cx| {
        let original_people = w.meetings_hub.read(cx).people.clone();
        let original_relationships = w.relationships.clone();
        for case in ["renamed", "missing", "duplicate", "revoked"] {
            w.meetings_hub.update(cx, |hub, _| {
                hub.people = original_people.clone();
                match case {
                    "renamed" => hub.people[0].display_name = "Renamed".into(),
                    "missing" => {
                        hub.people.remove(0);
                    }
                    "duplicate" => hub.people.push(hub.people[0].clone()),
                    _ => {}
                }
            });
            w.automation_state.permissions.set(
                AgentDomain::People,
                AgentAccess::Write,
                case != "revoked",
            );
            w.run_latest_automation_step(cx);
            assert!(w.automation_error.is_some(), "{case}");
            assert_eq!(w.relationships, original_relationships, "{case}");
            assert_eq!(w.automation_state.jobs[0].next_step, 0, "{case}");
        }
    });
    assert_eq!(std::fs::read(files.path()).unwrap(), before);
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}

#[gpui::test]
async fn person_reminder_save_failure_retry_never_duplicates_reminder(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    let files = Files::new();
    started(&workspace, cx, &files).await;
    let before = std::fs::read(files.path()).unwrap();
    let backup = desktop_state_backup_path(&files.path());
    if backup.is_file() {
        std::fs::remove_file(&backup).unwrap();
    }
    std::fs::create_dir(&backup).unwrap();
    workspace.update_in(cx, |w, _, cx| {
        w.run_latest_automation_step(cx);
        w.run_latest_automation_step(cx);
    });
    cx.condition(&workspace, |w, _| {
        matches!(w.save_status, SaveStatus::Failed(_))
    })
    .await;
    workspace.update_in(cx, |w, _, cx| {
        w.run_latest_automation_step(cx);
        assert_eq!(w.relationships[0].items.len(), 2);
        assert_eq!(w.automation_state.jobs[0].next_step, 1);
        assert!(w.work_items.is_empty());
    });
    assert_eq!(std::fs::read(files.path()).unwrap(), before);
    std::fs::remove_dir(&backup).unwrap();
    click(&workspace, cx, "desktop-retry-save").await;
    let saved = load_desktop_state_from_path(&files.path())
        .unwrap()
        .unwrap();
    assert_eq!(saved.relationships[0].items.len(), 2);
    assert_eq!(saved.automation_state.jobs[0].next_step, 1);
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}

#[gpui::test]
async fn person_reminder_background_progress_and_edited_profile_block_whole_undo(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let files = Files::new();
    let id = started(&workspace, cx, &files).await;
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
        w.save_status.is_saved() && w.automation_state.jobs[0].next_step == 3
    })
    .await;
    workspace.update_in(cx, |w, _, cx| {
        w.active_view = AppView::People;
        cx.notify();
    });
    click(&workspace, cx, "desktop-person-expand-0").await;
    click(&workspace, cx, "desktop-relationship-item-0").await;
    workspace.update_in(cx, |w, _, _| {
        assert!(w.relationships[0].items[0].completed_at_ms.is_some())
    });
    let before = std::fs::read(files.path()).unwrap();
    assert!(matches!(
        call(
            &workspace,
            cx,
            BridgeRequest::RequestRollback {
                workflow_id: id.clone(),
                dry_run: false
            }
        )
        .await,
        BridgeResponse::Error(_)
    ));
    workspace.update_in(cx, |w, _, cx| {
        w.active_view = AppView::Automations;
        cx.notify();
    });
    click(&workspace, cx, "desktop-undo-workflow").await;
    workspace.update_in(cx, |w, _, _| {
        assert!(w.automation_error.is_some());
        assert_eq!(w.work_items.len(), 1);
        assert_eq!(w.relationships[0].items.len(), 3);
    });
    assert_eq!(std::fs::read(files.path()).unwrap(), before);
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}
