//! Process lifecycle and argument contract with the `codex` CLI, checked
//! against an owned fixture script on a temporary `PATH` (never the installed
//! CLI).
#![cfg(unix)]
use codex_sdk_rs::{
    query, ApprovalPolicy, CodexAccess, ConfigOverride, QueryEvent, QueryOptions, SandboxMode,
    Session,
};
use std::{os::unix::fs::PermissionsExt, path::Path, time::Duration};
use tokio_stream::StreamExt;

const STARTED: &str = r#"{"type":"thread.started","thread_id":"thread-1"}"#;

fn fixture(directory: &Path, body: &str) {
    let path = directory.join("codex");
    // The SDK points codex at its workspace with `--cd`, not the process
    // working directory, so the fixture moves next to itself.
    std::fs::write(&path, format!("#!/bin/sh\ncd \"${{0%/*}}\"\n{body}\n"))
        .expect("write owned fixture CLI");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .expect("fixture executable");
}

/// A fixture that records its argv (one per line) and stdin, then prints `line`.
fn recording_fixture(directory: &Path, line: &str) {
    fixture(
        directory,
        &format!(
            "printf '%s\\n' \"$@\" > arguments\n/bin/cat > stdin_prompt\nprintf '%s\\n' '{line}'"
        ),
    );
}

fn options(directory: &Path, prompt: &str) -> QueryOptions {
    QueryOptions::new(prompt)
        .cwd(directory)
        .env("PATH", directory.display().to_string())
}

