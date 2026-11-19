use std::{path::Path, sync::Arc, time::Duration};

use bruhs_models::crew::{
    handed_back_note, ACTION_STOPPED, CLOSED_BY_OWNER, COMPUTERS_FULL, COMPUTER_OFF_DENIAL,
    LIVE_VIEW_ENDED, LOCAL_HOST_DENIAL, NOT_FOR_BRUHS, OWNER_HAS_COMPUTER, PAGE_CHANGED,
    PAGE_UNKNOWN, TAKE_CONTROL_FIRST, TURN_ON_IN_SETTINGS, USE_THE_WINDOW,
};
use bruhs_models::local_tools::{
    BrowserAction, ComputerInput, ComputerRequest, Control, GrantRequest, MouseButton, MouseKind,
    ToolScope,
};

use super::{BruhComputer, LocalToolsHost};
use crate::local_tools::{screencast::fake_chrome::FakeChrome, ComputerWatch, ToolError};

/// A bruh whose profile is `<home>/bruhs/<id>/browser`.
fn bruh_in(home: &Path, id: &str) -> BruhComputer {
    BruhComputer {
        agent_id: id.into(),
        name: id.to_uppercase(),
        paused: false,
        profile_dir: home.join("bruhs").join(id).join("browser"),
    }
}

fn navigate(url: &str) -> BrowserAction {
    BrowserAction::Navigate { url: url.into() }
}

/// Lets spawned tasks run, without moving a paused clock.
async fn settle() {
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
}

/// Waits for `check`, failing after a few seconds of real time.
async fn eventually(mut check: impl FnMut() -> bool) {
    for _ in 0..500 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the condition never held");
}

#[tokio::test]
async fn sync_turns_computers_on_and_off() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    assert_eq!(host.computer_state("a").await.unwrap().session_id, None);
    host.bruh_command("a", BrowserAction::Snapshot)
        .await
        .unwrap();
    let launch = &fake.launches()[0];
    assert!(
        launch.headless
            && launch.url == "about:blank"
            && launch.profile_dir.ends_with("bruhs/a/browser")
    );
    assert!(host.computer_state("a").await.unwrap().session_id.is_some());
    host.sync_bruhs(vec![]).await;
    assert_eq!(fake.closes("a"), 1);
    assert_eq!(
        host.bruh_command("a", BrowserAction::Snapshot)
            .await
            .unwrap_err()
            .to_string(),
        COMPUTER_OFF_DENIAL
    );
    assert!(matches!(
        host.computer_state("a").await,
        Err(ToolError::NotFound(message)) if message == TURN_ON_IN_SETTINGS
    ));
}

/// Where the guard can't read this computer's addresses or recover a
/// crashed profile (off Unix), a Computer says so and never launches.
#[tokio::test]
async fn a_system_without_the_guard_opens_no_bruh_browser() {
    use bruhs_models::crew::COMPUTER_UNSUPPORTED;
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages_refusing(COMPUTER_UNSUPPORTED);
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    let state = host.computer_state("a").await.unwrap();
    assert_eq!(state.error.as_deref(), Some(COMPUTER_UNSUPPORTED));
    assert!(matches!(
        host.open_computer("a").await,
        Err(ToolError::Unavailable(message)) if message == COMPUTER_UNSUPPORTED
    ));
    assert_eq!(
        host.bruh_command("a", BrowserAction::Snapshot)
            .await
            .unwrap_err()
            .to_string(),
        COMPUTER_UNSUPPORTED
    );
    assert!(fake.launches().is_empty());
    assert_eq!(
        host.computer_state("a").await.unwrap().error.as_deref(),
        Some(COMPUTER_UNSUPPORTED)
    );
    // The production host refuses exactly off Unix.
    assert_eq!(
        super::platform_refusal(),
        (!cfg!(unix)).then_some(COMPUTER_UNSUPPORTED)
    );
}

#[tokio::test]
async fn a_bruh_browser_starts_on_first_use_and_reopens_at_its_last_url() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    assert!(fake.launches().is_empty(), "syncing starts no browser");
    host.bruh_command("a", navigate("https://a.test/x"))
        .await
        .unwrap();
    assert_eq!(
        host.computer_state("a").await.unwrap().url.as_deref(),
        Some("https://a.test/x")
    );
    let closed = host.close_computer("a").await.unwrap();
    assert_eq!(closed.session_id, None);
    assert_eq!(fake.closes("a"), 1);
    host.bruh_command("a", BrowserAction::Snapshot)
        .await
        .unwrap();
    let launches = fake.launches();
    assert_eq!(launches.len(), 2);
    assert_eq!(launches[1].url, "https://a.test/x");
    assert_eq!(
        fake.actions("a"),
        [navigate("https://a.test/x"), BrowserAction::Snapshot]
    );
}

#[tokio::test]
async fn bruhs_cannot_script_read_logs_or_reach_this_computer() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    for action in [
        BrowserAction::Evaluate {
            expression: "document.cookie".into(),
        },
        BrowserAction::Console,
        BrowserAction::Network,
    ] {
        let error = host.bruh_command("a", action).await.unwrap_err();
        assert!(
            matches!(&error, ToolError::Refused(message) if message == NOT_FOR_BRUHS),
            "{error:?}"
        );
    }
    let error = host
        .bruh_command("a", navigate("http://localhost:3000"))
        .await
        .unwrap_err();
    assert!(
        matches!(&error, ToolError::Refused(message) if message == LOCAL_HOST_DENIAL),
        "{error:?}"
    );
    let error = host
        .bruh_command("a", BrowserAction::ClickAt { x: 5000.0, y: 10.0 })
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::Invalid(_)), "{error:?}");
    assert!(fake.launches().is_empty());
    assert!(fake.actions("a").is_empty());
}

