//! A macOS Labs preference shared by the desktop and its local task host.
//! Cache entries contain only a private checkout written from a Git tree.
mod index;

use super::{clone_fs, git, register_checkout, GitCommand, GitError};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io,
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, MetadataExt},
    },
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
use unicode_normalization::UnicodeNormalization;

fn dirs() -> io::Result<directories::ProjectDirs> {
    directories::ProjectDirs::from("dev", "bruhs", "bruhs-ai")
        .ok_or_else(|| io::Error::other("Worktree settings folder unavailable."))
}

struct Cache {
    flag: PathBuf,
    root: PathBuf,
}

impl Cache {
    fn local() -> io::Result<Self> {
        let dirs = dirs()?;
        Ok(Self {
            flag: dirs.data_dir().join("fast-worktree-checkout"),
            root: dirs.cache_dir().join("worktree-checkouts"),
        })
    }

    fn lock(&self) -> io::Result<File> {
        let data = self
            .flag
            .parent()
            .ok_or_else(|| io::Error::other("missing cache data directory"))?;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(data)?;
        File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.flag.with_extension("lock"))
    }

    fn set(&self, on: bool) -> io::Result<()> {
        let lock = self.lock()?;
        lock.lock()?;
        if on {
            if self.generation().is_none() {
                use std::io::Write;
                let parent = self
                    .flag
                    .parent()
                    .ok_or_else(|| io::Error::other("missing cache data directory"))?;
                let mut pending = tempfile::NamedTempFile::new_in(parent)?;
                pending.write_all(uuid::Uuid::new_v4().to_string().as_bytes())?;
                pending.as_file().sync_all()?;
                pending.persist(&self.flag)?;
            }
        } else {
            match fs::remove_file(&self.flag) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
            // Hide the old generation before another process can enable caching.
            if self.root.try_exists()? {
                let parent = self
                    .root
                    .parent()
                    .ok_or_else(|| io::Error::other("missing cache root parent directory"))?;
                let retired = tempfile::Builder::new()
                    .prefix("retired-checkouts-")
                    .tempdir_in(parent)?;
                fs::rename(&self.root, retired.path().join("entries"))?;
                drop(lock);
                retired.close()?;
            }
        }
        Ok(())
    }

    fn generation(&self) -> Option<String> {
        let flag = fs::read_to_string(&self.flag).ok()?;
        uuid::Uuid::parse_str(&flag).ok()?;
        Some(flag)
    }
}

pub fn enabled() -> bool {
    Cache::local().is_ok_and(|c| c.generation().is_some())
}

/// Finish outstanding cache operations before clearing their snapshots. The
/// blocking file lock must leave the async executor free to finish those jobs.
pub async fn set_enabled(on: bool) -> io::Result<()> {
    tokio::task::spawn_blocking(move || Cache::local()?.set(on))
        .await
        .map_err(io::Error::other)?
}

const LIMIT: usize = 64 * 1024 * 1024;

async fn output(repo: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let out = GitCommand::new(repo, args)
        .cap_stdout(LIMIT)
        .run()
        .await
        .ok()?;
    out.success().then_some(out.stdout)
}

async fn value(repo: &Path, args: &[&str]) -> Option<String> {
    String::from_utf8(output(repo, args).await?)
        .ok()
        .map(|s| s.trim_end_matches('\n').to_owned())
}

#[derive(PartialEq, Eq)]
struct Tree {
    id: String,
    bucket: PathBuf,
    profile: String,
    checkout_hook: Option<PathBuf>,
}

