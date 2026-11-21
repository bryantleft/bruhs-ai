use super::{ReactiveError, ReactiveRuntime};
use bruhs_models::reactive::*;
use serde_json::json;

fn setup() -> AutomationSetup {
    AutomationSetup {
        agent: ReactiveAgent {
            id: "paperwork-agent".into(),
            name: "Paperwork".into(),
            instructions: "Prepare reviewable drafts.".into(),
            memory: String::new(),
            model: "claude-sonnet-4-6".into(),
            project_path: std::env::temp_dir().to_string_lossy().into_owned(),
            enabled: true,
        },
        source: EventSource {
            id: "scans".into(),
            name: "Scanner".into(),
            enabled: true,
            kind: SourceKind::Custom,
        },
        subscription: Subscription {
            id: "paperwork".into(),
            source_id: "scans".into(),
            agent_id: "paperwork-agent".into(),
            instructions: "Extract vendor and total.".into(),
            enabled: true,
            auto_run: true,
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
        },
    }
}

async fn empty_runtime() -> (tempfile::TempDir, ReactiveRuntime) {
    let dir = tempfile::tempdir().unwrap();
    let runtime = ReactiveRuntime::open(dir.path().join("reactive.sqlite3"))
        .await
        .unwrap();
    (dir, runtime)
}

