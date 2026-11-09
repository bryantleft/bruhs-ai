//! `@`-mention file search in a project: fuzzy matching over the files git
//! tracks, ranked by recent change frequency, or a directory listing when
//! the query names a directory (`src/`, `src/ma`).
//!
//! Runs in-process for any surface (bruhs-server's `/api/agent/files`, the
//! desktop composer). The git file list and the change ranking are cached
//! per repository for a few seconds, so a search per keystroke does not
//! start two `git` processes per keystroke.

use bruhs_models::num::ToFloatLossy;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::time::Instant;

use crate::git::{self, GitError};

/// Most entries one search returns.
pub const MAX_RESULTS: usize = 30;
/// How long a repository's file list and ranking are reused.
pub const INDEX_TTL: Duration = Duration::from_secs(5);
/// Commits whose changed files feed the ranking.
const HOTSPOT_COMMITS: u32 = 200;

/// One search result. `path` is relative to the searched root; directories
/// end in `/` (both `path` and `name`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub name: String,
    #[serde(rename = "isDirectory")]
    pub is_directory: bool,
}

/// Why a search has no answer (as opposed to an answer with no matches).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FileSearchError {
    /// The root does not exist or cannot be resolved.
    #[error("project directory cannot be read: {0}")]
    Root(#[source] std::io::Error),
    /// Reading a directory failed part-way.
    #[error("directory cannot be read: {0}")]
    ReadDir(#[source] std::io::Error),
    /// Fuzzy search needs the root to be inside a git work tree.
    #[error("project is not a git repository")]
    NotARepository(#[source] GitError),
    #[error("listing project files failed: {0}")]
    Git(#[source] GitError),
}

/// What fuzzy search ranks: the tracked files and how often each changed
/// recently.
struct RepoIndex {
    files: Vec<String>,
    /// Directories implied by `files` (no trailing `/`).
    dirs: Vec<String>,
    hotspots: HashMap<String, i64>,
}

struct CachedIndex {
    at: Instant,
    index: Arc<RepoIndex>,
}

/// A file searcher with its own cache. Most callers use [`search`], which
/// shares one per process.
pub struct FileSearch {
    ttl: Duration,
    cache: Mutex<HashMap<PathBuf, CachedIndex>>,
}

impl Default for FileSearch {
    fn default() -> Self {
        Self::with_ttl(INDEX_TTL)
    }
}

impl FileSearch {
    /// A searcher that reuses a repository's index for `ttl`.
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            ttl,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Up to [`MAX_RESULTS`] entries under `root` matching `query`.
    ///
    /// A query containing `/` lists the directory before its last `/`
    /// (directories first, then by name), filtered by the text after it; a
    /// directory that does not exist (still being typed) or lies outside
    /// `root` has no matches. Any other query matches, case-insensitively,
    /// the paths git tracks: up to a third of the slots go to directories,
    /// the rest to files ranked by name match, recent changes and length.
    ///
    /// # Errors
    ///
    /// [`FileSearchError`] when `root` or a listed directory cannot be read,
    /// or (fuzzy search) `root` is not in a git repository or git fails.
    pub async fn search(
        &self,
        root: &Path,
        query: &str,
    ) -> Result<Vec<FileEntry>, FileSearchError> {
        let root = tokio::fs::canonicalize(root)
            .await
            .map_err(FileSearchError::Root)?;
        if query.contains('/') {
            return list_directory(&root, query).await;
        }
        let index = self.index(&root).await?;
        Ok(rank(&index, &query.to_lowercase()))
    }

    async fn index(&self, root: &Path) -> Result<Arc<RepoIndex>, FileSearchError> {
        if let Some(cached) = self.lock().get(root) {
            if cached.at.elapsed() < self.ttl {
                return Ok(Arc::clone(&cached.index));
            }
        }
        let index = Arc::new(load_index(root).await?);
        let mut cache = self.lock();
        // Forget expired repositories so the cache cannot grow without bound.
        cache.retain(|_, cached| cached.at.elapsed() < self.ttl);
        cache.insert(
            root.to_path_buf(),
            CachedIndex {
                at: Instant::now(),
                index: Arc::clone(&index),
            },
        );
        drop(cache);
        Ok(index)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<PathBuf, CachedIndex>> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// [`FileSearch::search`] with a process-wide cache.
pub async fn search(root: &Path, query: &str) -> Result<Vec<FileEntry>, FileSearchError> {
    static SHARED: OnceLock<FileSearch> = OnceLock::new();
    SHARED
        .get_or_init(FileSearch::default)
        .search(root, query)
        .await
}

async fn load_index(root: &Path) -> Result<RepoIndex, FileSearchError> {
    let (files, changes) = tokio::join!(
        git::tracked_files(root),
        git::recently_changed_files(root, HOTSPOT_COMMITS)
    );
    let files = files.map_err(|error| match &error {
        GitError::Failed(stderr) if stderr.contains("not a git repository") => {
            FileSearchError::NotARepository(error)
        }
        _ => FileSearchError::Git(error),
    })?;
    // The ranking only orders results; without it every file still matches.
    let changes = changes.unwrap_or_else(|error| {
        tracing::debug!("no change ranking for {}: {error}", root.display());
        Vec::new()
    });
    let mut dirs = HashSet::new();
    for file in &files {
        let mut end = 0;
        while let Some(slash) = file[end..].find('/') {
            end += slash;
            dirs.insert(file[..end].to_string());
            end += 1;
        }
    }
    // Earlier (= more recent) lines score higher: `len`, `len - 1`, …, 1.
    let total = i64::try_from(changes.len()).unwrap_or(i64::MAX);
    let mut hotspots: HashMap<String, i64> = HashMap::new();
    for (file, score) in changes.into_iter().zip((1..=total).rev()) {
        *hotspots.entry(file).or_default() += score;
    }
    Ok(RepoIndex {
        files,
        dirs: dirs.into_iter().collect(),
        hotspots,
    })
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn rank(index: &RepoIndex, query: &str) -> Vec<FileEntry> {
    struct Match<'a> {
        path: &'a str,
        score: f64,
    }
    let matches = |path: &str| query.is_empty() || path.to_lowercase().contains(query);
    let name_bonus = |path: &str| {
        if !query.is_empty() && file_name(path).to_lowercase().contains(query) {
            1000.0
        } else {
            0.0
        }
    };
    let by_score =
        |a: &Match<'_>, b: &Match<'_>| b.score.total_cmp(&a.score).then_with(|| a.path.cmp(b.path));

    let mut dirs: Vec<Match<'_>> = index
        .dirs
        .iter()
        .filter(|dir| matches(dir))
        .map(|dir| Match {
            path: dir,
            score: name_bonus(dir) + (10.0 - dir.split('/').count().to_f64_lossy()) * 50.0,
        })
        .collect();
    let mut files: Vec<Match<'_>> = index
        .files
        .iter()
        .filter(|file| matches(file))
        .map(|file| Match {
            path: file,
            score: index
                .hotspots
                .get(file.as_str())
                .copied()
                .unwrap_or(0)
                .to_f64_lossy()
                + name_bonus(file)
                - file.len().to_f64_lossy() * 0.1,
        })
        .collect();
    dirs.sort_by(by_score);
    files.sort_by(by_score);

    let dir_slots = (MAX_RESULTS / 3).min(dirs.len());
    let file_slots = MAX_RESULTS - dir_slots;
    dirs.into_iter()
        .take(dir_slots)
        .map(|m| FileEntry {
            path: format!("{}/", m.path),
            name: format!("{}/", file_name(m.path)),
            is_directory: true,
        })
        .chain(files.into_iter().take(file_slots).map(|m| FileEntry {
            path: m.path.to_string(),
            name: file_name(m.path).to_string(),
            is_directory: false,
        }))
        .collect()
}

async fn list_directory(root: &Path, query: &str) -> Result<Vec<FileEntry>, FileSearchError> {
    let (dir_part, name_part) = match query.rsplit_once('/') {
        Some((dir, name)) => (dir, name),
        None => ("", query),
    };
    let target = match tokio::fs::canonicalize(root.join(dir_part)).await {
        Ok(target) => target,
        // Still being typed.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(FileSearchError::ReadDir(error)),
    };
    if !target.starts_with(root) {
        return Ok(Vec::new());
    }
    let mut entries = match tokio::fs::read_dir(&target).await {
        Ok(entries) => entries,
        // A file, not a directory: nothing below it.
        Err(error) if error.kind() == std::io::ErrorKind::NotADirectory => return Ok(Vec::new()),
        Err(error) => return Err(FileSearchError::ReadDir(error)),
    };
    let name_lower = name_part.to_lowercase();
    let mut results = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(FileSearchError::ReadDir)?
    {
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if !name_lower.is_empty() && !file_name.to_lowercase().contains(&name_lower) {
            continue;
        }
        let Ok(relative) = entry.path().strip_prefix(root).map(Path::to_path_buf) else {
            continue;
        };
        let relative = relative.to_string_lossy().into_owned();
        let is_directory = entry
            .file_type()
            .await
            .map_err(FileSearchError::ReadDir)?
            .is_dir();
        results.push(if is_directory {
            FileEntry {
                path: format!("{relative}/"),
                name: format!("{file_name}/"),
                is_directory,
            }
        } else {
            FileEntry {
                path: relative,
                name: file_name,
                is_directory,
            }
        });
    }
    results.sort_by(|a, b| {
        b.is_directory
            .cmp(&a.is_directory)
            .then_with(|| a.name.cmp(&b.name))
    });
    results.truncate(MAX_RESULTS);
    Ok(results)
}

/// How many names one Files-panel directory shows.
const CHILDREN_LIMIT: usize = 500;

const SKIP_CHILD_DIRS: &[&str] = &["node_modules", "target", "dist"];

/// Immediate children of `relative` under `root` for the Files surface.
///
/// `relative` is empty at the project root. Names starting with `.` are
/// skipped, as are `node_modules`, `target`, and `dist`. A path that leaves
/// `root` is an empty list. This does not use the git index, so a project
/// that is not a repository still lists.
///
/// # Errors
///
/// [`FileSearchError`] when `root` or the directory cannot be read.
pub async fn list_children(root: &Path, relative: &str) -> Result<Vec<FileEntry>, FileSearchError> {
    let root = tokio::fs::canonicalize(root)
        .await
        .map_err(FileSearchError::Root)?;
    let relative = relative.trim().trim_matches('/');
    if relative.split('/').any(|part| part == "..") {
        return Ok(Vec::new());
    }
    let target = if relative.is_empty() {
        root.clone()
    } else {
        match tokio::fs::canonicalize(root.join(relative)).await {
            Ok(target) => target,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(FileSearchError::ReadDir(error)),
        }
    };
    if !target.starts_with(&root) {
        return Ok(Vec::new());
    }
    let mut entries = tokio::fs::read_dir(&target)
        .await
        .map_err(FileSearchError::ReadDir)?;
    let mut results = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(FileSearchError::ReadDir)?
    {
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if file_name.starts_with('.') || SKIP_CHILD_DIRS.contains(&file_name.as_str()) {
            continue;
        }
        let path = entry.path();
        let Ok(child) = path.strip_prefix(&root) else {
            continue;
        };
        let child = child.to_string_lossy().into_owned();
        let is_directory = entry
            .file_type()
            .await
            .map_err(FileSearchError::ReadDir)?
            .is_dir();
        results.push(if is_directory {
            FileEntry {
                path: format!("{child}/"),
                name: format!("{file_name}/"),
                is_directory,
            }
        } else {
            FileEntry {
                path: child,
                name: file_name,
                is_directory,
            }
        });
    }
    results.sort_by(|a, b| {
        b.is_directory
            .cmp(&a.is_directory)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    results.truncate(CHILDREN_LIMIT);
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A git repository with `files` staged (no commit, so no user
    /// configuration is needed).
    async fn repo(files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git_in(dir.path(), &["init", "-q"]).await;
        add(dir.path(), files).await;
        dir
    }

    async fn add(root: &Path, files: &[&str]) {
        for file in files {
            let path = root.join(file);
            tokio::fs::create_dir_all(path.parent().unwrap())
                .await
                .unwrap();
            tokio::fs::write(&path, b"x").await.unwrap();
        }
        let mut args = vec!["add", "--"];
        args.extend(files);
        git_in(root, &args).await;
    }

    async fn git_in(root: &Path, args: &[&str]) {
        let status = tokio::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status()
            .await
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn paths(entries: &[FileEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.path.as_str()).collect()
    }

    #[tokio::test]
    async fn fuzzy_search_matches_tracked_paths_with_directories_first() {
        let repo = repo(&["src/main.rs", "src/lib.rs", "docs/readme.md", "Cargo.toml"]).await;
        let search = FileSearch::default();

        let found = search.search(repo.path(), "MAIN").await.unwrap();
        assert_eq!(paths(&found), ["src/main.rs"]);
        assert_eq!(found[0].name, "main.rs");

        let found = search.search(repo.path(), "src").await.unwrap();
        assert_eq!(paths(&found), ["src/", "src/lib.rs", "src/main.rs"]);
        assert!(found[0].is_directory && found[0].name == "src/");

        let all = search.search(repo.path(), "").await.unwrap();
        assert_eq!(all.len(), 6, "{all:?}");
    }

    #[tokio::test]
    async fn directory_queries_list_children_and_stay_inside_the_root() {
        let repo = repo(&["src/main.rs", "src/nested/mod.rs"]).await;
        let search = FileSearch::default();

        let found = search.search(repo.path(), "src/").await.unwrap();
        assert_eq!(paths(&found), ["src/nested/", "src/main.rs"]);
        let found = search.search(repo.path(), "src/MA").await.unwrap();
        assert_eq!(paths(&found), ["src/main.rs"]);
        // Still being typed, a file, or outside the root: no matches.
        for query in ["missing/", "src/main.rs/", "../", "src/../../"] {
            assert!(
                search.search(repo.path(), query).await.unwrap().is_empty(),
                "{query}"
            );
        }
    }

    /// Failures are errors, not an empty list that reads as "no matches".
    #[tokio::test]
    async fn failures_are_reported() {
        let search = FileSearch::default();
        let plain = tempfile::tempdir().unwrap();
        assert!(matches!(
            search.search(plain.path(), "x").await,
            Err(FileSearchError::NotARepository(_))
        ));
        assert!(matches!(
            search.search(&plain.path().join("gone"), "x").await,
            Err(FileSearchError::Root(_))
        ));
    }

    /// Per-keystroke searches reuse the index until it expires.
    #[tokio::test]
    async fn the_index_is_reused_within_its_ttl() {
        let repo = repo(&["first.rs"]).await;
        let cached = FileSearch::with_ttl(Duration::from_secs(3600));
        let fresh = FileSearch::with_ttl(Duration::ZERO);
        assert_eq!(
            paths(&cached.search(repo.path(), "").await.unwrap()),
            ["first.rs"]
        );

        add(repo.path(), &["second.rs"]).await;
        assert_eq!(
            paths(&cached.search(repo.path(), "").await.unwrap()),
            ["first.rs"],
            "served from the cache"
        );
        assert_eq!(
            paths(&fresh.search(repo.path(), "").await.unwrap()),
            ["first.rs", "second.rs"]
        );
    }

    #[test]
    fn ranking_prefers_name_matches_then_recent_changes() {
        let index = RepoIndex {
            files: vec![
                "a/zz_config.rs".into(),
                "config/x.rs".into(),
                "b/config.rs".into(),
            ],
            dirs: vec!["a".into(), "b".into(), "config".into()],
            hotspots: HashMap::from([("b/config.rs".to_string(), 50)]),
        };
        let found = rank(&index, "config");
        assert_eq!(
            paths(&found),
            ["config/", "b/config.rs", "a/zz_config.rs", "config/x.rs"]
        );
    }

    #[test]
    fn entries_keep_the_web_wire_shape() {
        let entry = FileEntry {
            path: "src/".into(),
            name: "src/".into(),
            is_directory: true,
        };
        assert_eq!(
            serde_json::to_value(&entry).unwrap(),
            serde_json::json!({"path": "src/", "name": "src/", "isDirectory": true})
        );
    }

    #[tokio::test]
    async fn list_children_skips_hidden_and_heavy_directories() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        tokio::fs::create_dir_all(root.join("src")).await.unwrap();
        tokio::fs::write(root.join("src/lib.rs"), b"x")
            .await
            .unwrap();
        tokio::fs::write(root.join("readme.md"), b"x")
            .await
            .unwrap();
        tokio::fs::write(root.join(".secret"), b"x").await.unwrap();
        tokio::fs::create_dir_all(root.join("node_modules/pkg"))
            .await
            .unwrap();

        let listed = list_children(root, "").await.unwrap();
        assert_eq!(paths(&listed), ["src/", "readme.md"]);
        let nested = list_children(root, "src").await.unwrap();
        assert_eq!(paths(&nested), ["src/lib.rs"]);
        assert!(list_children(root, "src/../..").await.unwrap().is_empty());
    }
}
