//! Outbox, agent memory, follow-ups, grouping, lanes, restarts and rollover.

use super::{store, ReactiveError, ReactiveRuntime};
use bruhs_models::reactive::*;
use serde_json::json;
use std::sync::{Arc, Mutex};

fn agent(id: &str) -> ReactiveAgent {
    ReactiveAgent {
        id: id.into(),
        name: format!("Agent {id}"),
        instructions: "Prepare drafts for review.".into(),
        memory: String::new(),
        model: "claude-sonnet-4-6".into(),
        project_path: std::env::temp_dir().to_string_lossy().into_owned(),
        enabled: true,
    }
}

fn subscription(id: &str, agent_id: &str, auto_run: bool, on_busy: BusyPolicy) -> Subscription {
    Subscription {
        id: id.into(),
        source_id: "inbox".into(),
        agent_id: agent_id.into(),
        instructions: "Summarize the event.".into(),
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
        on_busy,
    }
}

async fn runtime_with(
    agents: &[&str],
    on_busy: BusyPolicy,
) -> (tempfile::TempDir, ReactiveRuntime) {
    let dir = tempfile::tempdir().unwrap();
    let runtime = ReactiveRuntime::open(dir.path().join("reactive.sqlite3"))
        .await
        .unwrap();
    let mut changes = vec![ReactiveChange::SaveSource {
        source: EventSource {
            id: "inbox".into(),
            name: "Inbox".into(),
            enabled: true,
            kind: SourceKind::Custom,
        },
    }];
    for id in agents {
        changes.push(ReactiveChange::SaveAgent { agent: agent(id) });
        changes.push(ReactiveChange::SaveSubscription {
            subscription: subscription(&format!("sub-{id}"), id, true, on_busy),
        });
    }
    for change in changes {
        runtime
            .command(ReactiveCommand::Apply { change })
            .await
            .unwrap();
    }
    (dir, runtime)
}

async fn claim_one(runtime: &ReactiveRuntime) -> Option<store::Claim> {
    runtime.call(store::claim_next).await.unwrap()
}

async fn finish(runtime: &ReactiveRuntime, id: &str, answer: &str, tokens: Option<u64>) {
    let (id, answer) = (id.to_owned(), answer.to_owned());
    runtime
        .call(move |connection| {
            store::finish_run(
                connection,
                &id,
                RunStatus::Completed,
                Some(answer),
                None,
                tokens,
            )
        })
        .await
        .unwrap();
}

async fn record_session(runtime: &ReactiveRuntime, id: &str) {
    let id = id.to_owned();
    runtime
        .call(move |connection| store::record_session(connection, &id, "s1", "claude-sonnet-4-6"))
        .await
        .unwrap();
}

async fn run(runtime: &ReactiveRuntime, id: &str) -> ReactiveRun {
    let id = id.to_owned();
    runtime
        .call(move |connection| store::run_record(connection, &id))
        .await
        .unwrap()
}

fn report(json: &str) -> String {
    format!("Done.\n\n```bruhs-report\n{json}\n```")
}

