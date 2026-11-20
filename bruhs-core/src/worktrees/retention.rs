//! Periodic removal of idle desktop checkouts. The desktop owns the atomic
//! workspace file; the host owns the Git operations and can run while it is closed.
use super::*;
use crate::models::TaskStatus;
use crate::{
    agent::ThreadStatus, coordination::CoordinationRuntime, delivery::DeliveryService,
    git::retention as git, store::AppState,
};
use bruhs_models::{
    coordination::worktree::{CleanupMode, ScratchCleanup, WorktreeRetention},
    ids::ThreadId,
};
use serde::Deserialize;
use std::time::{Duration, Instant};

const DAY: i64 = 86_400_000;
const GRACE: i64 = 60_000;

#[derive(Default, Deserialize)]
#[serde(default)]
struct Workspace {
    threads: Vec<Thread>,
    archived_threads: Vec<Archived>,
    recent_projects: Vec<Project>,
    project_list: ProjectList,
    project_defaults: BTreeMap<String, Override>,
    thread_origins: BTreeMap<String, serde_json::Value>,
    worktree_retention: WorktreeRetention,
    pending_scratch_cleanups: Vec<ScratchCleanup>,
    cleanup_keep_when: KeepWhen,
    cleanup_deleted_worktrees: bool,
}
#[derive(Deserialize)]
struct Archived {
    thread: Thread,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct ProjectList {
    entries: Vec<Project>,
}
#[derive(Deserialize)]
struct Project {
    path: PathBuf,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Override {
    cleanup_mode: Option<CleanupMode>,
    cleanup_deleted_worktrees: Option<bool>,
    worktree_retention: Option<WorktreeRetention>,
    cleanup_keep_when: Option<KeepWhen>,
}
#[derive(Deserialize)]
struct Thread {
    id: String,
    project_path: Option<PathBuf>,
    project_settings_root: Option<PathBuf>,
    branch: Option<String>,
    worktree_path: Option<PathBuf>,
    #[serde(default, deserialize_with = "bruhs_models::time::deserialize_legacy")]
    updated_at_ms: EpochMs,
    #[serde(default)]
    settled: bool,
    settled_at_ms: Option<EpochMs>,
    #[serde(default)]
    cleanup_busy: bool,
    #[serde(default)]
    prompt_queue: Vec<serde::de::IgnoredAny>,
}

#[derive(Clone, PartialEq, Eq)]
struct Candidate {
    id: ThreadId,
    members: BTreeMap<ThreadId, (i64, Option<i64>)>,
    path: PathBuf,
    branch: Option<String>,
    scope: PathBuf,
    activity: i64,
    settled: bool,
    settled_at: Option<i64>,
    archived: bool,
    rules: WorktreeRetention,
    keep: KeepWhen,
}
impl Workspace {
    async fn load(file: &Path) -> Result<Self, String> {
        let data = tokio::fs::read(file).await.map_err(|e| e.to_string())?;
        tokio::task::spawn_blocking(move || serde_json::from_slice(&data))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())
    }
    async fn load_candidates(
        file: &Path,
        checking: Option<(&Path, &Path)>,
    ) -> Result<Vec<Candidate>, String> {
        let mut workspace = Self::load(file).await?;
        if !workspace.enabled() {
            return Ok(Vec::new());
        }
        // Older desktop files sometimes store the linked checkout as the project. Use its
        // original project for inheritance, including the final check after moving it aside.
        let mut projects = BTreeMap::new();
        for thread in workspace
            .threads
            .iter_mut()
            .chain(workspace.archived_threads.iter_mut().map(|a| &mut a.thread))
        {
            let Some(path) = thread.worktree_path.as_deref() else {
                continue;
            };
            if (thread.branch.is_none() && scratch::is_managed_path(path))
                || thread.project_settings_root.is_some()
                || thread.project_path.as_deref() != Some(path)
            {
                continue;
            }
            let path = resolve(path);
            if !projects.contains_key(&path) {
                let main = match checking {
                    Some((current, project)) if current == path => Some(project.to_owned()),
                    _ => crate::git::main_checkout(&path).await,
                };
                projects.insert(path.clone(), main);
            }
            thread.project_settings_root = projects[&path].clone();
            if thread.project_settings_root.is_none() {
                thread.cleanup_busy = true;
            }
        }
        tokio::task::spawn_blocking(move || workspace.candidates())
            .await
            .map_err(|e| e.to_string())
    }
    fn enabled(&self) -> bool {
        self.worktree_retention != WorktreeRetention::default()
            || self.project_defaults.values().any(|p| {
                p.worktree_retention
                    .is_some_and(|r| r != WorktreeRetention::default())
            })
    }
    fn threads(&self) -> impl Iterator<Item = (&Thread, bool)> {
        self.threads
            .iter()
            .map(|t| (t, false))
            .chain(self.archived_threads.iter().map(|a| (&a.thread, true)))
    }
    fn projects(&self) -> impl Iterator<Item = &Project> {
        self.recent_projects
            .iter()
            .chain(&self.project_list.entries)
    }
    fn candidates(&self) -> Vec<Candidate> {
        let mut paths = BTreeMap::<PathBuf, PathBuf>::new();
        let mut canonical = |p: &Path| {
            paths
                .entry(p.to_owned())
                .or_insert_with(|| resolve(p))
                .clone()
        };
        let mut owners = BTreeMap::<PathBuf, Vec<(&Thread, bool)>>::new();
        let mut occupants = BTreeMap::<PathBuf, Vec<&str>>::new();
        for project in self.projects() {
            occupants
                .entry(canonical(&project.path))
                .or_default()
                .push("");
        }
        let mut ids = std::collections::BTreeSet::new();
        for (t, archived) in self.threads() {
            if !ids.insert(t.id.as_str()) {
                return Vec::new();
            }
            if let Some(p) = &t.project_path {
                if p.is_absolute() {
                    occupants.entry(canonical(p)).or_default().push(&t.id);
                }
            }
            if let Some(p) = &t.worktree_path {
                if p.is_absolute() {
                    let path = canonical(p);
                    occupants.entry(path.clone()).or_default().push(&t.id);
                    owners.entry(path).or_default().push((t, archived));
                }
            }
        }
        owners
            .into_iter()
            .filter_map(|(path, group)| {
                let members: BTreeMap<_, _> =
                    group.iter().map(|(t, _)| (t.id.as_str(), *t)).collect();
                let external = occupants
                    .range(path.clone()..)
                    .take_while(|(p, _)| p.starts_with(&path))
                    .flat_map(|(_, ids)| ids)
                    .any(|id| !members.contains_key(id));
                if external {
                    return None;
                }
                // Only a complete tree of side chats sharing the owner's branch can be reclaimed together.
                let parent =
                    |t: &Thread| self.thread_origins.get(&t.id)?.get("side_chat")?.as_str();
                let mut roots = group
                    .iter()
                    .filter(|(t, _)| parent(t).is_none_or(|p| !members.contains_key(p)));
                let &(t, archived) = roots.next()?;
                if roots.next().is_some() {
                    return None;
                }
                let mut activity = BTreeMap::new();
                for (member, _) in &group {
                    if member.cleanup_busy
                        || !member.prompt_queue.is_empty()
                        || member.branch != t.branch
                        || member.updated_at_ms.as_millis() <= 0
                    {
                        return None;
                    }
                    let mut ancestor = *member;
                    for _ in 0..group.len() {
                        if ancestor.id == t.id {
                            break;
                        }
                        ancestor = members.get(parent(ancestor)?)?;
                    }
                    if ancestor.id != t.id {
                        return None;
                    }
                    activity.insert(
                        ThreadId::try_from(member.id.as_str()).ok()?,
                        (
                            member.updated_at_ms.as_millis(),
                            member
                                .settled_at_ms
                                .filter(|_| member.settled)
                                .map(EpochMs::as_millis),
                        ),
                    );
                }
                self.candidate(path, t, archived, activity)
            })
            .collect()
    }
    fn policy(&self, scope: &Path) -> Option<(bool, WorktreeRetention, KeepWhen)> {
        let mut deleted = self.cleanup_deleted_worktrees;
        let mut rules = self.worktree_retention;
        let mut keep = self.cleanup_keep_when;
        if let Some(p) = self.project_defaults.get(scope.to_str()?) {
            match p.cleanup_mode.or(p.cleanup_deleted_worktrees.map(|b| {
                if b {
                    CleanupMode::Custom
                } else {
                    CleanupMode::Off
                }
            })) {
                Some(CleanupMode::Off) => return None,
                Some(CleanupMode::Custom) => {
                    deleted = p.cleanup_deleted_worktrees.unwrap_or(deleted);
                    rules = p.worktree_retention.unwrap_or(rules);
                    keep = p.cleanup_keep_when.unwrap_or(keep);
                }
                None => {}
            }
        }
        Some((deleted, rules, keep))
    }

