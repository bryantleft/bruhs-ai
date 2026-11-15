use super::*;
use bruhs_models::{
    coordination::board::{Lane, PullState},
    delivery::{Pull, PullWatch, WatchRequest},
};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct Fake {
    branch_reads: AtomicUsize,
    pull_reads: AtomicUsize,
    fail_pulls: std::sync::atomic::AtomicBool,
    pulls: std::sync::Mutex<Vec<Pull>>,
    watches: Vec<PullWatch>,
}

impl LinkSource for Arc<Fake> {
    fn branch<'a>(&'a self, path: &'a Path) -> LinkFuture<'a, Result<Option<String>, String>> {
        self.branch_reads.fetch_add(1, Ordering::SeqCst);
        let branch = path
            .file_name()
            .map(|name| format!("bruhs/{}", name.to_string_lossy()));
        Box::pin(async move { Ok(branch) })
    }
    fn pulls<'a>(
        &'a self,
        _: &'a Path,
        branch: &'a str,
    ) -> LinkFuture<'a, Result<Vec<Pull>, String>> {
        self.pull_reads.fetch_add(1, Ordering::SeqCst);
        let result = if self.fail_pulls.load(Ordering::SeqCst) {
            Err("gh did not answer within 8 seconds.".to_owned())
        } else {
            Ok(self
                .pulls
                .lock()
                .unwrap()
                .iter()
                .filter(|pull| pull.head == branch)
                .cloned()
                .collect())
        };
        Box::pin(async move { result })
    }
    fn watches(&self) -> LinkFuture<'_, Vec<PullWatch>> {
        let watches = self.watches.clone();
        Box::pin(async move { watches })
    }
}

fn pull(number: u64, head: &str, review: &str) -> Pull {
    Pull {
        number,
        title: "Fix toast".into(),
        body: String::new(),
        url: format!("https://github.com/o/r/pull/{number}"),
        state: "OPEN".into(),
        draft: false,
        base: "main".into(),
        head: head.into(),
        head_oid: "abc".into(),
        head_in_repository: true,
        base_oid: String::new(),
        merge_commit_oid: None,
        updated_at: String::new(),
        mergeable: "MERGEABLE".into(),
        review_decision: Some(review.into()),
        checks: vec![],
        comments: vec![],
        reviews: vec![],
        files: vec![],
    }
}

fn worktree_task(name: &str) -> CoordinatedTask {
    let mut task = fixture(None, TaskState::Completed);
    task.working_path = format!("/project/.worktrees/{name}");
    task
}

