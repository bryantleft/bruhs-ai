//! The local MCP workspace connection (`docs/local-mcp-workspace.md`): the
//! request protocol and the pure logic every app answers it with.
//!
//! `bruhs-mcp` turns a tool call into a [`BridgeRequest`] and a transport
//! carries it to the app that owns the workspace: a private Unix socket to the
//! native app, or bruhs-server's relay to the web tab that enabled the
//! session ([`relay`]). The app checks its current grants and answers from its
//! own records with the functions here, so a request reads, pages, filters and
//! proposes the same way on both apps. Transports never grant, approve or
//! execute anything.

mod chat;
mod life;
mod meetings;
mod notes;
mod page;
mod people;
mod proposal;
mod query;
pub mod relay;
mod reviews;

pub use chat::{
    authorize_chat_messages, read_chat_messages, read_chat_threads, ChatMessageRecord,
    ChatThreadSummary, ChatToolCallRecord, ChatUsageRecord, CHAT_THREAD_NOT_FOUND,
};
pub use life::{authorize_life_read, read_finance, read_health};
pub use meetings::{read_meeting, read_meetings};
pub use notes::{read_meeting_note, read_meeting_notes};
pub use page::{read_matching_page, read_page, read_records};
pub use people::{read_people, read_person};
pub use proposal::{ProposalError, WorkflowProposal};
pub use query::{LowercaseQuery, Matcher};
pub use reviews::{
    preview_operation, preview_workflow, BridgeReviews, Proposed, RollbackRequest, RollbackReview,
};

use crate::{AgentAccess, AgentDomain, AgentPermissionPolicy, WorkflowOperation};
use serde::{Deserialize, Serialize};

/// Largest encoded request an app accepts from a transport.
pub const MAX_REQUEST_BYTES: usize = 64 * 1024;
/// Largest encoded [`BridgeResponse`] a transport carries back.
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
/// Records in one read page.
pub const RECORD_LIMIT: usize = 50;
/// Longest `query` a read accepts, in UTF-8 bytes.
pub const MAX_QUERY_BYTES: usize = 512;
/// Largest `offset` a read accepts.
pub const MAX_OFFSET: usize = 100_000;
/// Longest identifier (record IDs and `request_id`), in bytes.
pub const MAX_ID_BYTES: usize = 128;
/// Distinct proposals one connection session may submit.
pub const PROPOSAL_LIMIT: usize = 100;

/// Every read checks its domain's current Read grant first, before it
/// resolves an ID, so a revoked grant cannot reveal whether a record exists.
pub const READ_NOT_GRANTED: &str = "Read access is not granted for this domain.";

/// Shown in Agent workflows while a session is enabled, on both apps.
pub const SESSION_ACTIVE_NOTE: &str =
    "Session active. Grant reads and writes separately below. Chat reads include summaries \
    and completed messages. Drafts, voice fingerprints, and transaction notes stay private. \
    Finance and Health also need Life consent. Sharing finance amounts and quantities needs \
    separate consent; labels remain visible when values are hidden. Writes require review \
    and start, and create local records. They never send messages, make payments, or change \
    providers. Disabling disconnects all clients.";
/// Shown in Agent workflows while no session is enabled, on both apps.
pub const SESSION_OFF_NOTE: &str =
    "Off by default. Connect only a trusted local client. Grant reads and writes separately \
    below. Chat reads include summaries and completed messages. Drafts, voice fingerprints, \
    and transaction notes stay private. Finance and Health also need Life consent. Sharing \
    finance amounts and quantities needs separate consent; labels remain visible when \
    values are hidden. Writes require review and start, and create local records. They \
    never send messages, make payments, or change providers.";
/// Shown when a session could not start or its transport stopped.
pub const CONNECTION_FAILED_NOTE: &str =
    "The local connection is unavailable. Access is disabled; you can retry.";
/// The app-wide notice while a proposal or undo request waits for review. It
/// never names the workflow or its payload.
pub const PENDING_REVIEW_NOTICE: &str =
    "An MCP proposal is awaiting your review. Open workflows to inspect the full payload.";
/// Shown beside an undo request in workflow review.
pub const ROLLBACK_REVIEW_NOTE: &str = "An MCP client requests undo. Review this workflow's completed steps. Undo removes unchanged created records and restores prior relationship context for reminders; Keep changes declines the request.";

