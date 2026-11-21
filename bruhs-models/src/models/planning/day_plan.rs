//! Validated day-plan mutations and ownership-aware undo.

use super::{CalendarEvent, DayPlanProposal, EpochMs, ProposedTimeBlock, WorkItem};
use crate::EventSource;
use std::collections::{HashMap, HashSet};
use std::fmt;

mod availability;
pub(super) use availability::unavailable_time;

const MINUTE_MS: f64 = 60_000.0;

/// Platform callers resolve these boundaries from local calendar fields.
/// Adding elapsed hours to local midnight is incorrect across DST transitions.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DayPlanWindow {
    pub start_at_ms: f64,
    pub end_at_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DayPlanChangeError {
    InvalidClock,
    InvalidWindow,
    Empty,
    InvalidBlock,
    DuplicateTask,
    DuplicateBlock,
    DuplicateStoredRecord,
    TaskMissing,
    TaskNotReady,
    TaskChanged,
    CalendarChanged,
    AlreadyApplied,
    GeneratedEventChanged,
    GeneratedLinkChanged,
    AvailabilityChanged,
    InvalidAvailability,
    AvailabilityUnavailable,
}

impl fmt::Display for DayPlanChangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidClock | Self::InvalidWindow | Self::InvalidBlock => {
                "The plan's times are no longer valid. Review a new day plan."
            }
            Self::Empty => "There are no focus blocks to apply. Review a new day plan.",
            Self::DuplicateTask | Self::DuplicateBlock | Self::DuplicateStoredRecord => {
                "The plan contains conflicting records. Review a new day plan."
            }
            Self::TaskMissing => "A planned task was deleted. Review a new day plan.",
            Self::TaskNotReady => "A planned task is no longer ready. Review a new day plan.",
            Self::TaskChanged => "A planned task changed. Review a new day plan.",
            Self::CalendarChanged => "Your calendar changed. Review a new day plan.",
            Self::AlreadyApplied => "This plan already has calendar blocks. Review your calendar.",
            Self::GeneratedEventChanged => {
                "A focus block changed after planning. Review it before undoing the plan."
            }
            Self::GeneratedLinkChanged => {
                "A focus block was linked to another task. Review it before undoing the plan."
            }
            Self::AvailabilityChanged => "Working hours changed. Review a new day plan.",
            Self::InvalidAvailability => "An enabled working-hours rule is invalid. Fix scheduling controls, then review a new day plan.",
            Self::AvailabilityUnavailable => "Working hours could not be read. Open scheduling controls, then review a new day plan.",
        })
    }
}

impl std::error::Error for DayPlanChangeError {}

/// Only plan-owned event content and task/event relationships are retained.
/// An undo never needs an old full task snapshot.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DayPlanOwnership {
    events: Vec<CalendarEvent>,
    links: Vec<(String, String)>,
}

#[derive(Debug)]
pub struct PreparedDayPlan {
    pub changed_tasks: Vec<WorkItem>,
    pub added_events: Vec<CalendarEvent>,
    pub ownership: DayPlanOwnership,
}

#[derive(Debug)]
pub struct PreparedDayPlanUndo {
    pub changed_tasks: Vec<WorkItem>,
    pub removed_event_ids: Vec<String>,
}

fn current_clock(now_ms: i64) -> Result<f64, DayPlanChangeError> {
    EpochMs::new(now_ms)
        .map(EpochMs::as_f64)
        .map_err(|_| DayPlanChangeError::InvalidClock)
}

pub(super) fn valid_interval(start: f64, end: f64) -> bool {
    EpochMs::try_from(start).is_ok() && EpochMs::try_from(end).is_ok() && end > start
}

type TaskIndex<'a> = HashMap<&'a str, &'a WorkItem>;
type EventIndex<'a> = HashMap<&'a str, &'a CalendarEvent>;

fn indexes<'a>(
    tasks: &'a [WorkItem],
    events: &'a [CalendarEvent],
) -> Result<(TaskIndex<'a>, EventIndex<'a>), DayPlanChangeError> {
    let by_task: HashMap<_, _> = tasks.iter().map(|task| (task.id.as_str(), task)).collect();
    let by_event: HashMap<_, _> = events
        .iter()
        .map(|event| (event.id.as_str(), event))
        .collect();
    if by_task.len() != tasks.len() || by_event.len() != events.len() {
        return Err(DayPlanChangeError::DuplicateStoredRecord);
    }
    Ok((by_task, by_event))
}

