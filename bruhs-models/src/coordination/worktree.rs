//! A task's own worktree: what it costs on disk and whether it can be
//! removed. Removing deletes the folder and keeps the branch, so nothing
//! committed is lost and the task's next run checks the branch out again.

use crate::EpochMs;
use serde::{Deserialize, Serialize};

/// A worktree the owner removed. `branch` is checked out again at the
/// task's working path before its next run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemovedWorktree {
    pub branch: String,
    pub removed_at: EpochMs,
}

/// Where a task's worktree stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeState {
    /// The task works in the project folder itself; there is nothing to remove.
    Shared,
    /// Its own worktree is on disk.
    Present,
    /// The owner removed it; its branch remains.
    Removed,
    /// The folder is gone, but not through Remove.
    Missing,
}

/// What deleting a worktree's folder would free, measured from the
/// filesystem's own accounting (APFS private sizes on macOS).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeFrees {
    /// Bytes deleting it frees at once.
    pub bytes: u64,
    /// Bytes that stay because clones or hard links elsewhere share them,
    /// such as build folders shared with the project checkout.
    pub shared: u64,
    /// Bytes that stay until local snapshots holding them are gone.
    pub snapshots: u64,
    /// Folders that could not be read, so their contents are not counted.
    pub unreadable: u64,
}

/// One task's worktree, for the task panel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskWorktree {
    pub state: WorktreeState,
    pub path: String,
    /// The checked-out branch, or the branch kept after removal.
    pub branch: Option<String>,
    /// Tracked changes and untracked files Git would not delete.
    pub changed_files: u32,
    /// Measured only on request and only where the platform reports private
    /// sizes; `None` otherwise, rather than an overstated logical size.
    pub frees: Option<WorktreeFrees>,
    /// Why the configured meter could not measure, when it was asked to.
    pub measure_error: Option<String>,
    /// Why Remove is refused right now, in words for the owner.
    pub blocked: Option<String>,
}

impl TaskWorktree {
    /// A worktree on disk, or one whose folder vanished but Git still lists,
    /// can be removed when nothing blocks it.
    pub fn removable(&self) -> bool {
        matches!(self.state, WorktreeState::Present | WorktreeState::Missing)
            && self.blocked.is_none()
    }
}

/// A worktree bruhs removed, kept so it can be checked out again: by the
/// next turn in that folder, or explicitly from another base.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemovedRecord {
    /// The main checkout the worktree belonged to.
    pub project: String,
    pub branch: String,
    pub removed_at: EpochMs,
}

/// The Workbench task working in a worktree, if any.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeTask {
    pub id: crate::ids::ThreadId,
    pub title: String,
}

/// One worktree in bruhs's worktrees folder: chat, task or issue work.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedWorktree {
    pub path: String,
    /// The main checkout it belongs to, when Git can tell.
    pub project: Option<String>,
    pub state: WorktreeState,
    pub branch: Option<String>,
    pub changed_files: u32,
    pub task: Option<WorktreeTask>,
    /// When the folder last changed, or when it was removed.
    pub updated_at: Option<EpochMs>,
    /// Removed with a branch that no longer exists, so it cannot come back
    /// on its own; it can start again from another base or be forgotten.
    #[serde(default)]
    pub branch_missing: bool,
    /// Why Remove is refused right now.
    pub blocked: Option<String>,
}
