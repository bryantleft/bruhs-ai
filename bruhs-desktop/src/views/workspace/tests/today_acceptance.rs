//! Today acceptance on desktop (docs/today-completion-2026-11-14.md),
//! through GPUI test frames and a real state file: an applied day plan and
//! its Undo survive a restart (TODAY-01), a replayed Apply or a block id
//! that is already taken adds nothing (TODAY-04), and Undo refuses a focus
//! block that changed or was adopted after planning, as the web does
//! (TODAY-06).

use super::launch::{init, quit};
use super::*;
use bruhs_models::DayPlanChangeError;
use chrono::TimeZone as _;
use std::path::Path;

/// 08:00, before the 09:00 to 18:00 workday the planner fills.
fn morning() -> chrono::DateTime<chrono::Local> {
    chrono::Local
        .with_ymd_and_hms(2026, 11, 13, 8, 0, 0)
        .single()
        .expect("a local time")
}

fn ready_task(id: &str, title: &str) -> DesktopWorkItem {
    DesktopWorkItem {
        id: id.into(),
        project_id: None,
        title: title.into(),
        description: None,
        status: DesktopWorkItemStatus::Todo,
        priority: DesktopPriority::High,
        due_date: None,
        planning: WorkItemPlanning::default(),
        source: None,
        updated_at: EpochMs::UNIX_EPOCH,
    }
}

/// The app launched from `path`, on Today at 08:00.
fn launch<'a>(
    cx: &'a mut gpui::TestAppContext,
    path: &Path,
) -> (Entity<Workspace>, &'a mut gpui::VisualTestContext) {
    let (workspace, cx) = super::launch::launch(cx, path);
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.day_plan_test_now = Some(morning());
        workspace.active_view = AppView::Today;
        cx.notify();
    });
    cx.run_until_parked();
    (workspace, cx)
}

async fn saved(workspace: &Entity<Workspace>, cx: &mut gpui::VisualTestContext) {
    cx.condition(workspace, |workspace, _| {
        !workspace.day_plan_busy() && workspace.save_status.is_saved()
    })
    .await;
    cx.run_until_parked();
}

/// Each task's focus-block links.
fn links(workspace: &Workspace) -> Vec<(String, Vec<String>)> {
    workspace
        .work_items
        .iter()
        .map(|task| (task.id.clone(), task.planning.time_block_event_ids.clone()))
        .collect()
}

/// What Today and Undo read: tasks' links, events and the receipt.
#[derive(Debug, PartialEq)]
struct PlanState {
    links: Vec<(String, Vec<String>)>,
    events: Vec<bruhs_models::CalendarEvent>,
    receipt: Option<bruhs_models::DayPlanOwnership>,
}

fn plan_state(workspace: &Entity<Workspace>, cx: &mut gpui::VisualTestContext) -> PlanState {
    workspace.update_in(cx, |workspace, _window, cx| PlanState {
        links: links(workspace),
        events: workspace.planning_calendar_events(cx),
        receipt: workspace
            .day_plan_undo
            .as_ref()
            .map(|plan| plan.ownership.clone()),
    })
}

/// Seeds ready tasks, saves, then reviews and applies a plan through
/// Today's buttons and waits for the save that carries it.
async fn apply_a_plan(
    workspace: &Entity<Workspace>,
    cx: &mut gpui::VisualTestContext,
    tasks: Vec<DesktopWorkItem>,
) -> bruhs_models::DayPlanProposal {
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.calendar_events.clear();
        workspace.work_items = tasks;
        workspace.persist_and_notify(cx);
    });
    saved(workspace, cx).await;
    click_debug_selector(cx, "desktop-today-plan");
    cx.run_until_parked();
    let reviewed = workspace.update_in(cx, |workspace, _window, _cx| {
        workspace
            .day_plan_proposal
            .clone()
            .expect("a reviewed plan")
    });
    click_debug_selector(cx, "desktop-today-plan-apply");
    cx.run_until_parked();
    saved(workspace, cx).await;
    reviewed
}

