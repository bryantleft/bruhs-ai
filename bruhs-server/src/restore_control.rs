//! Restores applied while nothing else runs (D15 to D17, rulings F20 to
//! F23): the check every server start makes before it opens a store, the
//! detached `bruhs-server restore-apply` that finishes a restore after the
//! server that scheduled it exits, and what a restore asks of this process
//! and the background service ([`RestoreControl`]).
//!
//! Only outcomes are logged: never a path, a name from inside a backup or
//! a keychain value. Every message is the core's fixed copy.

use std::ffi::OsString;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use bruhs_core::backup::{
    apply_pending, applying, restore_waits, take_instance_lock, take_service_restart, ApplyOutcome,
    BackupError, Holder, JobHold, RootKey, Roots,
};
use bruhs_core::secrets::{SecretChoice, SecretStore};
use bruhs_models::backup::RESTORE_ROLLED_BACK;

use crate::service::{self, Asker};

/// `bruhs-server restore-apply [--once]`.
pub(crate) const RESTORE_APPLY: &str = "restore-apply";
/// One try, for the desktop app that waits on it (ruling F23). The
/// caller may hand its instance lock over as standard input.
const ONCE: &str = "--once";
/// The variable that ties a server to the app that started it.
const EXIT_WITH_PARENT: &str = "BRUHS_EXIT_WITH_PARENT";
/// systemd sets it in every process a unit runs.
const INVOCATION_ID: &str = "INVOCATION_ID";

/// Nothing waits any more: applied, or never there.
pub(crate) const EXIT_DONE: i32 = 0;
/// The restore was refused and removed, or rolled back.
pub(crate) const EXIT_FAILED: i32 = 1;
/// A restore still waits (EX_TEMPFAIL).
pub(crate) const EXIT_WAITING: i32 = 75;

/// How long the detached applier keeps trying (D16).
const APPLIER_LIMIT: Duration = Duration::from_secs(10 * 60);
/// How long a server's start waits for a restore before it exits
/// (ruling F20 b).
const START_LIMIT: Duration = Duration::from_secs(60);
const PAUSE: Duration = Duration::from_secs(1);
/// Between the answer to an apply and the shutdown, so the answer leaves.
const EXIT_DELAY: Duration = Duration::from_millis(750);
/// `<core>/backups/applier.lock`: held by the detached applier for its
/// whole run (final review B minor), which no apply moves.
const APPLIER_LOCK: &str = "applier.lock";

/// What a restore asks of this process and the background service (D15,
/// ruling F22). A fake in the route tests.
pub(crate) trait RestoreControl: Send + Sync {
    /// The reactive protocol of the installed service's release, when one
    /// is installed and its release names it.
    fn service_installed_protocol(&self) -> Option<u32>;
    /// Whether the service is installed and loaded, so the applier starts
    /// it again after the restore (ruling F22 a).
    fn service_loaded(&self) -> bool;
    /// Whether this process is the background service.
    fn is_service_process(&self) -> bool;
    /// Stops the background service if it is loaded: `Ok(true)` when it
    /// was, so the applier starts it again.
    fn stop_service(&self) -> Result<bool, String>;
    /// Starts `bruhs-server restore-apply`, outliving this process and its
    /// service.
    fn spawn_applier(&self) -> std::io::Result<()>;
    /// Shuts this server down gracefully a moment from now, keeping new
    /// backup jobs out (`hold`) until it exits (final review C-M2).
    fn exit_soon(&self, hold: JobHold);
}

/// The control the restore routes take as an `Extension`.
pub(crate) type Control = std::sync::Arc<dyn RestoreControl>;

/// The real control, for the server's routes over `roots`.
pub(crate) fn real(roots: &Roots) -> Control {
    std::sync::Arc::new(RealRestoreControl::new(roots.core.clone()))
}

/// The real control: this process, launchd or systemd.
pub(crate) struct RealRestoreControl {
    /// The core data root, whose `logs/restore.log` takes the applier's
    /// output.
    core: PathBuf,
}

impl RealRestoreControl {
    pub(crate) fn new(core: PathBuf) -> Self {
        Self { core }
    }

    fn status() -> bruhs_models::api::BackgroundService {
        service::status(Asker::Server {
            pid: std::process::id(),
        })
    }
}

