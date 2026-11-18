//! Computers: the switch, keeping the host in step with the store,
//! removing a deleted bruh's whole `bruhs/<id>` tree, the wait while the
//! owner has the computer, and the gate on each browser call.

use super::approval::decide;
use super::computers::{mark_tree, sweep_trees, OWNED_MARKER};
use super::crew_tests::{
    agent, approvals, claim_one, decide_approval, decided, deny, finish, runtime_with,
    waiting_approval, Decision, LONG_WAIT, NOT_ANSWERED, SHORT_WAIT,
};
use super::{ReactiveError, ReactiveRuntime};
use crate::local_tools::{FakePages, LocalToolsHost};
use crate::PermissionDecision;
use bruhs_models::agent::AgentProvider;
use bruhs_models::crew::*;
use bruhs_models::local_tools::{BrowserAction, ComputerRequest};
use bruhs_models::reactive::*;
use bruhs_models::weekly_reports::{WeeklyReportCommand, WeeklyReportPreset};
use serde_json::{json, Value};
use std::{collections::HashSet, future::Future, path::Path, sync::Arc, time::Duration};

/// A Claude agent; [`full`] gives it Full access.
fn full_claude(id: &str) -> ReactiveAgent {
    agent(id)
}

fn access(level: AccessLevel) -> BruhAccess {
    BruhAccess {
        level,
        ..BruhAccess::default()
    }
}

fn set_level(id: &str, level: AccessLevel, look: BruhLook) -> ReactiveCommand {
    ReactiveCommand::SetBruh {
        agent_id: id.into(),
        look,
        access: access(level),
        effort: None,
    }
}

async fn full(runtime: &ReactiveRuntime, id: &str) {
    runtime
        .command(set_level(id, AccessLevel::Full, BruhLook::default()))
        .await
        .unwrap();
}

fn set_computer(id: &str, on: bool) -> ReactiveCommand {
    ReactiveCommand::SetComputer {
        agent_id: id.into(),
        on,
    }
}

fn delete(id: &str) -> ReactiveCommand {
    ReactiveCommand::Apply {
        change: ReactiveChange::DeleteAgent { id: id.into() },
    }
}

/// A bruh made with its own work folder under the runtime's home.
fn create_bruh(id: &str) -> ReactiveCommand {
    ReactiveCommand::CreateBruh {
        agent: ReactiveAgent {
            project_path: String::new(),
            ..agent(id)
        },
        look: BruhLook::default(),
        access: access(AccessLevel::Full),
        effort: None,
    }
}

/// Waits for `check`, failing after a few seconds of real time.
async fn eventually<F: Future<Output = bool>>(mut check: impl FnMut() -> F) {
    for _ in 0..500 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the condition never held");
}

fn home(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().canonicalize().unwrap()
}

/// `<home>/bruhs/<name>`, marked as `marker` when given.
fn tree(home: &Path, name: &str, marker: Option<&str>) -> std::path::PathBuf {
    let tree = home.join("bruhs").join(name);
    std::fs::create_dir_all(tree.join("browser")).unwrap();
    std::fs::write(tree.join("browser/Cookies"), b"kept").unwrap();
    if let Some(marker) = marker {
        std::fs::write(tree.join(OWNED_MARKER), marker).unwrap();
    }
    tree
}

fn start(runtime: &ReactiveRuntime) -> tokio::task::JoinHandle<()> {
    runtime.start(Arc::new(crate::AppState::new())).unwrap()
}

#[tokio::test]
async fn set_computer_saves_without_cancelling_runs() {
    let (_dir, runtime) = runtime_with(&[full_claude("a")]).await;
    full(&runtime, "a").await;
    runtime
        .command(ReactiveCommand::Chat {
            agent_id: "a".into(),
            message: "Book the flight".into(),
            run_id: None,
        })
        .await
        .unwrap();
    let snapshot = runtime.command(set_computer("a", true)).await.unwrap();
    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(snapshot.runs[0].status, RunStatus::Queued);
    assert!(snapshot.bruhs[0].computer);
    // A look edit keeps the switch.
    let look = BruhLook {
        face: BruhColor::Pandan,
        ..BruhLook::default()
    };
    let snapshot = runtime
        .command(set_level("a", AccessLevel::Full, look))
        .await
        .unwrap();
    assert!(snapshot.bruhs[0].computer);
    assert_eq!(snapshot.bruhs[0].look.face, BruhColor::Pandan);
    // Off cancels nothing either.
    let snapshot = runtime.command(set_computer("a", false)).await.unwrap();
    assert!(!snapshot.bruhs[0].computer);
    assert_eq!(snapshot.runs[0].status, RunStatus::Queued);
    assert!(claim_one(&runtime).await.is_some());
}

