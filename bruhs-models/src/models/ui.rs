//! UI surface enums — small state types shared by every UI surface
//! (web, desktop) so each one doesn't reinvent its own copy.
//!
//! Per-surface signal/context wrappers (Leptos `ReadSignal`, GPUI entities)
//! still live in their respective crates; only the data lives here.

use serde::{Deserialize, Serialize};

/// Top-level surface — the main panel switches between Chat, Tasks, Calendar,
/// Meetings, People, and Settings views.
///
/// `MeetingDetail` carries a `meeting_id` so the view is URL-addressable via
/// `#/meetings/<id>`. `Search` is the cross-meeting transcript search page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppView {
    Today,
    Inbox,
    Knowledge,
    Chat,
    Tasks,
    Calendar,
    Life,
    Automations,
    Meetings,
    People,
    Settings,
    /// Full-width routed detail view for a single meeting.
    MeetingDetail {
        meeting_id: String,
    },
    /// Local text only; never resolves a recording with the same identity.
    MeetingNoteDetail {
        note_id: String,
    },
    DocumentDetail {
        document_id: String,
    },
    ConversationDetail {
        conversation_id: String,
    },
    Booking {
        slug: String,
    },
    ClipDetail {
        clip_id: String,
    },
    /// Cross-meeting transcript search page.
    Search,
    /// Standalone terminals, independent of any chat thread.
    Terminal,
}

impl AppView {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Today => "today",
            Self::Inbox => "inbox",
            Self::Knowledge => "knowledge",
            Self::Chat => "chat",
            Self::Tasks => "tasks",
            Self::Calendar => "calendar",
            Self::Life => "life",
            Self::Automations => "automations",
            Self::Meetings => "meetings",
            Self::People => "people",
            Self::Settings => "settings",
            Self::MeetingDetail { .. } => "meeting_detail",
            Self::MeetingNoteDetail { .. } => "meeting_note_detail",
            Self::DocumentDetail { .. } => "document_detail",
            Self::ConversationDetail { .. } => "conversation_detail",
            Self::Booking { .. } => "booking",
            Self::ClipDetail { .. } => "clip_detail",
            Self::Search => "search",
            Self::Terminal => "terminal",
        }
    }
}

/// Which time grain the calendar surface renders at.
/// - Month: 6×7 calendar grid (default)
/// - Week:  7-column timeslot grid spanning Sun → Sat of the anchor week
/// - Day:   single-column timeslot grid for the anchor day
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CalendarMode {
    Month,
    Week,
    Day,
}

impl CalendarMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Day => "Day",
            Self::Week => "Week",
            Self::Month => "Month",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
        }
    }
}

/// Whether the tasks page renders as a board (Kanban) or a list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskViewMode {
    Board,
    List,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LifeSection {
    #[default]
    Finance,
    Health,
}

impl TaskViewMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Board => "Board",
            Self::List => "List",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Board => "board",
            Self::List => "list",
        }
    }

    #[expect(
        clippy::should_implement_trait,
        reason = "lenient parse with a default for persisted values; FromStr would have to be fallible"
    )]
    pub fn from_str(s: &str) -> Self {
        match s {
            "board" => Self::Board,
            _ => Self::List,
        }
    }
}

/// Workspace mode — general conversations or project-scoped coding threads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SidebarTab {
    Chat,
    Code,
}

impl SidebarTab {
    pub fn matches_project(self, project_name: &str) -> bool {
        match self {
            Self::Chat => project_name.is_empty(),
            Self::Code => !project_name.is_empty(),
        }
    }

    pub fn for_project(project_name: &str) -> Self {
        if project_name.is_empty() {
            Self::Chat
        } else {
            Self::Code
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Chat => "Chat",
            Self::Code => "Code",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            SidebarTab::Chat => "chat",
            SidebarTab::Code => "code",
        }
    }
    #[expect(
        clippy::should_implement_trait,
        reason = "lenient parse with a default for persisted values; FromStr would have to be fallible"
    )]
    pub fn from_str(s: &str) -> Self {
        match s {
            "code" => SidebarTab::Code,
            _ => SidebarTab::Chat,
        }
    }
}

/// Chat mode — Agent (Claude Code with tools) or LLM (plain chat without
/// tool execution). LLM mode reuses the agent endpoint but submits with
/// `permission_mode=plan` so no tools fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChatMode {
    Agent,
    Llm,
}

impl ChatMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChatMode::Agent => "agent",
            ChatMode::Llm => "llm",
        }
    }
    #[expect(
        clippy::should_implement_trait,
        reason = "lenient parse with a default for persisted values; FromStr would have to be fallible"
    )]
    pub fn from_str(s: &str) -> Self {
        match s {
            "llm" => ChatMode::Llm,
            _ => ChatMode::Agent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_modes_partition_general_and_project_threads() {
        for project in ["", "bruhs", "project with spaces"] {
            let mode = SidebarTab::for_project(project);
            assert!(mode.matches_project(project));
            assert_ne!(
                SidebarTab::Chat.matches_project(project),
                SidebarTab::Code.matches_project(project)
            );
        }
    }

    #[test]
    fn appview_meetings_as_str() {
        assert_eq!(AppView::Meetings.as_str(), "meetings");
    }

    #[test]
    fn appview_meetings_roundtrip() {
        let v = AppView::Meetings;
        let json = serde_json::to_string(&v).unwrap();
        let back: AppView = serde_json::from_str(&json).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn appview_meeting_detail_roundtrip() {
        let v = AppView::MeetingDetail {
            meeting_id: "abc-123".into(),
        };
        let json = serde_json::to_string(&v).unwrap();
        let back: AppView = serde_json::from_str(&json).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn appview_search_as_str() {
        assert_eq!(AppView::Search.as_str(), "search");
    }

    #[test]
    fn appview_inbox_routes_have_stable_names() {
        assert_eq!(AppView::Inbox.as_str(), "inbox");
        assert_eq!(
            AppView::ConversationDetail {
                conversation_id: "conversation-1".to_string(),
            }
            .as_str(),
            "conversation_detail"
        );
    }

    #[test]
    fn appview_settings_as_str() {
        assert_eq!(AppView::Settings.as_str(), "settings");
    }
}
