use crate::app_state::DesktopState;
use crate::theme;
use crate::views::workspace::persistence::{
    load_desktop_state_from_path, write_desktop_state_atomically, RestoredRecords,
};
use crate::views::workspace::{
    AppView, DesktopCalendarEvent, DesktopPriority, DesktopWorkItem, DesktopWorkItemStatus,
    Workspace,
};
use bruhs_models::{AvailabilityRule, EpochMs, WorkItemPlanning, WorkItemSource};
use chrono::{Local, TimeZone};
use gpui::{AppContext, Modifiers, VisualTestContext};
use std::{cell::RefCell, rc::Rc};

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("visible control: {selector}"));
    cx.simulate_click(bounds.center(), Modifiers::default());
    cx.run_until_parked();
}

#[gpui::test]
fn native_availability_plan_apply_restore_undo_and_changed_hours_keep_the_meeting_task(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        bruhs_ui::init(cx);
        bruhs_ui::Theme::change(bruhs_ui::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().unwrap());
    });
    let captured = Rc::new(RefCell::new(None));
    let target = captured.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *target.borrow_mut() = Some(workspace.clone());
        bruhs_ui::Root::new(workspace, window, cx)
    });
    let workspace = captured.borrow().clone().unwrap();
    let now = Local
        .with_ymd_and_hms(2026, 11, 16, 8, 0, 0)
        .single()
        .unwrap();
    let source = WorkItemSource::Meeting {
        meeting_id: "planning-meeting".into(),
        title: "Planning".into(),
    };
    workspace.update_in(cx, |workspace, _, cx| {
        workspace.work_items = vec![DesktopWorkItem {
            id: "meeting-task".into(),
            project_id: None,
            title: "Implement the meeting action".into(),
            description: Some("Keep meeting context".into()),
            status: DesktopWorkItemStatus::Todo,
            priority: DesktopPriority::High,
            due_date: None,
            planning: WorkItemPlanning {
                estimate_minutes: Some(90),
                ..Default::default()
            },
            source: Some(source.clone()),
            updated_at: EpochMs::new(now.timestamp_millis()).unwrap(),
        }];
        workspace.calendar_events.clear();
        workspace.scheduling_preferences.availability = vec![
            AvailabilityRule::weekday("Morning", 540, 600),
            AvailabilityRule::weekday("Afternoon", 840, 960),
        ];
        workspace.day_plan_test_now = Some(now);
        workspace.active_view = AppView::Today;
        cx.notify();
    });
    cx.run_until_parked();
    click(cx, "desktop-today-plan");
    workspace.update_in(cx, |workspace, _, _| {
        let plan = workspace.day_plan_proposal.as_ref().unwrap();
        assert_eq!(plan.blocks.len(), 1);
        assert_eq!(
            plan.blocks[0].start_at_ms,
            EpochMs::new(
                Local
                    .with_ymd_and_hms(2026, 11, 16, 14, 0, 0)
                    .single()
                    .unwrap()
                    .timestamp_millis()
            )
            .unwrap()
            .as_f64()
        );
        assert_eq!(
            plan.blocks[0].end_at_ms,
            EpochMs::new(
                Local
                    .with_ymd_and_hms(2026, 11, 16, 15, 30, 0)
                    .single()
                    .unwrap()
                    .timestamp_millis()
            )
            .unwrap()
            .as_f64()
        );
    });
    click(cx, "desktop-today-plan-apply");
    workspace.update_in(cx, |workspace, _, cx| {
        assert_eq!(workspace.calendar_events.len(), 1);
        assert_eq!(workspace.work_items[0].source, Some(source.clone()));
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("desktop-state.json");
        write_desktop_state_atomically(&path, &workspace.snapshot(cx), 1).unwrap();
        let mut restored = load_desktop_state_from_path(&path).unwrap().unwrap();
        workspace.day_plan_undo = restored.day_plan_undo.clone();
        let records = RestoredRecords::take_from_persisted(Some(&mut restored));
        workspace.work_items = records.work_items;
        workspace.calendar_events = records.calendar_events;
        workspace.work_items[0].description = Some("Owner's newer follow-up context".into());
        cx.notify();
    });
    cx.run_until_parked();
    click(cx, "desktop-today-plan-undo");
    workspace.update_in(cx, |workspace, _, cx| {
        assert!(workspace.calendar_events.is_empty());
        assert_eq!(workspace.work_items[0].source, Some(source.clone()));
        assert_eq!(
            workspace.work_items[0].description.as_deref(),
            Some("Owner's newer follow-up context")
        );
        assert!(workspace.work_items[0]
            .planning
            .time_block_event_ids
            .is_empty());
        workspace.scheduling_preferences.availability =
            vec![AvailabilityRule::weekday("Hours", 540, 1020)];
        cx.notify();
    });
    cx.run_until_parked();
    click(cx, "desktop-today-plan");
    workspace.update_in(cx, |workspace, _, cx| {
        workspace.scheduling_preferences.availability =
            vec![AvailabilityRule::weekday("Changed", 540, 570)];
        cx.notify();
    });
    cx.run_until_parked();
    click(cx, "desktop-today-plan-apply");
    workspace.update_in(cx, |workspace, _, _| {
        assert!(workspace.calendar_events.is_empty());
        assert!(workspace.work_items[0]
            .planning
            .time_block_event_ids
            .is_empty());
        assert_eq!(workspace.work_items[0].source, Some(source.clone()));
        assert!(workspace
            .day_plan_error
            .as_ref()
            .unwrap()
            .contains("Working hours changed"));
    });
    workspace.update_in(cx, |workspace, _, cx| {
        workspace.scheduling_preferences.availability =
            vec![AvailabilityRule::weekday("Hours", 540, 1020)];
        cx.notify();
    });
    cx.run_until_parked();
    click(cx, "desktop-today-plan");
    workspace.update_in(cx, |workspace, _, cx| {
        assert_eq!(
            workspace.day_plan_proposal.as_ref().unwrap().blocks.len(),
            1
        );
        let midnight = now.date_naive().and_hms_opt(0, 0, 0).unwrap();
        workspace.calendar_events.push(DesktopCalendarEvent {
            id: "owner-time-off".into(),
            title: "Owner's time off".into(),
            start_at: midnight,
            end_at: midnight + chrono::Duration::days(1),
            all_day: true,
            ..Default::default()
        });
        cx.notify();
    });
    cx.run_until_parked();
    click(cx, "desktop-today-plan-apply");
    workspace.update_in(cx, |workspace, _, _| {
        assert_eq!(workspace.calendar_events.len(), 1);
        assert_eq!(workspace.calendar_events[0].id, "owner-time-off");
        assert!(workspace.calendar_events[0].all_day);
        assert!(workspace.work_items[0]
            .planning
            .time_block_event_ids
            .is_empty());
        assert_eq!(workspace.work_items[0].source, Some(source));
        assert!(workspace
            .day_plan_error
            .as_ref()
            .unwrap()
            .contains("calendar changed"));
    });
    click(cx, "desktop-today-plan");
    workspace.update_in(cx, |workspace, _, _| {
        assert!(workspace
            .day_plan_proposal
            .as_ref()
            .unwrap()
            .blocks
            .is_empty());
    });
}
