//! Bruh profiles: looks, access and effort beside agents, bruh states and run pages.

use super::{store, ReactiveError, ReactiveRuntime};
use bruhs_models::crew::*;
use bruhs_models::reactive::*;
use bruhs_models::EpochMs;
use serde_json::json;

const ACCESS_CHANGED: &str = "Access changed. Review the new access before starting another run.";

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

async fn runtime_with(agents: &[ReactiveAgent]) -> (tempfile::TempDir, ReactiveRuntime) {
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

async fn claim_one(runtime: &ReactiveRuntime) -> Option<store::Claim> {
    runtime.call(store::claim_next).await.unwrap()
}

async fn finish(runtime: &ReactiveRuntime, id: &str, answer: &str) {
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
