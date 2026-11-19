use super::{store, worker, ReactiveError, ReactiveRuntime, SourceEmission};
use bruhs_models::{crew::BruhProfile, reactive::*, PermissionMode};
use serde_json::json;

fn agent(id: &str) -> ReactiveAgent {
    ReactiveAgent {
        id: id.into(),
        name: "Paperwork".into(),
        instructions: "Prepare drafts for review.".into(),
        memory: String::new(),
        model: "claude-sonnet-4-6".into(),
        project_path: std::env::temp_dir().to_string_lossy().into_owned(),
        enabled: true,
    }
}

fn source(id: &str) -> EventSource {
    EventSource {
        id: id.into(),
        name: "Scans".into(),
        enabled: true,
        kind: SourceKind::Custom,
    }
}

fn subscription(auto_run: bool) -> Subscription {
    Subscription {
        id: "paperwork".into(),
        source_id: "scans".into(),
        agent_id: "clerk".into(),
        instructions: "Extract the vendor and total. Never initiate a payment.".into(),
        enabled: true,
        auto_run,
        debounce_ms: 0,
        throttle_ms: 0,
        on_busy: Default::default(),
    }
}

async fn configured(auto_run: bool) -> (tempfile::TempDir, ReactiveRuntime) {
    let dir = tempfile::tempdir().unwrap();
    let runtime = ReactiveRuntime::open(dir.path().join("reactive.sqlite3"))
        .await
        .unwrap();
    for change in [
        ReactiveChange::SaveAgent {
            agent: agent("clerk"),
        },
        ReactiveChange::SaveSource {
            source: source("scans"),
        },
        ReactiveChange::SaveSubscription {
            subscription: subscription(auto_run),
        },
    ] {
        runtime
            .command(ReactiveCommand::Apply { change })
            .await
            .unwrap();
    }
    (dir, runtime)
}

#[tokio::test]
async fn duplicate_events_have_one_receipt_and_conflicting_reuse_fails() {
    let (_dir, runtime) = configured(false).await;
    let first = runtime
        .publish("scans", "invoice-1", json!({"total":"42"}))
        .await
        .unwrap();
    assert_eq!(
        runtime
            .publish("scans", "invoice-1", json!({"total":"42"}))
            .await
            .unwrap(),
        first
    );
    assert!(matches!(
        runtime
            .publish("scans", "invoice-1", json!({"total":"84"}))
            .await,
        Err(ReactiveError::Conflict(_))
    ));
    let snapshot = runtime.snapshot().await.unwrap();
    assert_eq!(snapshot.events.len(), 1);
    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(snapshot.runs[0].status, RunStatus::AwaitingReview);
    assert!(runtime.call(store::claim_next).await.unwrap().is_none());
}

#[tokio::test]
async fn second_runtime_cannot_recover_or_execute_a_live_database() {
    let (dir, runtime) = configured(true).await;
    runtime.publish("scans", "one", json!({})).await.unwrap();
    let store::Claim { run, .. } = runtime.call(store::claim_next).await.unwrap().unwrap();
    assert!(matches!(
        ReactiveRuntime::open(dir.path().join("reactive.sqlite3")).await,
        Err(ReactiveError::Conflict(_))
    ));
    assert_eq!(
        runtime.snapshot().await.unwrap().runs[0].status,
        RunStatus::Running
    );
    drop(runtime);
    let recovered = ReactiveRuntime::open(dir.path().join("reactive.sqlite3"))
        .await
        .unwrap();
    assert_eq!(recovered.snapshot().await.unwrap().runs[0].id, run.id);
    assert_eq!(
        recovered.snapshot().await.unwrap().runs[0].status,
        RunStatus::Interrupted
    );
}

#[tokio::test]
async fn cancelled_reactive_call_keeps_ownership_until_its_blocking_work_finishes() {
    struct Finished(Option<tokio::sync::oneshot::Sender<()>>);
    impl Drop for Finished {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }
    let (dir, runtime) = configured(true).await;
    let path = dir.path().join("reactive.sqlite3");
    let (entered_send, entered) = tokio::sync::oneshot::channel();
    let (resume, wait) = std::sync::mpsc::channel();
    let (finished_send, finished) = tokio::sync::oneshot::channel();
    let active = runtime.clone();
    let task = tokio::spawn(async move {
        active
            .call(move |_| {
                let _ = entered_send.send(());
                let _ = wait.recv();
                Ok(Finished(Some(finished_send)))
            })
            .await
    });
    entered.await.unwrap();
    task.abort();
    let _ = task.await;
    drop(runtime);
    assert!(matches!(
        ReactiveRuntime::open(&path).await,
        Err(ReactiveError::Conflict(_))
    ));
    resume.send(()).unwrap();
    finished.await.unwrap();
    let recovered = ReactiveRuntime::open(path)
        .await
        .expect("the canceled caller's worker has finished and released ownership");
    assert!(recovered.snapshot().await.is_ok());
}

