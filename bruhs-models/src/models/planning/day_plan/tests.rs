use super::*;
use crate::{
    propose_day_plan, propose_day_plan_in_window, Priority, WorkItemPlanning, WorkItemStatus,
};

const DAY: i64 = 1_800_000_000_000;
const NOW: i64 = DAY + 8 * 60 * 60 * 1_000;

fn window() -> DayPlanWindow {
    DayPlanWindow {
        start_at_ms: EpochMs::saturating_from_i64(DAY + 9 * 3_600_000).as_f64(),
        end_at_ms: EpochMs::saturating_from_i64(DAY + 18 * 3_600_000).as_f64(),
    }
}

fn task(id: &str) -> WorkItem {
    let mut task = WorkItem::new_at(format!("Task {id}"), NOW - 1_000);
    task.id = id.into();
    task.status = WorkItemStatus::Todo;
    task.priority = Priority::High;
    task
}

fn proposal(tasks: &[WorkItem]) -> DayPlanProposal {
    propose_day_plan(
        EpochMs::saturating_from_i64(NOW).as_f64(),
        EpochMs::saturating_from_i64(DAY).as_f64(),
        tasks,
        &[],
    )
}

fn applied() -> PreparedDayPlan {
    let tasks = [task("a")];
    prepare_day_plan(&proposal(&tasks), &tasks, &[], window(), NOW).unwrap()
}

#[test]
fn apply_and_undo_change_only_plan_owned_fields() {
    let original = task("a");
    let prepared = applied();
    assert_eq!(prepared.changed_tasks.len(), 1);
    assert_eq!(prepared.added_events.len(), 1);
    assert_eq!(prepared.changed_tasks[0].title, original.title);
    let undo = prepare_day_plan_undo(
        &prepared.ownership,
        &prepared.changed_tasks,
        &prepared.added_events,
        NOW + 1_000,
    )
    .unwrap();
    assert_eq!(
        undo.removed_event_ids,
        [prepared.added_events[0].id.clone()]
    );
    let mut expected = original;
    expected.updated_at = NOW + 1_000;
    assert_eq!(undo.changed_tasks, [expected]);
}

#[test]
fn stale_apply_cannot_resurrect_a_deleted_task() {
    let plan = proposal(&[task("a")]);
    assert_eq!(
        prepare_day_plan(&plan, &[], &[], window(), NOW).unwrap_err(),
        DayPlanChangeError::TaskMissing
    );
}

#[test]
fn stale_apply_refuses_completed_waiting_and_newly_blocked_tasks() {
    let original = task("a");
    let plan = proposal(std::slice::from_ref(&original));
    let mut completed = original.clone();
    completed.status = WorkItemStatus::Done;
    let mut waiting = original.clone();
    waiting.planning.waiting_on = Some("Avery".into());
    let mut blocked = original;
    blocked.planning.dependency_ids.push("missing".into());
    for changed in [completed, waiting, blocked] {
        assert_eq!(
            prepare_day_plan(&plan, &[changed], &[], window(), NOW).unwrap_err(),
            DayPlanChangeError::TaskNotReady
        );
    }
}

#[test]
fn changed_title_and_estimate_require_a_new_review() {
    let original = task("a");
    let plan = proposal(std::slice::from_ref(&original));
    let mut renamed = original.clone();
    renamed.title = "Different reviewed meaning".into();
    let mut resized = original;
    resized.planning.estimate_minutes = Some(90);
    for changed in [renamed, resized] {
        assert_eq!(
            prepare_day_plan(&plan, &[changed], &[], window(), NOW).unwrap_err(),
            DayPlanChangeError::TaskChanged
        );
    }
}

#[test]
fn apply_preserves_newer_unrelated_task_fields() {
    let original = task("a");
    let plan = proposal(std::slice::from_ref(&original));
    let mut newer = original;
    newer.description = Some("Newer owner description".into());
    newer.planning.cycle = Some("New cycle".into());
    newer.updated_at = NOW + 10_000;
    let prepared =
        prepare_day_plan(&plan, std::slice::from_ref(&newer), &[], window(), NOW).unwrap();
    let mut expected = newer;
    expected
        .planning
        .time_block_event_ids
        .push(plan.blocks[0].id.clone());
    assert_eq!(prepared.changed_tasks, [expected]);
}

