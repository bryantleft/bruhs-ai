//! Bruh profiles: looks, access and effort beside agents, bruh states and run pages.

use super::{store, worker, ReactiveError, ReactiveRuntime};
use bruhs_models::crew::*;
use bruhs_models::reactive::*;
use bruhs_models::EpochMs;
use serde_json::json;

const ACCESS_CHANGED: &str = "Access changed. Review the new access before starting another run.";

pub(super) fn agent(id: &str) -> ReactiveAgent {
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

pub(super) async fn runtime_with(agents: &[ReactiveAgent]) -> (tempfile::TempDir, ReactiveRuntime) {
    let dir = tempfile::tempdir().unwrap();
    let runtime = ReactiveRuntime::open(dir.path().join("reactive.sqlite3"))
        .await
        .unwrap();
    for agent in agents {
        runtime
            .command(ReactiveCommand::Apply {
                change: ReactiveChange::SaveAgent {
                    agent: agent.clone(),
                },
            })
            .await
            .unwrap();
    }
    (dir, runtime)
}

pub(super) async fn claim_one(runtime: &ReactiveRuntime) -> Option<store::Claim> {
    runtime.claim_next().await.unwrap()
}

pub(super) async fn finish(runtime: &ReactiveRuntime, id: &str, answer: &str) {
    let (id, answer) = (id.to_owned(), answer.to_owned());
    runtime
        .call(move |connection| {
            store::finish_run(
                connection,
                &id,
                RunStatus::Completed,
                Some(answer),
                None,
                None,
            )
        })
        .await
        .unwrap();
}

pub(super) async fn record_session(runtime: &ReactiveRuntime, id: &str) {
    let id = id.to_owned();
    runtime
        .call(move |connection| store::record_session(connection, &id, "s1", "claude-sonnet-4-6"))
        .await
        .unwrap();
}

pub(super) async fn run(runtime: &ReactiveRuntime, id: &str) -> ReactiveRun {
    let id = id.to_owned();
    runtime
        .call(move |connection| store::run_record(connection, &id))
        .await
        .unwrap()
}

async fn session(runtime: &ReactiveRuntime, agent_id: &str) -> Option<String> {
    let agent_id = agent_id.to_owned();
    runtime
        .call(move |connection| {
            Ok(connection.query_row(
                "SELECT session_id FROM reactive_agents WHERE id=?1",
                [agent_id],
                |row| row.get(0),
            )?)
        })
        .await
        .unwrap()
}

async fn chat(runtime: &ReactiveRuntime, agent_id: &str, message: &str) -> ReactiveRun {
    let snapshot = runtime
        .command(ReactiveCommand::Chat {
            agent_id: agent_id.into(),
            message: message.into(),
            run_id: None,
        })
        .await
        .unwrap();
    snapshot
        .runs
        .into_iter()
        .rfind(|run| run.agent_id == agent_id && run.owner_message.as_deref() == Some(message))
        .unwrap()
}

fn set_bruh(agent_id: &str, look: BruhLook, level: AccessLevel) -> ReactiveCommand {
    ReactiveCommand::SetBruh {
        agent_id: agent_id.into(),
        look,
        access: BruhAccess {
            level,
            ..BruhAccess::default()
        },
        effort: None,
    }
}

fn pandan() -> BruhLook {
    BruhLook {
        face: BruhColor::Pandan,
        ..BruhLook::default()
    }
}

/// A running run with a provider session, plus one queued behind it.
async fn running_and_queued(runtime: &ReactiveRuntime) -> (String, String) {
    let first = chat(runtime, "a", "First").await;
    let claim = claim_one(runtime).await.unwrap();
    assert_eq!(claim.run.id, first.id);
    record_session(runtime, &first.id).await;
    let second = chat(runtime, "a", "Second").await;
    assert_eq!(session(runtime, "a").await.as_deref(), Some("s1"));
    (first.id, second.id)
}

#[tokio::test]
async fn look_and_name_edits_keep_running_and_queued_runs() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    let (first, second) = running_and_queued(&runtime).await;
    runtime
        .command(set_bruh("a", pandan(), AccessLevel::ReadOnly))
        .await
        .unwrap();
    let renamed = ReactiveAgent {
        name: "Renamed".into(),
        ..agent("a")
    };
    let snapshot = runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveAgent { agent: renamed },
        })
        .await
        .unwrap();
    let (first, second) = (run(&runtime, &first).await, run(&runtime, &second).await);
    assert_eq!(first.status, RunStatus::Running);
    assert_eq!(second.status, RunStatus::Queued);
    assert_eq!((first.error, second.error), (None, None));
    assert_eq!(session(&runtime, "a").await.as_deref(), Some("s1"));
    assert_eq!(snapshot.agents[0].name, "Renamed");
    assert_eq!(snapshot.bruhs[0].name, "Renamed");
    assert_eq!(snapshot.bruhs[0].look.face, BruhColor::Pandan);
    // Effort alone is not cosmetic either way, but it never stops work.
    runtime
        .command(ReactiveCommand::SetBruh {
            agent_id: "a".into(),
            look: pandan(),
            access: BruhAccess::default(),
            effort: Some("high".into()),
        })
        .await
        .unwrap();
    assert_eq!(run(&runtime, &second.id).await.status, RunStatus::Queued);
    assert_eq!(session(&runtime, "a").await.as_deref(), Some("s1"));
}

#[tokio::test]
async fn other_agent_edits_still_cancel_runs() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    let (_first, second) = running_and_queued(&runtime).await;
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveAgent {
                agent: ReactiveAgent {
                    instructions: "Something else.".into(),
                    ..agent("a")
                },
            },
        })
        .await
        .unwrap();
    assert_eq!(run(&runtime, &second).await.status, RunStatus::Cancelled);
}

#[tokio::test]
async fn access_edits_cancel_pending_and_running_runs() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    let (first, second) = running_and_queued(&runtime).await;
    let snapshot = runtime
        .command(set_bruh("a", BruhLook::default(), AccessLevel::Full))
        .await
        .unwrap();
    for id in [&first, &second] {
        let run = run(&runtime, id).await;
        assert_eq!(run.status, RunStatus::Cancelled);
        assert_eq!(run.error.as_deref(), Some(ACCESS_CHANGED));
    }
    // The session survives: only what runs above the new access stops.
    assert_eq!(session(&runtime, "a").await.as_deref(), Some("s1"));
    assert_eq!(snapshot.bruhs[0].access.level, AccessLevel::Full);
}

#[tokio::test]
async fn codex_bruhs_cannot_be_set_to_ask() {
    let codex = ReactiveAgent {
        model: "gpt-5.3-codex".into(),
        ..agent("c")
    };
    assert_eq!(
        bruhs_models::agent::AgentProvider::for_model(&codex.model),
        bruhs_models::agent::AgentProvider::Codex
    );
    let (_dir, runtime) = runtime_with(&[codex]).await;
    let asked = runtime
        .command(set_bruh("c", BruhLook::default(), AccessLevel::Ask))
        .await;
    assert!(
        matches!(&asked, Err(ReactiveError::Invalid(message)) if message.contains("Codex can't ask")),
        "{asked:?}"
    );
    runtime
        .command(set_bruh("c", BruhLook::default(), AccessLevel::Full))
        .await
        .unwrap();
    let effort = runtime
        .command(ReactiveCommand::SetBruh {
            agent_id: "c".into(),
            look: BruhLook::default(),
            access: BruhAccess::default(),
            effort: Some("turbo".into()),
        })
        .await;
    assert!(
        matches!(&effort, Err(ReactiveError::Invalid(message)) if message == "Choose a supported effort."),
        "{effort:?}"
    );
    let missing = runtime
        .command(set_bruh("nobody", BruhLook::default(), AccessLevel::Full))
        .await;
    assert!(matches!(missing, Err(ReactiveError::NotFound(_))));
}

#[tokio::test]
async fn create_bruh_makes_a_private_work_folder_when_none_is_given() {
    let (dir, runtime) = runtime_with(&[]).await;
    let create = || ReactiveCommand::CreateBruh {
        agent: ReactiveAgent {
            project_path: String::new(),
            ..agent("b1")
        },
        look: BruhLook {
            face: BruhColor::Lychee,
            ..BruhLook::default()
        },
        access: BruhAccess::default(),
        effort: Some("medium".into()),
    };
    let snapshot = runtime.command(create()).await.unwrap();
    let work = dir.path().canonicalize().unwrap().join("bruhs/b1/work");
    assert_eq!(snapshot.agents[0].project_path, work.to_string_lossy());
    assert!(work.is_dir());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&work).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }
    assert_eq!(snapshot.bruhs[0].look.face, BruhColor::Lychee);
    assert_eq!(snapshot.bruhs[0].effort.as_deref(), Some("medium"));
    let again = runtime.command(create()).await;
    assert!(
        matches!(&again, Err(ReactiveError::Conflict(message)) if message == "A bruh with this ID already exists."),
        "{again:?}"
    );
    // An unsafe id never becomes a folder.
    let escape = runtime
        .command(ReactiveCommand::CreateBruh {
            agent: ReactiveAgent {
                project_path: String::new(),
                ..agent("../x")
            },
            look: BruhLook::default(),
            access: BruhAccess::default(),
            effort: None,
        })
        .await;
    assert!(matches!(escape, Err(ReactiveError::Invalid(_))));
    assert!(!dir.path().join("bruhs/x").exists() && !dir.path().join("x").exists());
}

