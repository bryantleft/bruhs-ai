//! Capability matrix rows closed on desktop
//! (docs/parity-capability-matrix-2026-11-16.md), checked through GPUI test
//! frames the way a user reaches them.

use super::*;
use bruhs_models::{Meeting, MeetingSummary, ObservationKind, PersonObservation, PersonProfile};
use gpui_component::zoom::px;

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

/// A read-only Google event half an hour long.
fn google_event(id: &str, title: &str, start_ms: i64) -> bruhs_models::CalendarEvent {
    bruhs_models::CalendarEvent {
        id: id.into(),
        title: title.into(),
        start_at_ms: EpochMs::saturating_from_i64(start_ms).as_f64(),
        end_at_ms: EpochMs::saturating_from_i64(start_ms + 1_800_000).as_f64(),
        all_day: false,
        color: None,
        notes: Some("Provider notes".into()),
        created_at: 1,
        updated_at: 1,
        source: bruhs_models::EventSource::Google,
        google_event_id: Some(format!("g-{id}")),
        calendar_id: Some("primary".into()),
        attendees: Vec::new(),
        organizer: None,
        location: Some("Room 4".into()),
        conferencing: None,
        recurring_event_id: None,
        html_link: Some("https://calendar.google.com/calendar/event?eid=synthetic".into()),
    }
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

/// PM-25: a local event's dialog records a meeting through the hub's
/// auto-record start, with the event's title and id as its link key. A
/// Google event's read-only dialog has no Record meeting, as on the web.
#[gpui::test]
fn event_dialog_records_a_meeting_for_a_local_event(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = parity_workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace
            .meetings_hub
            .update(cx, |hub, _| hub.test_recording_starts = Some(Vec::new()));
        let mut review = event(
            "design-review",
            dt(2026, 11, 16, 10, 0),
            dt(2026, 11, 16, 11, 0),
        );
        review.title = "Design review".into();
        workspace.calendar_events = vec![review];
        workspace.open_event_edit("design-review".into(), window, cx);
    });
    cx.run_until_parked();
    // The form's edits are not what records: the saved event is.
    workspace.update_in(cx, |workspace, window, cx| {
        workspace
            .event_create_title_input
            .update(cx, |input, cx| input.set_value("Unsaved title", window, cx));
    });
    click_debug_selector(cx, "desktop-event-record-meeting");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, cx| {
        let starts = workspace
            .meetings_hub
            .read(cx)
            .test_recording_starts
            .clone()
            .unwrap();
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0].title, "Design review");
        assert_eq!(
            starts[0].calendar_event_id.as_deref(),
            Some("design-review")
        );
    });

    let start_ms = EpochMs::now().as_millis() + 3_600_000;
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.close_event_create(window, cx);
        workspace.meetings_hub.update(cx, |hub, _| {
            hub.google_events = vec![google_event("google-sync", "Provider sync", start_ms)];
        });
        workspace.open_event_edit("google-sync".into(), window, cx);
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-event-create-dialog");
    assert_no_rendered_bounds(cx, "desktop-event-record-meeting");
}

/// A workspace restarted from `path`, as a launch reads it.
fn restart_from(
    workspace: &Entity<Workspace>,
    cx: &mut gpui::VisualTestContext,
    path: &std::path::Path,
) -> Entity<Workspace> {
    workspace.update_in(cx, |_, window, cx| {
        cx.new(|cx| {
            Workspace::from_loaded_state(
                load_desktop_state_with_backup(path),
                window,
                cx,
                crate::views::workspace::RestoredRecords::take_from_persisted,
                Workspace::restore_threads,
            )
        })
    })
}

