//! Reusing the project checkout's build folders in a new worktree through
//! copy-on-write clones, so an agent's first build or install starts warm
//! and many agent worktrees cost little more disk than one.
//!
//! Git checks the worktree out as usual first; only ignored build output is
//! cloned (see `docs/worktree-disk.md`):
//!
//! - Cargo `target/debug` beside each tracked `Cargo.toml`, without the
//!   path-keyed `incremental/`, and never while a build holds `.cargo-lock`.
//! - `node_modules` beside each tracked `package.json`, when that manifest
//!   and the lockfiles match between the checkout and the worktree.
//! - Folders the project configures.
//!
//! A filesystem that cannot clone makes this a no-op; nothing is ever copied
//! byte for byte.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use bruhs_models::delivery::{ProjectScripts, WorktreeConfig};

use super::clone_fs::clone_path;
use super::GitCommand;

/// Detected build folders considered per worktree.
const MAX_DETECTED_BUILD_DIRS: usize = 64;
/// Bytes of `.bruhs/config.json` read; the delivery budget is the same.
const CONFIG_LIMIT: u64 = 64 * 1024;
/// Maximum bytes read when comparing `package.json` or lockfiles.
const LOCKFILE_LIMIT: u64 = 4 * 1024 * 1024;
/// Lockfiles that pin what `node_modules` holds.
const NODE_LOCKFILES: [&str; 6] = [
    "package-lock.json",
    "npm-shrinkwrap.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "bun.lock",
    "bun.lockb",
];

/// The repository root of `repo` (which may name a subdirectory).
pub(super) async fn toplevel(repo: &Path) -> Option<PathBuf> {
    let out = GitCommand::new(repo, &["rev-parse", "--show-toplevel"])
        .run()
        .await
        .ok()?;
    out.success()
        .then(|| PathBuf::from(out.stdout_lossy().trim()))
        .filter(|path| !path.as_os_str().is_empty())
}

