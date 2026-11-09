use super::*;

const C: AgentProvider = AgentProvider::Codex;
const K: AgentProvider = AgentProvider::Claude;

#[test]
fn versions_are_found_in_cli_output() {
    let found = |text: &str| Version::find(text).map(|v| v.to_string());
    assert_eq!(found("2.1.319 (Claude Code)").as_deref(), Some("2.1.319"));
    assert_eq!(found("codex-cli 0.166.0\n").as_deref(), Some("0.166.0"));
    assert_eq!(found("0.167.0,abc123").as_deref(), Some("0.167.0"));
    assert_eq!(found("v1.2.3-beta.1+sha").as_deref(), Some("1.2.3-beta.1"));
    assert_eq!(found("@openai/codex@0.2.0").as_deref(), Some("0.2.0"));
    assert!(Version::find("Claude Code").is_none());
    assert!(Version::find("build 12").is_none());
}

#[test]
fn versions_order_numerically_with_prereleases_first() {
    assert!(is_newer("0.167.0", "0.166.0"));
    assert!(is_newer("2.1.320", "2.1.319"));
    assert!(is_newer("2.10.0", "2.9.9"));
    assert!(is_newer("1.0.0", "1.0.0-rc.1"));
    assert!(!is_newer("1.0.0-rc.1", "1.0.0"));
    assert!(!is_newer("2.1.319", "2.1.319 (Claude Code)"));
    assert!(!is_newer("1.2", "1.2.0"));
    assert!(!is_newer("garbage", "1.0.0"));
    assert!(!is_newer("1.0.0", "garbage"));
}

#[test]
fn compatibility_follows_t3s_ranges() {
    assert_eq!(compat(K, "2.1.319"), Some(Compat::Supported));
    assert_eq!(compat(K, "2.1.280"), Some(Compat::Supported));
    assert_eq!(compat(K, "2.1.279"), Some(Compat::Limited));
    assert_eq!(compat(K, "2.1.111"), Some(Compat::Limited));
    assert_eq!(compat(K, "2.1.110"), Some(Compat::Unsupported));
    assert_eq!(compat(C, "0.166.0"), Some(Compat::Supported));
    assert_eq!(compat(C, "0.155.9"), Some(Compat::Unsupported));
    assert_eq!(compat(C, "0.148.0"), Some(Compat::Broken));
    assert_eq!(compat(C, "nope"), None);
    assert_eq!(
        compat_message(C, Compat::Broken),
        "This provider version is known to be broken with this bruhs release. Use >=0.159.0."
    );
    assert_eq!(
        compat_message(K, Compat::Limited),
        "This provider version has limited compatibility with this bruhs release. Use >=2.1.301."
    );
}

/// provider, on PATH, resolved, manager, package, prefix
type Case = (
    AgentProvider,
    &'static str,
    &'static str,
    InstallManager,
    &'static str,
    Option<&'static str>,
);

fn detect(provider: AgentProvider, on_path: &str, resolved: &str) -> Install {
    detect_install(provider, on_path, resolved)
}