// Git can materialize different bytes for the same tree. Include checkout
// configuration and non-versioned attribute files in the compatibility key.
async fn describe(cache: &Cache, repo: &Path, start: &str) -> Option<Tree> {
    let spec = format!("{start}^{{tree}}");
    let desc = value(
        repo,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-common-dir",
            "--git-path",
            "hooks/post-index-change",
            "--git-path",
            "info/attributes",
            &spec,
        ],
    )
    .await?;
    let mut lines = desc.lines();
    let common = Path::new(lines.next()?).canonicalize().ok()?;
    let hook = Path::new(lines.next()?);
    if hook.try_exists().ok()? {
        return None;
    }
    let info_attrs = PathBuf::from(lines.next()?);
    let id = lines.next()?.to_owned();
    if lines.next().is_some() || id.len() != 40 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let config = output(repo, &["config", "--null", "--list"]).await?;
    let mut checkout_hook = None;
    for entry in config.split(|&b| b == 0) {
        let mut pair = entry.splitn(2, |&b| b == b'\n');
        let key = pair.next()?;
        let setting = pair.next().unwrap_or_default();
        if key == b"core.sparsecheckout" && !matches!(setting, b"false" | b"0" | b"off" | b"no") {
            return None;
        }
        if key == b"core.hookspath" {
            let folder = Path::new(std::ffi::OsStr::from_bytes(setting));
            checkout_hook = if folder.is_absolute() {
                None
            } else {
                if folder
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
                {
                    return None;
                }
                Some(folder.join("post-index-change"))
            };
        }
    }
    let mut hash = Sha256::new();
    // Branch tracking and identity change routinely and do not affect checkout.
    for entry in config.split(|&b| b == 0) {
        let key = entry.split(|&b| b == b'\n').next()?;
        if [
            b"core.".as_slice(),
            b"filter.",
            b"lfs.",
            b"extensions.",
            b"checkout.",
        ]
        .iter()
        .any(|prefix| key.starts_with(prefix))
        {
            hash.update(entry);
            hash.update([0]);
        }
    }
    let mut attrs = vec![info_attrs];
    for key in ["GIT_ATTR_SYSTEM", "GIT_ATTR_GLOBAL"] {
        let path = value(repo, &["var", key]).await?;
        if !path.is_empty() {
            attrs.push(PathBuf::from(path));
        }
    }
    for p in attrs {
        if !p.is_absolute() {
            return None;
        }
        if p == Path::new("/dev/null") {
            hash.update([0]);
            continue;
        }
        match fs::metadata(&p) {
            Ok(meta) if !meta.is_file() || meta.len() > LIMIT as u64 => return None,
            Err(e) if e.kind() != io::ErrorKind::NotFound => return None,
            _ => {}
        }
        match fs::read(p) {
            Ok(bytes) => hash.update(bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return None,
        }
        hash.update([0]);
    }
    let bucket = cache.root.join(format!(
        "{:x}",
        Sha256::digest(common.as_os_str().as_bytes())
    ));
    Some(Tree {
        id,
        bucket,
        profile: format!("{:x}", hash.finalize()),
        checkout_hook,
    })
}

impl Tree {
    fn entry(&self) -> PathBuf {
        self.bucket.join(format!("{}-{}", self.id, self.profile))
    }
    // Attribute transitions can change the checkout representation of an otherwise
    // identical blob. Leave those transitions to a fresh Git checkout.
    async fn changed_paths(&self, repo: &Path, base: &str) -> Option<usize> {
        let list = output(
            repo,
            &[
                "diff-tree",
                "-r",
                "--no-renames",
                "--name-only",
                "-z",
                base,
                &self.id,
            ],
        )
        .await?;
        let mut count = 0;
        for path in list.split(|&b| b == 0).filter(|p| !p.is_empty()) {
            if path.rsplit(|&b| b == b'/').next() == Some(b".gitattributes") {
                return None;
            }
            count += 1;
        }
        Some(count)
    }

    async fn eligible(&self, repo: &Path) -> bool {
        let Some(list) = output(repo, &["ls-tree", "--full-tree", "-rtz", &self.id]).await else {
            return false;
        };
        let mut names = vec![];
        let mut spellings = std::collections::HashSet::new();
        for line in list.split(|&b| b == 0).filter(|s| !s.is_empty()) {
            let Some(tab) = line.iter().position(|&b| b == b'\t') else {
                return false;
            };
            let Ok(path) = std::str::from_utf8(&line[tab + 1..]) else {
                return false;
            };
            if !spellings.insert(path.nfd().flat_map(char::to_lowercase).collect::<String>()) {
                // Git decides how to handle colliding names on this filesystem.
                return false;
            }
            if line.starts_with(b"040000 ") || line.starts_with(b"160000 ") {
                continue;
            }
            names.extend_from_slice(path.as_bytes());
            names.push(0);
        }
        let source = format!("--source={}", self.id);
        let Ok(out) = GitCommand::new(repo, &["check-attr", &source, "-z", "--stdin", "filter"])
            .stdin(names)
            .cap_stdout(LIMIT)
            .run()
            .await
        else {
            return false;
        };
        out.success()
            && out
                .stdout
                .split(|&b| b == 0)
                .skip(2)
                .step_by(3)
                .all(|v| v == b"unspecified" || v == b"unset")
    }
}

