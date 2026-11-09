//! Argument and stream contract with the `claude` CLI, checked against an
//! owned fixture script on a temporary `PATH` (never the installed CLI).
#![cfg(unix)]
use claude_sdk_rs::{query, PermissionMode, QueryEvent, QueryOptions, Tools};
use std::{os::unix::fs::PermissionsExt, path::Path, time::Duration};
use tokio_stream::StreamExt;

const SUCCESS: &str = r#"{"type":"result","subtype":"success","is_error":false,"result":"ok"}"#;

fn fixture(directory: &Path, body: &str) {
    let path = directory.join("claude");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write owned fixture CLI");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .expect("fixture executable");
}

/// A fixture that records its argv (one per line) and prints `line`.
fn recording_fixture(directory: &Path, line: &str) {
    fixture(
        directory,
        &format!("printf '%s\\n' \"$@\" > arguments\nprintf '%s\\n' '{line}'"),
    );
}

fn options(directory: &Path, prompt: &str) -> QueryOptions {
    QueryOptions::new(prompt)
        .cwd(directory)
        .env("PATH", directory.display().to_string())
}

async fn collect(mut stream: claude_sdk_rs::QueryStream) -> Vec<QueryEvent> {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            events.push(event);
        }
        events
    })
    .await
    .expect("stream must end once the fixture CLI exits")
}

fn arguments(directory: &Path) -> Vec<String> {
    std::fs::read_to_string(directory.join("arguments"))
        .expect("fixture recorded its arguments")
        .lines()
        .map(String::from)
        .collect()
}

#[tokio::test]
async fn flag_like_prompts_are_passed_after_end_of_options() {
    for prompt in ["--help", "-", "--dangerously-skip-permissions"] {
        let directory = tempfile::tempdir().unwrap();
        recording_fixture(directory.path(), SUCCESS);
        collect(query(options(directory.path(), prompt)).unwrap()).await;
        let args = arguments(directory.path());
        assert_eq!(
            &args[args.len() - 2..],
            ["--", prompt],
            "prompt {prompt:?} must be the single operand after `--`: {args:?}"
        );
        assert_eq!(
            args.iter().filter(|a| a.as_str() == prompt).count(),
            1,
            "{args:?}"
        );
    }
}

#[tokio::test]
async fn allowed_tools_are_one_argument_so_the_variadic_flag_cannot_swallow_the_prompt() {
    let directory = tempfile::tempdir().unwrap();
    recording_fixture(directory.path(), SUCCESS);
    let opts = options(directory.path(), "hello")
        .tools(Tools::Allow(vec!["Read".into(), "Bash(git log:*)".into()]));
    collect(query(opts).unwrap()).await;
    let args = arguments(directory.path());
    assert!(
        args.contains(&"--allowedTools=Read,Bash(git log:*)".to_string()),
        "{args:?}"
    );
    assert!(!args.contains(&"--allowedTools".to_string()), "{args:?}");
    assert_eq!(&args[args.len() - 2..], ["--", "hello"]);
}

#[tokio::test]
async fn flag_like_session_ids_are_rejected_before_spawning() {
    let directory = tempfile::tempdir().unwrap();
    recording_fixture(directory.path(), SUCCESS);
    let opts = options(directory.path(), "hi").resume("--dangerously-skip-permissions");
    let error = query(opts).expect_err("flag-like session id must be rejected");
    assert!(
        matches!(error, claude_sdk_rs::Error::InvalidSessionId(ref id) if id == "--dangerously-skip-permissions"),
        "{error:?}"
    );
    assert!(
        !directory.path().join("arguments").exists(),
        "CLI must not run"
    );
}

#[tokio::test]
async fn error_result_subtypes_end_an_interactive_turn_by_closing_stdin() {
    for subtype in ["error_max_turns", "error_during_execution"] {
        let directory = tempfile::tempdir().unwrap();
        // Streaming-input mode: the CLI keeps reading stdin after the result
        // and only exits once the SDK closes it.
        fixture(
            directory.path(),
            &format!(
                "read first_message\nprintf '%s\\n' '{{\"type\":\"result\",\"subtype\":\"{subtype}\",\"is_error\":true,\"session_id\":\"s-1\",\"errors\":[\"stopped: {subtype}\"]}}'\n/bin/cat > /dev/null"
            ),
        );
        let opts = options(directory.path(), "hi").permission_prompts(true);
        let events = collect(query(opts).unwrap()).await;
        let result = events
            .iter()
            .find(|e| matches!(e, QueryEvent::Result { .. }))
            .unwrap_or_else(|| panic!("{subtype} must be a Result event: {events:?}"));
        match result {
            QueryEvent::Result {
                is_error,
                session_id,
                errors,
                ..
            } => {
                assert!(*is_error, "{subtype}");
                assert_eq!(session_id, "s-1");
                assert_eq!(errors, &[format!("stopped: {subtype}")]);
            }
            _ => unreachable!(),
        }
    }
}

fn process_failure(events: &[QueryEvent]) -> &claude_sdk_rs::ProcessFailure {
    match events.last() {
        Some(QueryEvent::ProcessFailed(failure)) => failure,
        other => panic!("expected a final ProcessFailed event, got {other:?} in {events:?}"),
    }
}

