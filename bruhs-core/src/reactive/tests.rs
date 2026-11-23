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
        webhook_filters: Vec::new(),
        webhook_prompt: None,
        webhook_security: Default::default(),
        webhook_dedupe: Default::default(),
        task_title: None,
        task_conversation: Default::default(),
        task_checkout: Default::default(),
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
async fn task_webhook_rotation_preserves_isolated_retry_history() {
    use super::WebhookTarget;
    let (dir, runtime) = configured(false).await;
    let mut hook = source("scans");
    hook.kind = SourceKind::Webhook;
    let other = Subscription {
        id: "other".into(),
        ..subscription(false)
    };
    for change in [
        ReactiveChange::SaveSource { source: hook },
        ReactiveChange::SaveSubscription {
            subscription: other,
        },
    ] {
        runtime
            .command(ReactiveCommand::Apply { change })
            .await
            .unwrap();
    }
    async fn send(
        runtime: &ReactiveRuntime,
        task: &str,
        token: &str,
        content: i32,
    ) -> Result<ReactiveEvent, ReactiveError> {
        runtime
            .receive_addressed_webhook(
                WebhookTarget::Task(task.into()),
                token,
                "same-delivery",
                Some(json!(content)),
                Default::default(),
                bruhs_models::EpochMs::now(),
            )
            .await
    }
    let first = runtime
        .task_webhook_token("paperwork".into(), None)
        .await
        .unwrap();
    let other = runtime
        .task_webhook_token("other".into(), None)
        .await
        .unwrap();
    assert!(matches!(
        send(&runtime, "other", &first, 1).await,
        Err(ReactiveError::Unauthorized)
    ));
    let event = send(&runtime, "paperwork", &first, 1).await.unwrap();
    assert!(runtime
        .webhook_deliveries("other".into())
        .await
        .unwrap()
        .is_empty());
    assert!(matches!(
        send(&runtime, "paperwork", &first, 2).await,
        Err(ReactiveError::Conflict(_))
    ));
    assert!(runtime
        .webhook_deliveries("other".into())
        .await
        .unwrap()
        .is_empty());
    let sibling = send(&runtime, "other", &other, 2).await.unwrap();
    assert_ne!(sibling.id, event.id);
    let rotated = runtime
        .task_webhook_token("paperwork".into(), Some(first.clone()))
        .await
        .unwrap();
    assert_ne!(first, rotated);
    assert!(matches!(
        runtime
            .task_webhook_token("paperwork".into(), Some(first.clone()))
            .await,
        Err(ReactiveError::Conflict(_))
    ));
    drop(runtime);
    let runtime = ReactiveRuntime::open(dir.path().join("reactive.sqlite3"))
        .await
        .unwrap();
    assert!(matches!(
        send(&runtime, "paperwork", &first, 1).await,
        Err(ReactiveError::Unauthorized)
    ));
    assert_eq!(
        send(&runtime, "paperwork", &rotated, 1).await.unwrap().id,
        event.id
    );
    assert_eq!(
        send(&runtime, "other", &other, 2).await.unwrap().id,
        sibling.id
    );
    let source_key = runtime.webhook_token("scans").await.unwrap();
    runtime
        .publish_hook("scans", &source_key, "same-delivery", json!(3))
        .await
        .unwrap();
    let snapshot = runtime.snapshot().await.unwrap();
    assert_eq!(snapshot.runs.len(), 4);
    for secret in [first, rotated.clone(), other, source_key] {
        assert!(!serde_json::to_string(&snapshot).unwrap().contains(&secret));
    }
    for change in [
        ReactiveChange::DeleteSubscription {
            id: "paperwork".into(),
        },
        ReactiveChange::SaveSubscription {
            subscription: subscription(false),
        },
    ] {
        runtime
            .command(ReactiveCommand::Apply { change })
            .await
            .unwrap();
    }
    assert!(matches!(
        send(&runtime, "paperwork", &rotated, 1).await,
        Err(ReactiveError::Unauthorized)
    ));
    let fresh = runtime
        .task_webhook_token("paperwork".into(), None)
        .await
        .unwrap();
    assert_ne!(
        send(&runtime, "paperwork", &fresh, 1).await.unwrap().id,
        event.id
    );
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

#[tokio::test]
async fn webhook_filters_route_individual_tasks_and_retain_the_delivery_decision() {
    let (dir, runtime) = configured(false).await;
    let mut hook = source("hook");
    hook.kind = SourceKind::Webhook;
    let mut filtered = subscription(false);
    filtered.id = "filtered".into();
    filtered.source_id = hook.id.clone();
    filtered.webhook_filters = vec![
        WebhookFilter {
            path: "body.items.[].name".into(),
            test: WebhookTest::In(vec!["apple".into(), "pear".into()]),
        },
        WebhookFilter {
            path: "headers.X-Event".into(),
            test: WebhookTest::StartsWith("ready".into()),
        },
        WebhookFilter {
            path: "query.queue".into(),
            test: WebhookTest::Equals("local".into()),
        },
        WebhookFilter {
            path: "body.items[].name".into(),
            test: WebhookTest::NotEquals("skip".into()),
        },
        WebhookFilter {
            path: "body.details".into(),
            test: WebhookTest::Exists,
        },
    ];
    let mut always = filtered.clone();
    always.id = "always".into();
    always.webhook_filters.clear();
    for change in [
        ReactiveChange::SaveSource { source: hook },
        ReactiveChange::SaveSubscription {
            subscription: filtered.clone(),
        },
        ReactiveChange::SaveSubscription {
            subscription: always,
        },
    ] {
        runtime
            .command(ReactiveCommand::Apply { change })
            .await
            .unwrap();
    }
    let token = runtime.webhook_token("hook").await.unwrap();
    drop(runtime);
    let runtime = ReactiveRuntime::open(dir.path().join("reactive.sqlite3"))
        .await
        .unwrap();
    let request = || WebhookMetadata {
        headers: [("x-event".into(), vec!["ready.fixture-only".into()])]
            .into_iter()
            .collect(),
        query: [("queue".into(), vec!["first".into(), "local".into()])]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    let payload = json!({"items":[{"name":"pear"},{"name":"berry"}],"details":{}});
    let matched = runtime
        .publish_hook_request("hook", &token, "accepted", payload.clone(), request())
        .await
        .unwrap();
    assert!(matched.filtered_subscriptions.is_empty());
    let skipped = runtime
        .publish_hook("hook", &token, "missing-header", payload.clone())
        .await
        .unwrap();
    assert_eq!(skipped.filtered_subscriptions, ["filtered"]);
    let retried = runtime
        .publish_hook_request("hook", &token, "missing-header", payload, request())
        .await
        .unwrap();
    assert_eq!(skipped, retried);
    let excluded = runtime
        .publish_hook_request(
            "hook",
            &token,
            "excluded",
            json!({"items":[{"name":"pear"},{"name":"skip"}],"details":{}}),
            request(),
        )
        .await
        .unwrap();
    assert_eq!(excluded.filtered_subscriptions, ["filtered"]);
    let snapshot = runtime.snapshot().await.unwrap();
    assert_eq!(snapshot.runs.len(), 4);
    assert!(!serde_json::to_string(&snapshot)
        .unwrap()
        .contains("ready.fixture-only"));
    let mut changed_source = source("hook");
    changed_source.kind = SourceKind::Custom;
    let change = ReactiveChange::SaveSource {
        source: changed_source,
    };
    assert!(runtime
        .command(ReactiveCommand::Apply { change })
        .await
        .is_err());
    assert_eq!(
        runtime
            .snapshot()
            .await
            .unwrap()
            .sources
            .into_iter()
            .find(|source| source.id == "hook")
            .unwrap()
            .kind,
        SourceKind::Webhook,
    );
    filtered.webhook_filters[0].path = "invalid.root".into();
    assert!(runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveSubscription {
                subscription: filtered
            }
        })
        .await
        .is_err());
}

