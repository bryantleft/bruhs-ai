//! Rooms: saving them, owner posts, the runs a post queues and message pages.

use super::crew_tests::{agent, claim_one, finish, record_session, run, runtime_with};
use super::{store, worker, ReactiveError, ReactiveRuntime};
use bruhs_models::crew::room::*;
use bruhs_models::crew::{BruhStatus, STOPPED_LINE};
use bruhs_models::reactive::*;

fn named(id: &str, name: &str) -> ReactiveAgent {
    ReactiveAgent {
        name: name.into(),
        ..agent(id)
    }
}

fn room(id: &str, members: &[&str], lead: Option<&str>) -> Room {
    Room {
        id: id.into(),
        name: format!("Room {id}"),
        members: members.iter().map(|id| (*id).to_string()).collect(),
        lead: lead.map(Into::into),
    }
}

async fn create(runtime: &ReactiveRuntime, room: Room) -> ReactiveSnapshot {
    runtime
        .command(ReactiveCommand::CreateRoom { room })
        .await
        .unwrap()
}

async fn update(runtime: &ReactiveRuntime, room: Room) -> ReactiveSnapshot {
    runtime
        .command(ReactiveCommand::UpdateRoom { room })
        .await
        .unwrap()
}

async fn post(runtime: &ReactiveRuntime, room_id: &str, text: &str) -> ReactiveSnapshot {
    runtime
        .command(ReactiveCommand::PostRoom {
            room_id: room_id.into(),
            text: text.into(),
        })
        .await
        .unwrap()
}

/// The room's newest page, oldest first.
async fn messages(runtime: &ReactiveRuntime, room_id: &str) -> Vec<RoomMessage> {
    let mut page = runtime.room_messages(room_id.into(), None).await.unwrap();
    page.reverse();
    page
}

fn in_room(run: &ReactiveRun, room_id: &str) -> bool {
    run.trigger.as_ref().and_then(RunTrigger::room_id) == Some(room_id)
}

/// The room's queued runs in the order they were queued.
fn queued_in<'a>(snapshot: &'a ReactiveSnapshot, room_id: &str) -> Vec<&'a ReactiveRun> {
    // The snapshot lists live runs newest first.
    let mut runs: Vec<&ReactiveRun> = snapshot
        .runs
        .iter()
        .filter(|run| run.status == RunStatus::Queued && in_room(run, room_id))
        .collect();
    runs.reverse();
    runs
}

fn agents_of(runs: &[&ReactiveRun]) -> Vec<String> {
    runs.iter().map(|run| run.agent_id.clone()).collect()
}

fn find<'a>(snapshot: &'a ReactiveSnapshot, id: &str) -> &'a ReactiveRun {
    snapshot.runs.iter().find(|run| run.id == id).unwrap()
}

#[tokio::test]
async fn rooms_are_created_updated_and_deleted_with_their_rules() {
    let preset = format!("weekly-report-{}", uuid::Uuid::new_v4());
    let (_dir, runtime) =
        runtime_with(&[named("a", "Ada"), named("b", "Bo"), agent(&preset)]).await;
    let refused = |result: Result<ReactiveSnapshot, ReactiveError>| match result {
        Err(error) => error,
        Ok(_) => panic!("the command was accepted"),
    };

    let error = refused(
        runtime
            .command(ReactiveCommand::CreateRoom {
                room: room("r", &["a", "x"], None),
            })
            .await,
    );
    assert!(matches!(error, ReactiveError::Invalid(ref m) if m == "Pick members from your crew."));
    let error = refused(
        runtime
            .command(ReactiveCommand::CreateRoom {
                room: room("r", &["a", &preset], None),
            })
            .await,
    );
    assert!(
        matches!(error, ReactiveError::Invalid(ref m) if m == "Pick members from your crew."),
        "a weekly report agent is not a bruh"
    );
    let error = refused(
        runtime
            .command(ReactiveCommand::CreateRoom {
                room: room("a/b", &["a"], None),
            })
            .await,
    );
    assert!(matches!(error, ReactiveError::Invalid(_)));

    let snapshot = create(&runtime, room("r", &["a", "b"], Some("a"))).await;
    assert_eq!(
        snapshot.rooms,
        [RoomState {
            room: room("r", &["a", "b"], Some("a")),
            last_seq: 0,
            working: Vec::new(),
        }]
    );
    let error = refused(
        runtime
            .command(ReactiveCommand::CreateRoom {
                room: room("r", &["b"], None),
            })
            .await,
    );
    assert!(
        matches!(error, ReactiveError::Conflict(ref m) if m == "A room with this ID already exists.")
    );
    assert_eq!(
        runtime.snapshot().await.unwrap().rooms[0].room,
        room("r", &["a", "b"], Some("a")),
        "a refused create changes nothing"
    );

    let error = refused(
        runtime
            .command(ReactiveCommand::UpdateRoom {
                room: room("missing", &["a"], None),
            })
            .await,
    );
    assert!(matches!(error, ReactiveError::NotFound(_)));
    let error = refused(
        runtime
            .command(ReactiveCommand::UpdateRoom {
                room: room("r", &["a"], Some("b")),
            })
            .await,
    );
    assert!(matches!(error, ReactiveError::Invalid(ref m) if m == "The lead must be a member."));
    let renamed = Room {
        name: "Launch".into(),
        ..room("r", &["b"], None)
    };
    let snapshot = update(&runtime, renamed.clone()).await;
    assert_eq!(snapshot.rooms[0].room, renamed);

    let snapshot = runtime
        .command(ReactiveCommand::DeleteRoom { id: "r".into() })
        .await
        .unwrap();
    assert!(snapshot.rooms.is_empty());
    let error = refused(
        runtime
            .command(ReactiveCommand::DeleteRoom { id: "r".into() })
            .await,
    );
    assert!(matches!(error, ReactiveError::NotFound(_)));
}

