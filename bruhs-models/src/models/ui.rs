//! UI surface enums — small state types shared by every UI surface
//! (web, desktop) so each one doesn't reinvent its own copy.
//!
//! Per-surface signal/context wrappers (Leptos `ReadSignal`, GPUI entities)
//! still live in their respective crates; only the data lives here.

use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};

/// What `encodeURIComponent` leaves alone: letters, digits and `-_.!~*'()`.
const ROUTE_COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

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
    Workbench,
    Whiteboards {
        board_id: Option<String>,
    },
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
    /// One bruh's page: its chat, memory, routines and settings.
    Bruh {
        agent_id: String,
    },
    /// One room: its transcript, members and settings.
    Room {
        room_id: String,
    },
}

impl AppView {
    /// The view's route as the web writes it after `#`, such as
    /// `/inbox/<id>`. The desktop saves it to reopen the view at start.
    /// Chat is `/chat`; the web routes an open thread as `thread/<id>`.
    pub fn route(&self) -> String {
        match self {
            Self::Whiteboards { board_id } => board_id.as_ref().map_or_else(
                || "/whiteboards".to_string(),
                |id| format!("/whiteboards/{id}"),
            ),
            Self::MeetingDetail { meeting_id } => format!("/meetings/{meeting_id}"),
            Self::MeetingNoteDetail { note_id } => {
                format!(
                    "/meeting-notes/{}",
                    utf8_percent_encode(note_id, ROUTE_COMPONENT)
                )
            }
            Self::DocumentDetail { document_id } => format!("/knowledge/documents/{document_id}"),
            Self::ClipDetail { clip_id } => format!("/knowledge/clips/{clip_id}"),
            Self::ConversationDetail { conversation_id } => format!("/inbox/{conversation_id}"),
            Self::Booking { slug } => format!("/book/{slug}"),
            Self::Bruh { agent_id } => {
                format!("/bruhs/{}", utf8_percent_encode(agent_id, ROUTE_COMPONENT))
            }
            Self::Room { room_id } => {
                format!("/rooms/{}", utf8_percent_encode(room_id, ROUTE_COMPONENT))
            }
            other => format!("/{}", other.as_str()),
        }
    }

    /// The view a route names, with or without its `#`, or `None` for one
    /// this build does not know. `thread/<id>` names Chat. A whiteboard id
    /// must be a UUID and a meeting note or bruh id 1 to 128 bytes, as the
    /// web checks.
    pub fn from_route(route: &str) -> Option<Self> {
        let route = route.trim_start_matches('#');
        if route
            .strip_prefix("thread/")
            .is_some_and(|id| !id.is_empty())
        {
            return Some(Self::Chat);
        }
        if let Some(id) = route.strip_prefix("/whiteboards/") {
            return uuid::Uuid::parse_str(id).ok().map(|_| Self::Whiteboards {
                board_id: Some(id.into()),
            });
        }
        if let Some(rest) = route.strip_prefix("/meeting-notes/") {
            let note_id = percent_decode_str(rest).decode_utf8().ok()?;
            return (!note_id.trim().is_empty() && note_id.len() <= 128).then(|| {
                Self::MeetingNoteDetail {
                    note_id: note_id.into_owned(),
                }
            });
        }
        if let Some(rest) = route.strip_prefix("/bruhs/") {
            let agent_id = percent_decode_str(rest).decode_utf8().ok()?;
            return (!agent_id.trim().is_empty() && agent_id.len() <= 128).then(|| Self::Bruh {
                agent_id: agent_id.into_owned(),
            });
        }
        if let Some(rest) = route.strip_prefix("/rooms/") {
            let room_id = percent_decode_str(rest).decode_utf8().ok()?;
            return (!room_id.trim().is_empty() && room_id.len() <= 128).then(|| Self::Room {
                room_id: room_id.into_owned(),
            });
        }
        let with_id = |prefix: &str| route.strip_prefix(prefix).filter(|id| !id.is_empty());
        if let Some(id) = with_id("/meetings/") {
            return Some(Self::MeetingDetail {
                meeting_id: id.to_string(),
            });
        }
        if let Some(id) = with_id("/knowledge/documents/") {
            return Some(Self::DocumentDetail {
                document_id: id.to_string(),
            });
        }
        if let Some(id) = with_id("/knowledge/clips/") {
            return Some(Self::ClipDetail {
                clip_id: id.to_string(),
            });
        }
        if let Some(id) = with_id("/inbox/") {
            return Some(Self::ConversationDetail {
                conversation_id: id.to_string(),
            });
        }
        if let Some(slug) = with_id("/book/") {
            return Some(Self::Booking {
                slug: slug.to_string(),
            });
        }
        Some(match route {
            "/today" => Self::Today,
            "/inbox" => Self::Inbox,
            "/knowledge" => Self::Knowledge,
            "/chat" => Self::Chat,
            "/tasks" => Self::Tasks,
            "/calendar" => Self::Calendar,
            "/life" => Self::Life,
            "/automations" => Self::Automations,
            "/workbench" => Self::Workbench,
            "/whiteboards" => Self::Whiteboards { board_id: None },
            "/meetings" => Self::Meetings,
            "/people" => Self::People,
            "/search" => Self::Search,
            "/terminal" => Self::Terminal,
            "/settings" => Self::Settings,
            _ => return None,
        })
    }

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
            Self::Workbench => "workbench",
            Self::Whiteboards { .. } => "whiteboards",
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
            Self::Bruh { .. } => "bruh",
            Self::Room { .. } => "room",
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