#[tokio::test]
async fn set_computer_is_refused_for_report_agents_and_unknown_bruhs() {
    let (dir, runtime) = runtime_with(&[]).await;
    let missing = runtime.command(set_computer("nobody", true)).await;
    assert!(
        matches!(missing, Err(ReactiveError::NotFound(_))),
        "{missing:?}"
    );
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
    let managed = format!("weekly-report-{}", preset.id);
    let refused = runtime.command(set_computer(&managed, true)).await;
    assert!(
        matches!(&refused, Err(ReactiveError::Conflict(message)) if message.contains("weekly report preset")),
        "{refused:?}"
    );
}

#[tokio::test]
async fn the_runtime_syncs_its_computers_to_the_host() {
    let (_dir, runtime) = runtime_with(&[full_claude("a")]).await;
    full(&runtime, "a").await;
    let (host, fake) = LocalToolsHost::with_fake_pages();
    runtime.attach_computers(host.clone()).unwrap();
    assert!(matches!(
        runtime.attach_computers(host.clone()),
        Err(ReactiveError::Conflict(_))
    ));
    assert!(runtime.computers().is_some());
    assert!(host.computer_state("a").await.is_err());
    runtime.command(set_computer("a", true)).await.unwrap();
    eventually(|| async { host.computer_state("a").await.is_ok() }).await;
    // Read only takes the Computer away; the switch stays on.
    let snapshot = runtime
        .command(set_level("a", AccessLevel::ReadOnly, BruhLook::default()))
        .await
        .unwrap();
    assert!(snapshot.bruhs[0].computer);
    eventually(|| async { host.computer_state("a").await.is_err() }).await;
    full(&runtime, "a").await;
    eventually(|| async { host.computer_state("a").await.is_ok() }).await;
    host.open_computer("a").await.unwrap();
    let _held = fake.hold_next_action("a");
    let pending = tokio::spawn({
        let host = Arc::clone(&host);
        async move { host.bruh_command("a", BrowserAction::Snapshot).await }
    });
    eventually(|| async { fake.actions("a").len() == 1 }).await;
    runtime
        .command(ReactiveCommand::StopBruh {
            agent_id: "a".into(),
        })
        .await
        .unwrap();
    let error = tokio::time::timeout(Duration::from_secs(5), pending)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.to_string(), ACTION_STOPPED);
    assert_eq!(fake.closes("a"), 0, "stop keeps the browser open");
}

#[tokio::test]
async fn deleting_a_bruh_removes_its_whole_tree_after_its_browser_closes() {
    let (dir, runtime) = runtime_with(&[]).await;
    let home = home(&dir);
    let (host, fake) = LocalToolsHost::with_fake_pages();
    runtime.attach_computers(host.clone()).unwrap();
    runtime.command(create_bruh("a")).await.unwrap();
    let tree = home.join("bruhs/a");
    assert!(tree.join("work").is_dir());
    assert_eq!(
        std::fs::read_to_string(tree.join(OWNED_MARKER)).unwrap(),
        "a"
    );
    runtime.command(set_computer("a", true)).await.unwrap();
    eventually(|| async { host.computer_state("a").await.is_ok() }).await;
    host.open_computer("a").await.unwrap();
    assert_eq!(fake.launches()[0].profile_dir, tree.join("browser"));
    assert!(tree.join("browser/Cookies").is_file());
    runtime.command(delete("a")).await.unwrap();
    eventually(|| async { fake.closes("a") == 1 }).await;
    eventually(|| async { !tree.exists() }).await;
    assert_eq!(
        fake.closed_with_tree("a"),
        vec![true],
        "the browser closed before its tree went"
    );
    // The deleted bruh's slot is gone for good: nothing relaunches it.
    assert!(host.open_computer("a").await.is_err());
    assert!(!tree.exists());
    assert_eq!(fake.launches().len(), 1);
}