#[cfg(unix)]
#[tokio::test]
async fn database_symlink_alias_cannot_bypass_runtime_ownership() {
    let (dir, _runtime) = configured(false).await;
    let alias = dir.path().join("alias.sqlite3");
    std::os::unix::fs::symlink(dir.path().join("reactive.sqlite3"), &alias).unwrap();
    assert!(matches!(
        ReactiveRuntime::open(alias).await,
        Err(ReactiveError::Conflict(_))
    ));
}

#[tokio::test]
async fn a_runtime_rejects_duplicate_workers_and_releases_its_start_lease_on_abort() {
    let (_dir, runtime) = configured(false).await;
    let app = std::sync::Arc::new(crate::AppState::new());
    let first = runtime.start(app.clone()).unwrap();
    assert!(matches!(
        runtime.start(app.clone()),
        Err(ReactiveError::Conflict(_))
    ));
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    let restarted = runtime.start(app).unwrap();
    restarted.abort();
    assert!(restarted.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn pausing_or_renaming_preserves_source_cursors_but_definition_changes_create_new_identities()
{
    let (_dir, runtime) = configured(false).await;
    runtime
        .ingest(
            &source("scans"),
            vec![SourceEmission {
                key: "one".into(),
                payload: json!({"revision":1}),
            }],
            "folder",
            json!({"revision":1}),
        )
        .await
        .unwrap();
    let mut changed = source("scans");
    changed.enabled = false;
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveSource {
                source: changed.clone(),
            },
        })
        .await
        .unwrap();
    changed.enabled = true;
    changed.name = "Renamed".into();
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveSource {
                source: changed.clone(),
            },
        })
        .await
        .unwrap();
    assert_eq!(
        runtime.cursor("scans", "folder").await.unwrap(),
        Some(json!({"revision":1}))
    );
    assert!(matches!(
        runtime.publish("scans", "one", json!({"revision":2})).await,
        Err(ReactiveError::Conflict(_))
    ));
    changed.kind = SourceKind::AppEvent {
        topic: "meeting.summary".into(),
    };
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveSource { source: changed },
        })
        .await
        .unwrap();
    assert_eq!(runtime.cursor("scans", "folder").await.unwrap(), None);
    runtime
        .publish("scans", "one", json!({"revision":2}))
        .await
        .unwrap();
    let snapshot = runtime.snapshot().await.unwrap();
    assert_eq!(snapshot.events.len(), 2);
    assert!(snapshot.events.iter().all(|event| event.key == "one"));
}

#[tokio::test]
async fn all_unresolved_runs_stay_visible_even_after_recent_terminal_history_fills() {
    let (_dir, runtime) = configured(false).await;
    runtime
        .publish("scans", "waiting", json!({}))
        .await
        .unwrap();
    let waiting = runtime.snapshot().await.unwrap().runs[0].id.clone();
    for _ in 0..HISTORY_LIMIT + 1 {
        let snapshot = runtime
            .command(ReactiveCommand::Chat {
                agent_id: "clerk".into(),
                message: "Draft".into(),
                run_id: None,
            })
            .await
            .unwrap();
        let id = snapshot
            .runs
            .iter()
            .find(|run| run.status == RunStatus::Queued)
            .unwrap()
            .id
            .clone();
        runtime
            .command(ReactiveCommand::CancelRun { id })
            .await
            .unwrap();
    }
    let snapshot = runtime.snapshot().await.unwrap();
    assert_eq!(snapshot.runs.len(), HISTORY_LIMIT + 1);
    assert!(snapshot
        .runs
        .iter()
        .any(|run| run.id == waiting && run.status == RunStatus::AwaitingReview));
}

