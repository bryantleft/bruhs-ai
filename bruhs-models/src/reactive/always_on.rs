//! Records that keep a reactive agent useful between runs: what it told the
//! owner (outbox), what it remembers, and the follow-ups it scheduled.

use crate::EpochMs;
use serde::{Deserialize, Serialize};

/// Agent-written memory entries kept per agent.
pub const MAX_MEMORY_ENTRIES: usize = 40;
/// One memory entry's text budget.
pub const MAX_MEMORY_ENTRY_CHARS: usize = 600;
/// All of one agent's written memory, as loaded into a run.
pub const MAX_MEMORY_CHARS: usize = 8_000;
/// Pending follow-ups per agent.
pub const MAX_FOLLOW_UPS: usize = 20;
/// The furthest a follow-up may be scheduled.
pub const MAX_FOLLOW_UP_MS: i64 = 366 * 24 * 60 * 60 * 1000;
/// Queued runs for one agent that a single turn may handle together.
pub const MAX_BATCH_RUNS: usize = 20;
/// Agents that may run at the same time; one run per agent.
pub const MAX_LANES: usize = 3;
/// A session whose context passes this many tokens restarts fresh next run.
pub const ROLLOVER_CONTEXT_TOKENS: u64 = 120_000;
/// Recent owner-facing messages repeated to a fresh session so it does not repeat itself.
pub const RECENT_TOLD: usize = 5;
/// Memory keys: short stable slugs the agent reuses to update an entry.
pub const MAX_KEY_CHARS: usize = 64;

/// What a new event does while its agent is already running.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BusyPolicy {
    /// Wait; queued events for the agent run together next.
    #[default]
    Queue,
    /// Stop the current run and start again with its events plus the new one.
    Restart,
}

impl BusyPolicy {
    pub fn is_queue(&self) -> bool {
        *self == Self::Queue
    }
}

/// Why a run exists. A trigger type this build doesn't know (one a newer
/// service added) reads as [`RunTrigger::Other`], so the run, and the
/// snapshot that lists it, still decode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunTrigger {
    Event,
    Chat,
    FollowUp {
        follow_up_id: String,
    },
    Answer {
        outbox_id: String,
    },
    DraftOutcome {
        run_id: String,
    },
    Consolidation,
    Room {
        room_id: String,
        message_id: String,
        #[serde(default)]
        hop: u32,
        /// A run the owner's decision resumed; its prompt is not rebuilt.
        #[serde(default)]
        resumed: bool,
    },
    /// A trigger from a newer service. Never written by this build.
    #[serde(other)]
    Other,
}

impl RunTrigger {
    /// The room a room run belongs to.
    #[must_use]
    pub fn room_id(&self) -> Option<&str> {
        match self {
            Self::Room { room_id, .. } => Some(room_id),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Event => "Event",
            Self::Chat => "Message",
            Self::FollowUp { .. } => "Follow-up",
            Self::Answer { .. } => "Your answer",
            Self::DraftOutcome { .. } => "Draft decision",
            Self::Consolidation => "Memory tidy-up",
            Self::Room { .. } => "Room",
            Self::Other => "Run",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboxKind {
    Notify,
    Ask,
}

/// Delivery of an outbox item as a system notification, outside the app.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SystemDelivery {
    /// System notifications were off for this kind when the item was created.
    #[default]
    NotRequested,
    Pending,
    Delivered {
        at: EpochMs,
    },
    /// Read in the app before a notification was needed.
    Skipped,
    Failed {
        error: String,
    },
}

/// One message an agent sent the owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxItem {
    pub id: String,
    pub agent_id: String,
    pub run_id: String,
    pub kind: OutboxKind,
    pub title: String,
    pub message: String,
    pub created_at: EpochMs,
    #[serde(default)]
    pub read_at: Option<EpochMs>,
    /// A question answered or dismissed, or an update dismissed.
    #[serde(default)]
    pub resolved_at: Option<EpochMs>,
    #[serde(default)]
    pub answer: Option<String>,
    #[serde(default)]
    pub system: SystemDelivery,
    /// A tool call waiting on the owner's decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<crate::crew::ApprovalRequest>,
}

impl OutboxItem {
    pub fn is_open_approval(&self) -> bool {
        self.approval.is_some() && self.resolved_at.is_none()
    }

    pub fn is_unread(&self) -> bool {
        self.read_at.is_none() && self.resolved_at.is_none()
    }