// Bind the entire private tree and its index to their inode, permissions,
// timestamps and size. This catches edits, replacements and added ignored files
// without reading every blob on a cache hit. Never traverse symlinks.
fn seal(entry: &Path) -> io::Result<String> {
    let mut hash = Sha256::new();
    let mut paths = vec![entry.join("files"), entry.join("index")];
    while let Some(path) = paths.pop() {
        let meta = path.symlink_metadata()?;
        hash.update(
            path.strip_prefix(entry)
                .map_err(io::Error::other)?
                .as_os_str()
                .as_bytes(),
        );
        hash.update([0]);
        for n in [
            meta.dev(),
            meta.ino(),
            u64::from(meta.mode()),
            meta.size(),
            u64::from_ne_bytes(meta.mtime().to_ne_bytes()),
            u64::from_ne_bytes(meta.mtime_nsec().to_ne_bytes()),
            u64::from_ne_bytes(meta.ctime().to_ne_bytes()),
            u64::from_ne_bytes(meta.ctime_nsec().to_ne_bytes()),
        ] {
            hash.update(n.to_le_bytes());
        }
        if meta.is_dir() {
            let mut children = fs::read_dir(path)?
                .map(|e| e.map(|e| e.path()))
                .collect::<io::Result<Vec<_>>>()?;
            children.sort_unstable();
            paths.extend(children);
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}

async fn indexed(repo: &Path, stage: &Path, args: &[&str]) -> bool {
    let mut flags = vec![
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "core.fsmonitor=false",
        "-c",
        "core.splitIndex=false",
        "-c",
        "index.version=2",
        "-c",
        "index.skipHash=false",
        "-c",
        "core.untrackedCache=false",
        "-c",
        "core.trustctime=false",
        "-c",
        "core.checkStat=minimal",
    ];
    flags.extend_from_slice(args);
    GitCommand::new(repo, &flags)
        .env("GIT_INDEX_FILE", stage.join("index"))
        .env("GIT_WORK_TREE", stage.join("files"))
        .run()
        .await
        .is_ok_and(|out| out.success())
}

fn entries(tree: &Tree) -> io::Result<Vec<(SystemTime, String, PathBuf)>> {
    let mut list = vec![];
    for item in fs::read_dir(&tree.bucket)? {
        let item = item?;
        let name = item.file_name();
        let Some((id, profile)) = name.to_str().and_then(|s| s.split_once('-')) else {
            continue;
        };
        if id.len() == 40
            && id.bytes().all(|b| b.is_ascii_hexdigit())
            && profile.len() == 64
            && profile.bytes().all(|b| b.is_ascii_hexdigit())
            && item.file_type()?.is_dir()
        {
            list.push((item.metadata()?.modified()?, id.to_owned(), item.path()));
        }
    }
    list.sort_unstable_by_key(|item| std::cmp::Reverse(item.0));
    Ok(list)
}

async fn warm(cache: Cache, generation: String, repo: PathBuf) -> Option<()> {
    let lock = cache.lock().ok()?;
    lock.try_lock_shared().ok()?;
    if cache.generation().as_ref() != Some(&generation) {
        return None;
    }
    let tree = describe(&cache, &repo, "HEAD").await?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&tree.bucket)
        .ok()?;
    // A single writer per repository. Readers use only atomically published entries.
    let build = File::options()
        .create(true)
        .write(true)
        .truncate(false)
        .open(tree.bucket.join("build.lock"))
        .ok()?;
    build.try_lock().ok()?;
    if tree.entry().is_dir() || !tree.eligible(&repo).await {
        return None;
    }
    for (_, base, cached) in entries(&tree).ok()? {
        if cached
            .file_name()?
            .as_bytes()
            .ends_with(tree.profile.as_bytes())
            && tree
                .changed_paths(&repo, &base)
                .await
                .is_some_and(|count| count < 500)
        {
            return None;
        }
    }
    // The build lock also covers recovery from an interrupted snapshot writer.
    for child in fs::read_dir(&tree.bucket).ok()?.flatten() {
        if child.file_name().as_bytes().starts_with(b".building-") {
            let _ = fs::remove_dir_all(child.path());
        }
    }
    let stage = tempfile::Builder::new()
        .prefix(".building-")
        .tempdir_in(&tree.bucket)
        .ok()?;
    fs::create_dir(stage.path().join("files")).ok()?;
    if !indexed(
        &repo,
        stage.path(),
        &["read-tree", "--reset", "-u", &tree.id],
    )
    .await
    {
        return None;
    }
    let built = stage.path().to_owned();
    let stamp = tokio::task::spawn_blocking(move || seal(&built))
        .await
        .ok()?
        .ok()?;
    fs::write(stage.path().join("seal"), stamp).ok()?;
    fs::rename(stage.path(), tree.entry()).ok()?;
    if let Ok(items) = entries(&tree) {
        for (_, _, path) in items.into_iter().skip(3) {
            let _ = fs::remove_dir_all(path);
        }
    }
    let _ = File::open(&tree.bucket).and_then(|f| f.set_modified(SystemTime::now()));
    for child in fs::read_dir(&cache.root).ok()?.flatten() {
        if child.file_name().as_bytes().len() == 64
            && child
                .file_name()
                .as_bytes()
                .iter()
                .all(u8::is_ascii_hexdigit)
            && child
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > Duration::from_secs(30 * 86400))
        {
            // Skip repositories currently building.
            if let Ok(f) = File::options()
                .write(true)
                .open(child.path().join("build.lock"))
            {
                if f.try_lock().is_ok() {
                    let _ = fs::remove_dir_all(child.path());
                }
            }
        }
    }
    Some(())
}

