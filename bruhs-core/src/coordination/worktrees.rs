//! A finished task's own worktree: what it costs, removing it to free disk,
//! and checking its branch out again when the task next runs. Removing never
//! loses committed work: the branch stays, and a worktree with uncommitted
//! changes is refused. See `docs/worktree-disk.md`.

use super::*;
use bruhs_models::coordination::worktree::{RemovedWorktree, TaskWorktree, WorktreeState};

/// Removal and restore never interleave: a run that starts while its
/// worktree is being removed waits, then checks the branch out again.
static WORKTREE_OPS: Mutex<()> = Mutex::const_new(());
/// A removed folder is renamed to this prefix beside it, then deleted in the
/// background, so a large build folder never holds up the request or Git.
const STAGING_PREFIX: &str = ".bruhs-removing-";

/// Tasks using the folder `path` (forks can share their parent's).
fn sharing<'a>(data: &'a Data, path: &'a str) -> impl Iterator<Item = &'a CoordinatedTask> {
    data.tasks
        .values()
        .filter(move |task| task.working_path == path)
}

fn running(task: &CoordinatedTask) -> bool {
    task.state.active() || task.state == TaskState::Ready
}

/// `path` lies inside `root`, both as written and after resolving
/// symlinks of whatever part exists, so only worktrees bruhs created are
/// ever deleted or recreated.
fn inside(root: &Path, path: &Path) -> bool {
    use std::path::Component;
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return false;
    }
    let Ok(root) = std::fs::canonicalize(root) else {
        return false;
    };
    let mut existing = path;
    while !existing.exists() {
        match existing.parent() {
            Some(parent) => existing = parent,
            None => return false,
        }
    }
    let Ok(resolved) = std::fs::canonicalize(existing) else {
        return false;
    };
    let rest = path.strip_prefix(existing).unwrap_or(Path::new(""));
    resolved.join(rest).starts_with(&root) && resolved.join(rest) != root
}

