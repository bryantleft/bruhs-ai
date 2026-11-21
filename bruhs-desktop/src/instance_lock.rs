//! The desktop app's instance lock (D17, ruling F21) and the restore its
//! start applies (D16, rulings F20 and F23).
//!
//! The app holds `desktop.instance.lock` at the top of the desktop root for
//! its whole life, so a restore never replaces data under it. A relaunch
//! that a restore asked for waits up to a minute for the quitting app to
//! let go, then applies the waiting restore with the lock in hand: a
//! packaged app through its own `bruhs-server restore-apply --once` (whose
//! keychain items the server and the background service can then read
//! without asking), a source build in this process. Only outcomes are
//! logged, never a path or a name from inside a backup.

use std::fs::File;
use std::path::Path;
use std::time::{Duration, Instant};

use bruhs_core::backup::{applying, instance_lock_path, restore_waits, take_instance_lock, Roots};
use bruhs_core::secrets::{SecretChoice, SecretStore};

/// How long a start waits for the quitting app and for the restore.
const START_LIMIT: Duration = Duration::from_secs(60);
const PAUSE: Duration = Duration::from_secs(1);
/// The least time a try gets, even at the deadline: a try that only meets
/// a held lock answers in well under a second, so only one held up (by a
/// keychain prompt) runs past it.
const TRY_FLOOR: Duration = Duration::from_secs(5);
/// How often the wait for the quitting app logs it.
const WAITING_LOG_EVERY: Duration = Duration::from_secs(10);
/// What a packaged app's server answers from `restore-apply --once`.
const EXIT_DONE: i32 = 0;
const EXIT_FAILED: i32 = 1;
/// How a start ends when a restore is half applied and nothing opens.
const EXIT_WAITING: i32 = 75;

/// What one try at the waiting restore came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Try {
    /// Nothing waits any more.
    Done,
    /// Refused for good; nothing waits any more.
    Failed,
    /// A lock is held, an error left it waiting, or time ran out and the
    /// try was stopped.
    Waiting,
    /// Time ran out and the try still runs: it can't be stopped, so the
    /// app must not start (B Task 10 I1).
    Running,
}

/// How the app's start goes on.
enum Start {
    /// Run, keeping this lock (when it got one) for the app's life.
    Run(Option<File>),
    /// Stop without opening anything: a restore is half applied or still
    /// being applied, or it waits while another app holds the lock.
    Exit,
}

/// The instance lock at `path`, without waiting. `None` when another app
/// holds it, or (logged) when it can't be taken.
fn hold_at(path: &Path) -> Option<File> {
    if let Some(folder) = path.parent() {
        if let Err(error) = std::fs::create_dir_all(folder) {
            tracing::warn!("couldn't make the app's data folder: {error}");
            return None;
        }
    }
    match take_instance_lock(path) {
        Ok(Some(lock)) => Some(lock),
        Ok(None) => None,
        Err(error) => {
            tracing::warn!("couldn't take the instance lock: {error}");
            None
        }
    }
}

/// Takes the instance lock and applies a waiting restore with it (ruling
/// F21 a), before anything opens the app's data. Keep the answer for the
/// app's life. The app stops here, opening nothing (ruling F20), when a
/// restore is half applied, when a try at it still runs at the deadline,
/// or when one waits while another app holds the lock. With nothing
/// waiting, a second app runs without the lock (D17).
pub fn apply_restore_at_start() -> Option<File> {
    let root = crate::paths::app_data_dir()?;
    let roots = Roots::from_env();
    let bundled_server = bruhs_core::updates::release::current_app_bundle()
        .map(|bundle| bundle.join("Contents/MacOS/bruhs-server"))
        .filter(|server| server.is_file());
    let apply = |lock: &File, wait: Duration| match (&roots, &bundled_server) {
        (Some(roots), Some(server)) => apply_bundled(roots, server, lock, wait),
        (Some(roots), None) => apply_here(roots, &SecretChoice::from_env().store(), lock, wait),
        (None, _) => Try::Done,
    };
    match start_at(
        &instance_lock_path(&root),
        roots.as_ref(),
        &apply,
        START_LIMIT,
        PAUSE,
    ) {
        Start::Run(lock) => lock,
        Start::Exit => std::process::exit(EXIT_WAITING),
    }
}