    fn candidate(
        &self,
        path: PathBuf,
        t: &Thread,
        archived: bool,
        members: BTreeMap<ThreadId, (i64, Option<i64>)>,
    ) -> Option<Candidate> {
        let scope = t
            .project_settings_root
            .as_ref()
            .or(t.project_path.as_ref())?;
        let path = if t.branch.is_none() {
            let original = t.worktree_path.as_ref()?;
            if !scratch::is_managed_path(original) || resolve(original) != path || scope != original
            {
                return None;
            }
            original.clone()
        } else {
            path
        };
        let (_, rules, keep) = self.policy(scope)?;
        if rules == WorktreeRetention::default() {
            return None;
        }
        let activity = members.values().map(|(at, _)| *at).max()?;
        Some(Candidate {
            members,
            id: ThreadId::try_from(t.id.as_str()).ok()?,
            path,
            branch: t.branch.clone(),
            scope: scope.clone(),
            activity,
            settled: t.settled,
            settled_at: t
                .settled_at_ms
                .filter(|_| t.settled)
                .map(EpochMs::as_millis),
            archived,
            rules,
            keep,
        })
    }
}
impl Candidate {
    fn eligible(&self, now: i64) -> (bool, bool, bool) {
        if self.activity <= 0
            || self.activity > now
            || self.members.values().any(|(_, settled)| {
                settled.is_some_and(|s| now.saturating_sub(s.max(self.activity)) < GRACE)
            })
        {
            return (false, false, false);
        }
        let old = self
            .rules
            .inactive_days
            .is_some_and(|d| (1..=3650).contains(&d) && now - self.activity > i64::from(d) * DAY);
        let settled = self.settled_at.is_some_and(|s| {
            self.rules.settled_days.is_some_and(|d| {
                d <= 3650 && now.saturating_sub(s.max(self.activity)) >= i64::from(d) * DAY
            })
        });
        (
            old || settled,
            self.rules.unchanged && (self.settled || self.archived),
            self.rules.merged,
        )
    }
}

#[derive(Clone)]
pub struct RetentionHost {
    state: Arc<AppState>,
    coordination: CoordinationRuntime,
    delivery: DeliveryService,
    scratch_deletions: Arc<tokio::sync::Mutex<()>>,
}
impl RetentionHost {
    pub fn new(
        state: Arc<AppState>,
        coordination: CoordinationRuntime,
        delivery: DeliveryService,
    ) -> Self {
        Self {
            state,
            coordination,
            delivery,
            scratch_deletions: Default::default(),
        }
    }
    fn busy(&self, c: &Candidate) -> bool {
        c.members.keys().any(|id| {
            self.state.thread_status(id) == ThreadStatus::Streaming
                || self
                    .state
                    .registry()
                    .get_by_thread(id.as_str())
                    .iter()
                    .any(|t| t.status == TaskStatus::Running)
        }) || self
            .state
            .terminals()
            .list()
            .iter()
            .filter_map(|t| t.cwd.as_deref())
            .any(|p| resolve(Path::new(p)).starts_with(&c.path))
    }
    async fn scratch_available(&self, path: &Path, ids: Vec<String>) -> Result<(), String> {
        let path = resolve(path);
        if self.coordination.uses_scratch_folder(&path).await
            || self
                .state
                .terminals()
                .list()
                .iter()
                .filter_map(|t| t.cwd.as_deref())
                .any(|cwd| resolve(Path::new(cwd)).starts_with(&path))
        {
            return Err("A task or terminal uses the scratch folder.".into());
        }
        if let Some(runtime) = self.state.reactive() {
            if runtime
                .uses_scratch(path, ids)
                .await
                .map_err(|e| e.to_string())?
            {
                return Err("An enabled task uses the scratch folder.".into());
            }
        }
        Ok(())
    }