#[test]
fn installs_are_classified_in_t3s_order() {
    let cases: &[Case] = &[
        (
            C,
            "/opt/homebrew/bin/codex",
            "/opt/homebrew/Caskroom/codex/0.166.0/bin/codex",
            InstallManager::HomebrewCask,
            "codex",
            Some("/opt/homebrew"),
        ),
        (
            C,
            "/usr/local/bin/codex",
            "/usr/local/Cellar/codex/0.1/bin/codex",
            InstallManager::Homebrew,
            "codex",
            Some("/usr/local"),
        ),
        (
            K,
            "/Users/a/.local/bin/claude",
            "/Users/a/.local/share/claude/versions/2.1.319",
            InstallManager::Native,
            "@anthropic-ai/claude-code",
            None,
        ),
        (
            K,
            "/opt/homebrew/bin/claude",
            "/opt/homebrew/lib/node_modules/@anthropic-ai/claude-code/cli.js",
            InstallManager::Npm,
            "@anthropic-ai/claude-code",
            Some("/opt/homebrew"),
        ),
        (
            K,
            "/x/bin/claude",
            "/opt/homebrew/Cellar/node/26.1/lib/node_modules/@anthropic-ai/claude-code/cli.js",
            InstallManager::Npm,
            "@anthropic-ai/claude-code",
            Some("/opt/homebrew/Cellar/node/26.1"),
        ),
        (
            C,
            "/u/.nvm/versions/node/v26/bin/codex",
            "/u/.nvm/versions/node/v26/lib/node_modules/@openai/codex/bin/codex.js",
            InstallManager::Npm,
            "@openai/codex",
            Some("/u/.nvm/versions/node/v26"),
        ),
        (
            C,
            "/u/Library/pnpm/codex",
            "/u/Library/pnpm/global/5/node_modules/@openai/codex/bin/codex.js",
            InstallManager::Pnpm,
            "@openai/codex",
            None,
        ),
        (
            C,
            "/u/.bun/bin/codex",
            "/u/.bun/install/global/node_modules/@openai/codex/bin/codex.js",
            InstallManager::Bun,
            "@openai/codex",
            None,
        ),
        (
            C,
            "/u/.yarn/bin/codex",
            "/u/.config/yarn/global/node_modules/@openai/codex/bin/codex.js",
            InstallManager::Yarn,
            "@openai/codex",
            None,
        ),
        (
            C,
            "/u/.volta/bin/codex",
            "/u/.volta/bin/volta-shim",
            InstallManager::Volta,
            "@openai/codex",
            None,
        ),
        (
            C,
            "/u/.vite-plus/bin/codex",
            "/u/.vite-plus/lib/codex",
            InstallManager::VitePlus,
            "@openai/codex",
            None,
        ),
        (
            C,
            "/u/.codex/packages/standalone/bin/codex",
            "/u/.codex/packages/standalone/0.1/codex",
            InstallManager::Native,
            "@openai/codex",
            Some("/u/.codex"),
        ),
        (
            C,
            "/u/.local/share/mise/installs/codex/latest/bin/codex",
            "/u/.local/share/mise/installs/codex/0.1/bin/codex",
            InstallManager::Mise,
            "codex",
            None,
        ),
        // Unknown paths: Claude's own updater, Codex by hand.
        (
            K,
            "/usr/local/bin/claude",
            "/usr/local/bin/claude",
            InstallManager::Native,
            "@anthropic-ai/claude-code",
            None,
        ),
        (
            C,
            "/usr/local/bin/codex",
            "/usr/local/bin/codex",
            InstallManager::Unknown,
            "@openai/codex",
            None,
        ),
        (
            K,
            "/x/claude",
            "/x/node_modules/@anthropic-ai/claude-code/cli.js",
            InstallManager::Unknown,
            "@anthropic-ai/claude-code",
            None,
        ),
    ];
    for (provider, on_path, resolved, manager, package, prefix) in cases {
        let install = detect(*provider, on_path, resolved);
        assert_eq!(install.manager, *manager, "{resolved}");
        assert_eq!(install.package, *package, "{resolved}");
        assert_eq!(install.prefix.as_deref(), *prefix, "{resolved}");
    }
}

/// A mise shim resolves to mise itself; a Homebrew mise must not read as a
/// Homebrew install of `mise` (the reviewer's case).
#[test]
fn mise_shims_are_read_before_resolving() {
    let install = detect(
        C,
        "/Users/a/.local/share/mise/shims/codex",
        "/opt/homebrew/Cellar/mise/2025.1.0/bin/mise",
    );
    assert_eq!(install.manager, InstallManager::Mise);
    assert_eq!(install.package, "codex");
}

fn line(install: &Install, latest: Option<&str>, pnpm: Option<&str>) -> Option<String> {
    let context = CommandContext {
        resolved: "/Users/a/.local/share/claude/versions/2.1.319",
        latest,
        pnpm_version: pnpm,
    };
    update_command(install, &context).map(|argv| display_command(&argv))
}