#[tokio::test]
async fn an_owner_post_queues_one_room_run_per_recipient_with_hop_zero() {
    let (_dir, runtime) =
        runtime_with(&[named("a", "Ada"), named("b", "Bo"), named("c", "Cy")]).await;
    create(&runtime, room("r", &["a", "b", "c"], Some("a"))).await;
    let snapshot = post(&runtime, "r", "@Bo and @cy look").await;
    let queued: Vec<_> = {
        let mut queued: Vec<_> = snapshot
            .runs
            .iter()
            .filter(|run| run.status == RunStatus::Queued)
            .collect();
        // The snapshot lists live runs newest first.
        queued.reverse();
        queued
    };
    assert_eq!(
        queued
            .iter()
            .map(|run| run.agent_id.as_str())
            .collect::<Vec<_>>(),
        ["b", "c"]
    );
    let owner = &messages(&runtime, "r").await[0];
    assert_eq!((owner.author.clone(), owner.seq), (RoomAuthor::Owner, 1));
    assert_eq!(owner.text, "@Bo and @cy look");
    assert!(queued.iter().all(|run| run.trigger
        == Some(RunTrigger::Room {
            room_id: "r".into(),
            message_id: owner.id.clone(),
            hop: 0,
            resumed: false,
        })));
    assert!(queued[0].prompt.contains("You are Bo."));
    assert!(queued[0]
        .prompt
        .contains("Owner (to you):\n> @Bo and @cy look"));
    assert!(queued[1].prompt.contains("You are Cy."));
}

#[tokio::test]
async fn a_post_without_a_mention_goes_to_the_lead_or_sits() {
    let (_dir, runtime) = runtime_with(&[named("a", "Ada"), named("b", "Bo")]).await;
    create(&runtime, room("led", &["a", "b"], Some("a"))).await;
    create(&runtime, room("open", &["a", "b"], None)).await;

    let snapshot = post(&runtime, "led", "Who takes this?").await;
    assert_eq!(agents_of(&queued_in(&snapshot, "led")), ["a"]);

    let snapshot = post(&runtime, "open", "Who takes this?").await;
    assert!(queued_in(&snapshot, "open").is_empty());
    let kept = messages(&runtime, "open").await;
    assert_eq!(kept.len(), 1);
    assert_eq!(
        (kept[0].author.clone(), kept[0].text.as_str()),
        (RoomAuthor::Owner, "Who takes this?")
    );
}

#[tokio::test]
async fn a_paused_or_off_member_gets_a_system_line_instead_of_a_run() {
    let (_dir, runtime) =
        runtime_with(&[named("a", "Ada"), named("b", "Bo"), named("c", "Cy")]).await;
    create(&runtime, room("r", &["a", "b", "c"], None)).await;
    runtime
        .command(ReactiveCommand::StopBruh {
            agent_id: "b".into(),
        })
        .await
        .unwrap();
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveAgent {
                agent: ReactiveAgent {
                    enabled: false,
                    ..named("c", "Cy")
                },
            },
        })
        .await
        .unwrap();

    let snapshot = post(&runtime, "r", "@Bo hi").await;
    assert!(queued_in(&snapshot, "r").is_empty());
    let lines = messages(&runtime, "r").await;
    let last = lines.last().unwrap();
    assert_eq!(last.author, RoomAuthor::System);
    assert_eq!(last.text, skipped_line("Bo", BruhStatus::Paused));
    assert_eq!(last.text, "Bo is paused, so it did not get this message.");

    let snapshot = post(&runtime, "r", "@Cy hi").await;
    assert!(queued_in(&snapshot, "r").is_empty());
    let last = messages(&runtime, "r").await.pop().unwrap();
    assert_eq!(
        (last.author, last.text),
        (RoomAuthor::System, skipped_line("Cy", BruhStatus::Off))
    );

    let snapshot = post(&runtime, "r", "@Ada @Bo hi").await;
    assert_eq!(agents_of(&queued_in(&snapshot, "r")), ["a"]);
    let lines = messages(&runtime, "r").await;
    let texts: Vec<(RoomAuthor, &str)> = lines
        .iter()
        .map(|line| (line.author.clone(), line.text.as_str()))
        .collect();
    assert_eq!(
        texts[4..],
        [
            (RoomAuthor::Owner, "@Ada @Bo hi"),
            (
                RoomAuthor::System,
                "Bo is paused, so it did not get this message."
            ),
        ]
    );
    assert_eq!(
        lines.iter().map(|line| line.seq).collect::<Vec<_>>(),
        [1, 2, 3, 4, 5, 6]
    );
}

