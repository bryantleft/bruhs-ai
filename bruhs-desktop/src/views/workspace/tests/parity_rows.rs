//! Capability matrix rows closed on desktop
//! (docs/parity-capability-matrix-2026-11-16.md), checked through GPUI test
//! frames the way a user reaches them.

use super::*;
use bruhs_models::{Meeting, MeetingSummary, ObservationKind, PersonObservation, PersonProfile};

/// A workspace in a window, with the app's theme and dialogs root. Its hub
/// talks to a closed loopback port (9), never a bruhs-server running on
/// this machine; opening a meeting reads through the real Tokio adapter, so
/// the scheduler allows that I/O.
fn parity_workspace(
    cx: &mut gpui::TestAppContext,
) -> (Entity<Workspace>, &mut gpui::VisualTestContext) {
    cx.executor().allow_parking();
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().expect("desktop state should initialize"));
    });
    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.meetings_hub.update(cx, |hub, _| {
            hub.set_people_test_client(
                crate::server_client::ServerClient::new("http://127.0.0.1:9", "http://127.0.0.1:9")
                    .expect("a loopback client"),
            );
        });
    });
    (workspace, cx)
}

fn show_view(workspace: &Entity<Workspace>, cx: &mut gpui::VisualTestContext, view: AppView) {
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.active_view = view;
        cx.notify();
    });
    cx.run_until_parked();
}

/// PM-21, PM-22 and PM-24: Today's decisions open their meeting, its
/// follow-ups open the person's profile (even past a People search that
/// hides it), and "Calendar →" opens the calendar.
#[gpui::test]
fn today_rows_open_their_meeting_person_and_calendar(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = parity_workspace(cx);
    let hour_ago = EpochMs::now().as_f64() - 3_600_000.0;
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.meetings_hub.update(cx, |hub, _| {
            let mut meeting = Meeting::new("Roadmap review", hour_ago);
            meeting.id = "meeting-roadmap".into();
            meeting.summary = Some(MeetingSummary {
                tldr: String::new(),
                action_items: vec![],
                decisions: vec!["Ship the beta on Friday".into()],
                topics: vec![],
                generated_by: "fixture".into(),
                generated_at: 1,
            });
            hub.meetings = vec![meeting];
            let mut ada = PersonProfile::new("Ada");
            ada.id = "person-ada".into();
            ada.observations.push(PersonObservation {
                at_meeting_id: "meeting-roadmap".into(),
                kind: ObservationKind::ActionItemOwner,
                text: "Send the launch notes".into(),
                created_at: EpochMs::saturating_from_f64(hour_ago).as_millis(),
            });
            hub.people = vec![ada.into(), PersonProfile::new("Grace").into()];
        });
        workspace
            .people_page
            .search_input
            .update(cx, |input, cx| input.set_value("Grace", window, cx));
    });
    show_view(&workspace, cx, AppView::Today);

    click_debug_selector(cx, "desktop-today-follow-up-0");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, window, cx| {
        assert_eq!(workspace.active_view, AppView::People);
        assert_eq!(workspace.expanded_person_id.as_deref(), Some("person-ada"));
        assert_eq!(
            workspace.people_page.search_input.read(cx).value().as_ref(),
            "",
            "a search that hides the profile is cleared"
        );
        assert!(workspace.workspace_focus_handle.is_focused(window));
    });

    show_view(&workspace, cx, AppView::Today);
    click_debug_selector(cx, "desktop-today-decision-0");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(
            workspace.active_view,
            AppView::MeetingDetail {
                meeting_id: "meeting-roadmap".into()
            }
        );
    });

    show_view(&workspace, cx, AppView::Today);
    click_debug_selector(cx, "desktop-today-calendar-link");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.active_view, AppView::Calendar);
    });
}