#[tokio::test]
async fn create_bruh_keeps_a_given_folder() {
    let (dir, runtime) = runtime_with(&[]).await;
    let snapshot = runtime
        .command(ReactiveCommand::CreateBruh {
            agent: agent("b2"),
            look: BruhLook::default(),
            access: BruhAccess::default(),
            effort: None,
        })
        .await
        .unwrap();
    assert_eq!(snapshot.agents[0].project_path, agent("b2").project_path);
    assert!(!dir.path().join("bruhs").exists());
}

#[test]
fn create_bruh_parses_with_default_look_access_and_effort() {
    let command: ReactiveCommand = serde_json::from_value(json!({
        "action": "create_bruh",
        "agent": serde_json::to_value(agent("b")).unwrap(),
    }))
    .unwrap();
    let ReactiveCommand::CreateBruh {
        agent: parsed,
        look,
        access,
        effort,
    } = command.clone()
    else {
        panic!("{command:?}");
    };
    assert_eq!(parsed, agent("b"));
    assert_eq!(look, BruhLook::default());
    assert_eq!(
        (look.look, look.face, look.mouth),
        (Look::Front, BruhColor::Bruh, MouthColor::Bruh)
    );
    assert_eq!(access, BruhAccess::default());
    assert_eq!(access.level, AccessLevel::ReadOnly);
    assert!(!access.send_as_me && !access.can_deliver);
    assert_eq!(effort, None);
    let roundtrip: ReactiveCommand =
        serde_json::from_str(&serde_json::to_string(&command).unwrap()).unwrap();
    assert_eq!(roundtrip, command);
}

#[tokio::test]
async fn snapshot_and_signal_carry_bruh_states() {
    let (_dir, runtime) = runtime_with(&[]).await;
    runtime
        .command(ReactiveCommand::CreateBruh {
            agent: agent("a"),
            look: BruhLook::default(),
            access: BruhAccess::default(),
            effort: None,
        })
        .await
        .unwrap();
    let snapshot = runtime.snapshot().await.unwrap();
    assert_eq!(snapshot.protocol, CREW_PROTOCOL);
    assert_eq!(snapshot.bruhs.len(), 1);
    assert_eq!(snapshot.bruhs[0].agent_id, "a");
    assert_eq!(snapshot.bruhs[0].status, BruhStatus::Asleep);
    assert_eq!(snapshot.bruhs[0].next_wake_ms, None);
    let signal = runtime.outbox_signal().await.unwrap();
    assert_eq!(signal.protocol, CREW_PROTOCOL);
    assert_eq!(signal.bruhs, snapshot.bruhs);

    chat(&runtime, "a", "Hello").await;
    let claim = claim_one(&runtime).await.unwrap();
    let snapshot = runtime.snapshot().await.unwrap();
    assert_eq!(snapshot.bruhs[0].status, BruhStatus::Working);
    finish(
        &runtime,
        &claim.run.id,
        "Done.\n\n```bruhs-report\n{\"outcome\":\"notify\",\"title\":\"Hi\",\"message\":\"Hello back.\",\"follow_ups\":[{\"key\":\"later\",\"in_minutes\":60,\"note\":\"Check in\"}]}\n```",
    )
    .await;
    let snapshot = runtime.snapshot().await.unwrap();
    let due = snapshot
        .follow_ups
        .iter()
        .find(|item| item.key == "later")
        .unwrap()
        .due_at;
    let state = &snapshot.bruhs[0];
    assert_eq!(state.status, BruhStatus::Asleep);
    assert_eq!(state.next_wake_ms, Some(due));
    assert_eq!(state.unread, 1);
    assert_eq!(runtime.outbox_signal().await.unwrap().bruhs[0].unread, 1);

    let snapshot = runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveAgent {
                agent: ReactiveAgent {
                    enabled: false,
                    ..agent("a")
                },
            },
        })
        .await
        .unwrap();
    assert_eq!(snapshot.bruhs[0].status, BruhStatus::Off);
}

/// A run can start and end between two live frames, leaving every bruh's
/// state as it was. The signal still changes, so a chat shows the reply.
#[tokio::test]
async fn a_run_that_starts_and_ends_between_frames_changes_the_signal() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    let first = chat(&runtime, "a", "First").await;
    claim_one(&runtime).await.unwrap();
    finish(&runtime, &first.id, "Done").await;
    let before = runtime.outbox_signal().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let second = chat(&runtime, "a", "Second").await;
    claim_one(&runtime).await.unwrap();
    finish(&runtime, &second.id, "Done again").await;
    let after = runtime.outbox_signal().await.unwrap();
    assert_eq!(after.bruhs, before.bruhs, "the bruh looks the same");
    assert_ne!(after, before, "the signal still moved");
    assert_eq!(
        after.runs_at,
        Some(run(&runtime, &second.id).await.updated_at)
    );
}

#[tokio::test]
async fn bruh_states_wake_on_calendar_subscriptions_and_skip_report_presets() {
    let preset = format!("weekly-report-{}", uuid::Uuid::new_v4());
    let (_dir, runtime) = runtime_with(&[agent("a"), agent(&preset)]).await;
    for change in [
        ReactiveChange::SaveSource {
            source: EventSource {
                id: "morning".into(),
                name: "Morning".into(),
                enabled: true,
                kind: SourceKind::Calendar {
                    times: vec!["08:00".into()],
                    weekdays: Vec::new(),
                    time_zone: "UTC".into(),
                },
            },
        },
        ReactiveChange::SaveSubscription {
            subscription: Subscription {
                id: "wake".into(),
                source_id: "morning".into(),
                agent_id: "a".into(),
                instructions: "Plan the day.".into(),
                enabled: true,
                auto_run: true,
                debounce_ms: 0,
                throttle_ms: 0,
                on_busy: BusyPolicy::Queue,
            },
        },
    ] {
        runtime
            .command(ReactiveCommand::Apply { change })
            .await
            .unwrap();
    }
    let snapshot = runtime.snapshot().await.unwrap();
    assert_eq!(snapshot.bruhs.len(), 1, "report presets are not bruhs");
    let wake = snapshot.bruhs[0].next_wake_ms.unwrap().as_millis();
    let now = EpochMs::now().as_millis();
    assert!(wake > now && wake <= now + 24 * 60 * 60 * 1000);
    assert_eq!(wake % (60 * 1000), 0);
}

#[tokio::test]
async fn deleting_an_agent_removes_its_bruh_profile() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    runtime
        .command(set_bruh("a", pandan(), AccessLevel::ReadOnly))
        .await
        .unwrap();
    let rows = |runtime: &ReactiveRuntime| {
        let runtime = runtime.clone();
        async move {
            runtime
                .call(|connection| {
                    Ok(connection.query_row(
                        "SELECT COUNT(*) FROM reactive_bruhs WHERE agent_id='a'",
                        [],
                        |row| row.get::<_, i64>(0),
                    )?)
                })
                .await
                .unwrap()
        }
    };
    assert_eq!(rows(&runtime).await, 1);
    let snapshot = runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::DeleteAgent { id: "a".into() },
        })
        .await
        .unwrap();
    assert_eq!(rows(&runtime).await, 0);
    assert!(snapshot.bruhs.is_empty());
}

#[tokio::test]
async fn batched_chats_keep_the_owners_words() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    let first = chat(&runtime, "a", "Book the flight").await;
    chat(&runtime, "a", "And a hotel").await;
    let claim = claim_one(&runtime).await.unwrap();
    assert_eq!(claim.run.id, first.id);
    assert_eq!(claim.run.merged_runs.len(), 1);
    assert_eq!(claim.run.owner_message.as_deref(), Some("Book the flight"));
    assert!(
        claim.run.prompt.contains("Book the flight") && claim.run.prompt.contains("And a hotel")
    );
    let merged = run(&runtime, &claim.run.merged_runs[0]).await;
    assert_eq!(merged.owner_message.as_deref(), Some("And a hotel"));
    assert_eq!(claim.bruh, BruhProfile::default());
}

#[tokio::test]
async fn a_chat_can_name_its_run() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    let chat = |run_id: &str| ReactiveCommand::Chat {
        agent_id: "a".into(),
        message: "Check the notes".into(),
        run_id: Some(run_id.into()),
    };
    let snapshot = runtime.command(chat("r-1")).await.unwrap();
    let run = snapshot.runs.iter().find(|run| run.id == "r-1").unwrap();
    assert_eq!(run.status, RunStatus::Queued);
    assert_eq!(run.trigger, Some(RunTrigger::Chat));
    assert_eq!(run.owner_message.as_deref(), Some("Check the notes"));

    let repeated = runtime.command(chat("r-1")).await.unwrap_err();
    assert!(
        matches!(&repeated, ReactiveError::Conflict(message) if message == "A run with this ID already exists."),
        "{repeated:?}"
    );
    assert!(matches!(
        runtime.command(chat("bad/id")).await.unwrap_err(),
        ReactiveError::Invalid(_)
    ));
    let runs = runtime.snapshot().await.unwrap().runs;
    assert_eq!(runs.len(), 1);
}