#[tokio::test]
async fn at_most_four_bruh_browsers_run_at_once() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    let ids = ["a", "b", "c", "d", "e"];
    host.sync_bruhs(ids.iter().map(|id| bruh_in(home.path(), id)).collect())
        .await;
    for id in &ids[..4] {
        host.open_computer(id).await.unwrap();
    }
    let error = host.open_computer("e").await.unwrap_err();
    assert!(
        matches!(&error, ToolError::Refused(message) if message == COMPUTERS_FULL),
        "{error:?}"
    );
    assert_eq!(
        host.computer_state("e").await.unwrap().error.as_deref(),
        Some(COMPUTERS_FULL)
    );
    host.close_computer("a").await.unwrap();
    let opened = host.open_computer("e").await.unwrap();
    assert!(opened.session_id.is_some());
    assert_eq!(opened.error, None);
    assert_eq!(fake.launches().len(), 5);
}

#[tokio::test]
async fn concurrent_first_uses_never_open_a_fifth_browser() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    let ids = ["a", "b", "c", "d", "e", "f"];
    host.sync_bruhs(ids.iter().map(|id| bruh_in(home.path(), id)).collect())
        .await;
    let results = futures_util::future::join_all(
        ids.iter()
            .map(|id| host.bruh_command(id, BrowserAction::Snapshot)),
    )
    .await;
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 4);
    assert_eq!(fake.launches().len(), 4);
}

#[tokio::test(start_paused = true)]
async fn an_idle_browser_closes_after_ten_minutes_unless_watched_or_held() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    let reaper = host.start_idle_reaper();
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    host.bruh_command("a", BrowserAction::Snapshot)
        .await
        .unwrap();
    tokio::time::advance(Duration::from_secs(9 * 60)).await;
    settle().await;
    assert!(host.computer_state("a").await.unwrap().session_id.is_some());
    assert_eq!(fake.closes("a"), 0);
    tokio::time::advance(Duration::from_secs(2 * 60)).await;
    settle().await;
    assert_eq!(host.computer_state("a").await.unwrap().session_id, None);
    assert_eq!(fake.closes("a"), 1);
    reaper.abort();
}

/// The owner takes control of an idle open page just as the reaper looks:
/// the page stays open for them.
#[tokio::test(start_paused = true)]
async fn the_reaper_never_closes_a_page_the_owner_just_took() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    host.bruh_command("a", BrowserAction::Snapshot)
        .await
        .unwrap();
    tokio::time::advance(Duration::from_secs(11 * 60)).await;
    // Between the reaper's first look and its page lock, as Take control
    // on an open page takes no lock.
    *host.computers.before_reap.lock().unwrap() =
        Some(Box::new(|slot| slot.set_control(Control::Owner, None)));
    host.close_idle().await;
    assert_eq!(fake.closes("a"), 0);
    let state = host.computer_state("a").await.unwrap();
    assert!(state.session_id.is_some());
    assert_eq!(state.control, Control::Owner);
}

/// A failed step isn't use: a bruh retrying a broken page doesn't keep its
/// browser from closing when idle.
#[tokio::test(start_paused = true)]
async fn a_failed_step_doesnt_keep_the_browser_awake() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    host.bruh_command("a", BrowserAction::Snapshot)
        .await
        .unwrap();
    tokio::time::advance(Duration::from_secs(9 * 60)).await;
    fake.fail_next_action("a");
    assert!(host
        .bruh_command("a", BrowserAction::Snapshot)
        .await
        .is_err());
    tokio::time::advance(Duration::from_secs(2 * 60)).await;
    host.close_idle().await;
    assert_eq!(fake.closes("a"), 1);
}

/// A page whose target ended (a crash, or a page that closed itself) while
/// Chrome lives on is closed, and the next step reopens it where it was.
#[tokio::test]
async fn a_dead_page_reopens_on_the_next_step() {
    use bruhs_models::crew::BROWSER_PAGE_STOPPED;
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    host.bruh_command("a", navigate("https://a.test/x"))
        .await
        .unwrap();
    fake.kill_target("a");
    let error = host
        .bruh_command("a", BrowserAction::Snapshot)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), BROWSER_PAGE_STOPPED);
    assert_eq!(fake.closes("a"), 1);
    assert_eq!(host.computer_state("a").await.unwrap().session_id, None);
    host.bruh_command("a", BrowserAction::Snapshot)
        .await
        .unwrap();
    let launches = fake.launches();
    assert_eq!(launches.len(), 2);
    assert_eq!(launches[1].url, "https://a.test/x");
    // The owner's input reopens a page that died between steps too.
    host.computer_request("a", ComputerRequest::TakeControl)
        .await
        .unwrap();
    fake.kill_target("a");
    assert!(host.owner_input("a", ComputerInput::Reload).await.is_err());
    host.owner_input("a", ComputerInput::Reload).await.unwrap();
    assert_eq!(fake.launches().len(), 3);
}

#[tokio::test(start_paused = true)]
async fn the_idle_reaper_ends_with_its_host() {
    let (host, _fake) = LocalToolsHost::with_fake_pages();
    let reaper = host.start_idle_reaper();
    drop(host);
    tokio::time::timeout(Duration::from_secs(40), reaper)
        .await
        .expect("the reaper holds no strong reference")
        .unwrap();
}

