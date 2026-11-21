use crate::{
    prepare_day_plan_undo, prepare_day_plan_with_availability, propose_day_plan_with_availability,
    AvailabilityRule, CalendarEvent, DayPlanChangeError, DayPlanWindow, EpochMs, WorkItem,
    WorkItemSource,
};

const MONDAY: i64 = 1_794_787_200_000;
const MINUTE: i64 = 60_000;

fn instant(minutes: i64) -> f64 {
    EpochMs::new(MONDAY + minutes * MINUTE).unwrap().as_f64()
}
fn day() -> DayPlanWindow {
    DayPlanWindow {
        start_at_ms: instant(0),
        end_at_ms: instant(1440),
    }
}
fn task(minutes: u16) -> WorkItem {
    let mut task = WorkItem::new_at("Meeting action", MONDAY);
    task.planning.estimate_minutes = Some(minutes);
    task.source = Some(WorkItemSource::Meeting {
        meeting_id: "original-recording".into(),
        title: "Planning".into(),
    });
    task
}

#[test]
fn availability_closing_time_and_disabled_weekdays_leave_work_unscheduled() {
    let tasks = [task(60)];
    let rules = [AvailabilityRule::weekday("Hours", 540, 1020)];
    for now in [instant(990), instant(1440 * 5 + 480)] {
        let window = if now > day().end_at_ms {
            DayPlanWindow {
                start_at_ms: instant(1440 * 5),
                end_at_ms: instant(1440 * 6),
            }
        } else {
            day()
        };
        let plan =
            propose_day_plan_with_availability(now, window, &tasks, &[], &rules, |_| 0).unwrap();
        assert!(plan.blocks.is_empty());
        assert_eq!(plan.unscheduled_task_ids, [tasks[0].id.clone()]);
    }
}

#[test]
fn availability_handles_split_adjacent_early_and_late_rules() {
    for (minutes, now, rules, expected) in [
        (
            90,
            480,
            vec![
                AvailabilityRule::weekday("Morning", 540, 600),
                AvailabilityRule::weekday("Afternoon", 840, 960),
            ],
            (840, 930),
        ),
        (
            90,
            480,
            vec![
                AvailabilityRule::weekday("First", 540, 600),
                AvailabilityRule::weekday("Second", 600, 660),
            ],
            (540, 630),
        ),
        (
            45,
            360,
            vec![AvailabilityRule::weekday("Early", 420, 480)],
            (420, 465),
        ),
        (
            60,
            1110,
            vec![AvailabilityRule::weekday("Late", 1140, 1260)],
            (1140, 1200),
        ),
    ] {
        let plan = propose_day_plan_with_availability(
            instant(now),
            day(),
            &[task(minutes)],
            &[],
            &rules,
            |_| 0,
        )
        .unwrap();
        assert_eq!(plan.blocks.len(), 1);
        assert_eq!(
            (plan.blocks[0].start_at_ms, plan.blocks[0].end_at_ms),
            (instant(expected.0), instant(expected.1))
        );
        assert_eq!(
            plan.busy_blocks_respected, 0,
            "working-hour gaps are not calendar commitments"
        );
    }
}

#[test]
fn availability_apply_rechecks_current_hours_and_preserves_source_through_undo() {
    let tasks = [task(60)];
    let rules = [AvailabilityRule::weekday("Hours", 540, 1020)];
    let plan = propose_day_plan_with_availability(instant(480), day(), &tasks, &[], &rules, |_| 0)
        .unwrap();
    let changed = [AvailabilityRule::weekday("Changed", 540, 570)];
    assert_eq!(
        prepare_day_plan_with_availability(&plan, &tasks, &[], day(), MONDAY, &changed, |_| 0)
            .unwrap_err(),
        DayPlanChangeError::AvailabilityChanged
    );
    assert!(tasks[0].planning.time_block_event_ids.is_empty());
    let applied =
        prepare_day_plan_with_availability(&plan, &tasks, &[], day(), MONDAY, &rules, |_| 0)
            .unwrap();
    assert_eq!(applied.changed_tasks[0].source, tasks[0].source);
    let mut latest = applied.changed_tasks;
    latest[0].title = "Owner's later title".into();
    latest[0].description = Some("Owner's later context".into());
    let undone = prepare_day_plan_undo(
        &applied.ownership,
        &latest,
        &applied.added_events,
        MONDAY + MINUTE,
    )
    .unwrap();
    assert_eq!(undone.changed_tasks[0].source, tasks[0].source);
    assert_eq!(undone.changed_tasks[0].title, latest[0].title);
    assert_eq!(undone.changed_tasks[0].description, latest[0].description);
    assert!(undone.changed_tasks[0]
        .planning
        .time_block_event_ids
        .is_empty());
}

