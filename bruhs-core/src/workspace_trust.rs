//! Host-wide Workspace Trust store and passive repository risk scanner.
//!
//! Shared atomically by `bruhs-server` and `bruhs-desktop` via a lock-guarded
//! JSON file in the app data directory (`BRUHS_WORKSPACE_TRUST_FILE` overrides
//! the path for tests and isolated profiles).
//!
//! Inspection is strictly read-only, never spawns any subprocess, and never
//! follows symlinks.

use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use bruhs_models::workspace_trust::{WorkspaceRiskItem, WorkspaceRiskKind, WorkspaceTrustStatus};
use serde::{Deserialize, Serialize};

/// Overrides where trusted workspace paths are persisted (tests / profiles).
const STORE_ENV: &str = "BRUHS_WORKSPACE_TRUST_FILE";
const STORE_FILE_NAME: &str = "trusted-workspaces.json";
const MAX_CONFIG_BYTES: u64 = 64 * 1024;
const MAX_STORE_BYTES: u64 = 1024 * 1024;
const MAX_HOOK_ENTRIES: usize = 64;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct TrustStore {
    trusted_paths: BTreeSet<String>,
}

/// Default host-shared trust file path.
pub fn default_store_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(STORE_ENV).filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path));
    }
    directories::ProjectDirs::from("dev", "bruhs", "bruhs-ai")
        .map(|dirs| dirs.data_dir().join(STORE_FILE_NAME))
}

/// Whether `path` (or the verified main checkout of a linked worktree) is trusted.
/// Missing or unreadable trust stores fail closed (`false`).
pub fn is_trusted(path: &Path) -> bool {
    let Some(store) = default_store_path() else {
        return false;
    };
    is_trusted_at(&store, path)
}

/// Inspect `path` passively without executing any subprocess and report both
/// its trust state and any detected executable repository configuration.
pub fn inspect(path: &Path) -> WorkspaceTrustStatus {
    match default_store_path() {
        Some(store) => inspect_at(&store, path),
        None => inspect_with_trusted(path, false),
    }
}

/// Mark `path` as trusted or untrusted in the host store and return its
/// refreshed status.
pub fn set_trusted(path: &Path, trusted: bool) -> io::Result<WorkspaceTrustStatus> {
    let store = default_store_path()
        .ok_or_else(|| io::Error::other("Workspace trust storage folder is unavailable."))?;
    set_trusted_at(&store, path, trusted)
}

/// List all trusted canonical paths stored on this host.
pub fn list_trusted() -> io::Result<Vec<String>> {
    let store = default_store_path()
        .ok_or_else(|| io::Error::other("Workspace trust storage folder is unavailable."))?;
    Ok(load_store(&store)?.trusted_paths.into_iter().collect())
}

/// Remove an exact entry from the trust list, even if the folder no longer exists.
pub fn revoke_trusted(path: &str) -> io::Result<Vec<String>> {
    let store = default_store_path()
        .ok_or_else(|| io::Error::other("Workspace trust storage folder is unavailable."))?;
    revoke_trusted_at(&store, path)
}

fn revoke_trusted_at(store_path: &Path, path: &str) -> io::Result<Vec<String>> {
    let store = update_store(store_path, |store| {
        store.trusted_paths.remove(path);
    })?;
    Ok(store.trusted_paths.into_iter().collect())
}

pub(crate) fn is_trusted_at(store_path: &Path, project: &Path) -> bool {
    if project.as_os_str().is_empty() {
        return false;
    }
    let Ok(canonical) = fs::canonicalize(project) else {
        return false;
    };
    let Ok(store) = load_store(store_path) else {
        return false;
    };
    if store
        .trusted_paths
        .contains(canonical.to_string_lossy().as_ref())
    {
        return true;
    }
    main_checkout_no_exec(&canonical).is_some_and(|main| {
        store
            .trusted_paths
            .contains(main.to_string_lossy().as_ref())
    })
}

pub(crate) fn inspect_at(store_path: &Path, project: &Path) -> WorkspaceTrustStatus {
    let trusted = is_trusted_at(store_path, project);
    inspect_with_trusted(project, trusted)
}

pub(crate) fn set_trusted_at(
    store_path: &Path,
    project: &Path,
    trusted: bool,
) -> io::Result<WorkspaceTrustStatus> {
    let canonical = fs::canonicalize(project)?;
    if !canonical.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Workspace path must be an existing directory.",
        ));
    }
    let key = canonical.to_string_lossy().into_owned();
    update_store(store_path, |store| {
        if trusted {
            store.trusted_paths.insert(key);
        } else {
            store.trusted_paths.remove(&key);
        }
    })?;
    Ok(inspect_at(store_path, &canonical))
}

