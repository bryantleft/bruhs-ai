//! Update checks and provider updates, shared by bruhs-server (for the web)
//! and bruhs-desktop (in process); the pure rules are in
//! [`bruhs_models::updates`].
//!
//! - A provider check runs `<cli> --version`, reads which installer owns the
//!   binary, and (when checks are on) asks that installer's source for the
//!   latest version: Claude's release channel for its native installer,
//!   Homebrew, mise or pnpm for theirs, otherwise the `npm` on `PATH` (its
//!   registry and proxy settings apply), and the public registry only when
//!   there is no npm.
//! - An update runs detached from whoever asked, so closing a tab never
//!   kills an installer halfway. Updates through the same installer queue,
//!   across processes too (a lock file), and each run reports its step and
//!   the installer's latest line.
//! - The app check compares the build's commit with the checkout's remote
//!   default branch through read-only `git ls-remote`. bruhs never pulls.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bruhs_models::agent::AgentProvider;
use bruhs_models::time::EpochMs;
use bruhs_models::updates::{
    app_update_command, binary_name, detect_install, last_output_line, manual_hint, npm_package,
    parse_ls_remote_head, provider_cli_label, update_command, AppUpdate, AppUpdateState,
    CommandContext, Install, InstallManager, ProviderRun, ProviderUpdate, ProviderUpdateState,
    RunOutcome, RunPhase, UpdatesReport, Version, RUN_OUTPUT_LIMIT, UPDATE_TIMEOUT_MS,
};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::Mutex;

/// `--version`, `brew info`, `npm view` and `git ls-remote` answer in
/// seconds; slower is stuck or offline (T3: 10 s per probe).
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
/// T3's registry and release-channel timeout.
const HTTP_TIMEOUT: Duration = Duration::from_secs(4);
const UPDATE_TIMEOUT: Duration = Duration::from_millis(UPDATE_TIMEOUT_MS);
const NPM_REGISTRY: &str = "https://registry.npmjs.org";
const CLAUDE_RELEASES: &str = "https://downloads.claude.ai/claude-code-releases";
/// Finished runs kept for clients to read.
const RUN_HISTORY: usize = 10;

/// Where the running binary came from, captured at compile time by the
/// leaf crate's build script (`bruhs-core/build_info.rs`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuildInfo {
    /// Full commit id, or `None` when built outside a git checkout.
    pub commit: Option<String>,
    /// The checkout's top level.
    pub repo_root: Option<PathBuf>,
    /// What rebuilds and relaunches this surface (`scripts/dev.sh`).
    pub rebuild: String,
    /// A packaged release's version (`0.1.0`): it updates from GitHub
    /// releases instead of comparing commits.
    pub release_version: Option<String>,
    /// `owner/repo` the releases come from.
    pub release_repo: String,
}

impl BuildInfo {
    /// From the `BRUHS_BUILD_COMMIT` / `BRUHS_REPO_ROOT` values a build
    /// script emitted; empty strings mean unknown.
    pub fn new(commit: &str, repo_root: &str, rebuild: impl Into<String>) -> Self {
        let some = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_string());
        Self {
            commit: some(commit),
            repo_root: some(repo_root).map(PathBuf::from),
            rebuild: rebuild.into(),
            release_version: None,
            release_repo: bruhs_models::updates::RELEASE_REPO.to_string(),
        }
    }

    /// Mark a packaged release (`BRUHS_RELEASE_VERSION`); empty is a source
    /// build.
    #[must_use]
    pub fn with_release(mut self, version: &str) -> Self {
        let version = version.trim().trim_start_matches('v');
        self.release_version = (!version.is_empty()).then(|| version.to_string());
        self
    }
}

// ---------------------------------------------------------------------------
// Processes
// ---------------------------------------------------------------------------

struct Ran {
    ok: bool,
    stdout: String,
    stderr: String,
}

#[derive(Debug, thiserror::Error)]
enum RunError {
    #[error("`{0}` is not installed")]
    NotFound(String),
    #[error("`{0}` could not start: {1}")]
    Spawn(String, std::io::Error),
    #[error("`{0}` did not finish within {1:?}")]
    TimedOut(String, Duration),
}

fn program_name(program: &Path) -> String {
    program
        .file_name()
        .map_or_else(|| program.to_string_lossy(), |n| n.to_string_lossy())
        .into_owned()
}