#[test]
fn each_installer_has_t3s_update_command() {
    let at = |p, on, r| detect(p, on, r);
    assert_eq!(
        line(
            &at(C, "/b/codex", "/opt/homebrew/Caskroom/codex/0.1/bin/codex"),
            None,
            None
        )
        .as_deref(),
        Some("brew upgrade --cask codex")
    );
    assert_eq!(
        line(
            &at(C, "/b/codex", "/x/Cellar/codex/0.1/bin/codex"),
            None,
            None
        )
        .as_deref(),
        Some("brew upgrade codex")
    );
    assert_eq!(
        line(&at(K, "/b/claude", "/p/lib/node_modules/@anthropic-ai/claude-code/cli.js"), None, None).as_deref(),
        Some("npm install -g --prefix /p --allow-scripts=@anthropic-ai/claude-code @anthropic-ai/claude-code@latest")
    );
    assert_eq!(
        line(
            &at(
                K,
                "/u/.local/bin/claude",
                "/u/.local/share/claude/versions/2.1"
            ),
            None,
            None
        )
        .as_deref(),
        Some("/Users/a/.local/share/claude/versions/2.1.319 update")
    );
    let pnpm = at(
        C,
        "/u/Library/pnpm/codex",
        "/u/Library/pnpm/global/5/node_modules/@openai/codex/x.js",
    );
    assert_eq!(
        line(&pnpm, Some("0.167.0"), Some("10.4.0")).as_deref(),
        Some("pnpm add -g --allow-build=@openai/codex @openai/codex@0.167.0")
    );
    assert_eq!(
        line(&pnpm, None, Some("9.0.0")).as_deref(),
        Some("pnpm add -g @openai/codex@latest")
    );
    assert_eq!(
        line(&at(C, "/u/.bun/bin/codex", "/u/.bun/bin/codex"), None, None).as_deref(),
        Some("bun i -g @openai/codex@latest")
    );
    assert_eq!(
        line(
            &at(C, "/u/.yarn/bin/codex", "/u/.yarn/bin/codex"),
            None,
            None
        )
        .as_deref(),
        Some("yarn global add @openai/codex@latest")
    );
    assert_eq!(
        line(
            &at(C, "/u/.volta/bin/codex", "/u/.volta/bin/volta-shim"),
            None,
            None
        )
        .as_deref(),
        Some("volta install @openai/codex@latest")
    );
    assert_eq!(
        line(
            &at(C, "/u/.vite-plus/bin/codex", "/u/.vite-plus/bin/codex"),
            None,
            None
        )
        .as_deref(),
        Some("vp i -g @openai/codex")
    );
    let mut mise = at(C, "/u/.local/share/mise/shims/codex", "/u/mise");
    assert_eq!(line(&mise, None, None), None);
    mise.tracks_latest = true;
    assert_eq!(
        line(&mise, None, None).as_deref(),
        Some("mise upgrade codex")
    );
    assert_eq!(
        line(
            &at(C, "/usr/local/bin/codex", "/usr/local/bin/codex"),
            None,
            None
        ),
        None
    );
    assert_eq!(
        manual_hint(&at(C, "/usr/local/bin/codex", "/usr/local/bin/codex"), Some("0.167.0"), "/usr/local/bin/codex"),
        "Version 0.167.0 is out. bruhs runs /usr/local/bin/codex; update it with the app or tool that installed it."
    );
}

#[test]
fn last_output_line_strips_control_codes() {
    assert_eq!(
        last_output_line("one\n\u{1b}[32m==> Upgrading codex\u{1b}[0m\r\n\n").as_deref(),
        Some("==> Upgrading codex")
    );
    assert_eq!(last_output_line("50%\r75%\r100%").as_deref(), Some("100%"));
    assert_eq!(last_output_line("  \n"), None);
    let long = "x".repeat(500);
    assert_eq!(last_output_line(&long).unwrap().len(), LAST_LINE_LIMIT);
}

