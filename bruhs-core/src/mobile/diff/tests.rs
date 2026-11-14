use super::*;
fn command(root: &Path, args: &[&str]) -> Vec<u8> {
    let output = std::process::Command::new("git")
        .current_dir(root)
        .args([
            "-c",
            "user.name=Mobile fixture",
            "-c",
            "user.email=mobile@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?}: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
fn repo() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    command(root.path(), &["init", "--quiet"]);
    root
}
#[tokio::test]
async fn mobile_diff_pages_real_changes_without_helpers_index_writes_or_symlink_reads() {
    let root = repo();
    let dir = root.path();
    std::fs::write(dir.join("tracked.txt"), "old line\n").unwrap();
    std::fs::write(dir.join(".gitattributes"), "*.txt diff=evil filter=evil\n").unwrap();
    command(dir, &["add", "."]);
    command(dir, &["commit", "--quiet", "-m", "Synthetic base"]);
    let canary = dir.join("HELPER_RAN");
    let helper = format!("sh -c 'echo ran > {}; cat'", canary.display());
    for key in [
        "filter.evil.clean",
        "filter.evil.smudge",
        "filter.evil.process",
        "diff.evil.textconv",
        "diff.external",
        "core.fsmonitor",
    ] {
        command(dir, &["config", key, &helper]);
    }
    command(dir, &["config", "filter.evil.required", "true"]);
    std::fs::write(
        dir.join("tracked.txt"),
        format!("new line\n{}\n", "paged 東京\n".repeat(1000)),
    )
    .unwrap();
    std::fs::write(dir.join("untracked.txt"), "new untracked content\n").unwrap();
    #[cfg(unix)]
    let outside = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        std::fs::write(outside.path().join("canary"), "PRIVATE_OUTSIDE_NOT_READ").unwrap();
        symlink(outside.path().join("canary"), dir.join("outside-link")).unwrap();
    }
    let index = std::fs::read(dir.join(".git/index")).unwrap();
    let mut full = String::new();
    let mut offset = 0;
    let mut revision = None;
    loop {
        let page = read(dir, offset, revision.as_deref()).await.unwrap();
        assert!(!page.source_truncated);
        full.push_str(&page.text);
        revision = Some(page.revision);
        if let Some(next) = page.next_offset {
            offset = next;
        } else {
            break;
        }
    }
    assert!(offset > 0);
    assert!(full.contains("-old line"));
    assert!(full.contains("+new line"));
    assert!(full.contains("+new untracked content"));
    assert!(!full.contains("PRIVATE_OUTSIDE_NOT_READ"));
    assert!(
        !canary.exists(),
        "diff must not execute any configured helper"
    );
    assert_eq!(index, std::fs::read(dir.join(".git/index")).unwrap());
    std::fs::write(dir.join("tracked.txt"), "changed after first page\n").unwrap();
    assert!(matches!(
        read(dir, offset, revision.as_deref()).await,
        Err(MobileError::Conflict(_))
    ));
}
#[tokio::test]
async fn mobile_diff_supports_unborn_repositories_and_linked_worktrees() {
    let root = repo();
    let dir = root.path();
    std::fs::write(dir.join("new.txt"), "unborn staged line\n").unwrap();
    command(dir, &["add", "new.txt"]);
    assert!(read(dir, 0, None)
        .await
        .unwrap()
        .text
        .contains("+unborn staged line"));
    command(dir, &["commit", "--quiet", "-m", "Synthetic initial"]);
    let sibling = tempfile::tempdir().unwrap();
    let path = sibling.path().join("checkout");
    command(
        dir,
        &[
            "worktree",
            "add",
            "--quiet",
            "--detach",
            path.to_str().unwrap(),
            "HEAD",
        ],
    );
    std::fs::write(path.join("new.txt"), "only linked checkout changed\n").unwrap();
    assert!(read(&path, 0, None)
        .await
        .unwrap()
        .text
        .contains("+only linked checkout changed"));
    assert!(!read(dir, 0, None)
        .await
        .unwrap()
        .text
        .contains("+only linked checkout changed"));
    let nested = path.join("packages/app");
    std::fs::create_dir_all(&nested).unwrap();
    assert_eq!(
        read(&nested, 0, None).await.unwrap(),
        read(&path, 0, None).await.unwrap(),
        "a nested project resolves the same linked Git working tree"
    );
}

#[tokio::test]
async fn mobile_diff_keeps_repository_and_custom_exclusions_and_reads_split_indexes() {
    let root = repo();
    let dir = root.path();
    let global = tempfile::tempdir().unwrap();
    let ignore = global.path().join("global-ignore");
    std::fs::write(&ignore, "custom-global-secret.key\n").unwrap();
    command(
        dir,
        &["config", "core.excludesFile", ignore.to_str().unwrap()],
    );
    std::fs::write(dir.join(".git/info/exclude"), "local-secret.key\n").unwrap();
    std::fs::write(dir.join(".gitignore"), "tracked-ignore-secret.key\n").unwrap();
    std::fs::write(dir.join("tracked.txt"), "base\n").unwrap();
    command(dir, &["add", "."]);
    command(dir, &["commit", "--quiet", "-m", "Ignore fixture"]);
    command(dir, &["update-index", "--split-index"]);
    let index = std::fs::read(dir.join(".git/index")).unwrap();
    for file in [
        "local-secret.key",
        "custom-global-secret.key",
        "tracked-ignore-secret.key",
    ] {
        std::fs::write(dir.join(file), "PRIVATE_IGNORED_MOBILE_DIFF_CANARY").unwrap();
    }
    std::fs::write(dir.join("tracked.txt"), "changed with split index\n").unwrap();
    std::fs::write(dir.join("ordinary-new.txt"), "ordinary untracked\n").unwrap();
    let text = read(dir, 0, None).await.unwrap().text;
    assert!(text.contains("+changed with split index"));
    assert!(text.contains("ordinary untracked"));
    assert!(!text.contains("secret.key"));
    assert!(!text.contains("PRIVATE_IGNORED"));
    assert_eq!(index, std::fs::read(dir.join(".git/index")).unwrap());
}

#[tokio::test]
async fn mobile_diff_respects_custom_global_ignore_in_an_isolated_process() {
    let root = repo();
    let dir = root.path();
    let home = tempfile::tempdir().unwrap();
    let ignore = home.path().join("global-ignore");
    std::fs::write(&ignore, "global-only-secret.key\n").unwrap();
    let status = std::process::Command::new("git")
        .args([
            "config",
            "--file",
            home.path().join(".gitconfig").to_str().unwrap(),
            "core.excludesFile",
            ignore.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());
    std::fs::write(dir.join("tracked.txt"), "base\n").unwrap();
    command(dir, &["add", "."]);
    command(dir, &["commit", "--quiet", "-m", "Global ignore fixture"]);
    std::fs::write(dir.join("tracked.txt"), "global ignore changed\n").unwrap();
    std::fs::write(
        dir.join("global-only-secret.key"),
        "PRIVATE_GLOBAL_IGNORE_CANARY",
    )
    .unwrap();
    let output = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "mobile::diff::tests::mobile_diff_global_ignore_child",
            "--ignored",
            "--nocapture",
        ])
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("xdg"))
        .env("BRUHS_MOBILE_DIFF_PROJECT", dir)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "isolated global ignore check: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
#[tokio::test]
#[ignore = "called by the parent test with an isolated Git HOME"]
async fn mobile_diff_global_ignore_child() {
    let dir = PathBuf::from(
        std::env::var_os("BRUHS_MOBILE_DIFF_PROJECT").expect("parent supplies an isolated project"),
    );
    let text = read(&dir, 0, None).await.unwrap().text;
    assert!(text.contains("+global ignore changed"));
    assert!(!text.contains("global-only-secret"));
    assert!(!text.contains("PRIVATE_GLOBAL_IGNORE"));
}
