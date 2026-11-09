//! Request and response bodies of the bruhs-server HTTP API, shared by the
//! server, the web SPA and the desktop client so the three cannot drift.
//!
//! Every type here reads the JSON the current producers write (see the
//! fixture tests at the bottom); where an older client or record wrote a
//! different shape, the reader accepts both.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::PermissionMode;
use crate::ids::{MeetingId, ThreadId};
use crate::logic::composer_text::TextAttachment;
use crate::time::{self, EpochMs};
use crate::{MeetingStatus, WhisperModel};

fn default_true() -> bool {
    true
}

// ── Live events ────────────────────────────────────────────────────────────

/// `GET /api/events/stream`: the web app's one live connection. Each frame's
/// SSE `event:` name says which source it comes from; its data is the JSON
/// that source's own stream sends.
pub mod live_events {
    /// Path of the combined stream.
    pub const PATH: &str = "/api/events/stream";
    /// Query a client adds when it reopens the stream: it may have missed
    /// calendar events, so the server replays the calendar sync window.
    pub const RESUME_QUERY: &str = "resume=true";
    /// Full task list snapshot (`Vec<BackgroundTask>`), as `/api/tasks/stream`.
    pub const TASKS: &str = "tasks";
    /// `CalendarSyncEvent`, as `/api/calendar/events/stream`.
    pub const CALENDAR: &str = "calendar";
    /// `MeetingEvent`, as `/api/meetings/stream`.
    pub const MEETINGS: &str = "meetings";
}

// ── Meetings ───────────────────────────────────────────────────────────────

/// `POST /api/meetings`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateMeetingBody {
    pub title: String,
    pub started_at_ms: EpochMs,
    /// Google event id (or local event id) the meeting records.
    #[serde(default)]
    pub calendar_event_id: Option<String>,
}

/// `POST /api/meetings/{id}/stop`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopMeetingBody {
    pub ended_at_ms: EpochMs,
}

/// `PATCH /api/meetings/{id}`; `None` leaves a field unchanged.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PatchMeetingBody {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub status: Option<MeetingStatus>,
    #[serde(default)]
    pub ended_at_ms: Option<EpochMs>,
}

/// `POST /api/meetings/{id}/summarize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SummarizeRequest {
    /// Identity of this generation, from the caller's acknowledged intent
    /// checkpoint. Reuse it after an uncertain response; never mint one for
    /// an automatic retry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// When false, no per-person observations are extracted.
    #[serde(default = "default_true")]
    pub extract_observations: bool,
    /// Atomically replace this meeting's observations after a successful
    /// generation.
    #[serde(default)]
    pub replace_observations: bool,
}

impl Default for SummarizeRequest {
    fn default() -> Self {
        Self {
            request_id: None,
            extract_observations: true,
            replace_observations: false,
        }
    }
}

/// `DELETE /api/meetings/{id}/generations/{request_id}`: the options of the
/// request being fenced, so one the server never received is recorded too.
/// Unknown fields are rejected, like [`SummarizeRequest`]: a misspelled
/// option must not silently fence a different request identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelSummaryGenerationBody {
    #[serde(default = "default_true")]
    pub extract_observations: bool,
    #[serde(default)]
    pub replace_observations: bool,
}

// ── Google Calendar ────────────────────────────────────────────────────────

/// `POST /api/calendar/google/oauth/start`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartOAuthResponse {
    pub auth_url: String,
    /// PKCE state the server validates on callback; clients only pass it on.
    #[serde(default)]
    pub state: String,
}

// ── Capture daemon ─────────────────────────────────────────────────────────

/// `GET /api/capture/status`: whether the capture daemon answered, and what
/// it is doing.
///
/// On the wire this is the flat object the server has always sent:
/// `{running, recording, meeting_id, started_at_ms, system_audio_permission,
/// system_audio_capturing}`, with every field but `running` zero when the
/// daemon is down.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "CaptureStatusWire", into = "CaptureStatusWire")]
pub enum CaptureStatus {
    /// The daemon did not answer.
    #[default]
    NotRunning,
    Running(RunningCapture),
}

impl CaptureStatus {
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running(_))
    }

    /// The recording in progress, if any.
    pub fn recording(&self) -> Option<&ActiveRecording> {
        match self {
            Self::Running(running) => running.recording.as_ref(),
            Self::NotRunning => None,
        }
    }
}

/// A daemon that answered its status probe.
#[derive(Debug, Clone, PartialEq)]
pub struct RunningCapture {
    pub recording: Option<ActiveRecording>,
    pub system_audio_permission: SystemAudioPermission,
    /// Whether system audio is being captured alongside the microphone.
    pub system_audio_capturing: bool,
}

/// The meeting the daemon is recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveRecording {
    pub meeting_id: MeetingId,
    pub started_at_ms: EpochMs,
}