#[tokio::test]
async fn a_member_already_queued_in_the_room_is_not_queued_twice() {
    let (_dir, runtime) = runtime_with(&[named("a", "Ada"), named("b", "Bo")]).await;
    create(&runtime, room("r", &["a", "b"], None)).await;
    create(&runtime, room("s", &["a", "b"], None)).await;

    let first = post(&runtime, "r", "@Bo one").await;
    let snapshot = post(&runtime, "r", "@Bo two").await;
    let queued = queued_in(&snapshot, "r");
    assert_eq!(agents_of(&queued), ["b"]);
    assert_eq!(queued[0].id, queued_in(&first, "r")[0].id);
    assert_eq!(
        messages(&runtime, "r").await.len(),
        2,
        "both posts are kept"
    );

    let snapshot = post(&runtime, "s", "@Bo three").await;
    assert_eq!(agents_of(&queued_in(&snapshot, "s")), ["b"]);
    assert_eq!(agents_of(&queued_in(&snapshot, "r")), ["b"]);

    // A run the owner's decision resumed keeps its own prompt, so a new post
    // queues beside it.
    let mut resumed = queued_in(&snapshot, "r")[0].clone();
    let Some(RunTrigger::Room { resumed: flag, .. }) = resumed.trigger.as_mut() else {
        panic!("not a room run");
    };
    *flag = true;
    runtime
        .call(move |connection| store::save_run(connection, &resumed))
        .await
        .unwrap();
    let snapshot = post(&runtime, "r", "@Bo four").await;
    assert_eq!(agents_of(&queued_in(&snapshot, "r")), ["b", "b"]);
}

#[tokio::test]
async fn renaming_a_room_or_changing_its_lead_keeps_its_runs_and_removing_a_member_cancels_only_its(
) {
    let (_dir, runtime) = runtime_with(&[named("a", "Ada"), named("b", "Bo")]).await;
    create(&runtime, room("r", &["a", "b"], Some("a"))).await;
    create(&runtime, room("s", &["a", "b"], None)).await;
    let chat = runtime
        .command(ReactiveCommand::Chat {
            agent_id: "b".into(),
            message: "Outside the room".into(),
            run_id: None,
        })
        .await
        .unwrap()
        .runs[0]
        .id
        .clone();
    let elsewhere = queued_in(&post(&runtime, "s", "@Bo there").await, "s")[0]
        .id
        .clone();
    let snapshot = post(&runtime, "r", "@Ada @Bo go").await;
    let ids: Vec<String> = queued_in(&snapshot, "r")
        .iter()
        .map(|run| run.id.clone())
        .collect();
    assert_eq!(agents_of(&queued_in(&snapshot, "r")), ["a", "b"]);

    let snapshot = update(
        &runtime,
        Room {
            name: "Renamed".into(),
            ..room("r", &["a", "b"], Some("b"))
        },
    )
    .await;
    let still: Vec<String> = queued_in(&snapshot, "r")
        .iter()
        .map(|run| run.id.clone())
        .collect();
    assert_eq!(still, ids, "a name or lead change cancels nothing");

    let snapshot = update(&runtime, room("r", &["a"], None)).await;
    let removed = find(&snapshot, &ids[1]);
    assert_eq!(removed.status, RunStatus::Cancelled);
    assert_eq!(removed.error.as_deref(), Some("Removed from the room."));
    assert_eq!(find(&snapshot, &ids[0]).status, RunStatus::Queued);
    assert_eq!(find(&snapshot, &chat).status, RunStatus::Queued);
    assert_eq!(find(&snapshot, &elsewhere).status, RunStatus::Queued);
}

