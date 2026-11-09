//! Git working-tree queries shared by bruhs-server (HTTP routes) and
//! bruhs-desktop (direct calls): status, unified diff, branch list, checkout,
//! adding a worktree, and checkpointing that worktree for Edit from here.
//!
//! Callers resolve and confine the repository path themselves; every
//! function here trusts `dir`. Every `git` child is killed when its future is
//! dropped and has a deadline ([`GIT_TIMEOUT`]). Read-only commands run with
//! `GIT_OPTIONAL_LOCKS=0` so polling never contends for the user's index lock.

use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

use bruhs_models::logic::git::{parse_porcelain, BranchName, BranchNameError, GitStatus};
use bruhs_models::logic::text::truncate_utf8;

/// Working-tree diffs beyond this size are cut so UIs stay responsive.
pub const DIFF_LIMIT_BYTES: usize = 2 * 1024 * 1024;
/// At most this many untracked files are rendered as additions.
pub const UNTRACKED_LIMIT: usize = 50;
/// Deadline for one `git` invocation.
pub const GIT_TIMEOUT: Duration = Duration::from_secs(30);
/// Bytes of stderr kept for error messages.
const STDERR_LIMIT: usize = 16 * 1024;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GitError {
    /// `git` could not be started (not installed, bad cwd) or its pipes
    /// failed.
    #[error("git could not run: {0}")]
    Spawn(#[from] std::io::Error),
    /// `git` ran and exited non-zero; carries trimmed stderr.
    #[error("git failed: {0}")]
    Failed(String),
    /// `git` did not finish within [`GIT_TIMEOUT`]; it was killed.
    #[error("git timed out after {}s", GIT_TIMEOUT.as_secs())]
    TimedOut,
    /// The name is not a branch name git accepts.
    #[error("invalid branch name: {0}")]
    InvalidBranch(#[from] BranchNameError),
    /// The name is valid but no such local branch exists.
    #[error("no local branch named {0:?}")]
    UnknownBranch(String),
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct GitDiff {
    pub diff: String,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct Branches {
    pub current: Option<String>,
    #[serde(default)]
    pub branches: Vec<String>,
}

/// Branch list from `git branch --format=%(refname:short)` output, trimmed
/// and with the current branch moved to the front when present.
pub fn parse_branch_list(stdout: &str, current: Option<&str>) -> Vec<String> {
    let mut branches: Vec<String> = stdout
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if let Some(cur) = current {
        if let Some(pos) = branches.iter().position(|b| b == cur) {
            let b = branches.remove(pos);
            branches.insert(0, b);
        }
    }
    branches
}

/// Output of one `git` run.
struct GitOutput {
    stdout: Vec<u8>,
    /// `None` when the child was killed because stdout hit the cap.
    status: Option<ExitStatus>,
    stderr: String,
}

impl GitOutput {
    fn success(&self) -> bool {
        self.status.is_some_and(|s| s.success())
    }

    /// Stdout hit the cap and the child was stopped.
    fn capped(&self) -> bool {
        self.status.is_none()
    }

    fn stdout_lossy(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    fn failure(self) -> GitError {
        GitError::Failed(self.stderr.trim().to_string())
    }
}

/// One `git` invocation: `args` in `dir`, with extra environment.
struct GitCommand<'a> {
    dir: &'a Path,
    args: &'a [&'a str],
    env: Vec<(&'static str, std::ffi::OsString)>,
    /// Stop reading (and kill git) after this many stdout bytes.
    stdout_cap: Option<usize>,
    /// Bytes fed to git's stdin (otherwise stdin is null).
    stdin: Option<Vec<u8>>,
}

impl<'a> GitCommand<'a> {
    fn new(dir: &'a Path, args: &'a [&'a str]) -> Self {
        Self {
            dir,
            args,
            env: vec![("GIT_OPTIONAL_LOCKS", "0".into())],
            stdout_cap: None,
            stdin: None,
        }
    }

    fn stdin(mut self, bytes: Vec<u8>) -> Self {
        self.stdin = Some(bytes);
        self
    }

    fn env(mut self, key: &'static str, value: impl Into<std::ffi::OsString>) -> Self {
        self.env.push((key, value.into()));
        self
    }

    fn cap_stdout(mut self, limit: usize) -> Self {
        self.stdout_cap = Some(limit);
        self
    }

    async fn run(self) -> Result<GitOutput, GitError> {
        let mut command = Command::new("git");
        command
            .args(self.args)
            .current_dir(self.dir)
            .stdin(if self.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for (key, value) in &self.env {
            command.env(key, value);
        }
        let mut child = command.spawn()?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let stdin = child.stdin.take().zip(self.stdin);
        let cap = self.stdout_cap;
        let work = async move {
            // Stdin is fed on its own task so git can never block on a full
            // stdout pipe while we are still writing its input.
            let stdin_task = stdin.map(|(mut pipe, bytes)| {
                tokio::spawn(async move {
                    use tokio::io::AsyncWriteExt;
                    pipe.write_all(&bytes).await?;
                    pipe.shutdown().await
                })
            });
            // Stderr drains on its own task so a chatty git can never block
            // on a full stderr pipe while we read stdout (or vice versa).
            let stderr_task = tokio::spawn(read_bounded(stderr, STDERR_LIMIT));
            let (stdout, hit_cap) = match cap {
                Some(limit) => {
                    let mut buf = Vec::new();
                    if let Some(out) = stdout {
                        // One byte past the limit tells "exactly at the limit"
                        // apart from "more to come".
                        out.take(limit as u64 + 1).read_to_end(&mut buf).await?;
                    }
                    let hit = buf.len() > limit;
                    (buf, hit)
                }
                None => (read_bounded(stdout, usize::MAX).await?, false),
            };
            let status = if hit_cap {
                // Enough output: stop git instead of buffering the rest. If
                // the kill fails git has already exited, and its stdout pipe
                // is closed anyway, so waiting still returns.
                if let Err(e) = child.start_kill() {
                    tracing::debug!("git already exited before kill: {e}");
                }
                child.wait().await?;
                None
            } else {
                Some(child.wait().await?)
            };
            if let Some(task) = stdin_task {
                match task.await {
                    Ok(Ok(())) => {}
                    // git stopped reading early (it exited or was killed
                    // at the cap); its exit status says what happened.
                    Ok(Err(e)) => tracing::debug!("git stdin closed early: {e}"),
                    Err(join) => return Err(GitError::Spawn(std::io::Error::other(join))),
                }
            }
            let stderr = match stderr_task.await {
                Ok(Ok(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
                Ok(Err(e)) => return Err(GitError::Spawn(e)),
                Err(join) => return Err(GitError::Spawn(std::io::Error::other(join))),
            };
            Ok(GitOutput {
                stdout,
                status,
                stderr,
            })
        };
        // On timeout the future (and with it the child) is dropped, and
        // `kill_on_drop` kills git.
        tokio::time::timeout(GIT_TIMEOUT, work)
            .await
            .map_err(|_| GitError::TimedOut)?
    }
}

/// Read `reader` to the end, keeping at most `keep` bytes (the rest is
/// drained so the writer never blocks).
async fn read_bounded<R: AsyncRead + Unpin>(
    reader: Option<R>,
    keep: usize,
) -> std::io::Result<Vec<u8>> {
    let Some(mut reader) = reader else {
        return Ok(Vec::new());
    };
    let mut kept = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            return Ok(kept);
        }
        let room = keep.saturating_sub(kept.len());
        kept.extend_from_slice(&chunk[..n.min(room)]);
    }
}

async fn git(dir: &Path, args: &[&str]) -> Result<GitOutput, GitError> {
    GitCommand::new(dir, args).run().await
}

/// Every tracked file of the repository at `dir`, relative to `dir`
/// (`git ls-files -z`, so unusual names arrive unquoted).
pub async fn tracked_files(dir: &Path) -> Result<Vec<String>, GitError> {
    let out = git(dir, &["ls-files", "-z"]).await?;
    if !out.success() {
        return Err(out.failure());
    }
    Ok(out
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).into_owned())
        .collect())
}

/// The files changed by the last `commits` commits, newest first, one entry
/// per change (`git log --name-only`). Empty for a repository without
/// commits.
pub async fn recently_changed_files(dir: &Path, commits: u32) -> Result<Vec<String>, GitError> {
    let count = commits.to_string();
    let out = git(
        dir,
        &[
            "-c",
            "core.quotePath=false",
            "log",
            "--name-only",
            "--pretty=format:",
            "-n",
            &count,
        ],
    )
    .await?;
    if !out.success() {
        if out.stderr.contains("does not have any commits") {
            return Ok(Vec::new());
        }
        return Err(out.failure());
    }
    Ok(out
        .stdout_lossy()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(String::from)
        .collect())
}

/// Branch / ahead-behind / dirty files for the working tree at `dir`.
pub async fn status(dir: &Path) -> Result<GitStatus, GitError> {
    let out = git(
        dir,
        &[
            "status",
            "--porcelain=v1",
            "-b",
            "-z",
            "--untracked-files=normal",
        ],
    )
    .await?;
    if !out.success() {
        return Err(out.failure());
    }
    Ok(parse_porcelain(&out.stdout_lossy()))
}

const DIFF_ARGS: &[&str] = &["--no-color", "--no-ext-diff"];

/// `git diff HEAD` plus untracked files rendered as additions, as one
/// unified diff of at most [`DIFF_LIMIT_BYTES`]. Falls back to the index
/// diff on an unborn HEAD.
pub async fn diff(dir: &Path) -> Result<GitDiff, GitError> {
    let head_args = [&["diff", "HEAD"][..], DIFF_ARGS].concat();
    let mut out = GitCommand::new(dir, &head_args)
        .cap_stdout(DIFF_LIMIT_BYTES)
        .run()
        .await?;
    if !out.capped() && !out.success() {
        let index_args = [&["diff"][..], DIFF_ARGS].concat();
        out = GitCommand::new(dir, &index_args)
            .cap_stdout(DIFF_LIMIT_BYTES)
            .run()
            .await?;
        if !out.capped() && !out.success() {
            return Err(out.failure());
        }
    }
    let mut bytes = out.stdout;
    if bytes.len() > DIFF_LIMIT_BYTES {
        return Ok(finish(bytes, true));
    }

    let (files, mut truncated) = untracked_files(dir).await?;
    if !files.is_empty() {
        let budget = DIFF_LIMIT_BYTES - bytes.len();
        let (mut more, complete) = untracked_diff(dir, &files, budget).await?;
        truncated |= !complete || more.len() > budget;
        bytes.append(&mut more);
    }
    Ok(finish(bytes, truncated))
}

/// Lossy-decode `bytes`, cut to [`DIFF_LIMIT_BYTES`] on a char boundary.
fn finish(bytes: Vec<u8>, truncated: bool) -> GitDiff {
    let mut diff = String::from_utf8_lossy(&bytes).into_owned();
    let cut = diff.len() > DIFF_LIMIT_BYTES;
    truncate_utf8(&mut diff, DIFF_LIMIT_BYTES);
    GitDiff {
        diff,
        truncated: truncated || cut,
    }
}

/// Up to [`UNTRACKED_LIMIT`] untracked, non-ignored paths, and whether
/// there were more.
/// Paths are the raw bytes git printed (not decoded), so names that are not
/// UTF-8 reach `git add` unchanged.
async fn untracked_files(dir: &Path) -> Result<(Vec<Vec<u8>>, bool), GitError> {
    let out = git(dir, &["ls-files", "--others", "--exclude-standard", "-z"]).await?;
    if !out.success() {
        return Err(out.failure());
    }
    let mut files: Vec<Vec<u8>> = out
        .stdout
        .split(|&b| b == 0)
        .filter(|p| !p.is_empty())
        .map(<[u8]>::to_vec)
        .collect();
    let more = files.len() > UNTRACKED_LIMIT;
    files.truncate(UNTRACKED_LIMIT);
    Ok((files, more))
}

/// `dir/<raw>` as a filesystem path.
fn repo_path(dir: &Path, raw: &[u8]) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        dir.join(std::ffi::OsStr::from_bytes(raw))
    }
    #[cfg(not(unix))]
    {
        dir.join(String::from_utf8_lossy(raw).as_ref())
    }
}

/// NUL-terminated pathspec list for `--pathspec-from-file=- --pathspec-file-nul`.
fn nul_list(files: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::with_capacity(files.iter().map(|f| f.len() + 1).sum());
    for file in files {
        out.extend_from_slice(file);
        out.push(0);
    }
    out
}

/// Add `files` intent-to-add to the scratch `index`.
async fn add_intent_to_add(
    dir: &Path,
    index: &ScratchIndex,
    files: &[Vec<u8>],
) -> Result<GitOutput, GitError> {
    GitCommand::new(
        dir,
        &[
            "--literal-pathspecs",
            "add",
            "--intent-to-add",
            "--pathspec-from-file=-",
            "--pathspec-file-nul",
        ],
    )
    .env("GIT_INDEX_FILE", index.0.as_os_str())
    .stdin(nul_list(files))
    .run()
    .await
}

/// Temp index file, removed (with any lock file) on drop.
struct ScratchIndex(PathBuf);

impl ScratchIndex {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("bruhs-git-index-{}", uuid::Uuid::new_v4())))
    }
}