/// Run a query to completion with stdin closed and a deadline.
async fn query(
    program: &Path,
    args: &[&str],
    cwd: Option<&Path>,
    env: &[(&str, &str)],
) -> Result<Ran, RunError> {
    let name = program_name(program);
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    for (key, value) in env {
        command.env(key, value);
    }
    match tokio::time::timeout(QUERY_TIMEOUT, command.output()).await {
        Ok(Ok(output)) => Ok(Ran {
            ok: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }),
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            Err(RunError::NotFound(name))
        }
        Ok(Err(error)) => Err(RunError::Spawn(name, error)),
        Err(_) => Err(RunError::TimedOut(name, QUERY_TIMEOUT)),
    }
}

/// The first `name` on `path`, as found (not canonicalized).
fn find_on_path(name: &str, path: Option<&OsString>) -> Option<PathBuf> {
    std::env::split_paths(path?)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// The program for argv[0]: absolute paths as they are, Homebrew's own
/// `brew` beside the install when there is one (T3 prefers it), else `PATH`.
fn resolve_program(program: &str, install: &Install, path: Option<&OsString>) -> Option<PathBuf> {
    if Path::new(program).is_absolute() {
        return Some(PathBuf::from(program));
    }
    if program == "brew" {
        if let Some(prefix) = install.prefix.as_deref() {
            let own = Path::new(prefix).join("bin").join("brew");
            if own.is_file() {
                return Some(own);
            }
        }
    }
    find_on_path(program, path)
}

fn first_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
}

// ---------------------------------------------------------------------------
// Providers
// ---------------------------------------------------------------------------

/// Check one provider CLI: installed version, owner, compatibility, and,
/// when `online`, the latest version and the command that updates it.
pub async fn check_provider(provider: AgentProvider, online: bool) -> ProviderUpdate {
    check_provider_on(provider, std::env::var_os("PATH").as_ref(), online).await
}

async fn check_provider_on(
    provider: AgentProvider,
    path: Option<&OsString>,
    online: bool,
) -> ProviderUpdate {
    let mut update = ProviderUpdate {
        provider,
        state: ProviderUpdateState::NotInstalled,
        installed: None,
        latest: None,
        binary_path: None,
        install: None,
        update_command: None,
        manual_hint: None,
        compat: None,
        error: None,
    };
    let Some(on_path) = find_on_path(binary_name(provider), path) else {
        return update.settle();
    };
    let resolved = std::fs::canonicalize(&on_path).unwrap_or_else(|_| on_path.clone());
    let resolved_text = resolved.to_string_lossy().into_owned();
    let mut install = detect_install(provider, &on_path.to_string_lossy(), &resolved_text);
    if install.manager == InstallManager::Mise {
        install.tracks_latest = mise_tracks_latest(&install.package, path).await;
    }
    update.binary_path = Some(resolved_text.clone());

    let mut errors = Vec::new();
    match query(&on_path, &["--version"], None, &[]).await {
        Ok(ran) => match Version::find(&ran.stdout).or_else(|| Version::find(&ran.stderr)) {
            Some(version) => update.installed = Some(version.to_string()),
            None => errors.push(format!(
                "`{} --version` printed {:?}.",
                binary_name(provider),
                first_line(&ran.stdout)
            )),
        },
        Err(error) => errors.push(format!("{error}.")),
    }
    let mut pnpm_version = None;
    if online {
        match latest_version(provider, &install, path).await {
            Ok(version) => update.latest = Some(version),
            Err(error) => errors.push(error),
        }
        if install.manager == InstallManager::Pnpm {
            if let Some(pnpm) = find_on_path("pnpm", path) {
                pnpm_version = query(&pnpm, &["--version"], None, &[])
                    .await
                    .ok()
                    .map(|ran| ran.stdout.trim().to_string());
            }
        }
    }
    let context = CommandContext {
        resolved: &resolved_text,
        latest: update.latest.as_deref(),
        pnpm_version: pnpm_version.as_deref(),
    };
    update.update_command = update_command(&install, &context);
    if update.update_command.is_none() {
        update.manual_hint = Some(manual_hint(
            &install,
            update.latest.as_deref(),
            &resolved_text,
        ));
    }
    update.install = Some(install);
    if !errors.is_empty() {
        update.error = Some(errors.join(" "));
    }
    update.settle()
}

/// Whether mise's active `tool` was requested as `latest`, which is the
/// only case `mise upgrade` moves (T3's rule).
async fn mise_tracks_latest(tool: &str, path: Option<&OsString>) -> bool {
    let Some(mise) = find_on_path("mise", path) else {
        return false;
    };
    let Ok(ran) = query(&mise, &["ls", "--json", tool], None, &[]).await else {
        return false;
    };
    let Ok(rows) = serde_json::from_str::<serde_json::Value>(&ran.stdout) else {
        return false;
    };
    rows.as_array().is_some_and(|rows| {
        rows.iter().any(|row| {
            row["active"].as_bool() == Some(true)
                && row["requested_version"].as_str() == Some("latest")
        })
    })
}