#[tokio::test]
async fn deleting_a_room_cancels_its_runs_and_drops_its_messages() {
    let (_dir, runtime) = runtime_with(&[named("a", "Ada"), named("b", "Bo")]).await;
    create(&runtime, room("r", &["a", "b"], None)).await;
    create(&runtime, room("s", &["a", "b"], None)).await;
    let running = queued_in(&post(&runtime, "r", "@Ada start").await, "r")[0]
        .id
        .clone();
    let claim = claim_one(&runtime).await.unwrap();
    assert_eq!(claim.run.id, running);
    let queued = queued_in(&post(&runtime, "r", "@Bo next").await, "r")[0]
        .id
        .clone();
    let kept = queued_in(&post(&runtime, "s", "@Ada elsewhere").await, "s")[0]
        .id
        .clone();

    let snapshot = runtime
        .command(ReactiveCommand::DeleteRoom { id: "r".into() })
        .await
        .unwrap();
    for id in [&running, &queued] {
        let run = find(&snapshot, id);
        assert_eq!(run.status, RunStatus::Cancelled);
        assert_eq!(run.error.as_deref(), Some("The room was deleted."));
    }
    assert_eq!(find(&snapshot, &kept).status, RunStatus::Queued);
    assert!(messages(&runtime, "r").await.is_empty());
    assert_eq!(messages(&runtime, "s").await.len(), 1);
    assert_eq!(
        snapshot
            .rooms
            .iter()
            .map(|state| state.room.id.as_str())
            .collect::<Vec<_>>(),
        ["s"]
    );
}

#[tokio::test]
async fn room_messages_page_newest_first_by_rowid() {
    let (_dir, runtime) = runtime_with(&[named("a", "Ada")]).await;
    create(&runtime, room("r", &["a"], None)).await;
    create(&runtime, room("s", &["a"], None)).await;
    post(&runtime, "s", "Another room").await;
    for i in 0..=HISTORY_LIMIT {
        post(&runtime, "r", &format!("m{i}")).await;
    }

    let first = runtime.room_messages("r".into(), None).await.unwrap();
    assert_eq!(first.len(), HISTORY_LIMIT);
    assert_eq!(first[0].text, format!("m{HISTORY_LIMIT}"));
    assert_eq!(first[0].seq, 51);
    assert_eq!(first.last().unwrap().text, "m1");
    assert!(first.iter().all(|message| message.room_id == "r"));

    let before = first.last().unwrap().id.clone();
    let second = runtime
        .room_messages("r".into(), Some(before))
        .await
        .unwrap();
    assert_eq!(
        second
            .iter()
            .map(|message| (message.text.as_str(), message.seq))
            .collect::<Vec<_>>(),
        [("m0", 1)]
    );
    assert!(runtime
        .room_messages("missing".into(), None)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn rooms_ride_the_snapshot_and_the_signal() {
    let (_dir, runtime) = runtime_with(&[named("a", "Ada"), named("b", "Bo")]).await;
    create(&runtime, room("r", &["a", "b"], None)).await;
    let snapshot = post(&runtime, "r", "@Bo hi").await;
    assert_eq!(snapshot.rooms[0].last_seq, 1);
    assert_eq!(snapshot.rooms[0].working, ["b"]);
    assert_eq!(runtime.outbox_signal().await.unwrap().rooms, snapshot.rooms);

    let claim = claim_one(&runtime).await.unwrap();
    assert_eq!(
        runtime.snapshot().await.unwrap().rooms[0].working,
        ["b"],
        "a running room run is working too"
    );
    let snapshot = runtime
        .command(ReactiveCommand::CancelRun { id: claim.run.id })
        .await
        .unwrap();
    assert!(snapshot.rooms[0].working.is_empty());
    assert_eq!(runtime.outbox_signal().await.unwrap().rooms, snapshot.rooms);
}

#[tokio::test]
async fn a_full_queue_fails_the_post_and_keeps_nothing() {
    let (_dir, runtime) = runtime_with(&[named("a", "Ada"), named("b", "Bo")]).await;
    create(&runtime, room("r", &["a", "b"], None)).await;
    for i in 0..HISTORY_LIMIT {
        runtime
            .command(ReactiveCommand::Chat {
                agent_id: "a".into(),
                message: format!("Chat {i}"),
                run_id: None,
            })
            .await
            .unwrap();
    }
    let result = runtime
        .command(ReactiveCommand::PostRoom {
            room_id: "r".into(),
            text: "@Bo are you there".into(),
        })
        .await;
    assert!(matches!(result, Err(ReactiveError::Conflict(_))));
    assert!(messages(&runtime, "r").await.is_empty());
    assert_eq!(runtime.snapshot().await.unwrap().rooms[0].last_seq, 0);
}

// ── Room runs end in the room ──────────────────────────────────────────────

/// Finish a running run with `status` and `error`, as the worker does.
async fn end(runtime: &ReactiveRuntime, id: &str, status: RunStatus, error: Option<&str>) {
    let (id, error) = (id.to_owned(), error.map(str::to_owned));
    runtime
        .call(move |connection| store::finish_run(connection, &id, status, None, error, None))
        .await
        .unwrap();
}

/// Each line as (author, text, failed), oldest first.
async fn lines(runtime: &ReactiveRuntime, room_id: &str) -> Vec<(RoomAuthor, String, bool)> {
    messages(runtime, room_id)
        .await
        .into_iter()
        .map(|line| (line.author, line.text, line.failed))
        .collect()
}

fn bruh(id: &str) -> RoomAuthor {
    RoomAuthor::Bruh {
        agent_id: id.into(),
    }
}

#[tokio::test]
async fn a_room_run_shares_the_bruhs_session_and_names_the_room() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    runtime
        .command(ReactiveCommand::Chat {
            agent_id: "a".into(),
            message: "First".into(),
            run_id: None,
        })
        .await
        .unwrap();
    let chat = claim_one(&runtime).await.unwrap();
    record_session(&runtime, &chat.run.id).await;
    finish(&runtime, &chat.run.id, "Done").await;
    create(
        &runtime,
        Room {
            name: "Launch".into(),
            ..room("r", &["a"], None)
        },
    )
    .await;
    post(&runtime, "r", "@Agent a go").await;

    let claim = claim_one(&runtime).await.unwrap();
    assert!(in_room(&claim.run, "r"));
    assert_eq!(claim.run.session_id.as_deref(), Some("s1"));
    assert!(!claim.run.fresh_session);
    let turn = worker::turn(
        &claim.agent,
        &claim.bruh,
        &claim.run,
        &claim.context,
        std::path::Path::new("/home"),
    );
    assert!(turn.prompt.starts_with("Your name is Agent a.\n"));
    assert!(turn.prompt.contains("[Room \"Launch\" with "));
    assert_eq!(turn.session_id.as_deref(), Some("s1"));
}

#[tokio::test]
async fn a_completed_room_run_posts_its_reply_once() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    create(&runtime, room("r", &["a"], None)).await;
    post(&runtime, "r", "@Agent a go").await;
    let claim = claim_one(&runtime).await.unwrap();
    finish(&runtime, &claim.run.id, "Done.").await;

    let kept = messages(&runtime, "r").await;
    assert_eq!(kept.len(), 2);
    assert_eq!(kept[0].author, RoomAuthor::Owner);
    assert_eq!(
        (&kept[1].author, kept[1].text.as_str(), kept[1].failed),
        (&bruh("a"), "Done.", false)
    );
    assert_eq!(kept[1].run_id.as_deref(), Some(claim.run.id.as_str()));
    assert_eq!(kept[1].seq, 2);

    // A late result, stopped output or another save of the run adds nothing.
    finish(&runtime, &claim.run.id, "Again.").await;
    let id = claim.run.id.clone();
    runtime
        .call(move |connection| store::cancelled_output(connection, &id, "partial".into()))
        .await
        .unwrap();
    let done = run(&runtime, &claim.run.id).await;
    assert_eq!(done.status, RunStatus::Completed);
    runtime
        .call(move |connection| store::save_run(connection, &done))
        .await
        .unwrap();
    // `append` itself refuses a second message for the same run.
    let id = claim.run.id.clone();
    let again = runtime
        .call(move |connection| {
            store::rooms::append(
                connection,
                "r",
                RoomAuthor::System,
                "Twice".into(),
                store::rooms::Extra {
                    result_of: Some(id),
                    ..store::rooms::Extra::default()
                },
            )
        })
        .await
        .unwrap();
    assert!(again.is_none());
    assert_eq!(messages(&runtime, "r").await, kept);
}

