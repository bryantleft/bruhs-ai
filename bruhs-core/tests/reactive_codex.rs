//! Exercise the production worker against the Codex wire protocol without paid calls.
#![cfg(unix)]

use bruhs_core::{reactive::ReactiveRuntime, AppState};
use bruhs_models::reactive::*;
use std::{os::unix::fs::PermissionsExt, process::Command, sync::Arc, time::Duration};

const FIXTURE_MODE: &str = "BRUHS_REACTIVE_CODEX_RECEIPT_FIXTURE";

#[test]
fn codex_worker_requires_a_successful_completion_receipt() {
    if let Ok(mode) = std::env::var(FIXTURE_MODE) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(receipt(&mode));
        return;
    }
    for mode in ["success", "failed_exit", "missing_receipt", "turn_failed"] {
        let directory = tempfile::tempdir().unwrap();
        let completion = if mode == "missing_receipt" {
            ""
        } else if mode == "turn_failed" {
            r#"{"type":"turn.failed","error":{"message":"fixture failure"}}"#
        } else {
            r#"{"type":"turn.completed","usage":{"input_tokens":12,"cached_input_tokens":0,"output_tokens":4}}"#
        };
        let script = format!(
            "#!/bin/sh\n/bin/cat >/dev/null\nprintf '%s\\n' '{}' '{}' '{}'\nexit {}\n",
            r#"{"type":"thread.started","thread_id":"fixture-session"}"#,
            r#"{"type":"item.completed","item":{"id":"message-1","type":"agent_message","text":"fixture answer"}}"#,
            completion,
            if mode == "failed_exit" { 7 } else { 0 }
        );
        let executable = directory.path().join("codex");
        std::fs::write(&executable, script).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let inherited = std::env::var_os("PATH").unwrap_or_default();
        let path = std::env::join_paths(
            std::iter::once(directory.path().to_path_buf())
                .chain(std::env::split_paths(&inherited)),
        )
        .unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "codex_worker_requires_a_successful_completion_receipt",
                "--nocapture",
            ])
            .env(FIXTURE_MODE, mode)
            .env("PATH", path)
            .env("CODEX_HOME", directory.path().join("codex-home"))
            .env(
                "BRUHS_CODEX_LEAN_SESSIONS_FILE",
                directory.path().join("lean-sessions.json"),
            )
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{mode}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

async fn receipt(mode: &str) {
    let directory = tempfile::tempdir().expect("Create an isolated worker fixture");
    let runtime = ReactiveRuntime::open(directory.path().join("reactive.sqlite3"))
        .await
        .expect("Open the fixture database");
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveAutomation {
                automation: AutomationSetup {
                    agent: ReactiveAgent {
                        id: "agent".into(),
                        name: "Fixture".into(),
                        instructions: "Answer".into(),
                        memory: String::new(),
                        model: "gpt-6-luna".into(),
                        project_path: directory.path().to_string_lossy().into_owned(),
                        enabled: true,
                    },
                    source: EventSource {
                        id: "source".into(),
                        name: "Fixture".into(),
                        enabled: true,
                        kind: SourceKind::Custom,
                    },
                    subscription: Subscription {
                        id: "automation".into(),
                        source_id: "source".into(),
                        agent_id: "agent".into(),
                        instructions: "Answer".into(),
                        enabled: true,
                        auto_run: true,
                        debounce_ms: 0,
                        throttle_ms: 0,
                    },
                }
                .into(),
            },
        })
        .await
        .expect("Activate the fixture automation");
    let app = Arc::new(
        AppState::builder()
            .secret_store(bruhs_core::secrets::SecretStore::in_memory())
            .connections(bruhs_core::connections::Connections::in_memory())
            .build(),
    );
    let worker = runtime.start(app).expect("Start the fixture worker");
    runtime
        .publish(
            "source",
            "receipt-test",
            serde_json::json!({"fixture":true}),
        )
        .await
        .expect("Publish the fixture event");
    let run = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let snapshot = runtime.snapshot().await.expect("Read the fixture receipt");
            if let Some(run) = snapshot
                .runs
                .into_iter()
                .find(|run| matches!(run.status, RunStatus::Completed | RunStatus::Failed))
            {
                break run;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("fixture Codex run must settle");
    worker.abort();
    if mode == "success" {
        assert_eq!(run.status, RunStatus::Completed);
        assert_eq!(run.result.as_deref(), Some("fixture answer"));
        assert_eq!(run.session_id.as_deref(), Some("fixture-session"));
    } else {
        assert_eq!(run.status, RunStatus::Failed);
        assert!(run.error.is_some());
    }
}
