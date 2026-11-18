//! Room runs that ask, wait on the owner, stop or lose their bruh.

use super::*;
use bruhs_models::crew::{transcript, ApprovalState, TranscriptItem};
use bruhs_models::reactive::MAX_TEXT_CHARS;
use serde_json::{json, Value};

/// Open an approval for `run` as the worker would; returns its id.
async fn open(runtime: &ReactiveRuntime, run: &ReactiveRun, tool: &str, input: Value) -> String {
    let (opening, tool) = (run.clone(), tool.to_owned());
    runtime
        .call(move |connection| store::crew::open_approval(connection, &opening, &tool, &input))
        .await
        .unwrap()
}

async fn item(runtime: &ReactiveRuntime, id: &str) -> OutboxItem {
    let id = id.to_owned();
    runtime
        .call(move |connection| store::crew::approval(connection, &id))
        .await
        .unwrap()
}

async fn expire(runtime: &ReactiveRuntime, id: &str) {
    let id = id.to_owned();
    runtime
        .call(move |connection| store::crew::expire_approval(connection, &id))
        .await
        .unwrap();
}

async fn decide(
    runtime: &ReactiveRuntime,
    id: &str,
    approve: bool,
) -> Result<ReactiveSnapshot, ReactiveError> {
    runtime
        .command(ReactiveCommand::DecideApproval {
            id: id.into(),
            approve,
        })
        .await
}

fn report(outcome: &str, title: &str, message: &str) -> String {
    format!(
        "```bruhs-report\n{}\n```",
        json!({"outcome": outcome, "title": title, "message": message})
    )
}

fn room_trigger(room_id: &str, message_id: &str, hop: u32, resumed: bool) -> Option<RunTrigger> {
    Some(RunTrigger::Room {
        room_id: room_id.into(),
        message_id: message_id.into(),
        hop,
        resumed,
    })
}

/// The open question a run left in the outbox.
fn question<'a>(snapshot: &'a ReactiveSnapshot, run_id: &str) -> &'a OutboxItem {
    snapshot
        .outbox
        .iter()
        .find(|item| item.run_id == run_id && item.approval.is_none())
        .unwrap()
}

#[tokio::test]
async fn an_approval_from_a_room_run_shows_in_the_room_and_the_bruhs_chat() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    create(&runtime, room("r", &["a"], None)).await;
    post(&runtime, "r", "@Agent a write the notes").await;
    let claim = claim_one(&runtime).await.unwrap();
    let id = open(
        &runtime,
        &claim.run,
        "Write",
        json!({"file_path": "/tmp/notes.md", "content": "hi"}),
    )
    .await;

    let kept = messages(&runtime, "r").await;
    assert_eq!(kept.len(), 2);
    let card = &kept[1];
    assert_eq!(card.author, bruh("a"));
    assert_eq!(card.outbox_id.as_deref(), Some(id.as_str()));
    assert_eq!(card.run_id.as_deref(), Some(claim.run.id.as_str()));
    assert!(!card.failed);
    let snapshot = runtime.snapshot().await.unwrap();
    let approval = snapshot.outbox.iter().find(|item| item.id == id).unwrap();
    assert!(!approval.message.is_empty());
    assert_eq!(card.text, approval.message, "the card reads as the summary");

    // The room draws it as a card, and so does the bruh's chat.
    assert!(room_lines(&kept, &snapshot.runs, &snapshot.outbox, "r")
        .iter()
        .any(|line| matches!(line, RoomLine::Card { item, .. } if item.id == id)));
    assert!(transcript("a", &snapshot.runs, &snapshot.outbox)
        .iter()
        .any(|line| matches!(line, TranscriptItem::Card { item } if item.id == id)));

    // A chat run's approval stays out of the room.
    finish(&runtime, &claim.run.id, "Done.").await;
    runtime
        .command(ReactiveCommand::Chat {
            agent_id: "a".into(),
            message: "Write more".into(),
            run_id: None,
        })
        .await
        .unwrap();
    let chat = claim_one(&runtime).await.unwrap();
    assert!(!in_room(&chat.run, "r"));
    open(&runtime, &chat.run, "Bash", json!({"command": "ls"})).await;
    assert_eq!(messages(&runtime, "r").await.len(), 3);
}