#[tokio::test]
async fn unsuccessful_exit_surfaces_exit_code_and_stderr_tail() {
    let directory = tempfile::tempdir().unwrap();
    fixture(
        directory.path(),
        "echo 'warming up' >&2\necho 'fatal: OAuth token expired' >&2\nexit 3",
    );
    let events = collect(query(options(directory.path(), "hi")).unwrap()).await;
    let failure = process_failure(&events);
    assert_eq!(failure.exit_code, Some(3));
    assert_eq!(
        failure.stderr_tail,
        "warming up\nfatal: OAuth token expired"
    );
    assert!(failure.message.contains("claude exited unsuccessfully"));
}

#[tokio::test]
async fn a_stderr_flood_cannot_block_the_cli_and_only_its_tail_is_kept() {
    let directory = tempfile::tempdir().unwrap();
    // 1 MiB of stderr is far beyond a pipe buffer: an undrained pipe would
    // block the fixture before it ever prints its result.
    fixture(
        directory.path(),
        &format!(
            "/usr/bin/head -c 1048576 /dev/zero | /usr/bin/tr '\\0' x >&2\necho END >&2\nprintf '%s\\n' '{SUCCESS}'\nexit 1"
        ),
    );
    let events = collect(query(options(directory.path(), "hi")).unwrap()).await;
    assert!(
        matches!(
            events.first(),
            Some(QueryEvent::Result {
                is_error: false,
                ..
            })
        ),
        "{:?}",
        events.first()
    );
    let failure = process_failure(&events);
    assert_eq!(failure.exit_code, Some(1));
    assert!(failure.stderr_tail.len() <= 8 * 1024);
    assert!(failure.stderr_tail.ends_with("xxxEND"));
}

#[tokio::test]
async fn a_clean_exit_reports_no_failure() {
    let directory = tempfile::tempdir().unwrap();
    fixture(
        directory.path(),
        &format!("echo 'noise' >&2\nprintf '%s\\n' '{SUCCESS}'"),
    );
    let events = collect(query(options(directory.path(), "hi")).unwrap()).await;
    assert_eq!(events.len(), 1, "{events:?}");
    assert!(matches!(events[0], QueryEvent::Result { .. }));
}

#[tokio::test]
async fn a_missing_cli_is_reported_as_not_found() {
    let directory = tempfile::tempdir().unwrap();
    let error = query(options(directory.path(), "hi")).expect_err("no claude on PATH");
    assert!(
        matches!(error, claude_sdk_rs::Error::CliNotFound),
        "{error:?}"
    );
}

#[tokio::test]
async fn typed_options_map_to_single_cli_arguments() {
    let cases = [
        (PermissionMode::Default, "--permission-mode=default"),
        (PermissionMode::AcceptEdits, "--permission-mode=acceptEdits"),
        (PermissionMode::Plan, "--permission-mode=plan"),
        (
            PermissionMode::BypassPermissions,
            "--dangerously-skip-permissions",
        ),
    ];
    for (mode, expected) in cases {
        let directory = tempfile::tempdir().unwrap();
        recording_fixture(directory.path(), SUCCESS);
        let opts = options(directory.path(), "hi")
            .permission_mode(mode)
            .max_turns(2)
            .model("-odd-model")
            .resume("session-1")
            .system_prompt("-be terse");
        collect(query(opts).unwrap()).await;
        let args = arguments(directory.path());
        for arg in [
            expected,
            "--max-turns=2",
            "--model=-odd-model",
            "--resume=session-1",
            "--system-prompt=-be terse",
        ] {
            assert!(args.iter().any(|a| a == arg), "missing {arg}: {args:?}");
        }
        assert_eq!(&args[args.len() - 2..], ["--", "hi"]);
    }
}

/// A fixture that records whether `HOME` and `CARGO_PKG_NAME` (set by cargo
/// for this test process) reached it.
fn env_fixture(directory: &Path) {
    fixture(
        directory,
        &format!(
            "printf '%s\\n' \"${{HOME-unset}}\" \"${{CARGO_PKG_NAME-unset}}\" > environment\nprintf '%s\\n' '{SUCCESS}'"
        ),
    );
}

fn environment(directory: &Path) -> Vec<String> {
    std::fs::read_to_string(directory.join("environment"))
        .expect("fixture recorded its environment")
        .lines()
        .map(String::from)
        .collect()
}

#[tokio::test]
async fn env_remove_drops_one_inherited_variable() {
    let directory = tempfile::tempdir().unwrap();
    env_fixture(directory.path());
    collect(query(options(directory.path(), "hi").env_remove("HOME")).unwrap()).await;
    assert_eq!(environment(directory.path()), ["unset", "claude-sdk-rs"]);
}

#[tokio::test]
async fn without_inheritance_only_explicit_variables_reach_the_cli() {
    let directory = tempfile::tempdir().unwrap();
    env_fixture(directory.path());
    let opts = options(directory.path(), "hi")
        .inherit_env(false)
        .env("HOME", "/fixture-home");
    collect(query(opts).unwrap()).await;
    assert_eq!(environment(directory.path()), ["/fixture-home", "unset"]);
}