#[tokio::test]
async fn queue_budget_is_global_and_ingestion_capacity_accounts_for_subscription_fanout() {
    let (_dir, runtime) = configured(false).await;
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveAgent {
                agent: agent("other"),
            },
        })
        .await
        .unwrap();
    let mut other = subscription(false);
    other.id = "other".into();
    other.agent_id = "other".into();
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveSubscription {
                subscription: other,
            },
        })
        .await
        .unwrap();
    assert_eq!(
        runtime.ingestion_capacity(&source("scans")).await.unwrap(),
        HISTORY_LIMIT / 2
    );
    for index in 0..HISTORY_LIMIT / 2 {
        runtime
            .publish("scans", &index.to_string(), json!({}))
            .await
            .unwrap();
    }
    assert_eq!(
        runtime.ingestion_capacity(&source("scans")).await.unwrap(),
        0
    );
    assert!(matches!(
        runtime
            .command(ReactiveCommand::Chat {
                agent_id: "other".into(),
                message: "One more".into(),
                run_id: None,
            })
            .await,
        Err(ReactiveError::Conflict(_))
    ));
    assert_eq!(runtime.snapshot().await.unwrap().runs.len(), HISTORY_LIMIT);
}

#[tokio::test]
async fn oversized_results_are_marked_partial_and_cannot_claim_completion() {
    let (_dir, runtime) = configured(true).await;
    runtime.publish("scans", "one", json!({})).await.unwrap();
    let store::Claim { run, .. } = runtime.call(store::claim_next).await.unwrap().unwrap();
    let id = run.id;
    runtime
        .call(move |connection| {
            store::finish_run(
                connection,
                &id,
                RunStatus::Completed,
                Some("言".repeat(MAX_TEXT_CHARS + 1)),
                None,
                None,
            )
        })
        .await
        .unwrap();
    let snapshot = runtime.snapshot().await.unwrap();
    assert_eq!(snapshot.runs[0].status, RunStatus::Failed);
    assert!(snapshot.runs[0]
        .error
        .as_deref()
        .unwrap()
        .contains("partial"));
    let result = snapshot.runs[0].result.as_deref().unwrap();
    assert_eq!(result.chars().count(), MAX_TEXT_CHARS);
    assert!(result.ends_with("[Output truncated; retained prefix only.]"));
}

#[tokio::test]
async fn cancellation_retains_partial_output_without_overriding_its_receipt() {
    let (_dir, runtime) = configured(true).await;
    runtime.publish("scans", "one", json!({})).await.unwrap();
    let store::Claim { run, .. } = runtime.call(store::claim_next).await.unwrap().unwrap();
    runtime
        .command(ReactiveCommand::CancelRun { id: run.id.clone() })
        .await
        .unwrap();
    let id = run.id;
    runtime
        .call(move |connection| store::cancelled_output(connection, &id, "Partial draft".into()))
        .await
        .unwrap();
    let snapshot = runtime.snapshot().await.unwrap();
    assert_eq!(snapshot.runs[0].status, RunStatus::Cancelled);
    assert_eq!(snapshot.runs[0].result.as_deref(), Some("Partial draft"));
    assert!(snapshot.runs[0]
        .error
        .as_deref()
        .unwrap()
        .contains("partial"));
}

#[tokio::test]
async fn sources_are_idle_until_an_enabled_agent_subscribes_and_races_cannot_commit() {
    let (_dir, runtime) = configured(false).await;
    assert_eq!(
        runtime.source_configs().await.unwrap(),
        vec![source("scans")]
    );
    let mut disabled = subscription(false);
    disabled.enabled = false;
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveSubscription {
                subscription: disabled,
            },
        })
        .await
        .unwrap();
    assert!(runtime.source_configs().await.unwrap().is_empty());
    assert!(matches!(
        runtime
            .ingest(
                &source("scans"),
                vec![SourceEmission {
                    key: "late".into(),
                    payload: json!({})
                }],
                "folder",
                json!({})
            )
            .await,
        Err(ReactiveError::Conflict(_))
    ));
    assert_eq!(runtime.cursor("scans", "folder").await.unwrap(), None);
}