#[test]
fn availability_respects_all_day_time_off_in_review_and_at_apply() {
    let tasks = [task(45)];
    let original_tasks = tasks.clone();
    let rules = [AvailabilityRule::weekday("Hours", 540, 1020)];
    let events = [CalendarEvent {
        id: "time-off".into(),
        title: "Time off".into(),
        start_at_ms: day().start_at_ms,
        end_at_ms: day().end_at_ms,
        all_day: true,
        color: None,
        notes: None,
        created_at: MONDAY,
        updated_at: MONDAY,
        source: Default::default(),
        google_event_id: None,
        calendar_id: None,
        attendees: Vec::new(),
        organizer: None,
        location: None,
        conferencing: None,
        recurring_event_id: None,
        html_link: None,
    }];
    let original_events = events.clone();
    let available =
        propose_day_plan_with_availability(instant(480), day(), &tasks, &[], &rules, |_| 0)
            .unwrap();
    assert_eq!(available.blocks.len(), 1);
    let unavailable =
        propose_day_plan_with_availability(instant(480), day(), &tasks, &events, &rules, |_| 0)
            .unwrap();
    assert!(unavailable.blocks.is_empty());
    assert_eq!(unavailable.unscheduled_task_ids, [tasks[0].id.clone()]);
    assert_eq!(unavailable.busy_blocks_respected, 1);
    assert_eq!(
        prepare_day_plan_with_availability(
            &available,
            &tasks,
            &events,
            day(),
            MONDAY,
            &rules,
            |_| 0
        )
        .unwrap_err(),
        DayPlanChangeError::CalendarChanged
    );
    assert_eq!(tasks, original_tasks);
    assert_eq!(events, original_events);
}

#[test]
fn availability_rejects_malformed_enabled_rules_and_non_daily_searches() {
    let invalid = AvailabilityRule::weekday("Invalid", 600, 500);
    assert_eq!(
        propose_day_plan_with_availability(
            instant(480),
            day(),
            &[task(45)],
            &[],
            &[invalid],
            |_| 0
        )
        .unwrap_err(),
        DayPlanChangeError::InvalidAvailability
    );
    let mut invalid_end = AvailabilityRule::weekday("Invalid end", 600, 700);
    invalid_end.end_minute = 1441;
    assert!(!invalid_end.is_valid());
    let too_broad = DayPlanWindow {
        start_at_ms: instant(0),
        end_at_ms: instant(1440 * 365),
    };
    assert_eq!(
        propose_day_plan_with_availability(instant(480), too_broad, &[task(45)], &[], &[], |_| 0)
            .unwrap_err(),
        DayPlanChangeError::InvalidWindow
    );
    assert_eq!(
        propose_day_plan_with_availability(instant(480), day(), &[task(45)], &[], &[], |_| 0)
            .unwrap()
            .blocks
            .len(),
        0
    );
}

#[test]
fn availability_observes_offset_changes_inside_a_focus_block() {
    for (start, end, transition, before, after, minimum, maximum, minutes, expected) in [
        // 01:30 to 03:30 has only one elapsed hour at spring-forward.
        (
            1_772_956_800_000,
            1_773_039_600_000,
            1_772_964_000_000,
            -480,
            -420,
            90,
            210,
            90,
            None,
        ),
        (
            1_772_956_800_000,
            1_773_039_600_000,
            1_772_964_000_000,
            -480,
            -420,
            90,
            210,
            60,
            Some((1_772_962_200_000, 1_772_965_800_000)),
        ),
        // The repeated 01:00 to 01:30 is outside the configured 01:30–02:00.
        (
            1_793_516_400_000,
            1_793_606_400_000,
            1_793_523_600_000,
            -420,
            -480,
            90,
            120,
            45,
            None,
        ),
    ] {
        let rule = AvailabilityRule {
            weekdays: vec![0],
            ..AvailabilityRule::weekday("Sunday", minimum, maximum)
        };
        let window = DayPlanWindow {
            start_at_ms: EpochMs::new(start).unwrap().as_f64(),
            end_at_ms: EpochMs::new(end).unwrap().as_f64(),
        };
        let plan = propose_day_plan_with_availability(
            window.start_at_ms,
            window,
            &[task(minutes)],
            &[],
            &[rule],
            |at| if at < transition { before } else { after },
        )
        .unwrap();
        assert_eq!(
            plan.blocks
                .first()
                .map(|block| (block.start_at_ms, block.end_at_ms)),
            expected.map(|(start, end)| (
                EpochMs::new(start).unwrap().as_f64(),
                EpochMs::new(end).unwrap().as_f64()
            ))
        );
    }
}
