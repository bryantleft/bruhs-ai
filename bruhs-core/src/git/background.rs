//! Background remote status and clean-checkout fast-forward operations.
use super::*;

/// Fetch the current branch's configured remote, or the repository's default remote.
pub async fn fetch_project_remote(repo: &Path) -> Result<(), GitError> {
    let out = GitCommand::new(repo, &["fetch", "--quiet", "--no-recurse-submodules"])
        .allow_network()
        .respect_trusted_hooks()
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .run()
        .await?;
    if out.success() {
        Ok(())
    } else {
        Err(out.failure())
    }
}

/// Bring the checked-out default branch up to its tracked remote HEAD.
/// Other branches, detached HEADs, local changes and locally divergent commits stay in place.
pub async fn fast_forward_clean_default(repo: &Path) -> Result<bool, GitError> {
    let head = git(repo, &["symbolic-ref", "--quiet", "HEAD"]).await?;
    if !head.success() {
        return Ok(false);
    }
    let local_ref = head.stdout_lossy();
    let relation = git(
        repo,
        &[
            "for-each-ref",
            "--format=%(upstream:remotename)%09%(upstream:remoteref)%09%(upstream)",
            local_ref.trim(),
        ],
    )
    .await?;
    if !relation.success() {
        return Err(relation.failure());
    }
    let relation = relation.stdout_lossy();
    let mut parts = relation.trim_end().split('\t');
    let (Some(remote), Some(remote_ref), Some(upstream)) =
        (parts.next(), parts.next(), parts.next())
    else {
        return Ok(false);
    };
    if remote.is_empty()
        || remote == "."
        || remote_ref != local_ref.trim()
        || !upstream.starts_with("refs/remotes/")
    {
        return Ok(false);
    }
    let remote_head = format!("refs/remotes/{remote}/HEAD");
    let default = git(repo, &["symbolic-ref", "--quiet", &remote_head]).await?;
    if !default.success() || default.stdout_lossy().trim() != upstream {
        return Ok(false);
    }
    let state = git(
        repo,
        &[
            "status",
            "--porcelain=v1",
            "--untracked-files=normal",
            "--ignore-submodules=none",
        ],
    )
    .await?;
    if !state.success() {
        return Err(state.failure());
    }
    if !state.stdout.is_empty() {
        return Ok(false);
    }
    let relation = git(
        repo,
        &[
            "rev-list",
            "--left-right",
            "--count",
            &format!("HEAD...{upstream}"),
        ],
    )
    .await?;
    if !relation.success() {
        return Err(relation.failure());
    }
    let relation = relation.stdout_lossy();
    if !matches!(relation.trim().split_once('\t'),Some(("0",behind)) if behind!="0") {
        return Ok(false);
    }
    let out = GitCommand::new(
        repo,
        &[
            "-c",
            "merge.autoStash=false",
            "merge",
            "--ff-only",
            "--no-overwrite-ignore",
            upstream,
        ],
    )
    .respect_trusted_hooks()
    .run()
    .await?;
    if !out.success() {
        return Err(out.failure());
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::{fast_forward_clean_default, fetch_project_remote};
    use std::{fs, path::Path, process::Command};
    fn git(cwd: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .current_dir(cwd)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args([
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.test",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{args:?} {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    #[tokio::test]
    async fn auto_pull_advances_only_the_clean_default_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let origin = dir.path().join("origin");
        fs::create_dir(&origin).unwrap();
        git(&origin, &["init", "-b", "main"]);
        fs::write(origin.join("file"), "first").unwrap();
        git(&origin, &["add", "."]);
        git(&origin, &["commit", "-m", "initial"]);
        git(dir.path(), &["clone", "origin", "copy"]);
        let copy = dir.path().join("copy");
        fs::write(origin.join("file"), "second").unwrap();
        git(&origin, &["commit", "-am", "next"]);
        fetch_project_remote(&copy).await.unwrap();
        git(&copy, &["switch", "-c", "work", "--track", "origin/main"]);
        assert!(!fast_forward_clean_default(&copy).await.unwrap());
        git(&copy, &["switch", "main"]);
        fs::write(copy.join("local"), "in progress").unwrap();
        assert!(!fast_forward_clean_default(&copy).await.unwrap());
        fs::remove_file(copy.join("local")).unwrap();
        assert!(fast_forward_clean_default(&copy).await.unwrap());
        assert_eq!(fs::read_to_string(copy.join("file")).unwrap(), "second");
        fs::write(copy.join("local"), "local commit").unwrap();
        git(&copy, &["add", "."]);
        git(&copy, &["commit", "-m", "local"]);
        fs::write(origin.join("file"), "third").unwrap();
        git(&origin, &["commit", "-am", "divergence"]);
        fetch_project_remote(&copy).await.unwrap();
        assert!(!fast_forward_clean_default(&copy).await.unwrap());
        assert_eq!(fs::read_to_string(copy.join("file")).unwrap(), "second");
    }
}
