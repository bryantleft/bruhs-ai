//! The models a runtime's CLI lists for the signed-in account, cached briefly
//! so opening a picker does not start a process every time.

use std::ffi::OsStr;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use bruhs_models::runtime_models::{
    cursor_list_error, parse_acp_models, parse_antigravity_models, parse_cursor_models,
    RuntimeModels,
};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
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
    if !matches!(provider, AgentProvider::Cursor | AgentProvider::Antigravity) {
        return RuntimeModels::default();
    }
    if let Some(fresh) = cached(provider, Instant::now()) {
        return fresh;
    }
    let _listing = LISTING.lock().await;
    if let Some(fresh) = cached(provider, Instant::now()) {
        return fresh;
    }
    refresh_locked(provider).await
}

/// Refresh the signed-in account listing for an explicit status probe.
pub(super) async fn refresh_cursor() -> RuntimeModels {
    let _listing = LISTING.lock().await;
    refresh_locked(AgentProvider::Cursor).await
}

/// Refresh the signed-in Antigravity model listing for an explicit status probe.
pub(super) async fn refresh_antigravity() -> RuntimeModels {
    let _listing = LISTING.lock().await;
    refresh_locked(AgentProvider::Antigravity).await
}

async fn refresh_locked(provider: AgentProvider) -> RuntimeModels {
    let listed = match provider {
        AgentProvider::Cursor => match executable(provider) {
            Ok(program) => list_cursor(program.as_os_str(), LIST_TIMEOUT).await,
            Err(error) => RuntimeModels {
                models: Vec::new(),
                error: Some(error.to_string()),
            },
        },
        AgentProvider::Antigravity => match executable(provider) {
            Ok(program) => list_antigravity(program.as_os_str(), LIST_TIMEOUT).await,
            Err(error) => RuntimeModels {
                models: Vec::new(),
                error: Some(error.to_string()),
            },
        },
        _ => RuntimeModels::default(),
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

const ACP_LIST_BYTES: u64 = 512 * 1024;

/// Query `agy acp` (`initialize` + `session/new`) for its advertised Gemini and
/// third-party models, falling back to the signed-in account's
/// `fetchAvailableModels` endpoint when ACP does not return models.
async fn list_antigravity(program: &OsStr, timeout: Duration) -> RuntimeModels {
    let acp = list_antigravity_acp(program, timeout).await;
    if !acp.models.is_empty() {
        return acp;
    }
    if let Ok(Some(payload)) = crate::usage_hubs::antigravity_models().await {
        let models = parse_antigravity_models(&payload);
        if !models.is_empty() {
            return RuntimeModels {
                models,
                error: None,
            };
        }
    }
    acp
}

async fn list_antigravity_acp(program: &OsStr, timeout: Duration) -> RuntimeModels {
    let failed = |error: String| RuntimeModels {
        models: Vec::new(),
        error: Some(error),
    };
    let Ok(cwd) = tempfile::tempdir() else {
        return failed("Antigravity could not prepare a probe directory.".into());
    };
    let mut command = Command::new(program);
    command
        .arg("acp")
        .current_dir(cwd.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    crate::connections::protect_command(&mut command);
    if let Ok(vars) = crate::secret_sources::for_target("antigravity").await {
        for (key, value) in vars {
            command.env(key, value);
        }
    }
    #[cfg(unix)]
    command.process_group(0);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return failed("Antigravity CLI not found. Install it, then run `agy` to sign in.".into());
        }
        Err(error) => return failed(format!("Antigravity could not be started: {error}")),
    };
    #[cfg(unix)]
    let _group = super::process::ProcessGroup::of(&child);
    let Some(mut stdin) = child.stdin.take() else {
        return failed("Antigravity ACP stdin unavailable.".into());
    };
    let Some(stdout) = child.stdout.take() else {
        return failed("Antigravity ACP stdout unavailable.".into());
    };
    let cwd_str = cwd.path().to_string_lossy().to_string();
    let probe = async move {
        let init = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": 1,
                "clientInfo": {"name": "bruhs", "version": env!("CARGO_PKG_VERSION")},
                "clientCapabilities": {}
            }
        });
        stdin
            .write_all(format!("{init}\n").as_bytes())
            .await
            .map_err(|e| format!("Antigravity ACP write failed: {e}"))?;
        let mut lines = BufReader::new(stdout.take(ACP_LIST_BYTES)).lines();
        let mut initialized = false;
        while let Some(line) = lines
            .next_line()
            .await
            .map_err(|e| format!("Antigravity ACP read failed: {e}"))?
        {
            let Ok(frame) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if frame["id"] == 1 && frame.get("method").is_none() {
                if let Some(err) = frame.get("error") {
                    let msg = err["message"]
                        .as_str()
                        .unwrap_or("Antigravity ACP initialize failed.");
                    return Err(msg.to_string());
                }
                initialized = true;
                let new_session = json!({
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "session/new",
                    "params": {
                        "cwd": cwd_str,
                        "mcpServers": []
                    }
                });
                stdin
                    .write_all(format!("{new_session}\n").as_bytes())
                    .await
                    .map_err(|e| format!("Antigravity ACP session/new failed: {e}"))?;
                continue;
            }
            if initialized && frame["id"] == 2 && frame.get("method").is_none() {
                if let Some(err) = frame.get("error") {
                    let msg = err["message"]
                        .as_str()
                        .unwrap_or("Sign in with `agy` to list Antigravity models.");
                    return Err(msg.to_string());
                }
                return Ok(parse_acp_models(
                    &frame["result"],
                    AgentProvider::Antigravity,
                ));
            }
        }
        Err("Antigravity closed its ACP stream before listing models.".into())
    };
    match tokio::time::timeout(timeout, probe).await {
        Ok(Ok(models)) => RuntimeModels {
            models,
            error: None,
        },
        Ok(Err(error)) => failed(error),
        Err(_) => failed(format!(
            "Antigravity did not list models within {timeout:?}."
        )),
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
    #[tokio::test(start_paused = true)]
    async fn a_hung_listing_times_out_and_kills_what_it_started() {
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("child.pid");
        let program = fixture(
            &directory,
            &format!("sleep 30 &\necho $! > '{}'\nwait", pid_file.display()),
        );
        let timeout = Duration::from_millis(500);
        let mut listing = Box::pin(list_cursor(program.as_os_str(), timeout));
        // Start the real child without letting virtual time outrun OS startup.
        assert!(matches!(
            futures_util::poll!(&mut listing),
            std::task::Poll::Pending
        ));
        let started = Instant::now();
        let pid = loop {
            if let Ok(pid) = std::fs::read_to_string(&pid_file) {
                if pid.trim().parse::<u32>().is_ok_and(|pid| pid > 0) {
                    break pid;
                }
            }
            assert!(
                started.elapsed() < LIST_TIMEOUT,
                "The owned listing fixture did not start; cannot verify child cleanup."
            );
            std::thread::sleep(Duration::from_millis(50));
        };
        tokio::time::advance(timeout).await;
        let listed = listing.await;
        assert!(listed.error.unwrap().contains("did not list models"));
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

    #[tokio::test]
    async fn lists_antigravity_gemini_models_from_acp_session() {
        let directory = tempfile::tempdir().unwrap();
        let program = fixture(
            &directory,
            r#"[ "$1" = "acp" ] || exit 9
read -r init || exit 1
printf '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}\n'
read -r sess || exit 1
printf '{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1","configOptions":[{"id":"model","type":"select","options":[{"value":"gemini-4-neon","name":"Gemini 4 Neon"},{"value":"gemini-4-argon","name":"Gemini 4 Argon"}]}]}}\n'
"#,
        );
        let listed = list_antigravity_acp(program.as_os_str(), LIST_TIMEOUT).await;
        assert_eq!(listed.error, None);
        let ids: Vec<&str> = listed.models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["antigravity/gemini-4-neon", "antigravity/gemini-4-argon"]);
    }
}