impl Drop for ScratchIndex {
    fn drop(&mut self) {
        // Best effort: git may not have created either file.
        let _ = std::fs::remove_file(&self.0);
        let mut lock = self.0.clone().into_os_string();
        lock.push(".lock");
        let _ = std::fs::remove_file(lock);
    }
}

/// `files` rendered by git itself as new-file diffs, in two processes no
/// matter how many files: they are added intent-to-add to a scratch index
/// (the real index is never touched) and that index is diffed against the
/// work tree. Output is capped at `budget + 1` bytes.
///
/// Untracked files come and go while this runs. If the add fails (a listed
/// file vanished, so its pathspec matches nothing) it is retried once with
/// the files that still exist; if git still refuses, or the diff itself
/// fails, the untracked part is left out. The `bool` is `false` whenever
/// something was left out, so the caller can mark the diff truncated.
async fn untracked_diff(
    dir: &Path,
    files: &[Vec<u8>],
    budget: usize,
) -> Result<(Vec<u8>, bool), GitError> {
    let index = ScratchIndex::new();
    let mut complete = true;
    let mut added = add_intent_to_add(dir, &index, files).await?;
    if !added.success() {
        let mut present = Vec::with_capacity(files.len());
        for file in files {
            if tokio::fs::symlink_metadata(repo_path(dir, file))
                .await
                .is_ok()
            {
                present.push(file.clone());
            }
        }
        tracing::debug!(
            "git add -N failed ({}); retrying with {} of {} untracked files",
            added.stderr.trim(),
            present.len(),
            files.len()
        );
        complete = false;
        if present.is_empty() {
            return Ok((Vec::new(), complete));
        }
        added = add_intent_to_add(dir, &index, &present).await?;
        if !added.success() {
            tracing::warn!(
                "leaving untracked files out of the diff: git add -N failed: {}",
                added.stderr.trim()
            );
            return Ok((Vec::new(), complete));
        }
    }
    let diff_args = [&["diff"][..], DIFF_ARGS].concat();
    let out = GitCommand::new(dir, &diff_args)
        .env("GIT_INDEX_FILE", index.0.as_os_str())
        .cap_stdout(budget)
        .run()
        .await?;
    if !out.capped() && !out.success() {
        tracing::warn!(
            "leaving untracked files out of the diff: git diff failed: {}",
            out.stderr.trim()
        );
        return Ok((Vec::new(), false));
    }
    Ok((out.stdout, complete))
}

