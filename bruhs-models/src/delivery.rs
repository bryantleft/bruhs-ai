//! Reviewable repository delivery operations shared by native and web clients.

use serde::{Deserialize, Serialize};

/// Auto recognizes GitHub and GitLab.com remotes. Self-hosted GitLab is explicit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Forge {
    #[default]
    Auto,
    GitHub,
    GitLab,
}
fn legacy_forge() -> Forge {
    Forge::GitHub
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForgeRepository {
    pub forge: Forge,
    pub host: String,
    pub project: String,
    pub remote_url: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackerProvider {
    Linear,
    Jira,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackerConfig {
    pub provider: TrackerProvider,
    pub origin: String,
    pub email: String,
    #[serde(default)]
    pub oauth: bool,
}
/// Credential is accepted once and never serialized into settings or receipts.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackerSetup {
    pub config: TrackerConfig,
    pub token: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackerSetting {
    pub config: TrackerConfig,
    pub credential_saved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdin: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectScripts {
    #[serde(default)]
    pub setup: Vec<Command>,
    #[serde(default)]
    pub run: Vec<Command>,
    #[serde(default)]
    pub teardown: Vec<Command>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecyclePhase {
    Setup,
    Run,
    Teardown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewEvent {
    Comment,
    Approve,
    RequestChanges,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DiffSide {
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineComment {
    pub path: String,
    pub line: u32,
    pub side: DiffSide,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeliveryAction {
    CreatePull {
        title: String,
        body: String,
        base: String,
        head: String,
        draft: bool,
    },
    CommitPushPull {
        files: Vec<String>,
        message: String,
        title: String,
        body: String,
        base: String,
        head: String,
        draft: bool,
    },
    Review {
        number: u64,
        head_oid: String,
        event: ReviewEvent,
        body: String,
        #[serde(default)]
        comments: Vec<LineComment>,
    },
    Comment {
        number: u64,
        body: String,
    },
    RequestReviewers {
        number: u64,
        reviewers: Vec<String>,
    },
    Merge {
        number: u64,
        head_oid: String,
        method: MergeMethod,
        auto: bool,
    },
    RebaseStack {
        number: u64,
    },
    MergeStack {
        number: u64,
        method: MergeMethod,
    },
    /// Owner execution must separately acknowledge the server-current-head authority.
    MergeNativeStack {
        number: u64,
        method: MergeMethod,
    },
    Lifecycle {
        phase: LifecyclePhase,
    },
    StartTrackerIssue {
        provider: TrackerProvider,
        reference: String,
        base: String,
        branch: String,
        run_setup: bool,
    },
    StartIssue {
        number: u64,
        base: String,
        branch: String,
        run_setup: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareRequest {
    #[serde(default)]
    pub forge: Forge,
    pub path: String,
    pub action: DeliveryAction,
    /// Explicit execution budget, including setup commands. The UI asks for it.
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedAction {
    #[serde(default = "legacy_forge")]
    pub forge: Forge,
    #[serde(default)]
    pub repository: Option<ForgeRepository>,
    pub id: String,
    pub path: String,
    pub action: DeliveryAction,
    pub commands: Vec<Command>,
    pub expected_head: String,
    pub expected_branch: String,
    pub worktree_fingerprint: Option<String>,
    pub timeout_seconds: u64,
    pub created_at_ms: i64,
    pub workspace_path: Option<String>,
    pub issue: Option<Issue>,
    #[serde(default)]
    pub stack: Option<StackPlan>,
    #[serde(default)]
    pub native_stack: Option<NativeStackPlan>,
}

/// A native stack preview. Heads are observational, not atomic merge leases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeStackPlan {
    pub repository: String,
    pub stack_number: u64,
    pub base: String,
    pub base_oid: String,
    pub merge_through: u64,
    pub layers: Vec<StackLayer>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeMergeState {
    Submitting,
    Pending,
    Enqueued,
    Merged,
    Failed,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeMergeOperation {
    pub acknowledged_server_heads: bool,
    pub acknowledged_at_ms: i64,
    pub preflight: NativeStackPlan,
    pub operation_id: Option<String>,
    pub state: NativeMergeState,
    pub server_head_oid: Option<String>,
    pub merge_commit_oid: Option<String>,
    pub message: String,
}

/// Reviewed stack membership and immutable commit leases. Rebase result commits
/// are derived in an isolated worktree and published to local refs atomically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackPlan {
    pub repository: String,
    pub base: String,
    pub base_oid: String,
    #[serde(default)]
    pub local_base_oid: Option<String>,
    pub layers: Vec<StackLayer>,
    pub temporary_worktree: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackLayer {
    pub number: u64,
    pub head: String,
    pub base: String,
    pub head_oid: String,
    pub base_oid: String,
    pub local_oid: Option<String>,
    pub replay_after_oid: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionStatus {
    Prepared,
    Running,
    Completed,
    Failed,
    Interrupted,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionReceipt {
    pub prepared: PreparedAction,
    pub status: ActionStatus,
    pub completed_commands: usize,
    pub output: String,
    pub error: Option<String>,
    pub updated_at_ms: i64,
    #[serde(default)]
    pub native_operation: Option<NativeMergeOperation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub state: String,
    pub conclusion: Option<String>,
    pub url: Option<String>,
}

impl Check {
    pub fn failed(&self) -> bool {
        matches!(
            self.conclusion.as_deref().unwrap_or(&self.state),
            "FAILURE" | "ERROR" | "TIMED_OUT" | "ACTION_REQUIRED" | "STARTUP_FAILURE"
        )
    }
    pub fn pending(&self) -> bool {
        matches!(
            self.state.as_str(),
            "QUEUED" | "IN_PROGRESS" | "PENDING" | "WAITING" | "REQUESTED" | "EXPECTED"
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pull {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub url: String,
    pub state: String,
    pub draft: bool,
    pub base: String,
    pub head: String,
    pub head_oid: String,
    #[serde(default)]
    pub head_in_repository: bool,
    #[serde(default)]
    pub base_oid: String,
    #[serde(default)]
    pub merge_commit_oid: Option<String>,
    pub updated_at: String,
    pub mergeable: String,
    pub review_decision: Option<String>,
    pub checks: Vec<Check>,
    pub comments: Vec<PullComment>,
    pub reviews: Vec<PullComment>,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullComment {
    pub id: String,
    pub author: String,
    pub body: String,
    pub url: Option<String>,
    pub state: Option<String>,
    pub path: Option<String>,
    pub line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackLink {
    pub number: u64,
    pub parent: u64,
}

/// Stack edges come from branch relationships, never from matching titles.
pub fn stack_links(pulls: &[Pull]) -> Vec<StackLink> {
    pulls
        .iter()
        .filter_map(|child| {
            let candidates = pulls
                .iter()
                .filter(|parent| {
                    parent.number != child.number
                        && parent.head_in_repository
                        && parent.head == child.base
                })
                .collect::<Vec<_>>();
            match candidates.as_slice() {
                [parent] => Some(StackLink {
                    number: child.number,
                    parent: parent.number,
                }),
                _ => None,
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WatchRequest {
    #[serde(default)]
    pub forge: Forge,
    pub path: String,
    pub number: u64,
    pub poll_seconds: u64,
    /// Optional existing Custom reactive source, with separately reviewed subscribers.
    pub reactive_source_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullWatch {
    pub id: String,
    pub request: WatchRequest,
    #[serde(default)]
    pub repository: Option<ForgeRepository>,
    pub enabled: bool,
    pub next_poll_at_ms: i64,
    pub previous: Option<Pull>,
    pub owner_login: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchEvent {
    pub id: String,
    pub watch_id: String,
    pub number: u64,
    pub title: String,
    pub reasons: Vec<String>,
    pub url: String,
    pub created_at_ms: i64,
    pub delivered: bool,
    pub acknowledged: bool,
    pub reactive_source_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliverySnapshot {
    pub actions: Vec<ActionReceipt>,
    pub watches: Vec<PullWatch>,
    pub events: Vec<WatchEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeliveryQuery {
    Pulls,
    Pull {
        number: u64,
    },
    Diff {
        number: u64,
    },
    LineComments {
        number: u64,
    },
    Issue {
        number: u64,
    },
    Scripts,
    TrackerIssue {
        provider: TrackerProvider,
        reference: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadRequest {
    #[serde(default)]
    pub forge: Forge,
    pub path: String,
    pub query: DeliveryQuery,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ReadResult {
    Pulls {
        pulls: Vec<Pull>,
        stacks: Vec<StackLink>,
    },
    Pull {
        pull: Box<Pull>,
    },
    Diff {
        patch: String,
    },
    LineComments {
        comments: Vec<PullComment>,
    },
    Issue {
        issue: Issue,
    },
    Scripts {
        scripts: ProjectScripts,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionId {
    pub id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_distinguish_failures_from_work_and_cancelled_checks() {
        let check = |state: &str, conclusion: Option<&str>| Check {
            name: "tests".into(),
            state: state.into(),
            conclusion: conclusion.map(str::to_owned),
            url: None,
        };
        assert!(check("COMPLETED", Some("FAILURE")).failed());
        assert!(check("IN_PROGRESS", None).pending());
        assert!(check("PENDING", None).pending());
        assert!(!check("COMPLETED", Some("CANCELLED")).failed());
        assert!(!check("COMPLETED", Some("SUCCESS")).pending());
    }
}