#[tokio::test]
async fn an_empty_reply_reads_stopped() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    create(&runtime, room("r", &["a"], None)).await;
    post(&runtime, "r", "@Agent a go").await;
    let claim = claim_one(&runtime).await.unwrap();
    finish(&runtime, &claim.run.id, "  \n").await;
    assert_eq!(
        lines(&runtime, "r").await.pop().unwrap(),
        (bruh("a"), STOPPED_LINE.to_string(), false)
    );
}

#[tokio::test]
async fn a_reply_mentioning_a_member_hands_off_with_the_next_hop() {
    let (_dir, runtime) = runtime_with(&[agent("a"), agent("b"), agent("c")]).await;
    create(&runtime, room("r", &["a", "b"], Some("a"))).await;
    post(&runtime, "r", "@Agent a start").await;
    let claim = claim_one(&runtime).await.unwrap();
    finish(
        &runtime,
        &claim.run.id,
        "@Agent b take it, and @Agent a too",
    )
    .await;

    let reply = messages(&runtime, "r").await.pop().unwrap();
    assert_eq!(reply.author, bruh("a"));
    let snapshot = runtime.snapshot().await.unwrap();
    let queued = queued_in(&snapshot, "r");
    assert_eq!(
        agents_of(&queued),
        ["b"],
        "a bruh never hands work to itself, and a reply does not go to the lead"
    );
    assert_eq!(
        queued[0].trigger,
        Some(RunTrigger::Room {
            room_id: "r".into(),
            message_id: reply.id.clone(),
            hop: 1,
            resumed: false,
        })
    );

    // A reply that names nobody, or a non-member, hands off nothing.
    let claim = claim_one(&runtime).await.unwrap();
    assert_eq!(claim.run.agent_id, "b");
    finish(&runtime, &claim.run.id, "Done. @Agent c might know.").await;
    let snapshot = runtime.snapshot().await.unwrap();
    assert!(queued_in(&snapshot, "r").is_empty());
    assert!(snapshot
        .runs
        .iter()
        .all(|run| run.agent_id != "c" && run.status != RunStatus::Queued));
}