/// The `mcpServers` entry a client pastes to reach a session. It names the
/// private connection file; the session token itself never leaves that file.
pub fn mcp_client_config(connection_path: &str) -> serde_json::Value {
    serde_json::json!({
        "mcpServers": {
            "bruhs": {
                "command": "bruhs-mcp",
                "env": { "BRUHS_WORKSPACE_BRIDGE": connection_path }
            }
        }
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonReadSection {
    #[default]
    Profile,
    Observations,
    RelationshipItems,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeetingReadSection {
    #[default]
    Summary,
    Transcript,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinanceReadSection {
    Accounts,
    #[default]
    Transactions,
    Budgets,
    IncomeStreams,
    Holdings,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthReadSection {
    #[default]
    Workouts,
    Sleep,
    Recovery,
    Habits,
}

/// One request a connected client may make. None of them grants access,
/// approves a proposal or executes a step: the app decides all of that.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BridgeRequest {
    ReadFinance {
        #[serde(default)]
        section: FinanceReadSection,
        #[serde(default)]
        query: String,
        #[serde(default)]
        offset: usize,
    },
    ReadHealth {
        #[serde(default)]
        section: HealthReadSection,
        #[serde(default)]
        query: String,
        #[serde(default)]
        offset: usize,
    },
    ReadMeetingNotes {
        #[serde(default)]
        query: String,
        #[serde(default)]
        offset: usize,
    },
    ReadMeetingNote {
        note_id: String,
    },
    ReadMeeting {
        meeting_id: String,
        #[serde(default)]
        section: MeetingReadSection,
        #[serde(default)]
        offset: usize,
    },
    ReadPerson {
        person_id: String,
        #[serde(default)]
        section: PersonReadSection,
        #[serde(default)]
        offset: usize,
    },
    ReadChatMessages {
        thread_id: String,
        #[serde(default)]
        offset: usize,
    },
    Read {
        domain: AgentDomain,
        #[serde(default)]
        query: String,
        #[serde(default)]
        offset: usize,
    },
    Preview {
        operation: WorkflowOperation,
    },
    Propose {
        request_id: String,
        operation: WorkflowOperation,
    },
    PreviewWorkflow {
        proposal: WorkflowProposal,
    },
    ProposeWorkflow {
        request_id: String,
        proposal: WorkflowProposal,
    },
    Status {
        workflow_id: String,
    },
    RequestRollback {
        workflow_id: String,
        dry_run: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", content = "value", rename_all = "snake_case")]
pub enum BridgeResponse {
    Ok(serde_json::Value),
    Error(String),
}

impl From<Result<serde_json::Value, String>> for BridgeResponse {
    fn from(result: Result<serde_json::Value, String>) -> Self {
        match result {
            Ok(value) => Self::Ok(value),
            Err(message) => Self::Error(message),
        }
    }
}

/// The current Read grant for `domain`, checked before anything else.
pub fn require_read(policy: &AgentPermissionPolicy, domain: AgentDomain) -> Result<(), String> {
    if policy.allows(domain, AgentAccess::Read) {
        Ok(())
    } else {
        Err(READ_NOT_GRANTED.into())
    }
}

/// Domains with a write adapter in both apps. Every domain has one today;
/// a domain without one still answers with an explicit refusal.
pub const fn write_adapter_connected(domain: AgentDomain) -> bool {
    matches!(
        domain,
        AgentDomain::Chat
            | AgentDomain::Meetings
            | AgentDomain::Tasks
            | AgentDomain::Calendar
            | AgentDomain::Capture
            | AgentDomain::Knowledge
            | AgentDomain::Inbox
            | AgentDomain::Finance
            | AgentDomain::Health
            | AgentDomain::People
    )
}

/// A single operation's adapter and payload, before any grant is consulted.
pub fn validate_operation(operation: &WorkflowOperation) -> Result<(), String> {
    if !write_adapter_connected(operation.domain()) {
        return Err("This write adapter is not connected. Nothing was changed.".into());
    }
    if !operation.is_valid() {
        return Err("Invalid operation payload. Nothing was changed.".into());
    }
    Ok(())
}

/// A whole proposal's shape and adapters, before any grant is consulted.
pub fn validate_proposal(proposal: &WorkflowProposal) -> Result<(), String> {
    proposal.validate().map_err(|error| error.to_string())?;
    if proposal
        .operations
        .iter()
        .any(|operation| !write_adapter_connected(operation.domain()))
    {
        return Err(
            "This workflow includes an unconnected write adapter. Nothing was changed.".into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;
