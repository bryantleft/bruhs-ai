//! Every worktree bruhs created on this machine, from chat, Workbench tasks
//! and issue work: listing them, removing one without losing work, and
//! checking its branch out again when something next runs there. See
//! `docs/worktree-disk.md`.
//!
//! Removal keeps the branch and refuses anything that could lose work Git
//! cannot get back: uncommitted or untracked changes, a detached HEAD,
//! submodules, another checkout nested inside (which `git status` cannot
//! see), a program working in the folder, or a pull-request watch reading
//! it. A record of each removal is saved before anything moves, so a turn
//! in that folder, in either app, checks the branch out again.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, OnceLock};

use bruhs_models::coordination::worktree::{ManagedWorktree, RemovedRecord, WorktreeState};
use bruhs_models::EpochMs;
use tokio::io::AsyncReadExt;

use crate::coordination::LinkSource;

/// A removed folder is renamed to this prefix beside it, then deleted in the
/// background, so a large build folder never holds up the request or Git.
pub const STAGING_PREFIX: &str = ".bruhs-removing-";
/// Removal records, in the worktrees folder (shared by server and desktop).
const RECORDS: &str = ".bruhs-removed.json";
/// Held while removing or restoring, across processes.
const LOCK: &str = ".bruhs-ops.lock";
/// Bytes of the records file read.
const RECORDS_LIMIT: u64 = 1024 * 1024;

/// Pull-request watches live in the delivery service; the host registers
/// it so removal can refuse a folder a watch still reads.
static WATCH_SOURCE: OnceLock<Arc<dyn LinkSource>> = OnceLock::new();

/// Register where pull-request watches come from. The first call wins.
pub fn register_watch_source(source: Arc<dyn LinkSource>) {
    if WATCH_SOURCE.set(source).is_err() {
        tracing::debug!("worktree watch source was already registered");
    }
}

/// bruhs's worktrees folder.
pub fn root() -> Result<PathBuf, String> {
    crate::git::worktrees_root().map_err(|error| error.to_string())
}

/// `path` with its nearest existing ancestor's symlinks resolved, so a
/// folder that is gone still compares equal to its resolved form.
pub fn resolve(path: &Path) -> PathBuf {
    let mut existing = path;
    loop {
        if let Ok(resolved) = std::fs::canonicalize(existing) {
            // `join("")` would add a trailing separator.
            return match path.strip_prefix(existing) {
                Ok(rest) if !rest.as_os_str().is_empty() => resolved.join(rest),
                _ => resolved,
            };
        }
        match existing.parent() {
            Some(parent) => existing = parent,
            None => return path.to_path_buf(),
        }
    }
}

/// `path` lies strictly inside `root`, as written and with symlinks of
/// whatever part exists resolved, so only folders bruhs created are ever
/// deleted or recreated.
pub fn inside(root: &Path, path: &Path) -> bool {
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
    let resolved = resolve(path);
    resolved.starts_with(&root) && resolved != root
}

/// How long a removal or restore waits for another one to finish.
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(120);

/// The cross-process lock for removals and restores in `root`. Gives up
/// after [`LOCK_WAIT`] rather than blocking a turn indefinitely. It is not
/// reentrant: never take it while holding it.
pub async fn lock(root: &Path) -> Result<std::fs::File, String> {
    std::fs::create_dir_all(root).map_err(|error| error.to_string())?;
    let path = root.join(LOCK);
    tokio::task::spawn_blocking(move || {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|error| format!("Cannot open the worktrees lock: {error}"))?;
        let deadline = std::time::Instant::now() + LOCK_WAIT;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(std::fs::TryLockError::WouldBlock) => {
                    return Err(
                        "Another worktree removal or restore is still running. Try again shortly."
                            .into(),
                    )
                }
                Err(std::fs::TryLockError::Error(error)) => {
                    return Err(format!("Cannot lock the worktrees folder: {error}"))
                }
            }
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

/// The removal records. A missing file is empty; one that cannot be read
/// or parsed is an error, never silently replaced: an unparsable file is set
/// aside (`.bruhs-removed.json.corrupt-<ms>`) so its entries can be
/// recovered by hand, and the next call starts afresh.
async fn read_records(root: &Path) -> Result<BTreeMap<String, RemovedRecord>, String> {
    let path = root.join(RECORDS);
    let file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(format!("Cannot read {}: {error}", path.display())),
    };
    let mut bytes = Vec::new();
    file.take(RECORDS_LIMIT + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
    let parsed = if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > RECORDS_LIMIT {
        Err("it is larger than 1 MiB".to_string())
    } else {
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())
    };
    parsed.map_err(|why| {
        let aside = root.join(format!("{RECORDS}.corrupt-{}", EpochMs::now().as_millis()));
        match std::fs::rename(&path, &aside) {
            Ok(()) => format!(
                "The removed-worktree records could not be read ({why}) and were set aside as {}. Removed worktrees they listed no longer come back on their own; their branches remain.",
                aside.display()
            ),
            Err(error) => format!("The removed-worktree records could not be read ({why}) or set aside: {error}"),
        }
    })
}