/// The newest version `install`'s owner would install.
async fn latest_version(
    provider: AgentProvider,
    install: &Install,
    path: Option<&OsString>,
) -> Result<String, String> {
    let package = npm_package(provider);
    match install.manager {
        InstallManager::Homebrew | InstallManager::HomebrewCask => {
            return homebrew_latest(install, path).await;
        }
        InstallManager::Mise => return mise_latest(&install.package, path).await,
        InstallManager::Native if provider == AgentProvider::Claude => {
            return claude_channel_latest().await;
        }
        InstallManager::Pnpm => {
            if let Some(pnpm) = find_on_path("pnpm", path) {
                if let Ok(ran) = query(&pnpm, &["view", package, "version"], None, &[]).await {
                    if let Some(version) = Version::find(&ran.stdout).filter(|_| ran.ok) {
                        return Ok(version.to_string());
                    }
                }
            }
        }
        _ => {}
    }
    if let Some(npm) = find_on_path("npm", path) {
        let spec = format!("{package}@latest");
        let mut args = vec!["view", "--global"];
        let prefix = install
            .prefix
            .as_deref()
            .filter(|_| install.manager == InstallManager::Npm);
        if let Some(prefix) = prefix {
            args.extend(["--prefix", prefix]);
        }
        args.extend([spec.as_str(), "version"]);
        return match query(&npm, &args, None, &[("NO_UPDATE_NOTIFIER", "1")]).await {
            Ok(ran) if ran.ok => Version::find(&ran.stdout)
                .map(|v| v.to_string())
                .ok_or_else(|| format!("npm printed no version for {package}.")),
            // npm is configured (registry, proxy): do not go around it.
            Ok(ran) => Err(format!(
                "npm could not read the latest {package}: {}",
                first_line(&ran.stderr)
            )),
            Err(error) => Err(format!("{error}.")),
        };
    }
    http_version(&format!("{NPM_REGISTRY}/{package}/latest"), true).await
}

async fn homebrew_latest(install: &Install, path: Option<&OsString>) -> Result<String, String> {
    let brew = resolve_program("brew", install, path).ok_or("Homebrew is not on PATH.")?;
    let kind = if install.manager == InstallManager::HomebrewCask {
        "--cask"
    } else {
        "--formula"
    };
    let ran = query(
        &brew,
        &["info", "--json=v2", kind, &install.package],
        None,
        &[("HOMEBREW_NO_ENV_HINTS", "1")],
    )
    .await
    .map_err(|error| format!("{error}."))?;
    if !ran.ok {
        return Err(format!("brew info failed: {}", first_line(&ran.stderr)));
    }
    let json: serde_json::Value =
        serde_json::from_str(&ran.stdout).map_err(|_| "brew info printed no JSON.".to_string())?;
    json["casks"][0]["version"]
        .as_str()
        .or_else(|| json["formulae"][0]["versions"]["stable"].as_str())
        .and_then(Version::find)
        .map(|v| v.to_string())
        .ok_or_else(|| format!("brew info has no version for {}.", install.package))
}

async fn mise_latest(tool: &str, path: Option<&OsString>) -> Result<String, String> {
    let mise = find_on_path("mise", path).ok_or("mise is not on PATH.")?;
    let ran = query(&mise, &["latest", tool], None, &[])
        .await
        .map_err(|error| format!("{error}."))?;
    Version::find(&ran.stdout)
        .filter(|_| ran.ok)
        .map(|v| v.to_string())
        .ok_or_else(|| format!("mise latest {tool} failed: {}", first_line(&ran.stderr)))
}

/// Claude's native installer follows a release channel: the last
/// `autoUpdatesChannel` in the user settings, then the managed settings,
/// default `latest` (T3's lookup).
async fn claude_channel_latest() -> Result<String, String> {
    let channel = claude_channel(&claude_settings_texts());
    http_version(&format!("{CLAUDE_RELEASES}/{channel}"), false).await
}