    pub async fn remove_deleted_scratch(&self, request: &ScratchCleanup) -> Result<(), String> {
        let _guard = self.scratch_deletions.lock().await;
        let path = Path::new(&request.path);
        let id = &request.thread_id;
        let keep = request.keep_when;
        if !scratch::is_managed_path(path) {
            return Err("Choose the deleted thread’s own scratch folder.".into());
        }
        let owner = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("Invalid scratch folder.")?;
        let participants = if owner == id.as_str() {
            vec![id.to_string()]
        } else {
            vec![id.to_string(), owner.to_owned()]
        };
        let file =
            crate::delivery::writer::desktop_state_file().ok_or("Desktop state unavailable.")?;
        let target = resolve(path);
        scratch::remove(path, keep, scratch::Removal::DeleteChat, || async {
            let ws = Workspace::load(&file).await?;
            if !ws.pending_scratch_cleanups.contains(request)
                || !ws
                    .policy(path)
                    .is_some_and(|(on, _, current)| on && current == keep)
                || ws.threads().any(|(t, _)| {
                    (t.id == id.as_str() || t.id == owner)
                        || [t.project_path.as_deref(), t.worktree_path.as_deref()]
                            .into_iter()
                            .flatten()
                            .any(|p| resolve(p).starts_with(&target))
                })
                || ws.projects().any(|p| resolve(&p.path).starts_with(&target))
                || self.state.thread_status(id) == ThreadStatus::Streaming
                || self
                    .state
                    .registry()
                    .get_by_thread(id.as_str())
                    .iter()
                    .any(|t| t.status == TaskStatus::Running)
            {
                return Err("The thread, folder use, or cleanup settings changed.".into());
            }
            self.scratch_available(path, participants.clone()).await
        })
        .await
    }

