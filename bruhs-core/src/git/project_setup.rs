//! Owner-configured setup steps that must finish before a new worktree's first turn.
use super::*;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::OpenOptions;

/// Lifecycle options stored with a project action in the owner profile.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectActionLifecycle {
    pub run_on_worktree_create: bool,
    pub wait_for_setup: bool,
    pub run_on_settle: bool,
    pub stop_on_settle: bool,
}

/// A command saved in Bruhs project settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectAction {
    pub id: String,
    pub name: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_url: Option<String>,
    #[serde(default)]
    pub auto_open_preview: bool,
    #[serde(flatten)]
    pub lifecycle: ProjectActionLifecycle,
}

impl ProjectAction {
    /// A setup step must finish before its worktree is ready for the thread.
    pub fn setup_step(&self) -> Option<SetupAction> {
        (self.lifecycle.run_on_worktree_create && self.lifecycle.wait_for_setup).then(|| {
            SetupAction {
                id: self.id.clone(),
                name: self.name.clone(),
                command: self.command.clone(),
            }
        })
    }
}

#[derive(Clone, Debug)]
pub struct SetupAction {
    pub id: String,
    pub name: String,
    pub command: String,
}

/// Load global and project setup steps from the current host's owner profile.
pub async fn load_project_setup(project: &Path) -> Result<Vec<SetupAction>, GitError> {
    let Some(profile) = crate::delivery::writer::desktop_state_file() else {
        return Ok(Vec::new());
    };
    let project = project.to_path_buf();
    tokio::task::spawn_blocking(move || load_project_setup_from(&profile, &project))
        .await
        .map_err(std::io::Error::other)?
}

fn load_project_setup_from(profile: &Path, project: &Path) -> Result<Vec<SetupAction>, GitError> {
    #[derive(Deserialize)]
    struct Preferences {
        #[serde(default)]
        project_actions: BTreeMap<String, Vec<ProjectAction>>,
    }

    let file = match std::fs::File::open(profile) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(GitError::ProjectSetup(format!(
                "Cannot read saved actions. {error}"
            )))
        }
    };
    let prefs: Preferences = serde_json::from_reader(std::io::BufReader::new(file))
        .map_err(|error| GitError::ProjectSetup(format!("Cannot read saved actions. {error}")))?;
    let scoped = prefs
        .project_actions
        .get(project.to_string_lossy().as_ref())
        .or_else(|| {
            prefs.project_actions.iter().find_map(|(path, actions)| {
                (!path.is_empty()
                    && Path::new(path).canonicalize().ok().as_deref() == Some(project))
                .then_some(actions)
            })
        });
    Ok(prefs
        .project_actions
        .get("")
        .into_iter()
        .chain(scoped)
        .flatten()
        .filter_map(ProjectAction::setup_step)
        .collect())
}

pub async fn run_project_setup(checkout: &Path, actions: &[SetupAction]) -> Result<(), GitError> {
    if actions.is_empty() {
        return Ok(());
    }
    let identified = git(checkout, &["rev-parse", "--absolute-git-dir"]).await?;
    if !identified.success() {
        return Err(identified.failure());
    }
    let receipts = PathBuf::from(identified.stdout_lossy().trim_end()).join("bruhs-project-setup");
    tokio::fs::create_dir_all(&receipts).await?;
    let lock_path = receipts.join("setup.lock");
    // Serialise retrying clients for this particular checkout. Other worktrees can progress.
    let _lock = tokio::task::spawn_blocking(move || {
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        lock.lock()?;
        Ok::<_, std::io::Error>(lock)
    })
    .await
    .map_err(std::io::Error::other)??;
    for action in actions {
        let mut digest = Sha256::new();
        digest.update(action.id.as_bytes());
        digest.update([0]);
        digest.update(action.command.as_bytes());
        let complete = receipts.join(format!("{:x}", digest.finalize()));
        if tokio::fs::try_exists(&complete).await? {
            continue;
        }
        let shell = crate::terminal::pick_shell(std::env::var("SHELL").ok().as_deref());
        let mut command = Command::new(shell);
        #[cfg(windows)]
        command.args(["/D", "/S", "/C"]);
        #[cfg(not(windows))]
        command.arg("-lc");
        command
            .arg(&action.command)
            .current_dir(checkout)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn()?;
        let stderr = child.stderr.take();
        let (status, stderr) = tokio::try_join!(child.wait(), read_bounded(stderr, 4096))?;
        if !status.success() {
            return Err(GitError::ProjectSetup(format!(
                "{} ({status}) {}",
                action.name,
                String::from_utf8_lossy(&stderr).trim()
            )));
        }
        tokio::fs::write(complete, []).await?;
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn saved_setup_loads_only_waiting_actions_for_global_and_matching_project() {
        let fixture = tempfile::tempdir().unwrap();
        let project = fixture.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let alias = fixture.path().join("alias");
        std::os::unix::fs::symlink(&project, &alias).unwrap();
        let profile = fixture.path().join("desktop-state.json");
        assert!(load_project_setup_from(&profile, &project)
            .unwrap()
            .is_empty());
        let actions = serde_json::json!({
            "project_actions": {
                "": [
                    {"id":"manual", "name":"Manual", "command":"true"},
                    {"id":"global", "name":"Global", "command":"true", "run_on_worktree_create":true, "wait_for_setup":true},
                    {"id":"background", "name":"Background", "command":"true", "run_on_worktree_create":true},
                ],
                (alias.to_str().unwrap()): [
                    {"id":"project", "name":"Project", "command":"true", "run_on_worktree_create":true, "wait_for_setup":true},
                ],
                "/other/project": [
                    {"id":"other", "name":"Other", "command":"true", "run_on_worktree_create":true, "wait_for_setup":true},
                ],
            },
            "current_model": "unrelated owner preference",
        });
        std::fs::write(&profile, actions.to_string()).unwrap();
        let setup = load_project_setup_from(&profile, &project.canonicalize().unwrap()).unwrap();
        assert_eq!(
            setup
                .into_iter()
                .map(|action| action.id)
                .collect::<Vec<_>>(),
            ["global", "project"]
        );
    }

    #[tokio::test]
    async fn successful_steps_run_once_and_a_failed_step_can_be_retried() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path();
        assert!(Command::new("git")
            .arg("init")
            .current_dir(cwd)
            .output()
            .await
            .unwrap()
            .status
            .success());
        let first = SetupAction {
            id: "1".into(),
            name: "Record path".into(),
            command: "pwd >> rows".into(),
        };
        let second = SetupAction {
            id: "2".into(),
            name: "Wait for fixture".into(),
            command: "test -f ready".into(),
        };
        let steps = [first, second];
        assert!(matches!(
            run_project_setup(cwd, &steps).await,
            Err(GitError::ProjectSetup(_))
        ));
        std::fs::write(cwd.join("ready"), []).unwrap();
        let (a, b) = tokio::join!(
            run_project_setup(cwd, &steps),
            run_project_setup(cwd, &steps)
        );
        a.unwrap();
        b.unwrap();
        assert_eq!(
            std::fs::read_to_string(cwd.join("rows"))
                .unwrap()
                .lines()
                .count(),
            1
        );
    }
}
