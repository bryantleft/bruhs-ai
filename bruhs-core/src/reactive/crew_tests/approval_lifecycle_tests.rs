//! What happens to approvals and late grants when a bruh stops, its access
//! changes, a day passes or the service restarts.

use super::*;

async fn grant(runtime: &ReactiveRuntime, agent_id: &str, fingerprint: &str, age_ms: i64) {
    let (agent_id, fingerprint) = (agent_id.to_owned(), fingerprint.to_owned());
    runtime
        .call(move |connection| {
            connection.execute(
                "INSERT OR REPLACE INTO reactive_approval_grants(agent_id,fingerprint,created) VALUES(?1,?2,?3)",
                rusqlite::params![agent_id, fingerprint, EpochMs::now().as_millis() - age_ms],
            )?;
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn stop_and_access_changes_drop_late_grants_but_a_look_edit_keeps_them() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    grant(&runtime, "a", "f", 0).await;
    runtime
        .command(set_bruh("a", pandan(), AccessLevel::ReadOnly))
        .await
        .unwrap();
    assert!(take_grant(&runtime, "a", "f").await, "a look edit keeps it");

    grant(&runtime, "a", "f", 0).await;
    runtime
        .command(set_bruh("a", pandan(), AccessLevel::Ask))
        .await
        .unwrap();
    assert!(
        !take_grant(&runtime, "a", "f").await,
        "an access change drops it"
    );

    grant(&runtime, "a", "f", 0).await;
    runtime
        .command(ReactiveCommand::StopBruh {
            agent_id: "a".into(),
        })
        .await
        .unwrap();
    assert!(!take_grant(&runtime, "a", "f").await, "Stop drops it");

    grant(&runtime, "a", "f", 0).await;
    runtime.command(ReactiveCommand::StopAll).await.unwrap();
    assert!(!take_grant(&runtime, "a", "f").await, "Stop all drops it");
}

#[tokio::test]
async fn a_late_grant_expires_after_a_day() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    let ttl = store::crew::GRANT_TTL_MS;
    grant(&runtime, "a", "old", ttl + 60_000).await;
    grant(&runtime, "a", "new", ttl - 60_000).await;
    assert!(!take_grant(&runtime, "a", "old").await);
    assert!(take_grant(&runtime, "a", "new").await);
}

#[tokio::test]
async fn a_face_only_save_on_a_codex_bruh_stored_at_ask_keeps_its_runs() {
    let mut codex = agent("a");
    codex.model = "gpt-5.3-codex".into();
    let (_dir, runtime) = runtime_with(&[codex]).await;
    // Stored before Codex refused Ask: it already runs read only.
    runtime
        .call(|connection| {
            connection.execute(
                "INSERT INTO reactive_bruhs(agent_id,payload) VALUES('a',?1)",
                [serde_json::to_string(&profile(AccessLevel::Ask, None)).unwrap()],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let queued = chat(&runtime, "a", "Keep going").await;
    // The editor clamps Ask to Read only for Codex, so the save carries it.
    runtime
        .command(set_bruh("a", pandan(), AccessLevel::ReadOnly))
        .await
        .unwrap();
    assert_eq!(run(&runtime, &queued.id).await.status, RunStatus::Queued);
    // A real change of level still cancels.
    runtime
        .command(set_bruh("a", pandan(), AccessLevel::Full))
        .await
        .unwrap();
    let cancelled = run(&runtime, &queued.id).await;
    assert_eq!(cancelled.status, RunStatus::Cancelled);
    assert_eq!(cancelled.error.as_deref(), Some(ACCESS_CHANGED));
}

/// Open an approval for `run` directly, as the worker would, with a system
/// notification still to send.
async fn open_waiting(runtime: &ReactiveRuntime, run: &ReactiveRun) -> OutboxItem {
    runtime
        .command(ReactiveCommand::SetNotificationSettings {
            settings: NotificationSettings {
                system_ask: true,
                ..NotificationSettings::default()
            },
        })
        .await
        .unwrap();
    let opening = run.clone();
    let id = runtime
        .call(move |connection| {
            store::crew::open_approval(
                connection,
                &opening,
                "Bash",
                &json!({"command": "cargo test"}),
            )
        })
        .await
        .unwrap();
    let item = approval_item(runtime, &id).await;
    assert_eq!(item.system, SystemDelivery::Pending);
    item
}

#[tokio::test]
async fn a_restart_expires_the_approvals_its_interrupted_runs_waited_on() {
    let (dir, runtime) = runtime_with(&[agent("a")]).await;
    let run = claimed(&runtime, "a").await;
    let item = open_waiting(&runtime, &run).await;
    drop(runtime);
    let runtime = ReactiveRuntime::open(dir.path().join("reactive.sqlite3"))
        .await
        .unwrap();
    assert_eq!(
        super::run(&runtime, &run.id).await.status,
        RunStatus::Interrupted
    );
    let expired = approval_item(&runtime, &item.id).await;
    assert_eq!(expired.approval.unwrap().state, ApprovalState::Expired);
    assert!(
        expired.resolved_at.is_none(),
        "it stays open for a late answer"
    );
    assert_eq!(expired.system, SystemDelivery::Skipped);

    // A late approval resumes the bruh instead of being lost.
    let snapshot = decide_approval(&runtime, &item.id, true).await;
    assert!(snapshot
        .runs
        .iter()
        .any(|queued| queued.status == RunStatus::Queued
            && queued.trigger
                == Some(RunTrigger::Answer {
                    outbox_id: item.id.clone()
                })));
}

#[tokio::test]
async fn retrying_a_run_expires_its_old_approvals_so_a_late_answer_resumes() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    let first = claimed(&runtime, "a").await;
    let item = open_waiting(&runtime, &first).await;
    let id = first.id.clone();
    runtime
        .call(move |connection| {
            store::finish_run(
                connection,
                &id,
                RunStatus::Failed,
                None,
                Some("boom".into()),
                None,
            )
        })
        .await
        .unwrap();
    runtime
        .command(ReactiveCommand::RetryRun {
            id: first.id.clone(),
        })
        .await
        .unwrap();
    let expired = approval_item(&runtime, &item.id).await;
    assert_eq!(expired.approval.unwrap().state, ApprovalState::Expired);
    assert_eq!(expired.system, SystemDelivery::Skipped);
    // The retry runs under the same id, but nothing waits on the old card.
    let retried = claim_one(&runtime).await.unwrap().run;
    assert_eq!(retried.id, first.id);
    let snapshot = decide_approval(&runtime, &item.id, true).await;
    assert!(snapshot.runs.iter().any(|queued| queued.trigger
        == Some(RunTrigger::Answer {
            outbox_id: item.id.clone()
        })));
}

#[tokio::test]
async fn a_withdrawn_approval_skips_its_system_notification() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    let run = claimed(&runtime, "a").await;
    let item = open_waiting(&runtime, &run).await;
    let id = item.id.clone();
    runtime
        .call(move |connection| store::crew::withdraw_approval(connection, &id))
        .await
        .unwrap();
    let withdrawn = approval_item(&runtime, &item.id).await;
    assert_eq!(withdrawn.approval.unwrap().state, ApprovalState::Denied);
    assert_eq!(withdrawn.system, SystemDelivery::Skipped);
}