/// PM-23: a day plan whose save fails says so on Today. Dismiss clears the
/// error; "Refresh saved tasks and calendar" finds the saved workspace
/// unchanged and saves again, or reports another instance's save; and
/// "Retry day-plan recovery" reads the saved plan back after a start that
/// could not read it.
#[gpui::test]
async fn today_recovers_a_day_plan_that_was_not_saved(cx: &mut gpui::TestAppContext) {
    use crate::views::workspace::{SaveError, SaveStatus};
    use chrono::TimeZone as _;

    let (workspace, cx) = parity_workspace(cx);
    let dir =
        std::env::temp_dir().join(format!("bruhs-day-plan-recovery-{}", uuid::Uuid::new_v4()));
    let path = dir.join("state.json");
    let clock = chrono::Local
        .with_ymd_and_hms(2026, 11, 13, 8, 0, 0)
        .single()
        .unwrap();
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.persistence_test_path = Some(path.clone());
        workspace.active_view = AppView::Today;
        workspace.day_plan_test_now = Some(clock);
        workspace.calendar_events.clear();
        workspace.work_items = vec![DesktopWorkItem {
            id: "today-task".into(),
            project_id: None,
            title: "Protect focus".into(),
            description: None,
            status: DesktopWorkItemStatus::Todo,
            priority: DesktopPriority::High,
            due_date: None,
            planning: WorkItemPlanning::default(),
            updated_at: EpochMs::UNIX_EPOCH,
        }];
        workspace.persist_and_notify(cx);
    });
    cx.condition(&workspace, |workspace, _| workspace.save_status.is_saved())
        .await;

    // A directory where the backup goes makes the next save fail.
    let backup = desktop_state_backup_path(&path);
    fs::create_dir(&backup).unwrap();
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.draft_desktop_day_plan(cx);
        workspace.apply_desktop_day_plan(cx);
        assert!(workspace.day_plan_busy(), "the plan waits for its save");
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-day-plan-saving");
    cx.condition(&workspace, |workspace, _| !workspace.day_plan_busy())
        .await;
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert!(matches!(workspace.save_status, SaveStatus::Failed(_)));
        assert_eq!(
            workspace.day_plan_error.as_deref(),
            Some("Day plan was not saved.")
        );
    });
    assert_rendered_bounds(cx, "desktop-day-plan-error");
    assert_rendered_bounds(cx, "desktop-day-plan-dismiss");

    // Refresh: nothing newer was saved, so the window's plan is saved again.
    fs::remove_dir(&backup).unwrap();
    click_debug_selector(cx, "desktop-day-plan-refresh");
    cx.condition(&workspace, |workspace, _| {
        !workspace.day_plan_busy() && workspace.save_status.is_saved()
    })
    .await;
    cx.run_until_parked();
    let receipt = workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.day_plan_error, None);
        workspace
            .day_plan_undo
            .clone()
            .expect("the plan stays applied")
    });
    let saved = load_desktop_state_from_path(&path).unwrap().unwrap();
    assert_eq!(
        saved.day_plan_undo.map(|plan| plan.ownership),
        Some(receipt.ownership.clone()),
        "the retried save carries the plan"
    );
    assert_no_rendered_bounds(cx, "desktop-day-plan-error");
    assert_rendered_bounds(cx, "desktop-day-plan-applied");
    let saved_with_plan = fs::read(&path).unwrap();

    // An undo whose save fails says so too, and the save strip's Retry
    // clears the error once the undo reaches the disk.
    fs::remove_file(&backup).unwrap();
    fs::create_dir(&backup).unwrap();
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.undo_desktop_day_plan(cx);
    });
    cx.condition(&workspace, |workspace, _| !workspace.day_plan_busy())
        .await;
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(
            workspace.day_plan_error.as_deref(),
            Some("Day plan was not saved.")
        );
    });
    fs::remove_dir(&backup).unwrap();
    click_debug_selector(cx, "desktop-retry-save");
    cx.condition(&workspace, |workspace, _| workspace.save_status.is_saved())
        .await;
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.day_plan_error, None);
        assert!(workspace.day_plan_undo.is_none());
    });

    // Dismiss clears the error.
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.day_plan_error = Some("A planned task changed. Review a new day plan.".into());
        cx.notify();
    });
    cx.run_until_parked();
    click_debug_selector(cx, "desktop-day-plan-dismiss");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.day_plan_error, None);
    });

    // Another instance saved over the workspace: Refresh says so and pauses
    // saving instead of mixing the two.
    let saved_without_plan = fs::read(&path).unwrap();
    fs::write(&path, b"{}").unwrap();
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.day_plan_error = Some("A planned task changed. Review a new day plan.".into());
        cx.notify();
    });
    cx.run_until_parked();
    click_debug_selector(cx, "desktop-day-plan-refresh");
    cx.condition(&workspace, |workspace, _| !workspace.day_plan_busy())
        .await;
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(
            workspace.day_plan_error,
            Some(SaveError::Conflict.to_string())
        );
        assert_eq!(workspace.save_status, SaveStatus::Conflict);
        assert!(
            workspace.day_plan_undo.is_none(),
            "the window keeps its state"
        );
    });
    assert_eq!(fs::read(&path).unwrap(), b"{}", "nothing was saved over it");
    assert_ne!(saved_without_plan, b"{}");

    // A start that could not read the saved workspace: Retry reads the
    // saved plan back.
    fs::write(&path, &saved_with_plan).unwrap();
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.day_plan_loaded = false;
        workspace.day_plan_undo = None;
        workspace.day_plan_error = None;
        cx.notify();
    });
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "desktop-day-plan-applied");
    workspace.update_in(cx, |workspace, _window, cx| {
        // Web `begin_day_plan`: no plan is applied over an unread one.
        workspace.day_plan_proposal = Some(bruhs_models::DayPlanProposal {
            id: "proposal".into(),
            created_at_ms: 0,
            blocks: Vec::new(),
            unscheduled_count: 0,
            unscheduled_task_ids: Vec::new(),
            busy_blocks_respected: 0,
        });
        workspace.apply_desktop_day_plan(cx);
        assert_eq!(
            workspace.day_plan_error.as_deref(),
            Some("Day-plan recovery has not loaded. Retry loading before planning.")
        );
        workspace.day_plan_proposal = None;
        workspace.day_plan_error = None;
        cx.notify();
    });
    cx.run_until_parked();
    click_debug_selector(cx, "desktop-day-plan-retry");
    cx.condition(&workspace, |workspace, _| workspace.day_plan_loaded)
        .await;
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(
            workspace.day_plan_undo.as_ref().map(|plan| &plan.ownership),
            Some(&receipt.ownership)
        );
        assert_eq!(workspace.day_plan_error, None);
    });
    assert_no_rendered_bounds(cx, "desktop-day-plan-retry");
    assert_rendered_bounds(cx, "desktop-day-plan-applied");
    fs::remove_dir_all(dir).unwrap();
}