#[tokio::test]
async fn explicit_approval_and_opted_in_subscriptions_both_keep_plan_permissions() {
    let (_dir, runtime) = configured(false).await;
    runtime.publish("scans", "one", json!({})).await.unwrap();
    let run = runtime.snapshot().await.unwrap().runs.remove(0);
    runtime
        .command(ReactiveCommand::ApproveRun { id: run.id.clone() })
        .await
        .unwrap();
    let store::Claim { agent, run, .. } = runtime.call(store::claim_next).await.unwrap().unwrap();
    let turn = worker::turn(
        &agent,
        &BruhProfile::default(),
        &run,
        "",
        std::path::Path::new("/home"),
        &worker::TurnVault::default(),
    );
    assert_eq!(turn.permission_mode, PermissionMode::Plan);
    assert_eq!(
        turn.thread_access,
        bruhs_models::thread_link::ThreadAccess::Off
    );
    assert!(turn.fresh_session);
    let id = run.id.clone();
    runtime
        .call(move |connection| {
            store::finish_run(
                connection,
                &id,
                RunStatus::Completed,
                Some("Draft".into()),
                None,
                None,
            )
        })
        .await
        .unwrap();
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveSubscription {
                subscription: subscription(true),
            },
        })
        .await
        .unwrap();
    runtime.publish("scans", "two", json!({})).await.unwrap();
    let store::Claim { run, .. } = runtime.call(store::claim_next).await.unwrap().unwrap();
    assert_eq!(
        worker::turn(
            &agent,
            &BruhProfile::default(),
            &run,
            "",
            std::path::Path::new("/home"),
            &worker::TurnVault::default()
        )
        .permission_mode,
        PermissionMode::Plan
    );
}

#[tokio::test]
async fn restart_preserves_queued_work_and_marks_active_work_interrupted() {
    let (dir, runtime) = configured(true).await;
    runtime.publish("scans", "one", json!({})).await.unwrap();
    let store::Claim { run: running, .. } = runtime.call(store::claim_next).await.unwrap().unwrap();
    // Arrives mid-run, so it stays queued rather than joining the running turn.
    runtime.publish("scans", "two", json!({})).await.unwrap();
    let id = running.id.clone();
    runtime
        .call(move |connection| {
            store::record_session(connection, &id, "persistent-session", "claude-sonnet-4-6")
        })
        .await
        .unwrap();
    drop(runtime);
    let runtime = ReactiveRuntime::open(dir.path().join("reactive.sqlite3"))
        .await
        .unwrap();
    let snapshot = runtime.snapshot().await.unwrap();
    let interrupted = snapshot
        .runs
        .iter()
        .find(|run| run.id == running.id)
        .unwrap();
    assert_eq!(interrupted.status, RunStatus::Interrupted);
    assert_eq!(
        interrupted.session_id.as_deref(),
        Some("persistent-session")
    );
    assert_eq!(
        snapshot
            .runs
            .iter()
            .filter(|run| run.status == RunStatus::Queued)
            .count(),
        1
    );
    let store::Claim { run: next, .. } = runtime.call(store::claim_next).await.unwrap().unwrap();
    assert_ne!(next.id, running.id);
    assert_eq!(next.session_id.as_deref(), Some("persistent-session"));
}

#[tokio::test]
async fn source_checkpoint_and_all_emissions_roll_back_together_on_error() {
    let (_dir, runtime) = configured(false).await;
    let events = vec![
        SourceEmission {
            key: "first".into(),
            payload: json!({}),
        },
        SourceEmission {
            key: String::new(),
            payload: json!({}),
        },
    ];
    assert!(runtime
        .ingest(&source("scans"), events, "folder", json!({"revision":1}))
        .await
        .is_err());
    assert_eq!(runtime.cursor("scans", "folder").await.unwrap(), None);
    let snapshot = runtime.snapshot().await.unwrap();
    assert!(snapshot.events.is_empty() && snapshot.runs.is_empty());
    runtime
        .ingest(
            &source("scans"),
            vec![SourceEmission {
                key: "first".into(),
                payload: json!({}),
            }],
            "folder",
            json!({"revision":1}),
        )
        .await
        .unwrap();
    assert_eq!(
        runtime.cursor("scans", "folder").await.unwrap(),
        Some(json!({"revision":1}))
    );
}

#[tokio::test]
async fn stale_polling_results_and_disabled_sources_do_not_publish() {
    let (_dir, runtime) = configured(false).await;
    let original = source("scans");
    let mut changed = original.clone();
    changed.name = "New setup".into();
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveSource {
                source: changed.clone(),
            },
        })
        .await
        .unwrap();
    assert!(matches!(
        runtime
            .ingest(
                &original,
                vec![SourceEmission {
                    key: "one".into(),
                    payload: json!({})
                }],
                "folder",
                json!({})
            )
            .await,
        Err(ReactiveError::Conflict(_))
    ));
    changed.enabled = false;
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveSource { source: changed },
        })
        .await
        .unwrap();
    assert!(runtime.publish("scans", "two", json!({})).await.is_err());
    assert!(runtime.snapshot().await.unwrap().events.is_empty());
}