fn claude_settings_texts() -> Vec<String> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let config = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| home.map(|h| h.join(".claude")));
    let managed = if cfg!(target_os = "macos") {
        PathBuf::from("/Library/Application Support/ClaudeCode")
    } else {
        PathBuf::from("/etc/claude-code")
    };
    let mut files: Vec<PathBuf> = config
        .map(|c| c.join("settings.json"))
        .into_iter()
        .collect();
    files.push(managed.join("managed-settings.json"));
    if let Ok(entries) = std::fs::read_dir(managed.join("managed-settings.d")) {
        let mut extra: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect();
        extra.sort();
        files.extend(extra);
    }
    files
        .iter()
        .filter_map(|file| std::fs::read_to_string(file).ok())
        .collect()
}

/// The channel named last among the settings texts, else `latest`.
fn claude_channel(texts: &[String]) -> String {
    texts
        .iter()
        .rev()
        .filter_map(|text| serde_json::from_str::<serde_json::Value>(text).ok())
        .filter_map(|json| json["autoUpdatesChannel"].as_str().map(String::from))
        .find(|channel| {
            !channel.is_empty()
                && channel
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
        .unwrap_or_else(|| "latest".into())
}

/// A version from a URL: npm's JSON (`{"version": …}`) or a bare version.
async fn http_version(url: &str, json: bool) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?;
    let body = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| format!("{url} could not be reached: {error}"))?
        .text()
        .await
        .map_err(|error| format!("{url} sent an unreadable answer: {error}"))?;
    let text = if json {
        serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v["version"].as_str().map(String::from))
            .unwrap_or_default()
    } else {
        body
    };
    Version::find(&text)
        .map(|v| v.to_string())
        .ok_or_else(|| format!("{url} named no version."))
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

async fn git(root: &Path, args: &[&str]) -> Result<Ran, RunError> {
    query(
        Path::new("git"),
        args,
        Some(root),
        &[
            ("GIT_TERMINAL_PROMPT", "0"),
            ("GIT_OPTIONAL_LOCKS", "0"),
            // Never prompt through an askpass helper; the user's own
            // `core.sshCommand` still applies.
            ("SSH_ASKPASS_REQUIRE", "never"),
        ],
    )
    .await
}

/// Compare the build's commit with `origin`'s default branch. Offline, only
/// the build is described.
pub async fn check_app(build: &BuildInfo, online: bool) -> AppUpdate {
    if let Some(version) = &build.release_version {
        return release::check(build, version, online).await;
    }
    let mut app = AppUpdate {
        state: AppUpdateState::Unavailable,
        built_commit: build.commit.clone(),
        remote_commit: None,
        remote_branch: None,
        local_branch: None,
        repo_root: build
            .repo_root
            .as_ref()
            .map(|root| root.to_string_lossy().into_owned()),
        command: None,
        error: None,
        release: None,
    };
    let (Some(built), Some(root)) = (build.commit.as_deref(), build.repo_root.as_deref()) else {
        return app;
    };
    if !online {
        app.state = AppUpdateState::Unknown;
        return app;
    }
    app.state = app_state(&mut app, built, root).await;
    let on_default = app.local_branch.is_none();
    app.command = app
        .repo_root
        .as_deref()
        .and_then(|root| app_update_command(root, app.state, on_default, &build.rebuild));
    app
}

async fn app_state(app: &mut AppUpdate, built: &str, root: &Path) -> AppUpdateState {
    if !root.is_dir() {
        app.error = Some(format!("The checkout at {} is gone.", root.display()));
        return AppUpdateState::Unknown;
    }
    let listed = match git(root, &["ls-remote", "--symref", "origin", "HEAD"]).await {
        Ok(ran) if ran.ok => ran.stdout,
        Ok(ran) => {
            app.error = Some(format!(
                "git could not reach origin: {}",
                first_line(&ran.stderr)
            ));
            return AppUpdateState::Unknown;
        }
        Err(error) => {
            app.error = Some(format!("{error}."));
            return AppUpdateState::Unknown;
        }
    };
    let Some((branch, remote)) = parse_ls_remote_head(&listed) else {
        app.error = Some("origin did not name a default branch.".into());
        return AppUpdateState::Unknown;
    };
    app.remote_branch = Some(format!("origin/{branch}"));
    app.remote_commit = Some(remote.clone());
    if let Ok(ran) = git(root, &["symbolic-ref", "--short", "-q", "HEAD"]).await {
        let current = ran.stdout.trim();
        if !ran.ok && current.is_empty() {
            // Detached: `git pull` has nothing to fast-forward.
            app.local_branch = Some("a detached HEAD".into());
        } else if ran.ok && !current.is_empty() && current != branch {
            app.local_branch = Some(current.to_string());
        }
    }
    if remote == built {
        return AppUpdateState::UpToDate;
    }
    let commit = format!("{remote}^{{commit}}");
    let known = matches!(git(root, &["cat-file", "-e", &commit]).await, Ok(ran) if ran.ok);
    if !known {
        return AppUpdateState::Behind;
    }
    if is_ancestor(root, &remote, built).await {
        AppUpdateState::UpToDate
    } else if is_ancestor(root, &remote, "HEAD").await {
        AppUpdateState::RebuildNeeded
    } else {
        AppUpdateState::Behind
    }
}