/// macOS system-audio capture permission, as the daemon reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemAudioPermission {
    Granted,
    Denied,
    NotDetermined,
    Unsupported,
    /// A value this build does not know (or the empty string).
    #[serde(other)]
    Unknown,
}

/// Why a capture status object is self-contradictory.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CaptureStatusError {
    #[error("capture status says recording but has no meeting id")]
    RecordingWithoutMeeting,
    #[error("capture status says recording but has no start time")]
    RecordingWithoutStart,
    #[error("capture status has an invalid meeting id: {0}")]
    MeetingId(#[from] crate::ids::IdError),
    #[error("capture status has an invalid start time: {0}")]
    StartedAt(#[from] time::EpochMsError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CaptureStatusWire {
    running: bool,
    #[serde(default)]
    recording: bool,
    #[serde(default)]
    meeting_id: Option<String>,
    #[serde(default)]
    started_at_ms: Option<f64>,
    #[serde(default)]
    system_audio_permission: String,
    #[serde(default)]
    system_audio_capturing: bool,
}

impl TryFrom<CaptureStatusWire> for CaptureStatus {
    type Error = CaptureStatusError;

    fn try_from(wire: CaptureStatusWire) -> Result<Self, Self::Error> {
        if !wire.running {
            return Ok(Self::NotRunning);
        }
        let recording = if wire.recording {
            let meeting_id = wire
                .meeting_id
                .ok_or(CaptureStatusError::RecordingWithoutMeeting)?;
            let started_at_ms = wire
                .started_at_ms
                .ok_or(CaptureStatusError::RecordingWithoutStart)?;
            Some(ActiveRecording {
                meeting_id: MeetingId::try_from(meeting_id)?,
                started_at_ms: EpochMs::from_f64_floor(started_at_ms)?,
            })
        } else {
            None
        };
        // An unknown permission string is not an error: it reads as Unknown.
        let system_audio_permission =
            serde_json::from_value(Value::String(wire.system_audio_permission))
                .unwrap_or(SystemAudioPermission::Unknown);
        Ok(Self::Running(RunningCapture {
            recording,
            system_audio_permission,
            system_audio_capturing: wire.system_audio_capturing,
        }))
    }
}

impl From<CaptureStatus> for CaptureStatusWire {
    fn from(status: CaptureStatus) -> Self {
        match status {
            CaptureStatus::NotRunning => Self {
                running: false,
                recording: false,
                meeting_id: None,
                started_at_ms: None,
                system_audio_permission: String::new(),
                system_audio_capturing: false,
            },
            CaptureStatus::Running(running) => {
                let permission = match running.system_audio_permission {
                    SystemAudioPermission::Granted => "granted",
                    SystemAudioPermission::Denied => "denied",
                    SystemAudioPermission::NotDetermined => "not_determined",
                    SystemAudioPermission::Unsupported => "unsupported",
                    SystemAudioPermission::Unknown => "unknown",
                };
                Self {
                    running: true,
                    recording: running.recording.is_some(),
                    started_at_ms: running.recording.as_ref().map(|r| r.started_at_ms.as_f64()),
                    meeting_id: running.recording.map(|r| r.meeting_id.into_inner()),
                    system_audio_permission: permission.to_string(),
                    system_audio_capturing: running.system_audio_capturing,
                }
            }
        }
    }
}

/// `POST /capture/start` on the capture daemon. `None` options use the
/// daemon's defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureStartBody {
    pub meeting_id: MeetingId,
    pub started_at_ms: EpochMs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<WhisperModel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_model: Option<WhisperModel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcription: Option<crate::TranscriptionSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subspeaker_enabled: Option<bool>,
    /// Auto-link voice clusters to existing people.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cross_meeting_linking_auto: Option<bool>,
    /// Cosine-similarity threshold for cross-meeting linking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cross_meeting_linking_threshold: Option<f32>,
    /// Maximum number of sub-speaker clusters (2–20).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_speakers: Option<u32>,
    /// Read participant names from video frames (opt-in).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_ocr_enabled: Option<bool>,
}

// ── Threads and background tasks ───────────────────────────────────────────

/// A chat thread, as the server returns it (`POST /api/meetings/{id}/chat`)
/// and as the web stores it in IndexedDB.
///
/// Reads every shape written so far: the server's (`created_at`/`updated_at`
/// integers, `project_name: null` for general threads) and the web's
/// (`updated_at` as a string, possibly `"now"`; `project_name: ""` for
/// general threads; no `created_at`). Writes the server's shape, omitting
/// the optional fields that are unset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Thread {
    pub id: ThreadId,
    pub title: String,
    /// Working directory for the agent; `None` for general threads.
    #[serde(default)]
    pub project_path: Option<String>,
    /// Project display name; `None` for general threads.
    #[serde(default, deserialize_with = "deserialize_project_name")]
    pub project_name: Option<String>,
    #[serde(default, deserialize_with = "time::deserialize_legacy")]
    pub created_at: EpochMs,
    #[serde(deserialize_with = "time::deserialize_legacy")]
    pub updated_at: EpochMs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meeting_id: Option<MeetingId>,
    /// Provider session to resume on the next turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Branch the project was on when the last turn started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Model id of the last turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub pinned: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub settled: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// `""` is how the web marks a general (project-less) thread.