/// The project's `worktree` settings. Defaults when the file is absent;
/// copy-on-write off when it exists but is not a readable regular file of at
/// most [`CONFIG_LIMIT`] bytes or does not parse, so a typo never turns on
/// what the owner meant to turn off.
pub(super) async fn options(root: &Path) -> WorktreeConfig {
    let bruhs_dir = root.join(".bruhs");
    let file = bruhs_dir.join("config.json");
    let read_result = tokio::task::spawn_blocking(move || {
        match std::fs::symlink_metadata(&bruhs_dir) {
            Ok(meta) if meta.file_type().is_dir() => {}
            Ok(_) => {
                return Err("not a regular .bruhs directory".to_string());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(error) => return Err(error.to_string()),
        }
        match std::fs::symlink_metadata(&file) {
            Ok(meta) if !meta.file_type().is_file() || meta.len() > CONFIG_LIMIT => {
                return Err("not a regular file within 64 KiB".to_string());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        }
        match crate::workspace_trust::read_regular_file_bounded(&file, CONFIG_LIMIT) {
            Ok(Some(bytes)) => Ok(Some(bytes)),
            Ok(None) => Err("not a regular file within 64 KiB".to_string()),
            Err(error) => Err(error.to_string()),
        }
    })
    .await;
    let bytes = match read_result {
        Ok(Ok(Some(bytes))) => bytes,
        Ok(Ok(None)) => return WorktreeConfig::default(),
        Ok(Err(why)) => return disabled(&root.join(".bruhs/config.json"), &why),
        Err(error) => return disabled(&root.join(".bruhs/config.json"), &error.to_string()),
    };
    match serde_json::from_slice::<ProjectScripts>(&bytes) {
        Ok(scripts) => scripts.worktree,
        Err(error) => disabled(&root.join(".bruhs/config.json"), &error.to_string()),
    }
}

fn disabled(file: &Path, why: &str) -> WorktreeConfig {
    tracing::warn!(
        "{} cannot be used ({why}); new worktrees reuse no build folders",
        file.display()
    );
    WorktreeConfig {
        copy_on_write: false,
        build_dirs: Vec::new(),
    }
}

/// How a build folder is reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum BuildDirKind {
    /// Cargo `target/`: only `debug`, without `incremental` (keyed by the
    /// checkout's path, so useless elsewhere), and never while a build holds
    /// `.cargo-lock`.
    Cargo,
    /// `node_modules`, only while the manifest beside it and the lockfiles
    /// match, so the worktree never runs against other dependencies.
    Node,
    /// A configured folder, cloned whole.
    Configured,
}

/// Build folders next to tracked manifests: `target` beside each
/// `Cargo.toml`, `node_modules` beside each `package.json`, then the
/// configured folders. Manifests inside such folders are skipped.
pub(super) fn detect_build_dirs(
    tracked: &[String],
    configured: &[&str],
) -> Vec<(String, BuildDirKind)> {
    let mut found = BTreeSet::new();
    for path in tracked {
        let (dir, name) = match path.rsplit_once('/') {
            Some((dir, name)) => (Some(dir), name),
            None => (None, path.as_str()),
        };
        let inside_output = dir.is_some_and(|dir| {
            dir.split('/')
                .any(|segment| segment == "node_modules" || segment == "target")
        });
        if inside_output {
            continue;
        }
        let (folder, kind) = match name {
            "Cargo.toml" => ("target", BuildDirKind::Cargo),
            "package.json" => ("node_modules", BuildDirKind::Node),
            _ => continue,
        };
        let candidate = dir.map_or_else(|| folder.to_string(), |dir| format!("{dir}/{folder}"));
        found.insert((candidate, kind));
        if found.len() >= MAX_DETECTED_BUILD_DIRS {
            break;
        }
    }
    let mut dirs: Vec<_> = found.into_iter().collect();
    for dir in configured {
        if !dirs.iter().any(|(existing, _)| existing == dir) {
            dirs.push(((*dir).to_string(), BuildDirKind::Configured));
        }
    }
    dirs
}

/// Clone the project checkout's build folders into the worktree at `dest`.
/// Only folders Git ignores in the worktree, that exist as real directories
/// in the checkout and are absent from the worktree, and that no symlinked
/// folder leads to on either side, are cloned.
pub(super) async fn seed_build_dirs(
    root: &Path,
    dest: &Path,
    config: &WorktreeConfig,
) -> Vec<String> {
    let tracked = match super::tracked_files(dest).await {
        Ok(tracked) => tracked,
        Err(error) => {
            tracing::debug!("no build folders cloned into {}: {error}", dest.display());
            return Vec::new();
        }
    };
    let candidates: Vec<_> = detect_build_dirs(&tracked, &config.valid_build_dirs())
        .into_iter()
        .filter(|(dir, _)| {
            inside_without_symlinks(root, dir)
                && inside_without_symlinks(dest, dir)
                && std::fs::symlink_metadata(root.join(dir)).is_ok_and(|meta| meta.is_dir())
                && std::fs::symlink_metadata(dest.join(dir)).is_err()
        })
        .collect();
    if candidates.is_empty() {
        return Vec::new();
    }
    let ignored = ignored_dirs(dest, candidates.iter().map(|(dir, _)| dir.as_str())).await;
    let wanted: Vec<_> = candidates
        .into_iter()
        .filter(|(dir, _)| ignored.contains(dir))
        .collect();
    let (from, to) = (root.to_path_buf(), dest.to_path_buf());
    tokio::task::spawn_blocking(move || {
        let mut seeded = Vec::new();
        for (dir, kind) in wanted {
            let (src, dst) = (from.join(&dir), to.join(&dir));
            let result = match kind {
                BuildDirKind::Cargo => clone_cargo_target(&src, &dst),
                BuildDirKind::Node if !node_inputs_match(&from, &to, &dir) => Ok(false),
                BuildDirKind::Node | BuildDirKind::Configured => {
                    clone_path(&src, &dst).map(|()| true)
                }
            };
            match result {
                Ok(true) => seeded.push(dir),
                Ok(false) => {}
                Err(error) if error.kind() == std::io::ErrorKind::Unsupported => break,
                Err(error) => tracing::debug!("did not clone {dir}: {error}"),
            }
        }
        seeded
    })
    .await
    .unwrap_or_default()
}

/// `node_modules` at `dir` reflects the same dependencies in both places:
/// the `package.json` beside it and every lockfile beside it or at the
/// repository root are byte-identical (or absent on both sides).
pub(super) fn node_inputs_match(root: &Path, dest: &Path, dir: &str) -> bool {
    let parent = dir.rsplit_once('/').map(|(parent, _)| parent);
    let beside = |name: &str| parent.map_or_else(|| name.to_string(), |p| format!("{p}/{name}"));
    let mut inputs = vec![beside("package.json")];
    for lockfile in NODE_LOCKFILES {
        inputs.push(beside(lockfile));
        inputs.push(lockfile.to_string());
    }
    inputs.iter().all(|file| {
        let a_path = root.join(file);
        let b_path = dest.join(file);
        let a_missing = std::fs::symlink_metadata(&a_path)
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound);
        let b_missing = std::fs::symlink_metadata(&b_path)
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound);
        if a_missing && b_missing {
            return true;
        }
        if a_missing || b_missing {
            return false;
        }
        match (
            crate::workspace_trust::read_regular_file_bounded(&a_path, LOCKFILE_LIMIT),
            crate::workspace_trust::read_regular_file_bounded(&b_path, LOCKFILE_LIMIT),
        ) {
            (Ok(Some(a)), Ok(Some(b))) => a == b,
            _ => false,
        }
    })
}