#[tokio::test]
async fn a_claim_carries_the_bruh_profile() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    runtime
        .command(ReactiveCommand::SetBruh {
            agent_id: "a".into(),
            look: pandan(),
            access: BruhAccess {
                level: AccessLevel::Ask,
                send_as_me: true,
                can_deliver: false,
            },
            effort: Some("high".into()),
        })
        .await
        .unwrap();
    chat(&runtime, "a", "Go").await;
    let claim = claim_one(&runtime).await.unwrap();
    assert_eq!(claim.bruh.look, pandan());
    assert_eq!(claim.bruh.access.level, AccessLevel::Ask);
    assert!(claim.bruh.access.send_as_me);
    assert_eq!(claim.bruh.effort.as_deref(), Some("high"));
}

#[tokio::test]
async fn agent_runs_page_one_agents_runs_newest_first() {
    let (_dir, runtime) = runtime_with(&[agent("a"), agent("b")]).await;
    let mut ids = Vec::new();
    for message in ["one", "two", "three"] {
        ids.push(chat(&runtime, "a", message).await.id);
        chat(&runtime, "b", message).await;
    }
    let page = runtime.agent_runs("a".into(), None).await.unwrap();
    let got: Vec<&str> = page.iter().map(|run| run.id.as_str()).collect();
    let newest_first: Vec<&str> = ids.iter().rev().map(String::as_str).collect();
    assert_eq!(got, newest_first);
    let older = runtime
        .agent_runs("a".into(), Some(page[0].id.clone()))
        .await
        .unwrap();
    let got: Vec<&str> = older.iter().map(|run| run.id.as_str()).collect();
    assert_eq!(got, newest_first[1..]);
    assert!(runtime
        .agent_runs("nobody".into(), None)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn agent_runs_pages_reach_every_run_created_in_the_same_millisecond() {
    let (_dir, runtime) = runtime_with(&[agent("a"), agent("b")]).await;
    let template = chat(&runtime, "a", "Template").await;
    let total = HISTORY_LIMIT * 2 + 7;
    let ids: Vec<String> = (0..total).map(|n| format!("run-{n:03}")).collect();
    let saved = ids.clone();
    runtime
        .call(move |connection| {
            store::transaction(connection, |connection| {
                for id in &saved {
                    for agent_id in ["a", "b"] {
                        let run = ReactiveRun {
                            id: format!("{agent_id}-{id}"),
                            agent_id: agent_id.into(),
                            status: RunStatus::Completed,
                            ..template.clone()
                        };
                        store::save_run(connection, &run)?;
                    }
                }
                connection.execute("DELETE FROM reactive_runs WHERE id=?1", [&template.id])?;
                Ok(())
            })
        })
        .await
        .unwrap();
    let mut seen = Vec::new();
    let mut before = None;
    loop {
        let page = runtime
            .agent_runs("a".into(), before.clone())
            .await
            .unwrap();
        assert!(page.len() <= HISTORY_LIMIT);
        let Some(last) = page.last() else { break };
        before = Some(last.id.clone());
        seen.extend(page.into_iter().map(|run| run.id));
    }
    let newest_first: Vec<String> = ids.iter().rev().map(|id| format!("a-{id}")).collect();
    assert_eq!(seen, newest_first);
}

async fn bruh_status(runtime: &ReactiveRuntime, id: &str) -> BruhStatus {
    runtime
        .snapshot()
        .await
        .unwrap()
        .bruhs
        .into_iter()
        .find(|bruh| bruh.agent_id == id)
        .unwrap()
        .status
}

async fn watch_inbox(runtime: &ReactiveRuntime, agent_id: &str) {
    for change in [
        ReactiveChange::SaveSource {
            source: EventSource {
                id: "inbox".into(),
                name: "Inbox".into(),
                enabled: true,
                kind: SourceKind::Custom,
            },
        },
        ReactiveChange::SaveSubscription {
            subscription: Subscription {
                id: format!("sub-{agent_id}"),
                source_id: "inbox".into(),
                agent_id: agent_id.into(),
                instructions: "Summarize the event.".into(),
                enabled: true,
                auto_run: true,
                debounce_ms: 0,
                throttle_ms: 0,
                on_busy: BusyPolicy::Queue,
            },
        },
    ] {
        runtime
            .command(ReactiveCommand::Apply { change })
            .await
            .unwrap();
    }
}

async fn run_count(runtime: &ReactiveRuntime, agent_id: &str) -> usize {
    runtime
        .snapshot()
        .await
        .unwrap()
        .runs
        .iter()
        .filter(|run| run.agent_id == agent_id)
        .count()
}

#[tokio::test]
async fn a_run_awaiting_review_needs_the_owner() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    watch_inbox(&runtime, "a").await;
    let mut subscription = runtime.snapshot().await.unwrap().subscriptions[0].clone();
    subscription.auto_run = false;
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveSubscription { subscription },
        })
        .await
        .unwrap();
    assert_eq!(bruh_status(&runtime, "a").await, BruhStatus::Asleep);
    runtime.publish("inbox", "mail-1", json!({})).await.unwrap();
    let run = runtime.snapshot().await.unwrap().runs[0].clone();
    assert_eq!(run.status, RunStatus::AwaitingReview);
    assert_eq!(bruh_status(&runtime, "a").await, BruhStatus::NeedsYou);

    runtime
        .command(ReactiveCommand::CancelRun { id: run.id })
        .await
        .unwrap();
    assert_eq!(bruh_status(&runtime, "a").await, BruhStatus::Asleep);
}