impl RestoreControl for RealRestoreControl {
    fn service_installed_protocol(&self) -> Option<u32> {
        let status = Self::status();
        status
            .installed
            .then_some(status.reactive_protocol)
            .flatten()
    }

    fn service_loaded(&self) -> bool {
        let status = Self::status();
        status.installed && status.loaded
    }

    fn is_service_process(&self) -> bool {
        Self::status().service_pid == Some(std::process::id())
    }

    fn stop_service(&self) -> Result<bool, String> {
        service::stop_for_restore(Asker::Server {
            pid: std::process::id(),
        })
        .map_err(|error| error.to_string())
    }

    fn spawn_applier(&self) -> std::io::Result<()> {
        let exe = std::env::current_exe()?;
        let log = bruhs_core::backup::open_restore_log(&self.core).ok();
        let env: Vec<_> = std::env::vars_os().collect();
        let mut command = applier_command(&exe, log, &env);
        if command.get_program() == SYSTEMD_RUN {
            // systemd-run returns once the unit runs.
            let status = command.status()?;
            if !status.success() {
                return Err(std::io::Error::other("systemd-run refused the applier"));
            }
        } else {
            // Reaped by init once this process exits.
            drop(command.spawn()?);
        }
        Ok(())
    }

    fn exit_soon(&self, hold: JobHold) {
        hold.until_exit();
        let spawned = std::thread::Builder::new()
            .name("restore-exit".into())
            .spawn(|| {
                std::thread::sleep(EXIT_DELAY);
                EXIT.notify_one();
            });
        if spawned.is_err() {
            EXIT.notify_one();
        }
    }
}

/// Asked by [`RestoreControl::exit_soon`]; `serve` stops on it as on
/// SIGTERM.
static EXIT: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// Resolves once a restore asks this server to stop.
pub(crate) async fn exit_requested() {
    EXIT.notified().await;
}

const SYSTEMD_RUN: &str = "systemd-run";

/// The applier's command: this executable with `restore-apply`, without
/// the parent watch, its output appended to `log`, in a process group of
/// its own, or, under systemd (ruling F22 b), in a transient unit of its
/// own, since stopping the service's unit stops its whole cgroup. `env`
/// is this process's environment.
fn applier_command(exe: &Path, log: Option<File>, env: &[(OsString, OsString)]) -> Command {
    let mut command = if env.iter().any(|(key, _)| key == INVOCATION_ID) {
        let mut command = Command::new(SYSTEMD_RUN);
        command.args(["--user", "--collect", "--quiet"]);
        command.arg(format!(
            "--unit=bruhs-restore-{}",
            uuid::Uuid::new_v4().simple()
        ));
        // The unit starts with the user manager's environment; carry this
        // process's settings and the folders the roots resolve from, its
        // home among them, so a scratch home's restore stays there (final
        // review C-M6). Only names: systemd-run copies each value from its
        // own environment.
        for (key, _) in env {
            let Some(key) = key.to_str() else { continue };
            let carried = (key.starts_with("BRUHS_") && key != EXIT_WITH_PARENT)
                || key.starts_with("XDG_")
                || key == "HOME"
                || key == "PATH";
            if carried {
                command.arg(format!("--setenv={key}"));
            }
        }
        command.arg(exe).arg(RESTORE_APPLY);
        command
    } else {
        let mut command = Command::new(exe);
        command.arg(RESTORE_APPLY);
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        command
    };
    command.env_remove(EXIT_WITH_PARENT).stdin(Stdio::null());
    match log.and_then(|log| Some((log.try_clone().ok()?, log))) {
        Some((out, err)) => command.stdout(out).stderr(err),
        None => command.stdout(Stdio::null()).stderr(Stdio::null()),
    };
    command
}

type ApplyResult = Result<ApplyOutcome, BackupError>;

/// What one try came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Attempt {
    /// Nothing waits: applied, or never there.
    Done,
    /// Refused for good, or rolled back; nothing waits any more.
    Failed,
    /// A lock is held, or the try ran out of time.
    Busy,
    /// An error left the restore waiting for another try.
    Retry,
}

