//! Reviewable repository delivery operations shared by native and web clients.

pub mod generation;
pub mod writer_catalog;
use generation::GenerationOptions;
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
    /// Owner-reviewed checks run in a separate checkout of selected changes.
    #[serde(default)]
    pub check: Vec<Command>,
    #[serde(default)]
    pub teardown: Vec<Command>,
    #[serde(default)]
    pub worktree: WorktreeConfig,
}

/// How bruhs fills a new worktree of this project (`.bruhs/config.json`
/// `worktree`). By default the checkout's ignored build folders are cloned
/// into it copy-on-write where the filesystem supports that; see
/// `docs/worktree-disk.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeConfig {
    /// `false` reuses no build folder: the worktree is only Git's checkout.
    #[serde(default = "copy_on_write_default")]
    pub copy_on_write: bool,
    /// Ignored folders, relative to the repository root, to clone into a new
    /// worktree besides the detected Cargo `target/debug` and `node_modules`
    /// folders. Folders that store absolute paths, such as Python virtual
    /// environments, break when cloned to another place.
    #[serde(default)]
    pub build_dirs: Vec<String>,
}

fn copy_on_write_default() -> bool {
    true
}

impl Default for WorktreeConfig {
    fn default() -> Self {
        Self {
            copy_on_write: true,
            build_dirs: Vec::new(),
        }
    }
}

/// At most this many configured build folders are read.
pub const MAX_WORKTREE_BUILD_DIRS: usize = 32;

impl WorktreeConfig {
    /// The configured build folders that stay inside the repository: relative,
    /// without `.`/`..` segments, outside `.git`, and at most
    /// [`MAX_WORKTREE_BUILD_DIRS`] of them. Trailing slashes are dropped.
    pub fn valid_build_dirs(&self) -> Vec<&str> {
        self.build_dirs
            .iter()
            .map(|dir| dir.trim_end_matches('/'))
            .filter(|dir| is_repository_relative(dir))
            .take(MAX_WORKTREE_BUILD_DIRS)
            .collect()
    }
}

