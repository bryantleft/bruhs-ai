//! Project switches are host settings, shared by desktop and companion processes.
use super::*;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub(super) type Switches = BTreeMap<String, bool>;
pub(super) type Projects = BTreeMap<PathBuf, Switches>;

pub(super) fn load(root: &Path) -> io::Result<Projects> {
    match fs::read(root.join("project-provider-settings.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Projects::new()),
        Err(error) => Err(error),
    }
}

pub(super) fn save(root: &Path, project: &Path, id: &str, value: Option<bool>) -> io::Result<()> {
    let builtin = id
        .parse::<AgentProvider>()
        .is_ok_and(|p| p != AgentProvider::Acp && p.as_str() == id);
    let acp = id.strip_prefix("acp/").is_some_and(|name| {
        !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    });
    if !project.is_absolute() || !(builtin || acp) {
        return Err(io::Error::other(
            "Choose a project folder and a provider instance.",
        ));
    }
    let key = project
        .canonicalize()
        .unwrap_or_else(|_| project.to_owned());
    let _lock = super::super::lock_config(root)?;
    let mut settings = load(root)?;
    if let Some(value) = value {
        settings
            .entry(key)
            .or_default()
            .insert(id.to_owned(), value);
    } else if let Some(overrides) = settings.get_mut(&key) {
        overrides.remove(id);
        if overrides.is_empty() {
            settings.remove(&key);
        }
    } else {
        return Ok(());
    }
    let mut file = tempfile::NamedTempFile::new_in(root)?;
    serde_json::to_writer_pretty(&mut file, &settings)?;
    file.as_file().sync_all()?;
    file.persist(root.join("project-provider-settings.json"))?;
    Ok(())
}

fn git_pointer(path: &Path) -> Option<String> {
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > 16 * 1024 {
        return None;
    }
    fs::read_to_string(path).ok()
}

/// Resolve a linked worktree without starting Git on the UI thread. Confirm the
/// registration points back to this worktree before inheriting the main checkout.
fn main_checkout(folder: &Path) -> Option<PathBuf> {
    let dot = folder.join(".git");
    let pointer = git_pointer(&dot)?;
    let admin = folder
        .join(pointer.trim_end().strip_prefix("gitdir: ")?)
        .canonicalize()
        .ok()?;
    let common = admin
        .join(git_pointer(&admin.join("commondir"))?.trim_end())
        .canonicalize()
        .ok()?;
    if common.file_name()? != ".git" || admin.parent()? != common.join("worktrees") {
        return None;
    }
    let backlink = admin
        .join(git_pointer(&admin.join("gitdir"))?.trim_end())
        .canonicalize()
        .ok()?;
    (backlink == dot)
        .then(|| common.parent().map(Path::to_owned))
        .flatten()
}

pub(super) fn resolve(settings: &Projects, folder: &Path) -> (Switches, Switches) {
    if settings.is_empty() || folder.as_os_str().is_empty() {
        return Default::default();
    }
    let key = folder.canonicalize().unwrap_or_else(|_| folder.to_owned());
    let local = settings.get(&key).cloned().unwrap_or_default();
    let mut effective = main_checkout(&key)
        .and_then(|main| settings.get(&main))
        .cloned()
        .unwrap_or_default();
    effective.extend(local.clone());
    (effective, local)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_are_independent_and_reset_to_the_current_environment() {
        let store = tempfile::tempdir().unwrap();
        let first = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let (root, a, b) = (store.path(), first.path(), other.path());
        let provider = AgentProvider::Codex;
        ProviderAvailability::set_at(root, provider, false).unwrap();
        let agent: bruhs_models::agent_extensions::AcpInstance =
            serde_json::from_value(serde_json::json!({
                "id":"lab","name":"Lab","command":"agent","enabled":true
            }))
            .unwrap();
        for (scope, id, value) in [
            (a, "codex", true),
            (a, "acp/lab", false),
            (b, "claude", false),
        ] {
            save(root, scope, id, Some(value)).unwrap();
        }
        let all = ProviderAvailability::load_at(root).unwrap();
        let scoped = all.for_project(a);
        assert!(scoped.enabled(provider));
        assert!(!scoped.acp_enabled(&agent));
        assert_eq!(scoped.project_override("codex"), Some(true));
        assert!(!scoped.for_project(b).enabled(provider));
        assert!(scoped.for_project(b).acp_enabled(&agent));
        assert!(!scoped.for_project(b).enabled(AgentProvider::Claude));
        assert!(all.enabled(AgentProvider::Claude));
        save(root, a, "codex", None).unwrap();
        let scoped = ProviderAvailability::load_at(root).unwrap().for_project(a);
        assert!(!scoped.enabled(provider));
        assert_eq!(scoped.project_override("codex"), None);
        assert!(!scoped.acp_enabled(&agent));
        ProviderAvailability::set_at(root, provider, true).unwrap();
        assert!(ProviderAvailability::load_at(root)
            .unwrap()
            .for_project(a)
            .enabled(provider));
    }

    #[test]
    fn linked_checkout_inherits_registered_main_then_applies_its_own_switches() {
        let temporary = tempfile::tempdir().unwrap();
        let dir = temporary.path().canonicalize().unwrap();
        let main = dir.join("repo");
        let linked = dir.join("feature");
        let git = |cwd: &Path, args: &[&str]| {
            assert!(std::process::Command::new("git")
                .current_dir(cwd)
                .args(args)
                .output()
                .unwrap()
                .status
                .success());
        };
        git(&dir, &["init", "repo"]);
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
                "-m",
                "initial",
            ],
        );
        git(
            &main,
            &["worktree", "add", "--detach", linked.to_str().unwrap()],
        );
        let root = dir.join("prefs");
        save(&root, &main, "codex", Some(false)).unwrap();
        let available = ProviderAvailability::load_at(&root)
            .unwrap()
            .for_project(&linked);
        assert!(!available.enabled(AgentProvider::Codex));
        assert_eq!(available.project_override("codex"), None);
        save(&root, &linked, "codex", Some(true)).unwrap();
        let available = ProviderAvailability::load_at(&root).unwrap();
        assert!(available.for_project(&linked).enabled(AgentProvider::Codex));
        assert!(!available.for_project(&main).enabled(AgentProvider::Codex));
        // A copied .git pointer has no reciprocal registration.
        let other = dir.join("other");
        fs::create_dir(&other).unwrap();
        fs::copy(linked.join(".git"), other.join(".git")).unwrap();
        assert!(available.for_project(&other).enabled(AgentProvider::Codex));
    }
}