/// TODAY-01: Apply saves tasks, events and the Undo receipt together. A
/// restart from the state file brings all three back, Undo still works
/// there, and its result is saved too, as a web reload keeps the plan and
/// its Undo.
#[gpui::test]
async fn an_applied_plan_and_its_undo_survive_a_restart(cx: &mut gpui::TestAppContext) {
    init(cx);
    let files = tempfile::tempdir().expect("a temporary directory");
    let path = files.path().join("state.json");
    let mut second_launch = cx.clone();
    let mut third_launch = cx.clone();
    let (workspace, cx) = launch(cx, &path);
    apply_a_plan(
        &workspace,
        cx,
        vec![
            ready_task("task-focus", "Protect focus"),
            ready_task("task-brief", "Write the brief"),
        ],
    )
    .await;
    assert_rendered_bounds(cx, "desktop-day-plan-applied");
    assert_rendered_bounds(cx, "desktop-today-plan-undo");
    let applied = plan_state(&workspace, cx);
    assert_eq!(applied.events.len(), 2, "one focus block per ready task");
    assert!(applied.links.iter().all(|(_, ids)| ids.len() == 1));
    let receipt = applied.receipt.clone().expect("an Undo receipt");

    // One save carried all three.
    let on_disk = load_desktop_state_from_path(&path).unwrap().unwrap();
    assert_eq!(on_disk.calendar_events.len(), 2);
    assert_eq!(
        on_disk.day_plan_undo.map(|plan| plan.ownership),
        Some(receipt)
    );
    assert!(on_disk
        .work_items
        .iter()
        .all(|task| task.planning.time_block_event_ids.len() == 1));
    quit(&workspace, cx);

    let (restarted, cx) = launch(&mut second_launch, &path);
    assert_eq!(
        plan_state(&restarted, cx),
        applied,
        "tasks, events and the receipt come back"
    );
    assert_rendered_bounds(cx, "desktop-day-plan-applied");
    click_debug_selector(cx, "desktop-today-plan-undo");
    cx.run_until_parked();
    saved(&restarted, cx).await;
    let undone = plan_state(&restarted, cx);
    assert!(undone.events.is_empty(), "Undo removes the focus blocks");
    assert!(undone.links.iter().all(|(_, ids)| ids.is_empty()));
    assert_eq!(undone.receipt, None);
    restarted.update_in(cx, |restarted, _window, _cx| {
        assert_eq!(restarted.day_plan_error, None);
        let titles: Vec<&str> = restarted
            .work_items
            .iter()
            .map(|task| task.title.as_str())
            .collect();
        assert_eq!(titles, ["Protect focus", "Write the brief"]);
    });
    assert_no_rendered_bounds(cx, "desktop-day-plan-applied");
    quit(&restarted, cx);

    let (third, cx) = launch(&mut third_launch, &path);
    assert_eq!(plan_state(&third, cx), undone, "the Undo was saved");
    assert_no_rendered_bounds(cx, "desktop-day-plan-applied");
}