fn deserialize_project_name<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.filter(|name| !name.is_empty()))
}

/// What a background task runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    #[default]
    Shell,
    /// A kind this build does not know.
    #[serde(other)]
    Other,
}

/// Lifecycle of a background task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Running,
    Exited,
    Killed,
    Failed,
    /// A status this build does not know.
    #[serde(other)]
    Unknown,
}

/// A background shell the agent started (`GET /api/tasks`, the
/// `/api/tasks/stream` snapshots, and `background_task_registered` stream
/// events).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackgroundTask {
    pub id: String,
    pub thread_id: ThreadId,
    #[serde(default)]
    pub parent_task_id: Option<String>,
    #[serde(default)]
    pub kind: TaskKind,
    pub label: String,
    pub status: TaskStatus,
    pub started_at: EpochMs,
    #[serde(default)]
    pub ended_at: Option<EpochMs>,
    #[serde(default)]
    pub exit_code: Option<i32>,
    /// Tool input that started the task.
    #[serde(default)]
    pub meta: Value,
    /// Tail of the captured output.
    #[serde(default)]
    pub log_tail: String,
}

// ── Agent turns ────────────────────────────────────────────────────────────

/// `POST /api/agent`: run one turn; the response is an SSE stream of
/// [`AgentStreamMessage`]s.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentRequest {
    pub thread_id: ThreadId,
    pub prompt: String,
    /// Working directory of the turn; `None` (omitted) runs it in the user's
    /// home directory, which is what a chat without a project gets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_path: Option<String>,
    /// `None` lets the server choose [`crate::agent::DEFAULT_AGENT_MODEL`].
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<PermissionMode>,
    /// Provider session to resume (from a previous turn's `init`/`result`).
    #[serde(default)]
    pub session_id: Option<String>,
    /// Forget the provider session this thread would resume and start a new one.
    #[serde(default, skip_serializing_if = "skip_false")]
    pub fresh_session: bool,
    /// Pasted text the server writes beside the prompt. Empty stays off the wire.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text_attachments: Vec<TextAttachment>,
    /// Files already stored by `POST /api/agent/uploads`. Empty stays off the wire.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_attachments: Vec<crate::logic::attachments::UploadedFile>,
    /// Claude `--effort`. Omitted for Codex and when the CLI default should stand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Claude `--autocompact`. Omitted when unset, and never sent to Codex.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autocompact: Option<String>,
}

/// `POST /api/agent/second-opinion`: a read-only review of a thread's last
/// answer by the provider the thread does not use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecondOpinionRequest {
    /// The thread's project; missing reviews from the home directory.
    #[serde(default)]
    pub project_path: Option<String>,
    /// The provider the thread runs on; the other one reviews.
    pub thread_provider: crate::agent::AgentProvider,
    /// What the user asked for.
    pub task: String,
    /// The answer under review.
    pub answer: String,
    /// What to look at most closely; may be empty.
    #[serde(default)]
    pub focus: String,
}

/// Reply to [`SecondOpinionRequest`]: the review, or why there is none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecondOpinionResponse {
    pub reviewer: crate::agent::AgentProvider,
    #[serde(default)]
    pub review: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// What the review spent, so its cost can be counted next to the turns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<UsageInfo>,
}

/// `POST /api/agent/steer`: write one user message into the running Claude turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SteerRequest {
    pub thread_id: ThreadId,
    pub prompt: String,
}

fn skip_false(value: &bool) -> bool {
    !*value
}