#[test]
fn a_conflict_in_the_second_block_refuses_the_whole_plan() {
    let tasks = [task("a"), task("b")];
    let plan = proposal(&tasks);
    let mut independent = proposed_event(&plan.blocks[1], NOW + 1_000);
    independent.id = "independent-owner-event".into();
    independent.title = "New commitment".into();
    assert_eq!(
        prepare_day_plan(&plan, &tasks, &[independent], window(), NOW).unwrap_err(),
        DayPlanChangeError::CalendarChanged
    );
}

#[test]
fn replay_cannot_duplicate_a_calendar_block() {
    let prepared = applied();
    let tasks = [task("a")];
    assert_eq!(
        prepare_day_plan(
            &proposal(&tasks),
            &tasks,
            &prepared.added_events,
            window(),
            NOW,
        )
        .unwrap_err(),
        DayPlanChangeError::AlreadyApplied
    );
}

#[test]
fn duplicate_or_overlapping_proposal_blocks_are_refused() {
    let tasks = [task("a"), task("b")];
    let mut plan = proposal(&tasks);
    plan.blocks[1].start_at_ms = plan.blocks[0].start_at_ms;
    plan.blocks[1].end_at_ms = plan.blocks[0].end_at_ms;
    assert_eq!(
        prepare_day_plan(&plan, &tasks, &[], window(), NOW).unwrap_err(),
        DayPlanChangeError::CalendarChanged
    );
    plan.blocks[1].id = plan.blocks[0].id.clone();
    assert_eq!(
        prepare_day_plan(&plan, &tasks, &[], window(), NOW).unwrap_err(),
        DayPlanChangeError::DuplicateBlock
    );
}

#[test]
fn undo_preserves_newer_status_title_description_and_planning_edits() {
    let prepared = applied();
    let mut newer = prepared.changed_tasks[0].clone();
    newer.title = "Owner rename".into();
    newer.description = Some("Owner revision".into());
    newer.status = WorkItemStatus::Done;
    newer.priority = Priority::Urgent;
    newer.due_date = Some("2026-11-14".into());
    newer.planning.cycle = Some("Current cycle".into());
    newer.planning.estimate_minutes = Some(120);
    newer.planning.waiting_on = Some("Zoë".into());
    newer.planning.dependency_ids.push("dependency".into());
    newer
        .planning
        .time_block_event_ids
        .push("manual-block".into());
    newer.updated_at = NOW + 60_000;
    let mut expected = newer.clone();
    expected.planning.time_block_event_ids = vec!["manual-block".into()];
    let undo = prepare_day_plan_undo(
        &prepared.ownership,
        &[newer],
        &prepared.added_events,
        NOW + 30_000,
    )
    .unwrap();
    assert_eq!(undo.changed_tasks, [expected]);
}

#[test]
fn undo_preserves_independent_newer_events() {
    let prepared = applied();
    let mut independent = prepared.added_events[0].clone();
    independent.id = "independent-owner-block".into();
    independent.title = "Owner event".into();
    let events = [prepared.added_events[0].clone(), independent.clone()];
    let undo = prepare_day_plan_undo(
        &prepared.ownership,
        &prepared.changed_tasks,
        &events,
        NOW + 1_000,
    )
    .unwrap();
    assert_eq!(
        undo.removed_event_ids,
        [prepared.added_events[0].id.clone()]
    );
    assert!(!undo.removed_event_ids.contains(&independent.id));
}

#[test]
fn edited_generated_event_requires_review_and_no_partial_undo() {
    let prepared = applied();
    let mut edited = prepared.added_events[0].clone();
    edited.notes = Some("New owner notes".into());
    assert_eq!(
        prepare_day_plan_undo(
            &prepared.ownership,
            &prepared.changed_tasks,
            &[edited],
            NOW + 1_000,
        )
        .unwrap_err(),
        DayPlanChangeError::GeneratedEventChanged
    );
}