/// `path` names something inside a repository: non-empty `/`-separated
/// segments, none of them `.`, `..` or `.git` (in any case), no backslash
/// and no NUL.
pub fn is_repository_relative(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 1024
        && !path.contains(['\\', '\0'])
        && path.split('/').all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && !segment.eq_ignore_ascii_case(".git")
        })
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
        #[serde(default)]
        verification_id: Option<String>,
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
    /// Run configured checks against HEAD plus exactly these selected files.
    Verify {
        files: Vec<String>,
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

/// The host chooses the task's working directory and source; callers supply only
/// the action they want to review. Source context never grants execution access.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskPrepareRequest {
    #[serde(default)]
    pub forge: Forge,
    pub action: DeliveryAction,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDeliverySource {
    pub task_id: crate::ids::ThreadId,
    pub title: String,
    pub parent_id: Option<crate::ids::ThreadId>,
    pub work_item_origin: Option<crate::workbench_handoff::WorkItemTaskOrigin>,
}

impl From<&crate::coordination::CoordinatedTask> for TaskDeliverySource {
    fn from(task: &crate::coordination::CoordinatedTask) -> Self {
        Self {
            task_id: task.id.clone(),
            title: task.title.clone(),
            parent_id: task.parent_id.clone(),
            work_item_origin: task.work_item_origin.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDeliveryContext {
    pub source: TaskDeliverySource,
    pub path: String,
    pub branch: String,
    pub head: String,
    pub state: crate::coordination::TaskState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedAction {
    #[serde(default = "legacy_forge")]
    pub forge: Forge,
    #[serde(default)]
    pub repository: Option<ForgeRepository>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_task: Option<TaskDeliverySource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<SourceVerification>,
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

/// Evidence copied by the host from a completed independent check receipt.
/// It is revalidated against the selected payload before delivery starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceVerification {
    pub receipt_id: String,
    pub source_head: String,
    pub source_fingerprint: String,
    #[serde(default)]
    pub checked_tree: String,
    pub files: Vec<String>,
    pub commands: Vec<Command>,
    pub completed_at_ms: i64,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_tree: Option<String>,
    /// Immutable commit selected for push; not a claim that the remote accepted it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_commit: Option<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangedFileCount {
    Exact(usize),
    Capped,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed_files: Option<ChangedFileCount>,
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
        #[serde(default)]
        review_guides_enabled: bool,
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
    fn task_delivery_requests_cannot_substitute_a_path_or_source_snapshot() {
        let request = serde_json::json!({
            "forge":"git_hub", "action":{"type":"comment","number":42,"body":"Owner review"},
            "timeout_seconds":5
        });
        assert!(serde_json::from_value::<TaskPrepareRequest>(request.clone()).is_ok());
        for field in ["path", "source_task", "task_id"] {
            let mut substituted = request.clone();
            substituted[field] = serde_json::json!("another-project");
            assert!(serde_json::from_value::<TaskPrepareRequest>(substituted).is_err());
        }
    }

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

    #[test]
    fn worktree_config_defaults_to_copy_on_write_and_keeps_paths_inside_the_repository() {
        let scripts: ProjectScripts = serde_json::from_str(r#"{"setup":[]}"#).unwrap();
        assert_eq!(scripts.worktree, WorktreeConfig::default());
        assert!(scripts.worktree.copy_on_write);

        let scripts: ProjectScripts = serde_json::from_str(
            r#"{"worktree":{"copy_on_write":false,"build_dirs":["vendor/bundle/","../up","/abs",".git/x","a/./b","web/.cache",""]}}"#,
        )
        .unwrap();
        assert!(!scripts.worktree.copy_on_write);
        assert_eq!(
            scripts.worktree.valid_build_dirs(),
            ["vendor/bundle", "web/.cache"]
        );
        assert!(serde_json::from_str::<ProjectScripts>(r#"{"worktree":{"clone":true}}"#).is_err());
        assert!(!is_repository_relative("a\\..\\b"));
        assert!(!is_repository_relative(".GIT"));
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WritingStyle {
    #[default]
    Repository,
    Conventional,
    Custom,
}
impl WritingStyle {
    pub fn key(self) -> &'static str {
        match self {
            Self::Repository => "repository",
            Self::Conventional => "conventional",
            Self::Custom => "custom",
        }
    }
}

/// A project may explicitly follow its text generator even when its environment has a dedicated writer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WriterModel {
    #[default]
    TextGeneration,
    Separate {
        model: String,
        #[serde(default, skip_serializing_if = "GenerationOptions::is_empty")]
        options: GenerationOptions,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WriterDraft {
    Commit {
        files: Vec<String>,
    },
    Pull {
        base: String,
        head: String,
        #[serde(default)]
        files: Vec<String>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WriterDraftRequest {
    pub path: String,
    pub draft: WriterDraft,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WriterText {
    pub title: String,
    #[serde(default)]
    pub body: String,
}

/// Per-repository overrides inherit individual fields from the delivery host.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DeliveryOverrides {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_guides_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_guide_model: Option<GuideModel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_guide_instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub writer_model: Option<WriterModel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub writing_style: Option<WritingStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub writer_instructions: Option<String>,
    pub merge_method: Option<MergeMethod>,
    pub follow_templates: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remove_agent_credits: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryPreferences {
    #[serde(default)]
    pub review_guides_enabled: bool,
    #[serde(default)]
    pub review_guide_model: GuideModel,
    #[serde(default)]
    pub review_guide_instructions: String,
    #[serde(default)]
    pub writer_model: WriterModel,
    #[serde(default)]
    pub writing_style: WritingStyle,
    #[serde(default)]
    pub writer_instructions: String,
    pub merge_method: MergeMethod,
    pub follow_templates: bool,
    #[serde(default)]
    pub remove_agent_credits: bool,
    pub overrides: DeliveryOverrides,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryPreferenceScope {
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "setting",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum DeliveryPreferenceChange {
    ReviewGuidesEnabled(Option<bool>),
    ReviewGuideModel(Option<GuideModel>),
    ReviewGuideInstructions(Option<String>),
    WriterModel(Option<WriterModel>),
    GenerationOption {
        target: generation::GenerationTarget,
        model: String,
        kind: generation::OptionKind,
        value: String,
    },
    WritingStyle(Option<WritingStyle>),
    WriterInstructions(Option<String>),
    ResetWritingStyle,
    MergeMethod(Option<MergeMethod>),
    FollowTemplates(Option<bool>),
    RemoveAgentCredits(Option<bool>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetDeliveryPreference {
    pub path: Option<String>,
    pub change: DeliveryPreferenceChange,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GuideModel {
    #[default]
    Writer,
    Separate {
        model: String,
        #[serde(default, skip_serializing_if = "GenerationOptions::is_empty")]
        options: GenerationOptions,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewGuideRequest {
    pub path: String,
    #[serde(default)]
    pub forge: Forge,
    pub number: u64,
    pub head_oid: String,
    pub base_oid: String,
    #[serde(default)]
    pub generate: bool,
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReviewGuide {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_base_oid: Option<String>,
    pub model: String,
    pub head_oid: String,
    pub base_oid: String,
    pub stops: Vec<GuideStop>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct GuideStop {
    pub title: String,
    pub summary: String,
    pub attention: Option<GuideAttention>,
    #[serde(default)]
    pub files: Vec<GuideFile>,
    #[serde(default)]
    pub excerpts: Vec<GuideExcerpt>,
    #[serde(default)]
    pub questions: Vec<String>,
    #[serde(default)]
    pub notes: Vec<GuideNote>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct GuideAttention {
    pub kind: AttentionKind,
    pub note: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AttentionKind {
    Tradeoff,
    Contract,
    Product,
    Risk,
    Accessibility,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct GuideFile {
    #[serde(default)]
    pub source_side: GuideSide,
    pub path: String,
    pub note: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct GuideNote {
    pub path: String,
    pub line: usize,
    pub title: String,
    pub body: String,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuideSide {
    Old,
    #[default]
    New,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct GuideExcerpt {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    pub path: String,
    pub side: GuideSide,
    pub start: usize,
    pub end: usize,
    pub caption: String,
    #[serde(default)]
    pub code: String,
}