/// Local branches (most recent commit first, current first) and the
/// current branch name (`None` when detached or not a repo).
pub async fn branches(dir: &Path) -> Result<Branches, GitError> {
    let list = git(
        dir,
        &[
            "branch",
            "--format=%(refname:short)",
            "--sort=-committerdate",
        ],
    )
    .await?;
    if !list.success() {
        return Err(list.failure());
    }
    let head = git(dir, &["rev-parse", "--abbrev-ref", "HEAD"]).await?;
    let current = if head.success() {
        let name = head.stdout_lossy().trim().to_string();
        (!name.is_empty() && name != "HEAD").then_some(name)
    } else {
        None
    };
    let branches = parse_branch_list(&list.stdout_lossy(), current.as_deref());
    Ok(Branches { current, branches })
}

/// Switch the work tree at `dir` to the existing local branch `branch`.
///
/// The name must pass git's ref rules and name an existing
/// `refs/heads/<branch>`; it is then passed after `--` to `git switch`, so
/// it can never be read as a pathspec (`git checkout .` discards every
/// unstaged change) or an option. `git switch` refuses to overwrite local
/// changes.
pub async fn checkout(dir: &Path, branch: &str) -> Result<(), GitError> {
    let branch = BranchName::try_from(branch)?;
    let refname = format!("refs/heads/{branch}");
    let verify = GitCommand::new(dir, &["rev-parse", "--verify", "--quiet", &refname])
        .run()
        .await?;
    if !verify.success() {
        return Err(GitError::UnknownBranch(branch.into()));
    }
    let out = GitCommand::new(dir, &["switch", "--no-guess", "--", branch.as_str()])
        .run()
        .await?;
    if out.success() {
        Ok(())
    } else {
        Err(out.failure())
    }
}