#[tokio::test]
async fn a_completed_report_writes_outbox_memory_and_follow_ups_with_its_receipt() {
    let (_dir, runtime) = runtime_with(&["clerk"], BusyPolicy::Queue).await;
    runtime
        .publish("inbox", "one", json!({"total": 42}))
        .await
        .unwrap();
    let claim = claim_one(&runtime).await.unwrap();
    assert!(claim.run.fresh_session);
    assert!(claim.context.contains("new conversation"));
    record_session(&runtime, &claim.run.id).await;
    finish(
        &runtime,
        &claim.run.id,
        &report(r#"{"outcome":"notify","title":"Invoice due","message":"ACME 42 EUR due Friday.","remember":[{"key":"acme","kind":"note","text":"ACME bills monthly"},{"key":"invoice-42","kind":"progress","text":"Waiting for payment"}],"follow_ups":[{"key":"acme-due","in_minutes":60,"note":"Check whether it was paid"}]}"#),
        None,
    )
    .await;
    let snapshot = runtime.snapshot().await.unwrap();
    let done = snapshot
        .runs
        .iter()
        .find(|item| item.id == claim.run.id)
        .unwrap();
    assert_eq!(done.status, RunStatus::Completed);
    assert_eq!(done.result.as_deref(), Some("Done."));
    assert!(done.report.is_some());
    assert_eq!(snapshot.unread_count(), 1);
    let update = &snapshot.outbox[0];
    assert_eq!(update.kind, OutboxKind::Notify);
    assert_eq!(update.title, "Invoice due");
    assert_eq!(update.system, SystemDelivery::NotRequested);
    assert_eq!(snapshot.memories.len(), 2);
    assert!(snapshot
        .memories
        .iter()
        .all(|entry| entry.run_id.as_deref() == Some(claim.run.id.as_str())));
    let follow_up = &snapshot.follow_ups[0];
    assert_eq!(follow_up.key, "acme-due");
    assert_eq!(follow_up.status, FollowUpStatus::Pending);
    assert!(follow_up.auto_run);

    // A resumed session does not repeat memory the agent wrote itself.
    runtime.publish("inbox", "two", json!({})).await.unwrap();
    let next = claim_one(&runtime).await.unwrap();
    assert!(!next.run.fresh_session);
    assert!(next.context.is_empty(), "{}", next.context);
    finish(&runtime, &next.run.id, "No report", None).await;
    // An owner edit is repeated to the running conversation once.
    runtime
        .command(ReactiveCommand::EditMemory {
            agent_id: "clerk".into(),
            key: "acme".into(),
            kind: MemoryKind::Note,
            text: "ACME bills quarterly".into(),
        })
        .await
        .unwrap();
    runtime.publish("inbox", "three", json!({})).await.unwrap();
    let third = claim_one(&runtime).await.unwrap();
    assert!(third.context.contains("ACME bills quarterly"));
    assert!(third.context.contains("changed since your last run"));
}

#[tokio::test]
async fn runs_without_a_report_stay_quiet_but_drafts_ask_for_review() {
    let (_dir, runtime) = runtime_with(&["clerk"], BusyPolicy::Queue).await;
    runtime.publish("inbox", "one", json!({})).await.unwrap();
    let first = claim_one(&runtime).await.unwrap();
    finish(&runtime, &first.run.id, "Nothing changed.", None).await;
    assert!(runtime.snapshot().await.unwrap().outbox.is_empty());

    runtime.publish("inbox", "two", json!({})).await.unwrap();
    let second = claim_one(&runtime).await.unwrap();
    finish(
        &runtime,
        &second.run.id,
        r#"{"summary":"Add a task","operations":[{"type":"create_task","title":"Pay ACME"}]}"#,
        None,
    )
    .await;
    let snapshot = runtime.snapshot().await.unwrap();
    assert_eq!(snapshot.outbox.len(), 1);
    assert_eq!(snapshot.outbox[0].kind, OutboxKind::Ask);
    assert_eq!(snapshot.outbox[0].message, "Add a task");
    // A malformed block is reported, never applied.
    runtime.publish("inbox", "three", json!({})).await.unwrap();
    let third = claim_one(&runtime).await.unwrap();
    finish(
        &runtime,
        &third.run.id,
        "Body\n```bruhs-report\n{oops}\n```",
        None,
    )
    .await;
    let done = run(&runtime, &third.run.id).await;
    assert!(done.report_warning.unwrap().contains("not valid"));
    assert_eq!(done.result.as_deref(), Some("Body"));
}

#[tokio::test]
async fn queued_events_for_one_agent_run_together_and_agents_get_their_own_lanes() {
    let (_dir, runtime) = runtime_with(&["a", "b", "c", "d"], BusyPolicy::Queue).await;
    for key in ["one", "two", "three"] {
        runtime
            .publish("inbox", key, json!({"key": key}))
            .await
            .unwrap();
    }
    // Each of four agents has three queued runs; three lanes fill, one per agent.
    let mut claimed = Vec::new();
    while let Some(claim) = claim_one(&runtime).await {
        claimed.push(claim);
    }
    assert_eq!(claimed.len(), MAX_LANES);
    let agents: std::collections::HashSet<_> =
        claimed.iter().map(|claim| claim.agent.id.clone()).collect();
    assert_eq!(agents.len(), MAX_LANES);
    for claim in &claimed {
        assert_eq!(claim.run.merged_runs.len(), 2);
        assert!(claim.run.prompt.starts_with("3 items arrived"));
        for merged in &claim.run.merged_runs {
            let merged = run(&runtime, merged).await;
            assert_eq!(merged.status, RunStatus::Merged);
            assert_eq!(merged.merged_into.as_deref(), Some(claim.run.id.as_str()));
        }
    }
    // A finished lane frees room for the waiting agent.
    finish(&runtime, &claimed[0].run.id, "ok", None).await;
    let fourth = claim_one(&runtime).await.unwrap();
    assert!(!agents.contains(&fourth.agent.id));
}

#[tokio::test]
async fn a_restart_subscription_replaces_the_running_turn_with_all_its_events() {
    let (_dir, runtime) = runtime_with(&["clerk"], BusyPolicy::Restart).await;
    runtime
        .publish("inbox", "one", json!({"n": 1}))
        .await
        .unwrap();
    let first = claim_one(&runtime).await.unwrap();
    runtime
        .publish("inbox", "two", json!({"n": 2}))
        .await
        .unwrap();
    let stopped = run(&runtime, &first.run.id).await;
    assert_eq!(stopped.status, RunStatus::Merged);
    let next = claim_one(&runtime).await.unwrap();
    assert_eq!(next.run.merged_runs, vec![first.run.id.clone()]);
    assert!(next.run.prompt.contains("\"n\":1") && next.run.prompt.contains("\"n\":2"));
    // A late result from the replaced turn cannot complete it.
    finish(&runtime, &first.run.id, "late", None).await;
    assert_eq!(run(&runtime, &first.run.id).await.status, RunStatus::Merged);
}

#[tokio::test]
async fn due_follow_ups_wake_the_agent_and_cancelled_ones_do_not() {
    let (_dir, runtime) = runtime_with(&["clerk"], BusyPolicy::Queue).await;
    runtime.publish("inbox", "one", json!({})).await.unwrap();
    let first = claim_one(&runtime).await.unwrap();
    let past = "2020-01-01T00:00:00Z";
    finish(
        &runtime,
        &first.run.id,
        &report(&format!(r#"{{"follow_ups":[{{"key":"now","at":"{past}","note":"Check now"}},{{"key":"later","in_minutes":600,"note":"Check later"}},{{"key":"far","in_minutes":900000,"note":"Too far"}}]}}"#)),
        None,
    )
    .await;
    let done = run(&runtime, &first.run.id).await;
    assert!(done.report_warning.unwrap().contains("more than a year"));
    assert_eq!(runtime.fire_follow_ups().await.unwrap(), 1);
    let snapshot = runtime.snapshot().await.unwrap();
    let woke = snapshot
        .runs
        .iter()
        .find(|item| matches!(item.trigger, Some(RunTrigger::FollowUp { .. })))
        .unwrap();
    assert_eq!(woke.status, RunStatus::Queued);
    assert!(woke.prompt.contains("Check now"));
    let later = snapshot
        .follow_ups
        .iter()
        .find(|item| item.key == "later")
        .unwrap();
    runtime
        .command(ReactiveCommand::CancelFollowUp {
            id: later.id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(runtime.fire_follow_ups().await.unwrap(), 0);
}

#[tokio::test]
async fn answering_a_question_resumes_the_agent_and_resolves_it_once() {
    let (_dir, runtime) = runtime_with(&["clerk"], BusyPolicy::Queue).await;
    runtime.publish("inbox", "one", json!({})).await.unwrap();
    let first = claim_one(&runtime).await.unwrap();
    finish(
        &runtime,
        &first.run.id,
        &report(r#"{"outcome":"ask","title":"Which card?","message":"Pay ACME with the work or personal card?"}"#),
        None,
    )
    .await;
    let question = runtime.snapshot().await.unwrap().outbox[0].clone();
    assert!(question.is_open_question());
    runtime
        .command(ReactiveCommand::AnswerOutbox {
            id: question.id.clone(),
            message: "Work card.".into(),
        })
        .await
        .unwrap();
    let snapshot = runtime.snapshot().await.unwrap();
    let resolved = snapshot
        .outbox
        .iter()
        .find(|item| item.id == question.id)
        .unwrap();
    assert_eq!(resolved.answer.as_deref(), Some("Work card."));
    assert!(!resolved.is_unread());
    let reply = claim_one(&runtime).await.unwrap();
    assert!(matches!(reply.run.trigger, Some(RunTrigger::Answer { .. })));
    assert!(reply.run.prompt.contains("Work card."));
    assert!(matches!(
        runtime
            .command(ReactiveCommand::AnswerOutbox {
                id: question.id,
                message: "Again".into(),
            })
            .await,
        Err(ReactiveError::Conflict(_))
    ));
}

#[tokio::test]
async fn full_memory_keeps_forgets_and_queues_one_tidy_up() {
    let (_dir, runtime) = runtime_with(&["clerk"], BusyPolicy::Queue).await;
    let entries = (0..MAX_MEMORY_ENTRIES)
        .map(|index| format!(r#"{{"key":"k{index}","text":"{}"}}"#, "x".repeat(250)))
        .collect::<Vec<_>>();
    // Fill memory across runs; the fourth batch passes the character budget.
    for (round, chunk) in entries.chunks(10).enumerate() {
        runtime
            .publish("inbox", &format!("fill-{round}"), json!({}))
            .await
            .unwrap();
        let claim = claim_one(&runtime).await.unwrap();
        finish(
            &runtime,
            &claim.run.id,
            &report(&format!(r#"{{"remember":[{}]}}"#, chunk.join(","))),
            None,
        )
        .await;
    }
    let stored = runtime.snapshot().await.unwrap().memories.len();
    assert_eq!(stored, 30, "the batch that overflows is not saved");
    let snapshot = runtime.snapshot().await.unwrap();
    let last = snapshot
        .runs
        .iter()
        .find(|item| item.report_warning.is_some())
        .unwrap();
    assert!(last
        .report_warning
        .as_ref()
        .unwrap()
        .contains("Memory is full"));
    let tidy = snapshot
        .runs
        .iter()
        .filter(|item| item.trigger == Some(RunTrigger::Consolidation))
        .count();
    assert_eq!(tidy, 1);
    assert!(matches!(
        runtime
            .command(ReactiveCommand::ConsolidateMemory {
                agent_id: "clerk".into()
            })
            .await,
        Err(ReactiveError::Conflict(_))
    ));
    runtime
        .command(ReactiveCommand::ForgetMemory {
            agent_id: "clerk".into(),
            key: "k0".into(),
        })
        .await
        .unwrap();
    assert_eq!(runtime.snapshot().await.unwrap().memories.len(), stored - 1);
}

#[tokio::test]
async fn changed_instructions_or_unstamped_sessions_start_fresh() {
    let (_dir, runtime) = runtime_with(&["clerk"], BusyPolicy::Queue).await;
    runtime.publish("inbox", "one", json!({})).await.unwrap();
    let first = claim_one(&runtime).await.unwrap();
    record_session(&runtime, &first.run.id).await;
    finish(&runtime, &first.run.id, "ok", None).await;
    // Same instructions: the session resumes.
    runtime.publish("inbox", "two", json!({})).await.unwrap();
    let second = claim_one(&runtime).await.unwrap();
    assert_eq!(second.run.session_id.as_deref(), Some("s1"));
    finish(&runtime, &second.run.id, "ok", None).await;
    // A session saved before stamps existed (or under other instructions) is not resumed.
    runtime
        .call(|connection| {
            store::transaction(connection, |connection| {
                connection.execute(
                    "UPDATE reactive_session_stamps SET stamp='older-protocol'",
                    [],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap();
    runtime.publish("inbox", "three", json!({})).await.unwrap();
    let third = claim_one(&runtime).await.unwrap();
    assert!(third.run.session_id.is_none());
    assert!(third.run.fresh_session);
    assert!(third.context.contains("new conversation"));
}

#[tokio::test]
async fn a_large_context_starts_the_next_run_in_a_fresh_session() {
    let (_dir, runtime) = runtime_with(&["clerk"], BusyPolicy::Queue).await;
    runtime.publish("inbox", "one", json!({})).await.unwrap();
    let first = claim_one(&runtime).await.unwrap();
    let id = first.run.id.clone();
    runtime
        .call(move |connection| store::record_session(connection, &id, "s1", "claude-sonnet-4-6"))
        .await
        .unwrap();
    finish(
        &runtime,
        &first.run.id,
        "ok",
        Some(ROLLOVER_CONTEXT_TOKENS - 1),
    )
    .await;
    runtime.publish("inbox", "two", json!({})).await.unwrap();
    let second = claim_one(&runtime).await.unwrap();
    assert_eq!(second.run.session_id.as_deref(), Some("s1"));
    finish(
        &runtime,
        &second.run.id,
        "ok",
        Some(ROLLOVER_CONTEXT_TOKENS),
    )
    .await;
    runtime.publish("inbox", "three", json!({})).await.unwrap();
    let third = claim_one(&runtime).await.unwrap();
    assert!(third.run.session_id.is_none());
    assert!(third.run.fresh_session);
    assert_eq!(
        run(&runtime, &second.run.id).await.context_tokens,
        Some(ROLLOVER_CONTEXT_TOKENS)
    );
}

#[tokio::test]
async fn draft_decisions_wake_the_agent_once_per_outcome() {
    let (_dir, runtime) = runtime_with(&["clerk"], BusyPolicy::Queue).await;
    runtime.publish("inbox", "one", json!({})).await.unwrap();
    let first = claim_one(&runtime).await.unwrap();
    finish(
        &runtime,
        &first.run.id,
        r#"{"summary":"Add a task","operations":[{"type":"create_task","title":"Pay ACME"}]}"#,
        None,
    )
    .await;
    let decision = ReactiveCommand::DraftOutcome {
        run_id: first.run.id.clone(),
        workflow_id: "workflow-1".into(),
        outcome: DraftOutcomeKind::Applied,
        applied_steps: 1,
        total_steps: 1,
    };
    runtime.command(decision.clone()).await.unwrap();
    runtime.command(decision).await.unwrap();
    let snapshot = runtime.snapshot().await.unwrap();
    let woke: Vec<_> = snapshot
        .runs
        .iter()
        .filter(|item| matches!(item.trigger, Some(RunTrigger::DraftOutcome { .. })))
        .collect();
    assert_eq!(woke.len(), 1);
    assert!(woke[0].prompt.contains("“Add a task”"));
    assert!(woke[0].prompt.contains("approved and applied"));
}

#[tokio::test]
async fn system_notifications_follow_settings_quiet_hours_and_reads() {
    let (_dir, runtime) = runtime_with(&["clerk"], BusyPolicy::Queue).await;
    let shown = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let capture = shown.clone();
    runtime.set_notifier(Arc::new(move |title: &str, body: &str| {
        capture.lock().unwrap().push((title.into(), body.into()));
        Ok(())
    }));
    let notify = |key: &'static str| {
        let runtime = runtime.clone();
        async move {
            runtime.publish("inbox", key, json!({})).await.unwrap();
            let claim = claim_one(&runtime).await.unwrap();
            finish(
                &runtime,
                &claim.run.id,
                &report(&format!(
                    r#"{{"outcome":"notify","title":"Update {key}","message":"Details"}}"#
                )),
                None,
            )
            .await;
        }
    };
    // Off by default: nothing is delivered.
    notify("quiet").await;
    assert_eq!(runtime.deliver_notifications().await.unwrap(), 0);

    // Quiet hours covering the whole day hold delivery.
    let mut settings = NotificationSettings {
        system_notify: true,
        system_ask: true,
        quiet_hours: Some(QuietHours {
            start: "00:00".into(),
            end: "23:59".into(),
            time_zone: "UTC".into(),
        }),
    };
    runtime
        .command(ReactiveCommand::SetNotificationSettings {
            settings: settings.clone(),
        })
        .await
        .unwrap();
    notify("one").await;
    notify("two").await;
    assert_eq!(runtime.deliver_notifications().await.unwrap(), 0);
    assert!(shown.lock().unwrap().is_empty());

    // Outside quiet hours a burst becomes one digest.
    settings.quiet_hours = None;
    runtime
        .command(ReactiveCommand::SetNotificationSettings { settings })
        .await
        .unwrap();
    assert_eq!(runtime.deliver_notifications().await.unwrap(), 2);
    assert_eq!(shown.lock().unwrap()[0].0, "2 agent updates");
    let snapshot = runtime.snapshot().await.unwrap();
    assert!(snapshot
        .outbox
        .iter()
        .filter(|item| item.title != "Update quiet")
        .all(|item| matches!(item.system, SystemDelivery::Delivered { .. })));

    // An update read in the app first is never delivered.
    notify("three").await;
    runtime
        .command(ReactiveCommand::MarkOutboxRead { ids: Vec::new() })
        .await
        .unwrap();
    assert_eq!(runtime.deliver_notifications().await.unwrap(), 0);
    assert_eq!(runtime.snapshot().await.unwrap().unread_count(), 0);

    // A refused notification is recorded on the item.
    runtime.set_notifier(Arc::new(|_: &str, _: &str| Err("denied".into())));
    notify("four").await;
    runtime.deliver_notifications().await.unwrap();
    let failed = runtime
        .snapshot()
        .await
        .unwrap()
        .outbox
        .into_iter()
        .find(|item| item.title == "Update four")
        .unwrap();
    assert_eq!(
        failed.system,
        SystemDelivery::Failed {
            error: "denied".into()
        }
    );
}

#[tokio::test]
async fn calendar_sources_and_quiet_hours_require_real_time_zones() {
    let (_dir, runtime) = runtime_with(&["clerk"], BusyPolicy::Queue).await;
    let calendar = |time_zone: &str| ReactiveCommand::Apply {
        change: ReactiveChange::SaveSource {
            source: EventSource {
                id: "mornings".into(),
                name: "Mornings".into(),
                enabled: true,
                kind: SourceKind::Calendar {
                    times: vec!["08:00".into()],
                    weekdays: vec![1, 2, 3, 4, 5],
                    time_zone: time_zone.into(),
                },
            },
        },
    };
    runtime.command(calendar("Europe/Paris")).await.unwrap();
    assert!(matches!(
        runtime.command(calendar("Nowhere/Land")).await,
        Err(ReactiveError::Invalid(_))
    ));
    assert!(matches!(
        runtime
            .command(ReactiveCommand::SetNotificationSettings {
                settings: NotificationSettings {
                    system_notify: true,
                    system_ask: false,
                    quiet_hours: Some(QuietHours {
                        start: "22:00".into(),
                        end: "7".into(),
                        time_zone: "UTC".into(),
                    }),
                },
            })
            .await,
        Err(ReactiveError::Invalid(_))
    ));
}

#[tokio::test]
async fn scheduled_thread_modes_keep_queued_runs_separate_and_resume_only_their_own_session() {
    let (dir, runtime) = runtime_with(&["assistant"], BusyPolicy::Queue).await;
    let mut scheduled = subscription("scheduled", "assistant", true, BusyPolicy::Queue);
    scheduled.source_id = "hook".into();
    scheduled.task_conversation = TaskConversation::Continue;
    let save = |sub: Subscription| ReactiveCommand::Apply {
        change: ReactiveChange::SaveSubscription { subscription: sub },
    };
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveSource {
                source: EventSource {
                    id: "hook".into(),
                    name: "Scheduled webhook".into(),
                    enabled: true,
                    kind: SourceKind::Webhook,
                },
            },
        })
        .await
        .unwrap();
    runtime.command(save(scheduled.clone())).await.unwrap();
    for key in ["first", "second"] {
        runtime
            .publish("hook", key, json!({"key":key}))
            .await
            .unwrap();
    }
    let first = claim_one(&runtime).await.unwrap();
    assert!(first.run.fresh_session && first.run.merged_runs.is_empty());
    let dedicated = first.run.thread_id.clone();
    record_session(&runtime, &first.run.id).await;
    finish(&runtime, &first.run.id, "first done", None).await;
    drop(runtime);
    let runtime = ReactiveRuntime::open(dir.path().join("reactive.sqlite3"))
        .await
        .unwrap();
    let second = claim_one(&runtime).await.unwrap();
    assert_eq!(second.run.thread_id, dedicated);
    assert_eq!(second.run.session_id.as_deref(), Some("s1"));
    finish(&runtime, &second.run.id, "second done", None).await;

    for (mode, keys) in [
        (TaskConversation::FreshSession, ["third", "fourth"]),
        (TaskConversation::NewThread, ["fifth", "sixth"]),
    ] {
        scheduled.task_conversation = mode;
        runtime.command(save(scheduled.clone())).await.unwrap();
        let mut ids = Vec::new();
        for key in keys {
            runtime.publish("hook", key, json!({})).await.unwrap();
        }
        for _ in keys {
            let claim = claim_one(&runtime).await.unwrap();
            assert!(
                claim.run.fresh_session
                    && claim.run.session_id.is_none()
                    && claim.run.merged_runs.is_empty()
            );
            ids.push(claim.run.thread_id.clone());
            record_session(&runtime, &claim.run.id).await;
            finish(&runtime, &claim.run.id, "done", None).await;
        }
        assert_eq!(ids[0] == ids[1], mode == TaskConversation::FreshSession);
        let reply = ReactiveCommand::ChatInTaskThread {
            task_id: scheduled.id.clone(),
            thread_id: dedicated.clone(),
            message: "Inspect the result".into(),
        };
        let saved = runtime.snapshot().await.unwrap();
        if mode == TaskConversation::FreshSession {
            assert_eq!(saved.task_threads[&scheduled.id], dedicated);
            assert_eq!(ids[0], dedicated);
            runtime.command(reply).await.unwrap();
            let owner = claim_one(&runtime).await.unwrap();
            assert_eq!(owner.run.thread_id, dedicated);
            assert_eq!(owner.run.session_id.as_deref(), Some("s1"));
            assert_eq!(owner.run.trigger, Some(RunTrigger::Chat));
            finish(&runtime, &owner.run.id, "inspected", None).await;
        } else {
            assert!(!saved.task_threads.contains_key(&scheduled.id));
            assert!(runtime.command(reply).await.is_err());
            let page = runtime
                .agent_thread_runs("assistant".into(), None, Some(dedicated.clone()))
                .await
                .unwrap();
            assert_eq!(page.len(), 5);
            let earlier = runtime
                .agent_thread_runs(
                    "assistant".into(),
                    Some(page[1].id.clone()),
                    Some(dedicated.clone()),
                )
                .await
                .unwrap();
            assert_eq!(earlier, page[2..]);
        }
    }
    runtime.publish("inbox", "owner", json!({})).await.unwrap();
    let independent = claim_one(&runtime).await.unwrap();
    assert!(independent.run.fresh_session);
    assert_ne!(independent.run.thread_id, dedicated);
}

#[tokio::test]
async fn changing_the_task_checkout_prevents_retrying_old_work_in_the_new_checkout() {
    let (_dir, runtime) = runtime_with(&["assistant"], BusyPolicy::Queue).await;
    let save = |change| ReactiveCommand::Apply { change };
    runtime
        .command(save(ReactiveChange::SaveSource {
            source: EventSource {
                id: "hook".into(),
                name: "Hook".into(),
                kind: SourceKind::Webhook,
                enabled: true,
            },
        }))
        .await
        .unwrap();
    let mut scheduled = subscription("task", "assistant", true, BusyPolicy::Queue);
    scheduled.source_id = "hook".into();
    scheduled.task_conversation = TaskConversation::Continue;
    scheduled.task_checkout = TaskCheckout::Worktree {
        base_branch: "main".into(),
        start_from_remote: true,
    };
    runtime
        .command(save(ReactiveChange::SaveSubscription {
            subscription: scheduled.clone(),
        }))
        .await
        .unwrap();
    runtime.publish("hook", "old", json!({})).await.unwrap();
    let first = claim_one(&runtime).await.unwrap().run;
    assert_eq!(first.task_checkout, scheduled.task_checkout);
    scheduled.task_checkout = TaskCheckout::Project;
    runtime
        .command(save(ReactiveChange::SaveSubscription {
            subscription: scheduled,
        }))
        .await
        .unwrap();
    assert!(runtime
        .command(ReactiveCommand::RetryRun { id: first.id })
        .await
        .is_err());
    runtime.publish("hook", "new", json!({})).await.unwrap();
    let next = claim_one(&runtime).await.unwrap().run;
    assert!(next.task_checkout.is_project() && next.fresh_session);
    assert_ne!(first.thread_id, next.thread_id);
}

#[tokio::test]
async fn run_now_targets_one_paused_task_and_preserves_its_regular_schedule() {
    let (_dir, runtime) = runtime_with(&["assistant"], BusyPolicy::Queue).await;
    let save = |change| ReactiveCommand::Apply { change };
    runtime
        .command(save(ReactiveChange::SaveSource {
            source: EventSource {
                id: "clock".into(),
                name: "Clock".into(),
                enabled: true,
                kind: SourceKind::Schedule {
                    every_ms: 3_600_000,
                },
            },
        }))
        .await
        .unwrap();
    for (id, enabled) in [("manual", false), ("recurring", true)] {
        let mut sub = subscription(id, "assistant", true, BusyPolicy::Queue);
        sub.source_id = "clock".into();
        sub.enabled = enabled;
        sub.task_conversation = TaskConversation::NewThread;
        runtime
            .command(save(ReactiveChange::SaveSubscription { subscription: sub }))
            .await
            .unwrap();
    }
    let request_id = uuid::Uuid::new_v4();
    let (first, duplicate) = tokio::join!(
        runtime.run_linked_schedule("manual".into(), request_id),
        runtime.run_linked_schedule("manual".into(), request_id)
    );
    first.unwrap();
    duplicate.unwrap();
    let snapshot = runtime
        .command(ReactiveCommand::RunScheduledTask {
            id: "manual".into(),
            run_id: request_id.to_string(),
        })
        .await
        .unwrap();
    assert!(snapshot.events.is_empty());
    assert_eq!(snapshot.runs.len(), 1);
    let manual = claim_one(&runtime).await.unwrap().run;
    assert_eq!(manual.id, request_id.to_string());
    assert_eq!(manual.trigger, Some(RunTrigger::ManualTask));
    assert_eq!(manual.subscription_id.as_deref(), Some("manual"));
    assert!(
        !snapshot
            .subscriptions
            .iter()
            .find(|s| s.id == "manual")
            .unwrap()
            .enabled
    );
    finish(&runtime, &manual.id, "done", None).await;
    runtime
        .publish("clock", "scheduled-tick", json!({}))
        .await
        .unwrap();
    let normal = claim_one(&runtime).await.unwrap().run;
    assert_eq!(normal.subscription_id.as_deref(), Some("recurring"));
    assert_eq!(normal.trigger, Some(RunTrigger::Event));
    assert!(runtime
        .command(ReactiveCommand::RunScheduledTask {
            id: "recurring".into(),
            run_id: manual.id
        })
        .await
        .is_err());
}

#[tokio::test]
async fn schedule_can_select_an_existing_project_conversation_after_its_task_is_removed() {
    let (dir, runtime) = runtime_with(&["assistant"], BusyPolicy::Queue).await;
    let owner = agent("assistant");
    let project = owner.project_path.clone();
    let mut task = AutomationSetup {
        agent: owner,
        source: EventSource {
            id: "clock".into(),
            name: "Clock".into(),
            enabled: true,
            kind: SourceKind::Schedule { every_ms: 86400000 },
        },
        subscription: subscription("morning", "assistant", true, BusyPolicy::Queue),
    };
    task.subscription.source_id = task.source.id.clone();
    task.subscription.task_title = Some("München review".into());
    task.subscription.task_conversation = TaskConversation::NewThread;
    let save = |task: &AutomationSetup, thread: Option<bruhs_models::ids::ThreadId>| {
        ReactiveCommand::SaveScheduledTask {
            automation: Box::new(task.clone()),
            enabled: true,
            signing_secret: None,
            destination: thread.map(|thread_id| TaskDestinationEdit::Existing { thread_id }),
        }
    };
    let start = |id: &str, run: &str| ReactiveCommand::RunScheduledTask {
        id: id.into(),
        run_id: run.into(),
    };
    runtime.command(save(&task, None)).await.unwrap();
    runtime.command(start("morning", "initial")).await.unwrap();
    let initial = claim_one(&runtime).await.unwrap();
    let target = initial.run.thread_id.clone();
    record_session(&runtime, "initial").await;
    finish(&runtime, "initial", "Saved review", None).await;
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::DeleteSubscription {
                id: "morning".into(),
            },
        })
        .await
        .unwrap();
    drop(runtime);
    let runtime = ReactiveRuntime::open(dir.path().join("reactive.sqlite3"))
        .await
        .unwrap();
    assert!(runtime
        .task_threads("/another".into(), String::new())
        .await
        .unwrap()
        .is_empty());
    let choices = runtime
        .task_threads(project.clone(), "MÜNCHEN".into())
        .await
        .unwrap();
    assert_eq!(choices.len(), 1);
    assert_eq!(choices[0].thread_id, target);
    assert_eq!(choices[0].title, "München review");
    task.subscription.id = "evening".into();
    task.subscription.task_title = Some("Evening follow-up".into());
    task.subscription.task_conversation = TaskConversation::Continue;
    // A rejected retarget must roll back the rest of a new task's settings.
    let mut invalid = task.clone();
    invalid.subscription.task_checkout = TaskCheckout::Worktree {
        base_branch: "main".into(),
        start_from_remote: false,
    };
    assert!(runtime
        .command(save(&invalid, Some(target.clone())))
        .await
        .is_err());
    assert!(runtime
        .snapshot()
        .await
        .unwrap()
        .subscriptions
        .iter()
        .all(|s| s.id != "evening"));
    for name in ["evening", "weekly"] {
        task.subscription.id = name.into();
        let snapshot = runtime
            .command(save(&task, Some(target.clone())))
            .await
            .unwrap();
        assert_eq!(snapshot.task_threads[name], target);
        assert_eq!(snapshot.selected_task_threads.unwrap()[name], choices[0]);
        runtime.command(start(name, name)).await.unwrap();
        // Retrying the save keeps the queued turn.
        runtime
            .command(save(&task, Some(target.clone())))
            .await
            .unwrap();
        let claim = claim_one(&runtime).await.unwrap();
        assert_eq!(claim.run.id, name);
        assert_eq!(claim.run.thread_id, target);
        assert_eq!(claim.run.session_id.as_deref(), Some("s1"));
        finish(&runtime, name, "Received", None).await;
    }
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::DeleteSubscription {
                id: "evening".into(),
            },
        })
        .await
        .unwrap();
    let snapshot = runtime.snapshot().await.unwrap();
    assert_eq!(snapshot.task_threads["weekly"], target);
    let mut reset = save(&task, None);
    if let ReactiveCommand::SaveScheduledTask { destination, .. } = &mut reset {
        *destination = Some(TaskDestinationEdit::FirstRun {
            previous: target.clone(),
        });
    }
    assert!(!runtime
        .command(reset.clone())
        .await
        .unwrap()
        .task_threads
        .contains_key("weekly"));
    runtime.command(start("weekly", "fresh")).await.unwrap();
    let fresh = claim_one(&runtime).await.unwrap();
    assert_ne!(fresh.run.thread_id, target);
    assert!(fresh.run.session_id.is_none());
    if let ReactiveCommand::SaveScheduledTask { automation, .. } = &mut reset {
        automation.subscription.task_checkout = TaskCheckout::Existing {
            path: project.clone(),
        };
    }
    assert!(runtime.command(reset).await.is_err());
    assert_eq!(
        runtime.snapshot().await.unwrap().task_threads["weekly"],
        fresh.run.thread_id
    );
    finish(&runtime, "fresh", "Done", None).await;
    assert_eq!(
        runtime
            .agent_thread_runs("assistant".into(), None, Some(target))
            .await
            .unwrap()
            .len(),
        3
    );
}

#[tokio::test]
async fn older_task_destination_keeps_its_provider_session_when_upgraded() {
    let (dir, runtime) = runtime_with(&["assistant"], BusyPolicy::Queue).await;
    let mut sub = subscription("old", "assistant", true, BusyPolicy::Queue);
    sub.source_id = "old-hook".into();
    sub.task_conversation = TaskConversation::Continue;
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveSource {
                source: EventSource {
                    id: "old-hook".into(),
                    name: "Hook".into(),
                    kind: SourceKind::Webhook,
                    enabled: true,
                },
            },
        })
        .await
        .unwrap();
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveSubscription { subscription: sub },
        })
        .await
        .unwrap();
    runtime.publish("old-hook", "one", json!({})).await.unwrap();
    let first = claim_one(&runtime).await.unwrap();
    record_session(&runtime, &first.run.id).await;
    finish(&runtime, &first.run.id, "done", None).await;
    drop(runtime);
    let path = dir.path().join("reactive.sqlite3");
    {
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE reactive_task_threads(subscription_id TEXT PRIMARY KEY,agent_id TEXT NOT NULL,
            mode TEXT NOT NULL,thread_id TEXT NOT NULL UNIQUE,session_id TEXT,stamp TEXT);
            INSERT INTO reactive_task_threads SELECT d.subscription_id,c.agent_id,'continue',c.thread_id,c.session_id,c.stamp
            FROM task_conversations c JOIN task_destinations d USING(thread_id);
            DROP TRIGGER forget_task_destination;DROP TRIGGER forget_agent_conversations;
            DROP TABLE task_destinations;DROP TABLE task_conversations;
            CREATE TRIGGER discard_task_thread AFTER DELETE ON reactive_subscriptions
            BEGIN DELETE FROM reactive_task_threads WHERE subscription_id=OLD.id;END;").unwrap();
    }
    for _ in 0..2 {
        let runtime = ReactiveRuntime::open(&path).await.unwrap();
        let saved = runtime.snapshot().await.unwrap();
        assert_eq!(saved.task_threads["old"], first.run.thread_id);
        assert!(saved.selected_task_threads.unwrap().is_empty());
        runtime
            .command(ReactiveCommand::ChatInTaskThread {
                task_id: "old".into(),
                thread_id: first.run.thread_id.clone(),
                message: "Continue".into(),
            })
            .await
            .unwrap();
        let next = claim_one(&runtime).await.unwrap();
        assert_eq!(next.run.session_id.as_deref(), Some("s1"));
        finish(&runtime, &next.run.id, "continued", None).await;
    }
}

#[tokio::test]
async fn deleting_a_conversation_removes_only_its_live_schedules_and_transcript() {
    use std::collections::BTreeSet;
    let (dir, runtime) = runtime_with(&["assistant"], BusyPolicy::Queue).await;
    let mut setup = AutomationSetup {
        agent: agent("assistant"),
        source: EventSource {
            id: "clock".into(),
            name: "Clock".into(),
            enabled: true,
            kind: SourceKind::Schedule {
                every_ms: 86_400_000,
            },
        },
        subscription: subscription("morning", "assistant", true, BusyPolicy::Queue),
    };
    setup.subscription.source_id = "clock".into();
    setup.subscription.task_conversation = TaskConversation::Continue;
    let save = |s: &AutomationSetup, destination| ReactiveCommand::SaveScheduledTask {
        automation: Box::new(s.clone()),
        enabled: true,
        signing_secret: None,
        destination,
    };
    let start = |id: &str, run: &str| ReactiveCommand::RunScheduledTask {
        id: id.into(),
        run_id: run.into(),
    };
    runtime.command(save(&setup, None)).await.unwrap();
    runtime.command(start("morning", "first")).await.unwrap();
    let first = claim_one(&runtime).await.unwrap();
    record_session(&runtime, "first").await;
    finish(&runtime, "first", &report(r#"{"outcome":"notify","title":"Release","message":"A private report","follow_ups":[{"key":"check-release","in_minutes":60,"note":"Check this report"}]}"#), None).await;
    let thread = first.run.thread_id;
    let delete = |names: &[&str]| ReactiveCommand::DeleteTaskThread {
        thread_id: thread.clone(),
        linked_tasks: names.iter().map(|n| n.to_string()).collect::<BTreeSet<_>>(),
    };
    setup.subscription.id = "evening".into();
    runtime
        .command(save(
            &setup,
            Some(TaskDestinationEdit::Existing {
                thread_id: thread.clone(),
            }),
        ))
        .await
        .unwrap();
    setup.subscription.id = "other".into();
    runtime.command(save(&setup, None)).await.unwrap();
    runtime.command(start("evening", "active")).await.unwrap();
    let active = claim_one(&runtime).await.unwrap();
    assert_eq!(active.run.thread_id, thread);
    let approving = active.run.clone();
    runtime
        .call(move |db| {
            store::crew::open_approval(db, &approving, "Read", &json!({"path":"build.log"}))
        })
        .await
        .unwrap();
    let before = runtime.snapshot().await.unwrap();
    assert!(before.outbox.iter().any(|item| item.run_id == "first"));
    assert!(before
        .outbox
        .iter()
        .any(|item| item.run_id == "active" && item.approval.is_some()));
    assert!(before
        .follow_ups
        .iter()
        .any(|item| item.key == "check-release"));
    runtime.command(start("morning", "queued")).await.unwrap();
    runtime
        .command(start("other", "independent"))
        .await
        .unwrap();
    assert!(matches!(
        runtime.command(delete(&["morning"])).await,
        Err(ReactiveError::Conflict(_))
    ));
    assert_eq!(run(&runtime, "active").await.status, RunStatus::Running);
    let saved = runtime
        .command(delete(&["morning", "evening"]))
        .await
        .unwrap();
    assert!(saved.subscriptions.iter().any(|s| s.id == "other"));
    assert!(!saved
        .subscriptions
        .iter()
        .any(|s| s.id == "morning" || s.id == "evening"));
    assert!(saved.runs.iter().all(|r| r.thread_id != thread));
    assert!(saved
        .outbox
        .iter()
        .all(|item| item.run_id != "first" && item.run_id != "active"));
    assert!(saved
        .follow_ups
        .iter()
        .all(|item| item.key != "check-release"));
    assert!(saved.task_threads.values().all(|id| id != &thread));
    assert!(saved.sources.iter().any(|s| s.id == "clock" && s.enabled));
    runtime
        .command(delete(&["morning", "evening"]))
        .await
        .unwrap();
    drop(runtime);
    let runtime = ReactiveRuntime::open(dir.path().join("reactive.sqlite3"))
        .await
        .unwrap();
    assert!(runtime
        .agent_thread_runs("assistant".into(), None, Some(thread.clone()))
        .await
        .unwrap()
        .is_empty());
    assert!(runtime
        .task_threads(setup.agent.project_path.clone(), String::new())
        .await
        .unwrap()
        .iter()
        .all(|t| t.thread_id != thread));
    assert!(runtime.command(start("other", "first")).await.is_err());
    assert_eq!(claim_one(&runtime).await.unwrap().run.id, "independent");
}