    pub fn is_open_question(&self) -> bool {
        self.kind == OutboxKind::Ask && self.resolved_at.is_none()
    }
}

/// The live-stream summary clients use for badges and new-update banners.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxSignal {
    pub unread: usize,
    pub open_questions: usize,
    /// The newest unread item, so a client can show it without a full snapshot.
    #[serde(default)]
    pub latest_unread: Option<OutboxItem>,
    #[serde(default)]
    pub bruhs: Vec<crate::crew::BruhState>,
    #[serde(default)]
    pub rooms: Vec<crate::crew::room::RoomState>,
    #[serde(default)]
    pub protocol: u32,
    /// When a run last changed. A run can start and end between two frames,
    /// leaving every bruh's state as it was, so a client reads the crew again
    /// when this moves too. An older service sends none.
    #[serde(default)]
    pub runs_at: Option<crate::EpochMs>,
}

/// What a frame says about the outbox: its counts and the newest unread
/// item. A bruh's second approval in one run can leave every bruh's state
/// and `runs_at` as they were (it still needs you), so a crew page reads
/// the crew again when this changes too, or the new card never shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutboxMark {
    pub unread: usize,
    pub open_questions: usize,
    pub latest: Option<String>,
}

impl OutboxSignal {
    #[must_use]
    pub fn outbox_mark(&self) -> OutboxMark {
        OutboxMark {
            unread: self.unread,
            open_questions: self.open_questions,
            latest: self.latest_unread.as_ref().map(|item| item.id.clone()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    /// Facts, preferences and corrections about the owner or the work.
    Note,
    /// Where a multi-run task stands: what is done and what remains.
    Progress,
}

impl MemoryKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Note => "Note",
            Self::Progress => "Progress",
        }
    }
}

/// One entry of an agent's own memory, with where it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub agent_id: String,
    pub key: String,
    pub kind: MemoryKind,
    pub text: String,
    pub updated_at: EpochMs,
    /// The run that wrote it; `None` when the owner edited it.
    #[serde(default)]
    pub run_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FollowUpStatus {
    Pending,
    Fired,
    Cancelled,
}

/// A deadline an agent set for itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FollowUp {
    pub id: String,
    pub agent_id: String,
    pub key: String,
    pub note: String,
    pub due_at: EpochMs,
    pub status: FollowUpStatus,
    pub created_at: EpochMs,
    #[serde(default)]
    pub run_id: Option<String>,
    /// Whether the follow-up's run starts without review, like the run that set it.
    #[serde(default)]
    pub auto_run: bool,
    /// The run it started, once fired.
    #[serde(default)]
    pub fired_run_id: Option<String>,
}

/// A daily window in which system notifications wait, in a named time zone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuietHours {
    /// `HH:MM`, inclusive.
    pub start: String,
    /// `HH:MM`, exclusive. Earlier than `start` means the window crosses midnight.
    pub end: String,
    pub time_zone: String,
}

impl QuietHours {
    /// Whether a local minute of the day (0..1440) falls in the window.
    pub fn contains_minute(&self, minute: u16) -> Option<bool> {
        let start = parse_hhmm(&self.start)?;
        let end = parse_hhmm(&self.end)?;
        Some(if start == end {
            false
        } else if start < end {
            (start..end).contains(&minute)
        } else {
            minute >= start || minute < end
        })
    }
}

/// Under the notification switches on Agent workflows, on both apps.
pub const NOTIFICATIONS_NOTE: &str = "Sent by the local server, so they arrive while the app is closed when the background service is on. Off by default.";

/// How agent updates reach the owner outside the app.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotificationSettings {
    /// System notifications for updates. Off by default: agents start quiet.
    #[serde(default)]
    pub system_notify: bool,
    /// System notifications for questions.
    #[serde(default)]
    pub system_ask: bool,
    #[serde(default)]
    pub quiet_hours: Option<QuietHours>,
}