#[tokio::test]
async fn a_tree_that_could_not_be_removed_is_swept_at_start() {
    let (dir, runtime) = runtime_with(&[]).await;
    let home = home(&dir);
    let ghost = tree(&home, "ghost", Some("ghost"));
    let stranger = tree(&home, "stranger", None);
    let other = tree(&home, "x", Some("y"));
    let worker = start(&runtime);
    eventually(|| async { !ghost.exists() }).await;
    assert!(stranger.join("browser/Cookies").is_file());
    assert!(other.join("browser/Cookies").is_file());
    worker.abort();
}

#[tokio::test]
async fn a_live_bruhs_tree_stays() {
    let (dir, runtime) = runtime_with(&[agent("a")]).await;
    let home = home(&dir);
    let live = tree(&home, "a", Some("a"));
    let ghost = tree(&home, "ghost", Some("ghost"));
    let worker = start(&runtime);
    eventually(|| async { !ghost.exists() }).await;
    assert!(live.join("browser/Cookies").is_file());
    worker.abort();
    // A tree whose name only differs in case from a live id may be that
    // bruh's own tree on a case-insensitive disk.
    let cased = tree(&home, "A", Some("A"));
    let ids: HashSet<String> = ["a".to_owned()].into();
    assert_eq!(sweep_trees(&home, &ids), 0);
    assert!(cased.exists() && live.exists());
}

#[tokio::test]
async fn the_sweep_never_follows_a_symlink_or_touches_a_custom_folder() {
    let outside = tempfile::tempdir().unwrap();
    let outside = home(&outside);
    // A link inside `bruhs` to a marked folder elsewhere.
    let (dir, runtime) = runtime_with(&[]).await;
    let home_dir = home(&dir);
    let target = outside.join("target");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join(OWNED_MARKER), "link").unwrap();
    std::fs::write(target.join("keep"), b"keep").unwrap();
    std::fs::create_dir_all(home_dir.join("bruhs")).unwrap();
    std::os::unix::fs::symlink(&target, home_dir.join("bruhs/link")).unwrap();
    // A marked tree whose marker is a link to a file naming it.
    let named = outside.join("named");
    std::fs::write(&named, "trick").unwrap();
    let trick = tree(&home_dir, "trick", None);
    std::os::unix::fs::symlink(&named, trick.join(OWNED_MARKER)).unwrap();
    assert_eq!(sweep_trees(&home_dir, &HashSet::new()), 0);
    assert!(home_dir.join("bruhs/link").symlink_metadata().is_ok());
    assert!(target.join("keep").is_file());
    assert!(trick.exists());
    // A bruh with its own folder: deleting it leaves that folder alone.
    let custom = outside.join("custom");
    std::fs::create_dir_all(&custom).unwrap();
    std::fs::write(custom.join("notes"), b"mine").unwrap();
    runtime
        .command(ReactiveCommand::CreateBruh {
            agent: ReactiveAgent {
                project_path: custom.to_string_lossy().into_owned(),
                ..agent("c")
            },
            look: BruhLook::default(),
            access: access(AccessLevel::Full),
            effort: None,
        })
        .await
        .unwrap();
    runtime.command(create_bruh("d")).await.unwrap();
    let (host, _fake) = LocalToolsHost::with_fake_pages();
    runtime.attach_computers(host).unwrap();
    runtime.command(delete("c")).await.unwrap();
    runtime.command(delete("d")).await.unwrap();
    eventually(|| async { !home_dir.join("bruhs/d").exists() }).await;
    assert!(custom.join("notes").is_file());
    assert!(target.join("keep").is_file());
    // `<home>/bruhs` itself a link: the pass removes nothing.
    let linked = tempfile::tempdir().unwrap();
    let linked = home(&linked);
    let elsewhere = outside.join("elsewhere");
    tree(&elsewhere, "ghost", Some("ghost"));
    std::os::unix::fs::symlink(elsewhere.join("bruhs"), linked.join("bruhs")).unwrap();
    assert_eq!(sweep_trees(&linked, &HashSet::new()), 0);
    assert!(elsewhere.join("bruhs/ghost/browser/Cookies").is_file());
}