fn write_records(root: &Path, records: &BTreeMap<String, RemovedRecord>) -> Result<(), String> {
    use std::io::Write as _;
    let bytes = serde_json::to_vec_pretty(records).map_err(|error| error.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(root).map_err(|error| error.to_string())?;
    file.write_all(&bytes)
        .and_then(|()| file.as_file().sync_all())
        .map_err(|error| error.to_string())?;
    file.persist(root.join(RECORDS))
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// The key a folder's record is saved under.
fn key(path: &Path) -> String {
    resolve(path).to_string_lossy().into_owned()
}

/// The removal record for `path`, if bruhs removed it.
pub async fn record(root: &Path, path: &Path) -> Result<Option<RemovedRecord>, String> {
    Ok(read_records(root).await?.remove(&key(path)))
}

/// The removed worktree holding `folder`: `folder` itself or its nearest
/// ancestor inside `root`, with its record.
async fn removed_ancestor(
    root: &Path,
    folder: &Path,
) -> Result<Option<(PathBuf, RemovedRecord)>, String> {
    let mut records = read_records(root).await?;
    let root = resolve(root);
    let mut candidate = resolve(folder);
    while candidate.starts_with(&root) && candidate != root {
        if let Some(record) = records.remove(&candidate.to_string_lossy().into_owned()) {
            return Ok(Some((candidate, record)));
        }
        if !candidate.pop() {
            break;
        }
    }
    Ok(None)
}

/// Why `path` (a worktree of `project`) cannot be removed, or the branch it
/// keeps when it can. Checks only the folder; callers add their own reasons
/// (a running task, a wake).
pub async fn check(root: &Path, project: &Path, path: &Path) -> Result<String, String> {
    let programs = programs().await?;
    check_with(root, project, path, &programs).await
}

/// [`check`] with the programs' working directories read once by the caller.
async fn check_with(
    root: &Path,
    project: &Path,
    path: &Path,
    programs: &[Program],
) -> Result<String, String> {
    if !inside(root, path) {
        return Err("Only worktrees bruhs created can be removed here.".into());
    }
    if resolve(path) == resolve(project) || (path.exists() && !path.join(".git").is_file()) {
        return Err("This folder is not a linked worktree, so it is not removed here.".into());
    }
    let branch = match crate::git::worktree_branch(project, path).await {
        Ok(Some(Some(branch))) => branch,
        Ok(Some(None)) => return Err(
            "No branch is checked out here, so commits could be lost. Check out a branch first."
                .into(),
        ),
        Ok(None) => return Err(
            "Git no longer lists this worktree, so its branch is unknown. Nothing can be removed."
                .into(),
        ),
        Err(error) => return Err(format!("Git cannot list worktrees: {error}")),
    };
    if path.exists() {
        if path.join(".gitmodules").exists() {
            return Err(
                "This worktree has submodules. Check them, then remove it with git worktree remove."
                    .into(),
            );
        }
        let status = crate::git::status(path)
            .await
            .map_err(|error| format!("Git cannot read this worktree: {error}"))?;
        if !status.dirty.is_empty() {
            let count = status.dirty.len();
            return Err(format!(
                "{count} {} not committed. Commit or discard them first.",
                if count == 1 {
                    "change is"
                } else {
                    "changes are"
                }
            ));
        }
        if let Some(reason) = nested_checkout(project, path).await {
            return Err(reason);
        }
        if let Some(program) = program_in(programs, path) {
            return Err(format!(
                "{program} is working in this folder. Close it first."
            ));
        }
    }
    if let Some(source) = WATCH_SOURCE.get() {
        let inner = resolve(path);
        let watched = source.watches().await.into_iter().any(|watch| {
            watch.enabled && resolve(Path::new(&watch.request.path)).starts_with(&inner)
        });
        if watched {
            return Err(
                "A pull request watch reads this worktree. Turn it off in Delivery first.".into(),
            );
        }
    }
    Ok(branch)
}

/// Remove the checked worktree at `path`: save its record, move the folder
/// aside, let Git drop its record, then delete the folder in the
/// background. A Git refusal moves the folder back and drops the record.
/// Hold [`lock`] around [`check`] and this.
pub async fn unregister(
    root: &Path,
    project: &Path,
    path: &Path,
    branch: &str,
) -> Result<(), String> {
    let mut records = read_records(root).await?;
    records.insert(
        key(path),
        RemovedRecord {
            project: project.to_string_lossy().into_owned(),
            branch: branch.to_string(),
            removed_at: EpochMs::now(),
        },
    );
    write_records(root, &records)?;
    let result = move_and_unregister(project, path).await;
    if result.is_err() {
        records.remove(&key(path));
        if let Err(error) = write_records(root, &records) {
            tracing::error!("removal record for {} stayed: {error}", path.display());
        }
    }
    result
}

async fn move_and_unregister(project: &Path, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        sweep_staging(parent);
    }
    if !path.exists() {
        return crate::git::remove_worktree(project, path)
            .await
            .map_err(|error| format!("Git kept the worktree: {error}"));
    }
    let staging = staging_path(path)?;
    std::fs::rename(path, &staging)
        .map_err(|error| format!("Cannot move the worktree aside: {error}"))?;
    // A turn could have started after the check. A program working in the
    // folder moves with it; anything starting now finds the folder gone and
    // the next turn checks it out again. Git does not re-check a moved folder.
    let late = match programs().await {
        Ok(programs) => program_in(&programs, &staging)
            .map(|program| format!("{program} started working in this folder")),
        Err(error) => Some(error),
    };
    if let Some(reason) = late {
        if let Err(undo) = std::fs::rename(&staging, path) {
            tracing::error!(
                "worktree {} stayed at {}: {undo}",
                path.display(),
                staging.display()
            );
        }
        return Err(format!("{reason}. Nothing was removed."));
    }
    if let Err(error) = crate::git::remove_worktree(project, path).await {
        if let Err(undo) = std::fs::rename(&staging, path) {
            tracing::error!(
                "worktree {} stayed at {} after Git refused removal: {undo}",
                path.display(),
                staging.display()
            );
        }
        return Err(format!("Git kept the worktree: {error}"));
    }
    delete_in_background(staging);
    Ok(())
}

/// Check `branch` out again at `path`, a removed worktree of `project`, and
/// drop its record. A Git record left by an interrupted removal is dropped
/// first. Hold [`lock`].
pub async fn restore_branch(
    root: &Path,
    project: &Path,
    path: &Path,
    branch: &str,
    base: Option<&str>,
) -> Result<(), String> {
    if !inside(root, path) {
        return Err(
            "This worktree is outside bruhs's worktrees folder; it was not checked out again."
                .into(),
        );
    }
    if !path.exists()
        && crate::git::worktree_branch(project, path)
            .await
            .ok()
            .flatten()
            .is_some()
    {
        crate::git::remove_worktree(project, path)
            .await
            .map_err(|error| format!("Git kept a stale worktree record: {error}"))?;
    }
    crate::git::add_worktree(project, base.unwrap_or(branch), path, branch)
        .await
        .map_err(|error| match error {
            crate::git::GitError::UnknownBranch(_) if base.is_none() => format!(
                "This worktree was removed, and its branch {branch} no longer exists. Start it again from another branch in Workbench, Worktrees."
            ),
            error => format!("Checking out {branch} again failed: {error}"),
        })?;
    forget(root, path).await
}

/// Drop the removal record for `path` without checking anything out.
pub async fn forget(root: &Path, path: &Path) -> Result<(), String> {
    let mut records = read_records(root).await?;
    if records.remove(&key(path)).is_some() {
        write_records(root, &records)?;
    }
    Ok(())
}

/// Before a turn runs in `folder`: when bruhs removed that worktree (or the
/// worktree holding `folder`), check its branch out again. A folder that
/// exists, or one bruhs never removed, is left alone, so this costs one
/// `stat` on every other turn.
pub async fn restore_for_turn(folder: &Path) -> Result<(), String> {
    if folder.exists() {
        return Ok(());
    }
    let Ok(root) = root() else {
        return Ok(());
    };
    restore_for_turn_in(&root, folder).await
}

pub(crate) async fn restore_for_turn_in(root: &Path, folder: &Path) -> Result<(), String> {
    if folder.exists() || !inside(root, folder) {
        return Ok(());
    }
    let _lock = lock(root).await?;
    let Some((worktree, record)) = removed_ancestor(root, folder).await? else {
        return Ok(());
    };
    restore_branch(
        root,
        Path::new(&record.project),
        &worktree,
        &record.branch,
        None,
    )
    .await
}

/// Every worktree in `root` (two levels: project, then worktree), plus the
/// ones bruhs removed. Task ownership is filled in by the caller.
pub async fn list(root: &Path) -> Vec<ManagedWorktree> {
    let mut rows = Vec::new();
    // One scan of working directories for every row.
    let programs = programs().await;
    for folder in candidate_folders(root) {
        rows.push(describe(root, &folder, &programs).await);
    }
    let records = read_records(root).await.unwrap_or_else(|error| {
        tracing::warn!("{error}");
        BTreeMap::new()
    });
    for (path, record) in records {
        let branch_missing = !crate::git::branch_exists(Path::new(&record.project), &record.branch)
            .await
            .unwrap_or(true);
        rows.push(ManagedWorktree {
            path,
            project: Some(record.project),
            state: WorktreeState::Removed,
            branch: Some(record.branch),
            changed_files: 0,
            task: None,
            updated_at: Some(record.removed_at),
            branch_missing,
            blocked: None,
        });
    }
    rows.sort_by(|a, b| a.path.cmp(&b.path));
    rows
}

fn candidate_folders(root: &Path) -> Vec<PathBuf> {
    let mut folders = Vec::new();
    let Ok(projects) = std::fs::read_dir(root) else {
        return folders;
    };
    for project in projects.flatten() {
        if !project.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let Ok(worktrees) = std::fs::read_dir(project.path()) else {
            continue;
        };
        for worktree in worktrees.flatten() {
            let name = worktree.file_name();
            let hidden = name.to_string_lossy().starts_with('.');
            if !hidden
                && worktree.file_type().is_ok_and(|kind| kind.is_dir())
                && worktree.path().join(".git").is_file()
            {
                folders.push(worktree.path());
            }
        }
    }
    folders
}

async fn describe(
    root: &Path,
    folder: &Path,
    programs: &Result<Vec<Program>, String>,
) -> ManagedWorktree {
    let mut row = ManagedWorktree {
        path: folder.to_string_lossy().into_owned(),
        project: crate::git::main_checkout(folder)
            .await
            .map(|path| path.to_string_lossy().into_owned()),
        state: WorktreeState::Present,
        branch: None,
        changed_files: 0,
        task: None,
        updated_at: std::fs::metadata(folder)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|since| i64::try_from(since.as_millis()).ok())
            .and_then(|ms| EpochMs::try_from(ms).ok()),
        branch_missing: false,
        blocked: None,
    };
    match crate::git::status(folder).await {
        Ok(status) => {
            row.branch = status.branch;
            row.changed_files = u32::try_from(status.dirty.len()).unwrap_or(u32::MAX);
        }
        Err(error) => row.blocked = Some(format!("Git cannot read this worktree: {error}")),
    }
    if row.blocked.is_none() {
        row.blocked = match &row.project {
            Some(project) => match programs {
                Ok(programs) => check_with(root, Path::new(project), folder, programs)
                    .await
                    .err(),
                Err(error) => Some(error.clone()),
            },
            None => Some("Git cannot find the checkout this worktree belongs to.".into()),
        };
    }
    row
}