/// How often a try that keeps waiting logs it.
const WAITING_LOG_EVERY: Duration = Duration::from_secs(10);

/// One caller's tries at the waiting restore.
struct Applier<'a> {
    roots: &'a Roots,
    /// One `apply_pending`; `None` when it ran out of time.
    apply: &'a dyn Fn() -> Option<ApplyResult>,
    /// Starts the background service again.
    start_service: &'a dyn Fn(),
    /// When a try last logged that it waits.
    logged: std::cell::Cell<Option<Instant>>,
}

impl<'a> Applier<'a> {
    fn new(
        roots: &'a Roots,
        apply: &'a dyn Fn() -> Option<ApplyResult>,
        start_service: &'a dyn Fn(),
    ) -> Self {
        Self {
            roots,
            apply,
            start_service,
            logged: std::cell::Cell::new(None),
        }
    }

    /// Logs that the restore waits, at most once every ten seconds.
    fn waiting(&self, why: &str) {
        let due = self
            .logged
            .get()
            .is_none_or(|last| last.elapsed() >= WAITING_LOG_EVERY);
        if due {
            self.logged.set(Some(Instant::now()));
            tracing::info!("a restore waits for {why}");
        }
    }

    fn attempt(&self) -> Attempt {
        let attempt = match (self.apply)() {
            None => {
                self.waiting("the apply still running");
                Attempt::Busy
            }
            Some(Ok(ApplyOutcome::NothingPending)) => Attempt::Done,
            Some(Ok(ApplyOutcome::Applied(result))) => {
                tracing::info!(warnings = result.warnings.len(), "the restore is in place");
                Attempt::Done
            }
            Some(Ok(ApplyOutcome::Busy(holder))) => {
                self.waiting(holder_name(holder));
                Attempt::Busy
            }
            Some(Err(error)) => {
                tracing::warn!("restore not applied: {}", error.message());
                let rolled_back =
                    matches!(&error, BackupError::Refused(copy) if *copy == RESTORE_ROLLED_BACK);
                if !rolled_back && restore_waits(self.roots) {
                    Attempt::Retry
                } else {
                    Attempt::Failed
                }
            }
        };
        if matches!(attempt, Attempt::Done | Attempt::Failed) {
            // A rollback keeps the flag too (B Task 8 N1).
            if take_service_restart(self.roots) {
                (self.start_service)();
            }
        }
        attempt
    }

    /// Tries every `pause` until nothing waits or `limit` passes.
    fn run(&self, limit: Duration, pause: Duration) -> i32 {
        let deadline = Instant::now() + limit;
        loop {
            match self.attempt() {
                Attempt::Done => return EXIT_DONE,
                Attempt::Failed => return EXIT_FAILED,
                Attempt::Busy | Attempt::Retry if Instant::now() >= deadline => {
                    tracing::warn!("the restore didn't finish in time; it applies at next start");
                    return EXIT_WAITING;
                }
                Attempt::Busy | Attempt::Retry => std::thread::sleep(pause),
            }
        }
    }
}

fn holder_name(holder: Holder) -> &'static str {
    match holder {
        Holder::Server => "a running server",
        Holder::Desktop => "the desktop app",
        Holder::Restore => "another apply",
    }
}

/// Starts the background service, logging a failure.
fn start_service() {
    match service::start_after_restore() {
        Ok(()) => tracing::info!("the background service starts again"),
        Err(error) => tracing::warn!("the background service didn't start again: {error}"),
    }
}

