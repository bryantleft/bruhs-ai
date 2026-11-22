use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    #[default]
    Shell,
    Subagent,
    Swarm,
    BackgroundAgent,
}

impl TaskKind {
    pub const fn badge_label(self) -> &'static str {
        match self {
            Self::Shell => "Shell",
            Self::Subagent => "Subagent",
            Self::Swarm => "Teamwork",
            Self::BackgroundAgent => "Agent",
        }
    }

    pub const fn is_agent_work(self) -> bool {
        matches!(self, Self::Subagent | Self::Swarm | Self::BackgroundAgent)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Running,
    Exited,
    Killed,
    Failed,
}

impl TaskStatus {
    pub const fn succeeded(&self, exit_code: Option<i32>) -> bool {
        matches!(self, Self::Exited) && matches!(exit_code, None | Some(0))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundTask {
    pub id: String,
    pub thread_id: String,
    pub parent_task_id: Option<String>,
    pub kind: TaskKind,
    pub label: String,
    pub status: TaskStatus,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub exit_code: Option<i32>,
    pub meta: serde_json::Value,
    pub log_tail: String,
}

// No new() here — BackgroundTaskRegistry creates these

#[derive(Debug, Clone)]
pub enum TaskEvent {
    Created(BackgroundTask),
    Updated(BackgroundTask),
    Removed(String),
}
