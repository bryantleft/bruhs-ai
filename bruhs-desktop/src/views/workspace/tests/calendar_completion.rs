//! Calendar completion checks (docs/calendar-completion-2026-11-14.md):
//! read-only provider events (CAL-06), task links (CAL-14), instants on
//! disk (CAL-13) and fall-back planning, through GPUI test frames.

use super::*;
use crate::views::workspace::{
    persisted_calendar_event, planning_calendar_events_in, restore_calendar_event,
    DesktopProviderEvent, PersistedCalendarEvent,
};
use chrono::{Local, TimeZone};

const HOUR_MS: i64 = 3_600_000;

/// UTC milliseconds of a civil time.
fn utc(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> i64 {
    dt(year, month, day, hour, minute)
        .and_utc()
        .timestamp_millis()
}

/// US Pacific in 2026: PDT from 2026-03-08 10:00 UTC to 2026-11-01 09:00
/// UTC, PST otherwise. Synthetic, so no test reads the host zone.
fn pacific(instant: i64) -> i32 {
    if (utc(2026, 3, 8, 10, 0)..utc(2026, 11, 1, 9, 0)).contains(&instant) {
        -7 * 60
    } else {
        -8 * 60
    }
}

/// US Eastern in 2026, for a machine that travelled.
fn eastern(instant: i64) -> i32 {
    if (utc(2026, 3, 8, 7, 0)..utc(2026, 11, 1, 6, 0)).contains(&instant) {
        -4 * 60
    } else {
        -5 * 60
    }
}

fn legacy_record(start_at: &str, end_at: &str) -> PersistedCalendarEvent {
    PersistedCalendarEvent {
        id: "event-1".into(),
        title: "Standup".into(),
        start_at: start_at.into(),
        end_at: end_at.into(),
        ..Default::default()
    }
}

/// Files written before CAL-13 hold floating wall times. They are read in
/// the current zone once and saved as instants; loading and saving again
/// changes nothing.
#[test]
fn floating_wall_times_migrate_to_instants_once() {
    let record = legacy_record("2026-05-12T09:00:00", "2026-05-12T10:00:00");
    let event = restore_calendar_event(record, pacific).expect("a valid record");
    assert_eq!(event.start_at, dt(2026, 5, 12, 9, 0));
    let saved = persisted_calendar_event(&event, pacific);
    assert_eq!(saved.start_at_ms, Some(utc(2026, 5, 12, 16, 0)));
    assert_eq!(saved.end_at_ms, Some(utc(2026, 5, 12, 17, 0)));
    assert_eq!(saved.start_at, "2026-05-12T09:00:00");

    let reloaded = restore_calendar_event(saved.clone(), pacific).expect("a saved record");
    assert_eq!(reloaded.start_at, event.start_at);
    assert_eq!(reloaded.end_at, event.end_at);
    let resaved = persisted_calendar_event(&reloaded, pacific);
    assert_eq!(resaved.start_at_ms, saved.start_at_ms);
    assert_eq!(resaved.end_at_ms, saved.end_at_ms);
}

/// A legacy time in the repeated hour of a fall-back day migrates to its
/// first occurrence, as the web's `new Date(y, m, d, h, min)` reads it.
#[test]
fn an_ambiguous_legacy_time_migrates_to_its_first_occurrence() {
    let record = legacy_record("2026-11-01T01:30:00", "2026-11-01T02:30:00");
    let event = restore_calendar_event(record, pacific).expect("a valid record");
    let saved = persisted_calendar_event(&event, pacific);
    // 01:30 PDT is 08:30 UTC; the second 01:30 (PST) would be 09:30 UTC.
    assert_eq!(saved.start_at_ms, Some(utc(2026, 11, 1, 8, 30)));
}

/// Saved instants keep their own occurrence of a repeated hour: saving
/// must not fold the second 01:30 back onto the first.
#[test]
fn a_saved_second_occurrence_round_trips() {
    let second_one_thirty = utc(2026, 11, 1, 9, 30);
    let record = PersistedCalendarEvent {
        start_at_ms: Some(second_one_thirty),
        end_at_ms: Some(second_one_thirty + HOUR_MS),
        ..legacy_record("2026-11-01T01:30:00", "2026-11-01T02:30:00")
    };
    let event = restore_calendar_event(record, pacific).expect("a saved record");
    assert_eq!(event.start_at, dt(2026, 11, 1, 1, 30));
    let saved = persisted_calendar_event(&event, pacific);
    assert_eq!(saved.start_at_ms, Some(second_one_thirty));
}

/// Instants keep the moment when the machine zone changes, as the web
/// does: a 09:00 Pacific event reads 12:00 in Eastern.
#[test]
fn saved_instants_keep_their_moment_after_a_zone_change() {
    let record = legacy_record("2026-05-12T09:00:00", "2026-05-12T10:00:00");
    let saved = persisted_calendar_event(
        &restore_calendar_event(record, pacific).expect("a valid record"),
        pacific,
    );
    let travelled = restore_calendar_event(saved, eastern).expect("a saved record");
    assert_eq!(travelled.start_at, dt(2026, 5, 12, 12, 0));
    assert_eq!(travelled.end_at, dt(2026, 5, 12, 13, 0));
}

/// An edit changes the wall times, which then name new instants.
#[test]
fn an_edited_event_saves_its_new_wall_time() {
    let record = legacy_record("2026-05-12T09:00:00", "2026-05-12T10:00:00");
    let mut event = restore_calendar_event(record, pacific).expect("a valid record");
    event.start_at = dt(2026, 5, 12, 14, 0);
    event.end_at = dt(2026, 5, 12, 15, 0);
    let saved = persisted_calendar_event(&event, pacific);
    assert_eq!(saved.start_at_ms, Some(utc(2026, 5, 12, 21, 0)));
}

/// Events in the repeated hour of a fall-back day used to have no single
/// local reading and were dropped from Today and slot search.
#[test]
fn planning_keeps_events_in_the_repeated_fall_back_hour() {
    let first = DesktopCalendarEvent {
        id: "first".into(),
        title: "First 01:30".into(),
        start_at: dt(2026, 11, 1, 1, 30),
        end_at: dt(2026, 11, 1, 1, 45),
        ..Default::default()
    };
    let second = DesktopCalendarEvent {
        id: "second".into(),
        title: "Second 01:30".into(),
        ..Default::default()
    }
    .with_instants_in(utc(2026, 11, 1, 9, 30), utc(2026, 11, 1, 9, 45), pacific)
    .expect("a valid instant");
    let planned = planning_calendar_events_in(vec![first, second], pacific);
    assert_eq!(planned.len(), 2, "{planned:?}");
    let starts: Vec<i64> = planned
        .iter()
        .map(|event| EpochMs::saturating_from_f64(event.start_at_ms).as_millis())
        .collect();
    assert_eq!(
        starts,
        vec![utc(2026, 11, 1, 8, 30), utc(2026, 11, 1, 9, 30)]
    );
    assert_eq!(starts[1] - starts[0], HOUR_MS);
}

/// A workspace window on the calendar at the 1280×800 parity cell.
fn calendar_workspace(
    cx: &mut gpui::TestAppContext,
) -> (Entity<Workspace>, &mut gpui::VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
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
    cx.simulate_resize(gpui::size(gpui::px(1280.0), gpui::px(800.0)));
    (workspace, cx)
}

fn google_event(id: &str, title: &str, start_ms: i64, end_ms: i64) -> bruhs_models::CalendarEvent {
    bruhs_models::CalendarEvent {
        id: id.into(),
        title: title.into(),
        start_at_ms: EpochMs::saturating_from_i64(start_ms).as_f64(),
        end_at_ms: EpochMs::saturating_from_i64(end_ms).as_f64(),
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
        location: None,
        conferencing: None,
        recurring_event_id: None,
        html_link: Some("https://calendar.google.com/calendar/event?eid=synthetic".into()),
    }
}

/// CAL-06 on desktop: a Google block has no resize edge and no drag, and
/// clicking it opens a read-only detail with the provider link instead of
/// nothing. Save, Delete and forged edits change nothing.
#[gpui::test]
fn a_google_block_is_read_only_and_opens_its_provider_detail(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = calendar_workspace(cx);
    let start = Local
        .from_local_datetime(&dt(2026, 5, 12, 11, 0))
        .earliest()
        .expect("a local time")
        .timestamp_millis();
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.active_view = AppView::Calendar;
        workspace.calendar_mode = DesktopCalendarMode::Day;
        workspace.calendar_anchor = NaiveDate::from_ymd_opt(2026, 5, 12).expect("a date");
        workspace.calendar_events = vec![event(
            "local-1",
            dt(2026, 5, 12, 9, 0),
            dt(2026, 5, 12, 10, 0),
        )];
        workspace.meetings_hub.update(cx, |hub, _cx| {
            hub.google_events = vec![google_event(
                "google-1",
                "Provider sync",
                start,
                start + HOUR_MS,
            )];
        });
        cx.notify();
    });
    cx.run_until_parked();

    let google_block = calendar_timegrid_event_debug_selector("google-1");
    let local_block = calendar_timegrid_event_debug_selector("local-1");
    assert_rendered_bounds(cx, google_block);
    assert_rendered_bounds(cx, local_block);
    assert_rendered_bounds(cx, calendar_timegrid_event_resize_debug_selector("local-1"));
    assert_no_rendered_bounds(
        cx,
        calendar_timegrid_event_resize_debug_selector("google-1"),
    );

    // Pressing and moving over a Google block starts no drag.
    let bounds = rendered_bounds(cx, google_block);
    let from = bounds.center();
    cx.simulate_mouse_down(from, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_move(
        gpui::point(from.x, from.y + gpui::px(DESKTOP_HOUR_ROW_PX * 2.0)),
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    assert!(
        !cx.update(|_, cx| cx.has_active_drag()),
        "a Google block must not drag"
    );
    cx.simulate_mouse_up(from, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        workspace.dismiss_overlay(|overlay| matches!(overlay, Overlay::EventCreate { .. }));
        workspace.selected_calendar_event_id = None;
        workspace.event_dialog_provider = None;
    });
    cx.run_until_parked();

    click_debug_selector(cx, google_block);
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, cx| {
        assert!(workspace.event_create_visible(), "the click opens a detail");
        assert!(workspace.event_dialog_read_only());
        assert_eq!(
            workspace.event_dialog_provider,
            Some(DesktopProviderEvent {
                html_link: Some("https://calendar.google.com/calendar/event?eid=synthetic".into()),
            })
        );
        assert_eq!(
            workspace.event_create_title_input.read(cx).value().as_ref(),
            "Provider sync"
        );
        assert_eq!(workspace.event_color_label(), "Calendar color");
    });
    assert_rendered_bounds(cx, "desktop-event-provider-link");
    assert_no_rendered_bounds(cx, "desktop-event-create-delete");
    assert_no_rendered_bounds(cx, "desktop-event-open-task");

    // A forged save changes nothing and keeps the dialog read only.
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.event_create_title_input.update(cx, |input, cx| {
            input.set_value("Forged", window, cx);
        });
        workspace.save_event_from_form(window, cx);
        assert_eq!(workspace.calendar_events.len(), 1);
        assert_eq!(workspace.calendar_events[0].id, "local-1");
        let hub = workspace.meetings_hub.read(cx);
        assert_eq!(hub.google_events[0].title, "Provider sync");
    });
    click_debug_selector(cx, "desktop-event-create-submit");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert!(workspace.event_create_visible());
        assert_eq!(workspace.calendar_events.len(), 1);
    });

    // Closing returns the dialog to its local mode.
    click_debug_selector(cx, "desktop-event-create-cancel");
    cx.run_until_parked();
    click_debug_selector(cx, local_block);
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert!(!workspace.event_dialog_read_only());
        assert_eq!(
            workspace.selected_calendar_event_id.as_deref(),
            Some("local-1")
        );
    });
    assert_rendered_bounds(cx, "desktop-event-create-delete");
    assert_no_rendered_bounds(cx, "desktop-event-provider-link");
}