fn start_at(
    path: &Path,
    roots: Option<&Roots>,
    apply: &dyn Fn(&File, Duration) -> Try,
    limit: Duration,
    pause: Duration,
) -> Start {
    let waits = || roots.is_some_and(restore_waits);
    let deadline = Instant::now() + limit;
    let mut logged: Option<Instant> = None;
    // A relaunch waits for the app that asked for the restore to quit.
    let lock = loop {
        if let Some(lock) = hold_at(path) {
            break Some(lock);
        }
        if logged.is_none_or(|last| last.elapsed() >= WAITING_LOG_EVERY) {
            logged = Some(Instant::now());
            tracing::info!("another Bruhs app holds the instance lock");
        }
        if waits() && Instant::now() < deadline {
            std::thread::sleep(pause);
        } else {
            break None;
        }
    };
    let Some(roots) = roots else {
        return Start::Run(lock);
    };
    let Some(lock) = lock else {
        if restore_waits(roots) {
            tracing::error!(
                "a restore waits and another Bruhs app holds the instance lock; Bruhs stops"
            );
            return Start::Exit;
        }
        return Start::Run(None);
    };
    if !restore_waits(roots) {
        return Start::Run(Some(lock));
    }
    tracing::info!("applying the waiting restore");
    let deadline = Instant::now() + limit;
    loop {
        let wait = deadline
            .saturating_duration_since(Instant::now())
            .max(TRY_FLOOR);
        match apply(&lock, wait) {
            Try::Done | Try::Failed => return Start::Run(Some(lock)),
            Try::Running => {
                tracing::error!("a restore is still being applied; Bruhs starts once it finishes");
                return Start::Exit;
            }
            Try::Waiting if Instant::now() >= deadline => {
                return stop_if_half_applied(roots, lock);
            }
            Try::Waiting => std::thread::sleep(pause),
        }
    }
}

/// The app starts without applying, unless an apply is half done.
fn stop_if_half_applied(roots: &Roots, lock: File) -> Start {
    if applying(roots) {
        tracing::error!("a restore is half applied; Bruhs starts once it finishes");
        Start::Exit
    } else {
        tracing::warn!("a restore waits; Bruhs starts without it");
        Start::Run(Some(lock))
    }
}

