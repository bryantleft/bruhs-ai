//! Calendar time zones on desktop (docs/calendar-completion-2026-11-14.md),
//! in the zones the web harness uses. Each zone runs in a child process with
//! its own TZ, never by changing the parallel test runner's zone:
//!
//! - CAL-03: on the 2026-03-08 and 2026-11-01 daylight-saving days, a 10:00
//!   event sits on the 10:00 row, clicking the 11:00 row prefills 11:00, and
//!   dragging the event to the 14:00 row saves 14:00 local.
//! - CAL-05: a Google all-day date (2026-11-02) arrives as a UTC midnight
//!   and shows on that date only; a replayed frame and a restart from the
//!   state file leave it there.
//! - CAL-02: local timed and all-day events created and deleted in the
//!   dialog survive restarts from the state file.

use super::launch::{init, launch, quit, saved};
use super::*;
use crate::views::meetings_hub::local_offset_minutes_east;
use crate::views::workspace::persisted_calendar_event;
use chrono::{Local, TimeZone};

const ZONE_CHILD: &str = "BRUHS_CALENDAR_ZONE_TEST_CHILD";
const HOUR_MS: i64 = 3_600_000;

fn local_ms(date: NaiveDate, hour: u32) -> i64 {
    Local
        .from_local_datetime(&date.and_hms_opt(hour, 0, 0).expect("a time"))
        .single()
        .expect("an unambiguous local time")
        .timestamp_millis()
}

fn utc_ms(year: i32, month: u32, day: u32, hour: u32) -> i64 {
    chrono::Utc
        .with_ymd_and_hms(year, month, day, hour, 0, 0)
        .single()
        .expect("a UTC time")
        .timestamp_millis()
}

fn hour_row(date: NaiveDate, hour: u32) -> &'static str {
    leaked_debug_selector(format!("desktop-calendar-timegrid-hour-{date}-{hour}"))
}

fn show(
    workspace: &Entity<Workspace>,
    cx: &mut gpui::VisualTestContext,
    mode: DesktopCalendarMode,
    date: NaiveDate,
) {
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.active_view = AppView::Calendar;
        workspace.calendar_mode = mode;
        workspace.calendar_anchor = date;
        cx.notify();
    });
    cx.run_until_parked();
}