#[tokio::test]
async fn existing_work_folders_get_their_marker() {
    let dir = tempfile::tempdir().unwrap();
    let home = home(&dir);
    let work = home.join("bruhs/a/work");
    std::fs::create_dir_all(&work).unwrap();
    let elsewhere = home.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let (_kept, runtime) = {
        let runtime = ReactiveRuntime::open(home.join("reactive.sqlite3"))
            .await
            .unwrap();
        for agent in [
            ReactiveAgent {
                project_path: work.to_string_lossy().into_owned(),
                ..agent("a")
            },
            ReactiveAgent {
                project_path: elsewhere.to_string_lossy().into_owned(),
                ..agent("b")
            },
        ] {
            runtime
                .command(ReactiveCommand::Apply {
                    change: ReactiveChange::SaveAgent { agent },
                })
                .await
                .unwrap();
        }
        (dir, runtime)
    };
    let worker = start(&runtime);
    let marker = home.join("bruhs/a").join(OWNED_MARKER);
    eventually(|| async { marker.is_file() }).await;
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "a");
    assert!(
        !home.join("bruhs/b").exists(),
        "a custom folder gets no tree"
    );
    worker.abort();
}

#[tokio::test]
async fn a_failed_create_leaves_no_tree_behind() {
    let (dir, runtime) = runtime_with(&[agent("taken")]).await;
    let refused = runtime
        .command(ReactiveCommand::CreateBruh {
            agent: ReactiveAgent {
                project_path: String::new(),
                ..agent("fresh")
            },
            look: BruhLook::default(),
            access: access(AccessLevel::Full),
            effort: Some("not-an-effort".into()),
        })
        .await;
    assert!(refused.is_err());
    assert!(!home(&dir).join("bruhs/fresh").exists());
}

#[tokio::test]
async fn turning_computer_off_closes_the_browser_and_keeps_its_profile() {
    let (dir, runtime) = runtime_with(&[]).await;
    let tree = home(&dir).join("bruhs/a");
    let (host, fake) = LocalToolsHost::with_fake_pages();
    runtime.attach_computers(host.clone()).unwrap();
    runtime.command(create_bruh("a")).await.unwrap();
    runtime.command(set_computer("a", true)).await.unwrap();
    eventually(|| async { host.computer_state("a").await.is_ok() }).await;
    host.open_computer("a").await.unwrap();
    runtime.command(set_computer("a", false)).await.unwrap();
    eventually(|| async { fake.closes("a") == 1 }).await;
    assert!(host.computer_state("a").await.is_err());
    assert!(tree.join("browser/Cookies").is_file(), "the profile stays");
    assert_eq!(
        std::fs::read_to_string(tree.join(OWNED_MARKER)).unwrap(),
        "a"
    );
}

#[tokio::test]
async fn the_sync_loop_ends_with_its_runtime() {
    let (dir, runtime) = runtime_with(&[full_claude("a")]).await;
    let (host, _fake) = LocalToolsHost::with_fake_pages();
    runtime.attach_computers(host).unwrap();
    runtime.command(set_computer("a", true)).await.unwrap();
    let path = dir.path().join("reactive.sqlite3");
    drop(runtime);
    // The loop holds the runtime only during a pass, so its database lock
    // comes free and the same file opens again.
    for _ in 0..500 {
        if ReactiveRuntime::open(&path).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the sync loop kept the runtime alive");
}

#[test]
fn a_hard_linked_marker_is_replaced_not_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    let home = home(&dir);
    let outside = home.join("outside");
    std::fs::write(&outside, "someone else's").unwrap();
    let tree = home.join("bruhs/a");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::hard_link(&outside, tree.join(OWNED_MARKER)).unwrap();
    mark_tree(&home, "a").unwrap();
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "someone else's");
    assert_eq!(
        std::fs::read_to_string(tree.join(OWNED_MARKER)).unwrap(),
        "a"
    );
}

// ── Take control and the browser gate ──────────────────────────────────────

