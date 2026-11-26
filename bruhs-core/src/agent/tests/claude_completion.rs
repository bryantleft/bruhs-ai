#![cfg(unix)]

use super::*;
use std::os::unix::fs::PermissionsExt;

async fn events(result: bool, exit: i32) -> Vec<QueryEvent> {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("claude");
    let output = if result {
        r#"printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"result":"Final answer","session_id":"fixture-session","usage":{"input_tokens":10,"output_tokens":2}}'"#
    } else {
        ""
    };
    std::fs::write(&executable, format!("#!/bin/sh\n{output}\nexit {exit}\n")).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let options = claude_sdk_rs::QueryOptions::new("Synthetic settlement check")
        .cwd(directory.path())
        .inherit_env(false)
        .env("PATH", directory.path().display().to_string())
        .env("HOME", directory.path().display().to_string());
    let sessions: Sessions = Arc::new(Mutex::new(HashMap::new()));
    let registry = Arc::new(BackgroundTaskRegistry::new());
    let thread = ThreadId::new_v4();
    let (mut turn, mut receiver) = turn_context(&sessions, &registry, &thread, 1);
    turn.media_project = directory.path().display().to_string();
    let producer = tokio::spawn(stream_claude(
        options,
        turn,
        Arc::default(),
        Arc::default(),
        false,
    ));
    tokio::time::timeout(crate::capture_client::REQUEST_TIMEOUT, async {
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
    .unwrap()
}

#[tokio::test]
async fn claude_success_is_published_after_the_process_settles() {
    let result = events(true, 0).await;
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
async fn claude_result_cannot_hide_a_later_failed_exit() {
    let result = events(true, 7).await;
    assert!(
        matches!(result.last(), Some(QueryEvent::Error(error)) if error.contains('7')),
        "{result:?}"
    );
    assert!(
        !result.iter().any(|event| matches!(
            event,
            QueryEvent::Result {
                is_error: false,
                ..
            }
        )),
        "{result:?}"
    );
}

#[tokio::test]
async fn claude_eof_without_a_completion_is_an_error() {
    let result = events(false, 0).await;
    assert!(
        matches!(result.last(), Some(QueryEvent::Error(error)) if error.contains("completion")),
        "{result:?}"
    );
}
