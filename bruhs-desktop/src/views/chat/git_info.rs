//! The composer's git context (branch chip, dirty count, branch switcher)
//! and `@`-mention search, through bruhs-core off the UI thread.

use super::DesktopMentionItem;
use bruhs_core::files::{self, FileSearchError};
use bruhs_core::git;
use std::path::PathBuf;

/// What the composer shows about the project's repository.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct GitInfo {
    /// `None` when HEAD is detached or the project is not a repository.
    pub current: Option<String>,
    /// Local branches, current first, then most recent commit first.
    pub branches: Vec<String>,
    /// Uncommitted entries (web `GitStatus.dirty`).
    pub dirty: usize,
}

/// Branches and status of the repository at `dir`. A directory that is not
/// a repository (or a git failure) has no branches and nothing dirty.
pub(super) async fn load_git_info(dir: PathBuf) -> GitInfo {
    let (branches, status) = tokio::join!(git::branches(&dir), git::status(&dir));
    let (current, branches) = match branches {
        Ok(branches) => (branches.current, branches.branches),
        Err(error) => {
            tracing::debug!("no branches for {}: {error}", dir.display());
            (None, Vec::new())
        }
    };
    let dirty = match status {
        Ok(status) => status.dirty.len(),
        Err(error) => {
            tracing::debug!("no git status for {}: {error}", dir.display());
            0
        }
    };
    GitInfo {
        current,
        branches,
        dirty,
    }
}

/// `@`-mention matches for `query` under `root` (web `/api/agent/files`).
/// A failed search has no matches, like the web's.
pub(super) async fn mention_matches(root: PathBuf, query: String) -> Vec<DesktopMentionItem> {
    match files::search(&root, &query).await {
        Ok(entries) => entries
            .into_iter()
            .map(|entry| DesktopMentionItem {
                path: entry.path,
                name: entry.name,
                is_dir: entry.is_directory,
            })
            .collect(),
        // Only files git tracks are mentionable outside a directory query.
        Err(FileSearchError::NotARepository(_)) => Vec::new(),
        Err(error) => {
            tracing::warn!("file mention search failed in {}: {error}", root.display());
            Vec::new()
        }
    }
}