/// A Full Claude bruh `a` with Computer on, on an attached host, its
/// browser open.
async fn open_computer() -> (
    tempfile::TempDir,
    ReactiveRuntime,
    Arc<LocalToolsHost>,
    FakePages,
) {
    let (dir, runtime) = runtime_with(&[full_claude("a")]).await;
    full(&runtime, "a").await;
    let (host, fake) = LocalToolsHost::with_fake_pages();
    runtime.attach_computers(host.clone()).unwrap();
    runtime.command(set_computer("a", true)).await.unwrap();
    eventually(|| async { host.computer_state("a").await.is_ok() }).await;
    host.open_computer("a").await.unwrap();
    (dir, runtime, host, fake)
}

/// A chat claimed for `agent_id`, now Running.
async fn running(runtime: &ReactiveRuntime, agent_id: &str) -> ReactiveRun {
    runtime
        .command(ReactiveCommand::Chat {
            agent_id: agent_id.into(),
            message: "Do the work.".into(),
            run_id: None,
        })
        .await
        .unwrap();
    claim_one(runtime).await.unwrap().run
}

async fn request(host: &LocalToolsHost, request: ComputerRequest) {
    host.computer_request("a", request).await.unwrap();
}

/// `approval::decide` on a Claude bruh run whose Computer is `computer`, in
/// the background.
fn spawn_decide(
    runtime: &ReactiveRuntime,
    level: AccessLevel,
    computer: bool,
    run: &ReactiveRun,
    tool: &str,
    input: Value,
    timeout: Duration,
) -> tokio::task::JoinHandle<Decision> {
    let (runtime, run, tool) = (runtime.clone(), run.clone(), tool.to_owned());
    tokio::spawn(async move {
        let project = std::env::temp_dir().to_string_lossy().into_owned();
        decide(
            &runtime,
            &access(level),
            AgentProvider::Claude,
            computer,
            &run,
            &project,
            &tool,
            &input,
            timeout,
        )
        .await
    })
}

/// A browser call of the bruh's own tool.
fn browse(
    runtime: &ReactiveRuntime,
    run: &ReactiveRun,
    input: Value,
    timeout: Duration,
) -> tokio::task::JoinHandle<Decision> {
    spawn_decide(
        runtime,
        AccessLevel::Full,
        true,
        run,
        BRUH_BROWSER_TOOL,
        input,
        timeout,
    )
}

fn click() -> Value {
    json!({"command": {"action": "click", "selector": "#merge"}})
}

/// An edit inside the run's folder, which Full allows.
fn edit() -> Value {
    let file = std::env::temp_dir().canonicalize().unwrap().join("a.rs");
    json!({"file_path": file.to_string_lossy()})
}

fn allow() -> Option<PermissionDecision> {
    Some(PermissionDecision::Allow)
}

const PULL: &str = "https://github.com/o/r/pull/1";

#[tokio::test]
async fn a_gated_call_waits_while_the_owner_has_the_computer() {
    let (_dir, runtime, host, _fake) = open_computer().await;
    let run = running(&runtime, "a").await;
    request(&host, ComputerRequest::TakeControl).await;
    let task = spawn_decide(
        &runtime,
        AccessLevel::Full,
        true,
        &run,
        "Edit",
        edit(),
        LONG_WAIT,
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!task.is_finished(), "the call waits for the computer");
    request(&host, ComputerRequest::HandBack).await;
    assert_eq!(decided(task).await, allow());
}

#[tokio::test]
async fn the_wait_ends_with_a_note_when_control_is_never_handed_back() {
    let (_dir, runtime, host, _fake) = open_computer().await;
    let run = running(&runtime, "a").await;
    request(&host, ComputerRequest::TakeControl).await;
    let task = browse(
        &runtime,
        &run,
        json!({"command": {"action": "snapshot"}}),
        SHORT_WAIT,
    );
    assert_eq!(decided(task).await, deny(CONTROL_WAIT_EXPIRED));
    assert!(approvals(&runtime, "a").await.is_empty());
}

