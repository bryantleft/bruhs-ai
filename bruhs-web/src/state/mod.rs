use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet, VecDeque};

use bruhs_models::agent::Role;
use bruhs_models::EpochMs;

use crate::auth::AuthStatus;
use crate::durable_metadata::DurableMetadata;

// Pure data types live in bruhs-models so every surface (web/desktop/server)
// shares the same shape. Re-exported here so existing `crate::state::*`
// imports keep working.
pub use bruhs_models::speaker_id::ColorIndex;
pub use bruhs_models::{
    AppView, AutoRecordSettings, AutomationState, Calendar, CalendarEvent, CalendarMode,
    CaptureRecord, DiarizeSettings, EventSource, GoogleCalendarSyncState, InboxConversation,
    KnowledgeAttachment, KnowledgeDocument, LifeContext, LifeSection, Meeting, MeetingEvent,
    MeetingStatus, MeetingSummary, ObservationKind, PermissionMode, PersonObservation,
    PersonProfile, PersonalitySettings, Priority, Project, RecurrenceRule, RelationshipProfile,
    SchedulingPreferences, SelectedLifeRecord, SourceClip, Speaker, SpeakerIdentity, TaskViewMode,
    TodaySnapshot, TranscriptSegment, WhisperModel, WorkItem, WorkItemStatus,
};

mod calendar;
mod chat;
mod library;
mod meetings;
mod prefs;
mod settings;
mod sidebar_edit;
mod tasks;
mod terminal;
mod workspace;

pub use calendar::*;
pub use chat::*;
pub use library::*;
pub use meetings::*;
pub use prefs::*;
pub use settings::*;
pub use sidebar_edit::*;
pub use tasks::*;
pub use terminal::*;
pub use workspace::*;

/// Provide every app-wide context, one domain at a time. The workspace
/// goes first: meetings reads its banner stack.
pub fn provide_app_state() {
    workspace::provide();
    settings::provide();
    chat::provide();
    terminal::provide();
    tasks::provide();
    library::provide();
    calendar::provide();
    meetings::provide();
}