pub(super) fn warm_after_checkout(repo: &Path) {
    let Ok(cache) = Cache::local() else { return };
    let Some(generation) = cache.generation() else {
        return;
    };
    let repo = repo.to_owned();
    tokio::spawn(async move {
        warm(cache, generation, repo).await;
    });
}

fn clone_verified(entry: &Path, stage: &Path) -> Option<()> {
    let stamp = fs::read_to_string(entry.join("seal")).ok()?;
    if stamp != seal(entry).ok()? {
        let _ = fs::remove_dir_all(entry);
        return None;
    }
    clone_fs::clone_path(&entry.join("files"), &stage.join("files")).ok()?;
    let index = index::for_clone(&entry.join("index"), &stage.join("files"))?;
    fs::write(stage.join("index"), index).ok()?;
    (stamp == seal(entry).ok()?).then_some(())
}

async fn prepare(repo: &Path, tree: &Tree, stage: &Path) -> Option<()> {
    let mut candidates = entries(tree).ok()?;
    candidates.sort_by_key(|(_, id, _)| id != &tree.id);
    for (_, base, entry) in candidates {
        if !entry
            .file_name()?
            .as_bytes()
            .ends_with(tree.profile.as_bytes())
        {
            continue;
        }
        if base != tree.id
            && (tree.changed_paths(repo, &base).await.is_none() || !tree.eligible(repo).await)
        {
            continue;
        }
        let source = entry.clone();
        let target = stage.to_owned();
        tokio::task::spawn_blocking(move || clone_verified(&source, &target))
            .await
            .ok()??;
        if base != tree.id
            && !indexed(repo, stage, &["read-tree", "-m", "-u", &base, &tree.id]).await
        {
            return None;
        }
        if let Some(hook) = &tree.checkout_hook {
            if stage.join("files").join(hook).try_exists().ok()? {
                return None;
            }
        }
        let now = SystemTime::now();
        for p in [&entry, &tree.bucket] {
            let _ = File::open(p).and_then(|f| f.set_modified(now));
        }
        return Some(());
    }
    None
}