#[tokio::test]
async fn stop_all_cancels_every_run_and_pauses_every_bruh() {
    let (_dir, runtime) = runtime_with(&[agent("a"), agent("b")]).await;
    let (running, queued) = running_and_queued(&runtime).await;
    let other = chat(&runtime, "b", "Other").await;
    let approval = OutboxItem {
        id: "ask-1".into(),
        agent_id: "a".into(),
        run_id: running.clone(),
        kind: OutboxKind::Ask,
        title: "Run a command".into(),
        message: "Allow it?".into(),
        created_at: EpochMs::now(),
        read_at: None,
        resolved_at: None,
        answer: None,
        system: SystemDelivery::NotRequested,
        approval: Some(ApprovalRequest {
            tool: "Bash".into(),
            summary: "ls".into(),
            fingerprint: "f1".into(),
            state: ApprovalState::Waiting,
            detail: None,
        }),
    };
    runtime
        .call(move |connection| {
            connection.execute(
                "INSERT INTO reactive_outbox(id,agent_id,run_id,created,payload) VALUES(?1,?2,?3,?4,?5)",
                rusqlite::params![
                    approval.id,
                    approval.agent_id,
                    approval.run_id,
                    approval.created_at.as_millis(),
                    serde_json::to_string(&approval).unwrap()
                ],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let snapshot = runtime.command(ReactiveCommand::StopAll).await.unwrap();
    for id in [&running, &queued, &other.id] {
        let stopped = run(&runtime, id).await;
        assert_eq!(stopped.status, RunStatus::Cancelled, "{id}");
        assert_eq!(stopped.error.as_deref(), Some("Stopped by the owner."));
    }
    assert!(claim_one(&runtime).await.is_none());
    assert_eq!(snapshot.bruhs.len(), 2);
    assert!(snapshot
        .bruhs
        .iter()
        .all(|bruh| bruh.status == BruhStatus::Paused));
    let item = snapshot
        .outbox
        .iter()
        .find(|item| item.id == "ask-1")
        .unwrap();
    assert!(item.resolved_at.is_some());
    assert_eq!(item.approval.as_ref().unwrap().state, ApprovalState::Denied);
}

#[tokio::test]
async fn stop_all_cancels_report_agent_runs_but_pauses_only_bruhs() {
    let report_id = format!("weekly-report-{}", uuid::Uuid::nil());
    let (_dir, runtime) = runtime_with(&[agent("a"), agent(&report_id)]).await;
    let report_run = chat(&runtime, &report_id, "Report").await;
    runtime.command(ReactiveCommand::StopAll).await.unwrap();
    let stopped = run(&runtime, &report_run.id).await;
    assert_eq!(stopped.status, RunStatus::Cancelled);
    assert_eq!(stopped.error.as_deref(), Some("Stopped by the owner."));
    let paused =
        |id: String| runtime.call(move |connection| store::crew::is_paused(connection, &id));
    assert!(paused("a".into()).await.unwrap());
    assert!(!paused(report_id).await.unwrap());
}

#[tokio::test]
async fn a_paused_bruh_takes_no_events_chats_or_follow_ups_until_resumed() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    watch_inbox(&runtime, "a").await;
    let follow_up = FollowUp {
        id: "f1".into(),
        agent_id: "a".into(),
        key: "later".into(),
        note: "Check back".into(),
        due_at: EpochMs::new(1).unwrap(),
        status: FollowUpStatus::Pending,
        created_at: EpochMs::new(1).unwrap(),
        run_id: None,
        auto_run: true,
        fired_run_id: None,
    };
    runtime
        .call(move |connection| {
            connection.execute(
                "INSERT INTO reactive_follow_ups(id,agent_id,key,status,due,created,payload) VALUES(?1,?2,?3,'pending',1,1,?4)",
                rusqlite::params![follow_up.id, follow_up.agent_id, follow_up.key, serde_json::to_string(&follow_up).unwrap()],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    runtime
        .command(ReactiveCommand::StopBruh {
            agent_id: "a".into(),
        })
        .await
        .unwrap();
    assert_eq!(bruh_status(&runtime, "a").await, BruhStatus::Paused);

    runtime.publish("inbox", "one", json!({})).await.unwrap();
    assert_eq!(run_count(&runtime, "a").await, 0);
    let error = runtime
        .command(ReactiveCommand::Chat {
            agent_id: "a".into(),
            message: "Hello".into(),
            run_id: None,
        })
        .await
        .unwrap_err();
    assert!(
        matches!(&error, ReactiveError::Conflict(message) if message == "Agent a is paused. Resume it to send work."),
        "{error:?}"
    );
    assert_eq!(runtime.fire_follow_ups().await.unwrap(), 0);
    let snapshot = runtime.snapshot().await.unwrap();
    assert_eq!(snapshot.follow_ups[0].status, FollowUpStatus::Pending);

    runtime
        .command(ReactiveCommand::ResumeBruh {
            agent_id: "a".into(),
        })
        .await
        .unwrap();
    assert_ne!(bruh_status(&runtime, "a").await, BruhStatus::Paused);
    assert_eq!(runtime.fire_follow_ups().await.unwrap(), 1);
    runtime.publish("inbox", "two", json!({})).await.unwrap();
    assert_eq!(run_count(&runtime, "a").await, 2);
}

#[tokio::test]
async fn stop_bruh_only_pauses_that_bruh() {
    let (_dir, runtime) = runtime_with(&[agent("a"), agent("b")]).await;
    let a = chat(&runtime, "a", "One").await;
    let b = chat(&runtime, "b", "Two").await;
    runtime
        .command(ReactiveCommand::StopBruh {
            agent_id: "a".into(),
        })
        .await
        .unwrap();
    assert_eq!(run(&runtime, &a.id).await.status, RunStatus::Cancelled);
    assert_eq!(run(&runtime, &b.id).await.status, RunStatus::Queued);
    assert_ne!(bruh_status(&runtime, "b").await, BruhStatus::Paused);
    let claim = claim_one(&runtime).await.unwrap();
    assert_eq!(claim.run.id, b.id);
}

#[test]
fn instructions_stamp_ignores_name_and_access() {
    let a = agent("a");
    let renamed = ReactiveAgent {
        name: "New".into(),
        ..a.clone()
    };
    assert_eq!(
        worker::instructions_stamp(&a),
        worker::instructions_stamp(&renamed)
    );
    let instructions = worker::system_instructions(&a);
    assert!(!instructions.contains(&a.name));
    assert!(!instructions.contains("read-only"));
    assert!(instructions.starts_with("You are a persistent Bruhs agent."));
    assert!(instructions.contains("Prepare drafts for review."));
}

/// A finished run that left provider session "s1" on agent "a".
async fn finished_with_session(runtime: &ReactiveRuntime) {
    let first = chat(runtime, "a", "First").await;
    let claim = claim_one(runtime).await.unwrap();
    assert_eq!(claim.run.id, first.id);
    record_session(runtime, &first.id).await;
    finish(runtime, &first.id, "Done").await;
}

#[tokio::test]
async fn renaming_or_restyling_keeps_the_provider_session() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    finished_with_session(&runtime).await;
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveAgent {
                agent: ReactiveAgent {
                    name: "Renamed".into(),
                    ..agent("a")
                },
            },
        })
        .await
        .unwrap();
    runtime
        .command(set_bruh("a", pandan(), AccessLevel::ReadOnly))
        .await
        .unwrap();
    let next = chat(&runtime, "a", "Next").await;
    let claim = claim_one(&runtime).await.unwrap();
    assert_eq!(claim.run.id, next.id);
    assert_eq!(claim.agent.name, "Renamed");
    assert_eq!(claim.bruh.look.face, BruhColor::Pandan);
    assert_eq!(claim.run.session_id.as_deref(), Some("s1"));
    assert!(!claim.run.fresh_session);
}

#[tokio::test]
async fn access_edits_cancel_pending_runs_but_keep_the_session() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    finished_with_session(&runtime).await;
    let pending = chat(&runtime, "a", "Pending").await;
    runtime
        .command(set_bruh("a", BruhLook::default(), AccessLevel::Full))
        .await
        .unwrap();
    let pending = run(&runtime, &pending.id).await;
    assert_eq!(pending.status, RunStatus::Cancelled);
    assert_eq!(pending.error.as_deref(), Some(ACCESS_CHANGED));
    let next = chat(&runtime, "a", "Next").await;
    let claim = claim_one(&runtime).await.unwrap();
    assert_eq!(claim.run.id, next.id);
    assert_eq!(claim.bruh.access.level, AccessLevel::Full);
    assert_eq!(claim.run.session_id.as_deref(), Some("s1"));
    assert!(!claim.run.fresh_session);
}

fn queued_run(prompt: &str) -> ReactiveRun {
    let now = EpochMs::now();
    ReactiveRun {
        id: "r".into(),
        agent_id: "a".into(),
        source_id: None,
        subscription_id: None,
        event_id: None,
        status: RunStatus::Running,
        prompt: prompt.into(),
        thread_id: bruhs_models::ids::ThreadId::new_v4(),
        session_id: Some("s1".into()),
        created_at: now,
        updated_at: now,
        not_before: now,
        result: None,
        error: None,
        trigger: None,
        merged_runs: Vec::new(),
        merged_into: None,
        report: None,
        report_warning: None,
        fresh_session: false,
        context_tokens: None,
        owner_message: None,
    }
}

fn profile(level: AccessLevel, effort: Option<&str>) -> BruhProfile {
    BruhProfile {
        access: BruhAccess {
            level,
            ..BruhAccess::default()
        },
        effort: effort.map(str::to_owned),
        ..BruhProfile::default()
    }
}

#[test]
fn each_run_names_the_bruh_and_its_access() {
    use bruhs_models::agent::PermissionMode;
    let (claude, run) = (agent("a"), queued_run("Do the thing."));
    let full = worker::turn(
        &claude,
        &profile(AccessLevel::Full, None),
        &run,
        "ctx",
        std::path::Path::new("/home"),
        &worker::TurnVault::default(),
    );
    assert!(full.prompt.starts_with("Your name is Agent a.\n"));
    assert!(full.prompt.contains("Access: full."));
    assert!(full.prompt.contains("\n\nctx\n\n---\n\n"));
    assert!(full.prompt.ends_with("Do the thing."));
    // Default, not Accept Edits: every edit reaches the permission gate.
    assert_eq!(full.permission_mode, PermissionMode::Default);
    assert_eq!(full.unattended_access.level, AccessLevel::Full);
    assert_eq!(full.session_id.as_deref(), Some("s1"));
    assert!(!full.fresh_session);
    assert_eq!(full.effort, None);

    let no_context = worker::turn(
        &claude,
        &profile(AccessLevel::Full, None),
        &run,
        "",
        std::path::Path::new("/home"),
        &worker::TurnVault::default(),
    );
    assert!(no_context.prompt.ends_with("without asking.\nNever push, merge or open pull requests.\nNever send messages in the owner's name.\n\nDo the thing."));
    assert!(!no_context.prompt.contains("---"));

    let ask = worker::turn(
        &claude,
        &profile(AccessLevel::Ask, Some("high")),
        &run,
        "",
        std::path::Path::new("/home"),
        &worker::TurnVault::default(),
    );
    assert_eq!(ask.permission_mode, PermissionMode::Default);
    assert!(ask.prompt.contains("Access: ask first."));
    assert_eq!(ask.effort.as_deref(), Some("high"));

    let codex = ReactiveAgent {
        model: "gpt-5.3-codex".into(),
        ..agent("a")
    };
    let codex_ask = worker::turn(
        &codex,
        &profile(AccessLevel::Ask, None),
        &run,
        "",
        std::path::Path::new("/home"),
        &worker::TurnVault::default(),
    );
    assert_eq!(codex_ask.permission_mode, PermissionMode::Plan);
    assert!(codex_ask.prompt.contains("Access: read only."));
    assert_eq!(codex_ask.unattended_access.level, AccessLevel::ReadOnly);
}

/// Every bruh run, with its Computer on or off, is kept out of every
/// bruh's browser profile and cache, by name its own, and the folders that
/// hold the owner's saved logins.
#[test]
fn every_bruh_run_hides_the_browser_profiles() {
    let run = queued_run("Look it up.");
    let home = std::path::Path::new("/data");
    let vault = worker::TurnVault {
        private: vec!["/data/credentials".into()],
    };
    let expected: Vec<std::path::PathBuf> = [
        "/data/bruhs/a/browser",
        "/data/bruhs/a/browser-cache",
        "/data/bruhs/*/browser",
        "/data/bruhs/*/browser-cache",
        "/data/credentials",
    ]
    .map(std::path::PathBuf::from)
    .to_vec();
    for level in [AccessLevel::ReadOnly, AccessLevel::Ask, AccessLevel::Full] {
        for on in [false, true] {
            let bruh = BruhProfile {
                computer: on,
                ..profile(level, None)
            };
            let turn = worker::turn(&agent("a"), &bruh, &run, "", home, &vault);
            assert_eq!(turn.hidden_folders, expected, "{level:?} computer={on}");
        }
    }
}

/// Every bruh run's deny rules name the vault catalog and the login stores
/// at every level, but not the keychains folder: the sandbox reads the
/// rules, and git and gh read their logins from the keychain in process.
#[test]
fn every_bruh_run_hides_the_vault_folders() {
    let run = queued_run("Look it up.");
    let home = std::path::Path::new("/Users/a");
    let private = crate::secrets::private::private_folders(
        home,
        std::path::Path::new("/Users/a/Library/Application Support/bruhs-ai/credentials"),
    );
    let vault = worker::TurnVault {
        private: private.clone(),
    };
    let keychains = std::path::PathBuf::from("/Users/a/Library/Keychains");
    for level in [AccessLevel::ReadOnly, AccessLevel::Ask, AccessLevel::Full] {
        let turn = worker::turn(&agent("a"), &profile(level, None), &run, "", home, &vault);
        for folder in private.iter().filter(|folder| **folder != keychains) {
            assert!(turn.hidden_folders.contains(folder), "{level:?} {folder:?}");
        }
        assert!(!turn.hidden_folders.contains(&keychains), "{level:?}");
    }
}

#[test]
fn the_worker_turns_computer_on_only_when_available() {
    const BROWSER_LINE: &str = "You have your own browser, the local_browser tool.";
    let run = queued_run("Look it up.");
    let with_computer = |level, on| BruhProfile {
        computer: on,
        ..profile(level, None)
    };
    for level in [AccessLevel::Ask, AccessLevel::Full] {
        let turn = worker::turn(
            &agent("a"),
            &with_computer(level, true),
            &run,
            "",
            std::path::Path::new("/home"),
            &worker::TurnVault::default(),
        );
        assert_eq!(turn.computer.as_deref(), Some("a"), "{level:?}");
        assert!(turn.prompt.contains(BROWSER_LINE), "{level:?}");
    }
    let codex = ReactiveAgent {
        model: "gpt-5.3-codex".into(),
        ..agent("a")
    };
    for (agent, profile) in [
        (agent("a"), with_computer(AccessLevel::ReadOnly, true)),
        (agent("a"), with_computer(AccessLevel::Full, false)),
        (codex, with_computer(AccessLevel::Full, true)),
    ] {
        let turn = worker::turn(
            &agent,
            &profile,
            &run,
            "",
            std::path::Path::new("/home"),
            &worker::TurnVault::default(),
        );
        assert_eq!(
            turn.computer, None,
            "{} {:?}",
            agent.model, profile.access.level
        );
        assert!(!turn.prompt.contains(BROWSER_LINE));
    }
}

// ── Approvals ──────────────────────────────────────────────────────────────

pub(super) const LONG_WAIT: std::time::Duration = std::time::Duration::from_secs(30);
pub(super) const SHORT_WAIT: std::time::Duration = std::time::Duration::from_millis(50);
const DENIED: &str = "The owner denied this. Continue without it.";
pub(super) const NOT_ANSWERED: &str =
    "The owner has not answered yet. Stop here and say you are waiting on their approval.";

fn access(level: AccessLevel) -> BruhAccess {
    BruhAccess {
        level,
        ..BruhAccess::default()
    }
}

/// A chat claimed for `agent_id`, now Running.
async fn claimed(runtime: &ReactiveRuntime, agent_id: &str) -> ReactiveRun {
    chat(runtime, agent_id, "Do the work.").await;
    let claim = claim_one(runtime).await.unwrap();
    assert_eq!(claim.run.agent_id, agent_id);
    claim.run
}

pub(super) type Decision = Result<Option<crate::PermissionDecision>, ReactiveError>;

/// `approval::decide` on a Claude bruh run, in the background.
fn spawn_decide(
    runtime: &ReactiveRuntime,
    level: AccessLevel,
    run: &ReactiveRun,
    project_path: &str,
    tool: &str,
    input: serde_json::Value,
    timeout: std::time::Duration,
) -> tokio::task::JoinHandle<Decision> {
    let (runtime, run) = (runtime.clone(), run.clone());
    let (project_path, tool) = (project_path.to_owned(), tool.to_owned());
    tokio::spawn(async move {
        super::approval::decide(
            &runtime,
            &access(level),
            bruhs_models::agent::AgentProvider::Claude,
            false,
            &run,
            runtime
                .allowances
                .current_claim(&run.id)
                .unwrap_or_default(),
            &project_path,
            &tool,
            &input,
            timeout,
        )
        .await
    })
}

pub(super) async fn decided(
    task: tokio::task::JoinHandle<Decision>,
) -> Option<crate::PermissionDecision> {
    tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .expect("the approval wait should end")
        .unwrap()
        .unwrap()
}

pub(super) fn deny(reason: &str) -> Option<crate::PermissionDecision> {
    Some(crate::PermissionDecision::Deny {
        reason: Some(reason.into()),
    })
}

pub(super) async fn approvals(runtime: &ReactiveRuntime, agent_id: &str) -> Vec<OutboxItem> {
    runtime
        .snapshot()
        .await
        .unwrap()
        .outbox
        .into_iter()
        .filter(|item| item.agent_id == agent_id && item.approval.is_some())
        .collect()
}

async fn approval_item(runtime: &ReactiveRuntime, id: &str) -> OutboxItem {
    let id = id.to_owned();
    runtime
        .call(move |connection| store::crew::approval(connection, &id))
        .await
        .unwrap()
}

/// The newest waiting approval for `agent_id`, once the worker has opened it.
pub(super) async fn waiting_approval(runtime: &ReactiveRuntime, agent_id: &str) -> OutboxItem {
    let mut changes = runtime.watch_changes();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(item) = approvals(runtime, agent_id).await.into_iter().find(|item| {
                item.approval.as_ref().unwrap().state == ApprovalState::Waiting
                    && item.resolved_at.is_none()
            }) {
                return item;
            }
            changes.changed().await.unwrap();
        }
    })
    .await
    .expect("an approval should open")
}

