use super::{DeliveryError, DeliveryResult, DeliveryService};
use bruhs_models::delivery::{Command, Forge, ForgeRepository};
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
        self.read_scoped(cwd, program, args, None).await
    }

    pub(super) async fn github_repository(&self, cwd: &Path) -> DeliveryResult<ForgeRepository> {
        self
            .forge_repository(cwd, Forge::GitHub)
            .await?
            .ok_or_else(|| {
                DeliveryError::Invalid(
                    "GitHub delivery requires remote.origin.url. Add the intended GitHub origin and retry."
                        .into(),
                )
            })
    }

    /// GitHub's saved default remote can differ from origin. Select the
    /// repository explicitly for PR commands and API placeholder expansion.
    pub(super) async fn github_read(&self, cwd: &Path, args: &[&str]) -> DeliveryResult<String> {
        let repository = self.github_repository(cwd).await?;
        self.read_scoped(cwd, "gh", args, Some(&repository)).await
    }

    pub(super) async fn read_scoped(
        &self,
        cwd: &Path,
        program: &str,
        args: &[&str],
        repository: Option<&ForgeRepository>,
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
            self.run_command(cwd, &specification, &mut cancelled, repository),
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
        repository: Option<&ForgeRepository>,
    ) -> DeliveryResult<String> {
        self.run_with_output(cwd, specification, cancelled, repository, None)
            .await
    }

    /// Guide evidence stays in a private temporary file. Command diagnostics
    /// and ordinary JSON responses keep their normal in-memory output budget.
    pub(super) async fn spool(
        &self,
        cwd: &Path,
        program: &str,
        args: &[&str],
        repository: Option<&ForgeRepository>,
    ) -> DeliveryResult<std::fs::File> {
        use std::io::Seek;
        let mut file = tempfile::tempfile().map_err(|e| DeliveryError::Storage(e.to_string()))?;
        let output = file
            .try_clone()
            .map_err(|e| DeliveryError::Storage(e.to_string()))?;
        let specification = Command {
            program: program.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            stdin: None,
        };
        let (_owner, mut cancelled) = watch::channel(false);
        tokio::time::timeout(
            std::time::Duration::from_secs(90),
            self.run_with_output(
                cwd,
                &specification,
                &mut cancelled,
                repository,
                Some(output),
            ),
        )
        .await
        .map_err(|_| {
            DeliveryError::Command(format!("{program} timed out reading review data."))
        })??;
        file.rewind()
            .map_err(|e| DeliveryError::Storage(e.to_string()))?;
        Ok(file)
    }

    async fn run_with_output(
        &self,
        cwd: &Path,
        specification: &Command,
        cancelled: &mut watch::Receiver<bool>,
        repository: Option<&ForgeRepository>,
        output: Option<std::fs::File>,
    ) -> DeliveryResult<String> {
        let executable = match specification.program.as_str() {
            "gh" => self.gh.as_path(),
            "git" => self.git.as_path(),
            "glab" => self.glab.as_path(),
            name => Path::new(name),
        };
        let mut command = tokio::process::Command::new(executable);
        if specification.program == "git" {
            let trusted = crate::workspace_trust::is_trusted(cwd);
            let local_overrides = if trusted {
                Vec::new()
            } else {
                crate::git::repo_local_neutralizers(cwd, true).await
            };
            command.arg("--no-pager").args([
                "-c",
                "core.fsmonitor=false",
                "-c",
                "core.pager=cat",
                "-c",
                "diff.external=",
                "-c",
                "protocol.ext.allow=never",
                "-c",
                "diff.autoRefreshIndex=false",
            ]);
            if !trusted {
                command.args([
                    "-c",
                    &format!("core.hooksPath={}", crate::git::EMPTY_HOOKS_PATH),
                ]);
            }
            for kv in &local_overrides {
                command.arg("-c").arg(kv);
            }
            crate::git::strip_git_env(&mut command);
            command.env("GIT_OPTIONAL_LOCKS", "0");
        } else if matches!(specification.program.as_str(), "gh" | "glab") {
            // Isolate internal `git` subprocesses spawned by forge CLIs.
            crate::git::apply_safe_git_subprocess_env(&mut command);
        }
        command
            .args(&specification.args)
            .current_dir(cwd)
            .stdin(if specification.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(output.map_or_else(Stdio::piped, Stdio::from))
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .env("NO_COLOR", "1")
            .env("GH_PROMPT_DISABLED", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("FORCE_COLOR")
            .env_remove("GH_FORCE_TTY");
        if specification.program == "gh" {
            let repository = repository
                .filter(|repository| repository.forge == Forge::GitHub)
                .ok_or_else(|| DeliveryError::Conflict(
                    "This GitHub command has no reviewed repository. Add the intended origin and prepare the action again."
                        .into(),
                ))?;
            command.env(
                "GH_REPO",
                format!("{}/{}", repository.host, repository.project),
            );
            command.env("GH_HOST", &repository.host);
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
        let stdout_pipe = child.stdout.take();
        let stdout = async move {
            match stdout_pipe {
                Some(pipe) => bounded(pipe).await,
                None => Ok(Vec::new()),
            }
        };
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
            let (_, out, err) = tokio::try_join!(input, stdout, bounded(stderr))?;
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