// Registration happens only after the cloned tree is ready. Construction stays
// inside a private sibling of the destination until Git publishes the worktree.
async fn publish(
    cache: &Cache,
    repo: &Path,
    dest: &Path,
    stage: &Path,
    tree: &Tree,
    new_branch: Option<&str>,
    start: &str,
) -> Result<(), GitError> {
    let parent = dest
        .parent()
        .ok_or_else(|| GitError::Failed("Missing worktree parent.".into()))?;
    let name = dest
        .file_name()
        .ok_or_else(|| GitError::Failed("Missing worktree name.".into()))?;
    let private = stage.join("registration");
    fs::create_dir(&private)?;
    let registration = private.join(name);
    let registered = registration
        .to_str()
        .ok_or_else(|| GitError::Failed("Invalid checkout path.".into()))?;
    register_checkout(repo, registered, new_branch, start, true).await?;
    let finish = async {
        let actual = describe(cache, &registration, "HEAD").await;
        if actual.as_ref() != Some(tree) {
            return Err(GitError::Failed(
                "The checkout configuration or starting commit changed.".into(),
            ));
        }
        let index = value(
            &registration,
            &["rev-parse", "--path-format=absolute", "--git-path", "index"],
        )
        .await
        .ok_or_else(|| GitError::Failed("Git index unavailable.".into()))?;
        // Swap whole directories so even top-level files keep the inode and
        // ctime written into the prepared index. Both paths are private.
        let files = stage.join("files");
        fs::copy(registration.join(".git"), files.join(".git"))?;
        let from =
            std::ffi::CString::new(files.as_os_str().as_bytes()).map_err(io::Error::other)?;
        let to = std::ffi::CString::new(registration.as_os_str().as_bytes())
            .map_err(io::Error::other)?;
        // SAFETY: both NUL-terminated filesystem paths live through the call.
        if unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_SWAP) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        // The index can be on another volume when the owner chooses a checkout folder.
        fs::copy(stage.join("index"), index)?;
        if dest.symlink_metadata().is_ok() {
            return Err(GitError::Failed(
                "Worktree destination already exists.".to_owned(),
            ));
        }
        // Git appends the source basename when its destination is a directory.
        // Always name the existing parent, so a racing creator of the final
        // directory gets an "already exists" error instead of a nested checkout.
        let moved = git(
            repo,
            &[
                "worktree",
                "move",
                "--",
                registered,
                &parent.to_string_lossy(),
            ],
        )
        .await?;
        if moved.success() {
            Ok(())
        } else {
            Err(moved.failure())
        }
    }
    .await;
    if let Err(error) = finish {
        // Only the unpublished private directory belongs to this operation.
        let removed = git(repo, &["worktree", "remove", "--force", "--", registered]).await?;
        if !removed.success() || dest.symlink_metadata().is_ok() {
            return Err(error);
        }
        // Registration already claimed the branch. Give stock Git the final
        // destination, including its checkout filters and hooks, exactly once.
        tracing::debug!(%error,"Falling back to ordinary Git worktree creation");
        return register_checkout(
            repo,
            &dest.to_string_lossy(),
            None,
            new_branch.unwrap_or(start),
            false,
        )
        .await;
    }
    // --no-checkout defers the normal hook. It must see the final working directory.
    let head = value(dest, &["rev-parse", "HEAD"])
        .await
        .ok_or_else(|| GitError::Failed("Git HEAD unavailable.".into()))?;
    let out = git(
        dest,
        &[
            "hook",
            "run",
            "--ignore-missing",
            "post-checkout",
            "--",
            &"0".repeat(head.len()),
            &head,
            "1",
        ],
    )
    .await?;
    if out.success() {
        Ok(())
    } else {
        Err(out.failure())
    }
}

async fn use_cache(
    cache: &Cache,
    repo: &Path,
    dest: &Path,
    new_branch: Option<&str>,
    start: &str,
) -> Result<bool, GitError> {
    if !dest.is_absolute() || cache.generation().is_none() {
        return Ok(false);
    }
    let Ok(lock) = cache.lock() else {
        return Ok(false);
    };
    if lock.try_lock_shared().is_err() || cache.generation().is_none() {
        return Ok(false);
    }
    let Some(root) = super::cow::toplevel(repo).await else {
        return Ok(false);
    };
    let repo = root.as_path();
    let Some(tree) = describe(cache, repo, start).await else {
        return Ok(false);
    };
    if !tree.bucket.is_dir() {
        return Ok(false);
    }
    let Some(parent) = dest.parent() else {
        return Ok(false);
    };
    let Ok(stage) = tempfile::Builder::new()
        .prefix(".bruhs-checkout-")
        .tempdir_in(parent)
    else {
        return Ok(false);
    };
    if prepare(repo, &tree, stage.path()).await.is_none() {
        return Ok(false);
    }
    publish(cache, repo, dest, stage.path(), &tree, new_branch, start).await?;
    Ok(true)
}