pub(super) async fn decide_approval(
    runtime: &ReactiveRuntime,
    id: &str,
    approve: bool,
) -> ReactiveSnapshot {
    runtime
        .command(ReactiveCommand::DecideApproval {
            id: id.into(),
            approve,
        })
        .await
        .unwrap()
}

async fn take_grant(runtime: &ReactiveRuntime, agent_id: &str, fingerprint: &str) -> bool {
    let (agent_id, fingerprint) = (agent_id.to_owned(), fingerprint.to_owned());
    runtime
        .call(move |connection| store::crew::take_grant(connection, &agent_id, &fingerprint))
        .await
        .unwrap()
}

fn project() -> String {
    std::env::temp_dir().to_string_lossy().into_owned()
}

/// A network tool call whose host is this computer by name, by literal or
/// by one of this computer's own interface addresses is denied before any
/// approval opens, at Ask and Full.
#[tokio::test]
async fn network_calls_to_this_computer_are_denied_without_asking() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    let run = claimed(&runtime, "a").await;
    let mut calls = vec![
        (
            "WebFetch",
            json!({"url": "https://bruhs.localhost/api/reactive/agents/a/computer", "prompt": "Read"}),
        ),
        (
            "WebFetch",
            json!({"url": "http://127.0.0.1:3001/", "prompt": "Read"}),
        ),
        ("SandboxNetworkAccess", json!({"host": "[::1]:3001"})),
    ];
    let own = crate::local_tools::interface_addresses_for_tests()
        .into_iter()
        .find(
            |ip| matches!(ip, std::net::IpAddr::V4(v4) if !v4.is_loopback() && !v4.is_link_local()),
        );
    if let Some(own) = own {
        calls.push((
            "WebFetch",
            json!({"url": format!("http://{own}:3001/api"), "prompt": "Read"}),
        ));
        calls.push(("SandboxNetworkAccess", json!({"host": own.to_string()})));
    }
    for level in [AccessLevel::Ask, AccessLevel::Full] {
        for (tool, input) in &calls {
            let task = spawn_decide(
                &runtime,
                level,
                &run,
                &project(),
                tool,
                input.clone(),
                LONG_WAIT,
            );
            assert_eq!(
                decided(task).await,
                deny(bruhs_models::crew::LOCAL_HOST_DENIAL),
                "{level:?} {tool} {input}"
            );
        }
    }
    assert!(approvals(&runtime, "a").await.is_empty());
}

