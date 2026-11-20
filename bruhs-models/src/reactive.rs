//! Durable event-driven agents shared by the local server and both clients.

use crate::crew::room::{Room, RoomState};
use crate::crew::{BruhAccess, BruhLook, BruhState};
use crate::{ids::ThreadId, EpochMs};
use serde::{Deserialize, Serialize};
use serde_json::Value;

mod always_on;
mod draft;
mod report;
mod webhook;
pub use webhook::*;
pub use always_on::*;
pub use draft::{workflow_draft, ReactiveWorkflowDraft};
pub use report::*;

/// Matches the private tool transport's payload budget. Reject, never silently truncate input.
pub const MAX_PAYLOAD_BYTES: usize = 64 * 1024;
/// Agent context and memory stay within the existing thread-message budget.
pub const MAX_TEXT_CHARS: usize = crate::thread_link::MAX_MESSAGE_CHARS;
/// Snapshots show a page of recent activity; older identities remain in durable storage.
pub const HISTORY_LIMIT: usize = 50;
/// The app events an automation can start on, with how both apps name them.
pub const APP_EVENT_TOPICS: [(&str, &str); 3] = [
    ("meeting.completed", "Meeting completed"),
    ("meeting.transcribed", "Final transcript updated"),
    ("meeting.summary", "Summary ready"),
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReactiveAgent {
    pub id: String,
    pub name: String,
    pub instructions: String,
    #[serde(default)]
    pub memory: String,
    pub model: String,
    pub project_path: String,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventSource {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    pub kind: SourceKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceKind {
    Folder {
        path: String,
        pattern: String,
        recursive: bool,
        settle_ms: u64,
        poll_ms: u64,
    },
    Http {
        url: String,
        poll_ms: u64,
    },
    Command {
        command: String,
        cwd: String,
        poll_ms: u64,
        timeout_ms: u64,
    },
    Schedule {
        every_ms: u64,
    },
    /// A scheduled prompt that becomes due once, including after an offline interval.
    Once { due_at: EpochMs },
    /// Wall-clock times in a named IANA time zone, such as weekdays at 08:00.
    Calendar {
        /// `HH:MM` local times, 24-hour clock.
        times: Vec<String>,
        /// ISO weekdays (1 = Monday … 7 = Sunday). Empty means every day.
        #[serde(default)]
        weekdays: Vec<u8>,
        time_zone: String,
    },
    Webhook,
    AppEvent {
        topic: String,
    },
    Custom,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all="snake_case")]
pub enum TaskConversation {
    /// Existing automations continue in the agent's own conversation.
    #[default]
    Agent,
    NewThread,
    Continue,
    FreshSession,
}
impl TaskConversation {
    pub fn is_agent(&self) -> bool {
        *self == Self::Agent
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::NewThread => "new_thread",
            Self::Continue => "continue",
            Self::FreshSession => "fresh_session",
        }
    }
    pub fn has_destination(self) -> bool {
        matches!(self, Self::Continue | Self::FreshSession)
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Agent => "Agent conversation",
            Self::NewThread => "New thread each run",
            Self::Continue => "One thread, continuing the conversation",
            Self::FreshSession => "One thread, fresh session each run",
        }
    }
    pub const ALL: [Self; 4] = [
        Self::NewThread,
        Self::Continue,
        Self::FreshSession,
        Self::Agent,
    ];
}

/// An explicit destination change in a scheduled task edit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskDestinationEdit {
    Existing {
        thread_id: crate::ids::ThreadId,
    },
    /// Detach only the binding that was shown when the owner opened the editor.
    FirstRun {
        previous: crate::ids::ThreadId,
    },
}

/// A conversation found in the selected project's scheduled task history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskThreadChoice {
    pub thread_id: crate::ids::ThreadId,
    pub title: String,
    pub agent_id: String,
    pub checkout: TaskCheckout,
}

/// Filesystem location used when a scheduled task starts a provider turn.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskCheckout {
    #[default]
    Project,
    Existing {
        path: String,
    },
    Worktree {
        base_branch: String,
        start_from_remote: bool,
    },
}