#[tokio::test]
async fn stop_releases_a_run_waiting_for_the_computer() {
    let (_dir, runtime, host, _fake) = open_computer().await;
    let run = running(&runtime, "a").await;
    request(&host, ComputerRequest::TakeControl).await;
    let task = browse(&runtime, &run, click(), LONG_WAIT);
    tokio::time::sleep(Duration::from_millis(50)).await;
    runtime
        .command(ReactiveCommand::StopBruh {
            agent_id: "a".into(),
        })
        .await
        .unwrap();
    let decision = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("stop releases the wait at once")
        .unwrap()
        .unwrap();
    assert_eq!(decision, None);
}

#[tokio::test]
async fn turning_computer_off_releases_a_run_waiting_for_it() {
    let (_dir, runtime, host, _fake) = open_computer().await;
    let run = running(&runtime, "a").await;
    request(&host, ComputerRequest::TakeControl).await;
    let task = spawn_decide(
        &runtime,
        AccessLevel::Full,
        true,
        &run,
        "Edit",
        edit(),
        LONG_WAIT,
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    runtime.command(set_computer("a", false)).await.unwrap();
    // The slot is gone, and nothing holds the run any more.
    let decision = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("a removed slot releases the wait")
        .unwrap()
        .unwrap();
    assert_eq!(decision, allow());
}

#[tokio::test]
async fn browser_calls_are_gated_on_the_page_they_act_on() {
    let (_dir, runtime, _host, fake) = open_computer().await;
    let run = running(&runtime, "a").await;
    fake.set_url("a", PULL);
    // A page the model names never counts: the page is asked.
    let mut lie = click();
    lie["page_url"] = "https://shop.test/".into();
    let task = browse(&runtime, &run, lie, LONG_WAIT);
    let item = waiting_approval(&runtime, "a").await;
    assert_eq!(item.message, "Click #merge on github.com");
    assert_eq!(approvals(&runtime, "a").await.len(), 1);
    decide_approval(&runtime, &item.id, true).await;
    assert_eq!(decided(task).await, allow());
    let command = &click()["command"];
    let allowance = runtime
        .consume_browser_allowance("a", command)
        .await
        .unwrap()
        .expect("an approved step may run once");
    assert_eq!(allowance.run_id, run.id);
    assert_eq!(allowance.page_url.as_deref(), Some(PULL));
    assert_eq!(
        runtime
            .consume_browser_allowance("a", command)
            .await
            .unwrap(),
        None
    );

    fake.set_url("a", "https://shop.test/");
    assert_eq!(
        decided(browse(&runtime, &run, click(), LONG_WAIT)).await,
        allow()
    );
    assert_eq!(approvals(&runtime, "a").await.len(), 1);

    let local = json!({"command": {"action": "navigate", "url": "http://localhost:1"}});
    assert_eq!(
        decided(browse(&runtime, &run, local.clone(), LONG_WAIT)).await,
        deny(LOCAL_HOST_DENIAL)
    );
    assert_eq!(approvals(&runtime, "a").await.len(), 1);
    assert_eq!(
        runtime
            .consume_browser_allowance("a", &local["command"])
            .await
            .unwrap(),
        None,
        "a denied step leaves no allowance"
    );
}

#[tokio::test]
async fn a_late_browser_grant_matches_the_same_page_only() {
    let (_dir, runtime, _host, fake) = open_computer().await;
    let run = running(&runtime, "a").await;
    fake.set_url("a", PULL);
    assert_eq!(
        decided(browse(&runtime, &run, click(), SHORT_WAIT)).await,
        deny(NOT_ANSWERED)
    );
    let [item] = approvals(&runtime, "a").await.try_into().unwrap();
    finish(&runtime, &run.id, "Waiting on the owner's approval.").await;
    decide_approval(&runtime, &item.id, true).await;
    let resumed = claim_one(&runtime).await.unwrap().run;
    // Another page asks again, and leaves the grant for its own page.
    fake.set_url("a", "https://github.com/o/r/pull/2");
    assert_eq!(
        decided(browse(&runtime, &resumed, click(), SHORT_WAIT)).await,
        deny(NOT_ANSWERED)
    );
    assert_eq!(approvals(&runtime, "a").await.len(), 2);
    fake.set_url("a", PULL);
    assert_eq!(
        decided(browse(&runtime, &resumed, click(), LONG_WAIT)).await,
        allow()
    );
    assert_eq!(approvals(&runtime, "a").await.len(), 2);
    let allowance = runtime
        .consume_browser_allowance("a", &click()["command"])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(allowance.run_id, resumed.id);
}

#[tokio::test]
async fn input_approved_on_one_page_is_refused_on_another() {
    let (_dir, runtime, _host, fake) = open_computer().await;
    let run = running(&runtime, "a").await;
    fake.set_url("a", PULL);
    let task = browse(&runtime, &run, click(), LONG_WAIT);
    let item = waiting_approval(&runtime, "a").await;
    fake.set_url("a", "https://shop.test/");
    decide_approval(&runtime, &item.id, true).await;
    assert_eq!(decided(task).await, deny(PAGE_CHANGED));
    assert_eq!(
        runtime
            .consume_browser_allowance("a", &click()["command"])
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn input_on_a_page_that_does_not_answer_follows_the_forge_rule() {
    let (_dir, runtime, host, fake) = open_computer().await;
    let run = running(&runtime, "a").await;
    fake.set_url("a", "https://shop.test/");
    // The slot learns the address from a step.
    host.bruh_command("a", BrowserAction::Snapshot)
        .await
        .unwrap();
    assert_eq!(
        host.computer_state("a").await.unwrap().url.as_deref(),
        Some("https://shop.test/")
    );
    // The page answers nothing now; its last address doesn't count.
    fake.close_window("a");
    assert_eq!(
        decided(browse(&runtime, &run, click(), SHORT_WAIT)).await,
        deny(NOT_ANSWERED)
    );
    let [item] = approvals(&runtime, "a").await.try_into().unwrap();
    assert_eq!(item.message, "Click #merge");
}

#[tokio::test]
async fn only_the_bruhs_own_browser_tool_passes_the_gate() {
    let (_dir, runtime, _host, _fake) = open_computer().await;
    let run = running(&runtime, "a").await;
    let snapshot = json!({"command": {"action": "snapshot"}});
    let decision = |tool: &str, computer| {
        spawn_decide(
            &runtime,
            AccessLevel::Full,
            computer,
            &run,
            tool,
            snapshot.clone(),
            SHORT_WAIT,
        )
    };
    assert_eq!(decided(decision(BRUH_BROWSER_TOOL, true)).await, allow());
    for tool in [
        "mcp__bruhs_workbench__workspace_status",
        "mcp__other__local_browser",
        "mcp__bruhs_workbench__local_browser_2",
    ] {
        assert_eq!(
            decided(decision(tool, true)).await,
            deny(APP_TOOL_DENIAL),
            "{tool}"
        );
    }
    assert_eq!(
        decided(decision(BRUH_BROWSER_TOOL, false)).await,
        deny(COMPUTER_OFF_DENIAL)
    );
}

#[tokio::test]
async fn an_allowance_is_one_shot_per_call_and_ends_with_its_run() {
    let (_dir, runtime, _host, fake) = open_computer().await;
    let run = running(&runtime, "a").await;
    fake.set_url("a", "https://shop.test/");
    let command = &click()["command"];
    // Two identical calls: each may run once.
    for _ in 0..2 {
        assert_eq!(
            decided(browse(&runtime, &run, click(), LONG_WAIT)).await,
            allow()
        );
    }
    let other = json!({"action": "click", "selector": "#other"});
    assert_eq!(
        runtime
            .consume_browser_allowance("a", &other)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        runtime
            .consume_browser_allowance("b", command)
            .await
            .unwrap(),
        None
    );
    for _ in 0..2 {
        assert!(runtime
            .consume_browser_allowance("a", command)
            .await
            .unwrap()
            .is_some());
    }
    assert_eq!(
        runtime
            .consume_browser_allowance("a", command)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        decided(browse(&runtime, &run, click(), LONG_WAIT)).await,
        allow()
    );
    runtime
        .command(ReactiveCommand::StopBruh {
            agent_id: "a".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        runtime
            .consume_browser_allowance("a", command)
            .await
            .unwrap(),
        None,
        "a stopped run's allowance is gone"
    );
}