/// A file tool call whose real path is in a folder that holds the owner's
/// saved logins is denied at every level before any approval opens:
/// directly, through a symlink in the bruh's folder, in other letter case,
/// relative to the bruh's folder, and a search of the folder. A sibling
/// whose name only starts the same stays open.
#[cfg(unix)]
#[tokio::test]
async fn file_tools_never_reach_the_vault_folders() {
    let (dir, runtime) = runtime_with(&[agent("a")]).await;
    // The tempdir path is not canonical on macOS (/var is a symlink): the
    // roots are resolved when set, like the paths the tools name.
    let credentials = dir.path().join("credentials");
    let keychains = dir.path().join("Library").join("Keychains");
    let runtime = runtime.with_private_roots([credentials.clone(), keychains.clone()]);
    let run = claimed(&runtime, "a").await;
    let work = dir.path().join("bruhs").join("a").join("work");
    let sibling = dir.path().join("credentials-notes");
    for folder in [&credentials, &keychains, &work, &sibling] {
        std::fs::create_dir_all(folder).unwrap();
    }
    std::fs::write(credentials.join("vault.json"), "{}").unwrap();
    std::fs::write(keychains.join("login.keychain-db"), "x").unwrap();
    std::fs::write(sibling.join("notes.txt"), "notes").unwrap();
    std::os::unix::fs::symlink(&credentials, work.join("v")).unwrap();
    let at = |path: &std::path::Path| path.to_string_lossy().into_owned();
    let refused = [
        (
            "Read",
            json!({"file_path": at(&credentials.join("vault.json"))}),
        ),
        (
            "Read",
            json!({"file_path": at(&work.join("v").join("vault.json"))}),
        ),
        (
            "Read",
            json!({"file_path": at(&dir.path().join("CREDENTIALS").join("Vault.JSON"))}),
        ),
        ("Read", json!({"file_path": "v/vault.json"})),
        (
            "Edit",
            json!({"file_path": at(&work.join("v").join("vault.json")), "old_string": "{", "new_string": "["}),
        ),
        (
            "Write",
            json!({"file_path": at(&credentials.join("new.json")), "content": "x"}),
        ),
        ("Grep", json!({"pattern": ".", "path": at(&keychains)})),
        ("Glob", json!({"pattern": "**", "path": "v"})),
        ("LS", json!({"path": at(&credentials)})),
    ];
    let open = [
        ("Read", json!({"file_path": at(&sibling.join("notes.txt"))})),
        ("Grep", json!({"pattern": ".", "path": at(&work)})),
    ];
    for level in [AccessLevel::ReadOnly, AccessLevel::Ask, AccessLevel::Full] {
        for (tool, input) in &refused {
            let task = spawn_decide(
                &runtime,
                level,
                &run,
                &at(&work),
                tool,
                input.clone(),
                LONG_WAIT,
            );
            assert_eq!(
                decided(task).await,
                deny(VAULT_FILE_DENIAL),
                "{level:?} {tool} {input}"
            );
        }
        for (tool, input) in &open {
            let task = spawn_decide(
                &runtime,
                level,
                &run,
                &at(&work),
                tool,
                input.clone(),
                LONG_WAIT,
            );
            assert_eq!(
                decided(task).await,
                Some(crate::PermissionDecision::Allow),
                "{level:?} {tool} {input}"
            );
        }
    }
    assert!(approvals(&runtime, "a").await.is_empty());
}

/// A file tool call that reaches a browser profile through a symlink or a
/// change of case, or opens a hard-linked file in the bruhs' trees, is
/// denied at every level before any approval opens. A `browser` folder of
/// the bruh's own work stays open.
#[cfg(unix)]
#[tokio::test]
async fn file_calls_never_reach_a_browser_profile_by_another_name() {
    let (dir, runtime) = runtime_with(&[agent("a")]).await;
    let run = claimed(&runtime, "a").await;
    // The tempdir path is not canonical on macOS (/var is a symlink).
    let tree = dir.path().join("bruhs").join("a");
    let work = tree.join("work");
    let profile = tree.join("browser").join("Default");
    std::fs::create_dir_all(&profile).unwrap();
    std::fs::create_dir_all(work.join("x").join("browser")).unwrap();
    std::fs::write(profile.join("Cookies"), "cookie").unwrap();
    std::fs::write(work.join("x").join("browser").join("f"), "work").unwrap();
    std::os::unix::fs::symlink("../browser", work.join("b")).unwrap();
    std::fs::hard_link(profile.join("Cookies"), work.join("p")).unwrap();
    let at = |path: &std::path::Path| path.to_string_lossy().into_owned();
    let through_link = at(&work.join("b").join("Default").join("Cookies"));
    let other_case = at(&dir
        .path()
        .join("BRUHS")
        .join("A")
        .join("BROWSER")
        .join("Default")
        .join("Cookies"));
    let edit = |path: &str| json!({"file_path": path, "old_string": "c", "new_string": "d"});
    let refused = [
        ("Read", json!({"file_path": through_link}), PROFILE_DENIAL),
        ("Edit", edit(&through_link), PROFILE_DENIAL),
        (
            "Write",
            json!({"file_path": at(&work.join("b").join("Default").join("New")), "content": "x"}),
            PROFILE_DENIAL,
        ),
        ("Read", json!({"file_path": other_case}), PROFILE_DENIAL),
        (
            "Grep",
            json!({"pattern": ".", "path": at(&work.join("b"))}),
            PROFILE_DENIAL,
        ),
        // Relative to the bruh's folder.
        (
            "Glob",
            json!({"pattern": "**", "path": "b"}),
            PROFILE_DENIAL,
        ),
        (
            "Read",
            json!({"file_path": at(&tree.join("browser-cache").join("f"))}),
            PROFILE_DENIAL,
        ),
        (
            "Read",
            json!({"file_path": at(&work.join("p"))}),
            LINKED_FILE_DENIAL,
        ),
        ("Edit", edit(&at(&work.join("p"))), LINKED_FILE_DENIAL),
    ];
    let open = [
        (
            "Read",
            json!({"file_path": at(&work.join("x").join("browser").join("f"))}),
        ),
        ("Grep", json!({"pattern": ".", "path": at(&tree)})),
    ];
    for level in [AccessLevel::ReadOnly, AccessLevel::Ask, AccessLevel::Full] {
        for (tool, input, reason) in &refused {
            let task = spawn_decide(
                &runtime,
                level,
                &run,
                &at(&work),
                tool,
                input.clone(),
                LONG_WAIT,
            );
            assert_eq!(
                decided(task).await,
                deny(reason),
                "{level:?} {tool} {input}"
            );
        }
        for (tool, input) in &open {
            let task = spawn_decide(
                &runtime,
                level,
                &run,
                &at(&work),
                tool,
                input.clone(),
                LONG_WAIT,
            );
            assert_eq!(
                decided(task).await,
                Some(crate::PermissionDecision::Allow),
                "{level:?} {tool} {input}"
            );
        }
    }
    let task = spawn_decide(
        &runtime,
        AccessLevel::Full,
        &run,
        &at(&work),
        "Edit",
        edit(&at(&work.join("x").join("browser").join("f"))),
        LONG_WAIT,
    );
    assert_eq!(decided(task).await, Some(crate::PermissionDecision::Allow));
    assert!(approvals(&runtime, "a").await.is_empty());
}

#[tokio::test]
async fn ask_turns_a_write_into_an_approval_that_blocks_until_decided() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    let run = claimed(&runtime, "a").await;
    let input = json!({"command": "cargo test"});
    let task = spawn_decide(
        &runtime,
        AccessLevel::Ask,
        &run,
        &project(),
        "Bash",
        input.clone(),
        LONG_WAIT,
    );
    let item = waiting_approval(&runtime, "a").await;
    assert_eq!(item.kind, OutboxKind::Ask);
    assert_eq!(item.title, "Approval needed");
    assert_eq!(item.message, "Run cargo test");
    assert_eq!(item.run_id, run.id);
    let request = item.approval.clone().unwrap();
    assert_eq!(request.tool, "Bash");
    assert_eq!(request.summary, "Run cargo test");
    assert_eq!(request.fingerprint, approval_fingerprint("Bash", &input));
    assert_eq!(bruh_status(&runtime, "a").await, BruhStatus::NeedsYou);
    assert!(!task.is_finished());

    decide_approval(&runtime, &item.id, true).await;
    assert_eq!(decided(task).await, Some(crate::PermissionDecision::Allow));
    let item = approval_item(&runtime, &item.id).await;
    assert_eq!(item.approval.unwrap().state, ApprovalState::Approved);
    assert!(item.resolved_at.is_some() && item.read_at.is_some());
    // A live approval is used directly: no grant is left behind and no run queued.
    assert!(!take_grant(&runtime, "a", &request.fingerprint).await);
    assert_eq!(run_count(&runtime, "a").await, 1);
    // Deciding again the same way changes nothing; the other way conflicts.
    decide_approval(&runtime, &item.id, true).await;
    let conflict = runtime
        .command(ReactiveCommand::DecideApproval {
            id: item.id.clone(),
            approve: false,
        })
        .await
        .unwrap_err();
    assert_eq!(conflict.to_string(), "This request was already decided.");
}

