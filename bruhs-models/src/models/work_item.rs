//! Tasks. Presentation (glyphs, Tailwind classes, GPUI colours) lives in the
//! UI crates; this module only holds the data and its string forms.

use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::WorkItemPlanning;
use crate::UnknownVariant;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorkItemStatus {
    Backlog,
    Todo,
    InProgress,
    Done,
    Canceled,
}

impl WorkItemStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Backlog => "Backlog",
            Self::Todo => "Todo",
            Self::InProgress => "In Progress",
            Self::Done => "Done",
            Self::Canceled => "Canceled",
        }
    }

    pub fn is_completed(&self) -> bool {
        matches!(self, Self::Done | Self::Canceled)
    }

    pub fn all() -> &'static [WorkItemStatus] {
        &[
            Self::Backlog,
            Self::Todo,
            Self::InProgress,
            Self::Done,
            Self::Canceled,
        ]
    }

    /// Subset rendered as columns on the kanban board (Canceled is hidden).
    pub fn board_columns() -> &'static [WorkItemStatus] {
        &[Self::Backlog, Self::Todo, Self::InProgress, Self::Done]
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Backlog => "backlog",
            Self::Todo => "todo",
            Self::InProgress => "in_progress",
            Self::Done => "done",
            Self::Canceled => "canceled",
        }
    }
}

/// Parses the [`WorkItemStatus::as_str`] form (`"in_progress"`, …).
impl FromStr for WorkItemStatus {
    type Err = UnknownVariant;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::all()
            .iter()
            .copied()
            .find(|status| status.as_str() == s)
            .ok_or_else(|| UnknownVariant::new("task status", s))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Priority {
    None,
    Urgent,
    High,
    Medium,
    Low,
}

impl Priority {
    pub fn label(&self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Urgent => "Urgent",
            Self::High => "High",
            Self::Medium => "Medium",
            Self::Low => "Low",
        }
    }

    /// Number of filled bars (out of 4) for the bars-style PriorityIcon.
    pub fn bars(&self) -> u8 {
        match self {
            Self::Urgent => 4,
            Self::High => 3,
            Self::Medium => 2,
            Self::Low => 1,
            Self::None => 0,
        }
    }

    pub fn all() -> &'static [Priority] {
        &[
            Self::None,
            Self::Urgent,
            Self::High,
            Self::Medium,
            Self::Low,
        ]
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Urgent => "urgent",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }
}

/// Parses the [`Priority::as_str`] form (`"urgent"`, …).
impl FromStr for Priority {
    type Err = UnknownVariant;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::all()
            .iter()
            .copied()
            .find(|priority| priority.as_str() == s)
            .ok_or_else(|| UnknownVariant::new("task priority", s))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkItem {
    pub id: String,
    pub project_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub status: WorkItemStatus,
    pub priority: Priority,
    /// `YYYY-MM-DD`, kept as a string so the IndexedDB wire shape stays
    /// trivially serializable from JS without timezone games.
    pub due_date: Option<String>,
    #[serde(default)]
    pub planning: WorkItemPlanning,
    pub created_at: i64,
    pub updated_at: i64,
}

impl WorkItem {
    /// A new backlog task stamped with the system clock. Native only:
    /// `SystemTime::now()` panics on `wasm32-unknown-unknown`, where callers
    /// use [`WorkItem::new_at`] with the browser clock.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(title: impl Into<String>) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .map_or(0, |elapsed| {
                i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
            });
        Self::new_at(title, now)
    }

    /// Construct with an explicit Unix timestamp in milliseconds. Browser
    /// callers supply their platform clock instead of unsupported SystemTime.
    pub fn new_at(title: impl Into<String>, now: i64) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            project_id: None,
            title: title.into(),
            description: None,
            status: WorkItemStatus::Backlog,
            priority: Priority::None,
            due_date: None,
            planning: WorkItemPlanning::default(),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(
            self.status,
            WorkItemStatus::Todo | WorkItemStatus::InProgress
        )
    }

    pub fn is_completed(&self) -> bool {
        matches!(self.status, WorkItemStatus::Done | WorkItemStatus::Canceled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_clock_preserves_timestamp_and_new_task_defaults() {
        let task = WorkItem::new_at("Local task", 1_790_841_600_123);
        assert_eq!(task.created_at, 1_790_841_600_123);
        assert_eq!(task.updated_at, task.created_at);
        assert_eq!(task.title, "Local task");
        assert_eq!(task.status, WorkItemStatus::Backlog);
        assert_eq!(task.priority, Priority::None);
        assert_eq!(task.planning, WorkItemPlanning::default());
        assert!(task.project_id.is_none());
        assert!(task.description.is_none());
        assert!(task.due_date.is_none());
        assert!(Uuid::parse_str(&task.id).is_ok());
        assert_ne!(task.id, WorkItem::new_at("Local task", task.created_at).id);
    }

    #[test]
    fn status_and_priority_parse_their_string_forms() {
        for status in WorkItemStatus::all() {
            assert_eq!(status.as_str().parse::<WorkItemStatus>(), Ok(*status));
        }
        for priority in Priority::all() {
            assert_eq!(priority.as_str().parse::<Priority>(), Ok(*priority));
        }
        assert_eq!(
            "InProgress".parse::<WorkItemStatus>(),
            Err(UnknownVariant::new("task status", "InProgress"))
        );
        assert!("".parse::<Priority>().is_err());
    }

    #[test]
    fn legacy_task_gets_empty_planning_metadata() {
        let task: WorkItem = serde_json::from_str(
            r#"{"id":"t","project_id":null,"title":"Legacy","description":null,"status":"Todo","priority":"None","due_date":null,"created_at":0,"updated_at":0}"#,
        )
        .unwrap();
        assert_eq!(task.planning, WorkItemPlanning::default());
    }
}