/// Bruhs data directory `worktrees/`, where new worktrees are created.
pub fn worktrees_root() -> Result<PathBuf, GitError> {
    directories::ProjectDirs::from("dev", "bruhs", "bruhs-ai")
        .map(|dirs| dirs.data_dir().join("worktrees"))
        .ok_or_else(|| GitError::Failed("no data directory for worktrees".into()))
}

/// Destination for a worktree of `project` on `branch`.
pub fn worktree_dest(project: &Path, branch: &str) -> Result<PathBuf, GitError> {
    let name = project.to_string_lossy();
    Ok(worktrees_root()?
        .join(bruhs_models::logic::run_place::worktree_project_dir(&name))
        .join(bruhs_models::logic::run_place::worktree_dir_name(branch)))
}

/// Add a worktree of `repo` at `dest` on `branch`, starting from `base`
/// when `branch` does not exist yet.
///
/// A second call is a no-op when `dest` already exists and its HEAD is
/// `branch`. The checkout at `repo` is left on its current branch.
pub async fn add_worktree(
    repo: &Path,
    base: &str,
    dest: &Path,
    branch: &str,
) -> Result<(), GitError> {
    let branch = BranchName::try_from(branch)?;
    let base = BranchName::try_from(base)?;
    let base_ref = format!("refs/heads/{base}");
    let verify_base = GitCommand::new(repo, &["rev-parse", "--verify", "--quiet", &base_ref])
        .run()
        .await?;
    if !verify_base.success() {
        return Err(GitError::UnknownBranch(base.into()));
    }

    if dest.exists() {
        let head = GitCommand::new(dest, &["rev-parse", "--abbrev-ref", "HEAD"])
            .run()
            .await?;
        if head.success() && head.stdout_lossy().trim() == branch.as_str() {
            return Ok(());
        }
        return Err(GitError::Failed(
            "worktree destination already exists".into(),
        ));
    }

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|error| GitError::Failed(error.to_string()))?;
    }

    let dest_string = dest.to_string_lossy().into_owned();
    let branch_ref = format!("refs/heads/{branch}");
    let existing = GitCommand::new(repo, &["rev-parse", "--verify", "--quiet", &branch_ref])
        .run()
        .await?;
    let out = if existing.success() {
        GitCommand::new(repo, &["worktree", "add", &dest_string, branch.as_str()])
            .run()
            .await?
    } else {
        GitCommand::new(
            repo,
            &[
                "worktree",
                "add",
                "-b",
                branch.as_str(),
                &dest_string,
                base.as_str(),
            ],
        )
        .run()
        .await?
    };
    if out.success() {
        Ok(())
    } else {
        Err(out.failure())
    }
}