fn provider(p: AgentProvider, installed: Option<&str>, latest: Option<&str>) -> ProviderUpdate {
    let path = "/opt/homebrew/Caskroom/codex/0.1/bin/codex";
    let install = installed.map(|_| detect_install(p, path, path));
    ProviderUpdate {
        provider: p,
        state: ProviderUpdateState::Unknown,
        installed: installed.map(String::from),
        latest: latest.map(String::from),
        binary_path: installed.map(|_| path.into()),
        update_command: install
            .as_ref()
            .and_then(|i| update_command(i, &CommandContext::default())),
        install,
        manual_hint: None,
        compat: None,
        error: None,
    }
    .settle()
}

#[test]
fn provider_state_follows_the_versions_and_compat() {
    assert_eq!(
        provider(C, None, Some("1.0.0")).state,
        ProviderUpdateState::NotInstalled
    );
    assert_eq!(
        provider(C, Some("0.166.0"), Some("0.167.0")).state,
        ProviderUpdateState::Outdated
    );
    assert_eq!(
        provider(C, Some("0.167.0"), Some("0.167.0")).state,
        ProviderUpdateState::UpToDate
    );
    // Installed newer than the registry (a pre-release build) is current.
    assert_eq!(
        provider(C, Some("0.168.0"), Some("0.167.0")).state,
        ProviderUpdateState::UpToDate
    );
    assert_eq!(
        provider(C, Some("0.166.0"), None).state,
        ProviderUpdateState::Unknown
    );
    assert_eq!(
        provider(C, Some("0.166.0"), Some("0.167.0")).status_line(),
        "0.166.0 → 0.167.0 via Homebrew"
    );
    // A newer latest that is itself unsupported is not offered.
    let stale = provider(C, Some("0.140.0"), Some("0.150.0"));
    assert_eq!(stale.state, ProviderUpdateState::UpToDate);
    assert_eq!(stale.compat, Some(Compat::Broken));
    let (title, message) = stale.compat_warning().unwrap();
    assert_eq!(title, "Known broken version");
    assert!(
        message.contains("Homebrew's local package metadata may be stale."),
        "{message}"
    );
    assert!(provider(C, Some("0.167.0"), Some("0.167.0"))
        .compat_warning()
        .is_none());
}

fn app(state: AppUpdateState) -> AppUpdate {
    AppUpdate {
        state,
        built_commit: Some("3c5dfc3aaaaaaa".into()),
        remote_commit: Some("e723466bbbbbbb".into()),
        remote_branch: Some("origin/main".into()),
        local_branch: None,
        repo_root: Some("/Users/a/bruhs ai".into()),
        command: None,
        error: None,
        release: None,
    }
}

fn run(id: u64, p: AgentProvider, phase: RunPhase, outcome: Option<RunOutcome>) -> ProviderRun {
    ProviderRun {
        id,
        provider: p,
        phase,
        started_at: EpochMs::saturating_from_i64(1),
        finished_at: None,
        command: Some("brew upgrade --cask codex".into()),
        last_line: None,
        from_version: Some("0.166.0".into()),
        to_version: Some("0.167.0".into()),
        outcome,
        message: None,
        output: String::new(),
    }
}