#[tokio::test]
async fn stop_interrupts_a_pending_action_and_keeps_the_browser() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    host.open_computer("a").await.unwrap();
    let _held = fake.hold_next_action("a");
    let click = BrowserAction::Click {
        selector: "#go".into(),
    };
    let pending = tokio::spawn({
        let host = Arc::clone(&host);
        let click = click.clone();
        async move { host.bruh_command("a", click).await }
    });
    eventually(|| fake.actions("a").len() == 1).await;
    let mut paused = bruh_in(home.path(), "a");
    paused.paused = true;
    host.sync_bruhs(vec![paused.clone()]).await;
    let error = pending.await.unwrap().unwrap_err();
    assert_eq!(error.to_string(), ACTION_STOPPED);
    assert_eq!(fake.closes("a"), 0);
    assert_eq!(
        fake.releases("a"),
        1,
        "a cut-off click lets go of the button"
    );
    assert!(host.computer_state("a").await.unwrap().session_id.is_some());
    // Paused, the bruh can't act; resumed, it can.
    host.sync_bruhs(vec![paused]).await;
    assert_eq!(
        host.bruh_command("a", click.clone())
            .await
            .unwrap_err()
            .to_string(),
        ACTION_STOPPED
    );
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    host.bruh_command("a", click).await.unwrap();
    assert_eq!(fake.launches().len(), 1);
}

#[tokio::test]
async fn turning_computer_off_ends_a_pending_action_and_closes_the_browser() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    host.open_computer("a").await.unwrap();
    let _held = fake.hold_next_action("a");
    let pending = tokio::spawn({
        let host = Arc::clone(&host);
        async move { host.bruh_command("a", BrowserAction::Snapshot).await }
    });
    eventually(|| fake.actions("a").len() == 1).await;
    host.sync_bruhs(vec![]).await;
    assert_eq!(fake.closes("a"), 1);
    assert_eq!(fake.closed_with_tree("a"), [true]);
    let error = pending.await.unwrap().unwrap_err();
    assert!(
        matches!(&error, ToolError::Refused(message) if message == COMPUTER_OFF_DENIAL),
        "{error:?}"
    );
}

#[tokio::test]
async fn each_bruh_has_its_own_profile_and_bruh_grants_stay_out_of_verification() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    host.sync_bruhs(vec![bruh_in(home.path(), "a"), bruh_in(home.path(), "b")])
        .await;
    host.open_computer("a").await.unwrap();
    host.open_computer("b").await.unwrap();
    let launches = fake.launches();
    assert_eq!(launches.len(), 2);
    assert_ne!(launches[0].profile_dir, launches[1].profile_dir);
    assert!(launches
        .iter()
        .all(|launch| launch.profile_dir.join("Cookies").exists()));
    let snapshot = host.snapshot().await.unwrap();
    assert!(!snapshot
        .grants
        .iter()
        .any(|grant| grant.principal.starts_with("bruh:")));
    assert!(
        snapshot.browsers.is_empty(),
        "bruh pages stay out of Verification"
    );
    let error = host
        .grant(GrantRequest {
            principal: "bruh:a".into(),
            scope: ToolScope::Browser,
        })
        .unwrap_err();
    assert!(
        matches!(&error, ToolError::Invalid(message) if message == TURN_ON_IN_SETTINGS),
        "{error:?}"
    );
    assert!(host.snapshot().await.unwrap().grants.is_empty());
}

#[tokio::test]
async fn a_bruh_session_belongs_to_its_bruh_grant() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    let state = host.open_computer("a").await.unwrap();
    let launch = &fake.launches()[0];
    assert_eq!(launch.agent_id, "a");
    let (principal, grant_id) = host.bruh_grant("a").unwrap();
    assert_eq!(principal, "bruh:a");
    assert_eq!(launch.grant_id, grant_id);
    assert_eq!(fake.session("a").unwrap().grant_id, grant_id);
    assert_eq!(
        state.session_id,
        fake.session("a").map(|session| session.id)
    );
    // A thread route can't reach a bruh grant.
    assert!(host.require(&grant_id, ToolScope::Browser).is_err());
}

#[tokio::test]
async fn the_page_url_comes_from_the_open_page() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    assert_eq!(host.page_url("a").await, None);
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    host.bruh_command("a", navigate("https://a.test/1"))
        .await
        .unwrap();
    fake.set_url("a", "https://a.test/2");
    assert_eq!(
        host.page_url("a").await.as_deref(),
        Some("https://a.test/2")
    );
    host.close_computer("a").await.unwrap();
    assert_eq!(
        host.page_url("a").await.as_deref(),
        Some("https://a.test/2")
    );
}

#[tokio::test]
async fn an_open_page_that_does_not_answer_is_on_an_unknown_page() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    host.bruh_command("a", navigate("https://a.test/1"))
        .await
        .unwrap();
    // The page answers nothing now; where it was last doesn't count.
    fake.close_window("a");
    assert_eq!(host.page_url("a").await, None);
}

#[tokio::test]
async fn bruh_input_runs_only_on_the_page_it_was_allowed_on() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    host.bruh_command("a", navigate("https://shop.test/cart"))
        .await
        .unwrap();
    let click = || BrowserAction::Click {
        selector: "#buy".into(),
    };
    let allowed_on = Some("https://shop.test/");
    host.bruh_command_on("a", click(), allowed_on)
        .await
        .unwrap();
    // The page moved itself to GitHub after the gate looked.
    fake.set_url("a", "https://github.com/o/r/pull/1");
    let refused = |result: Result<_, ToolError>| matches!(result, Err(ToolError::Refused(message)) if message == PAGE_CHANGED);
    assert!(refused(
        host.bruh_command_on("a", click(), allowed_on).await
    ));
    // Reads and navigation don't act on the page, so they still run.
    host.bruh_command_on("a", BrowserAction::Snapshot, allowed_on)
        .await
        .unwrap();
    // Input allowed on GitHub runs there.
    host.bruh_command_on("a", click(), Some("https://github.com/o/r"))
        .await
        .unwrap();
    // Input allowed on a page nobody could name says so.
    let unknown = |result: Result<_, ToolError>| matches!(result, Err(ToolError::Refused(message)) if message == PAGE_UNKNOWN);
    assert!(unknown(host.bruh_command_on("a", click(), None).await));
    // A page that doesn't say where it is takes no input.
    fake.close_window("a");
    assert!(unknown(
        host.bruh_command_on("a", click(), Some("https://github.com/o/r"))
            .await
    ));
    let clicks = fake
        .actions("a")
        .into_iter()
        .filter(|action| *action == click())
        .count();
    assert_eq!(clicks, 2);
}

