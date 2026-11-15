//! The models a runtime's CLI lists for the signed-in account, cached briefly
//! so opening a picker does not start a process every time.

use std::ffi::OsStr;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use bruhs_models::runtime_models::{cursor_list_error, parse_cursor_models, RuntimeModels};
use tokio::process::Command;

use super::{executable, AgentProvider};

/// `--list-models` asks Cursor's backend; a run that outlives this is stuck.
const LIST_TIMEOUT: Duration = Duration::from_secs(15);
/// A listing stays fresh this long; model plans change rarely.
const FRESH_FOR: Duration = Duration::from_secs(10 * 60);
/// A failure is retried sooner, so signing in or updating shows up quickly.
const RETRY_AFTER: Duration = Duration::from_secs(30);

/// Each runtime's last answer and when it came, kept past expiry so a failed
/// refresh can still offer the models listed before.
static CACHE: Mutex<Vec<(AgentProvider, Instant, RuntimeModels)>> = Mutex::new(Vec::new());
/// One listing runs at a time; callers that wait for it reuse its answer.
static LISTING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The account's models for `provider`. Runtimes that cannot list models
/// return an empty list without an error. A failed refresh keeps the models
/// listed before and adds the error.
pub(crate) async fn list(provider: AgentProvider) -> RuntimeModels {
    if provider != AgentProvider::Cursor {
        return RuntimeModels::default();
    }
    if let Some(fresh) = cached(provider, Instant::now()) {
        return fresh;
    }
    let _listing = LISTING.lock().await;
    if let Some(fresh) = cached(provider, Instant::now()) {
        return fresh;
    }
    let listed = match executable(provider) {
        Ok(program) => list_cursor(program.as_os_str(), LIST_TIMEOUT).await,
        Err(error) => RuntimeModels {
            models: Vec::new(),
            error: Some(error.to_string()),
        },
    };
    store(provider, listed, Instant::now())
}

fn lock_cache() -> std::sync::MutexGuard<'static, Vec<(AgentProvider, Instant, RuntimeModels)>> {
    CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// `provider`'s cached answer if it is still fresh at `now`.
fn cached(provider: AgentProvider, now: Instant) -> Option<RuntimeModels> {
    lock_cache()
        .iter()
        .find(|(cached, _, _)| *cached == provider)
        .filter(|(_, at, models)| {
            let ttl = if models.error.is_some() {
                RETRY_AFTER
            } else {
                FRESH_FOR
            };
            now.saturating_duration_since(*at) < ttl
        })
        .map(|(_, _, models)| models.clone())
}

/// Cache `listed` and return what callers should see: on a failure with no
/// models, the models from the previous answer, with the new error.
fn store(provider: AgentProvider, mut listed: RuntimeModels, now: Instant) -> RuntimeModels {
    let mut cache = lock_cache();
    if listed.error.is_some() && listed.models.is_empty() {
        if let Some((_, _, previous)) = cache.iter().find(|(cached, _, _)| *cached == provider) {
            listed.models.clone_from(&previous.models);
        }
    }
    cache.retain(|(cached, _, _)| *cached != provider);
    cache.push((provider, now, listed.clone()));
    listed
}

/// `program --list-models` with stdin closed, in its own process group, and
/// without bruhs's private connection secrets. `models` would be taken as a
/// prompt by CLIs older than the subcommand; the flag fails cleanly with
/// "unknown option".
fn list_command(program: &OsStr) -> Command {
    let mut command = Command::new(program);
    command
        .arg("--list-models")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    crate::connections::protect_command(&mut command);
    #[cfg(unix)]
    command.process_group(0);
    command
}

