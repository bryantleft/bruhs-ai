use super::{
    desktop_state_backup_path, load_desktop_state_from_path, theme, AgentAccess, AgentDomain,
    AppView, AutomationState, BackgroundJobStatus, DesktopState, EpochMs, PersistedWorkspaceState,
    SaveStatus, Workspace,
};
use gpui::{AppContext, Entity};
use std::{cell::RefCell, fs, rc::Rc};

struct Fixture(std::path::PathBuf);

impl Fixture {
    fn path(&self) -> std::path::PathBuf {
        self.0.join("state.json")
    }
    fn saved(&self) -> PersistedWorkspaceState {
        load_desktop_state_from_path(&self.path()).unwrap().unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixture(
    cx: &mut gpui::TestAppContext,
) -> (Entity<Workspace>, &mut gpui::VisualTestContext, Fixture) {
    cx.update(|cx| {
        gpui_component::init(cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().unwrap());
    });
    let cell = Rc::new(RefCell::new(None));
    let capture = cell.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *capture.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = cell.borrow().clone().unwrap();
    let fixture = Fixture(
        std::env::temp_dir().join(format!("bruhs-native-background-{}", uuid::Uuid::new_v4())),
    );
    workspace.update_in(cx, |workspace, _, cx| {
        workspace.persistence_test_path = Some(fixture.path());
        workspace.automation_state = AutomationState::default();
        workspace.work_items.clear();
        workspace.calendar_events.clear();
        workspace.captures.clear();
        for domain in [
            AgentDomain::Tasks,
            AgentDomain::Calendar,
            AgentDomain::Capture,
        ] {
            workspace
                .automation_state
                .permissions
                .set(domain, AgentAccess::Write, true);
        }
        workspace.preview_focus_automation(cx);
    });
    (workspace, cx, fixture)
}

#[gpui::test]
async fn explicit_background_opt_in_completes_after_route_change(cx: &mut gpui::TestAppContext) {
    let (workspace, cx, fixture) = fixture(cx);
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    workspace.update_in(cx, |w, _, cx| w.start_latest_automation(cx));
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    cx.run_until_parked();
    assert_eq!(fixture.saved().automation_state.jobs[0].next_step, 0);
    workspace.update_in(cx, |w, _, cx| {
        w.active_view = AppView::Automations;
        cx.notify();
    });
    cx.run_until_parked();
    let button = cx
        .debug_bounds("desktop-enable-workflow-background")
        .unwrap();
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    workspace.update_in(cx, |w, _, cx| {
        assert!(w.automation_state.jobs[0].auto_run);
        assert_eq!(w.save_status, SaveStatus::Saving);
        w.active_view = AppView::Today;
        // A second activation while the opt-in is saving cannot stall the driver.
        w.run_latest_automation_step(cx);
        assert!(w.automation_error.is_none());
    });
    cx.condition(&workspace, |w, _| {
        w.save_status.is_saved()
            && w.automation_state.jobs[0].status == BackgroundJobStatus::Completed
    })
    .await;
    let saved = fixture.saved();
    assert_eq!(saved.automation_state.jobs[0].next_step, 3);
    assert_eq!(
        (
            saved.work_items.len(),
            saved.calendar_events.len(),
            saved.captures.len()
        ),
        (1, 1, 1)
    );
    workspace.update_in(cx, |w, _, cx| {
        assert_eq!(w.active_view, AppView::Today);
        w.drive_background_automations(cx);
        assert_eq!(w.work_items[0].id, saved.work_items[0].id);
        assert_eq!(w.work_items.len(), 1);
    });
}

#[gpui::test]
async fn opted_in_pause_survives_checkpoint_reload_and_resumes_without_replay(
    cx: &mut gpui::TestAppContext,
) {
    let (workspace, cx, fixture) = fixture(cx);
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    workspace.update_in(cx, |w, _, cx| w.start_latest_automation(cx));
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    workspace.update_in(cx, |w, _, cx| w.run_latest_automation_step(cx));
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    workspace.update_in(cx, |w, _, cx| {
        let id = w.automation_state.jobs[0].id.clone();
        assert!(w.automation_state.enable_background(&id));
        assert!(w
            .automation_state
            .pause_job(&id, EpochMs::now().as_millis()));
        w.active_view = AppView::Today;
        w.persist_and_notify(cx);
    });
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-workflow-status").is_some());
    let paused = fixture.saved();
    let task_id = paused.work_items[0].id.clone();
    assert_eq!(paused.automation_state.jobs[0].next_step, 1);
    workspace.update_in(cx, |w, _, cx| {
        // Rehydrate the actual saved checkpoint, then use the startup driver.
        w.automation_state = paused.automation_state;
        w.drive_background_automations(cx);
        assert_eq!(w.automation_state.jobs[0].next_step, 1);
        let id = w.automation_state.jobs[0].id.clone();
        assert!(w
            .automation_state
            .resume_job(&id, EpochMs::now().as_millis()));
        w.persist_and_notify(cx);
    });
    cx.condition(&workspace, |w, _| {
        w.save_status.is_saved()
            && w.automation_state.jobs[0].status == BackgroundJobStatus::Completed
    })
    .await;
    let saved = fixture.saved();
    assert_eq!(saved.work_items.len(), 1);
    assert_eq!(saved.work_items[0].id, task_id);
    assert_eq!((saved.calendar_events.len(), saved.captures.len()), (1, 1));
}

#[gpui::test]
async fn background_save_failure_blocks_next_step_and_retry_continues_once(
    cx: &mut gpui::TestAppContext,
) {
    let (workspace, cx, fixture) = fixture(cx);
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    workspace.update_in(cx, |w, _, cx| w.start_latest_automation(cx));
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let durable = fs::read(fixture.path()).unwrap();
    let backup = desktop_state_backup_path(&fixture.path());
    let retained = fixture.0.join("retained.json");
    fs::rename(&backup, &retained).unwrap();
    fs::create_dir(&backup).unwrap();
    workspace.update_in(cx, |w, _, cx| w.enable_latest_background_automation(cx));
    cx.condition(&workspace, |w, _| {
        matches!(w.save_status, SaveStatus::Failed(_))
    })
    .await;
    workspace.update_in(cx, |w, _, cx| {
        w.active_view = AppView::Today;
        w.drive_background_automations(cx);
        assert_eq!(w.automation_state.jobs[0].next_step, 0);
        assert!(w.work_items.is_empty());
        cx.notify();
    });
    assert_eq!(fs::read(fixture.path()).unwrap(), durable);
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-workflow-status").is_some());
    fs::remove_dir(&backup).unwrap();
    fs::rename(retained, backup).unwrap();
    let retry = cx.debug_bounds("desktop-retry-save").unwrap();
    cx.simulate_click(retry.center(), gpui::Modifiers::default());
    cx.condition(&workspace, |w, _| {
        w.save_status.is_saved()
            && w.automation_state.jobs[0].status == BackgroundJobStatus::Completed
    })
    .await;
    let saved = fixture.saved();
    assert_eq!(
        (
            saved.work_items.len(),
            saved.calendar_events.len(),
            saved.captures.len()
        ),
        (1, 1, 1)
    );
    assert_eq!(saved.automation_state.jobs[0].next_step, 3);
}

#[gpui::test]
async fn partial_native_undo_retires_job_and_preserves_user_edits(cx: &mut gpui::TestAppContext) {
    let (workspace, cx, fixture) = fixture(cx);
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    workspace.update_in(cx, |w, _, cx| w.start_latest_automation(cx));
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    workspace.update_in(cx, |w, _, cx| w.run_latest_automation_step(cx));
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let saved = fixture.saved();
    workspace.update_in(cx, |w, _, cx| {
        w.work_items[0].title = "User edit must survive".into();
        w.rollback_latest_automation(cx);
        assert!(w.automation_error.is_some());
        assert_eq!(w.work_items[0].title, "User edit must survive");
        assert_eq!(w.automation_state.jobs[0].next_step, 1);
        w.work_items[0].title = saved.work_items[0].title.clone();
        w.rollback_latest_automation(cx);
        assert_eq!(w.save_status, SaveStatus::Saving);
    });
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let undone = fixture.saved();
    assert_eq!(
        undone.automation_state.jobs[0].status,
        BackgroundJobStatus::RolledBack
    );
    assert!(undone.work_items.is_empty());
    workspace.update_in(cx, |w, _, cx| {
        w.drive_background_automations(cx);
        w.run_latest_automation_step(cx);
        assert!(w.calendar_events.is_empty());
        assert!(w.captures.is_empty());
    });
}

#[gpui::test]
async fn global_pause_and_revoked_resume_do_not_advance_workflow(cx: &mut gpui::TestAppContext) {
    let (workspace, cx, fixture) = fixture(cx);
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    workspace.update_in(cx, |w, _, cx| w.start_latest_automation(cx));
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    workspace.update_in(cx, |w, _, cx| {
        w.enable_latest_background_automation(cx);
        w.active_view = AppView::Today;
    });
    // Render without advancing the writer's debounce timer.
    cx.run_until_parked();
    let pause = cx
        .debug_bounds("desktop-pause-background-workflows")
        .unwrap();
    cx.simulate_click(pause.center(), gpui::Modifiers::default());
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    assert_eq!(
        fixture.saved().automation_state.jobs[0].status,
        BackgroundJobStatus::Paused
    );
    workspace.update_in(cx, |w, _, cx| {
        w.automation_state
            .permissions
            .set(AgentDomain::Capture, AgentAccess::Write, false);
        w.resume_latest_automation(cx);
        assert!(w.automation_error.is_some());
        assert_eq!(
            w.automation_state.jobs[0].status,
            BackgroundJobStatus::Paused
        );
        assert_eq!(w.automation_state.jobs[0].next_step, 0);
        w.drive_background_automations(cx);
        assert!(w.work_items.is_empty());
    });
}

#[gpui::test]
async fn a_refused_checkpoint_stores_no_record(cx: &mut gpui::TestAppContext) {
    use super::{AutomationRecord, CaptureKind, CaptureRecord};
    let (workspace, cx, _fixture) = fixture(cx);
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    workspace.update_in(cx, |w, _, cx| w.start_latest_automation(cx));
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    workspace.update_in(cx, |w, _, _| {
        let job_id = w.automation_state.jobs[0].id.clone();
        // The job stops being runnable between scheduling and the commit.
        assert!(w
            .automation_state
            .pause_job(&job_id, EpochMs::now().as_millis()));
        let capture = CaptureRecord::new(
            CaptureKind::Note,
            "from a stale step",
            EpochMs::now().as_millis(),
        )
        .expect("a valid capture");
        let result = w.commit_automation_record(
            &job_id,
            AutomationRecord::Capture(capture),
            EpochMs::now().as_millis(),
        );
        assert!(result.is_err());
        assert!(w.captures.is_empty(), "no record without its checkpoint");
        assert_eq!(w.automation_state.jobs[0].next_step, 0);
    });
}

#[gpui::test]
async fn an_accepted_checkpoint_stores_the_record_with_its_snapshot(cx: &mut gpui::TestAppContext) {
    use super::{AutomationRecord, CaptureKind, CaptureRecord};
    let (workspace, cx, _fixture) = fixture(cx);
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    workspace.update_in(cx, |w, _, cx| w.start_latest_automation(cx));
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    workspace.update_in(cx, |w, _, _| {
        let job_id = w.automation_state.jobs[0].id.clone();
        let capture = CaptureRecord::new(
            CaptureKind::Note,
            "from the step",
            EpochMs::now().as_millis(),
        )
        .expect("a valid capture");
        let id = capture.id.clone();
        w.commit_automation_record(
            &job_id,
            AutomationRecord::Capture(capture),
            EpochMs::now().as_millis(),
        )
        .expect("the running job accepts its next step");
        assert_eq!(w.captures[0].id, id);
        assert_eq!(w.automation_state.jobs[0].next_step, 1);
        let step = &w.automation_state.workflows[0].steps[0];
        assert_eq!(step.target_id.as_deref(), Some(id.as_str()));
        assert!(step.target_snapshot.is_some());
    });
}
