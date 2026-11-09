use super::*;

#[test]
fn build_info_treats_empty_as_unknown() {
    let info = BuildInfo::new("", " ", "scripts/dev.sh");
    assert_eq!(info.commit, None);
    assert_eq!(info.repo_root, None);
    let info = BuildInfo::new("abc\n", "/r", "x");
    assert_eq!(info.commit.as_deref(), Some("abc"));
    assert_eq!(info.repo_root.as_deref(), Some(Path::new("/r")));
}

#[test]
fn claude_channel_is_the_last_one_named() {
    let texts = |list: &[&str]| list.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
    assert_eq!(claude_channel(&texts(&[])), "latest");
    assert_eq!(claude_channel(&texts(&[r#"{"model":"x"}"#])), "latest");
    assert_eq!(
        claude_channel(&texts(&[
            r#"{"autoUpdatesChannel":"stable"}"#,
            "not json",
            r#"{"autoUpdatesChannel":"latest"}"#
        ])),
        "latest"
    );
    assert_eq!(
        claude_channel(&texts(&[r#"{"autoUpdatesChannel":"stable"}"#])),
        "stable"
    );
    // Nothing that could escape the URL path.
    assert_eq!(
        claude_channel(&texts(&[r#"{"autoUpdatesChannel":"../x"}"#])),
        "latest"
    );
}

#[tokio::test]
async fn an_app_without_git_metadata_is_unavailable() {
    let app = check_app(&BuildInfo::default(), true).await;
    assert_eq!(app.state, AppUpdateState::Unavailable);
    assert_eq!(app.command, None);
}

fn sh(dir: &Path, script: &str) -> String {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap();
    assert!(out.status.success(), "{script}: {out:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// A throwaway repo with a bare `origin`: the build is behind, then the
/// checkout pulls (rebuild needed), then a branch build gets no pull.
#[tokio::test]
async fn the_app_check_follows_origin() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    sh(
        dir,
        "git init -q -b main origin.git --bare && git init -q -b main work && cd work \
         && git commit -q --allow-empty -m one && git remote add origin ../origin.git \
         && git push -q origin main && git clone -q ../origin.git ../other",
    );
    let root = dir.join("work");
    let built = sh(dir, "git -C work rev-parse HEAD");
    let build = BuildInfo::new(&built, &root.to_string_lossy(), "scripts/dev.sh");

    assert_eq!(
        check_app(&build, true).await.state,
        AppUpdateState::UpToDate
    );
    assert_eq!(
        check_app(&build, false).await.state,
        AppUpdateState::Unknown
    );

    sh(
        dir,
        "cd other && git commit -q --allow-empty -m two && git push -q origin main",
    );
    let behind = check_app(&build, true).await;
    assert_eq!(behind.state, AppUpdateState::Behind);
    assert_eq!(behind.remote_branch.as_deref(), Some("origin/main"));
    assert_eq!(behind.local_branch, None);
    assert!(behind
        .command
        .unwrap()
        .ends_with("git pull --ff-only && scripts/dev.sh"));

    // On a feature branch: behind, but no pull to suggest.
    sh(dir, "git -C work switch -q -c feat/x");
    let branch = check_app(&build, true).await;
    assert_eq!(branch.state, AppUpdateState::Behind);
    assert_eq!(branch.local_branch.as_deref(), Some("feat/x"));
    assert_eq!(branch.command, None);
    sh(dir, "git -C work switch -q main");

    sh(dir, "git -C work pull -q --ff-only origin main");
    assert_eq!(
        check_app(&build, true).await.state,
        AppUpdateState::RebuildNeeded
    );

    // A local commit on top of origin is not "behind".
    sh(dir, "git -C work commit -q --allow-empty -m three");
    let ahead = BuildInfo::new(
        &sh(dir, "git -C work rev-parse HEAD"),
        &root.to_string_lossy(),
        "x",
    );
    assert_eq!(
        check_app(&ahead, true).await.state,
        AppUpdateState::UpToDate
    );
}

fn script(dir: &Path, name: &str, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    let file = dir.join(name);
    std::fs::write(&file, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A fake npm-global Claude: `bin/claude` links to
/// `prefix/lib/node_modules/@anthropic-ai/claude-code/cli.js`, which prints
/// the version in `bin/version`. The fake `npm` names 2.1.320 as latest and
/// runs `install_body` for `npm install`.
fn fake_claude(bin: &Path, install_body: &str) {
    let v = bin.join("version");
    let v = v.display();
    let package = bin.join("../prefix/lib/node_modules/@anthropic-ai/claude-code");
    std::fs::create_dir_all(&package).unwrap();
    script(
        &package,
        "cli.js",
        &format!("echo \"$(cat '{v}') (Claude Code)\""),
    );
    let link = bin.join("claude");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(package.join("cli.js"), &link).unwrap();
    script(
        bin,
        "npm",
        &format!("case \"$1\" in view) echo 2.1.320 ;; install) {install_body} ;; esac"),
    );
}

fn checker(dir: &Path) -> UpdateChecker {
    UpdateChecker::with_env(
        BuildInfo::default(),
        Some(OsString::from(dir.join("bin"))),
        Some(dir.join("locks")),
    )
}

async fn wait(checker: &UpdateChecker, id: u64) -> ProviderRun {
    for _ in 0..200 {
        let run = checker.runs().into_iter().find(|r| r.id == id).unwrap();
        if !run.is_active() {
            return run;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("run {id} never finished");
}

#[tokio::test]
async fn an_outdated_cli_is_updated_and_verified() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("version"), "2.1.319").unwrap();
    let v = bin.join("version");
    fake_claude(
        &bin,
        &format!("echo 'Checking…'; printf '\\033[32mUpdated to 2.1.320\\033[0m\\n'; echo 2.1.320 > '{}'", v.display()),
    );
    let checker = checker(dir.path());

    let report = checker.report(false, true).await;
    let claude = report.provider(AgentProvider::Claude).unwrap();
    assert_eq!(claude.state, ProviderUpdateState::Outdated, "{claude:?}");
    assert_eq!(
        claude.install.as_ref().unwrap().manager,
        InstallManager::Npm
    );
    assert_eq!(report.updatable_providers(), vec![AgentProvider::Claude]);
    // Codex is not installed here; offline reports skip the registry.
    assert_eq!(
        report.provider(AgentProvider::Codex).unwrap().state,
        ProviderUpdateState::NotInstalled
    );
    let offline = checker.report(false, false).await;
    assert_eq!(
        offline.provider(AgentProvider::Claude).unwrap().state,
        ProviderUpdateState::Unknown
    );

    let run = checker.start_update(AgentProvider::Claude).unwrap();
    assert_eq!(
        checker.start_update(AgentProvider::Claude),
        Err(StartError::AlreadyRunning)
    );
    let done = wait(&checker, run.id).await;
    assert_eq!(done.outcome, Some(RunOutcome::Updated), "{done:?}");
    assert_eq!(done.message.as_deref(), Some("Provider updated."));
    assert_eq!(done.from_version.as_deref(), Some("2.1.319"));
    assert_eq!(done.to_version.as_deref(), Some("2.1.320"));
    let command = done.command.unwrap();
    assert!(command.starts_with("npm install -g --prefix "), "{command}");
    assert!(command
        .ends_with(" --allow-scripts=@anthropic-ai/claude-code @anthropic-ai/claude-code@latest"));
    assert_eq!(done.last_line.as_deref(), Some("Updated to 2.1.320"));
    assert!(done.output.contains("Checking…"));
    // The cached report already has the new version.
    let after = checker.report(false, true).await;
    assert_eq!(
        after.provider(AgentProvider::Claude).unwrap().state,
        ProviderUpdateState::UpToDate
    );
    assert_eq!(after.runs.len(), 1);
}

#[tokio::test]
async fn failed_and_unchanged_runs_say_why() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("version"), "2.1.319").unwrap();
    fake_claude(
        &bin,
        "echo 'error: could not reach the update server' >&2; exit 3",
    );
    let checker = checker(dir.path());
    let run = checker.start_update(AgentProvider::Claude).unwrap();
    let failed = wait(&checker, run.id).await;
    assert_eq!(failed.outcome, Some(RunOutcome::Failed));
    assert_eq!(
        failed.message.as_deref(),
        Some("Update command exited with code 3: error: could not reach the update server")
    );

    fake_claude(&bin, "echo done");
    let run = checker.start_update(AgentProvider::Claude).unwrap();
    let unchanged = wait(&checker, run.id).await;
    assert_eq!(unchanged.outcome, Some(RunOutcome::Unchanged));
    assert_eq!(unchanged.pill_label(), "Claude Code still needs an update");
}

#[tokio::test]
async fn up_to_date_and_manual_installs_do_not_run_anything() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("version"), "2.1.320").unwrap();
    let marker = dir.path().join("ran");
    fake_claude(&bin, &format!("touch '{}'", marker.display()));
    // Codex on an unknown path: a manual update.
    script(&bin, "codex", "echo 'codex-cli 0.1.0'");
    let checker = checker(dir.path());

    let run = checker.start_update(AgentProvider::Claude).unwrap();
    let current = wait(&checker, run.id).await;
    assert_eq!(current.outcome, Some(RunOutcome::AlreadyCurrent));
    let run = checker.start_update(AgentProvider::Codex).unwrap();
    let manual = wait(&checker, run.id).await;
    assert_eq!(manual.outcome, Some(RunOutcome::Failed));
    assert!(
        manual
            .message
            .as_deref()
            .unwrap()
            .contains("update it with the app or tool that installed it"),
        "{manual:?}"
    );
    assert!(!marker.exists());
}

/// Two bruhs processes (two checkers) never run one installer at once.
#[tokio::test]
async fn installer_runs_wait_for_each_other_across_checkers() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("version"), "2.1.319").unwrap();
    let log = dir.path().join("log");
    let v = bin.join("version");
    fake_claude(
        &bin,
        &format!(
            "echo start >> '{l}'; /bin/sleep 0.4; echo end >> '{l}'; echo 2.1.320 > '{v}'",
            l = log.display(),
            v = v.display()
        ),
    );
    let first = checker(dir.path());
    let second = checker(dir.path());
    let a = first.start_update(AgentProvider::Claude).unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    let b = second.start_update(AgentProvider::Claude).unwrap();
    let a = wait(&first, a.id).await;
    let b = wait(&second, b.id).await;
    // Whichever took the lock first installed; the other waited, checked
    // again, and found nothing to do.
    let mut outcomes = [a.outcome, b.outcome];
    outcomes.sort_by_key(|o| format!("{o:?}"));
    assert_eq!(
        outcomes,
        [Some(RunOutcome::AlreadyCurrent), Some(RunOutcome::Updated)],
        "{a:?} {b:?}"
    );
    let log = std::fs::read_to_string(&log).unwrap();
    assert_eq!(log, "start\nend\n");
}

/// A version that moved outside bruhs stops reading as outdated as soon as
/// a run finds it current, not an hour later.
#[tokio::test]
async fn an_already_current_run_refreshes_the_cached_report() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("version"), "2.1.319").unwrap();
    fake_claude(&bin, "exit 9");
    let checker = checker(dir.path());
    let cached = checker.report(false, true).await;
    assert!(cached
        .provider(AgentProvider::Claude)
        .unwrap()
        .is_outdated());
    // Updated from a terminal meanwhile.
    std::fs::write(bin.join("version"), "2.1.320").unwrap();
    let run = checker.start_update(AgentProvider::Claude).unwrap();
    assert_eq!(
        wait(&checker, run.id).await.outcome,
        Some(RunOutcome::AlreadyCurrent)
    );
    let after = checker.report(false, true).await;
    assert_eq!(after.checked_at, cached.checked_at, "served from cache");
    assert_eq!(
        after.provider(AgentProvider::Claude).unwrap().state,
        ProviderUpdateState::UpToDate
    );
}

/// A helper the installer leaves running (and holding its output) does not
/// keep the run going once the installer itself has exited.
#[tokio::test]
async fn a_lingering_helper_does_not_hold_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("version"), "2.1.319").unwrap();
    let v = bin.join("version");
    fake_claude(
        &bin,
        &format!(
            "(/bin/sleep 30 &); echo installed; echo 2.1.320 > '{}'",
            v.display()
        ),
    );
    let checker = checker(dir.path());
    let started = std::time::Instant::now();
    let run = checker.start_update(AgentProvider::Claude).unwrap();
    let done = wait(&checker, run.id).await;
    assert_eq!(done.outcome, Some(RunOutcome::Updated), "{done:?}");
    assert_eq!(done.output, "installed");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn tail_keeps_the_end() {
    let long = "é".repeat(RUN_OUTPUT_LIMIT);
    let tail = tail(&long, RUN_OUTPUT_LIMIT);
    assert!(tail.starts_with('…'));
    assert!(tail.len() <= RUN_OUTPUT_LIMIT + '…'.len_utf8());
    assert_eq!(super::tail("  ok \n", 10), "ok");
}