#[test]
fn a_new_task_link_to_a_generated_event_prevents_its_deletion() {
    let prepared = applied();
    let mut other = task("another-owner");
    other
        .planning
        .time_block_event_ids
        .push(prepared.added_events[0].id.clone());
    let tasks = [prepared.changed_tasks[0].clone(), other];
    assert_eq!(
        prepare_day_plan_undo(
            &prepared.ownership,
            &tasks,
            &prepared.added_events,
            NOW + 1_000,
        )
        .unwrap_err(),
        DayPlanChangeError::GeneratedLinkChanged
    );
}

#[test]
fn missing_generated_event_can_be_reconciled_without_recreating_it() {
    let prepared = applied();
    let undo = prepare_day_plan_undo(
        &prepared.ownership,
        &prepared.changed_tasks,
        &[],
        NOW + 1_000,
    )
    .unwrap();
    assert!(undo.removed_event_ids.is_empty());
    assert_eq!(undo.changed_tasks[0].planning, WorkItemPlanning::default());
    let replay =
        prepare_day_plan_undo(&prepared.ownership, &undo.changed_tasks, &[], NOW + 2_000).unwrap();
    assert!(replay.changed_tasks.is_empty());
    assert!(replay.removed_event_ids.is_empty());
}

#[test]
fn invalid_clock_window_and_block_are_rejected() {
    let tasks = [task("a")];
    let mut plan = proposal(&tasks);
    assert_eq!(
        prepare_day_plan(&plan, &tasks, &[], window(), i64::MAX).unwrap_err(),
        DayPlanChangeError::InvalidClock
    );
    assert_eq!(
        prepare_day_plan(
            &plan,
            &tasks,
            &[],
            DayPlanWindow {
                start_at_ms: f64::NAN,
                end_at_ms: window().end_at_ms
            },
            NOW,
        )
        .unwrap_err(),
        DayPlanChangeError::InvalidWindow
    );
    plan.blocks[0].end_at_ms = f64::INFINITY;
    assert_eq!(
        prepare_day_plan(&plan, &tasks, &[], window(), NOW).unwrap_err(),
        DayPlanChangeError::InvalidBlock
    );
}

#[test]
fn review_that_outlives_its_first_start_time_requires_a_new_plan() {
    let tasks = [task("a")];
    let plan = proposal(&tasks);
    let start = EpochMs::try_from(plan.blocks[0].start_at_ms)
        .unwrap()
        .as_millis();
    assert!(prepare_day_plan(&plan, &tasks, &[], window(), start).is_ok());
    assert_eq!(
        prepare_day_plan(&plan, &tasks, &[], window(), start + 1).unwrap_err(),
        DayPlanChangeError::InvalidBlock
    );
}

mod workday {
    use super::*;

    struct WorkdayFixture {
        name: &'static str,
        midnight: f64,
        now: f64,
        window: DayPlanWindow,
        legacy_differs: bool,
    }

    include!("workday_fixtures.rs");

    fn full_day_tasks(now: f64) -> Vec<WorkItem> {
        (0..6)
            .map(|index| {
                let mut item = WorkItem::new_at(
                    format!("Focus task {index}"),
                    EpochMs::try_from(now).unwrap().as_millis(),
                );
                item.id = format!("workday-{index}");
                item.priority = Priority::High;
                item.status = WorkItemStatus::Todo;
                item.planning.estimate_minutes = Some(90);
                item
            })
            .collect()
    }