fn proposed_event(block: &ProposedTimeBlock, now_ms: i64) -> CalendarEvent {
    CalendarEvent {
        id: block.id.clone(),
        title: format!("Focus: {}", block.task_title),
        start_at_ms: block.start_at_ms,
        end_at_ms: block.end_at_ms,
        all_day: false,
        color: Some("persimmon".into()),
        notes: Some(format!("Planned from task {}", block.task_id)),
        created_at: now_ms,
        updated_at: now_ms,
        source: EventSource::Local,
        google_event_id: None,
        calendar_id: None,
        attendees: Vec::new(),
        organizer: None,
        location: None,
        conferencing: None,
        recurring_event_id: None,
        html_link: None,
    }
}

/// Prepare against the latest durable records, not the review-time snapshots.
/// No input is mutated; any failed precondition rejects the entire operation.
pub fn prepare_day_plan(
    proposal: &DayPlanProposal,
    tasks: &[WorkItem],
    events: &[CalendarEvent],
    window: DayPlanWindow,
    now_ms: i64,
) -> Result<PreparedDayPlan, DayPlanChangeError> {
    let now = current_clock(now_ms)?;
    if !valid_interval(window.start_at_ms, window.end_at_ms) || now >= window.end_at_ms {
        return Err(DayPlanChangeError::InvalidWindow);
    }
    if proposal.blocks.is_empty() {
        return Err(DayPlanChangeError::Empty);
    }
    let (by_task, by_event) = indexes(tasks, events)?;
    let completed: HashSet<_> = tasks
        .iter()
        .filter(|task| task.is_completed())
        .map(|task| task.id.as_str())
        .collect();
    let mut task_ids = HashSet::new();
    let mut block_ids = HashSet::new();
    let mut intervals = Vec::with_capacity(proposal.blocks.len());
    let mut changed_tasks = Vec::with_capacity(proposal.blocks.len());
    let mut added_events = Vec::with_capacity(proposal.blocks.len());
    let mut links = Vec::with_capacity(proposal.blocks.len());
    for block in &proposal.blocks {
        if block.id.is_empty()
            || block.task_id.is_empty()
            || !valid_interval(block.start_at_ms, block.end_at_ms)
            || block.start_at_ms < window.start_at_ms
            || block.end_at_ms > window.end_at_ms
            || block.start_at_ms < now
        {
            return Err(DayPlanChangeError::InvalidBlock);
        }
        if !task_ids.insert(block.task_id.as_str()) {
            return Err(DayPlanChangeError::DuplicateTask);
        }
        if !block_ids.insert(block.id.as_str()) {
            return Err(DayPlanChangeError::DuplicateBlock);
        }
        if by_event.contains_key(block.id.as_str()) {
            return Err(DayPlanChangeError::AlreadyApplied);
        }
        let task = by_task
            .get(block.task_id.as_str())
            .ok_or(DayPlanChangeError::TaskMissing)?;
        if task.is_completed()
            || task.planning.waiting_on.is_some()
            || !task.planning.time_block_event_ids.is_empty()
            || task
                .planning
                .dependency_ids
                .iter()
                .any(|id| !completed.contains(id.as_str()))
        {
            return Err(DayPlanChangeError::TaskNotReady);
        }
        let expected_minutes =
            (f64::from(task.planning.estimate_minutes.unwrap_or(45).clamp(15, 240)) / 15.0).ceil()
                * 15.0;
        if task.title != block.task_title
            || block.end_at_ms - block.start_at_ms != expected_minutes * MINUTE_MS
        {
            return Err(DayPlanChangeError::TaskChanged);
        }
        if events.iter().any(|event| {
            !valid_interval(event.start_at_ms, event.end_at_ms)
                || (block.start_at_ms < event.end_at_ms && block.end_at_ms > event.start_at_ms)
        }) || intervals
            .iter()
            .any(|&(start, end)| block.start_at_ms < end && block.end_at_ms > start)
        {
            return Err(DayPlanChangeError::CalendarChanged);
        }
        intervals.push((block.start_at_ms, block.end_at_ms));
        let mut changed = (*task).clone();
        changed.planning.time_block_event_ids.push(block.id.clone());
        changed.updated_at = changed.updated_at.max(now_ms);
        changed_tasks.push(changed);
        added_events.push(proposed_event(block, now_ms));
        links.push((block.task_id.clone(), block.id.clone()));
    }
    let ownership = DayPlanOwnership {
        events: added_events.clone(),
        links,
    };
    Ok(PreparedDayPlan {
        changed_tasks,
        added_events,
        ownership,
    })
}

