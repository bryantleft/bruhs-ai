//! Whether Edit from here may put a worktree's files back.
//!
//! The snapshot itself is git on the host. This module only decides when the
//! conversation rewind may also restore that snapshot.

/// The message was sent before checkpoints existed, or the snapshot failed.
pub const NO_CHECKPOINT: &str = "No checkpoint for this message.";

/// Another thread's worktree is this directory, inside it, or contains it.
pub const DIRECTORY_IN_USE: &str = "Another thread is using this directory.";

/// Slash-separated path without a trailing slash (the root keeps its slash).
pub fn normalize_dir(path: &str) -> String {
    let mut out = path.trim().replace('\\', "/");
    while out.len() > 1 && out.ends_with('/') {
        out.pop();
    }
    out
}

/// `a` and `b` are the same directory, or one is inside the other.
///
/// `/repo` does not match `/repo-other`.
pub fn paths_overlap(a: &str, b: &str) -> bool {
    let a = normalize_dir(a);
    let b = normalize_dir(b);
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a == b {
        return true;
    }
    let (shorter, longer) = if a.len() <= b.len() {
        (&a, &b)
    } else {
        (&b, &a)
    };
    longer
        .strip_prefix(shorter.as_str())
        .is_some_and(|rest| rest.starts_with('/'))
}

/// File restore is offered only when this thread has its own worktree.
pub fn offer_file_revert(worktree_path: Option<&str>) -> bool {
    worktree_path.is_some_and(|path| !path.trim().is_empty())
}

/// `None` when restore may run. The caller excludes this thread from `others`.
pub fn file_revert_block<'a>(
    worktree: &str,
    checkpoint: Option<&str>,
    others: impl IntoIterator<Item = &'a str>,
) -> Option<&'static str> {
    let ready = offer_file_revert(Some(worktree))
        && checkpoint
            .map(str::trim)
            .is_some_and(|tree| !tree.is_empty());
    if !ready {
        return Some(NO_CHECKPOINT);
    }
    if others
        .into_iter()
        .any(|other| paths_overlap(worktree, other))
    {
        Some(DIRECTORY_IN_USE)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_is_the_same_directory_or_a_parent() {
        assert!(paths_overlap("/repo", "/repo"));
        assert!(paths_overlap("/repo/", "/repo"));
        assert!(paths_overlap("/repo", "/repo/child"));
        assert!(paths_overlap(r"C:\repo", r"C:\repo\child"));
        assert!(!paths_overlap("/repo", "/repo-other"));
        assert!(!paths_overlap("/repo/child", "/repo/child-2"));
        assert!(!paths_overlap("", "/repo"));
        assert!(!paths_overlap("  ", "/repo"));
    }

    #[test]
    fn file_revert_waits_for_a_checkpoint_and_a_free_worktree() {
        assert!(!offer_file_revert(None));
        assert!(!offer_file_revert(Some("  ")));
        assert!(offer_file_revert(Some("/wt")));
        assert_eq!(
            file_revert_block("/wt", None, std::iter::empty()),
            Some(NO_CHECKPOINT)
        );
        assert_eq!(
            file_revert_block("/wt", Some("  "), std::iter::empty()),
            Some(NO_CHECKPOINT)
        );
        assert_eq!(
            file_revert_block("/wt", Some("abc"), ["/wt/nested"]),
            Some(DIRECTORY_IN_USE)
        );
        assert_eq!(file_revert_block("/wt", Some("abc"), ["/other"]), None);
    }
}