pub(super) async fn try_checkout(
    repo: &Path,
    dest: &Path,
    new_branch: Option<&str>,
    start: &str,
) -> Result<bool, GitError> {
    let Ok(cache) = Cache::local() else {
        return Ok(false);
    };
    use_cache(&cache, repo, dest, new_branch, start).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::Repo;
    use std::os::unix::fs::{symlink, PermissionsExt};

    #[tokio::test]
    async fn mac_checkout_cache_matches_git_and_keeps_live_files_private() {
        const NAME: &str="git::checkout_cache::tests::mac_checkout_cache_matches_git_and_keeps_live_files_private";
        if std::env::var_os("BRUHS_CHECKOUT_CACHE_FIXTURE").is_none() {
            let home = tempfile::tempdir().unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", NAME, "--nocapture"])
                .env("BRUHS_CHECKOUT_CACHE_FIXTURE", "1")
                .env("HOME", home.path())
                .env("CFFIXED_USER_HOME", home.path())
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .status()
                .unwrap();
            assert!(status.success());
            return;
        }
        let cache = Cache::local().unwrap();
        assert!(cache.flag.starts_with(std::env::var_os("HOME").unwrap()));
        let repo = Repo::new();
        let project = repo.path();
        let dests = tempfile::tempdir().unwrap();
        fs::write(project.join("tracked.sh"), b"#!/bin/sh\n").unwrap();
        fs::set_permissions(
            project.join("tracked.sh"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        symlink("tracked.sh", project.join("link")).unwrap();
        fs::write(project.join(".gitignore"), "ignored\n").unwrap();
        fs::write(project.join("ignored"), "private").unwrap();
        // A gitlink stays an empty, uninitialized submodule in the snapshot.
        let module_commit = repo.git(&["rev-parse", "HEAD"]);
        repo.git(&[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{},module", module_commit.trim()),
        ]);
        fs::create_dir(project.join("module")).unwrap();
        repo.git(&["add", "."]);
        repo.git(&["commit", "-qm", "files"]);
        fs::write(project.join("README.md"), "private edit").unwrap();
        let hook = project.join(".git/hooks/post-checkout");
        let hook_log = dests.path().join("hooks");
        fs::write(
            &hook,
            format!(
                "#!/bin/sh\nprintf '%s %s %s %s\\n' \"$PWD\" \"$1\" \"$2\" \"$3\" >> '{}'\n",
                hook_log.display()
            ),
        )
        .unwrap();
        fs::set_permissions(hook, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!enabled());
        set_enabled(true).await.unwrap();
        let first = dests.path().join("first");
        super::super::add_worktree(project, "main", &first, "bruhs/cold")
            .await
            .unwrap();
        let tree = describe(&cache, project, "main").await.unwrap();
        tokio::time::timeout(Duration::from_secs(12), async {
            while !tree.entry().join("seal").is_file() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        let warm = dests.path().join("warm");
        assert!(
            use_cache(&cache, project, &warm, Some("bruhs/warm"), "main")
                .await
                .unwrap()
        );
        assert_eq!(
            fs::read_to_string(warm.join("README.md")).unwrap(),
            "hello\n"
        );
        assert!(!warm.join("ignored").exists());
        assert!(fs::read_dir(warm.join("module")).unwrap().next().is_none());
        assert_eq!(
            fs::read_link(warm.join("link")).unwrap(),
            Path::new("tracked.sh")
        );
        assert_ne!(
            fs::metadata(warm.join("tracked.sh")).unwrap().mode() & 0o111,
            0
        );
        let index_stat = value(&warm, &["ls-files", "--debug", "--", "README.md"])
            .await
            .unwrap();
        let inode = u32::try_from(
            warm.join("README.md").symlink_metadata().unwrap().ino() & u64::from(u32::MAX),
        )
        .unwrap();
        assert!(
            index_stat
                .lines()
                .any(|line| line.split_whitespace().last() == Some(inode.to_string().as_str())),
            "Git should trust the cloned file on the first status: {index_stat}"
        );
        assert_eq!(
            value(&warm, &["status", "--porcelain"]).await.as_deref(),
            Some("")
        );
        assert_eq!(
            value(&warm, &["branch", "--show-current"]).await.as_deref(),
            Some("bruhs/warm")
        );
        let hooks = fs::read_to_string(&hook_log).unwrap();
        assert_eq!(hooks.lines().count(), 2);
        assert!(hooks
            .lines()
            .last()
            .unwrap()
            .starts_with(warm.canonicalize().unwrap().to_str().unwrap()));
        assert!(hooks.lines().last().unwrap().ends_with(" 1"));
        // A nearby commit reuses the snapshot while Git writes changed paths.
        fs::write(project.join("new"), "second").unwrap();
        repo.git(&["rm", "link"]);
        repo.git(&["add", "new"]);
        repo.git(&["commit", "-qm", "advance"]);
        repo.git(&["branch", "existing"]);
        let near = dests.path().join("near");
        assert!(use_cache(&cache, project, &near, None, "existing")
            .await
            .unwrap());
        assert_eq!(fs::read_to_string(near.join("new")).unwrap(), "second");
        assert!(!near.join("link").exists());
        assert_eq!(
            value(&near, &["status", "--porcelain"]).await.as_deref(),
            Some("")
        );
        assert_eq!(
            value(&near, &["rev-parse", "HEAD"]).await,
            value(project, &["rev-parse", "main"]).await
        );
        // A branch moving after the clone was prepared gets a stock checkout at
        // the new commit. Its hook still sees the final path exactly once.
        let pending = tempfile::tempdir_in(dests.path()).unwrap();
        let before = describe(&cache, project, "main").await.unwrap();
        prepare(project, &before, pending.path()).await.unwrap();
        fs::write(project.join("new"), "later").unwrap();
        repo.git(&["add", "new"]);
        repo.git(&["commit", "-qm", "advance again"]);
        let moved = dests.path().join("moved-base");
        let prior_hooks = fs::read_to_string(&hook_log).unwrap().lines().count();
        publish(
            &cache,
            project,
            &moved,
            pending.path(),
            &before,
            Some("bruhs/moved"),
            "main",
        )
        .await
        .unwrap();
        assert_eq!(fs::read_to_string(moved.join("new")).unwrap(), "later");
        assert_eq!(
            fs::read_to_string(&hook_log).unwrap().lines().count(),
            prior_hooks + 1
        );
        // Local edits in one clone stay there. Altering the private cache invalidates it.
        fs::write(warm.join("README.md"), "warm only").unwrap();
        assert_eq!(
            fs::read_to_string(near.join("README.md")).unwrap(),
            "hello\n"
        );
        fs::write(tree.entry().join("files/README.md"), "bad").unwrap();
        let bad = dests.path().join("bad");
        assert!(!use_cache(&cache, project, &bad, Some("bruhs/bad"), "main")
            .await
            .unwrap());
        assert!(!bad.exists());
        // A checkout filter must run in the actual working directory.
        fs::write(project.join(".gitattributes"), "*.md filter=custom\n").unwrap();
        repo.git(&["add", ".gitattributes"]);
        repo.git(&["commit", "-qm", "attributes"]);
        assert!(
            !describe(&cache, project, "main")
                .await
                .unwrap()
                .eligible(project)
                .await
        );
        repo.git(&["config", "filter.custom.smudge", "sed s/hello/filtered/g"]);
        let filtered = dests.path().join("filtered");
        super::super::add_worktree(project, "main", &filtered, "bruhs/filtered")
            .await
            .unwrap();
        assert_eq!(
            fs::read_to_string(filtered.join("README.md")).unwrap(),
            "filtered\n"
        );
        set_enabled(false).await.unwrap();
        assert!(!enabled() && !cache.root.exists());
        assert!(warm.join("README.md").is_file() && project.join("ignored").is_file());
    }
}