/// Validate current working hours as well as task/calendar state before applying.
/// No task or calendar input is changed when any reviewed block is unavailable.
pub fn prepare_day_plan_with_availability(
    proposal: &DayPlanProposal,
    tasks: &[WorkItem],
    events: &[CalendarEvent],
    window: DayPlanWindow,
    now_ms: i64,
    rules: &[crate::AvailabilityRule],
    offset_at: impl Fn(i64) -> i32 + Copy,
) -> Result<PreparedDayPlan, DayPlanChangeError> {
    let unavailable = unavailable_time(window, rules, offset_at)?;
    if proposal.blocks.iter().any(|block| {
        unavailable
            .iter()
            .any(|&(start, end)| block.start_at_ms < end && block.end_at_ms > start)
    }) {
        return Err(DayPlanChangeError::AvailabilityChanged);
    }
    prepare_day_plan(proposal, tasks, events, window, now_ms)
}

/// Timestamps do not establish ownership: they can change without changing
/// event content, and native projection does not retain them. Every persisted
/// user-editable field and source identity must otherwise remain unchanged.
fn same_owned_event(current: &CalendarEvent, generated: &CalendarEvent) -> bool {
    let mut expected = generated.clone();
    expected.created_at = current.created_at;
    expected.updated_at = current.updated_at;
    current == &expected
}

/// Reverse only the plan-owned links and unchanged generated events. A newer
/// unrelated task/planning edit is merged from current storage, never replaced.
pub fn prepare_day_plan_undo(
    ownership: &DayPlanOwnership,
    tasks: &[WorkItem],
    events: &[CalendarEvent],
    now_ms: i64,
) -> Result<PreparedDayPlanUndo, DayPlanChangeError> {
    current_clock(now_ms)?;
    let (_, by_event) = indexes(tasks, events)?;
    let owned_ids: HashSet<_> = ownership
        .events
        .iter()
        .map(|event| event.id.as_str())
        .collect();
    let owned_links: HashSet<_> = ownership
        .links
        .iter()
        .map(|(task, event)| (task.as_str(), event.as_str()))
        .collect();
    for task in tasks {
        for id in &task.planning.time_block_event_ids {
            if owned_ids.contains(id.as_str())
                && !owned_links.contains(&(task.id.as_str(), id.as_str()))
            {
                return Err(DayPlanChangeError::GeneratedLinkChanged);
            }
        }
    }
    let mut removed_event_ids = Vec::with_capacity(ownership.events.len());
    for generated in &ownership.events {
        if let Some(current) = by_event.get(generated.id.as_str()) {
            if !same_owned_event(current, generated) {
                return Err(DayPlanChangeError::GeneratedEventChanged);
            }
            removed_event_ids.push(generated.id.clone());
        }
    }
    let mut changed_tasks = Vec::new();
    for task in tasks {
        if !task
            .planning
            .time_block_event_ids
            .iter()
            .any(|id| owned_links.contains(&(task.id.as_str(), id.as_str())))
        {
            continue;
        }
        let mut changed = task.clone();
        changed
            .planning
            .time_block_event_ids
            .retain(|id| !owned_links.contains(&(changed.id.as_str(), id.as_str())));
        changed.updated_at = changed.updated_at.max(now_ms);
        changed_tasks.push(changed);
    }
    Ok(PreparedDayPlanUndo {
        changed_tasks,
        removed_event_ids,
    })
}

#[cfg(test)]
mod tests;