async fn save(runtime: &ReactiveRuntime, automation: AutomationSetup) -> ReactiveSnapshot {
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveAutomation {
                automation: Box::new(automation),
            },
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn a_single_review_activates_the_whole_setup_and_automatic_read_only_drafts() {
    let (_dir, runtime) = empty_runtime().await;
    let automation = setup();
    let proposed = runtime
        .command(ReactiveCommand::Propose {
            change: ReactiveChange::SaveAutomation {
                automation: Box::new(automation.clone()),
            },
        })
        .await
        .unwrap();
    assert!(
        proposed.agents.is_empty()
            && proposed.sources.is_empty()
            && proposed.subscriptions.is_empty()
    );
    assert!(runtime.source_configs().await.unwrap().is_empty());
    let snapshot = runtime
        .command(ReactiveCommand::DecideProposal {
            id: proposed.proposals[0].id.clone(),
            approve: true,
        })
        .await
        .unwrap();
    assert_eq!(
        (
            snapshot.agents.len(),
            snapshot.sources.len(),
            snapshot.subscriptions.len()
        ),
        (1, 1, 1)
    );
    let active = AutomationSetup::from_subscription(&snapshot, &snapshot.subscriptions[0]).unwrap();
    assert_eq!(active, automation);
    assert!(active.enabled() && active.subscription.auto_run);
    assert_eq!(runtime.source_configs().await.unwrap(), vec![active.source]);
    runtime
        .publish("scans", "invoice-1", json!({"total":42}))
        .await
        .unwrap();
    assert_eq!(
        runtime.snapshot().await.unwrap().runs[0].status,
        RunStatus::Queued
    );
}

#[tokio::test]
async fn invalid_members_and_a_disappeared_folder_leave_the_whole_setup_unapplied() {
    let (_dir, runtime) = empty_runtime().await;
    let mut automation = setup();
    automation.subscription.agent_id = "wrong-agent".into();
    assert!(matches!(
        runtime
            .command(ReactiveCommand::Apply {
                change: ReactiveChange::SaveAutomation {
                    automation: Box::new(automation)
                }
            })
            .await,
        Err(ReactiveError::Invalid(_))
    ));
    let folder = tempfile::tempdir().unwrap();
    let mut automation = setup();
    automation.source.kind = SourceKind::Folder {
        path: folder.path().to_string_lossy().into_owned(),
        pattern: "*.pdf".into(),
        recursive: false,
        settle_ms: 1500,
        poll_ms: 1000,
    };
    let proposed = runtime
        .command(ReactiveCommand::Propose {
            change: ReactiveChange::SaveAutomation {
                automation: Box::new(automation.clone()),
            },
        })
        .await
        .unwrap();
    drop(folder);
    assert!(matches!(
        runtime
            .command(ReactiveCommand::DecideProposal {
                id: proposed.proposals[0].id.clone(),
                approve: true
            })
            .await,
        Err(ReactiveError::Invalid(_))
    ));
    assert!(matches!(
        runtime
            .command(ReactiveCommand::Apply {
                change: ReactiveChange::SaveAutomation {
                    automation: Box::new(automation)
                }
            })
            .await,
        Err(ReactiveError::Invalid(_))
    ));
    let snapshot = runtime.snapshot().await.unwrap();
    assert!(
        snapshot.agents.is_empty()
            && snapshot.sources.is_empty()
            && snapshot.subscriptions.is_empty()
    );
    assert_eq!(snapshot.proposals[0].status, ProposalStatus::Pending);
}

#[tokio::test]
async fn bundle_validation_rejects_invalid_timing_without_creating_any_members() {
    let (_dir, runtime) = empty_runtime().await;
    let mut automation = setup();
    automation.subscription.debounce_ms = u64::MAX;
    assert!(matches!(
        runtime
            .command(ReactiveCommand::Propose {
                change: ReactiveChange::SaveAutomation {
                    automation: Box::new(automation)
                }
            })
            .await,
        Err(ReactiveError::Invalid(_))
    ));
    let snapshot = runtime.snapshot().await.unwrap();
    assert!(
        snapshot.agents.is_empty()
            && snapshot.sources.is_empty()
            && snapshot.subscriptions.is_empty()
            && snapshot.proposals.is_empty()
    );
}

#[tokio::test]
async fn pausing_one_automation_keeps_shared_work_and_agent_chat_available() {
    let (_dir, runtime) = empty_runtime().await;
    let mut first = setup();
    first.subscription.auto_run = false;
    save(&runtime, first.clone()).await;
    let mut second = first.clone();
    second.subscription.id = "other-automation".into();
    second.subscription.auto_run = true;
    save(&runtime, second).await;
    runtime
        .publish("scans", "invoice", json!({}))
        .await
        .unwrap();
    let paused = runtime
        .command(ReactiveCommand::SetAutomationEnabled {
            id: first.subscription.id.clone(),
            enabled: false,
        })
        .await
        .unwrap();
    let own = paused
        .runs
        .iter()
        .find(|run| run.subscription_id.as_deref() == Some("paperwork"))
        .unwrap();
    let sibling = paused
        .runs
        .iter()
        .find(|run| run.subscription_id.as_deref() == Some("other-automation"))
        .unwrap();
    assert_eq!(own.status, RunStatus::Cancelled);
    assert_eq!(sibling.status, RunStatus::Queued);
    assert!(paused.agents[0].enabled && paused.sources[0].enabled);
    assert_eq!(runtime.source_configs().await.unwrap().len(), 1);
    runtime
        .command(ReactiveCommand::Chat {
            agent_id: first.agent.id,
            message: "Explain this invoice.".into(),
            run_id: None,
        })
        .await
        .unwrap();
    let enabled = runtime
        .command(ReactiveCommand::SetAutomationEnabled {
            id: "paperwork".into(),
            enabled: true,
        })
        .await
        .unwrap();
    let own = enabled
        .subscriptions
        .iter()
        .find(|subscription| subscription.id == "paperwork")
        .unwrap();
    assert!(own.enabled);
    assert!(
        !own.auto_run,
        "The single toggle must preserve an older review-each-run policy"
    );
    assert_eq!(
        enabled
            .runs
            .iter()
            .find(|run| run.id == sibling.id)
            .unwrap()
            .status,
        RunStatus::Queued
    );
}

#[tokio::test]
async fn activation_never_wakes_another_enabled_subscription_through_shared_dependencies() {
    for dependency in ["agent", "source", "both"] {
        let (_dir, runtime) = empty_runtime().await;
        let first = setup();
        save(&runtime, first.clone()).await;
        let mut other = first.subscription.clone();
        other.id = "other".into();
        runtime
            .command(ReactiveCommand::Apply {
                change: ReactiveChange::SaveSubscription {
                    subscription: other,
                },
            })
            .await
            .unwrap();
        if dependency != "source" {
            let mut agent = first.agent;
            agent.enabled = false;
            runtime
                .command(ReactiveCommand::Apply {
                    change: ReactiveChange::SaveAgent { agent },
                })
                .await
                .unwrap();
        }
        if dependency != "agent" {
            let mut source = first.source;
            source.enabled = false;
            runtime
                .command(ReactiveCommand::Apply {
                    change: ReactiveChange::SaveSource { source },
                })
                .await
                .unwrap();
        }
        let before = runtime.snapshot().await.unwrap();
        let error = runtime
            .command(ReactiveCommand::SetAutomationEnabled {
                id: "paperwork".into(),
                enabled: true,
            })
            .await
            .unwrap_err();
        assert!(
            matches!(error, ReactiveError::Conflict(ref message) if message.contains("Advanced"))
        );
        assert_eq!(runtime.snapshot().await.unwrap(), before);
    }
}

#[tokio::test]
async fn enabling_dedicated_dependencies_is_atomic_and_preserves_the_existing_run_policy() {
    let (_dir, runtime) = empty_runtime().await;
    let mut automation = setup();
    automation.agent.enabled = false;
    automation.source.enabled = false;
    automation.subscription.enabled = false;
    automation.subscription.auto_run = false;
    save(&runtime, automation).await;
    let snapshot = runtime
        .command(ReactiveCommand::SetAutomationEnabled {
            id: "paperwork".into(),
            enabled: true,
        })
        .await
        .unwrap();
    let automation =
        AutomationSetup::from_subscription(&snapshot, &snapshot.subscriptions[0]).unwrap();
    assert!(automation.enabled());
    assert!(!automation.subscription.auto_run);
    runtime.publish("scans", "one", json!({})).await.unwrap();
    assert_eq!(
        runtime.snapshot().await.unwrap().runs[0].status,
        RunStatus::AwaitingReview
    );
}

#[tokio::test]
async fn scheduled_task_edit_and_activation_commit_together() {
    let (dir, runtime) = empty_runtime().await;
    let mut task = setup();
    task.source.kind = SourceKind::Webhook;
    let set = |automation: AutomationSetup, enabled| ReactiveCommand::SaveScheduledTask { destination: None,
        automation: Box::new(automation),
        enabled,
        signing_secret: None,
    };
    let saved = runtime.command(set(task.clone(), false)).await.unwrap();
    assert!(!saved.subscriptions[0].enabled);
    assert!(saved.sources[0].enabled && saved.agents[0].enabled);
    drop(runtime);

    let runtime = ReactiveRuntime::open(dir.path().join("reactive.sqlite3"))
        .await
        .unwrap();
    let token = runtime.webhook_token(&task.source.id).await.unwrap();
    runtime
        .publish_hook(&task.source.id, &token, "paused", serde_json::json!({}))
        .await
        .unwrap();
    assert!(runtime.snapshot().await.unwrap().runs.is_empty());
    task.subscription.enabled = false;
    task.subscription.instructions = "Prepare the new review".into();
    let saved = runtime.command(set(task.clone(), true)).await.unwrap();
    assert!(saved.subscriptions[0].enabled);
    assert_eq!(
        saved.subscriptions[0].instructions,
        "Prepare the new review"
    );
    runtime
        .publish_hook(&task.source.id, &token, "active", serde_json::json!({}))
        .await
        .unwrap();
    let queued = runtime.snapshot().await.unwrap().runs[0].clone();
    task.source.enabled = false;
    let retried = runtime.command(set(task.clone(), true)).await.unwrap();
    assert_eq!(retried.runs[0], queued);
    assert!(retried.sources[0].enabled);
    task.source.enabled = true;

    let mut sibling = saved.subscriptions[0].clone();
    sibling.id = "shared".into();
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveSubscription {
                subscription: sibling,
            },
        })
        .await
        .unwrap();
    task.subscription.task_title = Some("  Morning receipts  ".into());
    let renamed = runtime.command(set(task.clone(), true)).await.unwrap();
    assert_eq!(renamed.runs[0], queued);
    for sub in &renamed.subscriptions {
        let title = if sub.id == task.subscription.id { "Morning receipts" } else { "Paperwork" };
        assert_eq!(sub.title(&renamed.agents[0]), title);
    }
    assert_eq!(renamed.agents[0], task.agent);
    task.subscription.task_title = None;
    let legacy_save = runtime.command(set(task.clone(), true)).await.unwrap();
    assert_eq!(legacy_save.subscriptions, renamed.subscriptions);
    assert_eq!(legacy_save.runs[0], queued);
    task.subscription.task_title = Some("   ".into());
    assert!(runtime.command(set(task.clone(), true)).await.unwrap_err().to_string().contains("Task name"));
    task.subscription.task_title = None;

    task.agent.enabled = false;
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveAgent {
                agent: task.agent.clone(),
            },
        })
        .await
        .unwrap();
    task.subscription.instructions = "This edit must roll back".into();
    let error = runtime.command(set(task.clone(), true)).await.unwrap_err();
    assert!(error
        .to_string()
        .contains("also activate another automation"));
    let snapshot = runtime.snapshot().await.unwrap();
    assert!(!snapshot.agents[0].enabled);
    assert!(snapshot
        .subscriptions
        .iter()
        .all(|s| s.enabled && s.instructions == "Prepare the new review"));

    task.source.kind = SourceKind::Custom;
    assert!(runtime.command(set(task, false)).await.is_err());
}