    #[test]
    fn explicit_workday_keeps_nine_to_eighteen_on_both_dst_transition_dates() {
        for fixture in FIXTURES.iter().filter(|item| item.legacy_differs) {
            let tasks = full_day_tasks(fixture.now);
            let plan =
                propose_day_plan_in_window(fixture.now, fixture.window, &tasks, &[]).unwrap();
            assert_eq!(plan.blocks.len(), 6, "{}", fixture.name);
            assert_eq!(
                plan.blocks.first().unwrap().start_at_ms,
                fixture.window.start_at_ms,
                "{}",
                fixture.name
            );
            assert_eq!(
                plan.blocks.last().unwrap().end_at_ms,
                fixture.window.end_at_ms,
                "{}",
                fixture.name
            );
            assert!(plan.blocks.iter().all(|block| {
                block.start_at_ms >= fixture.window.start_at_ms
                    && block.end_at_ms <= fixture.window.end_at_ms
                    && block.end_at_ms - block.start_at_ms == 90.0 * MINUTE_MS
            }));
            let legacy = crate::propose_day_plan(fixture.now, fixture.midnight, &tasks, &[]);
            assert_ne!(
                legacy.blocks.first().unwrap().start_at_ms,
                plan.blocks.first().unwrap().start_at_ms,
                "the captured original planner must reproduce the recorded defect"
            );
            assert_ne!(
                legacy.blocks.last().unwrap().end_at_ms,
                plan.blocks.last().unwrap().end_at_ms,
                "the captured original planner must reproduce the recorded defect"
            );
        }
    }

    #[test]
    fn adjacent_normal_dates_and_utc_keep_existing_proposal_behavior() {
        for fixture in FIXTURES.iter().filter(|item| !item.legacy_differs) {
            let tasks = full_day_tasks(fixture.now);
            let actual =
                propose_day_plan_in_window(fixture.now, fixture.window, &tasks, &[]).unwrap();
            let previous = crate::propose_day_plan(fixture.now, fixture.midnight, &tasks, &[]);
            assert_eq!(actual, previous, "{}", fixture.name);
        }
    }

    #[test]
    fn busy_events_and_explicit_bounds_still_limit_available_work() {
        let fixture = &FIXTURES[1];
        let tasks = full_day_tasks(fixture.now);
        let first = ProposedTimeBlock {
            id: "owner-event".into(),
            task_id: "independent".into(),
            task_title: "Independent commitment".into(),
            start_at_ms: fixture.window.start_at_ms,
            end_at_ms: fixture.window.start_at_ms + 30.0 * MINUTE_MS,
        };
        let event = proposed_event(&first, EpochMs::try_from(fixture.now).unwrap().as_millis());
        let plan =
            propose_day_plan_in_window(fixture.now, fixture.window, &tasks, &[event]).unwrap();
        assert_eq!(plan.blocks.len(), 5);
        assert_eq!(plan.unscheduled_count, 1);
        assert_eq!(plan.blocks[0].start_at_ms, first.end_at_ms);
        assert!(plan
            .blocks
            .iter()
            .all(|block| block.end_at_ms <= fixture.window.end_at_ms));
    }

    #[test]
    fn an_elapsed_workday_has_no_blocks_and_keeps_unscheduled_work_visible() {
        let fixture = &FIXTURES[1];
        let tasks = full_day_tasks(fixture.now);
        let plan = propose_day_plan_in_window(
            fixture.window.end_at_ms + 60_000.0,
            fixture.window,
            &tasks,
            &[],
        )
        .unwrap();
        assert!(plan.blocks.is_empty());
        assert_eq!(plan.unscheduled_count, 6);
        assert_eq!(plan.unscheduled_task_ids.len(), 6);
    }

    #[test]
    fn invalid_time_inputs_are_explicit_errors() {
        let fixture = &FIXTURES[1];
        let tasks = full_day_tasks(fixture.now);
        assert_eq!(
            propose_day_plan_in_window(f64::NAN, fixture.window, &tasks, &[]).unwrap_err(),
            DayPlanChangeError::InvalidClock
        );
        assert_eq!(
            propose_day_plan_in_window(
                fixture.now,
                DayPlanWindow {
                    start_at_ms: fixture.window.end_at_ms,
                    end_at_ms: fixture.window.start_at_ms
                },
                &tasks,
                &[],
            )
            .unwrap_err(),
            DayPlanChangeError::InvalidWindow
        );
    }
}
