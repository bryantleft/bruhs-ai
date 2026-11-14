use super::{DeliveryError, DeliveryResult, DeliveryService};
use bruhs_models::delivery::Command;
use std::{path::Path, process::Stdio};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    sync::watch,
};

// Keep one command's response within the companion server's 1 MiB JSON request budget.
// stdout and stderr share that allocation; excess is an explicit failure, never truncated JSON.
pub(super) const OUTPUT_BYTES: usize = 1 << 20;

struct ProcessGroup(Option<u32>);
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.0.and_then(|pid| i32::try_from(pid).ok()) {
            // SAFETY: the child was created in its own process group and is still owned by
            // this runner. Its unreaped PID cannot be reused before this guard is cleared.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
    }
}

async fn bounded(mut stream: impl AsyncRead + Unpin) -> DeliveryResult<Vec<u8>> {
    let mut bytes = Vec::new();
    (&mut stream)
        .take(u64::try_from(OUTPUT_BYTES / 2 + 1).unwrap_or(u64::MAX))
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| {
            DeliveryError::Command(format!("Reading command output failed: {error}"))
        })?;
    if bytes.len() > OUTPUT_BYTES / 2 {
        return Err(DeliveryError::Command("The command exceeded its 512 KiB output budget. Inspect it in a terminal or narrow the requested result.".into()));
    }
    Ok(bytes)
}

impl DeliveryService {
    pub(super) async fn read(
        &self,
        cwd: &Path,
        program: &str,
        args: &[&str],
    ) -> DeliveryResult<String> {
        let specification = Command {
            program: program.into(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            stdin: None,
        };
        let (_cancel, mut cancelled) = watch::channel(false);
        // Same external CLI deadline as the existing pull lookup integration.
        tokio::time::timeout(
            std::time::Duration::from_secs(8),
            self.run_command(cwd, &specification, &mut cancelled),
        )
        .await
        .map_err(|_| {
            DeliveryError::Command(format!(
                "{program} did not answer within 8 seconds. Check sign-in/network access and retry."
            ))
        })?
    }

    pub(super) async fn run_command(
        &self,
        cwd: &Path,
        specification: &Command,
        cancelled: &mut watch::Receiver<bool>,
    ) -> DeliveryResult<String> {
        let executable = match specification.program.as_str() {
            "gh" => self.gh.as_path(),
            "git" => self.git.as_path(),
            "glab" => self.glab.as_path(),
            name => Path::new(name),
        };
        let mut command = tokio::process::Command::new(executable);
        if specification.program == "git" {
            command.args(["-c", "core.fsmonitor=false"]);
        }
        command
            .args(&specification.args)
            .current_dir(cwd)
            .stdin(if specification.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .env("NO_COLOR", "1")
            .env("GH_PROMPT_DISABLED", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("FORCE_COLOR")
            .env_remove("GH_FORCE_TTY");
        if specification.program == "git" {
            // CWD is the reviewed repository; inherited selectors must not redirect
            // its index, worktree or ref transaction into another checkout.
            for variable in [
                "GIT_DIR",
                "GIT_WORK_TREE",
                "GIT_COMMON_DIR",
                "GIT_INDEX_FILE",
                "GIT_NAMESPACE",
            ] {
                command.env_remove(variable);
            }
        }
        if specification.program == "gh" {
            command.env_remove("GH_REPO");
            command.env_remove("GH_HOST");
            command.env_remove("GH_DEBUG");
        }
        if specification.program == "glab" {
            for name in [
                "GITLAB_REPO",
                "GLAB_REPO",
                "GITLAB_HOST",
                "GITLAB_URI",
                "GITLAB_URL",
                "GLAB_DEBUG_HTTP",
            ] {
                command.env_remove(name);
            }
        }
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command.spawn().map_err(|error| {
            DeliveryError::Command(format!(
                "Could not start {}: {error}. Install/sign in to the CLI on this host.",
                specification.program
            ))
        })?;
        let mut group = ProcessGroup(child.id());
        let stdin = child.stdin.take();
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| DeliveryError::Command("Command stdout was unavailable.".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| DeliveryError::Command("Command stderr was unavailable.".into()))?;
        let input = async {
            if let Some(mut input) = stdin {
                if let Some(body) = &specification.stdin {
                    input
                        .write_all(body.as_bytes())
                        .await
                        .map_err(|error| DeliveryError::Command(error.to_string()))?;
                }
                input
                    .shutdown()
                    .await
                    .map_err(|error| DeliveryError::Command(error.to_string()))?;
            }
            Ok::<(), DeliveryError>(())
        };
        let work = async {
            let (_, out, err) = tokio::try_join!(input, bounded(stdout), bounded(stderr))?;
            let status = child
                .wait()
                .await
                .map_err(|error| DeliveryError::Command(error.to_string()))?;
            group.0 = None;
            let stdout = String::from_utf8(out).map_err(|_| {
                DeliveryError::Command("The command returned non-UTF-8 output.".into())
            })?;
            let stderr = String::from_utf8_lossy(&err);
            if !status.success() {
                return Err(DeliveryError::Command(format!(
                    "{} exited with {status}: {}. Earlier successful steps remain applied; inspect the receipt before preparing another action.",
                    specification.program,
                    stderr.trim()
                )));
            }
            Ok(if stderr.trim().is_empty() {
                stdout
            } else {
                format!("{stdout}\n{stderr}")
            })
        };
        tokio::select! {
            result = work => result,
            result = cancelled.changed() => {
                let detail = if result.is_err() { "The action owner stopped" } else { "The action was cancelled" };
                Err(DeliveryError::Command(format!("{detail}; the active process group was stopped. Earlier successful steps remain applied.")))
            }
        }
    }
}
