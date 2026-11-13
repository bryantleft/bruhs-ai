//! Durable event-driven agents shared by the local server and both clients.

use crate::{ids::ThreadId, EpochMs};
use serde::{Deserialize, Serialize};
use serde_json::Value;

mod draft;
pub use draft::{workflow_draft, ReactiveWorkflowDraft};

/// Matches the private tool transport's payload budget. Reject, never silently truncate input.
pub const MAX_PAYLOAD_BYTES: usize = 64 * 1024;
/// Agent context and memory stay within the existing thread-message budget.
pub const MAX_TEXT_CHARS: usize = crate::thread_link::MAX_MESSAGE_CHARS;
/// Snapshots show a page of recent activity; older identities remain in durable storage.
pub const HISTORY_LIMIT: usize = 50;

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
    Webhook,
    AppEvent {
        topic: String,
    },
    Custom,
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
    /// An explicit opt-in to unattended read-only turns. Never grants write access.
    #[serde(default)]
    pub auto_run: bool,
    #[serde(default)]
    pub debounce_ms: u64,
    #[serde(default)]
    pub throttle_ms: u64,
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

impl AutomationSetup {
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
    pub session_id: Option<String>,
    pub created_at: EpochMs,
    pub updated_at: EpochMs,
    pub not_before: EpochMs,
    pub result: Option<String>,
    pub error: Option<String>,
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
}