#[tokio::test]
async fn webhook_template_deliveries_isolate_task_prompts_and_mask_owner_previews() {
    let (_dir, runtime) = configured(false).await;
    let mut src = source("scans");
    src.kind = SourceKind::Webhook;
    let mut task = subscription(false);
    task.webhook_prompt = Some("Review {{body.subject}} {{headers.x-event}} {{query.zone}} {{body.password}} {{headers.authorization}} {{body.absent}}".into());
    let mut legacy = subscription(false);
    legacy.id = "legacy".into();
    for change in [
        ReactiveChange::SaveSource { source: src },
        ReactiveChange::SaveSubscription {
            subscription: legacy,
        },
        ReactiveChange::SaveSubscription {
            subscription: task.clone(),
        },
    ] {
        runtime
            .command(ReactiveCommand::Apply { change })
            .await
            .unwrap();
    }
    let token = runtime.webhook_token("scans").await.unwrap();
    let request = WebhookMetadata {
        headers: [
            ("x-event".into(), vec!["triage".into()]),
            ("authorization".into(), vec![format!("Bearer {token}")]),
        ]
        .into(),
        query: [("zone".into(), vec!["central".into()])].into(),
        ..Default::default()
    };
    let body = json!({"subject":"Invoice", "password":"demo-secret", "internal":"only-in-legacy"});
    runtime
        .publish_hook_request("scans", &token, "one", body, request)
        .await
        .unwrap();
    let runs = runtime.snapshot().await.unwrap().runs;
    let prompt_for = |id: &str| {
        &runs
            .iter()
            .find(|r| r.subscription_id.as_deref() == Some(id))
            .unwrap()
            .prompt
    };
    let prompt = prompt_for(&task.id);
    for used in ["Invoice", "triage", "central", "demo-secret"] {
        assert!(prompt.contains(used));
    }
    for omitted in ["only-in-legacy", "<event_data>", token.as_str()] {
        assert!(!prompt.contains(omitted));
    }
    assert!(prompt_for("legacy").contains("only-in-legacy"));
    let row = runtime
        .webhook_deliveries(task.id.clone())
        .await
        .unwrap()
        .remove(0);
    let detail = runtime
        .webhook_delivery(task.id.clone(), row.id)
        .await
        .unwrap();
    assert_eq!(detail.missing_fields, ["body.absent"]);
    let preview = detail.prompt.unwrap();
    assert!(preview.contains("central") && preview.contains("[redacted]"));
    assert!(!preview.contains("demo-secret") && !preview.contains(&token));
    let row = runtime
        .webhook_deliveries("legacy".into())
        .await
        .unwrap()
        .remove(0);
    let sibling = runtime
        .webhook_delivery("legacy".into(), row.id)
        .await
        .unwrap();
    assert!(sibling.missing_fields.is_empty());
    assert!(sibling.prompt.unwrap().contains("only-in-legacy"));
}