/// `(pid, name, working directory)` of one of this user's processes.
type Program = (u32, String, PathBuf);

/// Every program's working directory, read once.
async fn programs() -> Result<Vec<Program>, String> {
    tokio::task::spawn_blocking(working_directories)
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| format!("Cannot check which programs use this folder: {error}"))
}

/// A program (other than this one) working in `path` or inside it, as
/// `name (pid)`: an agent CLI, a shell, a dev server.
fn program_in(programs: &[Program], path: &Path) -> Option<String> {
    let inner = resolve(path);
    programs
        .iter()
        .find(|(pid, _, cwd)| *pid != std::process::id() && resolve(cwd).starts_with(&inner))
        .map(|(pid, name, _)| format!("{name} ({pid})"))
}

/// `(pid, name, working directory)` of this user's processes.
#[cfg(target_os = "linux")]
fn working_directories() -> std::io::Result<Vec<Program>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir("/proc")?.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        // Other users' processes are unreadable; they cannot write here anyway.
        let Ok(cwd) = std::fs::read_link(entry.path().join("cwd")) else {
            continue;
        };
        let name = std::fs::read_to_string(entry.path().join("comm")).unwrap_or_default();
        out.push((pid, name.trim().to_string(), cwd));
    }
    Ok(out)
}