fn update_store(store_path: &Path, update: impl FnOnce(&mut TrustStore)) -> io::Result<TrustStore> {
    let dir = store_path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(store_path.with_extension("lock"))?;
    lock.lock()?;
    let mut store = load_store(store_path)?;
    update(&mut store);
    let mut pending = tempfile::NamedTempFile::new_in(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        pending
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    serde_json::to_writer_pretty(&mut pending, &store).map_err(io::Error::other)?;
    pending.as_file().sync_all()?;
    pending.persist(store_path).map_err(|error| error.error)?;
    Ok(store)
}

fn load_store(store_path: &Path) -> io::Result<TrustStore> {
    let bytes = match read_regular_file_bounded(store_path, MAX_STORE_BYTES) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Ok(TrustStore::default()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(TrustStore::default()),
        Err(error) => return Err(error),
    };
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

fn inspect_with_trusted(project: &Path, trusted: bool) -> WorkspaceTrustStatus {
    let canonical = fs::canonicalize(project).unwrap_or_else(|_| project.to_path_buf());
    let path_str = canonical.to_string_lossy().into_owned();
    if !canonical.is_dir() {
        return WorkspaceTrustStatus {
            path: path_str,
            trusted,
            risks: Vec::new(),
        };
    }
    let mut risks = Vec::new();
    scan_git_risks(&canonical, &mut risks);
    scan_repo_config_risks(&canonical, &mut risks);
    WorkspaceTrustStatus {
        path: path_str,
        trusted,
        risks,
    }
}

/// Read a regular, non-symlink file up to `max_bytes`, never blocking on FIFOs
/// or devices. Returns `Ok(None)` when absent or not a regular file.
pub(crate) fn read_regular_file_bounded(
    path: &Path,
    max_bytes: u64,
) -> io::Result<Option<Vec<u8>>> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !meta.file_type().is_file() || meta.len() > max_bytes {
        return Ok(None);
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let fd_meta = file.metadata()?;
    if !fd_meta.is_file() || fd_meta.len() > max_bytes {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max_bytes {
        return Ok(None);
    }
    Ok(Some(bytes))
}

/// Check that every directory component in `rel` under `base` is a real
/// directory (not a symlink) and the final target is a regular file (or dir).
fn is_real_file_under(base: &Path, rel: &[&str]) -> bool {
    let Some((file_name, dirs)) = rel.split_last() else {
        return false;
    };
    let mut cur = base.to_path_buf();
    for dir in dirs {
        cur.push(dir);
        if !fs::symlink_metadata(&cur).is_ok_and(|m| m.file_type().is_dir()) {
            return false;
        }
    }
    cur.push(file_name);
    fs::symlink_metadata(&cur).is_ok_and(|m| m.file_type().is_file())
}

fn is_real_dir_under(base: &Path, rel: &[&str]) -> bool {
    let mut cur = base.to_path_buf();
    for dir in rel {
        cur.push(dir);
        if !fs::symlink_metadata(&cur).is_ok_and(|m| m.file_type().is_dir()) {
            return false;
        }
    }
    true
}

fn scan_repo_config_risks(root: &Path, risks: &mut Vec<WorkspaceRiskItem>) {
    for rel in [
        &[".claude", "settings.json"][..],
        &[".claude", "settings.local.json"][..],
    ] {
        if is_real_file_under(root, rel) {
            risks.push(WorkspaceRiskItem {
                kind: WorkspaceRiskKind::ClaudeConfig,
                label: WorkspaceRiskKind::ClaudeConfig.short_label().into(),
                detail: rel.join("/"),
            });
        }
    }
    if is_real_file_under(root, &[".mcp.json"]) {
        risks.push(WorkspaceRiskItem {
            kind: WorkspaceRiskKind::McpConfig,
            label: WorkspaceRiskKind::McpConfig.short_label().into(),
            detail: ".mcp.json".into(),
        });
    }
    for rel in [&["opencode.json"][..], &["opencode.jsonc"][..]] {
        if is_real_file_under(root, rel) {
            risks.push(WorkspaceRiskItem {
                kind: WorkspaceRiskKind::OpenCodeConfig,
                label: WorkspaceRiskKind::OpenCodeConfig.short_label().into(),
                detail: rel.join("/"),
            });
        }
    }
    if is_real_dir_under(root, &[".opencode"]) {
        risks.push(WorkspaceRiskItem {
            kind: WorkspaceRiskKind::OpenCodeConfig,
            label: WorkspaceRiskKind::OpenCodeConfig.short_label().into(),
            detail: ".opencode/".into(),
        });
    }
    for rel in [
        &[".codex", "config.toml"][..],
        &[".cursor", "mcp.json"][..],
        &[".cursor", "cli-config.json"][..],
    ] {
        if is_real_file_under(root, rel) {
            risks.push(WorkspaceRiskItem {
                kind: WorkspaceRiskKind::AgentConfig,
                label: WorkspaceRiskKind::AgentConfig.short_label().into(),
                detail: rel.join("/"),
            });
        }
    }
    if is_real_file_under(root, &[".bruhs", "config.json"]) {
        risks.push(WorkspaceRiskItem {
            kind: WorkspaceRiskKind::ProjectScripts,
            label: WorkspaceRiskKind::ProjectScripts.short_label().into(),
            detail: ".bruhs/config.json".into(),
        });
    }
    if is_real_file_under(root, &[".gitmodules"]) {
        risks.push(WorkspaceRiskItem {
            kind: WorkspaceRiskKind::GitSubmodules,
            label: WorkspaceRiskKind::GitSubmodules.short_label().into(),
            detail: ".gitmodules".into(),
        });
    }
}

fn scan_git_risks(root: &Path, risks: &mut Vec<WorkspaceRiskItem>) {
    let Some(common_git) = common_git_dir_no_exec(root) else {
        return;
    };
    let config_path = common_git.join("config");
    if let Ok(Some(bytes)) = read_regular_file_bounded(&config_path, MAX_CONFIG_BYTES) {
        let text = String::from_utf8_lossy(&bytes);
        for key in dangerous_git_config_keys(&text) {
            risks.push(WorkspaceRiskItem {
                kind: WorkspaceRiskKind::GitConfig,
                label: WorkspaceRiskKind::GitConfig.short_label().into(),
                detail: key,
            });
        }
    }
    let hooks_dir = common_git.join("hooks");
    if fs::symlink_metadata(&hooks_dir).is_ok_and(|m| m.file_type().is_dir()) {
        if let Ok(entries) = fs::read_dir(&hooks_dir) {
            let mut hook_names = Vec::new();
            for entry in entries.flatten().take(MAX_HOOK_ENTRIES) {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.ends_with(".sample") || name.starts_with('.') {
                    continue;
                }
                if entry.file_type().is_ok_and(|ft| ft.is_file()) {
                    hook_names.push(name);
                }
            }
            hook_names.sort();
            for hook in hook_names {
                risks.push(WorkspaceRiskItem {
                    kind: WorkspaceRiskKind::GitHooks,
                    label: WorkspaceRiskKind::GitHooks.short_label().into(),
                    detail: format!(".git/hooks/{hook}"),
                });
            }
        }
    }
}

/// Pure-Rust scanner over `.git/config` lines that flags executable or
/// config-inclusion directives without spawning `git`.
fn dangerous_git_config_keys(config_text: &str) -> Vec<String> {
    let mut found = BTreeSet::new();
    let mut section = String::new();
    for raw_line in config_text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            if let Some(end) = rest.find(']') {
                let header = rest[..end].trim();
                section = header
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        let is_false = matches!(
            value.to_ascii_lowercase().as_str(),
            "false" | "0" | "no" | "off" | "never"
        );
        let risky = match (section.as_str(), key.as_str()) {
            ("core", "fsmonitor") if !is_false => Some("core.fsmonitor"),
            ("core", "hookspath") => Some("core.hooksPath"),
            ("core", "sshcommand") => Some("core.sshCommand"),
            ("core", "gitproxy") => Some("core.gitProxy"),
            ("core", "askpass") => Some("core.askPass"),
            ("core", "pager") => Some("core.pager"),
            ("diff", "external") => Some("diff.external"),
            ("diff", "textconv") => Some("diff.*.textconv"),
            ("diff", "command") => Some("diff.*.command"),
            ("filter", "clean" | "smudge" | "process") => Some("filter.*"),
            ("credential", "helper") => Some("credential.helper"),
            ("remote", "uploadpack" | "receivepack") => Some("remote.*.uploadpack"),
            ("protocol", "allow") if !is_false => Some("protocol.*.allow"),
            ("include" | "includeif", "path") => Some("include.path"),
            _ => None,
        };
        if let Some(label) = risky {
            found.insert(label.to_string());
        }
    }
    found.into_iter().collect()
}

fn read_git_pointer(path: &Path) -> Option<String> {
    let bytes = read_regular_file_bounded(path, 16 * 1024).ok()??;
    String::from_utf8(bytes).ok()
}

/// Fast passive check (zero subprocesses): returns `false` only when `.git/config`
/// is a regular file with no `filter`, `diff`, `include`/`includeIf`, or
/// transport/credential directives that require `-c` neutralization.
pub(crate) fn repo_config_may_need_neutralizers(dir: &Path, include_transport: bool) -> bool {
    let Some(common_git) = common_git_dir_no_exec(dir) else {
        // Could be a subdirectory of a repo; check parent chain passively up to 16 levels.
        let mut cursor = dir.parent();
        let mut depth = 0usize;
        let mut found = None;
        while let Some(parent) = cursor {
            if depth >= 16 {
                return true;
            }
            if let Some(git_dir) = common_git_dir_no_exec(parent) {
                found = Some(git_dir);
                break;
            }
            cursor = parent.parent();
            depth += 1;
        }
        let Some(git_dir) = found else {
            return false;
        };
        return config_file_may_need_neutralizers(&git_dir.join("config"), include_transport);
    };
    config_file_may_need_neutralizers(&common_git.join("config"), include_transport)
}

fn config_file_may_need_neutralizers(config_path: &Path, include_transport: bool) -> bool {
    let Ok(Some(bytes)) = read_regular_file_bounded(config_path, MAX_CONFIG_BYTES) else {
        return true;
    };
    let text = String::from_utf8_lossy(&bytes);
    let keys = dangerous_git_config_keys(&text);
    keys.iter().any(|k| {
        matches!(
            k.as_str(),
            "filter.*" | "diff.*.textconv" | "diff.*.command" | "include.path"
        ) || (include_transport
            && matches!(
                k.as_str(),
                "core.sshCommand"
                    | "core.gitProxy"
                    | "core.askPass"
                    | "credential.helper"
                    | "remote.*.uploadpack"
            ))
    })
}

/// Resolve `.git` (directory or linked worktree pointer) without invoking `git`.
fn common_git_dir_no_exec(folder: &Path) -> Option<PathBuf> {
    let dot = folder.join(".git");
    let meta = fs::symlink_metadata(&dot).ok()?;
    if meta.file_type().is_dir() {
        return Some(dot);
    }
    if !meta.file_type().is_file() {
        return None;
    }
    let pointer = read_git_pointer(&dot)?;
    let admin = folder
        .join(pointer.trim_end().strip_prefix("gitdir: ")?)
        .canonicalize()
        .ok()?;
    if let Some(common_rel) = read_git_pointer(&admin.join("commondir")) {
        let common = admin.join(common_rel.trim_end()).canonicalize().ok()?;
        if common.is_dir() {
            return Some(common);
        }
    }
    admin.is_dir().then_some(admin)
}

/// Resolve a linked worktree's main checkout without starting Git. Confirms
/// reciprocal registration so a copied `.git` pointer cannot spoof trust.
fn main_checkout_no_exec(folder: &Path) -> Option<PathBuf> {
    let dot = folder.join(".git");
    let pointer = read_git_pointer(&dot)?;
    let admin = folder
        .join(pointer.trim_end().strip_prefix("gitdir: ")?)
        .canonicalize()
        .ok()?;
    let common = admin
        .join(read_git_pointer(&admin.join("commondir"))?.trim_end())
        .canonicalize()
        .ok()?;
    if common.file_name()? != ".git" || admin.parent()? != common.join("worktrees") {
        return None;
    }
    let backlink = admin
        .join(read_git_pointer(&admin.join("gitdir"))?.trim_end())
        .canonicalize()
        .ok()?;
    (backlink == dot)
        .then(|| common.parent().map(Path::to_owned))
        .flatten()
}

/// Whether `project` has executable external-agent configs that cannot be
/// isolated via CLI flags alone.
pub(crate) fn has_unisolatable_external_agent_config(project: &Path) -> Option<String> {
    let status = inspect(project);
    if status.trusted {
        return None;
    }
    let details: Vec<&str> = status
        .risks
        .iter()
        .filter(|risk| {
            matches!(
                risk.kind,
                WorkspaceRiskKind::McpConfig
                    | WorkspaceRiskKind::AgentConfig
                    | WorkspaceRiskKind::OpenCodeConfig
            )
        })
        .map(|risk| risk.detail.as_str())
        .collect();
    (!details.is_empty()).then(|| details.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_store_round_trips_and_defaults_untrusted() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("trusted-workspaces.json");
        let project = tmp.path().join("repo");
        fs::create_dir_all(&project).unwrap();

        assert!(!is_trusted_at(&store, &project));
        let status = set_trusted_at(&store, &project, true).unwrap();
        assert!(status.trusted);
        assert!(is_trusted_at(&store, &project));

        let revoked = set_trusted_at(&store, &project, false).unwrap();
        assert!(!revoked.trusted);
        assert!(!is_trusted_at(&store, &project));
    }

    #[test]
    fn revoking_a_missing_folder_removes_only_its_exact_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("trusted-workspaces.json");
        let missing = tmp.path().join("missing");
        let kept = tmp.path().join("kept");
        fs::create_dir(&missing).unwrap();
        fs::create_dir(&kept).unwrap();
        let missing_entry = set_trusted_at(&store, &missing, true).unwrap().path;
        let kept_entry = set_trusted_at(&store, &kept, true).unwrap().path;
        fs::remove_dir(&missing).unwrap();

        let expected = vec![kept_entry];
        assert_eq!(revoke_trusted_at(&store, &missing_entry).unwrap(), expected);
        assert_eq!(revoke_trusted_at(&store, &missing_entry).unwrap(), expected);
        fs::create_dir(&missing).unwrap();
        assert!(!is_trusted_at(&store, &missing));
        assert!(is_trusted_at(&store, &kept));
    }

    #[test]
    fn passive_scan_detects_git_and_agent_risks_without_executing_anything() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("trusted-workspaces.json");
        let repo = tmp.path().join("untrusted");
        fs::create_dir_all(repo.join(".git/hooks")).unwrap();
        fs::create_dir_all(repo.join(".claude")).unwrap();
        fs::write(
            repo.join(".git/config"),
            "[core]\n\tfsmonitor = /bin/true\n\thooksPath = custom-hooks\n[filter \"evil\"]\n\tclean = /bin/true\n",
        )
        .unwrap();
        fs::write(repo.join(".git/hooks/post-checkout"), "#!/bin/sh\n").unwrap();
        fs::write(repo.join(".git/hooks/pre-commit.sample"), "#!/bin/sh\n").unwrap();
        fs::write(repo.join(".claude/settings.json"), "{}").unwrap();
        fs::write(repo.join(".mcp.json"), "{}").unwrap();
        fs::write(repo.join(".gitmodules"), "").unwrap();

        let status = inspect_at(&store, &repo);
        assert!(!status.trusted);
        assert!(status.should_warn());
        let kinds: Vec<WorkspaceRiskKind> = status.risks.iter().map(|r| r.kind).collect();
        assert!(kinds.contains(&WorkspaceRiskKind::GitConfig));
        assert!(kinds.contains(&WorkspaceRiskKind::GitHooks));
        assert!(kinds.contains(&WorkspaceRiskKind::GitSubmodules));
        assert!(kinds.contains(&WorkspaceRiskKind::ClaudeConfig));
        assert!(kinds.contains(&WorkspaceRiskKind::McpConfig));
        assert!(!status.risks.iter().any(|r| r.detail.ends_with(".sample")));
    }

    #[cfg(unix)]
    #[test]
    fn linked_worktree_inherits_trust_only_with_reciprocal_registration() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let store = root.join("trusted-workspaces.json");
        let main = root.join("main");
        let linked = root.join("wt");
        let git = |cwd: &Path, args: &[&str]| {
            assert!(std::process::Command::new("git")
                .current_dir(cwd)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .args(args)
                .output()
                .unwrap()
                .status
                .success());
        };
        git(&root, &["init", "-q", "main"]);
        git(
            &main,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgSign=false",
                "commit",
                "--allow-empty",
                "-qm",
                "init",
            ],
        );
        git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                linked.to_str().unwrap(),
            ],
        );

        set_trusted_at(&store, &main, true).unwrap();
        assert!(is_trusted_at(&store, &linked));

        // A spoofed copy of the `.git` pointer file has no reciprocal backlink.
        let spoof = root.join("spoof");
        fs::create_dir(&spoof).unwrap();
        fs::copy(linked.join(".git"), spoof.join(".git")).unwrap();
        assert!(!is_trusted_at(&store, &spoof));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_configs_are_not_followed_by_scanner() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("trusted-workspaces.json");
        let outside = tmp.path().join("outside.json");
        fs::write(&outside, "{}").unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        std::os::unix::fs::symlink(&outside, repo.join(".mcp.json")).unwrap();
        let status = inspect_at(&store, &repo);
        assert!(status.risks.is_empty());
    }
}