#[tokio::test]
async fn oversized_webhook_prompt_does_not_replace_a_window_or_claim_its_replay_key() {
    let (_dir, runtime) = configured(false).await;
    let mut src = source("scans");
    src.kind = SourceKind::Webhook;
    let mut task = subscription(false);
    task.webhook_prompt = Some("{{body.text}} {{body.text}}".into());
    task.webhook_dedupe = WebhookDedupe {
        replay_key: "{{body.id}}".into(),
        coalesce_key: "{{body.group}}".into(),
        coalesce_window_seconds: Some(10),
    };
    for change in [
        ReactiveChange::SaveSource { source: src },
        ReactiveChange::SaveSubscription {
            subscription: task.clone(),
        },
    ] {
        runtime
            .command(ReactiveCommand::Apply { change })
            .await
            .unwrap();
    }
    let token = runtime.webhook_token("scans").await.unwrap();
    for (key, id, text, expected) in [
        ("a", "a", "accepted".into(), WebhookOutcome::WindowOpened),
        ("b", "b", "x".repeat(61_000), WebhookOutcome::Rejected),
        ("c", "b", "retry".into(), WebhookOutcome::Coalesced),
    ] {
        runtime
            .publish_hook(
                "scans",
                &token,
                key,
                json!({"id":id, "group":"one", "text":text}),
            )
            .await
            .unwrap();
        let deliveries = runtime.webhook_deliveries(task.id.clone()).await.unwrap();
        assert_eq!(deliveries[0].outcome, expected);
        if expected == WebhookOutcome::Rejected {
            let rejected = runtime
                .webhook_delivery(task.id.clone(), deliveries[0].id.clone())
                .await
                .unwrap();
            assert!(rejected.error.unwrap().contains("120,000"));
            assert!(rejected.prompt.is_none());
            let opener = runtime
                .webhook_delivery(task.id.clone(), deliveries[1].id.clone())
                .await
                .unwrap();
            assert!(opener.prompt.unwrap().contains("accepted"));
        }
    }
}

