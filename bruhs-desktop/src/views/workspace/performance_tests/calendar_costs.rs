use super::{fixture, print_samples, tasks};
use crate::views::workspace::{
    at_hour, calendar_month_days, event_overlaps_day, month_grid_start, work_item_due_on, AppView,
    DesktopCalendarEvent, DesktopCalendarMode, Workspace, DESKTOP_DATE_FORMAT,
};
use bruhs_models::{num::ToFloatLossy, CalendarEvent as RemoteEvent};
use chrono::{Duration, Local, NaiveDate, TimeZone};
use gpui::{Context, TestAppContext};
use std::{hint::black_box, time::Instant};

mod legacy;

fn event(id: &str, date: NaiveDate, hour: u32) -> DesktopCalendarEvent {
    DesktopCalendarEvent {
        id: id.into(),
        title: format!("Event {id}"),
        start_at: at_hour(date, hour),
        end_at: at_hour(date, hour) + Duration::minutes(30),
        all_day: false,
        color: Some("persimmon".into()),
        notes: Some("Complete saved event context. ".repeat(40)),
        ..Default::default()
    }
}

fn remote(id: &str, date: NaiveDate, hour: u32) -> RemoteEvent {
    let start = Local
        .from_local_datetime(&at_hour(date, hour))
        .single()
        .unwrap()
        .timestamp_millis();
    serde_json::from_value(serde_json::json!({
        "id": id, "title":format!("Remote {id}"),
        "start_at_ms":start.to_f64_lossy(), "end_at_ms":(start + 1_800_000).to_f64_lossy(),
        "all_day":false, "color":"pandan", "notes":"Remote event context. ".repeat(40),
        "created_at":0, "updated_at":0, "source":bruhs_models::EventSource::Google,
    }))
    .unwrap()
}

#[gpui::test]
fn event_merge_keeps_local_precedence_first_valid_remote_and_all_fields(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    let date = NaiveDate::from_ymd_opt(2026, 5, 12).unwrap();
    workspace.update(cx, |workspace, cx| {
        workspace.calendar_events = vec![event("local", date, 9), event("local", date, 10)];
        let mut invalid = remote("later-valid", date, 11);
        invalid.start_at_ms = f64::MAX;
        let mut first = remote("remote", date, 12);
        first.all_day = true;
        first.notes = Some("  exact\n\n🗒  ".into());
        let second = remote("remote", date, 13);
        workspace.meetings_hub.update(cx, |hub, _| {
            hub.google_events = vec![
                remote("local", date, 8),
                invalid,
                first,
                second,
                remote("later-valid", date, 14),
            ];
        });
        let expected = workspace.legacy_visible_calendar_events(cx);
        let actual = workspace.visible_calendar_events(cx);
        assert_eq!(
            actual
                .iter()
                .map(|event| event.id.as_str())
                .collect::<Vec<_>>(),
            ["local", "local", "remote", "later-valid"]
        );
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(&expected) {
            assert_eq!(actual.id, expected.id);
            assert_eq!(actual.title, expected.title);
            assert_eq!(actual.start_at, expected.start_at);
            assert_eq!(actual.end_at, expected.end_at);
            assert_eq!(actual.all_day, expected.all_day);
            assert_eq!(actual.color, expected.color);
            assert_eq!(actual.notes, expected.notes);
        }
    });
}