/// TODAY-04: Apply runs once. A second Apply while the first is saving
/// does nothing; replaying the reviewed plan afterwards, or after a
/// restart, and applying a plan whose block id is already a calendar
/// record are refused with the web's wording and add no block, link or
/// receipt.
#[gpui::test]
async fn a_replayed_apply_or_a_taken_block_id_adds_nothing(cx: &mut gpui::TestAppContext) {
    init(cx);
    let files = tempfile::tempdir().expect("a temporary directory");
    let path = files.path().join("state.json");
    let mut relaunch = cx.clone();
    let (workspace, cx) = launch(cx, &path);
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.calendar_events.clear();
        workspace.work_items = vec![ready_task("task-focus", "Protect focus")];
        workspace.persist_and_notify(cx);
    });
    saved(&workspace, cx).await;
    click_debug_selector(cx, "desktop-today-plan");
    cx.run_until_parked();
    let reviewed = workspace.update_in(cx, |workspace, _window, _cx| {
        workspace
            .day_plan_proposal
            .clone()
            .expect("a reviewed plan")
    });
    click_debug_selector(cx, "desktop-today-plan-apply");
    cx.run_until_parked();

    // The first Apply is still saving: pressing Apply again changes nothing.
    workspace.update_in(cx, |workspace, _window, cx| {
        assert!(workspace.day_plan_busy(), "the first Apply is saving");
        workspace.day_plan_proposal = Some(reviewed.clone());
        workspace.apply_desktop_day_plan(cx);
        assert_eq!(workspace.calendar_events.len(), 1);
        assert_eq!(
            workspace.work_items[0].planning.time_block_event_ids.len(),
            1
        );
    });
    saved(&workspace, cx).await;
    let applied = plan_state(&workspace, cx);
    let block_id = applied.links[0].1[0].clone();

    // The review is still open: replaying it is refused.
    click_debug_selector(cx, "desktop-today-plan-apply");
    cx.run_until_parked();
    let already_applied = DayPlanChangeError::AlreadyApplied.to_string();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(
            workspace.day_plan_error.as_deref(),
            Some(already_applied.as_str())
        );
        assert!(workspace.day_plan_proposal.is_none());
    });
    assert_eq!(plan_state(&workspace, cx), applied, "a replay adds nothing");
    assert_rendered_bounds(cx, "desktop-day-plan-error");
    click_debug_selector(cx, "desktop-day-plan-dismiss");
    cx.run_until_parked();

    // A new task's plan whose block id is already a calendar record.
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace
            .work_items
            .push(ready_task("task-brief", "Write the brief"));
        workspace.persist_and_notify(cx);
    });
    saved(&workspace, cx).await;
    click_debug_selector(cx, "desktop-today-plan");
    cx.run_until_parked();
    let before = plan_state(&workspace, cx);
    workspace.update_in(cx, |workspace, _window, cx| {
        let proposal = workspace
            .day_plan_proposal
            .as_mut()
            .expect("a plan for the new task");
        assert_eq!(proposal.blocks.len(), 1, "only the new task is ready");
        assert_eq!(proposal.blocks[0].task_id, "task-brief");
        proposal.blocks[0].id = block_id.clone();
        cx.notify();
    });
    cx.run_until_parked();
    click_debug_selector(cx, "desktop-today-plan-apply");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(
            workspace.day_plan_error.as_deref(),
            Some(already_applied.as_str())
        );
        let block = workspace
            .calendar_events
            .iter()
            .find(|event| event.id == block_id)
            .expect("the first plan's block");
        assert_eq!(block.title, "Focus: Protect focus", "not overwritten");
    });
    assert_eq!(plan_state(&workspace, cx), before, "nothing was added");
    saved(&workspace, cx).await;
    quit(&workspace, cx);

    // After a restart, the reviewed plan is refused again.
    let (restarted, cx) = launch(&mut relaunch, &path);
    let restored = plan_state(&restarted, cx);
    assert_eq!(restored, before, "the restart restores the applied plan");
    restarted.update_in(cx, |restarted, _window, cx| {
        restarted.day_plan_proposal = Some(reviewed.clone());
        cx.notify();
    });
    cx.run_until_parked();
    click_debug_selector(cx, "desktop-today-plan-apply");
    cx.run_until_parked();
    restarted.update_in(cx, |restarted, _window, _cx| {
        assert_eq!(
            restarted.day_plan_error.as_deref(),
            Some(already_applied.as_str())
        );
    });
    assert_eq!(plan_state(&restarted, cx), restored);
    let on_disk = load_desktop_state_from_path(&path).unwrap().unwrap();
    assert_eq!(on_disk.calendar_events.len(), 1, "no block was saved twice");
}

