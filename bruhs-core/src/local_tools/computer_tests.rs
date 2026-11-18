use std::{path::Path, sync::Arc, time::Duration};

use bruhs_models::crew::{
    ACTION_STOPPED, COMPUTERS_FULL, COMPUTER_OFF_DENIAL, LOCAL_HOST_DENIAL, NOT_FOR_BRUHS,
    TURN_ON_IN_SETTINGS,
};
use bruhs_models::local_tools::{BrowserAction, GrantRequest, ToolScope};

use super::{BruhComputer, LocalToolsHost};
use crate::local_tools::ToolError;

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
}

/// A stand-in for a Chrome a crash left behind: a shell whose command line
/// carries `args`, which exits on SIGTERM and takes its `sleep` with it.
/// Returns once the shell runs with its trap set, so `ps` shows `args`.
#[cfg(unix)]
async fn orphan(home: &Path, args: &[String]) -> tokio::process::Child {
    let ready = home.join(format!("ready-{}", uuid::Uuid::new_v4()));
    let child = tokio::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("trap 'kill $! 2>/dev/null; exit 0' TERM; : > \"$READY\"; sleep 30 & wait")
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