/// `bruhs-server restore-apply [--once]`: applies the waiting restore,
/// trying every second for up to 10 minutes, then starts the background
/// service again when the restore stopped it (D16). With `--once` it tries
/// once and answers [`EXIT_WAITING`] when the restore still waits; a
/// caller's instance lock on standard input counts as the caller's own.
/// `None` when the arguments name another command.
pub(crate) fn restore_apply_cli() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(RESTORE_APPLY.as_ref()) {
        return None;
    }
    let once = match args.next() {
        None => false,
        Some(arg) if arg == ONCE && args.next().is_none() => true,
        Some(_) => {
            eprintln!("Use: bruhs-server restore-apply [--once]");
            return Some(2);
        }
    };
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(std::io::stderr),
        )
        .init();
    let Some(roots) = Roots::from_env() else {
        tracing::error!("no home folder to restore into");
        return Some(EXIT_FAILED);
    };
    let store = SecretChoice::from_env().store();
    let lock = if once { stdin_lock() } else { None };
    // A start that gives up on an erroring restore must not open its
    // stores while this applier may still swap the data in.
    let _alive = (!once).then(|| hold_applier_lock(&roots)).flatten();
    let apply = || Some(apply_pending(&roots, &store, lock.as_ref()));
    let applier = Applier::new(&roots, &apply, &start_service);
    Some(if once {
        match applier.attempt() {
            Attempt::Done => EXIT_DONE,
            Attempt::Failed => EXIT_FAILED,
            Attempt::Busy | Attempt::Retry => EXIT_WAITING,
        }
    } else {
        tracing::info!("applying the waiting restore");
        applier.run(APPLIER_LIMIT, PAUSE)
    })
}

fn applier_lock_path(roots: &Roots) -> PathBuf {
    roots.backups(RootKey::Core).join(APPLIER_LOCK)
}

/// The detached applier's lock, held for as long as the answer lives.
/// `None`, logged, when another applier holds it or it can't be taken:
/// this one tries anyway, under `restore.lock` like any other.
fn hold_applier_lock(roots: &Roots) -> Option<File> {
    match take_instance_lock(&applier_lock_path(roots)) {
        Ok(Some(lock)) => Some(lock),
        Ok(None) => {
            tracing::info!("another applier is running");
            None
        }
        Err(_) => {
            tracing::warn!("the applier couldn't take its lock");
            None
        }
    }
}

/// Whether a detached applier is running now: its lock is held.
fn applier_alive(roots: &Roots) -> bool {
    matches!(take_instance_lock(&applier_lock_path(roots)), Ok(None))
}

/// The caller's instance lock, handed over as standard input: kept only
/// when this process can hold its lock, which a description that already
/// holds it can (ruling F23). Anything else on standard input is ignored.
fn stdin_lock() -> Option<File> {
    use std::os::fd::AsFd;
    handed_lock(File::from(
        std::io::stdin().as_fd().try_clone_to_owned().ok()?,
    ))
}

/// `file` when this process holds its lock: a handle on the very open
/// file whose lock the caller holds, or a lock nobody holds.
fn handed_lock(file: File) -> Option<File> {
    file.try_lock().ok()?;
    Some(file)
}

/// The start check every server makes before it opens a store (D16,
/// ruling F20 b): applies a waiting restore, or waits up to a minute for
/// the apply that holds it. `Some(code)` when this server must exit
/// without opening anything: a restore still waits behind a held lock, an
/// apply is half done, or an erroring restore waits while the detached
/// applier, which may still apply it, runs (final review B minor). Each
/// try's keychain calls get the time left, and a try that runs out counts
/// as busy.
pub(crate) async fn apply_at_start() -> Option<i32> {
    let roots = Roots::from_env()?;
    let checked = tokio::task::spawn_blocking(move || {
        let store = SecretChoice::from_env().store();
        check_at_start(&roots, &store, &start_service, START_LIMIT, PAUSE)
    })
    .await;
    match checked {
        Ok(code) => code,
        Err(_) => {
            tracing::error!("the restore check at start stopped");
            Some(EXIT_WAITING)
        }
    }
}

fn check_at_start(
    roots: &Roots,
    store: &SecretStore,
    start_service: &dyn Fn(),
    limit: Duration,
    pause: Duration,
) -> Option<i32> {
    if !restore_waits(roots) {
        return None;
    }
    let deadline = Instant::now() + limit;
    let apply = || {
        bounded(
            roots,
            store,
            deadline.saturating_duration_since(Instant::now()),
        )
    };
    start_check(roots, &apply, start_service, deadline, pause)
}

