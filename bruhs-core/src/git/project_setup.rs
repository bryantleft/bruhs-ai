//! Owner-configured setup steps that must finish before a new worktree's first turn.
use super::*;
use sha2::{Digest, Sha256};
use std::fs::OpenOptions;

#[derive(Clone, Debug)]
pub struct SetupAction {
    pub id: String,
    pub name: String,
    pub command: String,
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