/// The subset of `dirs` that Git ignores in `dest`.
async fn ignored_dirs<'a>(dest: &Path, dirs: impl Iterator<Item = &'a str>) -> BTreeSet<String> {
    let mut stdin = Vec::new();
    for dir in dirs {
        // A trailing slash lets directory-only patterns (`node_modules/`)
        // match a folder that does not exist in `dest` yet.
        stdin.extend_from_slice(dir.as_bytes());
        stdin.extend_from_slice(b"/\0");
    }
    let Ok(out) = GitCommand::new(dest, &["check-ignore", "-z", "--stdin"])
        .stdin(stdin)
        .run()
        .await
    else {
        return BTreeSet::new();
    };
    // Exit 1 means nothing is ignored; anything else but 0 is an error.
    if out
        .status
        .and_then(|status| status.code())
        .is_none_or(|code| code > 1)
    {
        return BTreeSet::new();
    }
    out.stdout
        .split(|byte| *byte == 0)
        .filter_map(|path| std::str::from_utf8(path).ok())
        .map(|path| path.trim_end_matches('/').to_string())
        .filter(|path| !path.is_empty())
        .collect()
}

/// Clone Cargo's `debug` profile from `src` (a `target/` folder) into a new
/// `dst`. `Ok(false)` when there is nothing to clone or a build is running.
fn clone_cargo_target(src: &Path, dst: &Path) -> std::io::Result<bool> {
    let debug = src.join("debug");
    if !std::fs::symlink_metadata(&debug).is_ok_and(|meta| meta.is_dir()) {
        return Ok(false);
    }
    // Every Cargo build, cross builds included (their host artifacts live
    // in `debug`), holds `debug/.cargo-lock`. Holding it here keeps builds
    // from writing while the clone is taken. No file means no build ran.
    let _lock = match std::fs::File::open(debug.join(".cargo-lock")) {
        Ok(file) => match file.try_lock() {
            Ok(()) => Some(file),
            Err(std::fs::TryLockError::WouldBlock) => {
                tracing::info!(
                    "a Cargo build is running in {}; not cloning it",
                    src.display()
                );
                return Ok(false);
            }
            Err(std::fs::TryLockError::Error(error)) => return Err(error),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    std::fs::create_dir(dst)?;
    let result = (|| {
        for name in [".rustc_info.json", "CACHEDIR.TAG"] {
            let file = src.join(name);
            if file.is_file() {
                clone_path(&file, &dst.join(name))?;
            }
        }
        let dst_debug = dst.join("debug");
        std::fs::create_dir(&dst_debug)?;
        for entry in std::fs::read_dir(&debug)? {
            let entry = entry?;
            let name = entry.file_name();
            if name == "incremental" || name == ".cargo-lock" {
                continue;
            }
            clone_path(&entry.path(), &dst_debug.join(&name))?;
        }
        Ok(true)
    })();
    if result.is_err() {
        if let Err(error) = std::fs::remove_dir_all(dst) {
            tracing::debug!(
                "could not remove a partial clone {}: {error}",
                dst.display()
            );
        }
    }
    result
}

/// Every folder above `rel` inside `base` is a real directory rather than a
/// symlink, so `base/rel` cannot resolve outside `base`.
pub(super) fn inside_without_symlinks(base: &Path, rel: &str) -> bool {
    let mut path = base.to_path_buf();
    let mut segments = rel.split('/').peekable();
    while let Some(segment) = segments.next() {
        if segments.peek().is_none() {
            break;
        }
        path.push(segment);
        if !std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_dir()) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A configured `web/node_modules` must not follow a committed `web`
    /// symlink out of the worktree.
    #[cfg(unix)]
    #[test]
    fn symlinked_parents_are_refused() {
        let base = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(base.path().join("real/node_modules")).unwrap();
        std::os::unix::fs::symlink("/tmp", base.path().join("web")).unwrap();
        assert!(inside_without_symlinks(base.path(), "real/node_modules"));
        assert!(inside_without_symlinks(base.path(), "node_modules"));
        assert!(!inside_without_symlinks(base.path(), "web/node_modules"));
        assert!(!inside_without_symlinks(
            base.path(),
            "missing/node_modules"
        ));
    }

    #[test]
    fn build_dirs_sit_beside_manifests_and_skip_output_folders() {
        let tracked = [
            "Cargo.toml",
            "crates/a/Cargo.toml",
            "package.json",
            "web/package.json",
            "web/node_modules/pkg/package.json",
            "target/debug/Cargo.toml",
            "README.md",
        ]
        .map(String::from);
        assert_eq!(
            detect_build_dirs(&tracked, &["vendor/bundle", "web/node_modules"]),
            [
                ("crates/a/target".to_string(), BuildDirKind::Cargo),
                ("node_modules".to_string(), BuildDirKind::Node),
                ("target".to_string(), BuildDirKind::Cargo),
                ("web/node_modules".to_string(), BuildDirKind::Node),
                ("vendor/bundle".to_string(), BuildDirKind::Configured),
            ]
        );
    }

    /// A worktree on another branch whose lockfile or manifest moved on gets
    /// no `node_modules`, so it never runs against stale dependencies.
    #[test]
    fn node_modules_needs_matching_manifest_and_lockfiles() {
        let (root, dest) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let write = |base: &Path, file: &str, text: &str| {
            let path = base.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        for base in [root.path(), dest.path()] {
            write(base, "pnpm-lock.yaml", "lock: 1\n");
            write(base, "web/package.json", "{}\n");
        }
        assert!(node_inputs_match(
            root.path(),
            dest.path(),
            "web/node_modules"
        ));

        write(dest.path(), "pnpm-lock.yaml", "lock: 2\n");
        assert!(!node_inputs_match(
            root.path(),
            dest.path(),
            "web/node_modules"
        ));
        write(dest.path(), "pnpm-lock.yaml", "lock: 1\n");

        write(root.path(), "web/package-lock.json", "{}\n");
        assert!(!node_inputs_match(
            root.path(),
            dest.path(),
            "web/node_modules"
        ));
    }
}