#[test]
fn month_projection_preserves_boundaries_exact_due_strings_counts_and_source_order() {
    let start = NaiveDate::from_ymd_opt(2026, 4, 26).unwrap();
    let mut events = (0..12)
        .map(|index| {
            event(
                &format!("event-{index}"),
                start + Duration::days(index % 5),
                9,
            )
        })
        .collect::<Vec<_>>();
    events[0].start_at = at_hour(start - Duration::days(1), 9);
    events[0].end_at = at_hour(start + Duration::days(2), 0);
    events[1].all_day = true;
    events[1].start_at = at_hour(start - Duration::days(2), 0);
    events[1].end_at = at_hour(start + Duration::days(44), 0);
    events[2].end_at = events[2].start_at;
    events[3].end_at = events[3].start_at - Duration::hours(1);
    events[4].end_at = events[4].start_at - Duration::days(2);
    let mut tasks = tasks(12);
    for (index, task) in tasks.iter_mut().enumerate() {
        task.due_date = Some(
            (start + Duration::days(i64::try_from(index % 3).unwrap()))
                .format(DESKTOP_DATE_FORMAT)
                .to_string(),
        );
    }
    tasks[0].due_date = Some("2026-4-26".into());
    tasks[1].due_date = None;
    let days = calendar_month_days(&events, &tasks, start);
    assert_eq!(days.len(), 42);
    for day in days {
        let expected_events = events
            .iter()
            .filter(|event| event_overlaps_day(event, day.date))
            .collect::<Vec<_>>();
        let expected_tasks = tasks
            .iter()
            .filter(|task| work_item_due_on(task, day.date))
            .collect::<Vec<_>>();
        assert_eq!(
            day.total_records,
            expected_events.len() + expected_tasks.len()
        );
        assert_eq!(
            day.events
                .iter()
                .map(|event| event.id.as_str())
                .collect::<Vec<_>>(),
            expected_events
                .iter()
                .take(3)
                .map(|event| event.id.as_str())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            day.tasks
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            expected_tasks
                .iter()
                .take(3)
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>()
        );
        for (actual, expected) in day.events.iter().zip(expected_events) {
            assert!(std::ptr::eq(*actual, expected));
        }
        for (actual, expected) in day.tasks.iter().zip(expected_tasks) {
            assert!(std::ptr::eq(*actual, expected));
        }
    }
}

fn populate(workspace: &mut Workspace, count: usize, cx: &mut Context<'_, Workspace>) {
    workspace.sidebar_visible = false;
    workspace.active_view = AppView::Calendar;
    workspace.calendar_anchor = NaiveDate::from_ymd_opt(2026, 5, 12).unwrap();
    let first = month_grid_start(workspace.calendar_anchor);
    workspace.calendar_events = (0..count)
        .map(|index| {
            event(
                &format!("local-{index}"),
                first + Duration::days(i64::try_from(index % 180).unwrap() - 70),
                8 + u32::try_from(index % 6).unwrap(),
            )
        })
        .collect();
    let google_events = (0..count)
        .map(|index| {
            remote(
                &format!("remote-{index}"),
                first + Duration::days(i64::try_from(index % 180).unwrap() - 70),
                8 + u32::try_from(index % 6).unwrap(),
            )
        })
        .collect();
    workspace
        .meetings_hub
        .update(cx, |hub, _| hub.google_events = google_events);
    workspace.work_items = tasks(count);
    for (index, task) in workspace.work_items.iter_mut().enumerate() {
        task.due_date = Some(
            (first + Duration::days(i64::try_from(index % 84).unwrap()))
                .format(DESKTOP_DATE_FORMAT)
                .to_string(),
        );
    }
}

#[gpui::test]
#[ignore = "diagnostic optimized test-layout costs, not GPU/OS input timing"]
fn dense_calendar_action_cost(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    cx.simulate_resize(gpui::size(gpui::px(1800.0), gpui::px(1000.0)));
    for (mode, count) in [
        (DesktopCalendarMode::Month, 100),
        (DesktopCalendarMode::Month, 1_000),
        (DesktopCalendarMode::Month, 5_000),
        (DesktopCalendarMode::Week, 1_000),
        (DesktopCalendarMode::Day, 1_000),
    ] {
        workspace.update(cx, |workspace, cx| {
            populate(workspace, count, cx);
            workspace.calendar_mode = mode;
        });
        for legacy in [true, false] {
            workspace.update(cx, |workspace, cx| {
                workspace.calendar_legacy_preparation.set(legacy);
                cx.notify();
            });
            cx.run_until_parked();
            let samples = (0..11)
                .map(|_| {
                    let start = Instant::now();
                    workspace.update(cx, |_, cx| cx.notify());
                    cx.run_until_parked();
                    start.elapsed()
                })
                .collect();
            print_samples(&format!("calendar redraw / {mode:?} / {count} local + {count} Google + {count} tasks / legacy={legacy}"), samples);
            if count == 1_000 {
                let samples = (0..12)
                    .map(|index| {
                        let start = Instant::now();
                        workspace.update(cx, |workspace, cx| {
                            if index % 2 == 0 {
                                workspace.calendar_next(cx);
                            } else {
                                workspace.calendar_prev(cx);
                            }
                        });
                        cx.run_until_parked();
                        start.elapsed()
                    })
                    .collect();
                print_samples(
                    &format!("calendar navigation / {mode:?} / 1000 each / legacy={legacy}"),
                    samples,
                );
            }
        }
    }
    workspace.read_with(cx, |workspace, cx| black_box(workspace.snapshot(cx)));
}