#[test]
fn report_summarizes_what_is_behind_and_what_runs() {
    let report = UpdatesReport {
        checked_at: EpochMs::saturating_from_i64(1_000),
        online: true,
        providers: vec![
            provider(C, Some("0.166.0"), Some("0.167.0")),
            provider(K, Some("2.1.319"), Some("2.1.319")),
        ],
        app: app(AppUpdateState::Behind),
        runs: vec![],
    };
    assert!(report.has_updates());
    assert_eq!(report.updatable_providers(), vec![C]);
    assert!(report.skipped_providers().is_empty());
    assert_eq!(sidebar_label(&report), "Update");
    assert_eq!(
        report.summary(),
        "Update 1 provider\nCodex 0.166.0 → 0.167.0 via Homebrew\nbruhs: new commits on the remote"
    );
    assert!(report.is_fresh(EpochMs::saturating_from_i64(1_000 + CHECK_TTL_MS - 1)));
    assert!(!report.is_fresh(EpochMs::saturating_from_i64(1_000 + CHECK_TTL_MS)));
    let mut failed = report.clone();
    failed.providers[1].error = Some("offline".into());
    assert!(!failed.is_fresh(EpochMs::saturating_from_i64(1_000 + FAILED_CHECK_TTL_MS)));

    let running = UpdatesReport {
        runs: vec![run(1, C, RunPhase::Running, None)],
        ..report.clone()
    };
    assert!(running.updatable_providers().is_empty());
    assert_eq!(running.skipped_providers(), vec![(C, "already updating")]);
    assert_eq!(
        running_label(&running.runs).as_deref(),
        Some("Updating Codex")
    );
    let two = [
        run(1, C, RunPhase::Running, None),
        run(2, K, RunPhase::Queued, None),
    ];
    assert_eq!(running_label(&two).as_deref(), Some("Updating 2 providers"));
    assert_eq!(confirm_title(2), "Update 2 providers?");

    let rebuild = UpdatesReport {
        providers: vec![],
        app: app(AppUpdateState::RebuildNeeded),
        ..report
    };
    assert_eq!(sidebar_label(&rebuild), "Rebuild bruhs");
}

#[test]
fn runs_read_like_t3s_pill() {
    let done = |outcome| run(3, C, RunPhase::Done, Some(outcome));
    assert_eq!(
        done(RunOutcome::Updated).pill_label(),
        "Codex updated: v0.167.0"
    );
    assert_eq!(
        done(RunOutcome::Failed).pill_label(),
        "Codex v0.167.0 update failed"
    );
    assert_eq!(
        done(RunOutcome::Unchanged).pill_label(),
        "Codex still needs an update"
    );
    assert!(done(RunOutcome::Failed).needs_attention());
    assert!(!done(RunOutcome::Updated).needs_attention());
    let mut running = run(4, C, RunPhase::Running, None);
    assert_eq!(running.phase_line(), "Running brew upgrade --cask codex");
    running.last_line = Some("==> Upgrading codex".into());
    assert_eq!(running.phase_line(), "==> Upgrading codex");
    assert_eq!(
        run(5, C, RunPhase::Queued, None).phase_line(),
        "Waiting for another provider update to finish."
    );
}

#[test]
fn app_commands_respect_the_branch() {
    assert_eq!(
        app_update_command(
            "/Users/a/bruhs ai",
            AppUpdateState::Behind,
            true,
            "scripts/dev.sh"
        )
        .as_deref(),
        Some("cd '/Users/a/bruhs ai' && git pull --ff-only && scripts/dev.sh")
    );
    assert_eq!(
        app_update_command("/r", AppUpdateState::Behind, false, "x"),
        None
    );
    assert_eq!(
        app_update_command("/r", AppUpdateState::RebuildNeeded, false, "scripts/dev.sh").as_deref(),
        Some("cd /r && scripts/dev.sh")
    );
    assert_eq!(
        app_update_command("/r", AppUpdateState::UpToDate, true, "x"),
        None
    );
    assert_eq!(
        app(AppUpdateState::Behind).status_line(),
        "Built from 3c5dfc3. origin/main is at e723466: pull and rebuild."
    );
    let branch = AppUpdate {
        local_branch: Some("feat/x".into()),
        ..app(AppUpdateState::Behind)
    };
    assert_eq!(
        branch.status_line(),
        "Built from 3c5dfc3 on feat/x. origin/main is at e723466: merge or rebase when you are ready, then rebuild."
    );
}

#[test]
fn ls_remote_head_names_the_default_branch() {
    let out = "ref: refs/heads/main\tHEAD\n3ccb33d805c224d272b479e68a9b51863c2d8191\tHEAD\n";
    assert_eq!(
        parse_ls_remote_head(out),
        Some((
            "main".into(),
            "3ccb33d805c224d272b479e68a9b51863c2d8191".into()
        ))
    );
    assert_eq!(
        parse_ls_remote_head("3ccb33d805c224d272b479e68a9b51863c2d8191\tHEAD\n"),
        None
    );
    assert_eq!(parse_ls_remote_head(""), None);
}