#[tokio::test]
async fn proposals_are_inert_until_approved_and_invalid_references_are_rejected() {
    let (_dir, runtime) = configured(false).await;
    let mut changed = agent("clerk");
    changed.memory = "Prefer itemized summaries.".into();
    let snapshot = runtime
        .command(ReactiveCommand::Propose {
            change: ReactiveChange::SaveAgent {
                agent: changed.clone(),
            },
        })
        .await
        .unwrap();
    assert!(snapshot.agents[0].memory.is_empty());
    let proposal = &snapshot.proposals[0];
    let snapshot = runtime
        .command(ReactiveCommand::DecideProposal {
            id: proposal.id.clone(),
            approve: true,
        })
        .await
        .unwrap();
    assert_eq!(snapshot.agents[0], changed);
    assert_eq!(snapshot.proposals[0].status, ProposalStatus::Approved);
    runtime
        .command(ReactiveCommand::DecideProposal {
            id: proposal.id.clone(),
            approve: true,
        })
        .await
        .unwrap();
    let mut bad = subscription(false);
    bad.agent_id = "missing".into();
    assert!(matches!(
        runtime
            .command(ReactiveCommand::Apply {
                change: ReactiveChange::SaveSubscription { subscription: bad }
            })
            .await,
        Err(ReactiveError::NotFound(_))
    ));
    assert_eq!(runtime.snapshot().await.unwrap().subscriptions.len(), 1);
}

#[tokio::test]
async fn disabling_or_deleting_configuration_cancels_pending_work() {
    let (_dir, runtime) = configured(true).await;
    runtime.publish("scans", "one", json!({})).await.unwrap();
    let mut disabled = agent("clerk");
    disabled.enabled = false;
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveAgent { agent: disabled },
        })
        .await
        .unwrap();
    let snapshot = runtime.snapshot().await.unwrap();
    assert_eq!(snapshot.runs[0].status, RunStatus::Cancelled);
    assert!(runtime
        .command(ReactiveCommand::RetryRun {
            id: snapshot.runs[0].id.clone()
        })
        .await
        .is_err());
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::DeleteSource { id: "scans".into() },
        })
        .await
        .unwrap();
    let snapshot = runtime.snapshot().await.unwrap();
    assert!(snapshot.sources.is_empty() && snapshot.subscriptions.is_empty());
    assert_eq!(snapshot.events.len(), 1);
}

#[tokio::test]
async fn memory_changes_drop_the_provider_session_and_fence_older_approved_turns() {
    let (_dir, runtime) = configured(true).await;
    runtime
        .command(ReactiveCommand::Chat {
            agent_id: "clerk".into(),
            message: "Remember these preferences.".into(),
            run_id: None,
        })
        .await
        .unwrap();
    let store::Claim { run, .. } = runtime.call(store::claim_next).await.unwrap().unwrap();
    let id = run.id.clone();
    runtime
        .call(move |connection| {
            store::record_session(connection, &id, "old-session", "claude-sonnet-4-6")
        })
        .await
        .unwrap();
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SetMemory {
                agent_id: "clerk".into(),
                memory: "Prefer a table.".into(),
            },
        })
        .await
        .unwrap();
    runtime
        .command(ReactiveCommand::Chat {
            agent_id: "clerk".into(),
            message: "Prepare a draft.".into(),
            run_id: None,
        })
        .await
        .unwrap();
    let store::Claim { agent, run, .. } = runtime.call(store::claim_next).await.unwrap().unwrap();
    let turn = worker::turn(
        &agent,
        &BruhProfile::default(),
        &run,
        "",
        std::path::Path::new("/home"),
        &worker::TurnVault::default(),
    );
    assert!(run.session_id.is_none() && turn.fresh_session);
    assert!(turn.instructions.unwrap().contains("Prefer a table."));
}