#[tokio::test]
async fn shared_component_definitions_cannot_be_silently_changed_by_a_bundle() {
    let (_dir, runtime) = empty_runtime().await;
    let automation = setup();
    save(&runtime, automation.clone()).await;
    let mut shared = automation.clone();
    shared.subscription.id = "other".into();
    save(&runtime, shared.clone()).await;
    for component in ["agent", "source"] {
        let mut changed = shared.clone();
        if component == "agent" {
            changed.agent.instructions = "Changed instructions".into();
        } else {
            changed.source.name = "Renamed shared source".into();
        }
        assert!(matches!(
            runtime
                .command(ReactiveCommand::Apply {
                    change: ReactiveChange::SaveAutomation {
                        automation: Box::new(changed)
                    }
                })
                .await,
            Err(ReactiveError::Conflict(_))
        ));
    }
    let snapshot = runtime.snapshot().await.unwrap();
    assert_eq!(snapshot.agents, vec![automation.agent]);
    assert_eq!(snapshot.sources, vec![automation.source]);
    assert_eq!(snapshot.subscriptions.len(), 2);
}

#[test]
fn existing_snapshot_and_individual_change_wire_shapes_remain_compatible() {
    let automation = setup();
    let mut legacy = serde_json::to_value(ReactiveSnapshot::default()).unwrap();
    legacy.as_object_mut().unwrap().remove("source_statuses");
    let snapshot: ReactiveSnapshot = serde_json::from_value(legacy).unwrap();
    assert_eq!(snapshot, ReactiveSnapshot::default());
    let change: ReactiveChange =
        serde_json::from_value(json!({"type":"save_agent","agent":automation.agent})).unwrap();
    assert!(matches!(change, ReactiveChange::SaveAgent { .. }));
    let mut incomplete = ReactiveSnapshot::default();
    incomplete
        .subscriptions
        .push(automation.subscription.clone());
    assert!(AutomationSetup::from_subscription(&incomplete, &automation.subscription).is_none());
}