#[tokio::test]
async fn the_command_route_can_repeat_the_hosts_refusals() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    let evaluate = BrowserAction::Evaluate {
        expression: "1".into(),
    };
    let refusal = |result: Result<(), ToolError>| match result {
        Err(ToolError::Refused(message)) => message,
        other => panic!("expected a refusal, got {other:?}"),
    };
    assert_eq!(
        refusal(host.check_bruh_command("a", &BrowserAction::Snapshot)),
        COMPUTER_OFF_DENIAL
    );
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    assert_eq!(
        refusal(host.check_bruh_command("a", &evaluate)),
        NOT_FOR_BRUHS
    );
    assert_eq!(
        refusal(host.check_bruh_command("a", &navigate("http://127.0.0.1:1"))),
        LOCAL_HOST_DENIAL
    );
    assert!(matches!(
        host.check_bruh_command("a", &BrowserAction::ClickAt { x: 5000.0, y: 1.0 }),
        Err(ToolError::Invalid(_))
    ));
    host.computer_request("a", ComputerRequest::TakeControl)
        .await
        .unwrap();
    assert_eq!(
        refusal(host.check_bruh_command("a", &BrowserAction::Snapshot)),
        OWNER_HAS_COMPUTER
    );
    host.computer_request("a", ComputerRequest::HandBack)
        .await
        .unwrap();
    host.check_bruh_command("a", &BrowserAction::Snapshot)
        .unwrap();
    host.sync_bruhs(vec![BruhComputer {
        paused: true,
        ..bruh_in(home.path(), "a")
    }])
    .await;
    assert_eq!(
        refusal(host.check_bruh_command("a", &BrowserAction::Snapshot)),
        ACTION_STOPPED
    );
    // Checking runs nothing.
    assert!(fake.actions("a").is_empty());
}

#[tokio::test]
async fn a_proxy_the_guard_cannot_use_stops_the_launch() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages_and_network(|| {
        Err(ToolError::Unavailable(
            super::super::network::SOCKS_UNSUPPORTED.into(),
        ))
    });
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    let error = host
        .bruh_command("a", BrowserAction::Snapshot)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), super::super::network::SOCKS_UNSUPPORTED);
    assert!(fake.launches().is_empty());
    let state = host.computer_state("a").await.unwrap();
    assert_eq!(
        state.error.as_deref(),
        Some(super::super::network::SOCKS_UNSUPPORTED)
    );
    assert_eq!(state.session_id, None);
}

#[tokio::test]
async fn close_all_closes_every_bruh_browser() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    host.sync_bruhs(vec![bruh_in(home.path(), "a"), bruh_in(home.path(), "b")])
        .await;
    host.open_computer("a").await.unwrap();
    host.open_computer("b").await.unwrap();
    host.close_all().await;
    assert_eq!((fake.closes("a"), fake.closes("b")), (1, 1));
    // Nothing opens after shutdown, so no profile folder comes back.
    let error = host.open_computer("a").await.unwrap_err();
    assert!(
        matches!(&error, ToolError::Refused(message) if message == COMPUTER_OFF_DENIAL),
        "{error:?}"
    );
    assert!(host
        .bruh_command("b", BrowserAction::Snapshot)
        .await
        .is_err());
    assert_eq!(fake.launches().len(), 2);
}

fn press(x: f64, y: f64) -> ComputerInput {
    ComputerInput::Mouse {
        kind: MouseKind::Down,
        x,
        y,
        button: MouseButton::Left,
        clicks: 1,
        modifiers: 0,
    }
}

#[tokio::test]
async fn taking_control_refuses_the_bruh_until_hand_back() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    host.computer_request("a", ComputerRequest::Open)
        .await
        .unwrap();
    let mut changes = host.control_changes("a").unwrap();
    let held = host
        .computer_request("a", ComputerRequest::TakeControl)
        .await
        .unwrap();
    assert_eq!(held.control, Control::Owner);
    assert_eq!(changes.borrow_and_update().control, Control::Owner);
    let error = host
        .bruh_command("a", BrowserAction::Snapshot)
        .await
        .unwrap_err();
    assert!(
        matches!(&error, ToolError::Refused(message) if message == OWNER_HAS_COMPUTER),
        "{error:?}"
    );
    assert!(fake.actions("a").is_empty());
    host.owner_input("a", press(10.0, 10.0)).await.unwrap();
    assert_eq!(fake.inputs("a"), [press(10.0, 10.0)]);
    let back = host
        .computer_request("a", ComputerRequest::HandBack)
        .await
        .unwrap();
    assert_eq!(back.control, Control::Bruh);
    let error = host.owner_input("a", press(10.0, 10.0)).await.unwrap_err();
    assert_eq!(error.to_string(), TAKE_CONTROL_FIRST);
    assert_eq!(fake.inputs("a").len(), 1, "nothing after hand back");
    assert!(changes.has_changed().unwrap());
    assert_eq!(changes.borrow_and_update().control, Control::Bruh);
    assert_eq!(
        fake.launches().len(),
        1,
        "a headless hand back keeps the page"
    );
    assert!(host.control_changes("b").is_none());
}

#[tokio::test]
async fn taking_control_lets_an_action_in_flight_finish() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    host.open_computer("a").await.unwrap();
    let held = fake.hold_next_action("a");
    let pending = tokio::spawn({
        let host = Arc::clone(&host);
        async move { host.bruh_command("a", BrowserAction::Snapshot).await }
    });
    eventually(|| fake.actions("a").len() == 1).await;
    let state = host
        .computer_request("a", ComputerRequest::TakeControl)
        .await
        .unwrap();
    assert_eq!(state.control, Control::Owner);
    held.notify_one();
    pending.await.unwrap().unwrap();
}