/// [`check_at_start`]'s tries, each one `apply`, until `deadline`.
fn start_check(
    roots: &Roots,
    apply: &dyn Fn() -> Option<ApplyResult>,
    start_service: &dyn Fn(),
    deadline: Instant,
    pause: Duration,
) -> Option<i32> {
    let applier = Applier::new(roots, apply, start_service);
    loop {
        let attempt = applier.attempt();
        let late = Instant::now() >= deadline;
        match attempt {
            Attempt::Done | Attempt::Failed => return None,
            Attempt::Busy if late => {
                tracing::warn!("a restore waits; this server stops without opening its data");
                return Some(EXIT_WAITING);
            }
            Attempt::Retry if late && applying(roots) => {
                tracing::warn!(
                    "a restore is half applied; this server stops without opening its data"
                );
                return Some(EXIT_WAITING);
            }
            // The detached applier may still apply it under this start's
            // stores (final review B minor): stop, as for a held lock.
            Attempt::Retry if late && applier_alive(roots) => {
                tracing::warn!("a restore waits; this server stops without opening its data");
                return Some(EXIT_WAITING);
            }
            // Nothing moved yet and no applier runs: today's data is
            // whole, and the restore waits for the next start.
            Attempt::Retry if late => return None,
            Attempt::Busy | Attempt::Retry => std::thread::sleep(pause),
        }
    }
}

