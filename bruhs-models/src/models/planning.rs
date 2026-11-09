use std::{cmp::Reverse, collections::HashSet};

use serde::{Deserialize, Serialize};

use crate::EpochMs;

use super::{
    CalendarEvent, Meeting, ObservationKind, PersonProfile, Priority, WorkItem, WorkItemStatus,
};

const MINUTE_MS: f64 = 60_000.0;
const FIFTEEN_MINUTES_MS: f64 = 15.0 * MINUTE_MS;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecurrenceRule {
    Daily,
    Weekdays,
    Weekly,
    Monthly,
}

impl RecurrenceRule {
    pub fn label(self) -> &'static str {
        match self {
            Self::Daily => "Daily",
            Self::Weekdays => "Weekdays",
            Self::Weekly => "Weekly",
            Self::Monthly => "Monthly",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Weekdays => "weekdays",
            Self::Weekly => "weekly",
            Self::Monthly => "monthly",
        }
    }

    #[expect(
        clippy::should_implement_trait,
        reason = "infallible-to-Option parse kept for existing callers; FromStr needs an error type"
    )]
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "daily" => Some(Self::Daily),
            "weekdays" => Some(Self::Weekdays),
            "weekly" => Some(Self::Weekly),
            "monthly" => Some(Self::Monthly),
            _ => None,
        }
    }
}

/// Advance a local `YYYY-MM-DD` key without introducing timezone semantics
/// into the persisted task model.
pub fn next_recurrence_date(date: &str, rule: RecurrenceRule) -> Option<String> {
    let mut parts = date.split('-');
    let year = parts.next()?.parse::<i32>().ok()?;
    let month = parts.next()?.parse::<u32>().ok()?;
    let day = parts.next()?.parse::<u32>().ok()?;
    if parts.next().is_some()
        || !(1..=12).contains(&month)
        || day == 0
        || day > month_days(year, month)
    {
        return None;
    }
    let (year, month, day) = match rule {
        RecurrenceRule::Daily => add_days(year, month, day, 1),
        RecurrenceRule::Weekdays => {
            let mut next = add_days(year, month, day, 1);
            while matches!(weekday(next.0, next.1, next.2), 0 | 6) {
                next = add_days(next.0, next.1, next.2, 1);
            }
            next
        }
        RecurrenceRule::Weekly => add_days(year, month, day, 7),
        RecurrenceRule::Monthly => {
            let (next_year, next_month) = if month == 12 {
                (year + 1, 1)
            } else {
                (year, month + 1)
            };
            (
                next_year,
                next_month,
                day.min(month_days(next_year, next_month)),
            )
        }
    };
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

fn add_days(mut year: i32, mut month: u32, mut day: u32, count: u32) -> (i32, u32, u32) {
    for _ in 0..count {
        day += 1;
        if day > month_days(year, month) {
            day = 1;
            if month == 12 {
                month = 1;
                year += 1;
            } else {
                month += 1;
            }
        }
    }
    (year, month, day)
}

fn month_days(year: i32, month: u32) -> u32 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if year % 400 == 0 || (year % 4 == 0 && year % 100 != 0) => 29,
        2 => 28,
        _ => 31,
    }
}

/// Sunday = 0. Tomohiko Sakamoto's Gregorian calendar algorithm.
fn weekday(mut year: i32, month: u32, day: u32) -> i32 {
    const OFFSETS: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    if month < 3 {
        year -= 1;
    }
    (year + year / 4 - year / 100
        + year / 400
        + OFFSETS[(month - 1) as usize]
        + i32::try_from(day).unwrap_or(0))
        % 7
}