#[tokio::test]
async fn an_input_after_hand_back_is_refused_once_with_the_note() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    host.computer_request("a", ComputerRequest::Open)
        .await
        .unwrap();
    host.computer_request("a", ComputerRequest::TakeControl)
        .await
        .unwrap();
    fake.set_url("a", "https://a.test/after");
    host.computer_request("a", ComputerRequest::HandBack)
        .await
        .unwrap();
    let click = || BrowserAction::Click {
        selector: "#go".into(),
    };
    let error = host.bruh_command("a", click()).await.unwrap_err();
    assert!(matches!(error, ToolError::Invalid(_)), "{error:?}");
    assert_eq!(error.to_string(), handed_back_note("https://a.test/after"));
    assert!(!fake.actions("a").contains(&click()));
    host.bruh_command("a", click()).await.unwrap();
    assert_eq!(fake.actions("a"), [click()]);
    // A read runs and carries the note; the input after it runs.
    host.computer_request("a", ComputerRequest::TakeControl)
        .await
        .unwrap();
    fake.set_url("a", "https://a.test/again");
    host.computer_request("a", ComputerRequest::HandBack)
        .await
        .unwrap();
    let reply = host
        .bruh_command("a", BrowserAction::Snapshot)
        .await
        .unwrap();
    assert_eq!(
        reply.value["note"],
        handed_back_note("https://a.test/again")
    );
    assert_eq!(reply.value["ok"], true);
    host.bruh_command("a", click()).await.unwrap();
    let later = host
        .bruh_command("a", BrowserAction::Snapshot)
        .await
        .unwrap();
    assert!(later.value.get("note").is_none(), "the note is read once");
    assert_eq!(
        fake.actions("a"),
        [
            click(),
            BrowserAction::Snapshot,
            click(),
            BrowserAction::Snapshot
        ]
    );
}

#[tokio::test]
async fn owner_input_needs_control_and_stays_in_bounds() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    assert!(matches!(
        host.owner_input("a", press(1.0, 1.0)).await,
        Err(ToolError::NotFound(message)) if message == TURN_ON_IN_SETTINGS
    ));
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    host.computer_request("a", ComputerRequest::Open)
        .await
        .unwrap();
    let error = host.owner_input("a", press(10.0, 10.0)).await.unwrap_err();
    assert_eq!(error.to_string(), TAKE_CONTROL_FIRST);
    host.computer_request("a", ComputerRequest::TakeControl)
        .await
        .unwrap();
    let error = host
        .owner_input(
            "a",
            ComputerInput::Navigate {
                url: "http://localhost:1".into(),
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&error, ToolError::Refused(message) if message == LOCAL_HOST_DENIAL),
        "{error:?}"
    );
    let error = host
        .owner_input("a", press(1281.0, 10.0))
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::Invalid(_)), "{error:?}");
    assert!(fake.inputs("a").is_empty());
    assert!(fake.actions("a").is_empty());
}

#[tokio::test]
async fn open_in_a_window_relaunches_headed_on_the_same_profile_and_back() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    host.bruh_command("a", navigate("https://a.test"))
        .await
        .unwrap();
    let window = host
        .computer_request("a", ComputerRequest::OpenWindow)
        .await
        .unwrap();
    assert_eq!(fake.closes("a"), 1);
    let launches = fake.launches();
    assert_eq!(launches.len(), 2);
    assert!(!launches[1].headless);
    assert_eq!(launches[1].profile_dir, launches[0].profile_dir);
    assert_eq!(launches[1].url, "https://a.test");
    assert!(
        launches[1].proxy.ip().is_loopback() && launches[1].proxy.port() != 0,
        "the window goes through a network guard too: {}",
        launches[1].proxy
    );
    assert_eq!((window.control, window.headed), (Control::Owner, true));
    assert_eq!(window.session_id, fake.session("a").map(|s| s.id));
    let error = host.owner_input("a", press(10.0, 10.0)).await.unwrap_err();
    assert_eq!(error.to_string(), USE_THE_WINDOW);
    assert!(fake.inputs("a").is_empty());
    let error = host
        .bruh_command("a", BrowserAction::Snapshot)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), OWNER_HAS_COMPUTER);
    fake.set_url("a", "https://a.test/signed-in");
    let back = host
        .computer_request("a", ComputerRequest::HandBack)
        .await
        .unwrap();
    assert_eq!(fake.closes("a"), 2);
    let launches = fake.launches();
    assert_eq!(launches.len(), 3);
    assert!(launches[2].headless);
    assert_eq!(launches[2].profile_dir, launches[0].profile_dir);
    assert_eq!(launches[2].url, "https://a.test/signed-in");
    assert_eq!((back.control, back.headed), (Control::Bruh, false));
    assert!(back.session_id.is_some());
    let reply = host
        .bruh_command("a", BrowserAction::Snapshot)
        .await
        .unwrap();
    assert_eq!(
        reply.value["note"],
        handed_back_note("https://a.test/signed-in")
    );
}

#[tokio::test]
async fn a_failed_read_keeps_the_note_for_the_next_call() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    host.computer_request("a", ComputerRequest::TakeControl)
        .await
        .unwrap();
    fake.set_url("a", "https://a.test/after");
    host.computer_request("a", ComputerRequest::HandBack)
        .await
        .unwrap();
    fake.fail_next_action("a");
    host.bruh_command("a", BrowserAction::Snapshot)
        .await
        .unwrap_err();
    let reply = host
        .bruh_command("a", BrowserAction::Snapshot)
        .await
        .unwrap();
    assert_eq!(
        reply.value["note"],
        handed_back_note("https://a.test/after")
    );
}