#[test]
fn report_round_trips_through_json() {
    let report = UpdatesReport {
        checked_at: EpochMs::saturating_from_i64(5),
        online: true,
        providers: vec![provider(C, Some("0.166.0"), Some("0.167.0"))],
        app: app(AppUpdateState::UpToDate),
        runs: vec![run(1, C, RunPhase::Done, Some(RunOutcome::Updated))],
    };
    let json = serde_json::to_string(&report).unwrap();
    assert_eq!(
        serde_json::from_str::<UpdatesReport>(&json).unwrap(),
        report
    );
    assert!(json.contains("\"state\":\"outdated\""));
    assert!(json.contains("\"manager\":\"homebrew_cask\""));
    assert!(json.contains("\"compat\":\"supported\""));
}

#[test]
fn release_assets_and_checksums_are_read_exactly() {
    assert_eq!(
        release_asset_names("arm64"),
        (
            "Bruhs-macOS-arm64.dmg".to_string(),
            "Bruhs-macOS-arm64.zip".to_string()
        )
    );
    let hash = "a".repeat(64);
    let sums = format!(
        "{hash}  Bruhs-macOS-arm64.zip\n{}  Bruhs-macOS-arm64.dmg\n",
        "b".repeat(64)
    );
    assert_eq!(
        checksum_for(&sums, "Bruhs-macOS-arm64.zip"),
        Some(hash.clone())
    );
    assert_eq!(
        checksum_for(&format!("{hash} *Bruhs.zip"), "Bruhs.zip"),
        Some(hash)
    );
    assert_eq!(checksum_for(&sums, "Bruhs-macOS-x86_64.zip"), None);
    assert_eq!(checksum_for("short  Bruhs.zip", "Bruhs.zip"), None);
}

#[test]
fn a_packaged_build_reads_like_a_release() {
    let release = |state, latest: Option<&str>| AppUpdate {
        release: Some(ReleaseUpdate {
            current_version: "0.1.0".into(),
            latest_version: latest.map(String::from),
            release_url: Some("https://github.com/bryantleft/bruhs-ai/releases/tag/v0.2.0".into()),
            download_url: latest.map(|_| "https://example.invalid/Bruhs-macOS-arm64.zip".into()),
            sha256: latest.map(|_| "c".repeat(64)),
        }),
        ..app(state)
    };
    assert_eq!(
        release(AppUpdateState::UpToDate, Some("0.1.0")).status_line(),
        "Bruhs v0.1.0 is the latest release."
    );
    let behind = release(AppUpdateState::Behind, Some("0.2.0"));
    assert_eq!(behind.status_line(), "Bruhs v0.1.0 → v0.2.0");
    assert!(behind.installable_release().is_some());
    assert!(release(AppUpdateState::UpToDate, Some("0.1.0"))
        .installable_release()
        .is_none());
    let report = UpdatesReport {
        checked_at: EpochMs::saturating_from_i64(1),
        online: true,
        providers: vec![],
        app: behind,
        runs: vec![],
    };
    assert_eq!(report.summary(), "Bruhs v0.2.0 is out");
    assert_eq!(sidebar_label(&report), "Update Bruhs");
}

#[test]
fn release_notes_link_to_the_versions_changelog_entry() {
    assert_eq!(
        release_notes_url("0.1.1"),
        "https://bruhs.ai/changelog#v0.1.1"
    );
    assert_eq!(
        release_notes_url("v0.2.0"),
        "https://bruhs.ai/changelog#v0.2.0"
    );
    let release = ReleaseUpdate {
        current_version: "0.1.0".into(),
        latest_version: Some("0.1.1".into()),
        release_url: Some("https://github.com/bryantleft/bruhs-ai/releases/tag/v0.1.1".into()),
        download_url: None,
        sha256: None,
    };
    assert_eq!(
        release.notes_url().as_deref(),
        Some("https://bruhs.ai/changelog#v0.1.1")
    );
    let unknown = ReleaseUpdate {
        latest_version: None,
        ..release
    };
    assert_eq!(unknown.notes_url(), None);
}