#[tokio::test]
async fn keyed_webhooks_keep_fixed_windows_and_seven_day_replay_across_restart() {
    let (dir, runtime) = configured(false).await;
    let mut src = source("scans");
    src.kind = SourceKind::Webhook;
    let mut task = subscription(false);
    task.webhook_dedupe = WebhookDedupe {
        replay_key: "{{body.delivery}}".into(),
        coalesce_key: "{{body.issue}}".into(),
        coalesce_window_seconds: Some(10),
    };
    let mut sibling = subscription(false);
    sibling.id = "legacy".into();
    for change in [
        ReactiveChange::SaveSource { source: src },
        ReactiveChange::SaveSubscription {
            subscription: task.clone(),
        },
        ReactiveChange::SaveSubscription {
            subscription: sibling,
        },
    ] {
        runtime
            .command(ReactiveCommand::Apply { change })
            .await
            .unwrap();
    }
    let token = runtime.webhook_token("scans").await.unwrap();
    let publish = async |runtime: &ReactiveRuntime, delivery: &str, issue: &str, version: u32| {
        runtime
            .publish_hook(
                "scans",
                &token,
                "stable-transport",
                json!({"delivery":delivery,"issue":issue,"version":version}),
            )
            .await
            .unwrap()
    };
    let first = publish(&runtime, "a", "A", 1).await;
    let original = runtime.webhook_deliveries(task.id.clone()).await.unwrap()[0].clone();
    assert_eq!(original.outcome, WebhookOutcome::WindowOpened);
    drop(runtime);
    let runtime = ReactiveRuntime::open(dir.path().join("reactive.sqlite3"))
        .await
        .unwrap();
    publish(&runtime, "a", "A", 2).await;
    assert_eq!(
        runtime.webhook_deliveries(task.id.clone()).await.unwrap()[0].outcome,
        WebhookOutcome::Duplicate
    );
    publish(&runtime, "b", "A", 3).await;
    publish(&runtime, "c", "B", 4).await;
    publish(&runtime, "d", "", 5).await;
    let snapshot = runtime.snapshot().await.unwrap();
    assert_eq!(
        snapshot.runs.len(),
        2,
        "one unkeyed immediate run and one legacy sibling"
    );
    let detail = runtime
        .webhook_delivery(task.id.clone(), original.id.clone())
        .await
        .unwrap();
    assert_eq!(detail.summary.received_at, original.received_at);
    assert!(detail.prompt.unwrap().contains("\"version\":3"));
    assert_eq!(
        runtime
            .call(store::webhook_windows::flush_due)
            .await
            .unwrap(),
        0
    );
    let first_time = first.created_at.as_millis();
    runtime
        .call(move |db| {
            let expires: i64 =
                db.query_row("SELECT MIN(expires) FROM webhook_replay", [], |r| r.get(0))?;
            let due: i64 =
                db.query_row("SELECT MIN(due) FROM webhook_windows", [], |r| r.get(0))?;
            assert_eq!(expires - first_time, 7 * 24 * 60 * 60 * 1000);
            assert_eq!(due - first_time, 10000);
            db.execute("UPDATE webhook_windows SET due=0", [])?;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        runtime
            .call(store::webhook_windows::flush_due)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        runtime
            .call(store::webhook_windows::flush_due)
            .await
            .unwrap(),
        0
    );
    let snapshot = runtime.snapshot().await.unwrap();
    let grouped: Vec<_> = snapshot
        .runs
        .iter()
        .filter(|r| r.trigger == Some(RunTrigger::WebhookWindow))
        .collect();
    assert_eq!(grouped.len(), 2);
    assert!(grouped
        .iter()
        .all(|run| run.status == RunStatus::AwaitingReview));
    assert!(grouped.iter().any(|r| r.prompt.contains("\"version\":3")));
    assert!(grouped.iter().any(|r| r.prompt.contains("\"version\":4")));
    assert_eq!(
        runtime
            .webhook_delivery(task.id.clone(), original.id)
            .await
            .unwrap()
            .summary
            .outcome,
        WebhookOutcome::AwaitingReview
    );
    runtime
        .call(|db| {
            db.execute("UPDATE webhook_replay SET expires=0", [])?;
            Ok(())
        })
        .await
        .unwrap();
    publish(&runtime, "a", "A", 6).await;
    assert_eq!(
        runtime.webhook_deliveries(task.id.clone()).await.unwrap()[0].outcome,
        WebhookOutcome::WindowOpened
    );
    runtime
        .command(ReactiveCommand::SetAutomationEnabled {
            id: task.id.clone(),
            enabled: false,
        })
        .await
        .unwrap();
    assert_eq!(
        runtime.webhook_deliveries(task.id.clone()).await.unwrap()[0].outcome,
        WebhookOutcome::Rejected
    );
    runtime
        .command(ReactiveCommand::SetAutomationEnabled {
            id: task.id.clone(),
            enabled: true,
        })
        .await
        .unwrap();
    publish(&runtime, "a", "A", 7).await;
    assert_eq!(
        runtime.webhook_deliveries(task.id.clone()).await.unwrap()[0].outcome,
        WebhookOutcome::WindowOpened
    );
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::DeleteSubscription {
                id: task.id.clone(),
            },
        })
        .await
        .unwrap();
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveSubscription {
                subscription: task.clone(),
            },
        })
        .await
        .unwrap();
    publish(&runtime, "a", "A", 7).await;
    assert_eq!(
        runtime.webhook_deliveries(task.id).await.unwrap()[0].outcome,
        WebhookOutcome::WindowOpened
    );
}