#[tokio::test]
async fn a_denied_or_replied_approval_tells_the_agent_why() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    let run = claimed(&runtime, "a").await;
    let task = spawn_decide(
        &runtime,
        AccessLevel::Ask,
        &run,
        &project(),
        "Bash",
        json!({"command": "cargo test"}),
        LONG_WAIT,
    );
    let item = waiting_approval(&runtime, "a").await;
    decide_approval(&runtime, &item.id, false).await;
    assert_eq!(decided(task).await, deny(DENIED));
    let denied = approval_item(&runtime, &item.id).await;
    assert_eq!(denied.approval.unwrap().state, ApprovalState::Denied);
    assert!(denied.resolved_at.is_some());

    let task = spawn_decide(
        &runtime,
        AccessLevel::Ask,
        &run,
        &project(),
        "Bash",
        json!({"command": "cargo build"}),
        LONG_WAIT,
    );
    let item = waiting_approval(&runtime, "a").await;
    runtime
        .command(ReactiveCommand::AnswerOutbox {
            id: item.id.clone(),
            message: "use make".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        decided(task).await,
        deny("The owner denied this and replied: use make")
    );
    let replied = approval_item(&runtime, &item.id).await;
    assert_eq!(replied.approval.unwrap().state, ApprovalState::Denied);
    assert_eq!(replied.answer.as_deref(), Some("use make"));
    // The waiting run relays the reply itself, so nothing new is queued.
    assert_eq!(run_count(&runtime, "a").await, 1);

    // Dismissing an approval denies it rather than leaving the run waiting.
    let task = spawn_decide(
        &runtime,
        AccessLevel::Ask,
        &run,
        &project(),
        "Bash",
        json!({"command": "cargo doc"}),
        LONG_WAIT,
    );
    let item = waiting_approval(&runtime, "a").await;
    runtime
        .command(ReactiveCommand::DismissOutbox {
            id: item.id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(decided(task).await, deny(DENIED));
    let dismissed = approval_item(&runtime, &item.id).await;
    assert_eq!(dismissed.approval.unwrap().state, ApprovalState::Denied);
}

#[tokio::test]
async fn an_unanswered_approval_expires_and_a_late_approval_resumes_with_a_one_shot_grant() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    let run = claimed(&runtime, "a").await;
    let input = json!({"command": "cargo test"});
    let ask = |run: &ReactiveRun, timeout| {
        spawn_decide(
            &runtime,
            AccessLevel::Ask,
            run,
            &project(),
            "Bash",
            input.clone(),
            timeout,
        )
    };
    assert_eq!(decided(ask(&run, SHORT_WAIT)).await, deny(NOT_ANSWERED));
    let [item] = approvals(&runtime, "a").await.try_into().unwrap();
    assert_eq!(
        item.approval.as_ref().unwrap().state,
        ApprovalState::Expired
    );
    assert!(item.resolved_at.is_none() && item.is_open_approval());
    assert_eq!(bruh_status(&runtime, "a").await, BruhStatus::NeedsYou);

    finish(&runtime, &run.id, "Waiting on the owner's approval.").await;
    let snapshot = decide_approval(&runtime, &item.id, true).await;
    let queued: Vec<_> = snapshot
        .runs
        .iter()
        .filter(|queued| queued.agent_id == "a" && queued.status == RunStatus::Queued)
        .collect();
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued[0].trigger,
        Some(RunTrigger::Answer {
            outbox_id: item.id.clone()
        })
    );
    assert_eq!(
        queued[0].prompt,
        "The owner approved your request: Run cargo test. Do it now."
    );
    let approved = approval_item(&runtime, &item.id).await;
    assert_eq!(approved.approval.unwrap().state, ApprovalState::Approved);
    assert!(approved.resolved_at.is_some());

    let resumed = claim_one(&runtime).await.unwrap().run;
    assert_eq!(resumed.id, queued[0].id);
    assert_eq!(
        decided(ask(&resumed, LONG_WAIT)).await,
        Some(crate::PermissionDecision::Allow)
    );
    assert_eq!(approvals(&runtime, "a").await.len(), 1);
    // The grant was one shot: the same call asks again.
    assert_eq!(decided(ask(&resumed, SHORT_WAIT)).await, deny(NOT_ANSWERED));
    assert_eq!(approvals(&runtime, "a").await.len(), 2);
}

#[tokio::test]
async fn a_late_denial_resumes_the_run_without_a_grant() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    let run = claimed(&runtime, "a").await;
    let input = json!({"command": "cargo test"});
    let task = spawn_decide(
        &runtime,
        AccessLevel::Ask,
        &run,
        &project(),
        "Bash",
        input.clone(),
        SHORT_WAIT,
    );
    assert_eq!(decided(task).await, deny(NOT_ANSWERED));
    let [item] = approvals(&runtime, "a").await.try_into().unwrap();
    finish(&runtime, &run.id, "Waiting on the owner's approval.").await;
    let snapshot = decide_approval(&runtime, &item.id, false).await;
    let resumed = snapshot
        .runs
        .iter()
        .find(|queued| queued.status == RunStatus::Queued)
        .unwrap();
    assert_eq!(
        resumed.prompt,
        "The owner denied your request: Run cargo test. Continue without it."
    );
    assert!(!take_grant(&runtime, "a", &approval_fingerprint("Bash", &input)).await);
}

#[tokio::test]
async fn stop_releases_a_run_waiting_on_approval() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    let waiting = claimed(&runtime, "a").await;
    let task = spawn_decide(
        &runtime,
        AccessLevel::Ask,
        &waiting,
        &project(),
        "Bash",
        json!({"command": "cargo test"}),
        LONG_WAIT,
    );
    let item = waiting_approval(&runtime, "a").await;
    runtime
        .command(ReactiveCommand::StopBruh {
            agent_id: "a".into(),
        })
        .await
        .unwrap();
    assert_eq!(decided(task).await, None);
    let stopped = approval_item(&runtime, &item.id).await;
    assert_eq!(stopped.approval.unwrap().state, ApprovalState::Denied);
    assert!(stopped.resolved_at.is_some());
    assert_eq!(
        run(&runtime, &waiting.id).await.status,
        RunStatus::Cancelled
    );
}

#[tokio::test]
async fn cancelling_a_waiting_run_closes_its_approval() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    let waiting = claimed(&runtime, "a").await;
    let task = spawn_decide(
        &runtime,
        AccessLevel::Ask,
        &waiting,
        &project(),
        "Bash",
        json!({"command": "cargo test"}),
        LONG_WAIT,
    );
    let item = waiting_approval(&runtime, "a").await;
    runtime
        .command(ReactiveCommand::CancelRun {
            id: waiting.id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(decided(task).await, None);
    let closed = approval_item(&runtime, &item.id).await;
    assert_eq!(closed.approval.unwrap().state, ApprovalState::Denied);
    assert!(closed.resolved_at.is_some());
    assert_eq!(bruh_status(&runtime, "a").await, BruhStatus::Asleep);
}

#[tokio::test]
async fn full_access_allows_in_folder_writes_without_an_item() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    let folder = tempfile::tempdir().unwrap();
    let project_path = folder.path().to_string_lossy().into_owned();
    let run = claimed(&runtime, "a").await;
    let decide = |level, tool: &str, input| {
        spawn_decide(
            &runtime,
            level,
            &run,
            &project_path,
            tool,
            input,
            SHORT_WAIT,
        )
    };
    // A new file, named through the folder's canonical path, is still inside.
    let canonical = std::fs::canonicalize(folder.path()).unwrap();
    let new_file = canonical.join("src").join("a.rs");
    let edit = json!({"file_path": new_file.to_string_lossy()});
    assert_eq!(
        decided(decide(AccessLevel::Full, "Edit", edit.clone())).await,
        Some(crate::PermissionDecision::Allow)
    );
    // Claude's Full shell runs in the folder's sandbox, so it doesn't ask.
    assert_eq!(
        decided(decide(
            AccessLevel::Full,
            "Bash",
            json!({"command": "cargo test"})
        ))
        .await,
        Some(crate::PermissionDecision::Allow)
    );
    assert!(approvals(&runtime, "a").await.is_empty());
    assert_eq!(
        decided(decide(AccessLevel::ReadOnly, "Edit", edit)).await,
        deny(READ_ONLY_DENIAL)
    );
    assert!(approvals(&runtime, "a").await.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlink_out_of_the_folder_or_a_network_host_still_asks() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    let (folder, outside) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::os::unix::fs::symlink(outside.path(), folder.path().join("link")).unwrap();
    std::fs::write(outside.path().join("existing.txt"), "x").unwrap();
    let project_path = folder.path().to_string_lossy().into_owned();
    let run = claimed(&runtime, "a").await;
    let real = std::fs::canonicalize(outside.path()).unwrap();
    for (tool, through_link, target) in [
        ("Edit", "existing.txt", real.join("existing.txt")),
        ("Write", "new/dir/file.txt", real.join("new/dir/file.txt")),
    ] {
        let path = folder.path().join("link").join(through_link);
        let task = spawn_decide(
            &runtime,
            AccessLevel::Full,
            &run,
            &project_path,
            tool,
            json!({"file_path": path.to_string_lossy()}),
            SHORT_WAIT,
        );
        assert_eq!(decided(task).await, deny(NOT_ANSWERED), "{tool}");
        let item = approvals(&runtime, "a")
            .await
            .into_iter()
            .find(|item| item.approval.as_ref().unwrap().tool == tool)
            .unwrap();
        // The card names the real target, not the link.
        assert_eq!(item.message, format!("{tool} {}", target.display()));
    }
    let task = spawn_decide(
        &runtime,
        AccessLevel::Full,
        &run,
        &project_path,
        "SandboxNetworkAccess",
        json!({"host": "example.com"}),
        SHORT_WAIT,
    );
    assert_eq!(decided(task).await, deny(NOT_ANSWERED));
    assert!(approvals(&runtime, "a")
        .await
        .iter()
        .any(|item| item.message == "Reach example.com"));
}

#[tokio::test]
async fn approval_items_do_not_hide_the_runs_own_update() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    let run = claimed(&runtime, "a").await;
    let opened = run.clone();
    let id = runtime
        .call(move |connection| {
            store::crew::open_approval(connection, &opened, "Bash", &json!({"command": "ls"}))
        })
        .await
        .unwrap();
    decide_approval(&runtime, &id, true).await;
    finish(
        &runtime,
        &run.id,
        "Done.\n\n```bruhs-report\n{\"outcome\":\"notify\",\"title\":\"Tests pass\",\"message\":\"All green.\"}\n```",
    )
    .await;
    let outbox = runtime.snapshot().await.unwrap().outbox;
    let mine: Vec<_> = outbox.iter().filter(|item| item.run_id == run.id).collect();
    assert_eq!(mine.len(), 2);
    assert!(mine.iter().any(|item| item.id == id));
    assert!(mine
        .iter()
        .any(|item| item.kind == OutboxKind::Notify && item.title == "Tests pass"));
    // A fresh session hears what it told the owner, not its approval cards.
    chat(&runtime, "a", "Next").await;
    let claim = claim_one(&runtime).await.unwrap();
    assert!(claim.context.contains("Tests pass"));
    assert!(!claim.context.contains("Approval needed"));
}