    pub async fn run(self, file: PathBuf) {
        let mut last = None;
        let mut next = Instant::now();
        let mut seen: Option<Vec<Candidate>> = None;
        loop {
            if let Ok(meta) = tokio::fs::metadata(&file).await {
                let version = (meta.modified().ok(), meta.len());
                let hourly = Instant::now() >= next;
                if hourly {
                    if let Ok(workspace) = Workspace::load(&file).await {
                        for request in workspace.pending_scratch_cleanups {
                            if let Err(reason) = self.remove_deleted_scratch(&request).await {
                                tracing::debug!(%reason, "scratch cleanup deferred");
                            }
                        }
                    }
                }
                if hourly || last != Some(version) {
                    if let Ok(candidates) = Workspace::load_candidates(&file, None).await {
                        if hourly || seen.as_ref() != Some(&candidates) {
                            self.sweep(&file, &candidates, EpochMs::now().as_millis())
                                .await;
                            seen = Some(candidates);
                        }
                        last = Some(version);
                        if hourly {
                            next = Instant::now() + Duration::from_secs(3600);
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }
    async fn sweep(&self, file: &Path, candidates: &[Candidate], now: i64) {
        let mut branches = BTreeMap::new();
        for c in candidates {
            if let Err(reason) = self.try_remove(file, c, now, &mut branches).await {
                tracing::debug!(thread=%c.id,%reason,"worktree retained");
            }
        }
    }
    async fn try_remove(
        &self,
        file: &Path,
        c: &Candidate,
        now: i64,
        branches: &mut BTreeMap<PathBuf, git::DefaultBranch>,
    ) -> Result<(), String> {
        let (due, unchanged, merged) = c.eligible(now);
        if !(due || unchanged || merged) || !c.path.is_dir() || self.busy(c) {
            return Ok(());
        }
        let Some(expected_branch) = &c.branch else {
            if !due {
                return Ok(());
            }
            return scratch::remove(&c.path, c.keep, scratch::Removal::RetainChat, || async {
                let current = Workspace::load_candidates(file, None).await?;
                if !current.contains(c) || self.busy(c) {
                    return Err("Thread activity or cleanup settings changed.".into());
                }
                self.scratch_available(&c.path, c.members.keys().map(ToString::to_string).collect())
                    .await
            })
            .await;
        };
        let root = locations::for_path(&c.path)
            .map_err(|e| e.to_string())?
            .ok_or("Outside the managed worktree folders")?;
        let project = crate::git::main_checkout(&c.path)
            .await
            .ok_or("Main checkout unavailable")?;
        if resolve(&c.scope) != resolve(&project) && resolve(&c.scope) != c.path {
            return Err("Project changed".into());
        }
        let tip = git::head(&c.path).await.map_err(|e| e.to_string())?;
        if !due {
            if !branches.contains_key(&project) {
                let base = git::default_branch(&project)
                    .await
                    .map_err(|e| e.to_string())?;
                branches.insert(project.clone(), base);
            }
            let base = &branches[&project];
            let integrated = git::integrated(&project, &tip, base)
                .await
                .map_err(|e| e.to_string())?;
            if !(unchanged && integrated) {
                if !merged || !base.remote {
                    return Ok(());
                }
                let pulls = self
                    .delivery
                    .pulls_from_branch(&c.path, expected_branch)
                    .await
                    .map_err(|e| e.to_string())?;
                let Some(pr) = pulls.first() else {
                    return Ok(());
                };
                if !pr.state.eq_ignore_ascii_case("merged")
                    || !pr.head_in_repository
                    || &pr.head != expected_branch
                    || !(integrated || (pr.head_oid == tip && pr.base == base.name))
                {
                    return Ok(());
                }
            }
        }
        let _lock = lock(&root).await?;
        if !c.path.exists() {
            return Ok(());
        }
        let branch = check_with(&root, &project, &c.path, &programs().await?, Some(c.keep)).await?;
        if &branch != expected_branch {
            return Err("Branch changed".into());
        }
        let guard = Check {
            host: self,
            project: &project,
            file,
            candidate: c,
            tip: &tip,
        };
        guard.validate(&c.path).await?;
        unregister_with_policy(
            &root,
            &project,
            &c.path,
            &branch,
            Some(c.keep),
            Some(&guard),
        )
        .await?;
        tracing::info!(thread=%c.id,path=%c.path.display(),"removed unused worktree");
        Ok(())
    }
}

pub(super) struct Check<'a> {
    host: &'a RetentionHost,
    file: &'a Path,
    project: &'a Path,
    candidate: &'a Candidate,
    tip: &'a str,
}
impl Check<'_> {
    pub(super) async fn validate(&self, checkout: &Path) -> Result<(), String> {
        let c = self.candidate;
        let current = Workspace::load_candidates(self.file, Some((&c.path, self.project)))
            .await?
            .into_iter()
            .find(|row| row.id == c.id);
        if current.as_ref() != Some(c)
            || self.host.busy(c)
            || self
                .host
                .coordination
                .task_in_worktree(&c.path)
                .await
                .is_some()
            || git::head(checkout).await.map_err(|e| e.to_string())? != self.tip
        {
            return Err("Thread activity, checkout, or cleanup settings changed".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_related_idle_chats_share_a_cleanup_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tree");
        let member = |id: &str| {
            json!({"id":id,"worktree_path":path,"project_path":dir.path(),
            "branch":"topic","updated_at_ms":DAY,"settled":true,"settled_at_ms":DAY})
        };
        let mut state = json!({"threads":[member("parent"),member("side"),member("grandchild")],
            "worktree_retention":{"unchanged":true,"settled_days":0,"inactive_days":2},
            "thread_origins":{"side":{"side_chat":"parent"},"grandchild":{"side_chat":"side"}}});
        let candidates = |state: &serde_json::Value| {
            serde_json::from_value::<Workspace>(state.clone())
                .unwrap()
                .candidates()
        };
        let c = candidates(&state).pop().unwrap();
        assert_eq!(c.id.as_str(), "parent");
        assert_eq!(c.members.len(), 3);
        assert_eq!(c.eligible(DAY + GRACE - 1), (false, false, false));
        assert_eq!(c.eligible(DAY + GRACE), (true, true, false));
        state["threads"][2]["settled_at_ms"] = (DAY + GRACE).into();
        assert_eq!(
            candidates(&state)[0].eligible(DAY + GRACE),
            (false, false, false)
        );
        state["threads"][1]["cleanup_busy"] = true.into();
        assert!(candidates(&state).is_empty());
        state["threads"][1]["cleanup_busy"] = false.into();
        state["thread_origins"]["side"] = json!({"handoff":"parent"});
        assert!(candidates(&state).is_empty());
        state["thread_origins"]["side"] = json!({"side_chat":"grandchild"});
        assert!(candidates(&state).is_empty());
    }

    #[test]
    fn a_registered_project_inside_a_checkout_keeps_it_after_it_leaves_recents() {
        let dir = tempfile::tempdir().unwrap();
        let tree = dir.path().join("tree");
        let ws: Workspace = serde_json::from_value(json!({
            "threads":[{"id":"owner","branch":"topic","project_path":dir.path(),"worktree_path":tree,"updated_at_ms":DAY}],
            "worktree_retention":{"inactive_days":1},
            "project_list":{"entries":[{"path":tree.join("nested")}]}
        })).unwrap();
        assert!(ws.candidates().is_empty());
    }

    #[test]
    fn inactive_uses_the_last_activity_and_unchanged_requires_settlement_or_archive() {
        let dir = tempfile::tempdir().unwrap();
        let t = json!({"id":"t","worktree_path":dir.path().join("checkout"),"project_path":dir.path(),
            "branch":"topic","updated_at_ms":DAY});
        let mut state = json!({"threads":[t],"worktree_retention":{"inactive_days":2,"unchanged":true,"merged":true}});
        let ws: Workspace = serde_json::from_value(state.clone()).unwrap();
        let c = ws.candidates().pop().unwrap();
        assert_eq!(c.eligible(DAY * 3), (false, false, true));
        assert_eq!(c.eligible(DAY * 3 + 1), (true, false, true));
        state["archived_threads"] = json!([{"thread":state["threads"][0]}]);
        state["threads"] = json!([]);
        let ws: Workspace = serde_json::from_value(state.clone()).unwrap();
        assert_eq!(ws.candidates()[0].eligible(DAY * 3), (false, true, true));
        state["project_defaults"][dir.path().to_str().unwrap()] = json!({"cleanup_mode":"off"});
        let ws: Workspace = serde_json::from_value(state).unwrap();
        assert!(ws.candidates().is_empty());
    }
}