#[tokio::test]
async fn a_window_closed_by_hand_hands_back_where_the_owner_ended_up() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    host.bruh_command("a", navigate("https://a.test"))
        .await
        .unwrap();
    host.computer_request("a", ComputerRequest::OpenWindow)
        .await
        .unwrap();
    // The owner signs in; the window's page is never asked where it is.
    fake.browse_window("a", "https://a.test/signed-in");
    let mut changes = host.control_changes("a").unwrap();
    tokio::time::timeout(
        Duration::from_secs(5),
        changes.wait_for(|state| state.url.as_deref() == Some("https://a.test/signed-in")),
    )
    .await
    .expect("the address follows the window")
    .unwrap();
    fake.close_window("a");
    let state = tokio::time::timeout(
        Duration::from_secs(5),
        changes.wait_for(|state| state.control == Control::Bruh),
    )
    .await
    .expect("a closed window is handed back")
    .unwrap()
    .clone();
    assert!(!state.headed);
    assert!(state.session_id.is_some());
    let launches = fake.launches();
    assert_eq!(launches.len(), 3);
    assert!(launches[2].headless);
    assert_eq!(launches[2].profile_dir, launches[0].profile_dir);
    assert_eq!(launches[2].url, "https://a.test/signed-in");
    assert_eq!(fake.closes("a"), 2, "the dead window is closed too");
    let reply = host
        .bruh_command("a", BrowserAction::Snapshot)
        .await
        .unwrap();
    assert_eq!(
        reply.value["note"],
        handed_back_note("https://a.test/signed-in")
    );
    // An older window's feed ending changes nothing.
    host.computer_request("a", ComputerRequest::OpenWindow)
        .await
        .unwrap();
    host.computer_request("a", ComputerRequest::HandBack)
        .await
        .unwrap();
    settle().await;
    assert_eq!(fake.launches().len(), 5);
    assert_eq!(
        host.computer_state("a").await.unwrap().control,
        Control::Bruh
    );
}

#[tokio::test]
async fn a_failed_window_leaves_control_where_it_was() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    host.computer_request("a", ComputerRequest::TakeControl)
        .await
        .unwrap();
    fake.fail_next_launch("a");
    let error = host
        .computer_request("a", ComputerRequest::OpenWindow)
        .await
        .unwrap_err();
    let state = host.computer_state("a").await.unwrap();
    assert_eq!(state.control, Control::Owner, "the owner still holds it");
    assert!(!state.headed);
    assert!(state.session_id.is_some(), "the live view has a page again");
    assert_eq!(state.error, Some(error.to_string()));
    assert!(fake.launches().last().unwrap().headless);
    assert_eq!(
        host.bruh_command("a", BrowserAction::Snapshot)
            .await
            .unwrap_err()
            .to_string(),
        OWNER_HAS_COMPUTER
    );
    // From the bruh, a failed window gives the browser back to it.
    host.computer_request("a", ComputerRequest::HandBack)
        .await
        .unwrap();
    fake.fail_next_launch("a");
    host.computer_request("a", ComputerRequest::OpenWindow)
        .await
        .unwrap_err();
    assert_eq!(
        host.computer_state("a").await.unwrap().control,
        Control::Bruh
    );
}

#[tokio::test]
async fn closing_while_held_hands_back_with_a_note() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    host.computer_request("a", ComputerRequest::TakeControl)
        .await
        .unwrap();
    assert_eq!(fake.launches().len(), 1, "taking control opens the browser");
    let closed = host
        .computer_request("a", ComputerRequest::Close)
        .await
        .unwrap();
    assert_eq!(closed.control, Control::Bruh);
    assert_eq!(closed.session_id, None);
    assert_eq!(fake.closes("a"), 1);
    let reply = host
        .bruh_command("a", BrowserAction::Snapshot)
        .await
        .unwrap();
    assert_eq!(fake.launches().len(), 2);
    assert_eq!(reply.value["note"], CLOSED_BY_OWNER);
}

#[tokio::test(start_paused = true)]
async fn a_held_computer_never_closes_for_idleness() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    let reaper = host.start_idle_reaper();
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    host.computer_request("a", ComputerRequest::Open)
        .await
        .unwrap();
    host.computer_request("a", ComputerRequest::TakeControl)
        .await
        .unwrap();
    for _ in 0..3 {
        tokio::time::advance(Duration::from_secs(10 * 60)).await;
        settle().await;
    }
    assert!(host.computer_state("a").await.unwrap().session_id.is_some());
    assert_eq!(fake.closes("a"), 0);
    // Handed back, it idles out like any other.
    host.computer_request("a", ComputerRequest::HandBack)
        .await
        .unwrap();
    tokio::time::advance(Duration::from_secs(11 * 60)).await;
    settle().await;
    assert_eq!(fake.closes("a"), 1);
    reaper.abort();
}

/// Whether watching failed because there is no such bruh live view.
fn ended(watch: Result<ComputerWatch, ToolError>) -> bool {
    matches!(watch, Err(ToolError::NotFound(message)) if message == LIVE_VIEW_ENDED)
}

#[tokio::test(start_paused = true)]
async fn a_watched_browser_never_closes_for_idleness() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    let reaper = host.start_idle_reaper();
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    let state = host.open_computer("a").await.unwrap();
    let watch = host
        .watch_computer(state.session_id.as_deref().unwrap())
        .unwrap_or_else(|_| panic!("the open browser can be watched"));
    for _ in 0..3 {
        tokio::time::advance(Duration::from_secs(10 * 60)).await;
        settle().await;
    }
    assert!(host.computer_state("a").await.unwrap().session_id.is_some());
    assert_eq!(fake.closes("a"), 0);
    // Unwatched, it idles out like any other.
    drop(watch);
    tokio::time::advance(Duration::from_secs(11 * 60)).await;
    settle().await;
    assert_eq!(fake.closes("a"), 1);
    reaper.abort();
}