/// `apply_pending` on a thread of its own, waiting at most `wait` for it.
/// A try still running then goes on in the background; it holds
/// `restore.lock`, so nothing else applies meanwhile.
fn bounded(roots: &Roots, store: &SecretStore, wait: Duration) -> Option<ApplyResult> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let (roots, store) = (roots.clone(), store.clone());
    let spawned = std::thread::Builder::new()
        .name("restore-at-start".into())
        .spawn(move || {
            // The receiver may be gone after a timeout.
            let _ = sender.send(apply_pending(&roots, &store, None));
        });
    if spawned.is_err() {
        return Some(Err(BackupError::Io(std::io::Error::other(
            "no thread for the restore",
        ))));
    }
    receiver.recv_timeout(wait).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_core::backup::test_support::{note, stage_restore, RESTORED, TODAY};
    use std::cell::Cell;
    use std::time::{Duration, Instant};

    struct Fixture {
        _dir: tempfile::TempDir,
        roots: Roots,
        store: SecretStore,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(dir.path()).unwrap();
        let home = base.join("home");
        let core = home.join("core");
        let roots = Roots {
            home: home.clone(),
            core: core.clone(),
            desktop: home.join("desktop"),
            config: core,
        };
        for folder in [&roots.core, &roots.desktop] {
            std::fs::create_dir_all(folder).unwrap();
        }
        Fixture {
            _dir: dir,
            roots,
            store: SecretStore::in_memory(),
        }
    }

    /// Holds `path`'s lock until the returned handle is dropped.
    fn hold(path: &Path) -> File {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = File::create(path).unwrap();
        file.lock().unwrap();
        file
    }

    fn applier<'a>(
        fixture: &'a Fixture,
        started: &'a Cell<u32>,
    ) -> (impl Fn() -> Option<ApplyResult> + 'a, impl Fn() + 'a) {
        let apply = move || Some(apply_pending(&fixture.roots, &fixture.store, None));
        let start = move || started.set(started.get() + 1);
        (apply, start)
    }

    #[test]
    fn the_applier_finishes_then_restarts_the_service() {
        for restart in [true, false] {
            let fixture = fixture();
            stage_restore(&fixture.roots, &fixture.store, restart).unwrap();
            // A server that is still shutting down holds the reactive lock.
            let server = hold(&fixture.roots.core.join("server/reactive.sqlite3.lock"));
            let release = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(2));
                drop(server);
            });
            let started = Cell::new(0);
            let (apply, start) = applier(&fixture, &started);
            let begun = Instant::now();
            let code = Applier::new(&fixture.roots, &apply, &start)
                .run(Duration::from_secs(30), Duration::from_secs(1));
            release.join().unwrap();
            assert_eq!(code, EXIT_DONE);
            assert!(begun.elapsed() >= Duration::from_secs(2));
            assert_eq!(note(&fixture.roots).as_deref(), Some(RESTORED));
            assert_eq!(started.get(), u32::from(restart), "restart {restart}");
            assert!(!restore_waits(&fixture.roots));
        }
    }

    #[test]
    fn the_applier_gives_up_after_its_limit() {
        let fixture = fixture();
        stage_restore(&fixture.roots, &fixture.store, true).unwrap();
        let _server = hold(&fixture.roots.core.join("server/reactive.sqlite3.lock"));
        let started = Cell::new(0);
        let (apply, start) = applier(&fixture, &started);
        let code = Applier::new(&fixture.roots, &apply, &start)
            .run(Duration::from_millis(600), Duration::from_millis(100));
        assert_eq!(code, EXIT_WAITING);
        assert_eq!(note(&fixture.roots).as_deref(), Some(TODAY));
        assert_eq!(started.get(), 0);
        assert!(restore_waits(&fixture.roots));
    }

    #[test]
    fn the_applier_leaves_the_parents_lifetime() {
        let dir = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(dir.path()).unwrap();
        let log = base.join("logs/restore.log");
        let open_log = || bruhs_core::backup::open_restore_log(&base).unwrap();
        // Outside systemd: this executable, its own process group, no
        // parent watch, output appended to the log.
        let script = base.join("applier.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\necho \"args $*\"\necho \"pgid $(ps -o pgid= -p $$ | tr -d ' ')\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let env: Vec<(OsString, OsString)> = vec![
            ("BRUHS_SECRET_STORE".into(), "file:/keys".into()),
            (EXIT_WITH_PARENT.into(), "1".into()),
            ("XDG_DATA_HOME".into(), "/data".into()),
            ("HOME".into(), "/home".into()),
        ];
        let command = applier_command(&script, Some(open_log()), &env);
        assert_eq!(command.get_program(), script.as_os_str());
        assert!(command
            .get_envs()
            .any(|(key, value)| key == EXIT_WITH_PARENT && value.is_none()));
        let mut command = command;
        let mut child = command.spawn().unwrap();
        let pid = child.id();
        assert!(child.wait().unwrap().success());
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(text.contains(&format!("args {RESTORE_APPLY}")), "{text}");
        assert!(text.contains(&format!("pgid {pid}")), "{text}");
        // Under systemd the applier gets a unit of its own, so stopping the
        // service's cgroup doesn't stop it.
        let mut under_systemd = env;
        under_systemd.push((INVOCATION_ID.into(), "abc".into()));
        let command = applier_command(&script, Some(open_log()), &under_systemd);
        assert_eq!(command.get_program(), "systemd-run");
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(args.starts_with(&["--user".into(), "--collect".into()]));
        assert!(args
            .iter()
            .any(|arg| arg.starts_with("--unit=bruhs-restore-")));
        assert!(!args.iter().any(|arg| arg.contains(EXIT_WITH_PARENT)));
        // Names only, never values: the settings and the roots' folders.
        for carried in [
            "--setenv=BRUHS_SECRET_STORE",
            "--setenv=XDG_DATA_HOME",
            "--setenv=HOME",
        ] {
            assert!(args.iter().any(|arg| arg == carried), "{args:?}");
        }
        assert!(!args
            .iter()
            .any(|arg| arg.contains("/keys") || arg.contains("HOME=")));
        assert!(args.ends_with(&[script.display().to_string(), RESTORE_APPLY.into()]));
    }

    #[test]
    fn a_start_opens_no_store_while_a_restore_waits() {
        let fixture = fixture();
        let started = Cell::new(0);
        let (_, start) = applier(&fixture, &started);
        let check = |limit| {
            check_at_start(
                &fixture.roots,
                &fixture.store,
                &start,
                limit,
                Duration::from_millis(100),
            )
        };
        // Nothing waits: the server starts at once.
        assert_eq!(check(Duration::from_secs(5)), None);
        stage_restore(&fixture.roots, &fixture.store, true).unwrap();
        // Another apply holds restore.lock: the server exits without
        // opening anything, the reactive database included (ruling F20 c).
        let restore = hold(&fixture.roots.core.join("backups/restore.lock"));
        assert_eq!(check(Duration::from_millis(600)), Some(EXIT_WAITING));
        assert!(!fixture.roots.core.join("server/reactive.sqlite3").exists());
        assert_eq!(note(&fixture.roots).as_deref(), Some(TODAY));
        drop(restore);
        // Free again: this start applies it, and starts the service.
        assert_eq!(check(Duration::from_secs(30)), None);
        assert_eq!(note(&fixture.roots).as_deref(), Some(RESTORED));
        assert_eq!(started.get(), 1);
    }

    /// Final review B minor: a start that gives up on an erroring restore
    /// opens its stores only once no detached applier, which could still
    /// swap the data in, runs.
    #[test]
    fn a_start_never_opens_beside_a_live_applier() {
        let fixture = fixture();
        stage_restore(&fixture.roots, &fixture.store, false).unwrap();
        let started = Cell::new(0);
        let start = || started.set(started.get() + 1);
        let erroring = || Some(Err(BackupError::Io(std::io::Error::other("not yet"))));
        let check = || {
            start_check(
                &fixture.roots,
                &erroring,
                &start,
                Instant::now() + Duration::from_millis(300),
                Duration::from_millis(100),
            )
        };
        let alive = hold_applier_lock(&fixture.roots).unwrap();
        assert!(applier_alive(&fixture.roots));
        assert!(hold_applier_lock(&fixture.roots).is_none(), "one applier");
        assert_eq!(check(), Some(EXIT_WAITING));
        drop(alive);
        assert!(!applier_alive(&fixture.roots));
        assert_eq!(check(), None);
        assert!(restore_waits(&fixture.roots), "it waits for the next start");
        assert_eq!(started.get(), 0);
    }

    /// A restore refused at plan time still starts the service it stopped
    /// (B Task 10 I5).
    #[test]
    fn a_refused_restore_still_starts_the_service_again() {
        let fixture = fixture();
        stage_restore(&fixture.roots, &fixture.store, true).unwrap();
        // The staged data went away: the restore is GONE.
        std::fs::remove_dir_all(fixture.roots.core.join("backups/staging")).unwrap();
        let started = Cell::new(0);
        let (apply, start) = applier(&fixture, &started);
        let code = Applier::new(&fixture.roots, &apply, &start)
            .run(Duration::from_secs(10), Duration::from_millis(100));
        assert_eq!(code, EXIT_FAILED);
        assert_eq!(started.get(), 1);
        assert!(!restore_waits(&fixture.roots));
        // The flag is taken once, and the record names no restore.
        assert!(!take_service_restart(&fixture.roots));
        assert!(bruhs_core::backup::result(&fixture.roots).is_none());
        assert_eq!(note(&fixture.roots).as_deref(), Some(TODAY));
    }

    #[test]
    fn a_handed_over_desktop_lock_counts_as_the_callers() {
        let fixture = fixture();
        stage_restore(&fixture.roots, &fixture.store, false).unwrap();
        let path = bruhs_core::backup::instance_lock_path(&fixture.roots.desktop);
        let app = bruhs_core::backup::take_instance_lock(&path)
            .unwrap()
            .unwrap();
        // Another handle on the same file doesn't hold the app's lock.
        assert!(handed_lock(File::open(&path).unwrap()).is_none());
        // The app's own handle, as `--once` gets it on standard input, does.
        let handed = handed_lock(app.try_clone().unwrap()).unwrap();
        assert_eq!(
            apply_pending(&fixture.roots, &fixture.store, None).unwrap(),
            ApplyOutcome::Busy(Holder::Desktop)
        );
        assert!(matches!(
            apply_pending(&fixture.roots, &fixture.store, Some(&handed)).unwrap(),
            ApplyOutcome::Applied(_)
        ));
        assert_eq!(note(&fixture.roots).as_deref(), Some(RESTORED));
        drop(handed);
        // The app still holds its lock.
        assert!(bruhs_core::backup::take_instance_lock(&path)
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn exit_soon_stops_the_server_the_usual_way() {
        let dir = tempfile::tempdir().unwrap();
        let host = bruhs_core::backup::BackupHost::new(
            Roots {
                home: dir.path().to_path_buf(),
                core: dir.path().join("core"),
                desktop: dir.path().join("desktop"),
                config: dir.path().join("core"),
            },
            SecretStore::in_memory(),
        );
        RealRestoreControl::new(dir.path().to_path_buf()).exit_soon(host.hold_jobs().unwrap());
        tokio::time::timeout(Duration::from_secs(5), exit_requested())
            .await
            .unwrap();
        // No backup job starts before the exit (final review C-M2).
        assert!(host.hold_jobs().is_err());
    }
}