async fn collect(mut stream: codex_sdk_rs::QueryStream) -> Vec<QueryEvent> {
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
async fn flag_like_prompts_are_sent_on_stdin_not_argv() {
    // `codex exec` reads the prompt from stdin when the operand is `-`, so a
    // literal `-` prompt cannot be passed as an argument at all.
    for prompt in ["--help", "-", "--dangerously-bypass-approvals-and-sandbox"] {
        let directory = tempfile::tempdir().unwrap();
        recording_fixture(directory.path(), STARTED);
        collect(query(options(directory.path(), prompt)).unwrap()).await;
        let args = arguments(directory.path());
        assert_eq!(&args[args.len() - 2..], ["--", "-"], "{args:?}");
        if prompt != "-" {
            assert!(!args.iter().any(|a| a == prompt), "{args:?}");
        }
        let stdin = std::fs::read_to_string(directory.path().join("stdin_prompt")).unwrap();
        assert_eq!(stdin, prompt);
    }
}

#[tokio::test]
async fn dropping_receiver_kills_and_reaps_a_silent_owned_provider_process() {
    let directory = tempfile::tempdir().unwrap();
    fixture(directory.path(), "echo $$ > owned_pid\nexec /bin/sleep 30");
    let stream = query(options(directory.path(), "hi")).unwrap();
    let pid_path = directory.path().join("owned_pid");
    let pid = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(&pid_path) {
                if let Ok(pid) = text.trim().parse::<u32>() {
                    break pid;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    drop(stream);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let status = tokio::process::Command::new("/bin/kill")
                .args(["-0", &pid.to_string()])
                .stderr(std::process::Stdio::null())
                .status()
                .await
                .unwrap();
            if !status.success() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("owned silent process must be killed and reaped after receiver drop");
}

#[tokio::test]
async fn resume_passes_the_session_id_and_prompt_as_operands() {
    let directory = tempfile::tempdir().unwrap();
    recording_fixture(directory.path(), STARTED);
    let opts = options(directory.path(), "continue").resume("thread-1");
    let events = collect(query(opts).unwrap()).await;
    assert!(matches!(&events[..], [QueryEvent::Init { thread_id }] if thread_id == "thread-1"));
    let args = arguments(directory.path());
    assert_eq!(&args[args.len() - 4..], ["resume", "--", "thread-1", "-"]);
    let stdin = std::fs::read_to_string(directory.path().join("stdin_prompt")).unwrap();
    assert_eq!(stdin, "continue");
}

#[tokio::test]
async fn flag_like_session_ids_are_rejected_before_spawning() {
    let directory = tempfile::tempdir().unwrap();
    recording_fixture(directory.path(), STARTED);
    let opts = options(directory.path(), "hi").resume("--last");
    let error = query(opts).expect_err("flag-like session id must be rejected");
    assert!(
        matches!(error, codex_sdk_rs::Error::InvalidSessionId(ref id) if id == "--last"),
        "{error:?}"
    );
    assert!(
        !directory.path().join("arguments").exists(),
        "CLI must not run"
    );
}

#[tokio::test]
async fn unsuccessful_exit_surfaces_exit_code_and_stderr_tail() {
    let directory = tempfile::tempdir().unwrap();
    fixture(
        directory.path(),
        "/bin/cat > /dev/null\necho 'Error: thread/resume failed: no rollout found' >&2\nexit 1",
    );
    let events = collect(query(options(directory.path(), "hi")).unwrap()).await;
    match &events[..] {
        [QueryEvent::ProcessFailed(failure)] => {
            assert_eq!(failure.exit_code, Some(1));
            assert_eq!(
                failure.stderr_tail,
                "Error: thread/resume failed: no rollout found"
            );
            assert!(failure.message.contains("codex exited unsuccessfully"));
        }
        other => panic!("expected only a ProcessFailed event, got {other:?}"),
    }
}

#[tokio::test]
async fn error_events_and_clean_exit_stream_through() {
    let directory = tempfile::tempdir().unwrap();
    fixture(
        directory.path(),
        r#"/bin/cat > /dev/null
printf '%s\n' '{"type":"error","message":"Reconnecting... 1/5"}'
printf '%s\n' '{"type":"turn.failed","error":{"message":"stream disconnected"}}'"#,
    );
    let events = collect(query(options(directory.path(), "hi")).unwrap()).await;
    assert!(
        matches!(
            &events[..],
            [QueryEvent::StreamError(notice), QueryEvent::Error(reason)]
                if notice == "Reconnecting... 1/5" && reason == "stream disconnected"
        ),
        "{events:?}"
    );
}

#[tokio::test]
async fn a_missing_cli_is_reported_as_not_found() {
    let directory = tempfile::tempdir().unwrap();
    let error = query(options(directory.path(), "hi")).expect_err("no codex on PATH");
    assert!(
        matches!(error, codex_sdk_rs::Error::CliNotFound),
        "{error:?}"
    );
}

#[tokio::test]
async fn new_threads_are_saved_so_they_can_be_resumed_unless_ephemeral() {
    for (session, ephemeral) in [(Session::New, false), (Session::Ephemeral, true)] {
        let directory = tempfile::tempdir().unwrap();
        recording_fixture(directory.path(), STARTED);
        let opts = options(directory.path(), "hi").session(session.clone());
        collect(query(opts).unwrap()).await;
        let args = arguments(directory.path());
        assert_eq!(
            args.iter().any(|a| a == "--ephemeral"),
            ephemeral,
            "{session:?}: {args:?}"
        );
        assert!(!args.iter().any(|a| a == "resume"), "{args:?}");
    }
}

#[tokio::test]
async fn access_maps_to_sandbox_and_approval_flags() {
    let cases = [
        (
            CodexAccess::default(),
            vec!["--ask-for-approval=never", "--sandbox=workspace-write"],
            "--dangerously-bypass-approvals-and-sandbox",
        ),
        (
            CodexAccess::Sandboxed {
                sandbox: SandboxMode::ReadOnly,
                approval: ApprovalPolicy::OnRequest,
            },
            vec!["--ask-for-approval=on-request", "--sandbox=read-only"],
            "--dangerously-bypass-approvals-and-sandbox",
        ),
        (
            CodexAccess::DangerouslyUnsandboxed,
            vec!["--dangerously-bypass-approvals-and-sandbox"],
            "--sandbox=workspace-write",
        ),
    ];
    for (access, present, absent) in cases {
        let directory = tempfile::tempdir().unwrap();
        recording_fixture(directory.path(), STARTED);
        collect(query(options(directory.path(), "hi").access(access)).unwrap()).await;
        let args = arguments(directory.path());
        for arg in present {
            assert!(args.iter().any(|a| a == arg), "missing {arg}: {args:?}");
        }
        assert!(!args.iter().any(|a| a == absent), "{args:?}");
        if access == CodexAccess::DangerouslyUnsandboxed {
            assert!(!args.iter().any(|a| a.starts_with("--ask-for-approval")));
        }
    }
}

#[tokio::test]
async fn config_overrides_are_single_arguments_and_keys_are_validated() {
    let directory = tempfile::tempdir().unwrap();
    recording_fixture(directory.path(), STARTED);
    let entry = ConfigOverride::new("model_reasoning_effort", "-high").unwrap();
    collect(query(options(directory.path(), "hi").config(entry)).unwrap()).await;
    let args = arguments(directory.path());
    assert!(
        args.iter()
            .any(|a| a == "--config=model_reasoning_effort=-high"),
        "{args:?}"
    );

    for key in ["", "sandbox_mode=danger-full-access"] {
        let error = ConfigOverride::new(key, "x").expect_err(key);
        assert!(
            matches!(error, codex_sdk_rs::Error::InvalidConfigKey(ref k) if k == key),
            "{error:?}"
        );
    }
}

/// Codex parses a `--config` value as TOML, so free text such as a meeting
/// transcript must arrive as one quoted TOML string: a bare `true` or `[1]`
/// would otherwise change type, and quotes or newlines would break parsing.
#[tokio::test]
async fn developer_instructions_are_one_quoted_toml_string() {
    let directory = tempfile::tempdir().unwrap();
    recording_fixture(directory.path(), STARTED);
    let text = "Say \"hi\"\n\\ok\t\u{7f}\u{1}";
    collect(query(options(directory.path(), "hi").developer_instructions(text)).unwrap()).await;
    let args = arguments(directory.path());
    let expected = r#"--config=developer_instructions="Say \"hi\"\n\\ok\t\u007F\u0001""#;
    assert!(args.iter().any(|a| a == expected), "{args:?}");
}

#[test]
fn toml_string_override_round_trips_through_a_toml_parser_shape() {
    let entry = ConfigOverride::string("developer_instructions", "true").unwrap();
    assert_eq!(entry.value(), "\"true\"", "a bare word stays a string");
}

/// A resumed turn reports its own usage, not the thread's running total:
/// the SDK reads the rollout's totals before and after the turn.
#[tokio::test]
async fn resumed_usage_is_this_turns_share_of_the_running_total() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    let day = home.join("sessions/2026/11/07");
    std::fs::create_dir_all(&day).unwrap();
    let rollout = day.join("rollout-2026-11-07T00-00-00-thread-1.jsonl");
    let count = |total: u64, cached: u64, last: u64| {
        format!(
            r#"{{"type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{total},"cached_input_tokens":{cached},"output_tokens":5}},"last_token_usage":{{"input_tokens":{last},"cached_input_tokens":0,"output_tokens":5}}}}}}}}"#
        )
    };
    std::fs::write(&rollout, format!("{}\n", count(21_739, 13_184, 21_739))).unwrap();
    let completed = r#"{"type":"turn.completed","usage":{"input_tokens":43496,"cached_input_tokens":34688,"output_tokens":10}}"#;
    fixture(
        directory.path(),
        &format!(
            "/bin/cat > /dev/null\nprintf '%s\\n' '{}' >> '{}'\nprintf '%s\\n' '{STARTED}' '{completed}'",
            count(43_496, 34_688, 21_757),
            rollout.display()
        ),
    );
    let events = collect(
        query(
            options(directory.path(), "hi")
                .env("CODEX_HOME", home.display().to_string())
                .resume("thread-1"),
        )
        .unwrap(),
    )
    .await;
    let usage = events
        .iter()
        .find_map(|event| match event {
            QueryEvent::Usage(usage) => Some(usage.clone()),
            _ => None,
        })
        .expect("usage event");
    assert_eq!(usage.input_tokens, 43_496 - 21_739);
    assert_eq!(usage.cached_input_tokens, 34_688 - 13_184);
    assert_eq!(usage.context_input_tokens, Some(21_757));
}

/// Without a rollout the CLI's own numbers pass through unchanged.
#[tokio::test]
async fn usage_without_a_rollout_is_passed_through() {
    let directory = tempfile::tempdir().unwrap();
    let completed = r#"{"type":"turn.completed","usage":{"input_tokens":7,"cached_input_tokens":3,"output_tokens":1}}"#;
    fixture(
        directory.path(),
        &format!("/bin/cat > /dev/null\nprintf '%s\\n' '{STARTED}' '{completed}'"),
    );
    let home = directory.path().join("empty-home");
    let events = collect(
        query(options(directory.path(), "hi").env("CODEX_HOME", home.display().to_string()))
            .unwrap(),
    )
    .await;
    let usage = events.iter().find_map(|event| match event {
        QueryEvent::Usage(usage) => Some(usage.clone()),
        _ => None,
    });
    let usage = usage.expect("usage event");
    assert_eq!((usage.input_tokens, usage.context_input_tokens), (7, None));
}