/// Tree with no entries. `git hash-object -t tree --stdin </dev/null`.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// A 40-character hex object id. Not a proof that the object is a tree.
pub fn is_tree_oid(oid: &str) -> bool {
    oid.len() == 40 && oid.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn add_reported_nothing(stderr: &str) -> bool {
    let stderr = stderr.to_ascii_lowercase();
    stderr.contains("nothing specified")
        || stderr.contains("nothing added")
        || stderr.contains("did not match any file")
}

/// Tree of `dir`'s worktree at this moment, including untracked files and
/// omitting ignored ones. The repository's own index is left alone: the
/// snapshot uses a private `GIT_INDEX_FILE`.
///
/// Callers that talk to a user worktree must confine `dir` with
/// [`ensure_bruhs_worktree`] first. This function itself accepts any
/// repository so a test can use a temp dir.
pub async fn snapshot_worktree(dir: &Path) -> Result<String, GitError> {
    let index_dir = tempfile::tempdir().map_err(|error| GitError::Failed(error.to_string()))?;
    let index_file = index_dir.path().join("index").into_os_string();
    let add = GitCommand::new(dir, &["add", "-A"])
        .env("GIT_INDEX_FILE", index_file.clone())
        .run()
        .await?;
    if !add.success() {
        if add_reported_nothing(&add.stderr) {
            return Ok(EMPTY_TREE.to_string());
        }
        return Err(add.failure());
    }
    let written = GitCommand::new(dir, &["write-tree"])
        .env("GIT_INDEX_FILE", index_file)
        .run()
        .await?;
    if !written.success() {
        return Err(written.failure());
    }
    let oid = written.stdout_lossy();
    let oid = oid.trim();
    if !is_tree_oid(oid) {
        return Err(GitError::Failed(format!("git write-tree returned {oid:?}")));
    }
    Ok(oid.to_string())
}

/// Put `dir`'s worktree and index back to `tree`. Files created after the
/// snapshot are removed. Ignored files stay (`git clean -fd`, not `-x`).
///
/// `tree` has to be a tree object in this repository. Callers must confine
/// `dir` with [`ensure_bruhs_worktree`] before calling this on a user path.
pub async fn restore_worktree(dir: &Path, tree: &str) -> Result<(), GitError> {
    let tree = tree.trim();
    if !is_tree_oid(tree) {
        return Err(GitError::Failed("checkpoint is not a tree".into()));
    }
    let kind = GitCommand::new(dir, &["cat-file", "-t", tree])
        .run()
        .await?;
    if !kind.success() || kind.stdout_lossy().trim() != "tree" {
        return Err(GitError::Failed("checkpoint is not a tree".into()));
    }
    let read = GitCommand::new(dir, &["read-tree", "--reset", tree])
        .run()
        .await?;
    if !read.success() {
        return Err(read.failure());
    }
    let checkout = GitCommand::new(dir, &["checkout-index", "-a", "-f"])
        .run()
        .await?;
    if !checkout.success() {
        return Err(checkout.failure());
    }
    let clean = GitCommand::new(dir, &["clean", "-fd"]).run().await?;
    if clean.success() {
        Ok(())
    } else {
        Err(clean.failure())
    }
}

/// Canonical `path` when it lives under the bruhs worktrees root.
///
/// The root is created when it is missing. A path that does not exist, or
/// that canonicalizes outside that root, is refused. Snapshot and restore
/// do not call this, so a temp repository can still be tested.
pub fn ensure_bruhs_worktree(path: &Path) -> Result<PathBuf, GitError> {
    let root = worktrees_root()?;
    std::fs::create_dir_all(&root).map_err(|error| GitError::Failed(error.to_string()))?;
    let root = std::fs::canonicalize(&root).map_err(|error| GitError::Failed(error.to_string()))?;
    let path = std::fs::canonicalize(path).map_err(|error| GitError::Failed(error.to_string()))?;
    if path.starts_with(&root) {
        Ok(path)
    } else {
        Err(GitError::Failed("path is not a bruhs worktree".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::logic::git::{is_valid_branch, DirtyFile, FileStatus};

    /// A throwaway repository with one commit on `main`. Git runs with an
    /// isolated config so the user's settings and hooks never apply.
    struct Repo {
        dir: tempfile::TempDir,
    }

    impl Repo {
        fn new() -> Self {
            let repo = Self {
                dir: tempfile::tempdir().unwrap(),
            };
            repo.git(&["init", "-q", "-b", "main"]);
            repo.write("README.md", "hello\n");
            repo.git(&["add", "README.md"]);
            repo.git(&["commit", "-q", "-m", "init"]);
            repo
        }

        fn path(&self) -> &Path {
            self.dir.path()
        }

        fn git(&self, args: &[&str]) -> String {
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
                .current_dir(self.path())
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

        fn write(&self, rel: &str, contents: &str) {
            let path = self.path().join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, contents).unwrap();
        }

        fn read(&self, rel: &str) -> String {
            std::fs::read_to_string(self.path().join(rel)).unwrap()
        }

        fn current_branch(&self) -> String {
            self.git(&["branch", "--show-current"]).trim().to_string()
        }
    }

    /// `checkout(".")` used to run `git checkout .`, discarding every
    /// unstaged change. Names git itself refuses never reach `git`.
    #[tokio::test]
    async fn checkout_refuses_pathspec_like_names() {
        for name in [".", "./", "a/.", ".hidden", "HEAD", "@", "x.lock", "a//b"] {
            assert!(!is_valid_branch(name), "{name:?}");
            assert!(
                matches!(
                    checkout(Path::new("/nonexistent"), name).await,
                    Err(GitError::InvalidBranch(_))
                ),
                "{name:?}"
            );
        }
    }

    /// `README.md` is a valid branch name and also a file. Without a branch
    /// of that name, `git checkout README.md` would discard the edit.
    #[tokio::test]
    async fn checkout_never_discards_local_changes() {
        let repo = Repo::new();
        repo.write("README.md", "edited\n");

        assert!(matches!(
            checkout(repo.path(), ".").await,
            Err(GitError::InvalidBranch(_))
        ));
        assert!(matches!(
            checkout(repo.path(), "README.md").await,
            Err(GitError::UnknownBranch(name)) if name == "README.md"
        ));
        assert_eq!(repo.read("README.md"), "edited\n");
        assert_eq!(repo.current_branch(), "main");

        // A real branch named like the file is switched to as a branch.
        repo.git(&["branch", "README.md"]);
        checkout(repo.path(), "README.md").await.unwrap();
        assert_eq!(repo.current_branch(), "README.md");
        assert_eq!(repo.read("README.md"), "edited\n");
    }

    #[tokio::test]
    async fn checkout_switches_to_an_existing_branch() {
        let repo = Repo::new();
        repo.git(&["branch", "feature/x"]);
        checkout(repo.path(), "feature/x").await.unwrap();
        assert_eq!(repo.current_branch(), "feature/x");
        let listed = branches(repo.path()).await.unwrap();
        assert_eq!(listed.current.as_deref(), Some("feature/x"));
        assert_eq!(listed.branches[0], "feature/x");
    }

    #[tokio::test]
    async fn status_handles_awkward_paths_via_z() {
        let repo = Repo::new();
        repo.write("README.md", "edited\n");
        repo.write("a -> b.txt", "x");
        repo.write("quote\"d.txt", "x");
        let status = status(repo.path()).await.unwrap();
        assert_eq!(status.branch.as_deref(), Some("main"));
        let mut dirty: Vec<(String, FileStatus)> = status
            .dirty
            .into_iter()
            .map(|f| (f.path, f.status))
            .collect();
        dirty.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            dirty,
            vec![
                ("README.md".to_string(), FileStatus::Modified),
                ("a -> b.txt".to_string(), FileStatus::Untracked),
                ("quote\"d.txt".to_string(), FileStatus::Untracked),
            ]
        );
    }

    #[tokio::test]
    async fn diff_includes_tracked_and_untracked_changes_without_touching_the_index() {
        let repo = Repo::new();
        repo.write("README.md", "edited\n");
        repo.write("new file.txt", "one\ntwo\n");
        repo.write("dir/*glob", "g\n");
        let index_before = std::fs::read(repo.path().join(".git/index")).unwrap();

        let got = diff(repo.path()).await.unwrap();
        assert!(!got.truncated);
        assert!(got.diff.contains("+edited"), "{}", got.diff);
        assert!(got.diff.contains("+++ b/new file.txt"), "{}", got.diff);
        assert!(got.diff.contains("+two"), "{}", got.diff);
        assert!(got.diff.contains("+++ b/dir/*glob"), "{}", got.diff);
        assert_eq!(got.diff.matches("new file mode").count(), 2);

        assert_eq!(
            std::fs::read(repo.path().join(".git/index")).unwrap(),
            index_before
        );
        let untracked = repo.git(&["ls-files", "--others", "--exclude-standard"]);
        assert_eq!(untracked.lines().count(), 2);
    }

    /// Untracked files can vanish between `ls-files` and `add`; one stale
    /// path used to fail the whole diff (git exits 128 on a pathspec that
    /// matches nothing).
    #[tokio::test]
    async fn untracked_files_that_vanish_are_skipped_not_fatal() {
        let repo = Repo::new();
        repo.write("kept.txt", "kept\n");
        let files = vec![b"kept.txt".to_vec(), b"gone.txt".to_vec()];
        let (out, complete) = untracked_diff(repo.path(), &files, DIFF_LIMIT_BYTES)
            .await
            .unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("+++ b/kept.txt"), "{out}");
        assert!(!out.contains("gone.txt"), "{out}");
        assert!(!complete);

        // Nothing left at all: an empty, incomplete untracked part.
        let (out, complete) = untracked_diff(repo.path(), &[b"gone.txt".to_vec()], 1024)
            .await
            .unwrap();
        assert!(out.is_empty());
        assert!(!complete);

        // And through `diff()`: the tracked part still comes back.
        repo.write("README.md", "edited\n");
        let got = diff(repo.path()).await.unwrap();
        assert!(got.diff.contains("+edited"));
        assert!(got.diff.contains("+++ b/kept.txt"));
    }

    /// Non-UTF-8 names (possible on Linux; APFS refuses them) must reach
    /// `git add` byte for byte.
    #[cfg(unix)]
    #[tokio::test]
    async fn non_utf8_untracked_names_are_passed_exactly() {
        use std::os::unix::ffi::OsStrExt;
        let repo = Repo::new();
        let name: &[u8] = b"caf\xe9.txt";
        let path = repo.path().join(std::ffi::OsStr::from_bytes(name));
        if std::fs::write(&path, "x\n").is_err() {
            return; // filesystem only allows UTF-8 names
        }
        let (files, more) = untracked_files(repo.path()).await.unwrap();
        assert!(!more);
        assert_eq!(files, vec![name.to_vec()]);
        let got = diff(repo.path()).await.unwrap();
        assert!(!got.truncated, "{got:?}");
        assert_eq!(got.diff.matches("new file mode").count(), 1);
    }

    #[tokio::test]
    async fn diff_works_on_an_unborn_branch() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::fs::write(dir.path().join("a.txt"), "a\n").unwrap();
        let got = diff(dir.path()).await.unwrap();
        assert!(got.diff.contains("+++ b/a.txt"), "{}", got.diff);
    }

    #[tokio::test]
    async fn untracked_files_beyond_the_limit_mark_the_diff_truncated() {
        let repo = Repo::new();
        for i in 0..=UNTRACKED_LIMIT {
            repo.write(&format!("f{i:03}.txt"), "x\n");
        }
        let got = diff(repo.path()).await.unwrap();
        assert!(got.truncated);
        assert_eq!(got.diff.matches("new file mode").count(), UNTRACKED_LIMIT);
    }

    /// The diff used to be read whole and then cut; now git is stopped once
    /// the limit is reached.
    #[tokio::test]
    async fn oversized_diffs_are_cut_at_the_limit() {
        let repo = Repo::new();
        let line = "x".repeat(1023) + "\n";
        repo.write("big.txt", &line.repeat(3 * 1024));
        repo.git(&["add", "big.txt"]);
        repo.git(&["commit", "-q", "-m", "big"]);
        repo.write("big.txt", &"y".repeat(1023).repeat(3 * 1024));
        let got = diff(repo.path()).await.unwrap();
        assert!(got.truncated);
        assert!(got.diff.len() <= DIFF_LIMIT_BYTES);
        assert!(got.diff.len() > DIFF_LIMIT_BYTES - 4);
    }

    #[tokio::test]
    async fn capped_reads_stop_the_child() {
        let dir = tempfile::tempdir().unwrap();
        // `git help -a` prints far more than 16 bytes.
        let out = GitCommand::new(dir.path(), &["help", "-a"])
            .cap_stdout(16)
            .run()
            .await
            .unwrap();
        assert!(out.capped());
        assert_eq!(out.stdout.len(), 17);
    }

    #[tokio::test]
    async fn errors_carry_git_stderr() {
        let dir = tempfile::tempdir().unwrap();
        match status(dir.path()).await {
            Err(GitError::Failed(msg)) => assert!(msg.contains("not a git repository"), "{msg}"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn branch_list_puts_current_first() {
        let out = " feature/a \nmain\n\nfix/b\n";
        assert_eq!(
            parse_branch_list(out, Some("main")),
            vec!["main", "feature/a", "fix/b"]
        );
        assert_eq!(
            parse_branch_list(out, None),
            vec!["feature/a", "main", "fix/b"]
        );
    }

    #[test]
    fn status_json_shape_matches_the_web_contract() {
        let s = GitStatus {
            branch: Some("main".into()),
            ahead: 1,
            behind: 0,
            dirty: vec![DirtyFile {
                path: "a.rs".into(),
                status: FileStatus::Modified,
                staged: false,
            }],
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "branch": "main", "ahead": 1, "behind": 0,
                "dirty": [{ "path": "a.rs", "status": "M", "staged": false }]
            })
        );
    }

    fn head_of(dir: &Path) -> String {
        let out = std::process::Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(out.status.success());
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A new worktree gets its own branch. The project's checkout, including
    /// a dirty file, stays as it was. Calling again is a no-op.
    #[tokio::test]
    async fn add_worktree_keeps_the_checkout_and_is_idempotent() {
        let repo = Repo::new();
        repo.write("README.md", "dirty\n");
        let dest_root = tempfile::tempdir().unwrap();
        let dest = dest_root.path().join("wt");

        add_worktree(repo.path(), "main", &dest, "bruhs/abc123")
            .await
            .unwrap();
        assert_eq!(head_of(&dest), "bruhs/abc123");
        assert_eq!(
            std::fs::read_to_string(dest.join("README.md")).unwrap(),
            "hello\n"
        );
        assert_eq!(repo.read("README.md"), "dirty\n");
        assert_eq!(repo.current_branch(), "main");

        add_worktree(repo.path(), "main", &dest, "bruhs/abc123")
            .await
            .unwrap();
        assert_eq!(repo.current_branch(), "main");

        let missing = dest_root.path().join("missing");
        assert!(matches!(
            add_worktree(repo.path(), "no-such", &missing, "bruhs/other").await,
            Err(GitError::UnknownBranch(name)) if name == "no-such"
        ));
        assert!(!missing.exists());
        assert!(matches!(
            add_worktree(repo.path(), "main", &missing, "bad..name").await,
            Err(GitError::InvalidBranch(_))
        ));

        let occupied = dest_root.path().join("occupied");
        std::fs::create_dir_all(&occupied).unwrap();
        assert!(matches!(
            add_worktree(repo.path(), "main", &occupied, "bruhs/abc123").await,
            Err(GitError::Failed(message)) if message.contains("already exists")
        ));
    }

    #[test]
    fn worktree_dest_is_one_directory_per_project_and_branch() {
        let dest = worktree_dest(Path::new("/repo/bruhs ai"), "bruhs/abc").unwrap();
        let dest = dest.to_string_lossy().replace('\\', "/");
        assert!(dest.ends_with("/worktrees/bruhs-ai/bruhs-abc"), "{dest}");
    }

    #[tokio::test]
    async fn snapshot_leaves_the_index_and_restore_puts_files_back() {
        let repo = Repo::new();
        repo.write("README.md", "edited\n");
        repo.write("note.txt", "before\n");
        repo.write(".gitignore", "secret\n");
        repo.write("secret", "keep\n");
        let head_tree = repo.git(&["rev-parse", "HEAD^{tree}"]);
        let tree = snapshot_worktree(repo.path()).await.unwrap();
        assert!(is_tree_oid(&tree));
        assert_eq!(repo.git(&["write-tree"]), head_tree);

        repo.write("README.md", "later\n");
        repo.write("later.txt", "after\n");
        std::fs::remove_file(repo.path().join("note.txt")).unwrap();
        restore_worktree(repo.path(), &tree).await.unwrap();
        assert_eq!(repo.read("README.md"), "edited\n");
        assert_eq!(repo.read("note.txt"), "before\n");
        assert_eq!(repo.read(".gitignore"), "secret\n");
        assert_eq!(repo.read("secret"), "keep\n");
        assert!(!repo.path().join("later.txt").exists());
        assert_eq!(repo.git(&["write-tree"]).trim(), tree);

        assert!(restore_worktree(repo.path(), "nope").await.is_err());
        assert!(restore_worktree(repo.path(), &"a".repeat(40))
            .await
            .is_err());
        assert_eq!(repo.read("README.md"), "edited\n");
    }

    #[tokio::test]
    async fn an_empty_worktree_snapshots_the_empty_tree() {
        let repo = Repo::new();
        std::fs::remove_file(repo.path().join("README.md")).unwrap();
        let tree = snapshot_worktree(repo.path()).await.unwrap();
        assert_eq!(tree, EMPTY_TREE);
        repo.write("later.txt", "after\n");
        restore_worktree(repo.path(), &tree).await.unwrap();
        assert!(!repo.path().join("README.md").exists());
        assert!(!repo.path().join("later.txt").exists());
    }

    #[test]
    fn a_checkout_outside_the_worktrees_root_is_refused() {
        let repo = Repo::new();
        assert!(matches!(
            ensure_bruhs_worktree(repo.path()),
            Err(GitError::Failed(message)) if message.contains("not a bruhs worktree")
        ));
        assert!(ensure_bruhs_worktree(&repo.path().join("missing")).is_err());
    }
}