/// CAL-03 in one zone, on one daylight-saving day.
async fn dst_day(
    workspace: &Entity<Workspace>,
    cx: &mut gpui::VisualTestContext,
    zone: &str,
    date: NaiveDate,
    la_fourteen_utc: i64,
) {
    let ten = local_ms(date, 10);
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.calendar_events = vec![DesktopCalendarEvent {
            id: "standup".into(),
            title: "Standup".into(),
            ..Default::default()
        }
        .with_instants_in(ten, ten + HOUR_MS, local_offset_minutes_east)
        .expect("a valid instant")];
        cx.notify();
    });
    show(workspace, cx, DesktopCalendarMode::Day, date);
    let block = rendered_bounds(cx, calendar_timegrid_event_debug_selector("standup"));
    let ten_row = rendered_bounds(cx, hour_row(date, 10));
    assert!(
        (f32::from(block.top()) - f32::from(ten_row.top())).abs() < 1.0,
        "{zone} {date}: the 10:00 event sits on the 10:00 row: {block:?} {ten_row:?}"
    );

    // Clicking the 11:00 row prefills 11:00.
    click_debug_selector(cx, hour_row(date, 11));
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, cx| {
        assert!(workspace.event_create_visible());
        assert_eq!(
            workspace.event_create_start_input.read(cx).value().as_ref(),
            format!("{date}T11:00"),
            "{zone} {date}"
        );
    });
    click_debug_selector(cx, "desktop-event-create-cancel");
    cx.run_until_parked();

    // Drag the event by its top to the 14:00 row.
    let block = rendered_bounds(cx, calendar_timegrid_event_debug_selector("standup"));
    let grab = gpui::point(block.center().x, block.top() + px(4.0));
    let start_drag = gpui::point(grab.x + px(6.0), grab.y);
    let fourteen = rendered_bounds(cx, hour_row(date, 14));
    let drop = gpui::point(start_drag.x, fourteen.top() + px(4.0));
    cx.simulate_mouse_down(grab, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_move(
        start_drag,
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_move(
        drop,
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(drop, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.run_until_parked();
    saved(workspace, cx).await;
    let (start_at, saved_ms) = workspace.update_in(cx, |workspace, _window, _cx| {
        let event = &workspace.calendar_events[0];
        (
            event.start_at,
            persisted_calendar_event(event, local_offset_minutes_east).start_at_ms,
        )
    });
    assert_eq!(
        start_at,
        date.and_hms_opt(14, 0, 0).expect("14:00"),
        "{zone}"
    );
    assert_eq!(
        saved_ms,
        Some(local_ms(date, 14)),
        "{zone} {date}: 14:00 local"
    );
    if zone == "America/Los_Angeles" {
        assert_eq!(
            saved_ms,
            Some(la_fourteen_utc),
            "{date}: 14:00 in Los Angeles"
        );
    }
}

fn google_all_day() -> bruhs_models::CalendarEvent {
    bruhs_models::CalendarEvent {
        id: "holiday".into(),
        title: "Synthetic holiday".into(),
        start_at_ms: EpochMs::saturating_from_i64(utc_ms(2026, 11, 2, 0)).as_f64(),
        end_at_ms: EpochMs::saturating_from_i64(utc_ms(2026, 11, 3, 0)).as_f64(),
        all_day: true,
        color: None,
        notes: None,
        created_at: 1,
        updated_at: 1,
        source: bruhs_models::EventSource::Google,
        google_event_id: Some("g-holiday".into()),
        calendar_id: Some("primary".into()),
        attendees: Vec::new(),
        organizer: None,
        location: None,
        conferencing: None,
        recurring_event_id: None,
        html_link: Some("https://calendar.google.com/calendar/event?eid=synthetic".into()),
    }
}

/// CAL-05 checks on the month: the event's date has records, its
/// neighbours none, and its pill is inside its own cell.
fn holiday_on_its_date(cx: &mut gpui::VisualTestContext, zone: &str) {
    assert_rendered_bounds(cx, "desktop-calendar-day-open-2026-11-02");
    assert_no_rendered_bounds(cx, "desktop-calendar-day-open-2026-11-01");
    assert_no_rendered_bounds(cx, "desktop-calendar-day-open-2026-11-03");
    let cell = rendered_bounds(cx, "desktop-calendar-day-2026-11-02");
    let pill = rendered_bounds(cx, calendar_event_pill_debug_selector("holiday"));
    assert!(
        cell.contains(&pill.center()),
        "{zone}: the holiday is in its own cell: {pill:?} {cell:?}"
    );
}

async fn in_zone(cx: &mut gpui::TestAppContext, zone: &str) {
    init(cx);
    let files = tempfile::tempdir().expect("a temporary directory");
    let path = files.path().join("state.json");
    let mut relaunch = cx.clone();
    let (workspace, cx) = launch(cx, &path);
    if zone == "America/Los_Angeles" {
        // The child really runs in Los Angeles: PDT on 2026-03-08 after 03:00.
        assert_eq!(local_offset_minutes_east(utc_ms(2026, 3, 8, 21)), -7 * 60);
        assert_eq!(local_offset_minutes_east(utc_ms(2026, 11, 1, 22)), -8 * 60);
    }
    cx.simulate_resize(gpui::size(px(1280.0), px(1400.0)));
    cx.run_until_parked();
    for (date, la_fourteen_utc) in [
        (
            NaiveDate::from_ymd_opt(2026, 3, 8).expect("a date"),
            utc_ms(2026, 3, 8, 21),
        ),
        (
            NaiveDate::from_ymd_opt(2026, 11, 1).expect("a date"),
            utc_ms(2026, 11, 1, 22),
        ),
    ] {
        dst_day(&workspace, cx, zone, date, la_fourteen_utc).await;
    }

    // CAL-05: the provider frame arrives twice, as a replay does.
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.calendar_events.clear();
        workspace.meetings_hub.update(cx, |hub, cx| {
            for _ in 0..2 {
                hub.apply_calendar_event(
                    bruhs_models::CalendarSyncEvent::EventUpserted {
                        event: Box::new(google_all_day()),
                    },
                    cx,
                );
            }
            assert_eq!(hub.google_events.len(), 1, "a replay adds nothing");
        });
    });
    let date = NaiveDate::from_ymd_opt(2026, 11, 2).expect("a date");
    show(&workspace, cx, DesktopCalendarMode::Month, date);
    holiday_on_its_date(cx, zone);
    saved(&workspace, cx).await;
    quit(&workspace, cx);

    // A restart reads the saved event and leaves it on its date.
    let (restarted, cx) = launch(&mut relaunch, &path);
    cx.simulate_resize(gpui::size(px(1280.0), px(1400.0)));
    show(&restarted, cx, DesktopCalendarMode::Month, date);
    holiday_on_its_date(cx, zone);
    restarted.update_in(cx, |workspace, _window, cx| {
        let hub = workspace.meetings_hub.read(cx);
        assert_eq!(hub.google_events.len(), 1);
        assert_eq!(
            hub.google_events[0].start_at_ms,
            EpochMs::saturating_from_i64(local_ms(date, 0)).as_f64(),
            "{zone}: local midnight of 2026-11-02"
        );
    });
}

/// Runs the test named `full_name` once per zone, each in a child process
/// with its own TZ.
fn run_in_zones(full_name: &str) {
    let (_, name) = full_name.split_once("::").expect("a test path");
    for zone in ["America/Los_Angeles", "UTC", "Asia/Tokyo"] {
        let result = std::process::Command::new(std::env::current_exe().expect("the test binary"))
            .args(["--exact", name, "--nocapture"])
            .env(ZONE_CHILD, zone)
            .env("TZ", zone)
            .output()
            .expect("the zone child runs");
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(
            result.status.success(),
            "{zone}: {stdout} {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(
            stdout.contains("1 passed; 0 failed"),
            "the {zone} child must run the selected test: {stdout}"
        );
    }
}

#[gpui::test]
async fn dst_days_and_all_day_dates_in_three_zones(cx: &mut gpui::TestAppContext) {
    if let Ok(zone) = std::env::var(ZONE_CHILD) {
        in_zone(cx, &zone).await;
        return;
    }
    run_in_zones(concat!(
        module_path!(),
        "::dst_days_and_all_day_dates_in_three_zones"
    ));
}

/// Fills the open event dialog and saves it.
fn save_in_dialog(
    workspace: &Entity<Workspace>,
    cx: &mut gpui::VisualTestContext,
    title: &str,
    start: &str,
    end: &str,
) {
    workspace.update_in(cx, |workspace, window, cx| {
        assert!(workspace.event_create_visible());
        for (input, value) in [
            (&workspace.event_create_title_input, title),
            (&workspace.event_create_start_input, start),
            (&workspace.event_create_end_input, end),
        ] {
            input.update(cx, |input, cx| {
                input.set_value(value.to_string(), window, cx)
            });
        }
    });
    click_debug_selector(cx, "desktop-event-create-submit");
    cx.run_until_parked();
}

/// What a restart must bring back: an event's identity, wall times and
/// saved instants.
#[derive(Clone, Debug, PartialEq)]
struct SavedEvent {
    id: String,
    title: String,
    all_day: bool,
    start_at: NaiveDateTime,
    end_at: NaiveDateTime,
    start_ms: Option<i64>,
    end_ms: Option<i64>,
}

fn saved_events(
    workspace: &Entity<Workspace>,
    cx: &mut gpui::VisualTestContext,
) -> Vec<SavedEvent> {
    workspace.update_in(cx, |workspace, _window, _cx| {
        workspace
            .calendar_events
            .iter()
            .map(|event| {
                let saved = persisted_calendar_event(event, local_offset_minutes_east);
                SavedEvent {
                    id: event.id.clone(),
                    title: event.title.clone(),
                    all_day: event.all_day,
                    start_at: event.start_at,
                    end_at: event.end_at,
                    start_ms: saved.start_at_ms,
                    end_ms: saved.end_at_ms,
                }
            })
            .collect()
    })
}

/// CAL-02 in one zone: a timed and an all-day event created in the dialog
/// on 2026-11-01 (25 hours long in Los Angeles) come back after a restart;
/// the all-day event spans local midnight to local midnight without
/// spilling into 2026-11-02; both are deleted in the dialog and stay
/// deleted after another restart.
async fn local_events_in_zone(cx: &mut gpui::TestAppContext, zone: &str) {
    init(cx);
    let files = tempfile::tempdir().expect("a temporary directory");
    let path = files.path().join("state.json");
    let mut second_launch = cx.clone();
    let mut third_launch = cx.clone();
    let (workspace, cx) = launch(cx, &path);
    let date = NaiveDate::from_ymd_opt(2026, 11, 1).expect("a date");
    show(&workspace, cx, DesktopCalendarMode::Day, date);
    click_debug_selector(cx, "desktop-calendar-new-event");
    cx.run_until_parked();
    save_in_dialog(
        &workspace,
        cx,
        "Planning",
        "2026-11-01T10:00",
        "2026-11-01T11:30",
    );
    click_debug_selector(cx, "desktop-calendar-new-event");
    cx.run_until_parked();
    click_debug_selector(cx, "desktop-event-create-all-day");
    cx.run_until_parked();
    save_in_dialog(
        &workspace,
        cx,
        "Offsite",
        "2026-11-01T09:00",
        "2026-11-01T10:00",
    );
    saved(&workspace, cx).await;
    let created = saved_events(&workspace, cx);
    assert_eq!(created.len(), 2, "{zone}: {created:?}");
    let offsite = created
        .iter()
        .find(|event| event.title == "Offsite")
        .expect("the all-day event");
    assert!(offsite.all_day, "{zone}: all day");
    assert_eq!(
        offsite.start_ms,
        Some(local_ms(date, 0)),
        "{zone}: local midnight"
    );
    let next_midnight = local_ms(date.succ_opt().expect("a date"), 0);
    assert_eq!(offsite.end_ms, Some(next_midnight), "{zone}");
    if zone == "America/Los_Angeles" {
        assert_eq!(
            next_midnight - local_ms(date, 0),
            25 * HOUR_MS,
            "the long day"
        );
    }
    let planning = created
        .iter()
        .find(|event| event.title == "Planning")
        .expect("the timed event");
    assert_eq!(planning.start_ms, Some(local_ms(date, 10)), "{zone}");
    assert_eq!(
        planning.end_ms,
        Some(local_ms(date, 11) + HOUR_MS / 2),
        "{zone}"
    );
    quit(&workspace, cx);

    let (restarted, cx) = launch(&mut second_launch, &path);
    assert_eq!(
        saved_events(&restarted, cx),
        created,
        "{zone}: a restart keeps them"
    );
    show(&restarted, cx, DesktopCalendarMode::Month, date);
    assert_rendered_bounds(cx, "desktop-calendar-day-open-2026-11-01");
    assert_no_rendered_bounds(cx, "desktop-calendar-day-open-2026-11-02");
    assert_no_rendered_bounds(cx, "desktop-calendar-day-open-2026-10-31");
    for event in &created {
        click_debug_selector(cx, "desktop-calendar-day-open-2026-11-01");
        cx.run_until_parked();
        let row = leaked_debug_selector(format!(
            "desktop-calendar-day-list-event-{}",
            crate::views::workspace::element_key(&event.id)
        ));
        click_debug_selector(cx, row);
        cx.run_until_parked();
        click_debug_selector(cx, "desktop-event-create-delete");
        cx.run_until_parked();
    }
    saved(&restarted, cx).await;
    assert!(saved_events(&restarted, cx).is_empty(), "{zone}: deleted");
    quit(&restarted, cx);

    let (third, cx) = launch(&mut third_launch, &path);
    assert!(saved_events(&third, cx).is_empty(), "{zone}: still deleted");
}

#[gpui::test]
async fn local_events_survive_restarts_in_three_zones(cx: &mut gpui::TestAppContext) {
    if let Ok(zone) = std::env::var(ZONE_CHILD) {
        local_events_in_zone(cx, &zone).await;
        return;
    }
    run_in_zones(concat!(
        module_path!(),
        "::local_events_survive_restarts_in_three_zones"
    ));
}