/// PM-26 and PM-27: a synced Google event is saved with the workspace, so
/// it comes back read only after a restart; Copy recovery leaves it out,
/// and the agent's calendar tools never see it. Settings › Calendar ›
/// Remove all removes it for good.
#[gpui::test]
async fn google_events_survive_a_restart_until_removed(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = parity_workspace(cx);
    let dir = std::env::temp_dir().join(format!("bruhs-google-events-{}", uuid::Uuid::new_v4()));
    let path = dir.join("state.json");
    let start_ms = EpochMs::now().as_millis() + 3_600_000;
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.persistence_test_path = Some(path.clone());
        workspace.meetings_hub.update(cx, |hub, cx| {
            hub.apply_calendar_event(
                bruhs_models::CalendarSyncEvent::EventUpserted {
                    event: Box::new(google_event("google-sync", "Provider sync", start_ms)),
                },
                cx,
            );
        });
    });
    cx.condition(&workspace, |workspace, _| workspace.save_status.is_saved())
        .await;
    let saved = load_desktop_state_from_path(&path).unwrap().unwrap();
    assert_eq!(saved.google_events.len(), 1);
    assert_eq!(saved.google_events[0].title, "Provider sync");
    assert_eq!(saved.google_events[0].location.as_deref(), Some("Room 4"));
    workspace.update_in(cx, |workspace, _window, cx| {
        assert!(
            workspace.recovery_snapshot(cx).google_events.is_empty(),
            "Copy recovery keeps only this window's own records"
        );
        assert_eq!(workspace.snapshot(cx).google_events.len(), 1);
    });

    let restarted = restart_from(&workspace, cx, &path);
    restarted.update_in(cx, |restarted, _window, cx| {
        let events = restarted.visible_calendar_events(cx);
        let google = events
            .iter()
            .find(|event| event.id == "google-sync")
            .expect("the Google event shows after a restart");
        assert!(google.is_read_only());
        assert_eq!(google.title, "Provider sync");
        assert!(
            restarted
                .calendar_events
                .iter()
                .all(|event| event.id != "google-sync"),
            "the agent's calendar tools read local events only"
        );
    });

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_settings(&OpenSettings, window, cx);
    });
    cx.run_until_parked();
    click_debug_selector(cx, "desktop-settings-nav-calendar");
    cx.run_until_parked();
    click_debug_selector(cx, "desktop-settings-google-clear");
    cx.run_until_parked();
    cx.condition(&workspace, |workspace, _| workspace.save_status.is_saved())
        .await;
    workspace.update_in(cx, |workspace, _window, cx| {
        assert!(workspace.meetings_hub.read(cx).google_events.is_empty());
    });
    assert_no_rendered_bounds(cx, "desktop-settings-google-clear");
    let saved = load_desktop_state_from_path(&path).unwrap().unwrap();
    assert!(saved.google_events.is_empty(), "Remove all is saved");
    let restarted = restart_from(&workspace, cx, &path);
    restarted.update_in(cx, |restarted, _window, cx| {
        assert!(restarted.meetings_hub.read(cx).google_events.is_empty());
        assert!(restarted
            .visible_calendar_events(cx)
            .iter()
            .all(|event| event.id != "google-sync"));
    });
    fs::remove_dir_all(dir).unwrap();
}

/// A saved list holds only Google's own events: anything else is dropped on
/// restore rather than shown as a provider's.
#[gpui::test]
fn restored_google_events_are_only_provider_events(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = parity_workspace(cx);
    workspace.update_in(cx, |workspace, _window, cx| {
        let mut local = google_event("forged", "Local look-alike", 0);
        local.source = bruhs_models::EventSource::Local;
        workspace.meetings_hub.update(cx, |hub, _| {
            hub.restore_google_events(vec![local, google_event("google-sync", "Provider sync", 0)]);
        });
        let ids = workspace
            .meetings_hub
            .read(cx)
            .google_events
            .iter()
            .map(|event| event.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["google-sync"]);
    });
}

fn task(id: &str, title: &str) -> DesktopWorkItem {
    DesktopWorkItem {
        id: id.into(),
        project_id: None,
        title: title.into(),
        description: None,
        status: DesktopWorkItemStatus::Todo,
        priority: DesktopPriority::None,
        due_date: None,
        planning: WorkItemPlanning::default(),
        updated_at: EpochMs::UNIX_EPOCH,
    }
}

fn dependencies(workspace: &Entity<Workspace>, cx: &mut gpui::VisualTestContext) -> Vec<String> {
    workspace.update_in(cx, |workspace, _window, _cx| {
        workspace
            .selected_work_item()
            .expect("a selected task")
            .planning
            .dependency_ids
    })
}

/// PM-28: the task detail picks a dependency from a list of the other
/// tasks and removes one dependency with its chip, as the web does. A
/// dependency whose task is gone says "Missing task".
#[gpui::test]
fn task_detail_picks_and_removes_one_dependency(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = parity_workspace(cx);
    cx.simulate_resize(gpui::size(px(1400.0), px(1400.0)));
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.active_view = AppView::Tasks;
        workspace.work_items = vec![
            task("write", "Write"),
            task("review", "Review"),
            task("ship", "Ship"),
        ];
        workspace.select_work_item_by_id("ship", window, cx);
    });
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "desktop-task-detail-focus-blocks");

    click_debug_selector(cx, "desktop-task-detail-select-dependencies");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-task-detail-dependency-placeholder");
    // Every other task, in list order: Write and Review, not Ship itself.
    assert_rendered_bounds(cx, "desktop-task-detail-dependency-option-1");
    assert_no_rendered_bounds(cx, "desktop-task-detail-dependency-option-2");
    click_debug_selector(cx, "desktop-task-detail-dependency-option-1");
    cx.run_until_parked();
    assert_eq!(dependencies(&workspace, cx), ["review"]);
    assert_no_rendered_bounds(cx, "desktop-task-detail-menu-dependencies");

    click_debug_selector(cx, "desktop-task-detail-select-dependencies");
    cx.run_until_parked();
    click_debug_selector(cx, "desktop-task-detail-dependency-option-0");
    cx.run_until_parked();
    // Picking one already there, or the placeholder, changes nothing.
    for option in [
        "desktop-task-detail-dependency-option-1",
        "desktop-task-detail-dependency-placeholder",
    ] {
        click_debug_selector(cx, "desktop-task-detail-select-dependencies");
        cx.run_until_parked();
        click_debug_selector(cx, option);
        cx.run_until_parked();
    }
    assert_eq!(dependencies(&workspace, cx), ["review", "write"]);

    // A chip removes only its own dependency.
    click_debug_selector(cx, "desktop-task-detail-dependency-chip-0");
    cx.run_until_parked();
    assert_eq!(dependencies(&workspace, cx), ["write"]);
    assert_no_rendered_bounds(cx, "desktop-task-detail-dependency-chip-1");

    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.work_items.retain(|task| task.id != "write");
        let ship = workspace
            .work_items
            .iter_mut()
            .find(|task| task.id == "ship")
            .unwrap();
        ship.planning.time_block_event_ids = vec!["block-1".into()];
        cx.notify();
    });
    cx.run_until_parked();
    // The web's note appears once the task has focus blocks, and the chip
    // for the deleted task stays (titled "Missing task") until removed.
    assert_rendered_bounds(cx, "desktop-task-detail-focus-blocks");
    assert_rendered_bounds(cx, "desktop-task-detail-dependency-chip-0");
    click_debug_selector(cx, "desktop-task-detail-dependency-chip-0");
    cx.run_until_parked();
    assert!(dependencies(&workspace, cx).is_empty());
}

