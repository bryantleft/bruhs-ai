//! Enums the desktop state file stores as their `as_str` strings.
//!
//! Reading is tolerant: a string this build does not know reads as the value
//! the desktop has always fallen back to, so one stale or hand-edited field
//! never makes the whole file unreadable. Writing produces the same strings
//! earlier builds wrote.

use bruhs_models::agent::{PermissionMode, Role};
use bruhs_models::{AppView, CalendarMode, Priority, SidebarTab, TaskViewMode, WorkItemStatus};
use serde::{Deserialize, Deserializer, Serializer};

pub(super) trait PersistedEnum: Sized {
    /// The string written to the state file.
    fn persisted_str(&self) -> &'static str;
    /// `raw` read back from the state file; unknown strings fall back.
    fn from_persisted(raw: &str) -> Self;
}

impl PersistedEnum for PermissionMode {
    fn persisted_str(&self) -> &'static str {
        self.as_str()
    }

    /// Unknown modes ask first.
    fn from_persisted(raw: &str) -> Self {
        Self::parse_lossy(raw)
    }
}

impl PersistedEnum for SidebarTab {
    fn persisted_str(&self) -> &'static str {
        self.as_str()
    }

    fn from_persisted(raw: &str) -> Self {
        Self::from_str(raw)
    }
}

impl PersistedEnum for TaskViewMode {
    fn persisted_str(&self) -> &'static str {
        self.as_str()
    }

    fn from_persisted(raw: &str) -> Self {
        Self::from_str(raw)
    }
}

impl PersistedEnum for CalendarMode {
    fn persisted_str(&self) -> &'static str {
        self.as_str()
    }

    /// Unknown modes read as the month grid, the calendar's start view.
    fn from_persisted(raw: &str) -> Self {
        match raw {
            "day" => Self::Day,
            "week" => Self::Week,
            _ => Self::Month,
        }
    }
}

impl PersistedEnum for AppView {
    fn persisted_str(&self) -> &'static str {
        self.as_str()
    }

    /// Top-level views read back as themselves. Detail views were saved
    /// without their record id, so they (and unknown names) read as Chat.
    fn from_persisted(raw: &str) -> Self {
        match raw {
            "today" => Self::Today,
            "inbox" => Self::Inbox,
            "knowledge" => Self::Knowledge,
            "tasks" => Self::Tasks,
            "calendar" => Self::Calendar,
            "life" => Self::Life,
            "automations" => Self::Automations,
            "meetings" => Self::Meetings,
            "people" => Self::People,
            "settings" => Self::Settings,
            "search" => Self::Search,
            "terminal" => Self::Terminal,
            _ => Self::Chat,
        }
    }
}

impl PersistedEnum for WorkItemStatus {
    fn persisted_str(&self) -> &'static str {
        self.as_str()
    }

    fn from_persisted(raw: &str) -> Self {
        raw.parse().unwrap_or(Self::Backlog)
    }
}

impl PersistedEnum for Priority {
    fn persisted_str(&self) -> &'static str {
        self.as_str()
    }

    fn from_persisted(raw: &str) -> Self {
        raw.parse().unwrap_or(Self::None)
    }
}

impl PersistedEnum for Role {
    fn persisted_str(&self) -> &'static str {
        self.as_str()
    }

    /// Unknown roles read as assistant text.
    fn from_persisted(raw: &str) -> Self {
        Self::parse_lossy(raw)
    }
}

/// `#[serde(with = "persisted_enum::required")]` for a `T: PersistedEnum`.
pub(super) mod required {
    use super::*;

    pub(in crate::views::workspace) fn serialize<T: PersistedEnum, S: Serializer>(
        value: &T,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(value.persisted_str())
    }

    pub(in crate::views::workspace) fn deserialize<'de, T: PersistedEnum, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<T, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(T::from_persisted(&raw))
    }
}

/// `#[serde(with = "persisted_enum::optional")]` for an `Option<T>`; `null`
/// stays `None`.
pub(super) mod optional {
    use super::*;

    pub(in crate::views::workspace) fn serialize<T: PersistedEnum, S: Serializer>(
        value: &Option<T>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(value) => serializer.serialize_some(value.persisted_str()),
            None => serializer.serialize_none(),
        }
    }

    pub(in crate::views::workspace) fn deserialize<'de, T: PersistedEnum, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<T>, D::Error> {
        let raw = Option::<String>::deserialize(deserializer)?;
        Ok(raw.as_deref().map(T::from_persisted))
    }
}

#[cfg(test)]
mod tests {
    use super::{PermissionMode, PersistedEnum, Priority, Role, WorkItemStatus};
    use bruhs_models::{AppView, CalendarMode, SidebarTab, TaskViewMode};

    fn round_trips<T: PersistedEnum + PartialEq + std::fmt::Debug>(values: &[T]) {
        for value in values {
            assert_eq!(&T::from_persisted(value.persisted_str()), value);
        }
    }

    #[test]
    fn every_variant_reads_back_as_itself() {
        round_trips(PermissionMode::all());
        round_trips(&[SidebarTab::Chat, SidebarTab::Code]);
        round_trips(&[TaskViewMode::Board, TaskViewMode::List]);
        round_trips(&[CalendarMode::Day, CalendarMode::Week, CalendarMode::Month]);
        round_trips(&[
            AppView::Today,
            AppView::Inbox,
            AppView::Knowledge,
            AppView::Chat,
            AppView::Tasks,
            AppView::Calendar,
            AppView::Life,
            AppView::Automations,
            AppView::Meetings,
            AppView::People,
            AppView::Settings,
            AppView::Search,
            AppView::Terminal,
        ]);
        round_trips(&[
            WorkItemStatus::Backlog,
            WorkItemStatus::Todo,
            WorkItemStatus::InProgress,
            WorkItemStatus::Done,
            WorkItemStatus::Canceled,
        ]);
        round_trips(&[
            Priority::None,
            Priority::Urgent,
            Priority::High,
            Priority::Medium,
            Priority::Low,
        ]);
        round_trips(&[Role::User, Role::Assistant, Role::Tool]);
    }

    #[test]
    fn unknown_strings_fall_back_like_earlier_builds() {
        assert_eq!(
            PermissionMode::from_persisted("yolo"),
            PermissionMode::Default
        );
        assert_eq!(SidebarTab::from_persisted("Chat"), SidebarTab::Chat);
        assert_eq!(TaskViewMode::from_persisted(""), TaskViewMode::List);
        assert_eq!(CalendarMode::from_persisted("year"), CalendarMode::Month);
        assert_eq!(AppView::from_persisted("meeting_detail"), AppView::Chat);
        assert_eq!(
            WorkItemStatus::from_persisted("blocked"),
            WorkItemStatus::Backlog
        );
        assert_eq!(Priority::from_persisted("p0"), Priority::None);
        assert_eq!(Role::from_persisted("system"), Role::Assistant);
    }
}