impl NotificationSettings {
    pub fn wants_system(&self, kind: OutboxKind) -> bool {
        match kind {
            OutboxKind::Notify => self.system_notify,
            OutboxKind::Ask => self.system_ask,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftOutcomeKind {
    Applied,
    Declined,
    Failed,
    RolledBack,
}

impl DraftOutcomeKind {
    pub fn from_workflow(status: crate::WorkflowStatus) -> Option<Self> {
        use crate::WorkflowStatus;
        match status {
            WorkflowStatus::Completed => Some(Self::Applied),
            WorkflowStatus::Declined => Some(Self::Declined),
            WorkflowStatus::Failed => Some(Self::Failed),
            WorkflowStatus::RolledBack => Some(Self::RolledBack),
            _ => None,
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::Applied => "approved and applied",
            Self::Declined => "declined",
            Self::Failed => "approved, but applying it failed",
            Self::RolledBack => "applied and then rolled back",
        }
    }
}

/// Minutes after midnight for a `HH:MM` 24-hour time.
pub fn parse_hhmm(text: &str) -> Option<u16> {
    let (hours, minutes) = text.split_once(':')?;
    if hours.len() != 2 || minutes.len() != 2 {
        return None;
    }
    let hours: u16 = hours.parse().ok()?;
    let minutes: u16 = minutes.parse().ok()?;
    (hours < 24 && minutes < 60).then_some(hours * 60 + minutes)
}

/// A memory or follow-up key: 1–64 of `a-z 0-9 - _ .`.
pub fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.chars().count() <= MAX_KEY_CHARS
        && key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'))
}

/// Shape checks for a calendar schedule; the time zone name is checked by the runtime.
pub fn calendar_shape_error(times: &[String], weekdays: &[u8], time_zone: &str) -> Option<String> {
    if times.is_empty() || times.len() > 24 {
        return Some("Choose between one and 24 times of day.".into());
    }
    if let Some(bad) = times.iter().find(|time| parse_hhmm(time).is_none()) {
        return Some(format!("“{bad}” is not a 24-hour HH:MM time."));
    }
    if weekdays.iter().any(|day| !(1..=7).contains(day)) {
        return Some("Weekdays are 1 (Monday) through 7 (Sunday).".into());
    }
    if time_zone.trim().is_empty() {
        return Some("Choose a time zone, such as America/Los_Angeles.".into());
    }
    None
}

pub const WEEKDAY_NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// "Weekdays at 08:00 (Europe/Paris)" style summary of a calendar schedule.
pub fn describe_calendar(times: &[String], weekdays: &[u8], time_zone: &str) -> String {
    let mut days: Vec<u8> = weekdays.to_vec();
    days.sort_unstable();
    days.dedup();
    let days = match days.as_slice() {
        [] | [1, 2, 3, 4, 5, 6, 7] => "Every day".to_owned(),
        [1, 2, 3, 4, 5] => "Weekdays".to_owned(),
        [6, 7] => "Weekends".to_owned(),
        days => days
            .iter()
            .filter_map(|day| WEEKDAY_NAMES.get(usize::from(*day).checked_sub(1)?))
            .copied()
            .collect::<Vec<_>>()
            .join(", "),
    };
    format!("{days} at {} ({time_zone})", times.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_hours_cross_midnight_and_reject_bad_times() {
        let hours = QuietHours {
            start: "22:00".into(),
            end: "07:30".into(),
            time_zone: "UTC".into(),
        };
        assert_eq!(hours.contains_minute(23 * 60), Some(true));
        assert_eq!(hours.contains_minute(7 * 60), Some(true));
        assert_eq!(hours.contains_minute(7 * 60 + 30), Some(false));
        assert_eq!(hours.contains_minute(12 * 60), Some(false));
        let day = QuietHours {
            start: "09:00".into(),
            end: "17:00".into(),
            time_zone: "UTC".into(),
        };
        assert_eq!(day.contains_minute(8 * 60 + 59), Some(false));
        assert_eq!(day.contains_minute(9 * 60), Some(true));
        let bad = QuietHours {
            start: "9:00".into(),
            ..day
        };
        assert_eq!(bad.contains_minute(0), None);
    }

    #[test]
    fn keys_and_calendar_shapes_are_bounded() {
        assert!(valid_key("invoice-42.status"));
        assert!(!valid_key("Invoice"));
        assert!(!valid_key(""));
        assert!(!valid_key(&"a".repeat(65)));
        assert!(calendar_shape_error(&["08:00".into()], &[1, 5], "UTC").is_none());
        assert!(calendar_shape_error(&[], &[], "UTC").is_some());
        assert!(calendar_shape_error(&["24:00".into()], &[], "UTC").is_some());
        assert!(calendar_shape_error(&["08:00".into()], &[0], "UTC").is_some());
        assert_eq!(
            describe_calendar(&["08:00".into()], &[5, 4, 3, 2, 1], "Europe/Paris"),
            "Weekdays at 08:00 (Europe/Paris)"
        );
        assert_eq!(
            describe_calendar(&["18:30".into()], &[2, 4], "UTC"),
            "Tue, Thu at 18:30 (UTC)"
        );
    }
}