#[tokio::test]
async fn a_ping_pong_stops_after_the_hop_limit_with_one_system_line() {
    let (_dir, runtime) = runtime_with(&[agent("a"), agent("b")]).await;
    create(&runtime, room("r", &["a", "b"], None)).await;
    post(&runtime, "r", "@Agent a start").await;
    let mut claimed = 0;
    let mut hops = Vec::new();
    while let Some(claim) = claim_one(&runtime).await {
        claimed += 1;
        let Some(RunTrigger::Room { hop, .. }) = claim.run.trigger else {
            panic!("not a room run");
        };
        hops.push(hop);
        let other = if claim.run.agent_id == "a" {
            "@Agent b"
        } else {
            "@Agent a"
        };
        finish(&runtime, &claim.run.id, &format!("{other} your turn")).await;
        assert!(claimed <= 20);
    }
    assert_eq!(claimed, 1 + 6);
    assert_eq!(hops, [0, 1, 2, 3, 4, 5, 6]);
    let all = messages(&runtime, "r").await;
    assert_eq!(
        all.iter()
            .filter(|m| m.author == RoomAuthor::System)
            .count(),
        1
    );
    assert_eq!(all.last().unwrap().text, handoff_stopped_line());
    assert_eq!(all.len(), 1 + 7 + 1);
}

#[tokio::test]
async fn a_hand_off_that_cannot_queue_says_so_in_the_room() {
    let (_dir, runtime) = runtime_with(&[agent("a"), agent("b"), agent("c")]).await;
    create(&runtime, room("r", &["a", "b"], None)).await;
    post(&runtime, "r", "@Agent a start").await;
    let claim = claim_one(&runtime).await.unwrap();
    // Fill the queue so it stays full once a's run ends.
    let mut chat = None;
    for i in 0..HISTORY_LIMIT - 1 {
        let snapshot = runtime
            .command(ReactiveCommand::Chat {
                agent_id: "c".into(),
                message: format!("Chat {i}"),
                run_id: None,
            })
            .await
            .unwrap();
        chat = snapshot.runs.into_iter().find(|run| run.agent_id == "c");
    }
    let mut extra = chat.unwrap();
    extra.id = uuid::Uuid::new_v4().to_string();
    runtime
        .call(move |connection| store::save_run(connection, &extra))
        .await
        .unwrap();

    finish(&runtime, &claim.run.id, "@Agent b over to you").await;
    let tail = lines(&runtime, "r").await;
    let error = format!("This runtime has {HISTORY_LIMIT} unresolved runs. Review or cancel them before accepting more events.");
    assert_eq!(
        tail[1..],
        [
            (bruh("a"), "@Agent b over to you".to_string(), false),
            (
                RoomAuthor::System,
                handoff_failed_line("Agent b", &error),
                false
            ),
        ]
    );
    let snapshot = runtime.snapshot().await.unwrap();
    assert!(queued_in(&snapshot, "r").is_empty());
    assert_eq!(
        run(&runtime, &claim.run.id).await.status,
        RunStatus::Completed
    );
}

#[tokio::test]
async fn a_queued_room_run_sees_messages_posted_while_it_waited() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    create(&runtime, room("r", &["a"], None)).await;
    let queued = queued_in(&post(&runtime, "r", "@Agent a first").await, "r")[0].clone();
    post(&runtime, "r", "and also this").await;
    assert!(!queued.prompt.contains("and also this"));

    let claim = claim_one(&runtime).await.unwrap();
    assert_eq!(claim.run.id, queued.id);
    assert!(claim
        .run
        .prompt
        .contains("Owner (to you):\n> @Agent a first"));
    assert!(claim.run.prompt.contains("\nOwner:\n> and also this\n"));
    assert_eq!(run(&runtime, &queued.id).await.prompt, claim.run.prompt);
}

#[tokio::test]
async fn a_resumed_room_run_keeps_its_own_prompt() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    create(&runtime, room("r", &["a"], None)).await;
    let mut queued = queued_in(&post(&runtime, "r", "@Agent a first").await, "r")[0].clone();
    queued.prompt = "The owner approved your request: deploy. Do it now.".into();
    let Some(RunTrigger::Room { resumed, .. }) = queued.trigger.as_mut() else {
        panic!("not a room run");
    };
    *resumed = true;
    runtime
        .call(move |connection| store::save_run(connection, &queued))
        .await
        .unwrap();
    let claim = claim_one(&runtime).await.unwrap();
    assert_eq!(
        claim.run.prompt,
        "The owner approved your request: deploy. Do it now."
    );
}