#[tokio::test]
async fn watching_needs_a_bruh_session() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    assert!(ended(host.watch_computer("nope")));
    assert!(ended(host.watch_computer("")));
    // A thread's verification grant and the bruh's own grant are no live
    // views; thread sessions live outside the bruh slots altogether.
    let thread = host
        .grant(GrantRequest {
            principal: "thread-1".into(),
            scope: ToolScope::Browser,
        })
        .unwrap();
    assert!(ended(host.watch_computer(&thread.id)));
    let (_, bruh_grant) = host.bruh_grant("a").unwrap();
    assert!(ended(host.watch_computer(&bruh_grant)));
    let session = host.open_computer("a").await.unwrap().session_id.unwrap();
    let Ok(mut watch) = host.watch_computer(&session) else {
        panic!("the open browser can be watched");
    };
    assert_eq!(watch.agent_id, "a");
    assert_eq!(
        watch.state.borrow().session_id.as_deref(),
        Some(session.as_str())
    );
    fake.publish_frame("a", vec![1, 2]);
    assert_eq!(&*watch.viewer.next_frame().await.unwrap(), &[1, 2]);
    host.close_computer("a").await.unwrap();
    assert!(ended(host.watch_computer(&session)));
    // Turning Computer off ends the live view.
    host.sync_bruhs(vec![]).await;
    watch.viewer.ack();
    assert!(watch.viewer.next_frame().await.is_none());
}

#[tokio::test]
async fn the_live_view_restarts_after_every_relaunch() {
    let home = tempfile::tempdir().unwrap();
    let (host, fake) = LocalToolsHost::with_fake_pages();
    let mut chrome = FakeChrome::start().await;
    fake.set_page_socket("a", &chrome.url);
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    let session = host.open_computer("a").await.unwrap().session_id.unwrap();
    // Nobody watches yet: no connection.
    chrome.settle(Duration::from_millis(50)).await;
    assert_eq!(chrome.count("Page.enable"), 0);
    let Ok(watch) = host.watch_computer(&session) else {
        panic!("the open browser can be watched");
    };
    chrome
        .wait_for("the first page's screencast", |chrome| {
            chrome.count_on(0, "Page.startScreencast") == 1
        })
        .await;
    // Each relaunch has a new page socket; the old connection goes with
    // its page.
    let relaunched = |n: usize| {
        move |chrome: &FakeChrome| {
            chrome.count_on(n, "Page.startScreencast") == 1 && chrome.closed(n - 1)
        }
    };
    host.computer_request("a", ComputerRequest::OpenWindow)
        .await
        .unwrap();
    chrome
        .wait_for("the window's screencast", relaunched(1))
        .await;
    host.computer_request("a", ComputerRequest::HandBack)
        .await
        .unwrap();
    chrome
        .wait_for("the handed back page's screencast", relaunched(2))
        .await;
    host.computer_request("a", ComputerRequest::OpenWindow)
        .await
        .unwrap();
    chrome
        .wait_for("the second window's screencast", relaunched(3))
        .await;
    fake.close_window("a");
    chrome
        .wait_for(
            "the screencast after a window closed by hand",
            relaunched(4),
        )
        .await;
    host.computer_request("a", ComputerRequest::Close)
        .await
        .unwrap();
    host.computer_request("a", ComputerRequest::Open)
        .await
        .unwrap();
    chrome
        .wait_for("the reopened page's screencast", relaunched(5))
        .await;
    drop(watch);
    chrome
        .wait_for("the screencast to stop", |chrome| {
            chrome.count_on(5, "Page.stopScreencast") == 1
        })
        .await;
    host.close_all().await;
}

/// A stand-in for a Chrome a crash left behind: a shell whose command line
/// carries `args`, which exits on SIGTERM and takes its `sleep` with it.
/// Returns once the shell runs with its trap set, so `ps` shows `args`.
#[cfg(unix)]
async fn orphan(home: &Path, args: &[String]) -> tokio::process::Child {
    shell(
        home,
        "trap 'kill $! 2>/dev/null; exit 0' TERM; : > \"$READY\"; sleep 30 & wait",
        args,
    )
    .await
}

/// Runs `script` with `args` on its command line, once it has touched
/// `$READY`.
#[cfg(unix)]
async fn shell(home: &Path, script: &str, args: &[String]) -> tokio::process::Child {
    let ready = home.join(format!("ready-{}", uuid::Uuid::new_v4()));
    let child = tokio::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(script)
        .arg("sh")
        .args(args)
        .env("READY", &ready)
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    eventually(|| ready.exists()).await;
    child
}

