//! Where a new thread runs: the current checkout, a new worktree, or the
//! previous worktree. Claude effort levels are the ones `claude --help`
//! lists for `--effort`.

use serde::{Deserialize, Serialize};

/// Static prefix for a branch created with a new worktree. Settings do not
/// rename it.
pub const WORKTREE_BRANCH_PREFIX: &str = "bruhs/";

/// `--effort` values Claude Code 2.1.311 accepts.
pub const CLAUDE_EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// The workspace menu on a new thread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceChoice {
    /// Run in the project's checkout.
    #[default]
    Checkout,
    /// Create a worktree from the selected base branch on the first send.
    NewWorktree,
    /// Run in the last worktree this client created.
    PreviousWorktree,
}

impl WorkspaceChoice {
    pub fn label(self) -> &'static str {
        match self {
            Self::Checkout => "Current checkout",
            Self::NewWorktree => "New worktree",
            Self::PreviousWorktree => "Previous worktree",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Checkout => "checkout",
            Self::NewWorktree => "new_worktree",
            Self::PreviousWorktree => "previous_worktree",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "new_worktree" => Self::NewWorktree,
            "previous_worktree" => Self::PreviousWorktree,
            _ => Self::Checkout,
        }
    }
}

/// A worktree the client can offer as "previous".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RememberedWorktree {
    pub path: String,
    pub branch: String,
    pub label: String,
}

/// `Some` when `value` is one of [`CLAUDE_EFFORTS`].
pub fn claude_effort(value: &str) -> Option<&'static str> {
    CLAUDE_EFFORTS.iter().copied().find(|level| *level == value)
}

/// Branch name for a worktree of `thread_id`: `bruhs/` plus up to 12 hex digits.
pub fn worktree_branch(thread_id: &str) -> String {
    let slug: String = thread_id
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .take(12)
        .flat_map(|c| c.to_lowercase())
        .collect();
    let slug = if slug.is_empty() {
        "thread".to_string()
    } else {
        slug
    };
    format!("{WORKTREE_BRANCH_PREFIX}{slug}")
}

/// One path component for the project directory under the worktrees root.
pub fn worktree_project_dir(project_path: &str) -> String {
    let name = project_path
        .rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or("project");
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if safe.is_empty() {
        "project".to_string()
    } else {
        safe
    }
}

/// Directory name for `branch`, with slashes turned into hyphens.
pub fn worktree_dir_name(branch: &str) -> String {
    branch.replace('/', "-")
}

/// Where the first send of a thread should run. Later sends stay put.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FirstSend {
    /// Keep the thread's project path.
    Stay,
    /// `git worktree add -b branch` from `base` in the project.
    Create { base: String, branch: String },
    /// Run in a worktree this client already created.
    UsePath { path: String, branch: String },
}