/// Planning metadata stays nested so old local task records deserialize with
/// one default and provider adapters never dictate the core task shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct WorkItemPlanning {
    /// Human-readable cycle label (for example "2026 W37" or "Launch").
    pub cycle: Option<String>,
    /// Task ids that must be complete before this task can enter the focus queue.
    #[serde(default)]
    pub dependency_ids: Vec<String>,
    /// User intent, in minutes. The planner snaps this to 15-minute boundaries.
    pub estimate_minutes: Option<u16>,
    pub recurrence: Option<RecurrenceRule>,
    /// Local calendar event ids created from this task.
    #[serde(default)]
    pub time_block_event_ids: Vec<String>,
    /// A person or external condition this task is waiting on.
    pub waiting_on: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgendaEntry {
    pub event_id: String,
    pub title: String,
    pub start_at_ms: f64,
    pub end_at_ms: f64,
    pub all_day: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FollowUp {
    pub person_id: String,
    pub person_name: String,
    pub text: String,
    pub meeting_id: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecentDecision {
    pub meeting_id: String,
    pub meeting_title: String,
    pub text: String,
    pub decided_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TodaySnapshot {
    pub now: Option<AgendaEntry>,
    pub next: Vec<AgendaEntry>,
    pub focus_queue: Vec<WorkItem>,
    pub overdue: Vec<WorkItem>,
    pub waiting_on: Vec<WorkItem>,
    pub follow_ups: Vec<FollowUp>,
    pub recent_decisions: Vec<RecentDecision>,
    pub briefing: String,
}

impl TodaySnapshot {
    #[expect(
        clippy::too_many_arguments,
        reason = "one input per record collection the snapshot joins; callers pass them positionally"
    )]
    pub fn build(
        now_ms: f64,
        day_start_ms: f64,
        day_end_ms: f64,
        today_key: &str,
        work_items: &[WorkItem],
        events: &[CalendarEvent],
        meetings: &[Meeting],
        people: &[PersonProfile],
    ) -> Self {
        let mut agenda = events
            .iter()
            .filter(|event| event.end_at_ms > day_start_ms && event.start_at_ms < day_end_ms)
            .map(|event| AgendaEntry {
                event_id: event.id.clone(),
                title: event.title.clone(),
                start_at_ms: event.start_at_ms,
                end_at_ms: event.end_at_ms,
                all_day: event.all_day,
            })
            .collect::<Vec<_>>();
        agenda.sort_by(|left, right| left.start_at_ms.total_cmp(&right.start_at_ms));

        let now = agenda
            .iter()
            .find(|entry| !entry.all_day && entry.start_at_ms <= now_ms && entry.end_at_ms > now_ms)
            .cloned();
        let next = agenda
            .into_iter()
            .filter(|entry| entry.all_day || entry.start_at_ms >= now_ms)
            .take(4)
            .collect();

        let completed = work_items
            .iter()
            .filter(|item| item.is_completed())
            .map(|item| item.id.as_str())
            .collect::<HashSet<_>>();
        let active = work_items
            .iter()
            .filter(|item| !item.is_completed())
            .collect::<Vec<_>>();

        let mut overdue = active
            .iter()
            .filter(|item| item.due_date.as_deref().is_some_and(|due| due < today_key))
            .map(|item| (*item).clone())
            .collect::<Vec<_>>();
        sort_focus_items(&mut overdue);

        let mut waiting_on = active
            .iter()
            .filter(|item| {
                item.planning
                    .waiting_on
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
            })
            .map(|item| (*item).clone())
            .collect::<Vec<_>>();
        sort_focus_items(&mut waiting_on);

        let mut focus_queue = active
            .iter()
            .filter(|item| item.planning.waiting_on.is_none())
            .filter(|item| {
                item.planning
                    .dependency_ids
                    .iter()
                    .all(|dependency| completed.contains(dependency.as_str()))
            })
            .map(|item| (*item).clone())
            .collect::<Vec<_>>();
        sort_focus_items(&mut focus_queue);
        focus_queue.truncate(6);

        let lookback_ms: i64 = 14 * 24 * 60 * 60 * 1_000;
        let cutoff = EpochMs::saturating_from_f64(now_ms).as_millis() - lookback_ms;
        let mut follow_ups = people
            .iter()
            .flat_map(|person| {
                person
                    .observations
                    .iter()
                    .filter(|observation| {
                        observation.created_at >= cutoff
                            && observation.kind == ObservationKind::ActionItemOwner
                    })
                    .map(move |observation| FollowUp {
                        person_id: person.id.clone(),
                        person_name: person.display_name.clone(),
                        text: observation.text.clone(),
                        meeting_id: observation.at_meeting_id.clone(),
                        created_at: observation.created_at,
                    })
            })
            .collect::<Vec<_>>();
        follow_ups.sort_by_key(|follow_up| Reverse(follow_up.created_at));
        follow_ups.truncate(6);

        let mut recent_decisions = meetings
            .iter()
            .filter(|meeting| {
                EpochMs::saturating_from_f64(meeting.started_at_ms).as_millis() >= cutoff
            })
            .flat_map(|meeting| {
                meeting.summary.iter().flat_map(move |summary| {
                    summary
                        .decisions
                        .iter()
                        .map(move |decision| RecentDecision {
                            meeting_id: meeting.id.clone(),
                            meeting_title: meeting.title.clone(),
                            text: decision.clone(),
                            decided_at: EpochMs::saturating_from_f64(meeting.started_at_ms)
                                .as_millis(),
                        })
                })
            })
            .collect::<Vec<_>>();
        recent_decisions.sort_by_key(|decision| Reverse(decision.decided_at));
        recent_decisions.truncate(6);

        let briefing = briefing_text(
            now.as_ref(),
            focus_queue.first(),
            overdue.len(),
            waiting_on.len(),
            follow_ups.len(),
        );

        Self {
            now,
            next,
            focus_queue,
            overdue,
            waiting_on,
            follow_ups,
            recent_decisions,
            briefing,
        }
    }
}

fn briefing_text(
    now: Option<&AgendaEntry>,
    focus: Option<&WorkItem>,
    overdue: usize,
    waiting: usize,
    follow_ups: usize,
) -> String {
    let opening = now.map_or_else(
        || "Your calendar is clear right now.".to_string(),
        |event| format!("You are in “{}” right now.", event.title),
    );
    let focus = focus.map_or_else(
        || "There is no ready work in the focus queue.".to_string(),
        |item| format!("Protect the next open block for “{}” first.", item.title),
    );
    let pressure = match (overdue, waiting, follow_ups) {
        (0, 0, 0) => "No overdue work or follow-ups need triage.".to_string(),
        _ => format!(
            "Triage {overdue} overdue, {waiting} waiting, and {follow_ups} follow-up item(s)."
        ),
    };
    format!("{opening} {focus} {pressure}")
}

fn priority_rank(priority: Priority) -> u8 {
    match priority {
        Priority::Urgent => 0,
        Priority::High => 1,
        Priority::Medium => 2,
        Priority::Low => 3,
        Priority::None => 4,
    }
}

fn status_rank(status: WorkItemStatus) -> u8 {
    match status {
        WorkItemStatus::InProgress => 0,
        WorkItemStatus::Todo => 1,
        WorkItemStatus::Backlog => 2,
        WorkItemStatus::Done => 3,
        WorkItemStatus::Canceled => 4,
    }
}

fn sort_focus_items(items: &mut [WorkItem]) {
    items.sort_by(|left, right| {
        status_rank(left.status)
            .cmp(&status_rank(right.status))
            .then_with(|| priority_rank(left.priority).cmp(&priority_rank(right.priority)))
            .then_with(|| left.due_date.cmp(&right.due_date))
            .then_with(|| right.updated_at.cmp(&left.updated_at))
            .then_with(|| left.id.cmp(&right.id))
    });
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProposedTimeBlock {
    pub id: String,
    pub task_id: String,
    pub task_title: String,
    pub start_at_ms: f64,
    pub end_at_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DayPlanProposal {
    pub id: String,
    pub created_at_ms: i64,
    pub blocks: Vec<ProposedTimeBlock>,
    /// Total ready tasks left out of the proposal.
    pub unscheduled_count: usize,
    /// A bounded sample for audit/detail UI; never grows with the whole task store.
    pub unscheduled_task_ids: Vec<String>,
    pub busy_blocks_respected: usize,
}

/// Produce a deterministic local proposal. The caller supplies local-midnight
/// bounds so this pure model remains timezone-independent on web and native.
pub fn propose_day_plan(
    now_ms: f64,
    day_start_ms: f64,
    work_items: &[WorkItem],
    events: &[CalendarEvent],
) -> DayPlanProposal {
    let workday_start = day_start_ms + 9.0 * 60.0 * MINUTE_MS;
    let workday_end = day_start_ms + 18.0 * 60.0 * MINUTE_MS;
    let mut cursor = round_up(now_ms.max(workday_start), FIFTEEN_MINUTES_MS);
    let completed = work_items
        .iter()
        .filter(|item| item.is_completed())
        .map(|item| item.id.as_str())
        .collect::<HashSet<_>>();
    let mut tasks = work_items
        .iter()
        .filter(|item| !item.is_completed())
        .filter(|item| item.planning.waiting_on.is_none())
        .filter(|item| item.planning.time_block_event_ids.is_empty())
        .filter(|item| {
            item.planning
                .dependency_ids
                .iter()
                .all(|dependency| completed.contains(dependency.as_str()))
        })
        .collect::<Vec<_>>();
    tasks.sort_by(|left, right| {
        status_rank(left.status)
            .cmp(&status_rank(right.status))
            .then_with(|| priority_rank(left.priority).cmp(&priority_rank(right.priority)))
            .then_with(|| left.due_date.cmp(&right.due_date))
            .then_with(|| right.updated_at.cmp(&left.updated_at))
            .then_with(|| left.id.cmp(&right.id))
    });

    let mut busy = events
        .iter()
        .filter(|event| !event.all_day)
        .filter(|event| event.end_at_ms > cursor && event.start_at_ms < workday_end)
        .map(|event| {
            (
                event.start_at_ms.max(workday_start),
                event.end_at_ms.min(workday_end),
            )
        })
        .collect::<Vec<_>>();
    busy.sort_by(|left, right| left.0.total_cmp(&right.0));
    let busy_blocks_respected = busy.len();

    let mut blocks = Vec::new();
    let mut unscheduled_task_ids = Vec::new();
    for task in tasks.iter().take(6) {
        let minutes = f64::from(task.planning.estimate_minutes.unwrap_or(45).clamp(15, 240));
        let duration = round_up(minutes * MINUTE_MS, FIFTEEN_MINUTES_MS);
        let Some(start) = first_open_slot(cursor, duration, workday_end, &busy) else {
            unscheduled_task_ids.push(task.id.clone());
            continue;
        };
        let end = start + duration;
        let block_id = format!("plan-{}-{}", EpochMs::saturating_from_f64(now_ms), task.id);
        blocks.push(ProposedTimeBlock {
            id: block_id,
            task_id: task.id.clone(),
            task_title: task.title.clone(),
            start_at_ms: start,
            end_at_ms: end,
        });
        busy.push((start, end));
        busy.sort_by(|left, right| left.0.total_cmp(&right.0));
        cursor = end;
    }
    for task in tasks.iter().take(50) {
        if !blocks.iter().any(|block| block.task_id == task.id)
            && !unscheduled_task_ids.iter().any(|id| id == &task.id)
        {
            unscheduled_task_ids.push(task.id.clone());
        }
    }

    let unscheduled_count = tasks.len().saturating_sub(blocks.len());
    DayPlanProposal {
        id: format!("day-plan-{}", EpochMs::saturating_from_f64(now_ms)),
        created_at_ms: EpochMs::saturating_from_f64(now_ms).as_millis(),
        blocks,
        unscheduled_count,
        unscheduled_task_ids,
        busy_blocks_respected,
    }
}

fn first_open_slot(
    mut cursor: f64,
    duration: f64,
    workday_end: f64,
    busy: &[(f64, f64)],
) -> Option<f64> {
    loop {
        if cursor + duration > workday_end {
            return None;
        }
        let conflict = busy
            .iter()
            .find(|(start, end)| cursor < *end && cursor + duration > *start);
        match conflict {
            Some((_, end)) => cursor = round_up(*end, FIFTEEN_MINUTES_MS),
            None => return Some(cursor),
        }
    }
}

fn round_up(value: f64, quantum: f64) -> f64 {
    (value / quantum).ceil() * quantum
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, title: &str, priority: Priority) -> WorkItem {
        let mut item = WorkItem::new(title);
        item.id = id.to_string();
        item.status = WorkItemStatus::Todo;
        item.priority = priority;
        item
    }

    fn event(id: &str, start_at_ms: f64, end_at_ms: f64) -> CalendarEvent {
        CalendarEvent {
            id: id.to_string(),
            title: "Busy".to_string(),
            start_at_ms,
            end_at_ms,
            all_day: false,
            color: None,
            notes: None,
            created_at: 0,
            updated_at: 0,
            source: Default::default(),
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

    #[test]
    fn proposal_prioritizes_and_avoids_calendar_conflicts() {
        let day = 1_800_000_000_000.0;
        let nine = day + 9.0 * 60.0 * MINUTE_MS;
        let busy = event("meeting", nine + 30.0 * MINUTE_MS, nine + 90.0 * MINUTE_MS);
        let low = task("low", "Later", Priority::Low);
        let mut urgent = task("urgent", "First", Priority::Urgent);
        urgent.planning.estimate_minutes = Some(30);

        let proposal = propose_day_plan(nine, day, &[low, urgent], &[busy]);

        assert_eq!(proposal.blocks.len(), 2);
        assert_eq!(proposal.blocks[0].task_id, "urgent");
        assert_eq!(proposal.blocks[0].start_at_ms, nine);
        assert_eq!(proposal.blocks[1].start_at_ms, nine + 90.0 * MINUTE_MS);
        assert_eq!(proposal.busy_blocks_respected, 1);
    }

    #[test]
    fn proposal_excludes_waiting_blocked_and_already_scheduled_work() {
        let day = 1_800_000_000_000.0;
        let mut done = task("done", "Prerequisite", Priority::None);
        done.status = WorkItemStatus::Done;
        let mut ready = task("ready", "Ready", Priority::High);
        ready.planning.dependency_ids.push("done".to_string());
        let mut blocked = task("blocked", "Blocked", Priority::Urgent);
        blocked.planning.dependency_ids.push("missing".to_string());
        let mut waiting = task("waiting", "Waiting", Priority::Urgent);
        waiting.planning.waiting_on = Some("Alex".to_string());
        let mut scheduled = task("scheduled", "Scheduled", Priority::Urgent);
        scheduled
            .planning
            .time_block_event_ids
            .push("event".to_string());

        let proposal = propose_day_plan(
            day + 9.0 * 60.0 * MINUTE_MS,
            day,
            &[done, ready, blocked, waiting, scheduled],
            &[],
        );

        assert_eq!(proposal.blocks.len(), 1);
        assert_eq!(proposal.blocks[0].task_id, "ready");
    }

    #[test]
    fn recurrence_advances_across_calendar_edges() {
        assert_eq!(
            next_recurrence_date("2026-09-11", RecurrenceRule::Weekdays).as_deref(),
            Some("2026-09-14")
        );
        assert_eq!(
            next_recurrence_date("2024-01-31", RecurrenceRule::Monthly).as_deref(),
            Some("2024-02-29")
        );
        assert_eq!(
            next_recurrence_date("2026-12-31", RecurrenceRule::Daily).as_deref(),
            Some("2027-01-01")
        );
    }

    #[test]
    fn today_snapshot_assembles_the_daily_loop() {
        let day = 1_800_000_000_000.0;
        let now = day + 10.0 * 60.0 * MINUTE_MS;
        let current = event(
            "current",
            day + 9.5 * 60.0 * MINUTE_MS,
            day + 10.5 * 60.0 * MINUTE_MS,
        );
        let next = event(
            "next",
            day + 11.0 * 60.0 * MINUTE_MS,
            day + 12.0 * 60.0 * MINUTE_MS,
        );
        let mut overdue = task("overdue", "Overdue", Priority::Urgent);
        overdue.due_date = Some("2026-09-09".to_string());
        let mut waiting = task("waiting", "Waiting", Priority::High);
        waiting.planning.waiting_on = Some("Sam".to_string());
        let mut blocked = task("blocked", "Blocked", Priority::Urgent);
        blocked
            .planning
            .dependency_ids
            .push("unfinished".to_string());

        let mut person = PersonProfile::new("Sam");
        person.observations.push(super::super::PersonObservation {
            at_meeting_id: "meeting".to_string(),
            kind: ObservationKind::ActionItemOwner,
            text: "Send the launch copy".to_string(),
            created_at: EpochMs::saturating_from_f64(now).as_millis(),
        });
        let mut meeting = Meeting::new("Launch sync", now);
        meeting.summary = Some(super::super::MeetingSummary {
            tldr: String::new(),
            action_items: Vec::new(),
            decisions: vec!["Ship on Friday".to_string()],
            topics: Vec::new(),
            generated_by: "test".to_string(),
            generated_at: EpochMs::saturating_from_f64(now).as_millis(),
        });

        let snapshot = TodaySnapshot::build(
            now,
            day,
            day + 24.0 * 60.0 * MINUTE_MS,
            "2026-09-10",
            &[overdue, waiting, blocked],
            &[current, next],
            &[meeting],
            &[person],
        );

        assert_eq!(
            snapshot.now.as_ref().map(|entry| entry.event_id.as_str()),
            Some("current")
        );
        assert_eq!(snapshot.next.len(), 1);
        assert_eq!(snapshot.overdue.len(), 1);
        assert_eq!(snapshot.waiting_on.len(), 1);
        assert_eq!(snapshot.focus_queue.len(), 1);
        assert_eq!(snapshot.focus_queue[0].id, "overdue");
        assert_eq!(snapshot.follow_ups.len(), 1);
        assert_eq!(snapshot.recent_decisions.len(), 1);
        assert!(snapshot.briefing.contains("Overdue"));
    }
}