/// CAL-14 on desktop: a focus block placed from a task offers Open task,
/// which opens that task in Tasks with its title focused.
#[gpui::test]
fn a_focus_block_opens_its_source_task(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = calendar_workspace(cx);
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.active_view = AppView::Calendar;
        workspace.calendar_mode = DesktopCalendarMode::Day;
        workspace.calendar_anchor = NaiveDate::from_ymd_opt(2026, 5, 12).expect("a date");
        workspace.calendar_events = vec![event(
            "block-1",
            dt(2026, 5, 12, 13, 0),
            dt(2026, 5, 12, 14, 0),
        )];
        workspace.work_items = vec![DesktopWorkItem {
            id: "task-1".into(),
            project_id: None,
            title: "Write the calendar audit".into(),
            description: None,
            status: DesktopWorkItemStatus::Todo,
            priority: DesktopPriority::Medium,
            due_date: None,
            planning: WorkItemPlanning {
                time_block_event_ids: vec!["block-1".into()],
                ..WorkItemPlanning::default()
            },
            updated_at: EpochMs::UNIX_EPOCH,
        }];
        cx.notify();
    });
    cx.run_until_parked();

    click_debug_selector(cx, calendar_timegrid_event_debug_selector("block-1"));
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-event-open-task");
    click_debug_selector(cx, "desktop-event-open-task");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, window, cx| {
        assert!(!workspace.event_create_visible());
        assert_eq!(workspace.active_view, AppView::Tasks);
        assert_eq!(workspace.selected_work_item_id.as_deref(), Some("task-1"));
        assert_eq!(
            workspace.task_detail_title_input.read(cx).value().as_ref(),
            "Write the calendar audit"
        );
        assert!(workspace
            .task_detail_title_input
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx));
    });
}

/// CAL-01/CAL-16 test frames: every view draws its header controls and
/// grid at the 1280×800 cell, and the view switch changes the grid.
#[gpui::test]
fn calendar_views_draw_their_controls_at_the_parity_cell(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = calendar_workspace(cx);
    for (mode, grid) in [
        (
            DesktopCalendarMode::Month,
            "desktop-calendar-day-2026-05-12",
        ),
        (DesktopCalendarMode::Week, "desktop-calendar-timegrid-7"),
        (DesktopCalendarMode::Day, "desktop-calendar-timegrid-1"),
    ] {
        workspace.update_in(cx, |workspace, _window, cx| {
            workspace.active_view = AppView::Calendar;
            workspace.calendar_mode = mode;
            workspace.calendar_anchor = NaiveDate::from_ymd_opt(2026, 5, 12).expect("a date");
            cx.notify();
        });
        cx.run_until_parked();
        assert_rendered_bounds(cx, "desktop-calendar-page");
        assert_rendered_bounds(cx, "desktop-calendar-title");
        assert_rendered_bounds(cx, grid);
        let page = rendered_bounds(cx, "desktop-calendar-page");
        assert!(f32::from(page.size.width) > 0.0 && f32::from(page.size.height) > 0.0);
    }
}