#[tokio::test]
async fn webhook_credentials_are_private_and_checked_before_publication() {
    let (_dir, runtime) = configured(false).await;
    let mut hook = source("hook");
    hook.kind = SourceKind::Webhook;
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveSource { source: hook },
        })
        .await
        .unwrap();
    let token = runtime.webhook_token("hook").await.unwrap();
    assert!(!serde_json::to_string(&runtime.snapshot().await.unwrap())
        .unwrap()
        .contains(&token));
    assert!(matches!(
        runtime
            .publish_hook("hook", "wrong", "one", json!({}))
            .await,
        Err(ReactiveError::Unauthorized)
    ));
    assert!(runtime.snapshot().await.unwrap().events.is_empty());
    runtime
        .publish_hook("hook", &token, "one", json!({}))
        .await
        .unwrap();
    assert_eq!(runtime.snapshot().await.unwrap().events.len(), 1);
}

#[tokio::test]
async fn debounce_coalesces_pending_runs_and_new_content_requires_new_review() {
    let (_dir, runtime) = configured(false).await;
    let mut delayed = subscription(false);
    delayed.debounce_ms = 10_000;
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveSubscription {
                subscription: delayed,
            },
        })
        .await
        .unwrap();
    runtime
        .publish("scans", "one", json!({"revision":1}))
        .await
        .unwrap();
    let id = runtime.snapshot().await.unwrap().runs[0].id.clone();
    runtime
        .command(ReactiveCommand::ApproveRun { id: id.clone() })
        .await
        .unwrap();
    runtime
        .publish("scans", "two", json!({"revision":2}))
        .await
        .unwrap();
    let snapshot = runtime.snapshot().await.unwrap();
    assert_eq!(snapshot.events.len(), 2);
    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(snapshot.runs[0].id, id);
    assert_eq!(snapshot.runs[0].status, RunStatus::AwaitingReview);
    assert!(snapshot.runs[0].prompt.contains("\"revision\":2"));
    assert!(runtime.call(store::claim_next).await.unwrap().is_none());
}

#[tokio::test]
async fn late_success_cannot_overwrite_cancellation() {
    let (_dir, runtime) = configured(true).await;
    runtime.publish("scans", "one", json!({})).await.unwrap();
    let store::Claim { run, .. } = runtime.call(store::claim_next).await.unwrap().unwrap();
    runtime
        .command(ReactiveCommand::CancelRun { id: run.id.clone() })
        .await
        .unwrap();
    let id = run.id;
    runtime
        .call(move |connection| {
            store::finish_run(
                connection,
                &id,
                RunStatus::Completed,
                Some("Late answer".into()),
                None,
                None,
            )
        })
        .await
        .unwrap();
    assert_eq!(
        runtime.snapshot().await.unwrap().runs[0].status,
        RunStatus::Cancelled
    );
}

#[tokio::test]
async fn app_completion_markers_are_idempotent_persistent_and_independent_of_sources() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reactive.sqlite3");
    let runtime = ReactiveRuntime::open(&path).await.unwrap();
    let changes = runtime.watch_changes();
    runtime
        .record_app_completion("meeting.transcription_completed", "meeting-1")
        .await
        .unwrap();
    runtime
        .record_app_completion("meeting.transcription_completed", "meeting-1")
        .await
        .unwrap();
    runtime
        .record_app_completion("meeting.summary", "meeting-2")
        .await
        .unwrap();
    assert!(changes.has_changed().unwrap());
    assert_eq!(
        runtime
            .app_completions("meeting.transcription_completed")
            .await
            .unwrap(),
        vec!["meeting-1"]
    );
    assert_eq!(
        runtime.app_completions("meeting.summary").await.unwrap(),
        vec!["meeting-2"]
    );
    assert!(runtime
        .app_completions("other.topic")
        .await
        .unwrap()
        .is_empty());
    assert!(matches!(
        runtime
            .record_app_completion("meeting.transcription_completed", "../bad")
            .await,
        Err(ReactiveError::Invalid(_))
    ));
    assert!(matches!(
        runtime
            .record_app_completion("invalid\ntopic", "meeting-1")
            .await,
        Err(ReactiveError::Invalid(_))
    ));
    let snapshot = runtime.snapshot().await.unwrap();
    assert!(snapshot.sources.is_empty() && snapshot.events.is_empty() && snapshot.runs.is_empty());
    drop(runtime);
    let recovered = ReactiveRuntime::open(path).await.unwrap();
    assert_eq!(
        recovered
            .app_completions("meeting.transcription_completed")
            .await
            .unwrap(),
        vec!["meeting-1"]
    );
    assert_eq!(
        recovered.app_completions("meeting.summary").await.unwrap(),
        vec!["meeting-2"]
    );
}