/// Decide the first send. A locked thread, and a thread with no project,
/// stays where it is. `base_branch` empty means the current git branch.
pub fn first_send(
    run_locked: bool,
    choice: WorkspaceChoice,
    project_path: Option<&str>,
    base_branch: &str,
    current_branch: Option<&str>,
    previous: Option<&RememberedWorktree>,
    thread_id: &str,
) -> Result<FirstSend, &'static str> {
    if run_locked || project_path.is_none() {
        return Ok(FirstSend::Stay);
    }
    match choice {
        WorkspaceChoice::Checkout => Ok(FirstSend::Stay),
        WorkspaceChoice::NewWorktree => {
            let base = if !base_branch.is_empty() {
                base_branch
            } else if let Some(current) = current_branch {
                current
            } else {
                return Err("Wait for the branch list, then choose a base branch.");
            };
            if !crate::logic::git::is_valid_branch(base) {
                return Err("Choose a branch this repository has.");
            }
            Ok(FirstSend::Create {
                base: base.to_string(),
                branch: worktree_branch(thread_id),
            })
        }
        WorkspaceChoice::PreviousWorktree => match previous {
            Some(prev) if !prev.path.is_empty() => Ok(FirstSend::UsePath {
                path: prev.path.clone(),
                branch: prev.branch.clone(),
            }),
            _ => Err("No previous worktree."),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::git::is_valid_branch;

    #[test]
    fn a_worktree_branch_is_a_bruhs_prefixed_name_git_accepts() {
        let branch = worktree_branch("A1B2C3D4-E5F6-7890-ABCD-EF1234567890");
        assert_eq!(branch, "bruhs/a1b2c3d4e5f6");
        assert!(is_valid_branch(&branch));
        assert_eq!(worktree_branch("xyz"), "bruhs/thread");
        assert!(is_valid_branch(&worktree_branch("")));
    }

    #[test]
    fn claude_effort_accepts_only_the_cli_list() {
        assert_eq!(claude_effort("high"), Some("high"));
        assert_eq!(claude_effort("max"), Some("max"));
        assert_eq!(claude_effort("minimal"), None);
        assert_eq!(claude_effort(""), None);
    }

    #[test]
    fn workspace_choice_round_trips_its_pref_and_hides_unknown_values() {
        assert_eq!(
            WorkspaceChoice::parse(WorkspaceChoice::NewWorktree.as_str()),
            WorkspaceChoice::NewWorktree
        );
        assert_eq!(WorkspaceChoice::parse("nope"), WorkspaceChoice::Checkout);
        assert_eq!(
            WorkspaceChoice::PreviousWorktree.label(),
            "Previous worktree"
        );
    }

    #[test]
    fn worktree_paths_stay_single_components() {
        assert_eq!(worktree_project_dir("/repo/bruhs ai"), "bruhs-ai");
        assert_eq!(worktree_dir_name("bruhs/abc"), "bruhs-abc");
    }

    fn remembered(path: &str) -> RememberedWorktree {
        RememberedWorktree {
            path: path.into(),
            branch: "bruhs/prev".into(),
            label: "Prev".into(),
        }
    }

    #[test]
    fn the_first_send_stays_checks_out_or_creates_from_the_current_branch() {
        let stay = first_send(
            false,
            WorkspaceChoice::Checkout,
            Some("/repo"),
            "",
            Some("feature/x"),
            None,
            "abc",
        )
        .unwrap();
        assert_eq!(stay, FirstSend::Stay);
        let created = first_send(
            false,
            WorkspaceChoice::NewWorktree,
            Some("/repo"),
            "",
            Some("feature/x"),
            None,
            "A1B2C3D4-E5F6",
        )
        .unwrap();
        assert_eq!(
            created,
            FirstSend::Create {
                base: "feature/x".into(),
                branch: "bruhs/a1b2c3d4e5f6".into(),
            }
        );
        let picked = first_send(
            false,
            WorkspaceChoice::NewWorktree,
            Some("/repo"),
            "main",
            Some("feature/x"),
            None,
            "abc",
        )
        .unwrap();
        assert!(matches!(picked, FirstSend::Create { base, .. } if base == "main"));
    }

    #[test]
    fn a_locked_thread_and_a_general_thread_do_not_move() {
        assert_eq!(
            first_send(
                true,
                WorkspaceChoice::NewWorktree,
                Some("/repo"),
                "main",
                Some("main"),
                None,
                "abc",
            )
            .unwrap(),
            FirstSend::Stay
        );
        assert_eq!(
            first_send(
                false,
                WorkspaceChoice::NewWorktree,
                None,
                "main",
                Some("main"),
                None,
                "abc",
            )
            .unwrap(),
            FirstSend::Stay
        );
    }

    #[test]
    fn previous_worktree_needs_a_remembered_path_and_a_missing_base_waits() {
        let previous = remembered("/wt");
        assert_eq!(
            first_send(
                false,
                WorkspaceChoice::PreviousWorktree,
                Some("/repo"),
                "",
                Some("main"),
                Some(&previous),
                "abc",
            )
            .unwrap(),
            FirstSend::UsePath {
                path: "/wt".into(),
                branch: "bruhs/prev".into(),
            }
        );
        assert_eq!(
            first_send(
                false,
                WorkspaceChoice::PreviousWorktree,
                Some("/repo"),
                "",
                Some("main"),
                None,
                "abc",
            )
            .unwrap_err(),
            "No previous worktree."
        );
        assert_eq!(
            first_send(
                false,
                WorkspaceChoice::NewWorktree,
                Some("/repo"),
                "",
                None,
                None,
                "abc",
            )
            .unwrap_err(),
            "Wait for the branch list, then choose a base branch."
        );
        assert_eq!(
            first_send(
                false,
                WorkspaceChoice::NewWorktree,
                Some("/repo"),
                "bad..name",
                Some("main"),
                None,
                "abc",
            )
            .unwrap_err(),
            "Choose a branch this repository has."
        );
    }
}
