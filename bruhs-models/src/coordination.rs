//! Server-owned task workspaces. Legacy chat history remains owned by its client.
use crate::{agent::PermissionMode, ids::ThreadId, EpochMs};
use serde::{Deserialize, Serialize};

pub mod board;
pub mod linked_thread;
pub mod worktree;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Ready,
    Running,
    WaitingApproval,
    WaitingChild,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
    Limited,
}

impl TaskState {
    pub fn active(self) -> bool {
        matches!(
            self,
            Self::Running | Self::WaitingApproval | Self::WaitingChild
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relationship {
    Root,
    Delegate,
    Fork,
    SideChat,
    Verify,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Isolation {
    Shared,
    Worktree { base_branch: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewTask {
    pub title: String,
    pub project_path: String,
    pub prompt: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_timeout_seconds: Option<u64>,
    #[serde(default)]
    pub permission_mode: PermissionMode,
    pub isolation: Isolation,
    #[serde(default)]
    pub spec_id: Option<ThreadId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskMessage {
    pub role: String,
    pub text: String,
    pub at: EpochMs,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingApproval {
    pub id: String,
    pub tool: String,
    pub input: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CoordinatedTask {
    pub id: ThreadId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_item_origin: Option<crate::workbench_handoff::WorkItemTaskOrigin>,
    pub parent_id: Option<ThreadId>,
    #[serde(default)]
    pub source_context: Vec<TaskMessage>,
    #[serde(default)]
    pub source_revision: Option<u64>,
    pub relationship: Relationship,
    pub title: String,
    pub project_path: String,
    pub working_path: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_timeout_seconds: Option<u64>,
    pub permission_mode: PermissionMode,
    pub spec_id: Option<ThreadId>,
    pub spec_revision: Option<u64>,
    pub state: TaskState,
    pub messages: Vec<TaskMessage>,
    pub live_output: String,
    pub error: Option<String>,
    pub session_id: Option<String>,
    pub approvals: Vec<PendingApproval>,
    pub created_at: EpochMs,
    pub updated_at: EpochMs,
    #[serde(default)]
    pub resume_at: Option<EpochMs>,
    #[serde(default)]
    pub auto_resume_on_limit: bool,
    #[serde(default)]
    pub limit_reported: bool,
    /// Set when the owner archives a finished task; it leaves the board's
    /// lanes but keeps its record. A new message brings it back.
    #[serde(default)]
    pub archived_at: Option<EpochMs>,
    /// Set when the owner removed this task's worktree to free disk. Its
    /// branch stays, and the task's next run checks it out again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed_worktree: Option<worktree::RemovedWorktree>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LivingSpec {
    pub id: ThreadId,
    pub project_path: String,
    pub title: String,
    pub content: String,
    pub revision: u64,
    pub updated_at: EpochMs,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuestProposal {
    pub id: ThreadId,
    pub task_id: ThreadId,
    pub text: String,
    pub at: EpochMs,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CoordinationSnapshot {
    #[serde(default)]
    pub can_control: bool,
    #[serde(default)]
    pub revision: u64,
    pub tasks: Vec<CoordinatedTask>,
    pub specs: Vec<LivingSpec>,
    pub guest_proposals: Vec<GuestProposal>,
    #[serde(default)]
    pub wake_subscriptions: Vec<WakeSubscription>,
    #[serde(default)]
    pub wakes: Vec<TaskWake>,
    #[serde(default)]
    pub wake_error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeKind {
    Checks,
    Review,
    Head,
    Conflict,
    Terminal,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WakeSubscription {
    pub id: ThreadId,
    pub task_id: ThreadId,
    pub watch_id: String,
    pub kinds: Vec<WakeKind>,
    pub project_path: String,
    pub model: String,
    pub permission_mode: PermissionMode,
    pub enabled: bool,
    pub approved_from_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskWakeState {
    Queued,
    Reserved,
    Completed,
    Unconfirmed,
    Blocked,
    Dismissed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskWake {
    pub id: ThreadId,
    pub subscription_id: ThreadId,
    pub task_id: ThreadId,
    pub event_id: String,
    pub event_digest: String,
    pub created_at_ms: i64,
    #[serde(default)]
    pub queued_revision: u64,
    pub prompt: String,
    pub state: TaskWakeState,
    pub outcome: Option<TaskState>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum CoordinationCommand {
    Create {
        task: NewTask,
        #[serde(default)]
        parent_id: Option<ThreadId>,
    },
    Send {
        id: ThreadId,
        prompt: String,
    },
    SetEffort {
        id: ThreadId,
        effort: Option<String>,
    },
    SetExecutionBudget {
        id: ThreadId,
        seconds: Option<u64>,
    },
    Cancel {
        id: ThreadId,
    },
    Fork {
        id: ThreadId,
        title: String,
        prompt: String,
        relationship: Relationship,
        model: String,
        #[serde(default)]
        effort: Option<String>,
        isolation: Isolation,
    },
    Fanout {
        task: NewTask,
        models: Vec<String>,
        #[serde(default)]
        parent_id: Option<ThreadId>,
    },
    BringBack {
        id: ThreadId,
    },
    SaveSpec {
        id: Option<ThreadId>,
        project_path: String,
        title: String,
        content: String,
        expected_revision: u64,
    },
    AttachSpec {
        id: ThreadId,
        spec_id: ThreadId,
    },
    Approval {
        id: ThreadId,
        request_id: String,
        allow: bool,
    },
    GuestProposal {
        id: ThreadId,
        accept: bool,
    },
    Delete {
        id: ThreadId,
    },
    ScheduleReset {
        id: ThreadId,
    },
    CancelResume {
        id: ThreadId,
    },
    AutoResume {
        id: ThreadId,
        enabled: bool,
    },
    SaveWakeSubscription {
        task_id: ThreadId,
        watch_id: String,
        kinds: Vec<WakeKind>,
        expected_model: String,
        expected_permission_mode: PermissionMode,
    },
    SetWakeSubscription {
        id: ThreadId,
        enabled: bool,
    },
    DismissWake {
        id: ThreadId,
    },
    Archive {
        id: ThreadId,
    },
    Unarchive {
        id: ThreadId,
    },
    /// Delete a finished task's own worktree folder, keeping its branch.
    RemoveWorktree {
        id: ThreadId,
    },
}

pub fn validate_new_task(task: &NewTask) -> Result<(), &'static str> {
    if task.execution_timeout_seconds == Some(0) {
        return Err("Choose a positive execution budget or leave it unlimited.");
    }
    if task.title.trim().is_empty() {
        return Err("Task title cannot be empty.");
    }
    if task.prompt.trim().is_empty() {
        return Err("Task prompt cannot be empty.");
    }
    if task.project_path.trim().is_empty() {
        return Err("Choose a project folder.");
    }
    if task.prompt.chars().count() > crate::logic::composer_text::MESSAGE_CHAR_LIMIT {
        return Err(crate::logic::composer_text::MESSAGE_LIMIT_HINT);
    }
    if task.model.parse::<crate::agent::ModelId>().is_err() {
        return Err("Choose a valid runtime and model.");
    }
    validate_task_effort(&task.model, task.effort.as_deref())?;
    if let Isolation::Worktree { base_branch } = &task.isolation {
        if crate::logic::git::BranchName::try_from(base_branch.as_str()).is_err() {
            return Err("Choose a valid worktree base branch.");
        }
    }
    Ok(())
}

pub fn validate_task_effort(model: &str, effort: Option<&str>) -> Result<(), &'static str> {
    let Some(effort) = effort else { return Ok(()) };
    if crate::logic::run_place::claude_effort(effort).is_none() {
        return Err("Choose provider default, low, medium, high, xhigh or max reasoning effort.");
    }
    if !task_effort_supported(model) {
        return Err("Reasoning effort overrides are available for Claude and Codex tasks. Choose provider default for this runtime.");
    }
    Ok(())
}

pub fn task_effort_supported(model: &str) -> bool {
    matches!(
        crate::agent::provider_for_model_id(model),
        crate::agent::AgentProvider::Claude | crate::agent::AgentProvider::Codex
    )
}

pub fn narrower(parent: PermissionMode, child: PermissionMode) -> bool {
    let rank = |mode| match mode {
        PermissionMode::Plan => 0,
        PermissionMode::Default => 1,
        PermissionMode::AcceptEdits => 2,
        PermissionMode::BypassPermissions => 3,
    };
    rank(child) <= rank(parent)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task() -> NewTask {
        NewTask {
            title: "Implement change".into(),
            project_path: "/tmp/project".into(),
            prompt: "Fix the bug".into(),
            model: "codex/default".into(),
            effort: None,
            execution_timeout_seconds: None,
            permission_mode: PermissionMode::Default,
            isolation: Isolation::Shared,
            spec_id: None,
        }
    }

    #[test]
    fn effort_is_validated_for_the_runtime_and_legacy_requests_keep_default() {
        let mut task = task();
        let legacy = serde_json::to_value(&task).unwrap();
        assert!(legacy.get("effort").is_none());
        assert!(serde_json::from_value::<NewTask>(legacy)
            .unwrap()
            .effort
            .is_none());
        for level in crate::logic::run_place::CLAUDE_EFFORTS {
            task.effort = Some(level.into());
            validate_new_task(&task).unwrap();
        }
        task.effort = Some("turbo".into());
        assert!(validate_new_task(&task).is_err());
        task.effort = Some("high".into());
        task.model = "opencode/custom/local-model".into();
        assert!(validate_new_task(&task)
            .unwrap_err()
            .contains("provider default"));
        task.effort = None;
        validate_new_task(&task).unwrap();
    }

    #[test]
    fn reject_blank_prompt_before_starting_a_process() {
        let mut task = task();
        task.prompt = "  ".into();
        assert!(validate_new_task(&task).is_err());
    }

    #[test]
    fn child_may_only_narrow_parent_access() {
        assert!(!narrower(PermissionMode::Plan, PermissionMode::AcceptEdits));
        assert!(narrower(PermissionMode::Default, PermissionMode::Plan));
        assert!(narrower(
            PermissionMode::AcceptEdits,
            PermissionMode::Default
        ));
    }
}
