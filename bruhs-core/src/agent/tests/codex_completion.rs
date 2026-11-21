#![cfg(unix)]

use super::*;
use std::os::unix::fs::PermissionsExt;

const INIT: &str = r#"{"type":"thread.started","thread_id":"fixture-thread"}"#;
const TEXT: &str = r#"{"type":"item.completed","item":{"type":"agent_message","id":"answer","text":"Verified final answer"}}"#;
const USAGE: &str = r#"{"type":"turn.completed","usage":{"input_tokens":20,"cached_input_tokens":7,"output_tokens":3}}"#;

async fn events(lines: &[&str], exit: i32, session: codex_sdk_rs::Session) -> Vec<QueryEvent> {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("codex");
    let output = lines
        .iter()
        .map(|line| format!("printf '%s\\n' '{line}'\n"))
        .collect::<String>();
    std::fs::write(
        &executable,
        format!("#!/bin/sh\n/bin/cat >/dev/null\n{output}exit {exit}\n"),
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let options = codex_sdk_rs::QueryOptions::new("Synthetic completion check")
        .cwd(directory.path())
        .session(session)
        .inherit_env(false)
        .env("PATH", directory.path().display().to_string())
        .env("HOME", directory.path().display().to_string())
        .env(
            "CODEX_HOME",
            directory.path().join("codex-home").display().to_string(),
        );
    let sessions: Sessions = Arc::new(Mutex::new(HashMap::new()));
    let registry = Arc::new(BackgroundTaskRegistry::new());
    let thread = ThreadId::new_v4();
    lock_sessions(&sessions).insert(
        thread.clone(),
        ThreadSession {
            process_id: None,
            status: ThreadStatus::Streaming,
            turn: 1,
            session: None,
        },
    );
    let (mut turn, mut receiver) = turn_context(&sessions, &registry, &thread, 1);
    turn.provider = AgentProvider::Codex;
    turn.media_project = directory.path().display().to_string();
    let producer = tokio::spawn(stream_codex(options, turn, "fixture-model".into(), None));
    let result = tokio::time::timeout(crate::capture_client::REQUEST_TIMEOUT, async {
        let mut result = Vec::new();
        while let Some(event) = receiver.recv().await {
            if let OrchestratorEvent::Query(event) = event {
                result.push(event);
            }
        }
        producer.await.unwrap();
        result
    })
    .await
    .unwrap();
    assert_eq!(lock_sessions(&sessions)[&thread].status, ThreadStatus::Idle);
    result
}

#[tokio::test]
async fn successful_codex_process_emits_one_common_completion_with_final_text_and_usage() {
    let commentary = r#"{"type":"item.completed","item":{"type":"agent_message","id":"commentary","text":"Inspecting the project"}}"#;
    let result = events(
        &[INIT, commentary, TEXT, USAGE],
        0,
        codex_sdk_rs::Session::New,
    )
    .await;
    let completions = result
        .iter()
        .filter(|event| matches!(event, QueryEvent::Result { .. }))
        .count();
    assert_eq!(completions, 1, "{result:?}");
    match result.last().unwrap() {
        QueryEvent::Result {
            result,
            session_id,
            usage,
            is_error,
            ..
        } => {
            assert_eq!(result, "Verified final answer");
            assert_eq!(session_id, "fixture-thread");
            assert!(!is_error);
            assert_eq!(usage.input_tokens, 13);
            assert_eq!(usage.cache_read_input_tokens, 7);
            assert_eq!(usage.output_tokens, 3);
        }
        other => panic!("expected settled completion, got {other:?}"),
    }
}

#[tokio::test]
async fn codex_completion_does_not_hide_a_later_failed_process_exit() {
    let result = events(&[INIT, TEXT, USAGE], 7, codex_sdk_rs::Session::New).await;
    assert!(result
        .iter()
        .any(|event| matches!(event, QueryEvent::Usage(_))));
    assert!(matches!(result.last(), Some(QueryEvent::Error(error)) if error.contains('7')));
    assert!(!result.iter().any(|event| matches!(
        event,
        QueryEvent::Result {
            is_error: false,
            ..
        }
    )));
}

#[tokio::test]
async fn codex_clean_exit_without_turn_completion_reports_an_error() {
    let result = events(&[INIT, TEXT], 0, codex_sdk_rs::Session::New).await;
    assert!(
        matches!(result.last(), Some(QueryEvent::Error(error)) if error.contains("completion")),
        "{result:?}"
    );
}

#[tokio::test]
async fn codex_transient_stream_error_can_recover_before_completion() {
    let reconnect = r#"{"type":"error","message":"Reconnecting..."}"#;
    let result = events(
        &[INIT, reconnect, TEXT, USAGE],
        0,
        codex_sdk_rs::Session::New,
    )
    .await;
    assert!(
        matches!(
            result.last(),
            Some(QueryEvent::Result {
                is_error: false,
                ..
            })
        ),
        "{result:?}"
    );
}

#[tokio::test]
async fn resumed_codex_completion_retains_the_requested_session_without_a_new_init() {
    let result = events(
        &[TEXT, USAGE],
        0,
        codex_sdk_rs::Session::Resume("saved-thread".into()),
    )
    .await;
    assert!(
        matches!(result.last(), Some(QueryEvent::Result { session_id, .. }) if session_id == "saved-thread"),
        "{result:?}"
    );
}

#[tokio::test]
async fn new_codex_turn_without_a_session_cannot_claim_resumable_completion() {
    let result = events(&[TEXT, USAGE], 0, codex_sdk_rs::Session::New).await;
    assert!(
        matches!(result.last(), Some(QueryEvent::Error(error)) if error.contains("session ID")),
        "{result:?}"
    );
}