#[tokio::test]
async fn a_room_run_whose_trigger_is_gone_keeps_its_stored_prompt() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    create(&runtime, room("r", &["a"], None)).await;
    let queued = queued_in(&post(&runtime, "r", "@Agent a first").await, "r")[0].clone();
    runtime
        .call(|connection| {
            connection.execute("DELETE FROM reactive_room_messages WHERE room_id='r'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let claim = claim_one(&runtime).await.unwrap();
    assert_eq!(claim.run.prompt, queued.prompt);
}

#[tokio::test]
async fn room_runs_are_never_batched_with_chat_runs() {
    let (_dir, runtime) = runtime_with(&[agent("a"), agent("b")]).await;
    create(&runtime, room("r", &["a", "b"], None)).await;
    let chat = |agent_id: &'static str, message: &'static str| {
        let runtime = &runtime;
        async move {
            runtime
                .command(ReactiveCommand::Chat {
                    agent_id: agent_id.into(),
                    message: message.into(),
                    run_id: None,
                })
                .await
                .unwrap()
                .runs
                .into_iter()
                .find(|run| run.owner_message.as_deref() == Some(message))
                .unwrap()
                .id
        }
    };

    // A room run first: it is claimed alone.
    let room_run = queued_in(&post(&runtime, "r", "@Agent a go").await, "r")[0]
        .id
        .clone();
    let a_chat = chat("a", "Chat for a").await;
    let claim = claim_one(&runtime).await.unwrap();
    assert_eq!(claim.run.id, room_run);
    assert!(claim.run.merged_runs.is_empty());
    assert_eq!(run(&runtime, &a_chat).await.status, RunStatus::Queued);

    // A chat first: later chats join it, the room run waits its turn.
    let first = chat("b", "One for b").await;
    let b_room = queued_in(&post(&runtime, "r", "@Agent b go").await, "r")
        .into_iter()
        .find(|run| run.agent_id == "b")
        .unwrap()
        .id
        .clone();
    let second = chat("b", "Two for b").await;
    let claim = claim_one(&runtime).await.unwrap();
    assert_eq!(claim.run.id, first);
    assert_eq!(claim.run.merged_runs, std::slice::from_ref(&second));
    assert_eq!(run(&runtime, &second).await.status, RunStatus::Merged);
    let waiting = run(&runtime, &b_room).await;
    assert_eq!(waiting.status, RunStatus::Queued);
    assert!(waiting.merged_into.is_none());
}

#[tokio::test]
async fn a_running_room_run_is_never_restarted_by_an_event() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
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
                id: "sub-a".into(),
                source_id: "inbox".into(),
                agent_id: "a".into(),
                instructions: "Summarize the event.".into(),
                enabled: true,
                auto_run: true,
                debounce_ms: 0,
                throttle_ms: 0,
                on_busy: BusyPolicy::Restart,
            },
        },
    ] {
        runtime
            .command(ReactiveCommand::Apply { change })
            .await
            .unwrap();
    }
    create(&runtime, room("r", &["a"], None)).await;
    post(&runtime, "r", "@Agent a go").await;
    let claim = claim_one(&runtime).await.unwrap();
    runtime
        .publish("inbox", "one", serde_json::json!({"n": 1}))
        .await
        .unwrap();
    let running = run(&runtime, &claim.run.id).await;
    assert_eq!(running.status, RunStatus::Running);
    let queued: Vec<ReactiveRun> = runtime
        .snapshot()
        .await
        .unwrap()
        .runs
        .into_iter()
        .filter(|run| run.status == RunStatus::Queued)
        .collect();
    assert_eq!(queued.len(), 1);
    assert!(queued[0].merged_runs.is_empty());
    assert!(!in_room(&queued[0], "r"));
}

#[tokio::test]
async fn a_failed_or_interrupted_room_run_posts_a_failed_line() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    create(&runtime, room("r", &["a"], None)).await;
    post(&runtime, "r", "@Agent a one").await;
    let claim = claim_one(&runtime).await.unwrap();
    end(&runtime, &claim.run.id, RunStatus::Failed, Some("boom")).await;
    let failed = messages(&runtime, "r").await.pop().unwrap();
    assert_eq!(
        (failed.author, failed.text.as_str(), failed.failed),
        (bruh("a"), "boom", true)
    );
    assert_eq!(failed.run_id.as_deref(), Some(claim.run.id.as_str()));

    post(&runtime, "r", "@Agent a two").await;
    let claim = claim_one(&runtime).await.unwrap();
    end(&runtime, &claim.run.id, RunStatus::Interrupted, None).await;
    assert_eq!(
        lines(&runtime, "r").await.pop().unwrap(),
        (bruh("a"), STOPPED_LINE.to_string(), true)
    );

    // A failure never hands off, even when its error names a member.
    post(&runtime, "r", "@Agent a three").await;
    let claim = claim_one(&runtime).await.unwrap();
    end(&runtime, &claim.run.id, RunStatus::Failed, Some("@Agent a")).await;
    assert!(queued_in(&runtime.snapshot().await.unwrap(), "r").is_empty());
}