async fn is_ancestor(root: &Path, ancestor: &str, tip: &str) -> bool {
    matches!(
        git(root, &["merge-base", "--is-ancestor", ancestor, tip]).await,
        Ok(ran) if ran.ok
    )
}

// ---------------------------------------------------------------------------
// Report, cache and runs
// ---------------------------------------------------------------------------

/// Every provider and the app, checked concurrently.
pub async fn check_all(build: &BuildInfo, online: bool) -> UpdatesReport {
    check_all_on(build, std::env::var_os("PATH").as_ref(), online).await
}

async fn check_all_on(build: &BuildInfo, path: Option<&OsString>, online: bool) -> UpdatesReport {
    let (claude, codex, app) = tokio::join!(
        check_provider_on(AgentProvider::Claude, path, online),
        check_provider_on(AgentProvider::Codex, path, online),
        check_app(build, online),
    );
    UpdatesReport {
        checked_at: EpochMs::now(),
        online,
        providers: vec![claude, codex],
        app,
        runs: Vec::new(),
    }
}

struct Inner {
    build: BuildInfo,
    /// `PATH` for tests; the process `PATH` otherwise.
    path: Option<OsString>,
    /// Where cross-process installer locks live.
    lock_dir: Option<PathBuf>,
    /// The last online report.
    last: Mutex<Option<UpdatesReport>>,
    runs: std::sync::Mutex<Vec<ProviderRun>>,
    next_run: AtomicU64,
    /// One queue per installer.
    queues: std::sync::Mutex<HashMap<&'static str, Arc<Mutex<()>>>>,
}

/// The cached report plus the update runs of this process. Cloning shares
/// both.
#[derive(Clone)]
pub struct UpdateChecker {
    inner: Arc<Inner>,
}

/// Why an update did not start.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StartError {
    #[error("An update is already running for this provider.")]
    AlreadyRunning,
}

impl UpdateChecker {
    pub fn new(build: BuildInfo) -> Self {
        let lock_dir = directories::ProjectDirs::from("dev", "bruhs", "bruhs-ai")
            .map(|dirs| dirs.data_dir().join("updates"));
        Self::with_env(build, std::env::var_os("PATH"), lock_dir)
    }

