use super::{theme, AppView, DesktopState, Workspace};
use bruhs_models::{AgentAccess, AgentDomain, AutomationState, WorkflowOperation};
use gpui::{AppContext, Entity, TestAppContext, VisualTestContext};

fn fixture(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    cx.executor().allow_parking();
    cx.update(|cx| {
        gpui_component::init(cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().unwrap());
    });
    let cell = std::rc::Rc::new(std::cell::RefCell::new(None));
    let capture = cell.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *capture.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = cell.borrow().clone().unwrap();
    workspace.update_in(cx, |w, _, cx| {
        w.automation_state = AutomationState::default();
        w.work_items.clear();
        w.active_view = AppView::Automations;
        w.automation_state
            .permissions
            .set(AgentDomain::Tasks, AgentAccess::Write, true);
        cx.notify();
    });
    (workspace, cx)
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("Missing {selector}"));
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    cx.run_until_parked();
}

#[gpui::test]
async fn history_controls_pause_resume_and_run_only_the_selected_job(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        for title in ["First plan", "Second plan"] {
            let id = draft(w, title);
            w.automation_state.start_job(&id, 3).unwrap();
        }
        cx.notify();
    });
    click(cx, "desktop-previous-workflow");
    click(cx, "desktop-pause-workflow");
    workspace.update_in(cx, |w, _, _| {
        assert_eq!(w.reviewed_automation().unwrap().title, "First plan");
        assert_eq!(
            w.automation_state.jobs[0].status,
            bruhs_models::BackgroundJobStatus::Paused
        );
        assert_eq!(
            w.automation_state.jobs[1].status,
            bruhs_models::BackgroundJobStatus::Running
        );
    });
    click(cx, "desktop-resume-workflow");
    click(cx, "desktop-run-workflow-step");
    workspace.update_in(cx, |w, _, _| {
        assert_eq!(w.work_items.len(), 1);
        assert_eq!(w.work_items[0].title, "First plan");
        assert_eq!(w.automation_state.jobs[1].next_step, 0);
    });
    click(cx, "desktop-next-workflow");
    click(cx, "desktop-enable-workflow-background");
    workspace.update_in(cx, |w, _, _| {
        assert_eq!(w.reviewed_automation().unwrap().title, "Second plan");
        assert!(!w.automation_state.jobs[0].auto_run);
        assert!(w.automation_state.jobs[1].auto_run);
    });
}

#[gpui::test]
async fn undo_of_older_reviewed_workflow_preserves_newer_records_on_disk(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    let directory =
        std::env::temp_dir().join(format!("bruhs-reviewed-undo-{}", uuid::Uuid::new_v4()));
    let path = directory.join("state.json");
    workspace.update_in(cx, |w, _, cx| {
        w.persistence_test_path = Some(path.clone());
        for title in ["Older completed plan", "Newer completed plan"] {
            let id = draft(w, title);
            w.automation_state.start_job(&id, 3).unwrap();
        }
        w.persist_and_notify(cx);
    });
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    click(cx, "desktop-run-workflow-step");
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    click(cx, "desktop-previous-workflow");
    click(cx, "desktop-run-workflow-step");
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    click(cx, "desktop-undo-workflow");
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let saved = super::load_desktop_state_from_path(&path).unwrap().unwrap();
    assert_eq!(saved.work_items.len(), 1);
    assert_eq!(saved.work_items[0].title, "Newer completed plan");
    assert_eq!(
        saved.automation_state.workflows[0].status,
        bruhs_models::WorkflowStatus::RolledBack
    );
    assert_eq!(
        saved.automation_state.workflows[1].status,
        bruhs_models::WorkflowStatus::Completed
    );
    workspace.update_in(cx, |w, _, _| {
        assert_eq!(
            w.reviewed_automation().unwrap().title,
            "Older completed plan"
        )
    });
    std::fs::remove_dir_all(directory).unwrap();
}

fn draft(w: &mut Workspace, title: &str) -> String {
    let id = w.automation_state.create_workflow(
        title,
        vec![WorkflowOperation::CreateTask {
            title: title.into(),
        }],
        1,
    );
    w.automation_state.preview(&id, 2).unwrap();
    id
}

#[gpui::test]
async fn preview_never_exposes_controls_for_another_running_workflow(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        let earlier = draft(w, "Earlier running plan");
        w.automation_state.start_job(&earlier, 3).unwrap();
        draft(w, "Different displayed plan");
        cx.notify();
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-start-workflow").is_some());
    assert!(
        cx.debug_bounds("desktop-run-workflow-step").is_none(),
        "Run must belong to the displayed payload"
    );
    assert!(cx
        .debug_bounds("desktop-enable-workflow-background")
        .is_none());
}