/// Run the listing, killing the CLI and anything it started if it outlives
/// `timeout` or the caller stops waiting.
async fn list_cursor(program: &OsStr, timeout: Duration) -> RuntimeModels {
    let failed = |error: String| RuntimeModels {
        models: Vec::new(),
        error: Some(error),
    };
    let child = match list_command(program).spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return failed(
                "Cursor CLI not found. Install it, then run `cursor-agent login`.".into(),
            );
        }
        Err(error) => return failed(format!("Cursor could not be started: {error}")),
    };
    #[cfg(unix)]
    let mut group = super::process::ProcessGroup::of(&child);
    let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => return failed(format!("Cursor could not be read: {error}")),
        Err(_) => return failed(format!("Cursor did not list models within {timeout:?}.")),
    };
    #[cfg(unix)]
    group.release();
    if !output.status.success() {
        return failed(cursor_list_error(&String::from_utf8_lossy(&output.stderr)));
    }
    match parse_cursor_models(&String::from_utf8_lossy(&output.stdout)) {
        Some(models) => RuntimeModels {
            models,
            error: None,
        },
        None => failed(
            "Cursor printed a model list bruhs does not recognize. Check `cursor-agent --list-models`."
                .into(),
        ),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn fixture(directory: &tempfile::TempDir, body: &str) -> std::path::PathBuf {
        let path = directory.path().join("cursor-agent");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write the fixture CLI");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("make the fixture CLI executable");
        path
    }

    #[tokio::test]
    async fn lists_models_from_a_successful_run() {
        let directory = tempfile::tempdir().unwrap();
        let program = fixture(
            &directory,
            r#"[ "$1" = "--list-models" ] || exit 9
printf 'Available models\n\nauto - Auto (current, default)\ngpt-5 - GPT-5\n\nTip: use --model <id>\n'"#,
        );
        let listed = list_cursor(program.as_os_str(), LIST_TIMEOUT).await;
        assert_eq!(listed.error, None);
        let ids: Vec<&str> = listed
            .models
            .iter()
            .map(|model| model.id.as_str())
            .collect();
        assert_eq!(ids, ["cursor/auto", "cursor/gpt-5"]);
    }

    #[tokio::test]
    async fn failures_report_the_cli_reason() {
        let directory = tempfile::tempdir().unwrap();
        let program = fixture(
            &directory,
            "echo \"error: unknown option '--list-models'\" >&2; exit 1",
        );
        let listed = list_cursor(program.as_os_str(), LIST_TIMEOUT).await;
        assert!(listed.models.is_empty());
        assert_eq!(
            listed.error.as_deref(),
            Some("This Cursor CLI cannot list models. Run `cursor-agent update`.")
        );

        let missing =
            list_cursor(OsStr::new("/nonexistent/bruhs-cursor-agent"), LIST_TIMEOUT).await;
        assert!(missing.error.unwrap().starts_with("Cursor CLI not found"));
    }

    #[tokio::test]
    async fn unrecognized_output_is_an_error_not_an_empty_list() {
        let directory = tempfile::tempdir().unwrap();
        let program = fixture(&directory, "echo 'Something new'");
        let listed = list_cursor(program.as_os_str(), LIST_TIMEOUT).await;
        assert!(listed.models.is_empty());
        assert!(listed.error.unwrap().contains("does not recognize"));
        let empty = fixture(&directory, "echo 'No models available for this account.'");
        assert_eq!(
            list_cursor(empty.as_os_str(), LIST_TIMEOUT).await,
            RuntimeModels::default()
        );
    }

    #[test]
    fn the_listing_never_receives_private_connection_secrets() {
        let command = list_command(OsStr::new("cursor-agent"));
        let removed: Vec<_> = command
            .as_std()
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(key, _)| key.to_os_string())
            .collect();
        for key in crate::connections::PRIVATE_ENV {
            assert!(removed.iter().any(|removed| removed == key), "{key}");
        }
    }

    /// A wrapper script that does not `exec` leaves its child running; the
    /// timeout kills the whole process group.
    #[tokio::test]
    async fn a_hung_listing_times_out_and_kills_what_it_started() {
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("child.pid");
        let program = fixture(
            &directory,
            &format!("sleep 30 &\necho $! > '{}'\nwait", pid_file.display()),
        );
        let listed = list_cursor(program.as_os_str(), Duration::from_millis(500)).await;
        assert!(listed.error.unwrap().contains("did not list models"));
        let pid = std::fs::read_to_string(&pid_file).unwrap();
        let alive = || {
            std::process::Command::new("kill")
                .args(["-0", pid.trim()])
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        };
        for _ in 0..50 {
            if !alive() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("the listing's child process outlived the timeout");
    }

    #[test]
    fn the_cache_expires_failures_sooner_and_keeps_models_through_one() {
        let provider = AgentProvider::Cursor;
        lock_cache().retain(|(cached, _, _)| *cached != provider);
        let start = Instant::now();
        let listed = RuntimeModels {
            models: parse_cursor_models("Available models\ngpt-5 - GPT-5\n").unwrap(),
            error: None,
        };
        assert_eq!(store(provider, listed.clone(), start), listed);
        assert_eq!(cached(provider, start + RETRY_AFTER), Some(listed.clone()));
        assert_eq!(cached(provider, start + FRESH_FOR), None);

        let failed = RuntimeModels {
            models: Vec::new(),
            error: Some("timed out".into()),
        };
        let later = start + FRESH_FOR;
        let kept = store(provider, failed, later);
        assert_eq!(kept.models, listed.models);
        assert_eq!(kept.error.as_deref(), Some("timed out"));
        assert_eq!(cached(provider, later), Some(kept));
        assert_eq!(cached(provider, later + RETRY_AFTER), None);
        lock_cache().retain(|(cached, _, _)| *cached != provider);
    }
}
