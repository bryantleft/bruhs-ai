//! Rooms: saving them, owner posts, the runs a post queues and message pages.

use super::crew_tests::{agent, claim_one, runtime_with};
use super::{store, ReactiveError, ReactiveRuntime};
use bruhs_models::crew::room::*;
use bruhs_models::crew::BruhStatus;
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
        .contains("Owner (to you): @Bo and @cy look"));
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
