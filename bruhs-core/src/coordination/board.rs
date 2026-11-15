//! Host side of the orchestration board: the branch each task's worktree is
//! on and the pull requests opened from it or linked through a delivery
//! watch. Lookups run in the background and are cached, so a board request
//! answers from memory and never waits on `git`, `gh` or `glab`.
use bruhs_models::{
    coordination::{
        board::{self, PullState, TaskBoard, TaskLink, TaskPull},
        CoordinationSnapshot,
    },
    delivery::{Pull, PullWatch},
    ids::ThreadId,
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    future::Future,
    path::Path,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore};

/// An agent can switch branches mid-task; re-read soon after.
const BRANCH_TTL: Duration = Duration::from_secs(20);
/// Open or missing pull requests change with reviews and checks. At most one
/// forge call per worktree per window, and only while someone has the board open.
const OPEN_TTL: Duration = Duration::from_secs(60);
/// Merged or closed requests rarely change again.
const SETTLED_TTL: Duration = Duration::from_secs(15 * 60);
/// Forge CLIs at once; each takes up to the delivery read deadline (8s).
const LOOKUPS_AT_ONCE: usize = 4;

pub type LinkFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Where links come from. The host uses [`crate::delivery::DeliveryService`];
/// tests substitute a fake so no CLI runs.
pub trait LinkSource: Send + Sync {
    fn branch<'a>(&'a self, path: &'a Path) -> LinkFuture<'a, Result<Option<String>, String>>;
    fn pulls<'a>(
        &'a self,
        path: &'a Path,
        branch: &'a str,
    ) -> LinkFuture<'a, Result<Vec<Pull>, String>>;
    fn watches(&self) -> LinkFuture<'_, Vec<PullWatch>>;
}

impl LinkSource for crate::delivery::DeliveryService {
    fn branch<'a>(&'a self, path: &'a Path) -> LinkFuture<'a, Result<Option<String>, String>> {
        Box::pin(async move { self.current_branch(path).await.map_err(lookup_error) })
    }
    fn pulls<'a>(
        &'a self,
        path: &'a Path,
        branch: &'a str,
    ) -> LinkFuture<'a, Result<Vec<Pull>, String>> {
        Box::pin(async move {
            self.pulls_from_branch(path, branch)
                .await
                .map_err(lookup_error)
        })
    }
    fn watches(&self) -> LinkFuture<'_, Vec<PullWatch>> {
        Box::pin(async move { self.snapshot().await.watches })
    }
}

/// Delivery errors end with advice about action receipts; a lookup has none.
pub(super) fn lookup_error(error: crate::delivery::DeliveryError) -> String {
    let message = error.to_string();
    message
        .split(" Earlier successful steps remain applied")
        .next()
        .unwrap_or(&message)
        .trim_end_matches(['.', ';', ' '])
        .to_owned()
        + "."
}

struct Stamped<T> {
    value: T,
    at: Instant,
}

#[derive(Default)]
struct Cache {
    /// Working directory → checked-out branch.
    branches: HashMap<String, Stamped<Option<String>>>,
    /// (working directory, branch) → requests from that branch.
    pulls: HashMap<(String, String), Stamped<Vec<TaskPull>>>,
    /// Working directory → the last lookup failure there.
    errors: BTreeMap<String, String>,
}

struct Inner {
    source: Arc<dyn LinkSource>,
    cache: Mutex<Cache>,
    refreshing: AtomicBool,
}

#[derive(Clone)]
pub struct BoardLinks(Arc<Inner>);

impl BoardLinks {
    pub fn new(source: impl LinkSource + 'static) -> Self {
        Self(Arc::new(Inner {
            source: Arc::new(source),
            cache: Mutex::new(Cache::default()),
            refreshing: AtomicBool::new(false),
        }))
    }

    /// The board for `snapshot` from what is cached now. Stale lookups start
    /// in the background and `refreshing` tells the reader to ask again.
    pub async fn board(&self, snapshot: &CoordinationSnapshot) -> TaskBoard {
        // A guest's scoped snapshot has no paths: it gets lanes from task state
        // alone, and never touches the owner's cache or sees host lookup errors.
        if !snapshot.can_control {
            return TaskBoard {
                revision: snapshot.revision,
                cards: board::cards(snapshot, &BTreeMap::new()),
                refreshing: false,
                link_error: None,
            };
        }
        let watched = watched_pulls(snapshot, &self.0.source.watches().await);
        let worktrees = worktrees(snapshot);
        let now = Instant::now();
        let (links, stale, link_error) = {
            let mut cache = self.0.cache.lock().await;
            let live = worktrees.values().cloned().collect::<HashSet<_>>();
            cache.branches.retain(|path, _| live.contains(path));
            cache.pulls.retain(|(path, _), _| live.contains(path));
            cache.errors.retain(|path, _| live.contains(path));
            let mut stale = Vec::new();
            let mut links = BTreeMap::new();
            for task in &snapshot.tasks {
                let mut link = TaskLink::default();
                if let Some(path) = worktrees.get(&task.id) {
                    let branch = cache.branches.get(path);
                    link.branch = branch.and_then(|entry| entry.value.clone());
                    let pulls = link
                        .branch
                        .as_ref()
                        .and_then(|branch| cache.pulls.get(&(path.clone(), branch.clone())));
                    if let Some(pulls) = pulls {
                        link.pulls = pulls.value.clone();
                    }
                    // Archived work keeps what was read last; it is not refreshed.
                    let due = branch.is_none_or(|entry| now.duration_since(entry.at) >= BRANCH_TTL)
                        || (link.branch.is_some()
                            && pulls.is_none_or(|entry| {
                                now.duration_since(entry.at) >= pulls_ttl(&entry.value)
                            }));
                    if due && task.archived_at.is_none() && !stale.contains(path) {
                        stale.push(path.clone());
                    }
                }
                for pull in watched.get(&task.id).into_iter().flatten() {
                    if !link.pulls.iter().any(|known| known.url == pull.url) {
                        link.pulls.push(pull.clone());
                    }
                }
                if link != TaskLink::default() {
                    links.insert(task.id.clone(), link);
                }
            }
            (links, stale, cache.errors.values().next().cloned())
        };
        if !stale.is_empty() {
            self.refresh(stale);
        }
        TaskBoard {
            revision: snapshot.revision,
            cards: board::cards(snapshot, &links),
            refreshing: self.0.refreshing.load(Ordering::Acquire),
            link_error,
        }
    }