#[tokio::test]
async fn a_question_from_a_room_run_shows_in_the_room() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    create(&runtime, room("r", &["a"], None)).await;
    post(&runtime, "r", "@Agent a one").await;
    let claim = claim_one(&runtime).await.unwrap();
    let answer = format!(
        "Done.\n\n{}",
        report("ask", "Which card?", "Work or personal?")
    );
    finish(&runtime, &claim.run.id, &answer).await;

    let snapshot = runtime.snapshot().await.unwrap();
    let asked = question(&snapshot, &claim.run.id);
    assert_eq!(asked.kind, OutboxKind::Ask);
    let kept = messages(&runtime, "r").await;
    assert_eq!(kept.len(), 3, "the post, the question and the reply");
    let card = &kept[1];
    assert_eq!(
        (
            &card.author,
            card.text.as_str(),
            card.outbox_id.as_deref(),
            card.run_id.as_deref()
        ),
        (
            &bruh("a"),
            "Work or personal?",
            Some(asked.id.as_str()),
            Some(claim.run.id.as_str())
        )
    );
    assert_eq!(
        (kept[2].text.as_str(), kept[2].outbox_id.as_deref()),
        ("Done.", None)
    );
    assert!(room_lines(&kept, &snapshot.runs, &snapshot.outbox, "r")
        .iter()
        .any(|line| matches!(line, RoomLine::Card { item, .. } if item.id == asked.id)));
}

#[tokio::test]
async fn an_approval_then_a_question_from_one_room_run_both_reach_the_room() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    create(&runtime, room("r", &["a"], None)).await;
    post(&runtime, "r", "@Agent a one").await;
    let claim = claim_one(&runtime).await.unwrap();
    let approval = open(&runtime, &claim.run, "Bash", json!({"command": "ls"})).await;
    decide(&runtime, &approval, true).await.unwrap();
    let answer = format!(
        "Listed.\n\n{}",
        report("ask", "Which folder?", "Docs or src?")
    );
    finish(&runtime, &claim.run.id, &answer).await;

    let snapshot = runtime.snapshot().await.unwrap();
    let asked = question(&snapshot, &claim.run.id);
    assert_eq!(asked.message, "Docs or src?");
    let cards: Vec<_> = messages(&runtime, "r")
        .await
        .into_iter()
        .filter_map(|message| message.outbox_id)
        .collect();
    assert_eq!(cards, [approval, asked.id.clone()]);
}

#[tokio::test]
async fn an_empty_reply_with_a_report_says_what_the_report_says() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    create(&runtime, room("r", &["a"], None)).await;
    // Each round's lines after the owner's post, with the run's question.
    let reply = |report: String| {
        let runtime = &runtime;
        async move {
            post(runtime, "r", "@Agent a go").await;
            let before = messages(runtime, "r").await.len();
            let claim = claim_one(runtime).await.unwrap();
            finish(runtime, &claim.run.id, &report).await;
            let snapshot = runtime.snapshot().await.unwrap();
            let asked = snapshot
                .outbox
                .iter()
                .find(|item| item.run_id == claim.run.id)
                .map(|item| item.id.clone());
            let added: Vec<_> = messages(runtime, "r")
                .await
                .into_iter()
                .skip(before)
                .map(|line| (line.author, line.text, line.outbox_id))
                .collect();
            (added, asked)
        }
    };

    // A question is said once, as its card.
    let (added, asked) = reply(report("ask", "Which card?", "Work or personal?")).await;
    assert!(asked.is_some());
    assert_eq!(added, [(bruh("a"), "Work or personal?".to_string(), asked)]);
    let (added, asked) = reply(report("ask", "Which card?", "")).await;
    assert!(asked.is_some());
    assert_eq!(added, [(bruh("a"), "Which card?".to_string(), asked)]);
    // A room run's update gets no card, so its words go in the room.
    let (added, asked) = reply(report("notify", "Shipped", "It is live.")).await;
    assert_eq!(asked, None);
    assert_eq!(added, [(bruh("a"), "It is live.".to_string(), None)]);
    let (added, _) = reply(report("silent", "", "")).await;
    assert_eq!(added, [(bruh("a"), STOPPED_LINE.to_string(), None)]);
}

