use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Shell,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Running,
    Exited,
    Killed,
    Failed,
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