/// Ask until the background refresh has settled.
async fn settled(
    links: &BoardLinks,
    snapshot: &CoordinationSnapshot,
) -> bruhs_models::coordination::board::TaskBoard {
    for _ in 0..200 {
        let board = links.board(snapshot).await;
        if !board.refreshing {
            return board;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    panic!("the board never finished refreshing");
}

fn snapshot(tasks: Vec<CoordinatedTask>) -> CoordinationSnapshot {
    CoordinationSnapshot {
        can_control: true,
        tasks,
        ..CoordinationSnapshot::default()
    }
}

#[tokio::test]
async fn worktree_tasks_pick_up_their_branch_and_pull_request_in_the_background() {
    let fake = Arc::new(Fake::default());
    fake.pulls
        .lock()
        .unwrap()
        .push(pull(7974, "bruhs/cloud-label", "REVIEW_REQUIRED"));
    let links = BoardLinks::new(Arc::clone(&fake));
    let snapshot = snapshot(vec![worktree_task("cloud-label")]);

    let first = links.board(&snapshot).await;
    assert_eq!(
        first.cards[0].lane,
        Lane::Building,
        "answers from cache at once"
    );
    assert!(first.refreshing);

    let board = settled(&links, &snapshot).await;
    let card = &board.cards[0];
    assert_eq!(card.branch.as_deref(), Some("bruhs/cloud-label"));
    assert_eq!(card.pulls[0].number, 7974);
    assert_eq!(
        (card.lane, card.status.label.as_str()),
        (Lane::InReview, "Awaiting review")
    );

    // Within the cache window no CLI runs again.
    let reads = (
        fake.branch_reads.load(Ordering::SeqCst),
        fake.pull_reads.load(Ordering::SeqCst),
    );
    links.board(&snapshot).await;
    links.board(&snapshot).await;
    assert_eq!(
        (
            fake.branch_reads.load(Ordering::SeqCst),
            fake.pull_reads.load(Ordering::SeqCst)
        ),
        reads
    );
    assert_eq!(reads, (1, 1));
}

#[tokio::test]
async fn tasks_in_the_project_folder_never_run_branch_lookups() {
    let fake = Arc::new(Fake::default());
    let links = BoardLinks::new(Arc::clone(&fake));
    let mut shared = fixture(None, TaskState::Completed);
    shared.working_path = shared.project_path.clone();
    let board = links.board(&snapshot(vec![shared])).await;
    assert!(!board.refreshing);
    assert_eq!(board.cards[0].branch, None);
    assert_eq!(fake.branch_reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn archived_tasks_are_not_refreshed() {
    let fake = Arc::new(Fake::default());
    let links = BoardLinks::new(Arc::clone(&fake));
    let mut archived = worktree_task("old");
    archived.archived_at = Some(EpochMs::now());
    let board = links.board(&snapshot(vec![archived])).await;
    assert!(!board.refreshing);
    assert!(board.cards[0].archived);
    assert_eq!(fake.branch_reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_failed_forge_lookup_is_reported_and_keeps_the_card() {
    let fake = Arc::new(Fake::default());
    fake.fail_pulls.store(true, Ordering::SeqCst);
    let links = BoardLinks::new(Arc::clone(&fake));
    let snapshot = snapshot(vec![worktree_task("toast")]);
    links.board(&snapshot).await;
    let board = settled(&links, &snapshot).await;
    assert_eq!(
        board.link_error.as_deref(),
        Some("gh did not answer within 8 seconds.")
    );
    assert_eq!(board.cards[0].branch.as_deref(), Some("bruhs/toast"));
    assert_eq!(board.cards[0].lane, Lane::Building);
}

#[tokio::test]
async fn an_approved_delivery_watch_links_a_pull_request_to_a_shared_task() {
    let mut task = fixture(None, TaskState::Completed);
    task.working_path = task.project_path.clone();
    let mut merged = pull(6625, "main", "APPROVED");
    merged.state = "MERGED".into();
    let fake = Arc::new(Fake {
        watches: vec![PullWatch {
            id: "watch-1".into(),
            request: WatchRequest {
                forge: bruhs_models::delivery::Forge::GitHub,
                path: "/project".into(),
                number: 6625,
                poll_seconds: 60,
                reactive_source_id: None,
            },
            repository: None,
            enabled: true,
            next_poll_at_ms: 0,
            previous: Some(merged),
            owner_login: "me".into(),
            error: None,
        }],
        ..Fake::default()
    });
    let mut snapshot = snapshot(vec![task.clone()]);
    snapshot.wake_subscriptions.push(WakeSubscription {
        id: ThreadId::new_v4(),
        task_id: task.id.clone(),
        watch_id: "watch-1".into(),
        kinds: vec![WakeKind::Review],
        project_path: task.project_path.clone(),
        model: task.model.clone(),
        permission_mode: task.permission_mode,
        enabled: true,
        approved_from_ms: 0,
    });
    let links = BoardLinks::new(fake);
    let board = links.board(&snapshot).await;
    assert_eq!(board.cards[0].pulls[0].state, PullState::Merged);
    assert_eq!(board.cards[0].lane, Lane::Done);
}

#[test]
fn lookup_errors_drop_receipt_advice() {
    let error = crate::delivery::DeliveryError::Command(
        "gh exited with exit status: 1: HTTP 401. Earlier successful steps remain applied; inspect the receipt before preparing another action.".into(),
    );
    assert_eq!(
        super::super::board::lookup_error(error),
        "gh exited with exit status: 1: HTTP 401."
    );
}

#[tokio::test]
async fn a_guest_board_never_touches_the_owner_cache() {
    let fake = Arc::new(Fake::default());
    fake.pulls
        .lock()
        .unwrap()
        .push(pull(7974, "bruhs/cloud-label", "REVIEW_REQUIRED"));
    let links = BoardLinks::new(Arc::clone(&fake));
    let owner = snapshot(vec![worktree_task("cloud-label")]);
    links.board(&owner).await;
    settled(&links, &owner).await;
    let reads = fake.pull_reads.load(Ordering::SeqCst);

    // A guest sees its one task without paths, exactly as `scoped_snapshot` builds it.
    let mut task = owner.tasks[0].clone();
    task.project_path.clear();
    task.working_path.clear();
    let guest = CoordinationSnapshot {
        tasks: vec![task],
        ..CoordinationSnapshot::default()
    };
    let board = links.board(&guest).await;
    assert!(!board.refreshing && board.link_error.is_none());
    assert_eq!(board.cards[0].branch, None);

    let board = links.board(&owner).await;
    assert_eq!(
        board.cards[0].pulls[0].number, 7974,
        "the owner's links survive"
    );
    assert!(!board.refreshing);
    assert_eq!(fake.pull_reads.load(Ordering::SeqCst), reads);
}

#[tokio::test]
async fn a_fork_pull_request_with_the_same_branch_name_is_not_the_task_s() {
    let fake = Arc::new(Fake::default());
    let mut fork = pull(41, "bruhs/toast", "REVIEW_REQUIRED");
    fork.head_in_repository = false;
    fake.pulls.lock().unwrap().push(fork);
    let links = BoardLinks::new(Arc::clone(&fake));
    let snapshot = snapshot(vec![worktree_task("toast")]);
    links.board(&snapshot).await;
    let board = settled(&links, &snapshot).await;
    assert!(board.cards[0].pulls.is_empty());
}