#[tokio::test]
async fn a_late_answer_after_the_bruh_left_the_room_goes_to_its_chat() {
    let (_dir, runtime) = runtime_with(&[agent("a"), agent("b")]).await;
    create(&runtime, room("r", &["a", "b"], None)).await;
    post(&runtime, "r", "@Agent a one").await;
    let claim = claim_one(&runtime).await.unwrap();
    let approval = open(&runtime, &claim.run, "Bash", json!({"command": "ls"})).await;
    expire(&runtime, &approval).await;
    finish(
        &runtime,
        &claim.run.id,
        &format!(
            "@Agent b look\n\n{}",
            report("ask", "Which card?", "Work or personal?")
        ),
    )
    .await;
    let asked = question(&runtime.snapshot().await.unwrap(), &claim.run.id).clone();
    // b's hand-off run is not part of this.
    let handed = queued_in(&runtime.snapshot().await.unwrap(), "r")[0]
        .id
        .clone();
    runtime
        .command(ReactiveCommand::CancelRun { id: handed })
        .await
        .unwrap();
    update(&runtime, room("r", &["b"], None)).await;
    let kept = messages(&runtime, "r").await;

    let snapshot = runtime
        .command(ReactiveCommand::AnswerOutbox {
            id: asked.id.clone(),
            message: "Work".into(),
        })
        .await
        .unwrap();
    let answered = snapshot
        .runs
        .iter()
        .find(|run| run.status == RunStatus::Queued)
        .unwrap()
        .clone();
    assert_eq!(
        answered.trigger,
        Some(RunTrigger::Answer {
            outbox_id: asked.id.clone()
        })
    );
    let snapshot = decide(&runtime, &approval, true).await.unwrap();
    let approved = snapshot
        .runs
        .iter()
        .find(|run| run.status == RunStatus::Queued && run.id != answered.id)
        .unwrap()
        .clone();
    assert_eq!(
        approved.trigger,
        Some(RunTrigger::Answer {
            outbox_id: approval.clone()
        })
    );

    // The two answers may run as one batch.
    while let Some(claim) = claim_one(&runtime).await {
        finish(&runtime, &claim.run.id, "@Agent b noted").await;
    }
    for id in [&answered.id, &approved.id] {
        assert!(!run(&runtime, id).await.status.is_pending());
    }
    assert_eq!(
        messages(&runtime, "r").await,
        kept,
        "the room hears nothing"
    );
    assert!(queued_in(&runtime.snapshot().await.unwrap(), "r").is_empty());

    // A deleted room resumes in the chat too.
    create(&runtime, room("r2", &["b"], None)).await;
    post(&runtime, "r2", "@Agent b two").await;
    let claim = claim_one(&runtime).await.unwrap();
    finish(&runtime, &claim.run.id, &report("ask", "When?", "Today?")).await;
    let asked = question(&runtime.snapshot().await.unwrap(), &claim.run.id).clone();
    runtime
        .command(ReactiveCommand::DeleteRoom { id: "r2".into() })
        .await
        .unwrap();
    let snapshot = runtime
        .command(ReactiveCommand::AnswerOutbox {
            id: asked.id.clone(),
            message: "Yes".into(),
        })
        .await
        .unwrap();
    assert!(snapshot
        .runs
        .iter()
        .any(|run| run.status == RunStatus::Queued
            && run.trigger
                == Some(RunTrigger::Answer {
                    outbox_id: asked.id.clone()
                })));
}

#[tokio::test]
async fn a_config_change_withdraws_a_room_runs_approvals() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    create(&runtime, room("r", &["a"], None)).await;
    post(&runtime, "r", "@Agent a one").await;
    let claim = claim_one(&runtime).await.unwrap();
    let waiting = open(&runtime, &claim.run, "Bash", json!({"command": "ls"})).await;
    let expired = open(&runtime, &claim.run, "Bash", json!({"command": "pwd"})).await;
    expire(&runtime, &expired).await;
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
    assert_eq!(
        run(&runtime, &claim.run.id).await.status,
        RunStatus::Cancelled
    );
    for id in [&waiting, &expired] {
        let withdrawn = item(&runtime, id).await;
        assert_eq!(withdrawn.approval.unwrap().state, ApprovalState::Denied);
        assert!(withdrawn.resolved_at.is_some());
    }
    assert!(matches!(
        decide(&runtime, &expired, true).await,
        Err(ReactiveError::Conflict(_))
    ));
}