impl TaskCheckout {
    pub fn is_project(&self) -> bool {
        matches!(self, Self::Project)
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Project => Ok(()),
            Self::Existing { path } => {
                if path.trim().is_empty() || path.contains('\0') {
                    return Err("Enter the absolute path of the checkout.".into());
                }
                Ok(())
            }
            Self::Worktree { base_branch, .. } => {
                crate::logic::git::BranchName::try_from(base_branch.as_str())
                    .map(|_| ())
                    .map_err(|error| format!("Choose a valid base branch. {error}"))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Subscription {
    pub id: String,
    pub source_id: String,
    pub agent_id: String,
    pub instructions: String,
    #[serde(default)]
    pub enabled: bool,
    /// An explicit opt-in to unattended runs. Runs keep the bruh's access level; they never exceed it.
    #[serde(default)]
    pub auto_run: bool,
    #[serde(default)]
    pub debounce_ms: u64,
    #[serde(default)]
    pub throttle_ms: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub webhook_filters: Vec<WebhookFilter>,
    /// Owner-authored request template. Older subscriptions use their original event envelope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "WebhookSecurity::is_empty")]
    pub webhook_security: WebhookSecurity,
    #[serde(default, skip_serializing_if = "WebhookDedupe::is_empty")]
    pub webhook_dedupe: WebhookDedupe,
    /// Scheduled task name. Older tasks display their agent's name until named.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_title: Option<String>,
    #[serde(default, skip_serializing_if = "TaskConversation::is_agent")]
    pub task_conversation: TaskConversation,
    #[serde(default, skip_serializing_if = "TaskCheckout::is_project")]
    pub task_checkout: TaskCheckout,
    /// What a new event does while this subscription's agent is mid-run.
    #[serde(default, skip_serializing_if = "BusyPolicy::is_queue")]
    pub on_busy: BusyPolicy,
}

/// One reviewable automation. The subscription is its stable identity; its
/// components remain available independently in advanced settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomationSetup {
    pub agent: ReactiveAgent,
    pub source: EventSource,
    pub subscription: Subscription,
}

impl Subscription {
    pub fn title<'a>(&'a self, agent: &'a ReactiveAgent) -> &'a str {
        self.task_title.as_deref().unwrap_or(&agent.name)
    }
}

impl AutomationSetup {
    pub fn title(&self) -> &str {
        self.subscription.title(&self.agent)
    }

    pub fn from_subscription(
        snapshot: &ReactiveSnapshot,
        subscription: &Subscription,
    ) -> Option<Self> {
        Some(Self {
            agent: snapshot
                .agents
                .iter()
                .find(|agent| agent.id == subscription.agent_id)?
                .clone(),
            source: snapshot
                .sources
                .iter()
                .find(|source| source.id == subscription.source_id)?
                .clone(),
            subscription: subscription.clone(),
        })
    }

    pub fn enabled(&self) -> bool {
        self.agent.enabled && self.source.enabled && self.subscription.enabled
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReactiveEvent {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filtered_subscriptions: Vec<String>,
    pub id: String,
    pub source_id: String,
    pub key: String,
    pub payload: Value,
    pub created_at: EpochMs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    AwaitingReview,
    Queued,
    Running,
    Completed,
    Failed,
    Interrupted,
    Cancelled,
    /// Folded into another run (`merged_into`) that handles its events together.
    Merged,
}

impl RunStatus {
    pub fn is_pending(self) -> bool {
        matches!(self, Self::AwaitingReview | Self::Queued)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReactiveRun {
    pub id: String,
    pub agent_id: String,
    pub source_id: Option<String>,
    pub subscription_id: Option<String>,
    pub event_id: Option<String>,
    pub status: RunStatus,
    pub prompt: String,
    pub thread_id: ThreadId,
    #[serde(default, skip_serializing_if = "TaskCheckout::is_project")]
    pub task_checkout: TaskCheckout,
    pub session_id: Option<String>,
    pub created_at: EpochMs,
    pub updated_at: EpochMs,
    pub not_before: EpochMs,
    pub result: Option<String>,
    pub error: Option<String>,
    /// What started this run. Absent on runs saved before triggers were recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<RunTrigger>,
    /// Runs folded into this one; their events are handled by this turn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merged_runs: Vec<String>,
    /// The run that took over this one's events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged_into: Option<String>,
    /// The agent's parsed `bruhs-report` block, applied when the run completed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<RunReport>,
    /// Why part of the report was not applied, shown with the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_warning: Option<String>,
    /// This run started a new provider session; memory carried continuity.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fresh_session: bool,
    /// Context window after the run's last model request, when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u64>,
    /// The owner's own words on a Chat run, kept when batching rewrites `prompt`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReactiveChange {
    SaveAutomation { automation: Box<AutomationSetup> },
    SaveAgent { agent: ReactiveAgent },
    SaveSource { source: EventSource },
    SaveSubscription { subscription: Subscription },
    DeleteAgent { id: String },
    DeleteSource { id: String },
    DeleteSubscription { id: String },
    SetMemory { agent_id: String, memory: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Pending,
    Approved,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReactiveProposal {
    pub id: String,
    pub change: ReactiveChange,
    pub status: ProposalStatus,
    pub created_at: EpochMs,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReactiveCommand {
    Apply {
        change: ReactiveChange,
    },
    Propose {
        change: ReactiveChange,
    },
    DecideProposal {
        id: String,
        approve: bool,
    },
    /// Run one time-based task immediately. Repeating a request with the same run ID is idempotent.
    RunScheduledTask { id: String, run_id: String },
    /// Commit edited task settings and a requested activation change together.
    SaveScheduledTask {
        automation: Box<AutomationSetup>,
        enabled: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signing_secret: Option<WebhookSigningSecret>,
        /// Omitted by older clients and edits that keep the current destination.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        destination: Option<TaskDestinationEdit>,
    },
    SetAutomationEnabled {
        id: String,
        enabled: bool,
    },
    Publish {
        source_id: String,
        key: String,
        payload: Value,
    },
    Chat {
        agent_id: String,
        message: String,
        /// The id the sender picks for the run this message starts, so a
        /// `message_bruh` sender that waits can read that run until it
        /// ends. Only sent to a service that speaks rooms.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
    },
    /// Delete a task conversation and the schedules linked to it when the owner chose Delete.
    DeleteTaskThread {
        thread_id: crate::ids::ThreadId,
        linked_tasks: std::collections::BTreeSet<String>,
    },
    /// An owner reply in a scheduled task's saved destination.
    ChatInTaskThread {
        task_id: String,
        thread_id: crate::ids::ThreadId,
        message: String,
    },
    CreateRoom {
        room: Room,
    },
    UpdateRoom {
        room: Room,
    },
    DeleteRoom {
        id: String,
    },
    PostRoom {
        room_id: String,
        text: String,
    },
    ApproveRun {
        id: String,
    },
    CancelRun {
        id: String,
    },
    RetryRun {
        id: String,
    },
    /// Mark outbox items read; an empty list marks every item read.
    MarkOutboxRead {
        #[serde(default)]
        ids: Vec<String>,
    },
    /// Answer an agent's question. Resumes that agent's conversation.
    AnswerOutbox {
        id: String,
        message: String,
    },
    /// Resolve a question or update without starting a run.
    DismissOutbox {
        id: String,
    },
    /// Owner correction of one agent-written memory entry.
    EditMemory {
        agent_id: String,
        key: String,
        kind: MemoryKind,
        text: String,
    },
    ForgetMemory {
        agent_id: String,
        key: String,
    },
    /// Queue a run that reviews and tidies the agent's own memory.
    ConsolidateMemory {
        agent_id: String,
    },
    CancelFollowUp {
        id: String,
    },
    SetNotificationSettings {
        settings: NotificationSettings,
    },
    /// A client reports the owner's decision on a workflow made from a run's draft.
    DraftOutcome {
        run_id: String,
        workflow_id: String,
        outcome: DraftOutcomeKind,
        #[serde(default)]
        applied_steps: u32,
        #[serde(default)]
        total_steps: u32,
    },
    CreateBruh {
        agent: ReactiveAgent,
        #[serde(default)]
        look: BruhLook,
        #[serde(default)]
        access: BruhAccess,
        #[serde(default)]
        effort: Option<String>,
    },
    SetBruh {
        agent_id: String,
        look: BruhLook,
        access: BruhAccess,
        #[serde(default)]
        effort: Option<String>,
    },
    /// Pause every bruh and cancel what is running.
    StopAll,
    StopBruh {
        agent_id: String,
    },
    ResumeBruh {
        agent_id: String,
    },
    ResumeAll,
    /// Decide a tool call a bruh paused on.
    DecideApproval {
        id: String,
        approve: bool,
    },
    /// Turn a bruh's Computer on or off. Cancels no runs; off closes the
    /// browser and keeps its profile.
    SetComputer {
        agent_id: String,
        on: bool,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceStatus {
    pub source_id: String,
    pub last_checked_at: Option<EpochMs>,
    pub last_event_at: Option<EpochMs>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReactiveSnapshot {
    pub agents: Vec<ReactiveAgent>,
    pub sources: Vec<EventSource>,
    pub subscriptions: Vec<Subscription>,
    pub events: Vec<ReactiveEvent>,
    pub runs: Vec<ReactiveRun>,
    pub proposals: Vec<ReactiveProposal>,
    #[serde(default)]
    pub source_statuses: Vec<SourceStatus>,
    /// Persistent task destinations, including threads outside the recent runs page.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub task_threads: std::collections::BTreeMap<String, crate::ids::ThreadId>,
    /// Manually selected destinations. Its presence signals support for project thread search.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_task_threads: Option<std::collections::BTreeMap<String,TaskThreadChoice>>,
    /// This host can delete scheduled conversations and their linked tasks atomically.
    #[serde(default)]
    pub can_delete_task_threads: bool,
    /// The host supports per-task webhook URLs and rotation.
    #[serde(default)]
    pub can_address_task_webhooks: bool,
    /// Next occurrences for active time-based sources. Older hosts omit this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_run_by_source: Option<std::collections::BTreeMap<String, EpochMs>>,
    /// Unresolved questions plus a page of recent updates, newest first.
    #[serde(default)]
    pub outbox: Vec<OutboxItem>,
    #[serde(default)]
    pub memories: Vec<MemoryEntry>,
    /// Pending follow-ups plus a page of recent fired or cancelled ones.
    #[serde(default)]
    pub follow_ups: Vec<FollowUp>,
    #[serde(default)]
    pub notification_settings: NotificationSettings,
    /// Every bruh as the rail shows it.
    #[serde(default)]
    pub bruhs: Vec<BruhState>,
    /// Every room with its recent messages.
    #[serde(default)]
    pub rooms: Vec<RoomState>,
    /// Crew wire version the service speaks; 0 from an older service.
    #[serde(default)]
    pub protocol: u32,
}

impl ReactiveSnapshot {
    /// Updates the owner has not read yet.
    pub fn unread_count(&self) -> usize {
        self.outbox.iter().filter(|item| item.is_unread()).count()
    }
}