#[tokio::test]
async fn a_cancelled_room_run_posts_a_stopped_line() {
    let (_dir, runtime) = runtime_with(&[agent("a"), agent("b")]).await;
    create(&runtime, room("r", &["a", "b"], None)).await;
    post(&runtime, "r", "@Agent a one").await;
    let claim = claim_one(&runtime).await.unwrap();
    runtime
        .command(ReactiveCommand::CancelRun {
            id: claim.run.id.clone(),
        })
        .await
        .unwrap();
    let stopped = messages(&runtime, "r").await.pop().unwrap();
    assert_eq!(
        (stopped.author, stopped.text.as_str()),
        (RoomAuthor::System, "Agent a did not answer. Stopped.")
    );
    assert_eq!(stopped.run_id.as_deref(), Some(claim.run.id.as_str()));
    // The worker's partial output after a cancel adds nothing.
    let id = claim.run.id.clone();
    runtime
        .call(move |connection| store::cancelled_output(connection, &id, "partial".into()))
        .await
        .unwrap();
    assert_eq!(messages(&runtime, "r").await.len(), 2);

    let queued = queued_in(&post(&runtime, "r", "@Agent b two").await, "r")[0]
        .id
        .clone();
    update(&runtime, room("r", &["a"], None)).await;
    assert_eq!(
        lines(&runtime, "r").await.pop().unwrap(),
        (
            RoomAuthor::System,
            stopped_line("Agent b", Some("Removed from the room.")),
            false
        )
    );
    assert_eq!(run(&runtime, &queued).await.status, RunStatus::Cancelled);
}

#[tokio::test]
async fn a_room_reply_is_not_repeated_as_an_update() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    create(&runtime, room("r", &["a"], None)).await;
    post(&runtime, "r", "@Agent a one").await;
    let claim = claim_one(&runtime).await.unwrap();
    finish(
        &runtime,
        &claim.run.id,
        "Done.\n\n```bruhs-report\n{\"outcome\":\"notify\",\"title\":\"Hi\",\"message\":\"Hello back.\"}\n```",
    )
    .await;
    assert!(runtime.snapshot().await.unwrap().outbox.is_empty());
    assert_eq!(
        lines(&runtime, "r").await.pop().unwrap(),
        (bruh("a"), "Done.".to_string(), false)
    );

    post(&runtime, "r", "@Agent a two").await;
    let claim = claim_one(&runtime).await.unwrap();
    finish(
        &runtime,
        &claim.run.id,
        "Done.\n\n```bruhs-report\n{\"outcome\":\"ask\",\"title\":\"Which card?\",\"message\":\"Work or personal?\"}\n```",
    )
    .await;
    let outbox = runtime.snapshot().await.unwrap().outbox;
    assert_eq!(outbox.len(), 1);
    assert_eq!(
        (outbox[0].kind, outbox[0].run_id.as_str()),
        (OutboxKind::Ask, claim.run.id.as_str())
    );

    // A chat run still gets its update.
    runtime
        .command(ReactiveCommand::Chat {
            agent_id: "a".into(),
            message: "Hi".into(),
            run_id: None,
        })
        .await
        .unwrap();
    let claim = claim_one(&runtime).await.unwrap();
    finish(
        &runtime,
        &claim.run.id,
        "Done.\n\n```bruhs-report\n{\"outcome\":\"notify\",\"title\":\"Hi\",\"message\":\"Hello back.\"}\n```",
    )
    .await;
    assert_eq!(runtime.snapshot().await.unwrap().outbox.len(), 2);
}

#[tokio::test]
async fn a_room_run_cannot_be_retried() {
    let (_dir, runtime) = runtime_with(&[agent("a")]).await;
    create(&runtime, room("r", &["a"], None)).await;
    let id = queued_in(&post(&runtime, "r", "@Agent a go").await, "r")[0]
        .id
        .clone();
    runtime
        .command(ReactiveCommand::CancelRun { id: id.clone() })
        .await
        .unwrap();
    let result = runtime
        .command(ReactiveCommand::RetryRun { id: id.clone() })
        .await;
    assert!(
        matches!(result, Err(ReactiveError::Conflict(ref m)) if m == "Post in the room again to retry.")
    );
    assert_eq!(run(&runtime, &id).await.status, RunStatus::Cancelled);
}

mod handoff_tests;
mod lifecycle_tests;