#[tokio::test]
async fn a_late_approval_resumes_a_room_run_in_its_room() {
    let (_dir, runtime) = runtime_with(&[agent("a"), agent("b")]).await;
    create(&runtime, room("r", &["a", "b"], None)).await;
    post(&runtime, "r", "@Agent a start").await;
    let first = claim_one(&runtime).await.unwrap();
    finish(&runtime, &first.run.id, "@Agent b over to you").await;
    let claim = claim_one(&runtime).await.unwrap();
    assert_eq!(claim.run.agent_id, "b");
    let Some(RunTrigger::Room {
        message_id, hop, ..
    }) = claim.run.trigger.clone()
    else {
        panic!("not a room run");
    };
    assert_eq!(hop, 1);

    let id = open(
        &runtime,
        &claim.run,
        "Bash",
        json!({"command": "cargo test"}),
    )
    .await;
    expire(&runtime, &id).await;
    finish(&runtime, &claim.run.id, "Waiting on the owner's approval.").await;
    let snapshot = decide(&runtime, &id, true).await.unwrap();
    let [resumed] = queued_in(&snapshot, "r").try_into().unwrap();
    assert_eq!(resumed.agent_id, "b");
    assert_eq!(
        resumed.trigger,
        room_trigger("r", &message_id, 0, true),
        "the owner's decision restarts the hop count"
    );
    assert_eq!(
        resumed.prompt,
        "The owner approved your request: Run cargo test. Do it now."
    );

    let claim = claim_one(&runtime).await.unwrap();
    assert_eq!(claim.run.id, resumed.id);
    assert_eq!(claim.run.prompt, resumed.prompt, "the decision is kept");
    finish(&runtime, &claim.run.id, "Tests pass.").await;
    let last = messages(&runtime, "r").await.pop().unwrap();
    assert_eq!(
        (last.author, last.text.as_str(), last.run_id.as_deref()),
        (bruh("b"), "Tests pass.", Some(resumed.id.as_str()))
    );

    // A late denial resumes in the room too.
    post(&runtime, "r", "@Agent a again").await;
    let claim = claim_one(&runtime).await.unwrap();
    let Some(RunTrigger::Room { message_id, .. }) = claim.run.trigger.clone() else {
        panic!("not a room run");
    };
    let id = open(&runtime, &claim.run, "Bash", json!({"command": "rm -r x"})).await;
    expire(&runtime, &id).await;
    finish(&runtime, &claim.run.id, "Waiting.").await;
    let snapshot = decide(&runtime, &id, false).await.unwrap();
    let [resumed] = queued_in(&snapshot, "r").try_into().unwrap();
    assert_eq!(resumed.trigger, room_trigger("r", &message_id, 0, true));
    assert!(resumed.prompt.starts_with("The owner denied your request"));
}

#[tokio::test]
async fn a_question_card_counts_only_in_its_own_room() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    create(&runtime, room("r", &["a"], None)).await;
    create(&runtime, room("s", &["a"], None)).await;
    post(&runtime, "r", "@Agent a one").await;
    let claim = claim_one(&runtime).await.unwrap();
    finish(
        &runtime,
        &claim.run.id,
        &report("ask", "Which card?", "Work or personal?"),
    )
    .await;
    let mut asked = run(&runtime, &claim.run.id).await;
    let here = asked.clone();
    assert!(runtime
        .call(move |connection| store::rooms::question_only(connection, &here))
        .await
        .unwrap());
    let Some(RunTrigger::Room { room_id, .. }) = asked.trigger.as_mut() else {
        panic!("not a room run");
    };
    *room_id = "s".into();
    assert!(
        !runtime
            .call(move |connection| store::rooms::question_only(connection, &asked))
            .await
            .unwrap(),
        "the card is in room r, not s"
    );
}