#[tokio::test]
async fn full_webhook_queue_accepts_existing_windows_and_retries_failed_new_windows() {
    let (_dir, runtime) = configured(false).await;
    let mut src = source("scans");
    src.kind = SourceKind::Webhook;
    let mut task = subscription(false);
    task.webhook_dedupe = WebhookDedupe {
        replay_key: "{{body.delivery}}".into(),
        coalesce_key: "{{body.issue}}".into(),
        coalesce_window_seconds: None,
    };
    for change in [
        ReactiveChange::SaveSource { source: src },
        ReactiveChange::SaveSubscription {
            subscription: task.clone(),
        },
    ] {
        runtime
            .command(ReactiveCommand::Apply { change })
            .await
            .unwrap();
    }
    let token = runtime.webhook_token("scans").await.unwrap();
    let send = async |identity: String, issue: usize| {
        runtime
            .publish_hook(
                "scans",
                &token,
                &identity,
                json!({"delivery":identity,"issue":issue}),
            )
            .await
    };
    for n in 0..HISTORY_LIMIT {
        send(format!("open-{n}"), n).await.unwrap();
    }
    assert!(matches!(
        send("backlog".into(), HISTORY_LIMIT).await,
        Err(ReactiveError::Conflict(_))
    ));
    send("update".into(), 0).await.unwrap();
    runtime.call(|db| {
        db.execute("UPDATE webhook_windows SET due=0 WHERE rowid=(SELECT MIN(rowid) FROM webhook_windows)",[])?;
        Ok(())
    }).await.unwrap();
    assert_eq!(
        runtime
            .call(store::webhook_windows::flush_due)
            .await
            .unwrap(),
        1
    );
    let run = runtime.snapshot().await.unwrap().runs.remove(0);
    assert!(run.prompt.contains("update"));
    runtime
        .command(ReactiveCommand::CancelRun { id: run.id })
        .await
        .unwrap();
    send("backlog".into(), HISTORY_LIMIT).await.unwrap();
    assert_eq!(
        runtime.webhook_deliveries(task.id).await.unwrap()[0].outcome,
        WebhookOutcome::WindowOpened
    );
}

#[test]
fn desktop_turn_recovery_can_only_claim_the_latest_interrupted_owner_once() {
    use std::io::{ErrorKind, Write};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("turn.lock");
    let id = uuid::Uuid::new_v4();
    let mut owner = super::claim_turn_file(&path, None).unwrap();
    owner.write_all(id.as_bytes()).unwrap();
    assert_eq!(
        super::claim_turn_file(&path, Some(id)).unwrap_err().kind(),
        ErrorKind::WouldBlock
    );
    drop(owner);
    assert_eq!(
        super::claim_turn_file(&path, Some(uuid::Uuid::new_v4()))
            .unwrap_err()
            .kind(),
        ErrorKind::NotFound
    );
    drop(super::claim_turn_file(&path, Some(id)).unwrap());
    assert_eq!(
        super::claim_turn_file(&path, Some(id)).unwrap_err().kind(),
        ErrorKind::NotFound
    );
    std::fs::write(&path, id.as_bytes()).unwrap();
    drop(super::claim_turn_file(&path, None).unwrap());
    assert_eq!(
        super::claim_turn_file(&path, Some(id)).unwrap_err().kind(),
        ErrorKind::NotFound
    );
}