/// `POST /api/agent/approve`: answer a `permission_request` event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApproveRequest {
    #[serde(alias = "requestId")]
    pub request_id: String,
    pub approved: bool,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Reply to [`ApproveRequest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApproveResponse {
    pub success: bool,
}

/// Token counts for a turn. Accepts the web's older `cache_*_tokens` names.
///
/// The four counts are billed tokens (one request on a `usage` event, the
/// turn's sum on a `result`). `context_tokens` is how full the window is
/// after the latest request; `None` on a turn sum, so keep the last value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageInfo {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default, alias = "cache_read_tokens")]
    pub cache_read_input_tokens: u64,
    #[serde(default, alias = "cache_creation_tokens")]
    pub cache_creation_input_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u64>,
}

/// An MCP server the provider loaded for the turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerInfo {
    pub name: String,
    #[serde(default)]
    pub status: String,
}

/// One provider event of an agent turn: the `data:` object of an
/// `/api/agent` SSE message whose `envelope` is `"query"`.
///
/// Missing fields read as empty, like the web's stream reducer always did.
/// An unrecognized `type` reads as [`AgentStreamEvent::Unknown`]; to keep the
/// unrecognized payload, read the stream as [`AgentStreamMessage`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentStreamEvent {
    SecretRequest(crate::secrets::SecretRequest),
    SecretResolved {
        id: String,
    },
    /// The provider session started.
    Init {
        #[serde(default)]
        session_id: String,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        tools: Vec<String>,
        #[serde(default)]
        mcp_servers: Vec<McpServerInfo>,
    },
    /// A chunk of assistant prose.
    Text {
        #[serde(default)]
        text: String,
    },
    /// A chunk of visible reasoning.
    Thinking {
        #[serde(default)]
        thinking: String,
    },
    ToolCall {
        #[serde(default)]
        id: String,
        #[serde(default)]
        tool: String,
        #[serde(default)]
        input: Value,
    },
    ToolResult {
        #[serde(default)]
        tool_use_id: String,
        #[serde(default)]
        content: String,
        #[serde(default)]
        is_error: bool,
    },
    /// Running token totals.
    Usage(UsageInfo),
    /// The turn finished; `is_error` marks a failed turn whose reason is in
    /// `result`.
    Result {
        #[serde(default)]
        result: String,
        #[serde(default)]
        session_id: String,
        #[serde(default)]
        duration_ms: u64,
        #[serde(default)]
        usage: UsageInfo,
        #[serde(default)]
        is_error: bool,
    },
    /// The provider is waiting for the user to allow or deny a tool call;
    /// answer with [`ApproveRequest`].
    PermissionRequest {
        #[serde(default)]
        id: String,
        #[serde(default)]
        tool: String,
        #[serde(default)]
        input: Value,
        #[serde(default)]
        tool_use_id: Option<String>,
    },
    /// The provider withdrew a pending permission request.
    PermissionCancelled {
        #[serde(default)]
        id: String,
    },
    /// The turn failed.
    Error {
        #[serde(default)]
        message: String,
    },
    /// A provider event bruhs passes through uninterpreted (for example a
    /// transient Codex reconnect notice).
    Raw {
        #[serde(default)]
        data: Value,
    },
    /// A `type` this build does not know.
    #[serde(other)]
    Unknown,
}

/// One `data:` payload of the `/api/agent` SSE stream.
///
/// The server wraps provider events as `{"envelope":"query","type":…}` and
/// announces background shells as
/// `{"envelope":"background_task_registered","task":{…}}`. Reading never
/// fails on a JSON object: anything unrecognized (an unknown `type` or
/// `envelope`, or a malformed known event) is kept verbatim in
/// [`AgentStreamMessage::Unknown`].
#[derive(Debug, Clone, PartialEq)]
pub enum AgentStreamMessage {
    Event(AgentStreamEvent),
    BackgroundTaskRegistered(Box<BackgroundTask>),
    Unknown(Value),
}

const QUERY_ENVELOPE: &str = "query";
const TASK_ENVELOPE: &str = "background_task_registered";

impl AgentStreamMessage {
    /// Classify an already-parsed payload.
    pub fn from_value(value: Value) -> Self {
        match value.get("envelope").and_then(Value::as_str) {
            Some(TASK_ENVELOPE) => match value.get("task").map(BackgroundTask::deserialize) {
                Some(Ok(task)) => Self::BackgroundTaskRegistered(Box::new(task)),
                _ => Self::Unknown(value),
            },
            Some(QUERY_ENVELOPE) | None => match AgentStreamEvent::deserialize(&value) {
                Ok(AgentStreamEvent::Unknown) | Err(_) => Self::Unknown(value),
                Ok(event) => Self::Event(event),
            },
            Some(_) => Self::Unknown(value),
        }
    }
}

#[derive(Serialize)]
struct QueryEnvelope<'a> {
    envelope: &'static str,
    #[serde(flatten)]
    event: &'a AgentStreamEvent,
}

#[derive(Serialize)]
struct TaskEnvelope<'a> {
    envelope: &'static str,
    task: &'a BackgroundTask,
}

impl Serialize for AgentStreamMessage {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Event(event) => QueryEnvelope {
                envelope: QUERY_ENVELOPE,
                event,
            }
            .serialize(serializer),
            Self::BackgroundTaskRegistered(task) => TaskEnvelope {
                envelope: TASK_ENVELOPE,
                task,
            }
            .serialize(serializer),
            Self::Unknown(value) => value.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for AgentStreamMessage {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Value::deserialize(deserializer).map(Self::from_value)
    }
}

#[cfg(test)]
mod tests;