fn plural(count: u32, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

impl CoordinationRuntime {
    /// Where `id`'s worktree stands and whether it can be removed. With
    /// `measure`, the configured meter (see `worktree_meter`) reports what
    /// deleting it frees.
    pub async fn task_worktree(
        &self,
        id: &ThreadId,
        measure: bool,
    ) -> Result<TaskWorktree, CoordinationError> {
        let root = crate::git::worktrees_root().map_err(|error| invalid(&error.to_string()))?;
        self.task_worktree_in(id, measure, &root).await
    }

    pub(super) async fn task_worktree_in(
        &self,
        id: &ThreadId,
        measure: bool,
        root: &Path,
    ) -> Result<TaskWorktree, CoordinationError> {
        let task = self.task(id).await?;
        let mut view = TaskWorktree {
            state: WorktreeState::Present,
            path: task.working_path.clone(),
            branch: None,
            changed_files: 0,
            frees: None,
            measure_error: None,
            blocked: None,
        };
        if task.working_path == task.project_path {
            view.state = WorktreeState::Shared;
            view.blocked = Some("This task works in the project folder.".into());
            return Ok(view);
        }
        if let Some(removed) = &task.removed_worktree {
            view.state = WorktreeState::Removed;
            view.branch = Some(removed.branch.clone());
            view.blocked =
                Some("Already removed. The task's next run checks its branch out again.".into());
            return Ok(view);
        }
        let path = Path::new(&task.working_path);
        let project = Path::new(&task.project_path);
        let listed = crate::git::worktree_branch(project, path)
            .await
            .ok()
            .flatten();
        if path.exists() {
            match crate::git::status(path).await {
                Ok(status) => {
                    view.branch = status.branch;
                    view.changed_files = u32::try_from(status.dirty.len()).unwrap_or(u32::MAX);
                }
                Err(error) => {
                    view.blocked = Some(format!("Git cannot read this worktree: {error}"));
                }
            }
        } else {
            view.state = WorktreeState::Missing;
            view.branch = listed.clone().flatten();
        }
        if view.blocked.is_none() {
            view.blocked = self
                .refusal(&task, root, listed.as_ref(), view.changed_files)
                .await;
        }
        if view.blocked.is_none() {
            view.blocked = nested_checkout(project, path).await;
        }
        if measure && view.state == WorktreeState::Present {
            match crate::worktree_meter::load(crate::worktree_meter::config_path().as_deref()).await
            {
                Ok(config) => match crate::worktree_meter::measure(&config, path).await {
                    Ok(frees) => view.frees = frees,
                    Err(error) => view.measure_error = Some(error),
                },
                Err(error) => view.measure_error = Some(error),
            }
        }
        Ok(view)
    }

    /// Why `task`'s worktree cannot be removed right now, apart from nested
    /// checkouts. `listed` is Git's entry for it (`Some(branch)` when listed).
    async fn refusal(
        &self,
        task: &CoordinatedTask,
        root: &Path,
        listed: Option<&Option<String>>,
        changed: u32,
    ) -> Option<String> {
        if !inside(root, Path::new(&task.working_path)) {
            return Some("Only worktrees bruhs created can be removed here.".into());
        }
        let (sharers, queued, waking) = {
            let data = self.0.data.lock().await;
            let sharers: Vec<ThreadId> = sharing(&data, &task.working_path)
                .map(|task| task.id.clone())
                .collect();
            let queued = sharing(&data, &task.working_path).any(running);
            let waking = data
                .wake_subscriptions
                .values()
                .any(|wake| wake.enabled && sharers.contains(&wake.task_id));
            drop(data);
            (sharers, queued, waking)
        };
        // A stopped task's process can still be exiting.
        let live = {
            let cancels = self.0.cancels.lock().await;
            sharers.iter().any(|id| cancels.contains_key(id))
        };
        if queued || live {
            return Some(
                "This task, or one sharing its worktree, is still running or queued. Stop it first."
                    .into(),
            );
        }
        if waking {
            return Some("This task waits for pull request events, which read its worktree. Turn its wake off first.".into());
        }
        match listed {
            None => Some(
                "Git no longer lists this worktree, so its branch is unknown. Nothing can be removed."
                    .into(),
            ),
            Some(None) => Some(
                "No branch is checked out here, so commits could be lost. Check out a branch first."
                    .into(),
            ),
            Some(Some(_)) if changed > 0 => Some(format!(
                "{} not committed. Commit or discard them first.",
                plural(changed, "change is", "changes are")
            )),
            Some(Some(_)) => None,
        }
    }

    /// `RemoveWorktree`: delete `id`'s own worktree folder under bruhs's
    /// worktrees folder and keep its branch.
    pub(super) async fn remove_worktree(&self, id: &ThreadId) -> Result<(), CoordinationError> {
        let root = crate::git::worktrees_root().map_err(|error| invalid(&error.to_string()))?;
        self.remove_worktree_in(id, &root).await
    }

    pub(super) async fn remove_worktree_in(
        &self,
        id: &ThreadId,
        root: &Path,
    ) -> Result<(), CoordinationError> {
        let _ops = WORKTREE_OPS.lock().await;
        let task = self.task(id).await?;
        if task.working_path == task.project_path {
            return Err(invalid(
                "This task works in the project folder; it has no worktree to remove.",
            ));
        }
        if task.removed_worktree.is_some() {
            return Ok(());
        }
        let path = PathBuf::from(&task.working_path);
        let project = PathBuf::from(&task.project_path);
        let listed = crate::git::worktree_branch(&project, &path)
            .await
            .map_err(|error| conflict(&format!("Git cannot list worktrees: {error}")))?;
        let changed = if path.exists() {
            if path.join(".gitmodules").exists() {
                return Err(conflict(
                    "This worktree has submodules. Check them, then remove it with git worktree remove.",
                ));
            }
            let status = crate::git::status(&path)
                .await
                .map_err(|error| conflict(&format!("Git cannot read this worktree: {error}")))?;
            u32::try_from(status.dirty.len()).unwrap_or(u32::MAX)
        } else {
            0
        };
        if let Some(reason) = self.refusal(&task, root, listed.as_ref(), changed).await {
            return Err(conflict(&reason));
        }
        if let Some(reason) = nested_checkout(&project, &path).await {
            return Err(conflict(&reason));
        }
        let Some(Some(branch)) = listed else {
            return Err(conflict(
                "Git no longer lists this worktree. Nothing was removed.",
            ));
        };

        // Record the removal before touching anything, under the same lock
        // that admits runs, so a crash or a failed save never strands a task
        // without the branch it needs to come back.
        let working = task.working_path.clone();
        self.mark_removed(&working, Some(branch), true).await?;
        if let Err(error) = self.unregister(&project, &path).await {
            if let Err(undo) = self.mark_removed(&working, None, false).await {
                tracing::error!(%undo, "removal marker for {working} could not be cleared");
            }
            return Err(error);
        }
        Ok(())
    }

    /// Set (`Some(branch)`) or clear the removal marker on every task in
    /// `working`. With `recheck`, refuse when one of them has started.
    async fn mark_removed(
        &self,
        working: &str,
        branch: Option<String>,
        recheck: bool,
    ) -> Result<(), CoordinationError> {
        let mut data = self.0.data.lock().await;
        if recheck && sharing(&data, working).any(running) {
            return Err(conflict(
                "A task sharing this worktree started meanwhile. Nothing was removed.",
            ));
        }
        let removed = branch.map(|branch| RemovedWorktree {
            branch,
            removed_at: EpochMs::now(),
        });
        for task in data
            .tasks
            .values_mut()
            .filter(|task| task.working_path == working)
        {
            task.removed_worktree.clone_from(&removed);
        }
        self.persist(&mut data).await?;
        drop(data);
        Ok(())
    }

    /// Move the folder aside, let Git drop its record, then delete it in the
    /// background. A Git refusal moves the folder back.
    async fn unregister(&self, project: &Path, path: &Path) -> Result<(), CoordinationError> {
        if let Some(parent) = path.parent() {
            sweep_staging(parent);
        }
        if !path.exists() {
            return crate::git::remove_worktree(project, path)
                .await
                .map_err(|error| conflict(&format!("Git kept the worktree: {error}")));
        }
        let staging = staging_path(path)?;
        std::fs::rename(path, &staging)
            .map_err(|error| conflict(&format!("Cannot move the worktree aside: {error}")))?;
        if let Err(error) = crate::git::remove_worktree(project, path).await {
            if let Err(undo) = std::fs::rename(&staging, path) {
                tracing::error!(
                    "worktree {} stayed at {} after Git refused removal: {undo}",
                    path.display(),
                    staging.display()
                );
            }
            return Err(conflict(&format!("Git kept the worktree: {error}")));
        }
        delete_in_background(staging);
        Ok(())
    }

    /// Before a run: check out the branch of a removed worktree again at the
    /// task's working path. Nothing to do for a worktree that is in place.
    pub(super) async fn restore_worktree(&self, id: &ThreadId) -> Result<(), CoordinationError> {
        if self.removal_of(id).await?.is_none() {
            return Ok(());
        }
        let root = crate::git::worktrees_root().map_err(|error| invalid(&error.to_string()))?;
        self.restore_worktree_in(id, &root).await
    }

    /// The removal marker on `id` or a task sharing its folder.
    async fn removal_of(
        &self,
        id: &ThreadId,
    ) -> Result<Option<(CoordinatedTask, RemovedWorktree)>, CoordinationError> {
        let data = self.0.data.lock().await;
        let task = data.tasks.get(id).cloned().ok_or_else(|| not_found(id))?;
        let removed =
            sharing(&data, &task.working_path).find_map(|task| task.removed_worktree.clone());
        drop(data);
        Ok(removed.map(|removed| (task, removed)))
    }

    pub(super) async fn restore_worktree_in(
        &self,
        id: &ThreadId,
        root: &Path,
    ) -> Result<(), CoordinationError> {
        let _ops = WORKTREE_OPS.lock().await;
        let Some((task, removed)) = self.removal_of(id).await? else {
            return Ok(());
        };
        let path = PathBuf::from(&task.working_path);
        let project = PathBuf::from(&task.project_path);
        if !inside(root, &path) {
            return Err(invalid(
                "This task's worktree is outside bruhs's worktrees folder; it was not checked out again.",
            ));
        }
        // A removal interrupted after the folder moved aside leaves Git's
        // record of a folder that is gone; drop it before checking out again.
        if !path.exists()
            && crate::git::worktree_branch(&project, &path)
                .await
                .ok()
                .flatten()
                .is_some()
        {
            crate::git::remove_worktree(&project, &path)
                .await
                .map_err(|error| conflict(&format!("Git kept a stale worktree record: {error}")))?;
        }
        crate::git::add_worktree(&project, &removed.branch, &path, &removed.branch)
            .await
            .map_err(|error| {
                conflict(&format!(
                    "This task's worktree was removed, and checking out {} again failed: {error}",
                    removed.branch
                ))
            })?;
        self.mark_removed(&task.working_path, None, false).await
    }
}

/// Work `git status` in the worktree cannot see: another linked worktree
/// Git lists inside it (agents make these under ignored folders such as
/// `.claude/`), or a `.git` a few folders down inside ignored files.
async fn nested_checkout(project: &Path, path: &Path) -> Option<String> {
    let inner = std::fs::canonicalize(path).ok()?;
    let listed = crate::git::worktree_paths(project)
        .await
        .unwrap_or_default();
    if let Some(found) = listed.iter().find(|listed| {
        std::fs::canonicalize(listed)
            .is_ok_and(|listed| listed != inner && listed.starts_with(&inner))
    }) {
        return Some(format!(
            "Another worktree is inside this one, at {}. Remove it first so its work is not lost.",
            found.display()
        ));
    }
    let root = inner.clone();
    let found = tokio::task::spawn_blocking(move || find_git_below(&root))
        .await
        .ok()
        .flatten()?;
    Some(format!(
        "A separate Git checkout is inside this worktree, at {}. Move or remove it first so its work is not lost.",
        found.display()
    ))
}

/// Folders skipped when looking for nested checkouts: dependency and build
/// output, which hold no checkouts of their own and can be huge.
const SKIPPED_FOR_NESTED: [&str; 2] = ["node_modules", "target"];

/// The first `.git` below `root` (not root's own), at most four folders
/// down and after at most 50,000 entries.
fn find_git_below(root: &Path) -> Option<PathBuf> {
    let mut queue = std::collections::VecDeque::from([(root.to_path_buf(), 0u8)]);
    let mut seen = 0u32;
    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > 50_000 {
                return None;
            }
            let name = entry.file_name();
            if name == ".git" {
                if depth > 0 {
                    return Some(entry.path());
                }
                continue;
            }
            let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
            if is_dir && depth < 4 && !SKIPPED_FOR_NESTED.iter().any(|skip| name == *skip) {
                queue.push_back((entry.path(), depth + 1));
            }
        }
    }
    None
}

