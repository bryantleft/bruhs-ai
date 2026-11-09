pub mod agent;
pub mod capture_client;
mod codex_lean;
pub mod connections;
pub mod editor;
pub mod files;
pub mod git;
pub mod google_calendar;
pub mod import;
pub mod meetings;
pub mod models;
pub mod paste_files;
pub mod preview;
pub mod pulls;
pub mod registry;
pub mod second_opinion;
pub mod secrets;
pub mod skills;
pub mod store;
pub mod summarize;
pub mod summary_generation;
pub mod terminal;
pub mod thread_link;
pub mod transcript;
pub mod updates;
pub mod uploads;
#[cfg(unix)]
pub mod workspace_bridge;

pub use agent::{
    claude_uses_api_key, AgentOrchestrator, AgentTurn, McpServerInfo, OrchestratorEvent,
    OrchestratorEventStream, PermissionDecision, PermissionResolveError, QueryEvent,
    SendMessageError, SteerError, ThreadStatus, UsageInfo,
};
pub use capture_client::{CaptureClientError, DaemonStatus};
pub use google_calendar::{
    GoogleCalendarHandle, HandleError as GoogleCalendarHandleError, StartOAuthResponse,
};
pub use meetings::{
    default_meeting_store, InMemoryMeetingStore, MeetingRecords, MeetingStore, MeetingStoreError,
    MeetingStoreResult, PeopleStore, SpeakerLinkJobs, SqliteMeetingStore, SummaryGenerationStore,
};
pub use models::*;
pub use paste_files::write_paste_files;
pub use pulls::{list_pulls, PullList, PullRef};
pub use registry::{BackgroundTaskRegistry, RemoveTaskError};
pub use skills::{list_skills, SkillRef};
pub use store::{AppState, AppStateBuilder, CreateThreadError};
pub use transcript::{RefreshedTranscript, TranscriptError, TranscriptRefresh, TranscriptSource};
pub use uploads::{
    is_upload_path, open_for_viewer, require_uploaded, write_upload, OpenError, OpenedFile,
};