#[tokio::test]
async fn a_late_answer_resumes_a_room_question_in_its_room() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    create(&runtime, room("r", &["a"], None)).await;
    let message_id = queued_in(&post(&runtime, "r", "@Agent a one").await, "r")[0]
        .trigger
        .as_ref()
        .and_then(|trigger| match trigger {
            RunTrigger::Room { message_id, .. } => Some(message_id.clone()),
            _ => None,
        })
        .unwrap();
    let claim = claim_one(&runtime).await.unwrap();
    finish(
        &runtime,
        &claim.run.id,
        &report("ask", "Which card?", "Work or personal?"),
    )
    .await;
    let asked = question(&runtime.snapshot().await.unwrap(), &claim.run.id).clone();

    let snapshot = runtime
        .command(ReactiveCommand::AnswerOutbox {
            id: asked.id.clone(),
            message: "Work".into(),
        })
        .await
        .unwrap();
    let [resumed] = queued_in(&snapshot, "r").try_into().unwrap();
    assert_eq!(resumed.trigger, room_trigger("r", &message_id, 0, true));
    assert!(resumed.prompt.contains("Their reply:\nWork"));

    // A new post while the resume waits still reaches the bruh.
    let snapshot = post(&runtime, "r", "@Agent a also").await;
    assert_eq!(queued_in(&snapshot, "r").len(), 2);

    let claim = claim_one(&runtime).await.unwrap();
    assert_eq!(claim.run.id, resumed.id);
    assert_eq!(claim.run.prompt, resumed.prompt);
    finish(&runtime, &claim.run.id, "Got it.").await;
    let last = messages(&runtime, "r").await.pop().unwrap();
    assert_eq!(
        (last.author, last.text.as_str(), last.run_id.as_deref()),
        (bruh("a"), "Got it.", Some(resumed.id.as_str()))
    );
}

#[tokio::test]
async fn stop_all_ends_room_runs_with_one_line_each_and_no_new_hand_offs() {
    let (_dir, runtime) = runtime_with(&[agent("a"), agent("b")]).await;
    create(&runtime, room("r", &["a", "b"], None)).await;
    post(&runtime, "r", "@Agent a @Agent b go").await;
    let claim = claim_one(&runtime).await.unwrap();
    assert_eq!(claim.run.agent_id, "a");
    let waiting = open(&runtime, &claim.run, "Bash", json!({"command": "ls"})).await;
    assert_eq!(queued_in(&runtime.snapshot().await.unwrap(), "r").len(), 1);

    runtime.command(ReactiveCommand::StopAll).await.unwrap();
    let after_stop = lines(&runtime, "r").await;
    let stopped: Vec<_> = after_stop
        .iter()
        .filter(|(author, ..)| *author == RoomAuthor::System)
        .collect();
    assert_eq!(
        stopped,
        [
            &(
                RoomAuthor::System,
                "Agent a did not answer. Stopped by the owner.".to_string(),
                false
            ),
            &(
                RoomAuthor::System,
                "Agent b did not answer. Stopped by the owner.".to_string(),
                false
            ),
        ]
    );
    let denied = item(&runtime, &waiting).await;
    assert_eq!(denied.approval.unwrap().state, ApprovalState::Denied);
    assert!(denied.resolved_at.is_some());

    // The provider's late answer changes nothing and hands nothing off.
    finish(&runtime, &claim.run.id, "@Agent b keep going").await;
    assert_eq!(lines(&runtime, "r").await, after_stop);
    assert!(queued_in(&runtime.snapshot().await.unwrap(), "r").is_empty());
    assert_eq!(
        run(&runtime, &claim.run.id).await.status,
        RunStatus::Cancelled
    );
}