#[tokio::test]
async fn deciding_an_approval_changes_the_live_signal() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    let run = claimed(&runtime, "a").await;
    let task = spawn_decide(
        &runtime,
        AccessLevel::Ask,
        &run,
        &project(),
        "Bash",
        json!({"command": "cargo test"}),
        LONG_WAIT,
    );
    let item = waiting_approval(&runtime, "a").await;
    let before = runtime.outbox_signal().await.unwrap();
    assert_eq!(before.open_questions, 1);
    assert_eq!(before.bruhs[0].status, BruhStatus::NeedsYou);
    decide_approval(&runtime, &item.id, true).await;
    let after = runtime.outbox_signal().await.unwrap();
    assert_eq!(after.open_questions, 0);
    assert_eq!(after.bruhs[0].status, BruhStatus::Working);
    assert_eq!(decided(task).await, Some(crate::PermissionDecision::Allow));
}

#[tokio::test]
async fn an_answer_after_expiry_resumes_even_while_the_run_wraps_up() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    let wrapping = claimed(&runtime, "a").await;
    let ask = |command: &str| {
        spawn_decide(
            &runtime,
            AccessLevel::Ask,
            &wrapping,
            &project(),
            "Bash",
            json!({ "command": command }),
            SHORT_WAIT,
        )
    };
    assert_eq!(decided(ask("cargo test")).await, deny(NOT_ANSWERED));
    assert_eq!(decided(ask("cargo build")).await, deny(NOT_ANSWERED));
    let expired = approvals(&runtime, "a").await;
    let test = expired
        .iter()
        .find(|item| item.message == "Run cargo test")
        .unwrap();
    let build = expired
        .iter()
        .find(|item| item.message == "Run cargo build")
        .unwrap();
    // The run is still Running, but nothing waits on these answers any more.
    assert_eq!(run(&runtime, &wrapping.id).await.status, RunStatus::Running);
    let snapshot = decide_approval(&runtime, &test.id, true).await;
    let resumed: Vec<_> = snapshot
        .runs
        .iter()
        .filter(|queued| queued.status == RunStatus::Queued)
        .collect();
    assert_eq!(resumed.len(), 1);
    assert_eq!(
        resumed[0].trigger,
        Some(RunTrigger::Answer {
            outbox_id: test.id.clone()
        })
    );
    let snapshot = runtime
        .command(ReactiveCommand::AnswerOutbox {
            id: build.id.clone(),
            message: "use make".into(),
        })
        .await
        .unwrap();
    let replies: Vec<_> = snapshot
        .runs
        .iter()
        .filter(|queued| {
            queued.status == RunStatus::Queued
                && queued.trigger
                    == Some(RunTrigger::Answer {
                        outbox_id: build.id.clone(),
                    })
        })
        .collect();
    assert_eq!(replies.len(), 1);
    assert!(replies[0].prompt.contains("use make"));
    // The approval left a one-shot grant for the resumed run.
    finish(&runtime, &wrapping.id, "Waiting on the owner's approval.").await;
    let next = claim_one(&runtime).await.unwrap().run;
    let task = spawn_decide(
        &runtime,
        AccessLevel::Ask,
        &next,
        &project(),
        "Bash",
        json!({"command": "cargo test"}),
        LONG_WAIT,
    );
    assert_eq!(decided(task).await, Some(crate::PermissionDecision::Allow));
}

#[tokio::test]
async fn set_vault_keeps_runs_sessions_and_other_fields() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    runtime
        .command(set_bruh("a", pandan(), AccessLevel::Full))
        .await
        .unwrap();
    runtime
        .command(ReactiveCommand::SetComputer {
            agent_id: "a".into(),
            on: true,
        })
        .await
        .unwrap();
    finished_with_session(&runtime).await;
    let queued = chat(&runtime, "a", "Queued").await;
    let set_vault = |agent_id: &str, entries: Vec<String>| ReactiveCommand::SetVault {
        agent_id: agent_id.into(),
        entries,
    };
    let snapshot = runtime
        .command(set_vault("a", vec!["v1-b".into(), "v1-a".into()]))
        .await
        .unwrap();
    let bruh = &snapshot.bruhs[0];
    assert_eq!(bruh.vault, ["v1-a", "v1-b"]);
    assert!(bruh.computer);
    assert_eq!(
        (bruh.access.level, bruh.look.face),
        (AccessLevel::Full, BruhColor::Pandan)
    );
    assert_eq!(run(&runtime, &queued.id).await.status, RunStatus::Queued);
    assert_eq!(session(&runtime, "a").await.as_deref(), Some("s1"));

    // An older-style SetBruh, which knows nothing of grants, keeps them.
    let snapshot = runtime
        .command(set_bruh("a", BruhLook::default(), AccessLevel::Full))
        .await
        .unwrap();
    assert_eq!(snapshot.bruhs[0].vault, ["v1-a", "v1-b"]);
    assert!(snapshot.bruhs[0].computer);

    let too_many = runtime
        .command(set_vault("a", (0..65).map(|i| format!("v{i}")).collect()))
        .await;
    assert!(
        matches!(&too_many, Err(ReactiveError::Invalid(message)) if message == "Grant at most 64 vault entries."),
        "{too_many:?}"
    );
    let bad = runtime.command(set_vault("a", vec!["../x".into()])).await;
    assert!(
        matches!(&bad, Err(ReactiveError::Invalid(message)) if message == "That isn't a vault entry."),
        "{bad:?}"
    );
    let missing = runtime.command(set_vault("nobody", vec![])).await;
    assert!(
        matches!(missing, Err(ReactiveError::NotFound(_))),
        "{missing:?}"
    );

    // Nothing above stopped the queued run or its session.
    let claim = claim_one(&runtime).await.unwrap();
    assert_eq!(claim.run.id, queued.id);
    assert_eq!(claim.bruh.vault, ["v1-a", "v1-b"]);
    assert_eq!(claim.run.session_id.as_deref(), Some("s1"));
    assert!(!claim.run.fresh_session);
}

#[tokio::test]
async fn set_vault_is_refused_for_report_agents() {
    use bruhs_models::weekly_reports::{WeeklyReportCommand, WeeklyReportPreset};
    let (dir, runtime) = runtime_with(&[]).await;
    let preset = WeeklyReportPreset {
        id: uuid::Uuid::new_v4().to_string(),
        project_path: dir.path().display().to_string(),
        model: bruhs_models::agent::DEFAULT_AGENT_MODEL.into(),
        local_git: true,
        github_pull_requests: false,
        github_deployments: false,
        scheduled: false,
        schedule: None,
    };
    runtime
        .weekly_report_command(WeeklyReportCommand::SavePreset {
            preset: preset.clone(),
        })
        .await
        .unwrap();
    let refused = runtime
        .command(ReactiveCommand::SetVault {
            agent_id: format!("weekly-report-{}", preset.id),
            entries: vec!["v1-g".into()],
        })
        .await;
    assert!(
        matches!(&refused, Err(ReactiveError::Conflict(message)) if message.contains("weekly report preset")),
        "{refused:?}"
    );
}

mod approval_lifecycle_tests;