#[tokio::test]
async fn one_time_schedule_queues_one_run_across_database_reopen() {
    let (dir, runtime) = empty_runtime().await;
    let mut task = setup();
    let due_at = bruhs_models::EpochMs::now();
    task.source.kind = SourceKind::Once { due_at };
    task.subscription.auto_run = false;
    let initial = save(&runtime, task.clone()).await;
    assert_eq!(initial.next_run_by_source.unwrap()[&task.source.id], due_at);
    let client = reqwest::Client::new();
    super::sources::poll_once(&runtime, &task.source, &client)
        .await
        .unwrap();
    drop(runtime);
    let reopened = ReactiveRuntime::open(dir.path().join("reactive.sqlite3"))
        .await
        .unwrap();
    super::sources::poll_once(&reopened, &task.source, &client)
        .await
        .unwrap();
    let snapshot = reopened.snapshot().await.unwrap();
    assert!(snapshot.next_run_by_source.unwrap().is_empty());
    assert_eq!(snapshot.events.len(), 1);
    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(
        snapshot.runs[0].subscription_id.as_ref(),
        Some(&task.subscription.id)
    );
}

#[tokio::test]
async fn webhook_security_survives_restart_and_expires_from_trusted_arrival() {
    use bruhs_models::EpochMs;
    const BODY: &str = "{ \"fixture\": true }";
    const ORIGINAL: &str = "8WXw8yasdfbIup88Pro+U1z5zaZm7jo+BI41BM+2izA=";
    const ROTATED: &str = "qED+Sau62g2R8yUsSR7EULRtaOicq3TcTt6AwZASiYc=";
    let (dir, runtime) = empty_runtime().await;
    let mut task = setup();
    task.source.kind = SourceKind::Webhook;
    task.subscription.webhook_security = WebhookSecurity {
        signature: Some(WebhookSignature {
            header: "x-check".into(),
            prefix: String::new(),
            encoding: WebhookSignatureEncoding::Base64,
        }),
        max_delivery_age_minutes: Some(1),
    };
    let save_task =
        |task: &AutomationSetup, secret: Option<&str>| ReactiveCommand::SaveScheduledTask { destination: None,
            automation: Box::new(task.clone()),
            enabled: true,
            signing_secret: secret.map(|s| WebhookSigningSecret(s.into())),
        };
    assert!(runtime.command(save_task(&task, None)).await.is_err());
    runtime
        .command(save_task(&task, Some("fixture-signing-key")))
        .await
        .unwrap();
    runtime.command(save_task(&task, None)).await.unwrap();
    let mut invalid = task.clone();
    invalid
        .subscription
        .webhook_security
        .max_delivery_age_minutes = Some(0);
    assert!(runtime
        .command(save_task(&invalid, Some("uncommitted replacement")))
        .await
        .is_err());
    drop(runtime);
    let runtime = ReactiveRuntime::open(dir.path().join("reactive.sqlite3"))
        .await
        .unwrap();
    let token = runtime.webhook_token("scans").await.unwrap();
    let deliver = |id: &'static str, values: Vec<&str>, received| {
        let request = WebhookMetadata {
            raw_body: Some(BODY.into()),
            headers: [
                (
                    "x-check".into(),
                    values.into_iter().map(String::from).collect(),
                ),
                ("x-received-at".into(), vec!["2099-12-01".into()]),
            ]
            .into(),
            ..Default::default()
        };
        runtime.publish_hook_received(
            "scans",
            &token,
            id,
            json!({"fixture":true}),
            request,
            received,
        )
    };
    let old = EpochMs::new(EpochMs::now().as_millis() - 61_000).unwrap();
    deliver("same", vec![ORIGINAL], old).await.unwrap();
    assert!(runtime.snapshot().await.unwrap().runs.is_empty());
    let history = runtime
        .webhook_deliveries(task.subscription.id.clone())
        .await
        .unwrap();
    assert_eq!(history[0].outcome, WebhookOutcome::Expired);
    assert_eq!(history[0].received_at, old);
    assert!(deliver("same", vec![ORIGINAL, ORIGINAL], EpochMs::now())
        .await
        .is_err());
    runtime
        .command(ReactiveCommand::Publish {
            source_id: "scans".into(),
            key: "same".into(),
            payload: json!({"fixture":true}),
        })
        .await
        .unwrap();
    assert!(runtime.snapshot().await.unwrap().runs.is_empty());
    deliver("same", vec![ORIGINAL], EpochMs::now())
        .await
        .unwrap();
    runtime
        .command(save_task(&task, Some("replacement-key")))
        .await
        .unwrap();
    assert!(deliver("two", vec![ORIGINAL], EpochMs::now())
        .await
        .is_err());
    deliver("two", vec![ROTATED], EpochMs::now()).await.unwrap();
    let mut disabled = task.clone();
    disabled.subscription.webhook_security.signature = None;
    runtime.command(save_task(&disabled, None)).await.unwrap();
    assert!(runtime.command(save_task(&task, None)).await.is_err());
    assert_eq!(runtime.snapshot().await.unwrap().runs.len(), 2);
}
