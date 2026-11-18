//! A post for a bruh that already has a run queued in the room: the run
//! answers the newest post, and its hop count stays honest.

use super::*;

/// The room trigger of `run`: (message id, hop).
fn target(run: &ReactiveRun) -> (String, u32) {
    match &run.trigger {
        Some(RunTrigger::Room {
            message_id, hop, ..
        }) => (message_id.clone(), *hop),
        _ => panic!("not a room run"),
    }
}

#[tokio::test]
async fn an_owner_post_retargets_a_queued_hand_off_and_restarts_its_hop() {
    let (_dir, runtime) = runtime_with(&[named("a", "Ada"), named("b", "Bo")]).await;
    create(&runtime, room("r", &["a", "b"], None)).await;
    post(&runtime, "r", "@Ada start").await;
    let ada = claim_one(&runtime).await.unwrap();
    finish(&runtime, &ada.run.id, "@Bo do X").await;
    let handed = queued_in(&runtime.snapshot().await.unwrap(), "r")[0].clone();
    assert_eq!(target(&handed).1, 1);

    post(&runtime, "r", "@Bo stop, do Y instead").await;
    let owner = messages(&runtime, "r").await.pop().unwrap();
    assert_eq!(owner.author, RoomAuthor::Owner);
    let snapshot = runtime.snapshot().await.unwrap();
    let [queued] = queued_in(&snapshot, "r").try_into().unwrap();
    assert_eq!(queued.id, handed.id, "still one run for Bo");
    assert_eq!(
        target(queued),
        (owner.id.clone(), 0),
        "the owner's post is the one Bo answers, and it restarts the count"
    );
    assert!(queued.prompt.contains("stop, do Y instead"));

    let claim = claim_one(&runtime).await.unwrap();
    assert_eq!(claim.run.id, handed.id);
    assert_eq!(target(&claim.run), (owner.id, 0));
}

#[tokio::test]
async fn a_bruh_reply_deduped_into_a_queued_run_keeps_the_higher_hop() {
    let (_dir, runtime) = runtime_with(&[named("a", "Ada"), named("b", "Bo")]).await;
    create(&runtime, room("r", &["a", "b"], None)).await;
    post(&runtime, "r", "@Ada @Bo go").await;
    let ada = claim_one(&runtime).await.unwrap();
    assert_eq!(ada.run.agent_id, "a");
    finish(&runtime, &ada.run.id, "@Bo your turn").await;
    let reply = messages(&runtime, "r").await.pop().unwrap();
    assert_eq!(reply.author, bruh("a"));

    let snapshot = runtime.snapshot().await.unwrap();
    let [queued] = queued_in(&snapshot, "r").try_into().unwrap();
    assert_eq!(queued.agent_id, "b");
    assert_eq!(target(queued), (reply.id.clone(), 1));

    let claim = claim_one(&runtime).await.unwrap();
    assert_eq!(target(&claim.run), (reply.id, 1));
}