    #[test]
    fn every_view_reads_back_from_its_route() {
        let views = [
            AppView::Today,
            AppView::Inbox,
            AppView::Knowledge,
            AppView::Chat,
            AppView::Tasks,
            AppView::Calendar,
            AppView::Life,
            AppView::Automations,
            AppView::Workbench,
            AppView::Whiteboards { board_id: None },
            AppView::Whiteboards {
                board_id: Some("0f8fad5b-d9cb-469f-a165-70867728950e".into()),
            },
            AppView::Meetings,
            AppView::People,
            AppView::Settings,
            AppView::MeetingDetail {
                meeting_id: "m-1".into(),
            },
            AppView::MeetingNoteDetail {
                note_id: "note 1/ä".into(),
            },
            AppView::DocumentDetail {
                document_id: "d-1".into(),
            },
            AppView::ConversationDetail {
                conversation_id: "c-1".into(),
            },
            AppView::Booking {
                slug: "intro".into(),
            },
            AppView::ClipDetail {
                clip_id: "clip-1".into(),
            },
            AppView::Search,
            AppView::Terminal,
            AppView::Bruh {
                agent_id: "bruh 1/ä".into(),
            },
            AppView::Room {
                room_id: "room 1/ä".into(),
            },
        ];
        for view in views {
            assert_eq!(AppView::from_route(&view.route()), Some(view.clone()));
            assert_eq!(
                AppView::from_route(&format!("#{}", view.route())),
                Some(view)
            );
        }
        assert_eq!(
            AppView::MeetingNoteDetail {
                note_id: "note 1/ä".into()
            }
            .route(),
            "/meeting-notes/note%201%2F%C3%A4"
        );
        assert_eq!(
            AppView::Bruh {
                agent_id: "b1".into()
            }
            .route(),
            "/bruhs/b1"
        );
        assert_eq!(AppView::from_route("/bruhs/"), None);
        assert_eq!(
            AppView::Room {
                room_id: "r1".into()
            }
            .route(),
            "/rooms/r1"
        );
        assert_eq!(AppView::from_route("/rooms/"), None);
    }

    #[test]
    fn unknown_or_invalid_routes_name_no_view() {
        assert_eq!(AppView::from_route("#thread/abc"), Some(AppView::Chat));
        assert_eq!(AppView::from_route("#thread/"), None);
        assert_eq!(AppView::from_route("/whiteboards/not-a-uuid"), None);
        assert_eq!(AppView::from_route("/meeting-notes/%20"), None);
        assert_eq!(AppView::from_route("/inbox/"), None);
        assert_eq!(AppView::from_route("today"), None);
        assert_eq!(AppView::from_route("/nowhere"), None);
    }
}