/// A fresh name beside `path` to move it to before deletion.
fn staging_path(path: &Path) -> Result<PathBuf, CoordinationError> {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(invalid("The worktree path has no parent folder."));
    };
    let stamp = EpochMs::now().as_millis();
    Ok(parent.join(format!(
        "{STAGING_PREFIX}{}-{stamp}",
        name.to_string_lossy()
    )))
}

fn delete_in_background(staging: PathBuf) {
    tokio::task::spawn_blocking(move || {
        if let Err(error) = crate::git::remove_tree(&staging) {
            tracing::warn!("could not finish deleting {}: {error}", staging.display());
        }
    });
}

/// Finish deleting folders a stopped server left half-deleted.
fn sweep_staging(parent: &Path) {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let is_staging = name.to_string_lossy().starts_with(STAGING_PREFIX);
        if is_staging && entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            delete_in_background(entry.path());
        }
    }
}

#[cfg(test)]
#[expect(
    clippy::significant_drop_tightening,
    reason = "each fixture keeps its temporary repository until the test ends"
)]
mod tests {
    use super::*;

    struct Fixture {
        _dir: tempfile::TempDir,
        root: PathBuf,
        project: PathBuf,
        runtime: CoordinationRuntime,
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn task(project: &Path, working: &Path, state: TaskState) -> CoordinatedTask {
        CoordinatedTask {
            id: ThreadId::new_v4(),
            parent_id: None,
            source_context: vec![],
            source_revision: None,
            relationship: Relationship::Root,
            title: "Worktree task".into(),
            project_path: project.to_string_lossy().into_owned(),
            working_path: working.to_string_lossy().into_owned(),
            model: "claude-sonnet-4-5".into(),
            permission_mode: PermissionMode::Plan,
            spec_id: None,
            spec_revision: None,
            state,
            messages: vec![],
            live_output: String::new(),
            error: None,
            session_id: None,
            approvals: vec![],
            created_at: EpochMs::now(),
            updated_at: EpochMs::now(),
            resume_at: None,
            auto_resume_on_limit: false,
            limit_reported: false,
            archived_at: None,
            removed_worktree: None,
        }
    }