/// Applies the waiting restore in this process, waiting at most `wait`
/// for the keychain. A try still running then can't be stopped, so it
/// answers [`Try::Running`] and the app doesn't start.
fn apply_here(roots: &Roots, store: &SecretStore, lock: &File, wait: Duration) -> Try {
    let lock = match lock.try_clone() {
        Ok(lock) => lock,
        Err(error) => {
            tracing::warn!("couldn't share the instance lock: {error}");
            return Try::Waiting;
        }
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    let (roots, store) = (roots.clone(), store.clone());
    let spawned = std::thread::Builder::new()
        .name("restore-at-start".into())
        .spawn(move || {
            let outcome = bruhs_core::backup::apply_pending(&roots, &store, Some(&lock));
            let waits = outcome.is_err() && restore_waits(&roots);
            // The receiver may be gone after a timeout.
            let _ = sender.send((outcome, waits));
        });
    if let Err(error) = spawned {
        tracing::warn!("no thread for the restore: {error}");
        return Try::Waiting;
    }
    use bruhs_core::backup::ApplyOutcome;
    match receiver.recv_timeout(wait) {
        Ok((Ok(ApplyOutcome::NothingPending), _)) => Try::Done,
        Ok((Ok(ApplyOutcome::Applied(result)), _)) => {
            tracing::info!(warnings = result.warnings.len(), "the restore is in place");
            Try::Done
        }
        Ok((Ok(ApplyOutcome::Busy(_)), _)) => Try::Waiting,
        Ok((Err(error), waits)) => {
            tracing::warn!("restore not applied: {}", error.message());
            if waits {
                Try::Waiting
            } else {
                Try::Failed
            }
        }
        Err(_) => Try::Running,
    }
}

/// Applies the waiting restore through the bundle's `bruhs-server
/// restore-apply --once` (ruling F23), handing it the instance lock as
/// standard input. Waits at most `wait`; a server still trying then is
/// stopped and waited for, so no try outlives the start (B Task 10 I1).
fn apply_bundled(roots: &Roots, server: &Path, lock: &File, wait: Duration) -> Try {
    let stdin = match lock.try_clone() {
        Ok(lock) => lock,
        Err(error) => {
            tracing::warn!("couldn't share the instance lock: {error}");
            return Try::Waiting;
        }
    };
    let log = bruhs_core::backup::open_restore_log(&roots.core).ok();
    let output = |log: &Option<File>| {
        log.as_ref()
            .and_then(|log| log.try_clone().ok())
            .map_or_else(std::process::Stdio::null, std::process::Stdio::from)
    };
    let child = std::process::Command::new(server)
        .args(["restore-apply", "--once"])
        .env_remove("BRUHS_EXIT_WITH_PARENT")
        .stdin(stdin)
        .stdout(output(&log))
        .stderr(output(&log))
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => {
            tracing::warn!("couldn't start the server to apply the restore: {error}");
            return Try::Waiting;
        }
    };
    let deadline = Instant::now() + wait;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return match status.code() {
                    Some(EXIT_DONE) => Try::Done,
                    Some(EXIT_FAILED) => Try::Failed,
                    _ => Try::Waiting,
                }
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                tracing::warn!("the restore took too long; stopping its try");
                if let Err(error) = child.kill() {
                    tracing::warn!("couldn't stop the restore's try: {error}");
                }
                return match child.wait() {
                    Ok(_) => Try::Waiting,
                    Err(_) => Try::Running,
                };
            }
            Err(error) => {
                tracing::warn!("lost the server applying the restore: {error}");
                let _ = child.kill();
                return match child.wait() {
                    Ok(_) => Try::Waiting,
                    Err(_) => Try::Running,
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_core::backup::test_support::{note, stage_restore, RESTORED, TODAY};
    use bruhs_core::backup::{apply_pending, ApplyOutcome, Holder};

    struct Fixture {
        _dir: tempfile::TempDir,
        roots: Roots,
        store: SecretStore,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(dir.path()).unwrap();
        let home = base.join("home");
        let roots = Roots {
            home: home.clone(),
            core: home.join("core"),
            desktop: home.join("desktop"),
            config: home.join("core"),
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

    fn in_process<'a>(fixture: &'a Fixture) -> impl Fn(&File, Duration) -> Try + 'a {
        move |lock, wait| apply_here(&fixture.roots, &fixture.store, lock, wait)
    }

    #[test]
    fn the_instance_lock_is_held_for_the_apps_life() {
        let fixture = fixture();
        let path = instance_lock_path(&fixture.roots.desktop);
        let first = hold_at(&path).unwrap();
        assert!(hold_at(&path).is_none());
        stage_restore(&fixture.roots, &fixture.store, false).unwrap();
        assert_eq!(
            apply_pending(&fixture.roots, &fixture.store, None).unwrap(),
            ApplyOutcome::Busy(Holder::Desktop)
        );
        assert_eq!(note(&fixture.roots).as_deref(), Some(TODAY));
        drop(first);
        assert!(matches!(
            apply_pending(&fixture.roots, &fixture.store, None).unwrap(),
            ApplyOutcome::Applied(_)
        ));
        assert!(hold_at(&path).is_some());
    }

    #[test]
    fn a_relaunch_applies_once_the_quitting_app_lets_go() {
        let fixture = fixture();
        let path = instance_lock_path(&fixture.roots.desktop);
        stage_restore(&fixture.roots, &fixture.store, false).unwrap();
        // The app that asked for the restore is still quitting.
        let quitting = hold_at(&path).unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(1));
            drop(quitting);
        });
        let apply = in_process(&fixture);
        let start = start_at(
            &path,
            Some(&fixture.roots),
            &apply,
            Duration::from_secs(20),
            Duration::from_millis(100),
        );
        release.join().unwrap();
        let Start::Run(Some(lock)) = start else {
            panic!("the relaunch started without its lock");
        };
        assert_eq!(note(&fixture.roots).as_deref(), Some(RESTORED));
        // The lock stays held while the app runs.
        assert!(hold_at(&path).is_none());
        drop(lock);
        assert!(hold_at(&path).is_some());
    }

    #[test]
    fn a_start_waits_for_a_restore_then_goes_on_or_stops() {
        let fixture = fixture();
        let path = instance_lock_path(&fixture.roots.desktop);
        let apply = in_process(&fixture);
        let start = |limit| {
            start_at(
                &path,
                Some(&fixture.roots),
                &apply,
                limit,
                Duration::from_millis(100),
            )
        };
        // Nothing waits: the lock, at once; and a second app beside it runs
        // without one (D17).
        let first = start(Duration::from_secs(5));
        assert!(matches!(first, Start::Run(Some(_))));
        assert!(matches!(start(Duration::from_secs(5)), Start::Run(None)));
        drop(first);
        // A server still holds its database: the app starts without
        // applying, and the restore waits (ruling F20 b).
        stage_restore(&fixture.roots, &fixture.store, false).unwrap();
        let server = std::fs::create_dir_all(fixture.roots.core.join("server"))
            .and_then(|()| File::create(fixture.roots.core.join("server/reactive.sqlite3.lock")))
            .unwrap();
        server.lock().unwrap();
        assert!(matches!(
            start(Duration::from_millis(500)),
            Start::Run(Some(_))
        ));
        assert_eq!(note(&fixture.roots).as_deref(), Some(TODAY));
        drop(server);
        // Another app holds the lock while the restore waits: this one
        // stops once the wait is over, rather than run where a later apply
        // could replace its data (B Task 10 I2).
        let other = hold_at(&path).unwrap();
        assert!(matches!(start(Duration::from_millis(500)), Start::Exit));
        assert_eq!(note(&fixture.roots).as_deref(), Some(TODAY));
        drop(other);
        assert!(matches!(
            start(Duration::from_secs(20)),
            Start::Run(Some(_))
        ));
        assert_eq!(note(&fixture.roots).as_deref(), Some(RESTORED));
    }

    /// No try at the restore outlives a start that gives up on it (B Task
    /// 10 I1).
    #[test]
    fn no_try_outlives_the_start() {
        let fixture = fixture();
        let path = instance_lock_path(&fixture.roots.desktop);
        stage_restore(&fixture.roots, &fixture.store, false).unwrap();
        // A try that still runs stops the start.
        let still_running = |_: &File, _: Duration| Try::Running;
        assert!(matches!(
            start_at(
                &path,
                Some(&fixture.roots),
                &still_running,
                Duration::from_secs(5),
                Duration::from_millis(100),
            ),
            Start::Exit
        ));
        // In this process a try can't be stopped: out of time, it answers
        // Running, and it goes on to finish.
        let lock = hold_at(&path).unwrap();
        assert_eq!(
            apply_here(&fixture.roots, &fixture.store, &lock, Duration::ZERO),
            Try::Running
        );
        let begun = Instant::now();
        while restore_waits(&fixture.roots) && begun.elapsed() < Duration::from_secs(20) {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(note(&fixture.roots).as_deref(), Some(RESTORED));
        // A bundled server's try is stopped and waited for.
        let dir = tempfile::tempdir().unwrap();
        let server = |name: &str, body: &str| {
            let path = dir.path().join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755))
                .unwrap();
            path
        };
        let pid_file = dir.path().join("pid");
        let stuck = server(
            "stuck.sh",
            &format!("echo $$ > '{}'\nexec sleep 30", pid_file.display()),
        );
        let begun = Instant::now();
        assert_eq!(
            apply_bundled(&fixture.roots, &stuck, &lock, Duration::from_secs(3)),
            Try::Waiting
        );
        assert!(begun.elapsed() < Duration::from_secs(15));
        // Reaped either way; when it got as far as `sleep`, that is gone.
        if let Ok(pid) = std::fs::read_to_string(&pid_file) {
            let alive = std::process::Command::new("kill")
                .args(["-0", pid.trim()])
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap()
                .success();
            assert!(!alive, "the stuck try still runs");
        }
        // Its answers map as `--once` gives them.
        for (code, answer) in [(0, Try::Done), (1, Try::Failed), (75, Try::Waiting)] {
            let quick = server(&format!("exit-{code}.sh"), &format!("exit {code}"));
            assert_eq!(
                apply_bundled(&fixture.roots, &quick, &lock, Duration::from_secs(10)),
                answer
            );
        }
    }
}