/// How long `lsof` may take before the check gives up (and refuses).
#[cfg(not(target_os = "linux"))]
const LSOF_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

#[cfg(not(target_os = "linux"))]
fn working_directories() -> std::io::Result<Vec<Program>> {
    use std::io::Read as _;
    if cfg!(windows) {
        return Ok(Vec::new());
    }
    // -b avoids kernel calls that block on network mounts; -w hides warnings.
    let mut child = std::process::Command::new("lsof")
        .args(["-nP", "-b", "-w", "-d", "cwd", "-F", "pcn"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("lsof has no output"))?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let deadline = std::time::Instant::now() + LSOF_TIMEOUT;
    while child.try_wait()?.is_none() {
        if std::time::Instant::now() >= deadline {
            child.kill()?;
            child.wait()?;
            return Err(std::io::Error::other("lsof took too long"));
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let bytes = reader
        .join()
        .map_err(|_| std::io::Error::other("lsof output was lost"))??;
    Ok(parse_lsof(&bytes))
}

/// `lsof -F pcn` records: `p<pid>`, `c<command>`, `n<path>`.
#[cfg(not(target_os = "linux"))]
fn parse_lsof(stdout: &[u8]) -> Vec<Program> {
    let mut out = Vec::new();
    let (mut pid, mut name) = (None, String::new());
    for line in String::from_utf8_lossy(stdout).lines() {
        let (tag, value) = line.split_at(line.len().min(1));
        match tag {
            "p" => {
                pid = value.parse().ok();
                name.clear();
            }
            "c" => name = value.to_string(),
            "n" => {
                if let Some(pid) = pid {
                    out.push((pid, name.clone(), PathBuf::from(value)));
                }
            }
            _ => {}
        }
    }
    out
}

/// Work `git status` in the worktree cannot see: another linked worktree
/// Git lists inside it (agents make these under ignored folders such as
/// `.claude/`), or a `.git` a few folders down inside ignored files.
pub(crate) async fn nested_checkout(project: &Path, path: &Path) -> Option<String> {
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
        .unwrap_or(Err(TooManyEntries))
        .map_err(|TooManyEntries| {
            "This worktree holds too many files to check for nested checkouts. Remove it with git worktree remove after checking it yourself.".to_string()
        });
    match found {
        Ok(None) => None,
        Ok(Some(found)) => Some(format!(
            "A separate Git checkout is inside this worktree, at {}. Move or remove it first so its work is not lost.",
            found.display()
        )),
        Err(reason) => Some(reason),
    }
}

/// Folders skipped when looking for nested checkouts: dependency, build and
/// cache folders that tools manage, which hold no checkouts of their own
/// and can be huge.
const SKIPPED_FOR_NESTED: [&str; 9] = [
    "node_modules",
    "target",
    ".venv",
    "venv",
    "__pycache__",
    ".next",
    ".turbo",
    ".gradle",
    ".pnpm-store",
];
/// Folders deeper than this are not searched.
const NESTED_DEPTH: u8 = 8;
/// Entries read before the search gives up and refuses removal.
const NESTED_ENTRIES: u32 = 300_000;

/// The search for nested checkouts read too much to finish.
#[derive(Debug)]
struct TooManyEntries;

/// The first `.git` below `root` (not root's own), at most
/// [`NESTED_DEPTH`] folders down. Refuses rather than guessing after
/// [`NESTED_ENTRIES`] entries.
fn find_git_below(root: &Path) -> Result<Option<PathBuf>, TooManyEntries> {
    let mut queue = std::collections::VecDeque::from([(root.to_path_buf(), 0u8)]);
    let mut seen = 0u32;
    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > NESTED_ENTRIES {
                return Err(TooManyEntries);
            }
            let name = entry.file_name();
            if name == ".git" {
                if depth > 0 {
                    return Ok(Some(entry.path()));
                }
                continue;
            }
            let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
            if is_dir
                && depth < NESTED_DEPTH
                && !SKIPPED_FOR_NESTED.iter().any(|skip| name == *skip)
            {
                queue.push_back((entry.path(), depth + 1));
            }
        }
    }
    Ok(None)
}

/// A fresh name beside `path` to move it to before deletion.
fn staging_path(path: &Path) -> Result<PathBuf, String> {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err("The worktree path has no parent folder.".into());
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

/// Finish deleting folders a stopped host left half-deleted.
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
mod tests;