    fn with_env(build: BuildInfo, path: Option<OsString>, lock_dir: Option<PathBuf>) -> Self {
        Self {
            inner: Arc::new(Inner {
                build,
                path,
                lock_dir,
                last: Mutex::new(None),
                runs: std::sync::Mutex::new(Vec::new()),
                next_run: AtomicU64::new(1),
                queues: std::sync::Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn build(&self) -> &BuildInfo {
        &self.inner.build
    }

    /// The runs this process knows, newest last.
    pub fn runs(&self) -> Vec<ProviderRun> {
        self.inner
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The report with the current runs. Online reports are cached for their
    /// TTL (`force` checks now); an offline report reads installed versions
    /// each time, which costs two `--version` runs.
    pub async fn report(&self, force: bool, online: bool) -> UpdatesReport {
        let mut report = if online {
            let mut last = self.inner.last.lock().await;
            match last.as_ref() {
                Some(report) if !force && report.is_fresh(EpochMs::now()) => report.clone(),
                _ => {
                    let report =
                        check_all_on(&self.inner.build, self.inner.path.as_ref(), true).await;
                    *last = Some(report.clone());
                    report
                }
            }
        } else {
            check_all_on(&self.inner.build, self.inner.path.as_ref(), false).await
        };
        report.runs = self.runs();
        report
    }

    /// Start updating `provider` in the background and return the new run.
    /// Must be called inside a tokio runtime. The run outlives the caller:
    /// a dropped request or closed window does not stop the installer.
    pub fn start_update(&self, provider: AgentProvider) -> Result<ProviderRun, StartError> {
        let run = {
            let mut runs = self
                .inner
                .runs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if runs.iter().any(|r| r.provider == provider && r.is_active()) {
                return Err(StartError::AlreadyRunning);
            }
            let run = ProviderRun {
                id: self.inner.next_run.fetch_add(1, Ordering::Relaxed),
                provider,
                phase: RunPhase::Checking,
                started_at: EpochMs::now(),
                finished_at: None,
                command: None,
                last_line: None,
                from_version: None,
                to_version: None,
                outcome: None,
                message: None,
                output: String::new(),
            };
            runs.push(run.clone());
            let finished = runs.iter().filter(|r| !r.is_active()).count();
            if finished > RUN_HISTORY {
                let mut drop = finished - RUN_HISTORY;
                runs.retain(|r| {
                    if drop > 0 && !r.is_active() {
                        drop -= 1;
                        false
                    } else {
                        true
                    }
                });
            }
            run
        };
        let checker = self.clone();
        let id = run.id;
        let drive = tokio::spawn(async move { checker.drive(id, provider).await });
        // A run that panics must not stay "active" (and refuse every later
        // update with 409) forever.
        let watcher = self.clone();
        tokio::spawn(async move {
            if drive.await.is_err() {
                watcher.finish(id, RunOutcome::Failed, "The update stopped unexpectedly.");
            }
        });
        Ok(run)
    }

    fn edit(&self, id: u64, change: impl FnOnce(&mut ProviderRun)) {
        let mut runs = self
            .inner
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(run) = runs.iter_mut().find(|r| r.id == id) {
            change(run);
        }
    }

    fn finish(&self, id: u64, outcome: RunOutcome, message: impl Into<String>) {
        let message = message.into();
        self.edit(id, |run| {
            run.phase = RunPhase::Done;
            run.outcome = Some(outcome);
            run.message = Some(message);
            run.finished_at = Some(EpochMs::now());
        });
    }

    /// Why `update` needs no installer run, if it does not.
    fn nothing_to_run(update: &ProviderUpdate) -> Option<(RunOutcome, String)> {
        if update.state == ProviderUpdateState::UpToDate {
            return Some((RunOutcome::AlreadyCurrent, "Already up to date.".into()));
        }
        if update.update_command.is_none() || update.install.is_none() {
            let why = update
                .manual_hint
                .clone()
                .unwrap_or_else(|| update.status_line());
            return Some((RunOutcome::Failed, why));
        }
        if !update.is_outdated() {
            return Some((RunOutcome::Failed, update.status_line()));
        }
        None
    }

    /// Put a fresh provider entry in the cached report, so a version that
    /// moved outside bruhs (a terminal, another bruhs) stops reading as
    /// outdated without waiting for the TTL.
    async fn remember(&self, update: &ProviderUpdate) {
        let mut last = self.inner.last.lock().await;
        if let Some(report) = last.take() {
            *last = Some(report.with_provider(update.clone()));
        }
    }

    async fn drive(&self, id: u64, provider: AgentProvider) {
        let path = self.inner.path.as_ref();
        // 1. Re-resolve who owns the CLI and what is latest.
        let first = check_provider_on(provider, path, true).await;
        self.edit(id, |run| {
            run.from_version.clone_from(&first.installed);
            run.to_version.clone_from(&first.latest);
            run.command = first.command_line();
        });
        if let Some((outcome, message)) = Self::nothing_to_run(&first) {
            self.remember(&first).await;
            self.finish(id, outcome, message);
            return;
        }
        let manager = first
            .install
            .as_ref()
            .map_or(InstallManager::Unknown, |install| install.manager);

        // 2. Wait for other updates through the same installer, here and in
        //    other bruhs processes.
        let queue = {
            let mut queues = self
                .inner
                .queues
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            Arc::clone(queues.entry(manager.key()).or_default())
        };
        let mut waited = false;
        let _queued = match queue.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                waited = true;
                self.edit(id, |run| run.phase = RunPhase::Queued);
                queue.lock().await
            }
        };
        let _file_lock = match self.lock_installer(id, manager.key()).await {
            Ok((lock, other_process)) => {
                waited |= other_process;
                lock
            }
            Err(message) => {
                self.finish(id, RunOutcome::Failed, message);
                return;
            }
        };

        // 3. Whoever held the lock may have updated it already: check again.
        let before = if waited {
            self.edit(id, |run| run.phase = RunPhase::Checking);
            let again = check_provider_on(provider, path, true).await;
            self.edit(id, |run| {
                run.from_version.clone_from(&again.installed);
                run.to_version.clone_from(&again.latest);
                run.command = again.command_line();
            });
            if let Some((outcome, message)) = Self::nothing_to_run(&again) {
                self.remember(&again).await;
                self.finish(id, outcome, message);
                return;
            }
            again
        } else {
            first
        };
        let (Some(argv), Some(install)) = (before.update_command.clone(), before.install.clone())
        else {
            self.finish(id, RunOutcome::Failed, before.status_line());
            return;
        };

        // 4. Run it.
        self.edit(id, |run| run.phase = RunPhase::Running);
        let Some((program, args)) = argv.split_first() else {
            self.finish(id, RunOutcome::Failed, "The update command is empty.");
            return;
        };
        let Some(program) = resolve_program(program, &install, path) else {
            self.finish(
                id,
                RunOutcome::Failed,
                format!("`{program}` is not on PATH."),
            );
            return;
        };
        let mut env = vec![
            ("NONINTERACTIVE", "1".to_string()),
            ("HOMEBREW_NO_ENV_HINTS", "1".to_string()),
            ("NO_UPDATE_NOTIFIER", "1".to_string()),
        ];
        if provider == AgentProvider::Codex && install.manager == InstallManager::Native {
            if let Some(home) = &install.prefix {
                env.push(("CODEX_HOME", home.clone()));
            }
        }
        if let Some(path) = path {
            env.push(("PATH", path.to_string_lossy().into_owned()));
        }
        if let Err(message) = self.run_installer(id, &program, args, &env).await {
            self.finish(id, RunOutcome::Failed, message);
            return;
        }

        // 5. Verify.
        self.edit(id, |run| run.phase = RunPhase::Verifying);
        let after = check_provider_on(provider, path, true).await;
        self.remember(&after).await;
        let moved = after.installed != before.installed;
        if !after.is_outdated() && (moved || after.state == ProviderUpdateState::UpToDate) {
            self.edit(id, |run| run.to_version.clone_from(&after.installed));
            self.finish(id, RunOutcome::Updated, "Provider updated.");
        } else {
            let label = provider_cli_label(provider);
            let message = if moved {
                format!("{label} updated, but a newer version is still available.")
            } else {
                format!("The update finished, but {label} is still the same version.")
            };
            self.finish(id, RunOutcome::Unchanged, message);
        }
    }

    /// Take the installer's lock file, marking the run queued while another
    /// bruhs process holds it. The flag says whether it had to wait. No lock
    /// directory (or one that cannot be created) means no cross-process
    /// lock; the installer then runs as it would from a terminal.
    async fn lock_installer(
        &self,
        id: u64,
        key: &'static str,
    ) -> Result<(Option<InstallerLock>, bool), String> {
        let Some(dir) = self.inner.lock_dir.clone() else {
            return Ok((None, false));
        };
        let path = dir.join(format!("{key}.lock"));
        match InstallerLock::try_take(&dir, &path) {
            Ok(Some(lock)) => Ok((Some(lock), false)),
            Ok(None) => {
                self.edit(id, |run| run.phase = RunPhase::Queued);
                let waited = tokio::task::spawn_blocking(move || InstallerLock::take(&path)).await;
                match waited {
                    Ok(Ok(lock)) => Ok((Some(lock), true)),
                    Ok(Err(error)) => Err(format!(
                        "Could not wait for the other {key} update to finish: {error}"
                    )),
                    Err(error) => Err(format!(
                        "Could not wait for the other {key} update to finish: {error}"
                    )),
                }
            }
            Err(error) => {
                tracing::warn!("installer lock {}: {error}", path.display());
                Ok((None, false))
            }
        }
    }

    /// Run the installer, following its latest line. Its output goes to a
    /// file, not pipes, and it runs in its own process group: quitting bruhs
    /// (or Ctrl-C on `scripts/dev.sh`) never cuts an install in half, and a
    /// helper process that keeps the output open cannot hold the run past
    /// the installer's exit. A timeout stops the whole group.
    async fn run_installer(
        &self,
        id: u64,
        program: &Path,
        args: &[String],
        env: &[(&str, String)],
    ) -> Result<(), String> {
        let log = tempfile::NamedTempFile::new()
            .map_err(|error| format!("The installer's log could not be created: {error}"))?;
        let open_error = |error: std::io::Error| format!("The installer's log failed: {error}");
        let stdout = log.as_file().try_clone().map_err(open_error)?;
        let stderr = stdout.try_clone().map_err(open_error)?;
        let mut reader = tokio::fs::File::from_std(log.reopen().map_err(open_error)?);
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .kill_on_drop(false);
        #[cfg(unix)]
        command.process_group(0);
        for (key, value) in env {
            command.env(key, value);
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("`{}` could not start: {error}", program_name(program)))?;
        let group = child.id().and_then(|pid| i32::try_from(pid).ok());

        let mut all = String::new();
        let mut chunk = Vec::new();
        let follow = |all: &mut String, chunk: &mut Vec<u8>| {
            if chunk.is_empty() {
                return;
            }
            all.push_str(&String::from_utf8_lossy(chunk));
            chunk.clear();
            if all.len() > RUN_OUTPUT_LIMIT * 2 {
                let mut cut = all.len() - RUN_OUTPUT_LIMIT;
                while !all.is_char_boundary(cut) {
                    cut += 1;
                }
                all.drain(..cut);
            }
            let last = last_output_line(all);
            self.edit(id, |run| run.last_line = last);
        };
        let deadline = tokio::time::Instant::now() + UPDATE_TIMEOUT;
        let status = loop {
            tokio::select! {
                status = child.wait() => break Some(status),
                () = tokio::time::sleep(Duration::from_millis(250)) => {}
            }
            let _ = reader.read_to_end(&mut chunk).await;
            follow(&mut all, &mut chunk);
            if tokio::time::Instant::now() >= deadline {
                break None;
            }
        };
        let _ = reader.read_to_end(&mut chunk).await;
        follow(&mut all, &mut chunk);
        let last = last_output_line(&all).unwrap_or_default();
        let tail = tail(&all, RUN_OUTPUT_LIMIT);
        self.edit(id, |run| run.output = tail);
        match status {
            None => {
                kill_group(group);
                let _ = child.kill().await;
                Err("Update timed out after 30 minutes.".into())
            }
            Some(Err(error)) => Err(format!("The installer could not be waited on: {error}")),
            Some(Ok(status)) if status.success() => Ok(()),
            Some(Ok(status)) => Err(match status.code() {
                Some(code) => format!("Update command exited with code {code}: {last}"),
                None => format!("Update command was stopped: {last}"),
            }
            .trim_end_matches(": ")
            .to_string()),
        }
    }
}

/// Stop an installer's whole process group.
#[cfg(unix)]
fn kill_group(group: Option<i32>) {
    if let Some(id) = group.filter(|id| *id > 0) {
        // SAFETY: a positive PID of our own child, which leads its own process
        // group (`process_group(0)`); the negative PID targets that group. No
        // pointers or shared memory are involved.
        unsafe {
            libc::kill(-id, libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
fn kill_group(_group: Option<i32>) {}

fn tail(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    if trimmed.len() <= limit {
        return trimmed.to_string();
    }
    let mut start = trimmed.len() - limit;
    while !trimmed.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &trimmed[start..])
}

/// An exclusive `flock` on a per-installer file, released when dropped (or
/// when the process dies).
struct InstallerLock {
    _file: std::fs::File,
}

impl InstallerLock {
    fn open(path: &Path) -> std::io::Result<std::fs::File> {
        std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
    }

    /// The lock now, or `None` when another process holds it.
    fn try_take(dir: &Path, path: &Path) -> std::io::Result<Option<Self>> {
        std::fs::create_dir_all(dir)?;
        let file = Self::open(path)?;
        if flock(&file, true)? {
            Ok(Some(Self { _file: file }))
        } else {
            Ok(None)
        }
    }

    /// Block until the lock is free.
    fn take(path: &Path) -> std::io::Result<Self> {
        let file = Self::open(path)?;
        flock(&file, false)?;
        Ok(Self { _file: file })
    }
}

/// `flock(LOCK_EX)`; with `nonblocking`, `Ok(false)` when it is held.
#[cfg(unix)]
fn flock(file: &std::fs::File, nonblocking: bool) -> std::io::Result<bool> {
    use std::os::fd::AsRawFd;
    let flags = if nonblocking {
        libc::LOCK_EX | libc::LOCK_NB
    } else {
        libc::LOCK_EX
    };
    loop {
        // SAFETY: the descriptor belongs to `file`, which outlives the call;
        // flock takes no pointers.
        let rc = unsafe { libc::flock(file.as_raw_fd(), flags) };
        if rc == 0 {
            return Ok(true);
        }
        let error = std::io::Error::last_os_error();
        match error.kind() {
            std::io::ErrorKind::Interrupted => {}
            std::io::ErrorKind::WouldBlock if nonblocking => return Ok(false),
            _ => return Err(error),
        }
    }
}

#[cfg(not(unix))]
fn flock(_file: &std::fs::File, _nonblocking: bool) -> std::io::Result<bool> {
    Ok(true)
}

pub mod release;

#[cfg(test)]
#[cfg(unix)]
mod tests;
