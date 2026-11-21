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
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bruhs_core::backup::{applying, instance_lock_path, restore_waits, take_instance_lock, Roots};
use bruhs_core::secrets::SecretStore;

/// How long a start waits for the quitting app and for the restore.
const START_LIMIT: Duration = Duration::from_secs(60);
const PAUSE: Duration = Duration::from_secs(1);
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
    /// A lock is held, an error left it waiting, or time ran out.
    Waiting,
}

/// How the app's start goes on.
enum Start {
    /// Run, keeping this lock (when it got one) for the app's life.
    Run(Option<File>),
    /// Stop without opening anything: a restore is half applied.
    Exit,
}

/// The instance lock at `path`, without waiting. `None` (logged) when
/// another app holds it; the app still starts.
fn hold_at(path: &Path) -> Option<File> {
    if let Some(folder) = path.parent() {
        if let Err(error) = std::fs::create_dir_all(folder) {
            tracing::warn!("couldn't make the app's data folder: {error}");
            return None;
        }
    }
    match take_instance_lock(path) {
        Ok(Some(lock)) => Some(lock),
        Ok(None) => {
            tracing::info!("another Bruhs app holds the instance lock");
            None
        }
        Err(error) => {
            tracing::warn!("couldn't take the instance lock: {error}");
            None
        }
    }
}

/// Takes the instance lock and applies a waiting restore with it (ruling
/// F21 a), before anything opens the app's data. Keep the answer for the
/// app's life. A restore that is half applied and can't finish now stops
/// the app here, since nothing may open its data (ruling F20).
pub fn apply_restore_at_start() -> Option<File> {
    let root = crate::paths::app_data_dir()?;
    let roots = Roots::from_env();
    let bundled_server = bruhs_core::updates::release::current_app_bundle()
        .map(|bundle| bundle.join("Contents/MacOS/bruhs-server"))
        .filter(|server| server.is_file());
    let apply = |lock: &File, wait: Duration| match (&roots, &bundled_server) {
        (Some(roots), Some(server)) => apply_bundled(roots, server, lock, wait),
        (Some(roots), None) => apply_here(roots, &SecretStore::os(), lock, wait),
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
    // A relaunch waits for the app that asked for the restore to quit.
    let lock = loop {
        match hold_at(path) {
            Some(lock) => break Some(lock),
            None if waits() && Instant::now() < deadline => std::thread::sleep(pause),
            None => break None,
        }
    };
    let (Some(lock), Some(roots)) = (lock, roots) else {
        return stop_if_half_applied(roots, None);
    };
    if !restore_waits(roots) {
        return Start::Run(Some(lock));
    }
    tracing::info!("applying the waiting restore");
    let deadline = Instant::now() + limit;
    loop {
        let wait = deadline.saturating_duration_since(Instant::now());
        match apply(&lock, wait) {
            Try::Done | Try::Failed => return Start::Run(Some(lock)),
            Try::Waiting if Instant::now() >= deadline => {
                return stop_if_half_applied(Some(roots), Some(lock));
            }
            Try::Waiting => std::thread::sleep(pause),
        }
    }
}

/// The app starts without applying, unless an apply is half done.
fn stop_if_half_applied(roots: Option<&Roots>, lock: Option<File>) -> Start {
    if roots.is_some_and(applying) {
        tracing::error!("a restore is half applied; Bruhs starts once it finishes");
        Start::Exit
    } else {
        if roots.is_some_and(restore_waits) {
            tracing::warn!("a restore waits; Bruhs starts without it");
        }
        Start::Run(lock)
    }
}

/// Applies the waiting restore in this process, waiting at most `wait`
/// for the keychain. A try still running then goes on in the background;
/// it holds `restore.lock`, so nothing else applies meanwhile.
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
        Err(_) => Try::Waiting,
    }
}

/// Applies the waiting restore through the bundle's `bruhs-server
/// restore-apply --once` (ruling F23), handing it the instance lock as
/// standard input. Waits at most `wait`; a server still applying then is
/// left to finish.
fn apply_bundled(roots: &Roots, server: &Path, lock: &File, wait: Duration) -> Try {
    let stdin = match lock.try_clone() {
        Ok(lock) => lock,
        Err(error) => {
            tracing::warn!("couldn't share the instance lock: {error}");
            return Try::Waiting;
        }
    };
    let log = restore_log(&roots.core);
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
            Ok(None) => return Try::Waiting,
            Err(error) => {
                tracing::warn!("lost the server applying the restore: {error}");
                return Try::Waiting;
            }
        }
    }
}

/// `<core>/logs/restore.log`, where the server's applier writes too. A
/// link there is left alone and the output dropped.
fn restore_log(core: &Path) -> Option<File> {
    use std::os::unix::fs::OpenOptionsExt;
    let logs: PathBuf = core.join("logs");
    std::fs::create_dir_all(&logs).ok()?;
    let path = logs.join("restore.log");
    if std::fs::symlink_metadata(&path).is_ok_and(|metadata| !metadata.is_file()) {
        return None;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .ok()
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
        // Nothing waits: the lock, at once.
        assert!(matches!(start(Duration::from_secs(5)), Start::Run(Some(_))));
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
        // Another app holds the lock and nothing applies: it starts
        // without the lock once the wait is over.
        let other = hold_at(&path).unwrap();
        assert!(matches!(
            start(Duration::from_millis(500)),
            Start::Run(None)
        ));
        drop(other);
        assert!(matches!(
            start(Duration::from_secs(20)),
            Start::Run(Some(_))
        ));
        assert_eq!(note(&fixture.roots).as_deref(), Some(RESTORED));
    }
}