/// TODAY-06: Undo removes only what the plan made, as it made it. A focus
/// block edited in the event dialog after planning, or linked to another
/// task, makes Undo refuse with the web's wording and change nothing, and
/// Undo stays offered. Once the block reads as planned again, Undo works;
/// a block the user deleted stays deleted and its link goes.
#[gpui::test]
async fn undo_refuses_a_changed_or_adopted_focus_block(cx: &mut gpui::TestAppContext) {
    init(cx);
    let files = tempfile::tempdir().expect("a temporary directory");
    let path = files.path().join("state.json");
    let (workspace, cx) = launch(cx, &path);
    apply_a_plan(
        &workspace,
        cx,
        vec![
            ready_task("task-focus", "Protect focus"),
            ready_task("task-brief", "Write the brief"),
            DesktopWorkItem {
                status: DesktopWorkItemStatus::Done,
                ..ready_task("task-done", "Already done")
            },
        ],
    )
    .await;
    let applied = plan_state(&workspace, cx);
    let focus_block = applied.links[0].1[0].clone();
    let brief_block = applied.links[1].1[0].clone();
    let show_day = |workspace: &Entity<Workspace>, cx: &mut gpui::VisualTestContext| {
        workspace.update_in(cx, |workspace, _window, cx| {
            workspace.active_view = AppView::Calendar;
            workspace.calendar_mode = DesktopCalendarMode::Day;
            workspace.calendar_anchor = morning().date_naive();
            cx.notify();
        });
        cx.run_until_parked();
    };
    let show_today = |workspace: &Entity<Workspace>, cx: &mut gpui::VisualTestContext| {
        workspace.update_in(cx, |workspace, _window, cx| {
            workspace.active_view = AppView::Today;
            cx.notify();
        });
        cx.run_until_parked();
    };
    let rename_in_dialog = |workspace: &Entity<Workspace>,
                            cx: &mut gpui::VisualTestContext,
                            block: &str,
                            title: &str| {
        click_debug_selector(cx, calendar_timegrid_event_debug_selector(block));
        cx.run_until_parked();
        workspace.update_in(cx, |workspace, window, cx| {
            assert!(workspace.event_create_visible());
            assert!(!workspace.event_dialog_read_only());
            workspace.event_create_title_input.update(cx, |input, cx| {
                input.set_value(title.to_string(), window, cx);
            });
        });
        click_debug_selector(cx, "desktop-event-create-submit");
        cx.run_until_parked();
    };

    // Edited in the event dialog after planning: Undo refuses.
    show_day(&workspace, cx);
    rename_in_dialog(&workspace, cx, &focus_block, "Focus: Protect focus, moved");
    saved(&workspace, cx).await;
    show_today(&workspace, cx);
    let edited = plan_state(&workspace, cx);
    click_debug_selector(cx, "desktop-today-plan-undo");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(
            workspace.day_plan_error,
            Some(DayPlanChangeError::GeneratedEventChanged.to_string())
        );
    });
    assert_eq!(plan_state(&workspace, cx), edited, "Undo changed nothing");
    assert_rendered_bounds(cx, "desktop-day-plan-error");
    assert_rendered_bounds(cx, "desktop-today-plan-undo");
    click_debug_selector(cx, "desktop-day-plan-dismiss");
    cx.run_until_parked();

    // Back to its planned title, then adopted by another task: Undo refuses.
    show_day(&workspace, cx);
    rename_in_dialog(&workspace, cx, &focus_block, "Focus: Protect focus");
    saved(&workspace, cx).await;
    assert_eq!(plan_state(&workspace, cx), applied, "planned content again");
    show_today(&workspace, cx);
    // No control links an existing block to a task, so a newer edit from
    // another writer (an agent, another device) is applied directly.
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.work_items[2]
            .planning
            .time_block_event_ids
            .push(focus_block.clone());
        workspace.persist_and_notify(cx);
    });
    saved(&workspace, cx).await;
    let adopted = plan_state(&workspace, cx);
    click_debug_selector(cx, "desktop-today-plan-undo");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(
            workspace.day_plan_error,
            Some(DayPlanChangeError::GeneratedLinkChanged.to_string())
        );
    });
    assert_eq!(plan_state(&workspace, cx), adopted, "Undo changed nothing");
    assert_rendered_bounds(cx, "desktop-today-plan-undo");
    click_debug_selector(cx, "desktop-day-plan-dismiss");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.work_items[2]
            .planning
            .time_block_event_ids
            .clear();
        workspace.persist_and_notify(cx);
    });
    saved(&workspace, cx).await;

    // A block the user deleted: Undo removes the other block and both
    // links, and recreates nothing.
    show_day(&workspace, cx);
    click_debug_selector(cx, calendar_timegrid_event_debug_selector(&brief_block));
    cx.run_until_parked();
    click_debug_selector(cx, "desktop-event-create-delete");
    cx.run_until_parked();
    saved(&workspace, cx).await;
    show_today(&workspace, cx);
    click_debug_selector(cx, "desktop-today-plan-undo");
    cx.run_until_parked();
    saved(&workspace, cx).await;
    let undone = plan_state(&workspace, cx);
    assert!(undone.events.is_empty(), "{:?}", undone.events);
    assert!(undone.links.iter().all(|(_, ids)| ids.is_empty()));
    assert_eq!(undone.receipt, None);
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.day_plan_error, None);
    });
    let on_disk = load_desktop_state_from_path(&path).unwrap().unwrap();
    assert!(on_disk.calendar_events.is_empty());
    assert!(on_disk.day_plan_undo.is_none());
}