/// PM-29: Delete task asks first, as the web does. Cancel keeps the task
/// open; Delete removes it and leaves focus on the workspace.
#[gpui::test]
fn task_delete_asks_before_deleting(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = parity_workspace(cx);
    cx.simulate_resize(gpui::size(px(1400.0), px(1000.0)));
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.active_view = AppView::Tasks;
        workspace.work_items = vec![task("ship", "Ship v2"), task("write", "Write")];
        workspace.select_work_item_by_id("ship", window, cx);
    });
    cx.run_until_parked();

    click_debug_selector(cx, "desktop-task-detail-delete");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-task-delete-dialog");
    assert_rendered_bounds(cx, "desktop-task-delete-prompt");
    click_debug_selector(cx, "desktop-task-delete-cancel");
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "desktop-task-delete-dialog");
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.work_items.len(), 2);
        assert_eq!(workspace.selected_work_item_id.as_deref(), Some("ship"));
    });

    click_debug_selector(cx, "desktop-task-detail-delete");
    cx.run_until_parked();
    click_debug_selector(cx, "desktop-task-delete-confirm");
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "desktop-task-delete-dialog");
    workspace.update_in(cx, |workspace, window, _cx| {
        let ids = workspace
            .work_items
            .iter()
            .map(|task| task.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["write"], "only the confirmed task is deleted");
        assert_eq!(workspace.selected_work_item_id, None);
        assert!(workspace.workspace_focus_handle.is_focused(window));
    });
}

/// PM-30: in a compact window the project sidebar cannot show, so the Tasks
/// header offers the web's "Projects" dialog. Choosing a project, or All
/// Tasks, filters the list and closes it. A wide window shows the sidebar
/// and no Projects button, as the web hides it from `md` up.
#[gpui::test]
fn compact_tasks_switch_project_from_the_projects_dialog(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = parity_workspace(cx);
    cx.simulate_resize(gpui::size(px(760.0), px(700.0)));
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.active_view = AppView::Tasks;
        workspace.sidebar_visible = true;
        workspace.projects = vec![
            DesktopProject {
                id: "alpha".into(),
                name: "Alpha".into(),
            },
            DesktopProject {
                id: "beta".into(),
                name: "Beta".into(),
            },
        ];
        let mut alpha = task("alpha-task", "Alpha task");
        alpha.project_id = Some("alpha".into());
        let mut beta = task("beta-task", "Beta task");
        beta.project_id = Some("beta".into());
        workspace.work_items = vec![alpha, beta];
        workspace.selected_project_id = None;
        cx.notify();
    });
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "desktop-project-sidebar");

    click_debug_selector(cx, "desktop-task-open-projects");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-task-projects-dialog");
    assert_rendered_bounds(cx, "desktop-task-projects-list");
    click_debug_selector(cx, project_row_debug_selector("beta"));
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "desktop-task-projects-dialog");
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.selected_project_id.as_deref(), Some("beta"));
    });
    assert_rendered_bounds(cx, task_row_debug_selector("beta-task"));
    assert_no_rendered_bounds(cx, task_row_debug_selector("alpha-task"));

    click_debug_selector(cx, "desktop-task-open-projects");
    cx.run_until_parked();
    click_debug_selector(cx, "desktop-project-all");
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "desktop-task-projects-dialog");
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.selected_project_id, None);
    });

    cx.simulate_resize(gpui::size(px(1400.0), px(900.0)));
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-project-sidebar");
    assert_no_rendered_bounds(cx, "desktop-task-open-projects");
}