#[cfg(unix)]
async fn stop_orphan(mut child: tokio::process::Child) {
    if let Some(pid) = child.id().and_then(|pid| libc::pid_t::try_from(pid).ok()) {
        // SAFETY: `pid` is this test's own unreaped child.
        unsafe { libc::kill(pid, libc::SIGTERM) };
    }
    let _ = child.wait().await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_chrome_left_by_a_crash_is_stopped_before_its_profile_reopens() {
    use super::super::browser::{release_profile, PROFILE_IN_USE};
    let home = tempfile::tempdir().unwrap();
    let policy = crate::local_tools::ProcessPolicy::default();
    let profile = |id: &str| {
        let dir = home.path().join("bruhs").join(id).join("browser");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    };
    let pid_file = |dir: &Path| dir.with_file_name("browser.pid");
    // A pid file naming a live process on this profile: it is stopped.
    let a = profile("a");
    let mut stale = orphan(home.path(), &[format!("--user-data-dir={}", a.display())]).await;
    std::fs::write(pid_file(&a), stale.id().unwrap().to_string()).unwrap();
    let (released, exited) = tokio::join!(release_profile(&a, policy), stale.wait());
    released.unwrap();
    assert!(exited.unwrap().success(), "stopped with SIGTERM");
    assert!(!pid_file(&a).exists());
    // A pid file naming some other process (a reused pid): left alone.
    let b = profile("b");
    let other = orphan(home.path(), &[format!("--user-data-dir={}2", b.display())]).await;
    std::fs::write(pid_file(&b), other.id().unwrap().to_string()).unwrap();
    release_profile(&b, policy).await.unwrap();
    assert!(!pid_file(&b).exists());
    // The lock held by a live Chrome Bruhs didn't start: an error, no kill.
    let c = profile("c");
    let holder = orphan(home.path(), &[format!("--user-data-dir={}", c.display())]).await;
    std::os::unix::fs::symlink(
        format!("host-{}", holder.id().unwrap()),
        c.join("SingletonLock"),
    )
    .unwrap();
    let error = release_profile(&c, policy).await.unwrap_err();
    assert_eq!(error.to_string(), PROFILE_IN_USE);
    // A lock whose process isn't on this profile is stale.
    std::os::unix::fs::symlink(
        format!("host-{}", other.id().unwrap()),
        b.join("SingletonLock"),
    )
    .unwrap();
    release_profile(&b, policy).await.unwrap();
    let mut other = other;
    let mut holder = holder;
    assert!(other.try_wait().unwrap().is_none(), "never killed");
    assert!(holder.try_wait().unwrap().is_none(), "never killed");
    stop_orphan(other).await;
    stop_orphan(holder).await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_stale_chrome_that_ignores_sigterm_is_killed_after_the_grace() {
    use super::super::browser::release_profile_within;
    use std::os::unix::process::ExitStatusExt;
    let home = tempfile::tempdir().unwrap();
    let profile = home.path().join("bruhs").join("a").join("browser");
    std::fs::create_dir_all(&profile).unwrap();
    let pid_file = profile.with_file_name("browser.pid");
    // Short sleeps, so nothing outlives the shell for long.
    let mut stubborn = shell(
        home.path(),
        "trap '' TERM; : > \"$READY\"; while :; do sleep 0.05; done",
        &[format!("--user-data-dir={}", profile.display())],
    )
    .await;
    std::fs::write(&pid_file, stubborn.id().unwrap().to_string()).unwrap();
    let grace = Duration::from_millis(300);
    let policy = crate::local_tools::ProcessPolicy::default();
    let (released, exited) = tokio::join!(
        release_profile_within(&profile, policy, grace),
        stubborn.wait()
    );
    released.unwrap();
    assert_eq!(exited.unwrap().signal(), Some(libc::SIGKILL));
    assert!(!pid_file.exists());
}

#[tokio::test]
#[ignore = "launches headless Chrome on a temp profile; run explicitly for local acceptance"]
async fn live_bruh_computer_runs_behind_the_guard_and_closes_cleanly() {
    let home = tempfile::tempdir().unwrap();
    let host = LocalToolsHost::new().unwrap();
    let bruh = bruh_in(home.path(), "a");
    let pid_file = bruh.profile_dir.with_file_name("browser.pid");
    host.sync_bruhs(vec![bruh.clone()]).await;
    let reply = host
        .bruh_command("a", BrowserAction::Snapshot)
        .await
        .unwrap();
    assert_eq!(reply.value["url"], "about:blank");
    let pid: u32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    // Chrome runs headless behind the bruh's network guard.
    let command = std::process::Command::new("ps")
        .args(["-ww", "-o", "command=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let command = String::from_utf8_lossy(&command.stdout);
    assert!(
        command.contains("--proxy-server=http://127.0.0.1:"),
        "{command}"
    );
    assert!(
        command.contains("--proxy-bypass-list=<-loopback>"),
        "{command}"
    );
    assert!(command.contains("--headless=new"), "{command}");
    host.sync_bruhs(vec![]).await;
    assert!(!pid_file.exists(), "the pid file goes with the browser");
    #[cfg(unix)]
    {
        let pid = libc::pid_t::try_from(pid).unwrap();
        // SAFETY: signal 0 only checks whether `pid` exists.
        let alive = unsafe { libc::kill(pid, 0) } == 0;
        assert!(!alive, "Chrome exited");
    }
    assert!(bruh.profile_dir.join("Default").exists());
}

#[tokio::test]
#[ignore = "launches headless Chrome on a temp profile; run explicitly for local acceptance"]
async fn live_bruh_computer_streams_jpeg_frames_to_its_viewers() {
    let home = tempfile::tempdir().unwrap();
    let host = LocalToolsHost::new().unwrap();
    host.sync_bruhs(vec![bruh_in(home.path(), "a")]).await;
    let session = host.open_computer("a").await.unwrap().session_id.unwrap();
    let Ok(mut watch) = host.watch_computer(&session) else {
        panic!("the open browser can be watched");
    };
    let Ok(mut second) = host.watch_computer(&session) else {
        panic!("a second viewer can join");
    };
    for viewer in [&mut watch.viewer, &mut second.viewer] {
        let frame = tokio::time::timeout(Duration::from_secs(10), viewer.next_frame())
            .await
            .expect("Chrome sends a frame")
            .unwrap();
        assert!(frame.starts_with(&[0xff, 0xd8]), "a JPEG");
        viewer.ack();
    }
    // A relaunch while watched streams the new page.
    host.close_computer("a").await.unwrap();
    host.open_computer("a").await.unwrap();
    host.bruh_command("a", BrowserAction::Scroll { dx: 0.0, dy: 10.0 })
        .await
        .unwrap();
    let frame = tokio::time::timeout(Duration::from_secs(10), watch.viewer.next_frame())
        .await
        .expect("the reopened page sends a frame")
        .unwrap();
    assert!(frame.starts_with(&[0xff, 0xd8]), "a JPEG");
    drop((watch, second));
    host.close_all().await;
}