#[tokio::test]
async fn deleting_a_bruh_takes_it_out_of_every_room() {
    let (_dir, runtime) = runtime_with(&[agent("a"), agent("b")]).await;
    create(&runtime, room("r1", &["a", "b"], Some("a"))).await;
    create(&runtime, room("r2", &["a"], None)).await;
    create(&runtime, room("r3", &["b"], Some("b"))).await;
    post(&runtime, "r1", "@Agent a hi").await;

    let snapshot = runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::DeleteAgent { id: "a".into() },
        })
        .await
        .unwrap();
    let rooms: Vec<_> = snapshot.rooms.iter().map(|state| &state.room).collect();
    assert_eq!(
        rooms,
        [
            &room("r1", &["b"], None),
            &room("r2", &[], None),
            &room("r3", &["b"], Some("b")),
        ]
    );
    let left = (RoomAuthor::System, left_line("Agent a"), false);
    let r1 = lines(&runtime, "r1").await;
    assert_eq!(r1.last(), Some(&left));
    assert_eq!(
        r1[r1.len() - 2].1,
        stopped_line(
            "Agent a",
            Some("Configuration changed or was disabled. Review the new setup before starting another run.")
        ),
        "its run stops first"
    );
    assert_eq!(lines(&runtime, "r2").await, [left]);
    assert!(lines(&runtime, "r3").await.is_empty());
}

#[tokio::test]
async fn a_cancelled_room_run_withdraws_its_approvals() {
    let (_dir, runtime) = runtime_with(&[agent("a"), agent("b")]).await;
    create(&runtime, room("r", &["a", "b"], None)).await;

    // Taken out of the room while it waits.
    post(&runtime, "r", "@Agent a one").await;
    let claim = claim_one(&runtime).await.unwrap();
    let waiting = open(&runtime, &claim.run, "Bash", json!({"command": "ls"})).await;
    update(&runtime, room("r", &["b"], None)).await;
    let withdrawn = item(&runtime, &waiting).await;
    assert_eq!(withdrawn.approval.unwrap().state, ApprovalState::Denied);
    assert!(withdrawn.resolved_at.is_some());

    // The room is deleted after the wait ran out.
    post(&runtime, "r", "@Agent b two").await;
    let claim = claim_one(&runtime).await.unwrap();
    let expired = open(&runtime, &claim.run, "Bash", json!({"command": "ls"})).await;
    expire(&runtime, &expired).await;
    runtime
        .command(ReactiveCommand::DeleteRoom { id: "r".into() })
        .await
        .unwrap();
    let withdrawn = item(&runtime, &expired).await;
    assert_eq!(withdrawn.approval.unwrap().state, ApprovalState::Denied);
    assert!(withdrawn.resolved_at.is_some());
    // So a late answer resumes nothing.
    assert!(matches!(
        decide(&runtime, &expired, true).await,
        Err(ReactiveError::Conflict(_))
    ));
    assert!(runtime
        .snapshot()
        .await
        .unwrap()
        .runs
        .iter()
        .all(|run| run.status != RunStatus::Queued));
}

#[tokio::test]
async fn a_failed_room_line_never_says_to_retry() {
    let (dir, runtime) = runtime_with(&[agent("a")]).await;
    create(&runtime, room("r", &["a"], None)).await;
    let failed_line = (bruh("a"), STOPPED_LINE.to_string(), true);

    post(&runtime, "r", "@Agent a one").await;
    let claim = claim_one(&runtime).await.unwrap();
    finish(&runtime, &claim.run.id, &"x".repeat(MAX_TEXT_CHARS + 1)).await;
    assert_eq!(run(&runtime, &claim.run.id).await.status, RunStatus::Failed);
    assert_eq!(lines(&runtime, "r").await.pop().unwrap(), failed_line);

    post(&runtime, "r", "@Agent a two").await;
    let claim = claim_one(&runtime).await.unwrap();
    end(
        &runtime,
        &claim.run.id,
        RunStatus::Failed,
        Some("The provider ended without a completion receipt. Review the partial output before retrying."),
    )
    .await;
    assert_eq!(lines(&runtime, "r").await.pop().unwrap(), failed_line);

    // A restart interrupts the running one.
    post(&runtime, "r", "@Agent a three").await;
    let claim = claim_one(&runtime).await.unwrap();
    drop(runtime);
    let runtime = ReactiveRuntime::open(dir.path().join("reactive.sqlite3"))
        .await
        .unwrap();
    assert_eq!(
        run(&runtime, &claim.run.id).await.status,
        RunStatus::Interrupted
    );
    assert_eq!(lines(&runtime, "r").await.pop().unwrap(), failed_line);
    assert!(lines(&runtime, "r")
        .await
        .iter()
        .all(|(_, text, _)| !text.to_lowercase().contains("retry")));
}