    /// One refresh at a time; a request during one is served from cache and
    /// told to ask again.
    fn refresh(&self, paths: Vec<String>) {
        if self.0.refreshing.swap(true, Ordering::AcqRel) {
            return;
        }
        let inner = Arc::clone(&self.0);
        tokio::spawn(async move {
            struct Done(Arc<Inner>);
            impl Drop for Done {
                fn drop(&mut self) {
                    self.0.refreshing.store(false, Ordering::Release);
                }
            }
            let done = Done(inner);
            let slots = Arc::new(Semaphore::new(LOOKUPS_AT_ONCE));
            let mut lookups = tokio::task::JoinSet::new();
            for path in paths {
                let inner = Arc::clone(&done.0);
                let slots = Arc::clone(&slots);
                lookups.spawn(async move {
                    let Ok(_slot) = slots.acquire_owned().await else {
                        return;
                    };
                    refresh_path(&inner, path).await;
                });
            }
            while lookups.join_next().await.is_some() {}
        });
    }
}

fn pulls_ttl(pulls: &[TaskPull]) -> Duration {
    let settled = !pulls.is_empty()
        && pulls
            .iter()
            .all(|pull| matches!(pull.state, PullState::Merged | PullState::Closed));
    if settled {
        SETTLED_TTL
    } else {
        OPEN_TTL
    }
}

async fn refresh_path(inner: &Inner, path: String) {
    let branch = match inner.source.branch(Path::new(&path)).await {
        Ok(branch) => branch,
        Err(error) => {
            let mut cache = inner.cache.lock().await;
            cache.errors.insert(path.clone(), error);
            // Keep the last known branch, and wait a full window before retrying.
            let previous = cache.branches.remove(&path).and_then(|entry| entry.value);
            cache.branches.insert(
                path,
                Stamped {
                    value: previous,
                    at: Instant::now(),
                },
            );
            drop(cache);
            return;
        }
    };
    let fresh_pulls = {
        let mut cache = inner.cache.lock().await;
        cache.branches.insert(
            path.clone(),
            Stamped {
                value: branch.clone(),
                at: Instant::now(),
            },
        );
        let fresh = branch.as_ref().is_some_and(|branch| {
            cache
                .pulls
                .get(&(path.clone(), branch.clone()))
                .is_some_and(|entry| entry.at.elapsed() < pulls_ttl(&entry.value))
        });
        drop(cache);
        fresh
    };
    let Some(branch) = branch else {
        inner.cache.lock().await.errors.remove(&path);
        return;
    };
    if fresh_pulls {
        return;
    }
    let result = inner.source.pulls(Path::new(&path), &branch).await;
    let mut cache = inner.cache.lock().await;
    let pulls = match result {
        Ok(pulls) => {
            cache.errors.remove(&path);
            // `--head` matches a branch name in any fork; only this repository's
            // branch is the task's.
            pulls
                .iter()
                .filter(|pull| pull.head_in_repository && pull.head == branch)
                .map(TaskPull::from_pull)
                .collect()
        }
        Err(error) => {
            cache.errors.insert(path.clone(), error);
            cache
                .pulls
                .remove(&(path.clone(), branch.clone()))
                .map(|entry| entry.value)
                .unwrap_or_default()
        }
    };
    cache.pulls.insert(
        (path, branch),
        Stamped {
            value: pulls,
            at: Instant::now(),
        },
    );
    drop(cache);
}

/// Tasks that own a worktree, by id → working directory. A task in the
/// project folder shares whatever branch the owner has checked out, so its
/// branch says nothing about the task; it links only through watches.
fn worktrees(snapshot: &CoordinationSnapshot) -> HashMap<ThreadId, String> {
    snapshot
        .tasks
        .iter()
        .filter(|task| board::has_card(task, snapshot) && task.working_path != task.project_path)
        .map(|task| (task.id.clone(), task.working_path.clone()))
        .collect()
}

/// Pull requests the owner tied to a task by approving a delivery watch to
/// wake it, as last polled by that watch.
fn watched_pulls(
    snapshot: &CoordinationSnapshot,
    watches: &[PullWatch],
) -> HashMap<ThreadId, Vec<TaskPull>> {
    let mut linked: HashMap<ThreadId, Vec<TaskPull>> = HashMap::new();
    for subscription in snapshot.wake_subscriptions.iter().filter(|s| s.enabled) {
        let pull = watches
            .iter()
            .find(|watch| watch.enabled && watch.id == subscription.watch_id)
            .and_then(|watch| watch.previous.as_ref());
        if let Some(pull) = pull {
            linked
                .entry(subscription.task_id.clone())
                .or_default()
                .push(TaskPull::from_pull(pull));
        }
    }
    linked
}