    /// A project with one commit, and `tasks` built from its worktrees
    /// folder, opened in a runtime.
    async fn fixture(tasks: impl FnOnce(&Path, &Path) -> Vec<CoordinatedTask>) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        let root = dir.path().join("worktrees");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        git(&project, &["init", "-q", "-b", "main"]);
        std::fs::write(project.join(".gitignore"), "target/\n").unwrap();
        std::fs::write(project.join("README.md"), "hello\n").unwrap();
        git(&project, &["add", "-A"]);
        git(&project, &["commit", "-q", "-m", "init"]);
        let mut data = Data::default();
        for task in tasks(&project, &root) {
            data.tasks.insert(task.id.clone(), task);
        }
        let store = dir.path().join("tasks.json");
        std::fs::write(&store, serde_json::to_vec(&data).unwrap()).unwrap();
        let runtime = CoordinationRuntime::open(store, Arc::new(crate::AppState::new()))
            .await
            .unwrap();
        Fixture {
            _dir: dir,
            root,
            project,
            runtime,
        }
    }

    fn add_worktree(project: &Path, path: &Path, branch: &str) {
        git(
            project,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                branch,
                path.to_str().unwrap(),
                "main",
            ],
        );
    }

    /// Removing deletes the folder with its ignored build output and keeps
    /// the branch with its commit; the next run checks it out again.
    #[tokio::test]
    async fn remove_keeps_the_branch_and_restore_checks_it_out_again() {
        let mut ids = Vec::new();
        let f = fixture(|project, root| {
            let task = task(project, &root.join("p/bruhs-task"), TaskState::Completed);
            ids.push(task.id.clone());
            vec![task]
        })
        .await;
        let id = ids.remove(0);
        let path = f.root.join("p/bruhs-task");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        add_worktree(&f.project, &path, "bruhs/task");
        std::fs::write(path.join("work.txt"), "done\n").unwrap();
        git(&path, &["add", "work.txt"]);
        git(&path, &["commit", "-q", "-m", "work"]);
        std::fs::create_dir_all(path.join("target/debug")).unwrap();
        std::fs::write(path.join("target/debug/app"), "build output").unwrap();

        let view = f
            .runtime
            .task_worktree_in(&id, false, &f.root)
            .await
            .unwrap();
        assert_eq!(view.state, WorktreeState::Present);
        assert_eq!(view.branch.as_deref(), Some("bruhs/task"));
        assert!(view.removable(), "{view:?}");

        f.runtime.remove_worktree_in(&id, &f.root).await.unwrap();
        assert!(!path.exists());
        assert!(!git(&f.project, &["worktree", "list"]).contains("bruhs-task"));
        assert!(git(&f.project, &["log", "--format=%s", "bruhs/task"]).contains("work"));
        let task = f.runtime.task(&id).await.unwrap();
        assert_eq!(task.removed_worktree.as_ref().unwrap().branch, "bruhs/task");
        assert_eq!(
            f.runtime
                .task_worktree_in(&id, false, &f.root)
                .await
                .unwrap()
                .state,
            WorktreeState::Removed
        );
        // Removing again is a no-op.
        f.runtime.remove_worktree_in(&id, &f.root).await.unwrap();

        f.runtime.restore_worktree_in(&id, &f.root).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(path.join("work.txt")).unwrap(),
            "done\n"
        );
        assert_eq!(
            git(&path, &["branch", "--show-current"]).trim(),
            "bruhs/task"
        );
        assert!(f
            .runtime
            .task(&id)
            .await
            .unwrap()
            .removed_worktree
            .is_none());
        // The staged folder is deleted in the background.
        let parent = f.root.join("p");
        for _ in 0..200 {
            let left = std::fs::read_dir(&parent).unwrap().flatten().any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(STAGING_PREFIX)
            });
            if !left {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("the removed folder was never deleted");
    }

    /// Uncommitted work, a running task, a detached HEAD and folders bruhs
    /// did not create all keep the worktree in place.
    #[tokio::test]
    async fn remove_refuses_whatever_could_lose_work() {
        let mut ids = Vec::new();
        let f = fixture(|project, root| {
            let dirty = task(project, &root.join("p/dirty"), TaskState::Completed);
            let busy = task(project, &root.join("p/busy"), TaskState::Completed);
            let detached = task(project, &root.join("p/detached"), TaskState::Failed);
            let outside = task(project, &project.join("../elsewhere"), TaskState::Completed);
            let shared = task(project, project, TaskState::Completed);
            ids.extend([&dirty, &busy, &detached, &outside, &shared].map(|task| task.id.clone()));
            vec![dirty, busy, detached, outside, shared]
        })
        .await;
        let [dirty, busy, detached, outside, shared] = <[ThreadId; 5]>::try_from(ids).unwrap();
        std::fs::create_dir_all(f.root.join("p")).unwrap();
        add_worktree(&f.project, &f.root.join("p/dirty"), "bruhs/dirty");
        add_worktree(&f.project, &f.root.join("p/busy"), "bruhs/busy");
        git(
            &f.project,
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                f.root.join("p/detached").to_str().unwrap(),
                "main",
            ],
        );
        std::fs::write(f.root.join("p/dirty/new.txt"), "unsaved\n").unwrap();
        // Opening the store marks running work interrupted; start it again.
        f.runtime
            .0
            .data
            .lock()
            .await
            .tasks
            .get_mut(&busy)
            .unwrap()
            .state = TaskState::Running;

        let view = f
            .runtime
            .task_worktree_in(&dirty, false, &f.root)
            .await
            .unwrap();
        assert_eq!(view.changed_files, 1);
        assert!(!view.removable());
        for (id, needle) in [
            (&dirty, "not committed"),
            (&busy, "still running"),
            (&detached, "No branch"),
            (&outside, "Only worktrees bruhs created"),
            (&shared, "project folder"),
        ] {
            let error = f.runtime.remove_worktree_in(id, &f.root).await.unwrap_err();
            assert!(error.to_string().contains(needle), "{needle}: {error}");
        }
        assert!(f.root.join("p/dirty/new.txt").exists());
        assert!(f.root.join("p/busy").exists());
        assert!(f.root.join("p/detached").exists());
    }

    /// Forks that share a worktree are all marked, and any of them restores it.
    #[tokio::test]
    async fn sharing_tasks_follow_the_worktree() {
        let mut ids = Vec::new();
        let f = fixture(|project, root| {
            let working = root.join("p/shared");
            let parent = task(project, &working, TaskState::Completed);
            let child = task(project, &working, TaskState::Cancelled);
            ids.extend([parent.id.clone(), child.id.clone()]);
            vec![parent, child]
        })
        .await;
        let path = f.root.join("p/shared");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        add_worktree(&f.project, &path, "bruhs/shared");
        f.runtime
            .remove_worktree_in(&ids[0], &f.root)
            .await
            .unwrap();
        assert!(f
            .runtime
            .task(&ids[1])
            .await
            .unwrap()
            .removed_worktree
            .is_some());
        f.runtime
            .restore_worktree_in(&ids[1], &f.root)
            .await
            .unwrap();
        assert!(path.join("README.md").is_file());
        assert!(f
            .runtime
            .task(&ids[0])
            .await
            .unwrap()
            .removed_worktree
            .is_none());
    }

    /// Work `git status` cannot see keeps the worktree: an agent's own
    /// worktree under an ignored folder, or a nested clone.
    #[tokio::test]
    async fn nested_checkouts_and_pull_request_wakes_keep_the_worktree() {
        let mut ids = Vec::new();
        let f = fixture(|project, root| {
            let nested = task(project, &root.join("p/nested"), TaskState::Completed);
            let clone = task(project, &root.join("p/clone"), TaskState::Completed);
            let waking = task(project, &root.join("p/waking"), TaskState::Completed);
            ids.extend([&nested, &clone, &waking].map(|task| task.id.clone()));
            vec![nested, clone, waking]
        })
        .await;
        let [nested, clone, waking] = <[ThreadId; 3]>::try_from(ids).unwrap();
        std::fs::create_dir_all(f.root.join("p")).unwrap();
        for (name, branch) in [
            ("nested", "bruhs/nested"),
            ("clone", "bruhs/clone"),
            ("waking", "bruhs/waking"),
        ] {
            add_worktree(&f.project, &f.root.join("p").join(name), branch);
            std::fs::write(
                f.root.join("p").join(name).join(".gitignore"),
                ".claude/\nvendor/\n",
            )
            .unwrap();
            git(&f.root.join("p").join(name), &["add", ".gitignore"]);
            git(
                &f.root.join("p").join(name),
                &["commit", "-q", "-m", "ignore"],
            );
        }
        let inner = f.root.join("p/nested/.claude/worktrees/n");
        add_worktree(&f.project, &inner, "nested-agent");
        std::fs::write(inner.join("work.txt"), "uncommitted\n").unwrap();
        let cloned = f.root.join("p/clone/vendor/lib");
        std::fs::create_dir_all(&cloned).unwrap();
        git(&cloned, &["init", "-q"]);
        {
            let mut data = f.runtime.0.data.lock().await;
            let wake = WakeSubscription {
                id: ThreadId::new_v4(),
                task_id: waking.clone(),
                watch_id: "watch".into(),
                kinds: vec![WakeKind::Review],
                project_path: f.project.to_string_lossy().into_owned(),
                model: "claude-sonnet-4-5".into(),
                permission_mode: PermissionMode::Plan,
                enabled: true,
                approved_from_ms: 0,
            };
            data.wake_subscriptions.insert(wake.id.clone(), wake);
        }
        for (id, needle) in [
            (&nested, "Another worktree is inside"),
            (&clone, "separate Git checkout"),
            (&waking, "pull request events"),
        ] {
            let view = f
                .runtime
                .task_worktree_in(id, false, &f.root)
                .await
                .unwrap();
            assert!(!view.removable(), "{needle}: {view:?}");
            let error = f.runtime.remove_worktree_in(id, &f.root).await.unwrap_err();
            assert!(error.to_string().contains(needle), "{needle}: {error}");
        }
        assert!(inner.join("work.txt").exists());
        assert!(cloned.join(".git").exists());
    }

    /// A removal cut short after the folder moved aside (a crash, or a full
    /// disk) still comes back: the marker was saved first, and restore drops
    /// Git's stale record before checking the branch out again.
    #[tokio::test]
    async fn an_interrupted_removal_restores() {
        let mut ids = Vec::new();
        let f = fixture(|project, root| {
            let task = task(project, &root.join("p/cut"), TaskState::Completed);
            ids.push(task.id.clone());
            vec![task]
        })
        .await;
        let id = ids.remove(0);
        let path = f.root.join("p/cut");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        add_worktree(&f.project, &path, "bruhs/cut");
        f.runtime
            .mark_removed(&path.to_string_lossy(), Some("bruhs/cut".into()), true)
            .await
            .unwrap();
        std::fs::rename(&path, f.root.join("p/.bruhs-removing-cut-1")).unwrap();
        assert!(git(&f.project, &["worktree", "list"]).contains("cut"));

        f.runtime.restore_worktree_in(&id, &f.root).await.unwrap();
        assert_eq!(
            git(&path, &["branch", "--show-current"]).trim(),
            "bruhs/cut"
        );
        assert!(f
            .runtime
            .task(&id)
            .await
            .unwrap()
            .removed_worktree
            .is_none());
    }
}
