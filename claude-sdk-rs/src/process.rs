//! Subprocess plumbing shared by the CLI wrappers: spawning with piped
//! stdout/stderr, a line pump that notices a dropped consumer, a bounded
//! stderr tail drained concurrently, kill-and-reap, and exit-status reporting.
//!
//! This file is the single copy for two crates. `claude-sdk-rs` owns it and
//! publishes it as a private module; the unpublished `codex-sdk-rs` compiles
//! the same file in with `#[path]`. A shared crate would have to be published
//! to crates.io before `claude-sdk-rs` could be, so keep this file free of
//! anything crate-specific: both crates must use every item in it.

use std::collections::VecDeque;
use std::fmt;
use std::io;
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Bytes of stderr kept for [`ProcessFailure::stderr_tail`].
const STDERR_TAIL_BYTES: usize = 8 * 1024;
/// How long a CLI may keep running after closing stdout before it is killed.
const EXIT_GRACE: Duration = Duration::from_secs(5);
/// How long to wait for stderr to reach EOF once the CLI is gone; a
/// grandchild that inherited the pipe can keep it open indefinitely.
const STDERR_SETTLE: Duration = Duration::from_secs(1);

/// Why a CLI process ended abnormally. Carried by the `ProcessFailed` event,
/// which is always the last event of a stream.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProcessFailure {
    /// What went wrong, e.g. `claude exited unsuccessfully (exit status: 1)`.
    pub message: String,
    /// The process's exit code, when it exited on its own. `None` when it was
    /// killed by a signal (including by this SDK after an I/O failure).
    pub exit_code: Option<i32>,
    /// The last 8 KiB of the process's stderr, lossily decoded and trimmed;
    /// empty when it wrote nothing.
    pub stderr_tail: String,
}

impl fmt::Display for ProcessFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)?;
        if !self.stderr_tail.is_empty() {
            write!(f, ": {}", self.stderr_tail)?;
        }
        Ok(())
    }
}

/// How the line pump stopped.
pub(crate) enum Ending {
    /// stdout reached EOF: the CLI is done (or about to be).
    Eof,
    /// The stream's receiver was dropped; nobody wants the output any more.
    ConsumerGone,
    /// Talking to the CLI failed; the message says how.
    Failed(String),
}

/// Start from an empty environment unless `inherit`, remove `remove`, then
/// set `set` (later entries win).
pub(crate) fn apply_env(
    cmd: &mut Command,
    inherit: bool,
    remove: &[String],
    set: &[(String, String)],
) {
    if !inherit {
        cmd.env_clear();
    }
    for key in remove {
        cmd.env_remove(key);
    }
    for (key, value) in set {
        cmd.env(key, value);
    }
}

/// A spawned CLI whose stdout is read line by line and whose stderr is
/// drained in the background. The child is killed if this is dropped.
pub(crate) struct CliProcess {
    program: &'static str,
    child: Child,
    stdout: BufReader<ChildStdout>,
    stderr: StderrTail,
    #[cfg(unix)]
    group: ProcessGroup,
}

/// The process group a child was spawned into with `process_group(0)`; the
/// whole group is killed on drop unless [`ProcessGroup::release`]d.
#[cfg(unix)]
pub(crate) struct ProcessGroup(Option<i32>);

#[cfg(unix)]
impl ProcessGroup {
    /// The group led by `child`, which must have been spawned with
    /// `process_group(0)`.
    pub(crate) fn of(child: &Child) -> Self {
        Self(
            child
                .id()
                .and_then(|id| i32::try_from(id).ok())
                .filter(|id| *id > 0),
        )
    }

    /// Leave the group running: the child exited on its own, so its id may
    /// be reused and must not be signalled.
    pub(crate) fn release(&mut self) {
        self.0 = None;
    }

    fn terminate(&mut self) {
        if let Some(id) = self.0.take() {
            // SAFETY: spawn creates a new process group whose positive id is
            // the child pid. Negating it targets only that owned group.
            let _ = unsafe { libc::kill(-id, libc::SIGKILL) };
        }
    }
}

#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        self.terminate();
    }
}

impl CliProcess {
    pub(crate) fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    /// Spawn `cmd` with stdout and stderr piped and `kill_on_drop` set; stdin
    /// is left as the caller configured it.
    ///
    /// # Panics
    ///
    /// Panics outside a Tokio runtime.
    pub(crate) fn spawn(program: &'static str, cmd: &mut Command) -> io::Result<Self> {
        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);
        let mut child = cmd.spawn()?;
        #[cfg(unix)]
        let group = ProcessGroup::of(&child);
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("child stdout was not piped"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("child stderr was not piped"))?;
        Ok(Self {
            program,
            child,
            stdout: BufReader::new(stdout),
            stderr: StderrTail::drain(stderr),
            #[cfg(unix)]
            group,
        })
    }

    /// The child's stdin, if it was piped and not taken yet.
    pub(crate) fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.stdin.take()
    }

    /// The next stdout line without its line ending (invalid UTF-8 is
    /// replaced, not fatal), or why there is none. Notices a dropped receiver
    /// even while the CLI is silent.
    pub(crate) async fn next_line<E>(&mut self, tx: &mpsc::Sender<E>) -> Result<String, Ending> {
        self.next_line_with_limit(tx, None).await
    }

    /// Read at most the caller's frame budget plus one byte, so a malformed
    /// peer cannot allocate an unlimited line before the size check runs.
    pub(crate) async fn next_line_with_limit<E>(
        &mut self,
        tx: &mpsc::Sender<E>,
        limit: Option<u64>,
    ) -> Result<String, Ending> {
        let mut buf = Vec::new();
        let mut input =
            (&mut self.stdout).take(limit.map_or(u64::MAX, |limit| limit.saturating_add(1)));
        let read = tokio::select! {
            () = tx.closed() => return Err(Ending::ConsumerGone),
            read = input.read_until(b'\n', &mut buf) => read,
        };
        match read {
            Ok(0) => Err(Ending::Eof),
            Ok(n) if limit.is_some_and(|limit| u64::try_from(n).map_or(true, |n| n > limit)) => {
                Err(Ending::Failed(format!(
                    "{} output exceeded the configured frame budget",
                    self.program
                )))
            }
            Ok(_) => {
                if buf.last() == Some(&b'\n') {
                    buf.pop();
                }
                if buf.last() == Some(&b'\r') {
                    buf.pop();
                }
                Ok(String::from_utf8_lossy(&buf).into_owned())
            }
            Err(e) => Err(Ending::Failed(format!(
                "reading {} output failed: {e}",
                self.program
            ))),
        }
    }

    /// Make sure the CLI is gone and report whether it failed.
    ///
    /// - [`Ending::ConsumerGone`]: kill and reap; nothing to report.
    /// - [`Ending::Eof`]: give the CLI [`EXIT_GRACE`] to exit, killing it after
    ///   that; report a non-zero exit (or a death by signal not caused here).
    /// - [`Ending::Failed`]: kill it unless it already exited, then report.
    pub(crate) async fn finish(mut self, ending: Ending) -> Option<ProcessFailure> {
        let status = match ending {
            Ending::ConsumerGone => {
                self.kill_and_reap().await;
                self.stderr.abort();
                return None;
            }
            Ending::Failed(message) => {
                let status = match self.child.try_wait() {
                    Ok(Some(status)) => Some(status),
                    _ => self.kill_and_reap().await,
                };
                let exit_code = status.and_then(|s| s.code());
                return Some(self.failure(message, exit_code).await);
            }
            Ending::Eof => match tokio::time::timeout(EXIT_GRACE, self.child.wait()).await {
                Ok(Ok(status)) => status,
                Ok(Err(e)) => {
                    let message = format!("waiting for {} failed: {e}", self.program);
                    return Some(self.failure(message, None).await);
                }
                // It closed stdout, so its output is complete; it just would
                // not exit.
                Err(_elapsed) => {
                    self.kill_and_reap().await;
                    self.stderr.abort();
                    return None;
                }
            },
        };
        if status.success() {
            // A completed turn may intentionally have started a background task.
            #[cfg(unix)]
            {
                self.group.release();
            }
            self.stderr.abort();
            return None;
        }
        let message = format!("{} exited unsuccessfully ({status})", self.program);
        Some(self.failure(message, status.code()).await)
    }

    async fn kill_and_reap(&mut self) -> Option<ExitStatus> {
        #[cfg(unix)]
        self.group.terminate();
        // An error here means the process already exited; `wait` below still
        // reaps it and returns its status.
        let _ = self.child.start_kill();
        // If even waiting fails there is no status to report, and
        // `kill_on_drop` still covers the process.
        self.child.wait().await.ok()
    }

    async fn failure(self, message: String, exit_code: Option<i32>) -> ProcessFailure {
        #[cfg(unix)]
        {
            let mut group = self.group;
            group.terminate();
        }
        ProcessFailure {
            message,
            exit_code,
            stderr_tail: self.stderr.finish().await,
        }
    }
}

/// The last [`STDERR_TAIL_BYTES`] of a child's stderr, drained by a
/// background task so a chatty CLI can never block on a full pipe.
struct StderrTail {
    tail: Arc<Mutex<VecDeque<u8>>>,
    task: JoinHandle<()>,
}

impl StderrTail {
    fn drain(mut stderr: ChildStderr) -> Self {
        let tail = Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL_BYTES)));
        let sink = Arc::clone(&tail);
        let task = tokio::spawn(async move {
            let mut chunk = [0_u8; 4096];
            // EOF or a read error both mean there is nothing more to keep.
            while let Ok(read @ 1..) = stderr.read(&mut chunk).await {
                push_tail(&sink, &chunk[..read]);
            }
        });
        Self { tail, task }
    }

    /// Let the drain reach EOF (briefly), then return the tail.
    async fn finish(self) -> String {
        let mut task = self.task;
        if tokio::time::timeout(STDERR_SETTLE, &mut task)
            .await
            .is_err()
        {
            task.abort();
        }
        let mut tail = self.tail.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8_lossy(tail.make_contiguous())
            .trim()
            .to_string()
    }

    fn abort(self) {
        self.task.abort();
    }
}

fn push_tail(tail: &Mutex<VecDeque<u8>>, bytes: &[u8]) {
    let mut tail = tail.lock().unwrap_or_else(PoisonError::into_inner);
    tail.extend(bytes);
    let excess = tail.len().saturating_sub(STDERR_TAIL_BYTES);
    tail.drain(..excess);
}

#[cfg(test)]
mod process_tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn oversized_frame_fails_before_a_silent_writer_reaches_eof() {
        let mut command = Command::new("sh");
        command.args(["-c", "printf 123456789; sleep 30"]);
        let mut process = CliProcess::spawn("fixture", &mut command).unwrap();
        let (tx, _rx) = mpsc::channel::<()>(1);
        let result =
            tokio::time::timeout(EXIT_GRACE, process.next_line_with_limit(&tx, Some(8))).await;
        assert!(
            matches!(result, Ok(Err(Ending::Failed(message))) if message.contains("frame budget"))
        );
        assert!(process.finish(Ending::ConsumerGone).await.is_none());
    }

    #[test]
    fn stderr_tail_keeps_only_the_most_recent_bytes() {
        let tail = Mutex::new(VecDeque::new());
        push_tail(&tail, &vec![b'a'; STDERR_TAIL_BYTES]);
        push_tail(&tail, b"newest");
        let kept: Vec<u8> = tail.lock().unwrap().iter().copied().collect();
        assert_eq!(kept.len(), STDERR_TAIL_BYTES);
        assert!(kept.ends_with(b"newest"));
        assert!(kept.starts_with(b"aaa"));

        push_tail(&tail, &vec![b'z'; STDERR_TAIL_BYTES * 2]);
        assert!(tail.lock().unwrap().iter().all(|&b| b == b'z'));
    }

    #[test]
    fn failure_display_includes_the_stderr_tail() {
        let failure = ProcessFailure {
            message: "cli exited unsuccessfully (exit status: 3)".into(),
            exit_code: Some(3),
            stderr_tail: "auth expired".into(),
        };
        assert_eq!(
            failure.to_string(),
            "cli exited unsuccessfully (exit status: 3): auth expired"
        );
        let quiet = ProcessFailure {
            stderr_tail: String::new(),
            ..failure
        };
        assert_eq!(
            quiet.to_string(),
            "cli exited unsuccessfully (exit status: 3)"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancelling_a_turn_terminates_its_shell_descendants() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 30 & echo $!; wait"]);
        let mut process = CliProcess::spawn("fixture", &mut command).unwrap();
        let (tx, rx) = mpsc::channel::<()>(1);
        let pid = process
            .next_line(&tx)
            .await
            .unwrap_or_else(|_| panic!("missing child pid"));
        let pid: u32 = pid.trim().parse().unwrap();
        drop(rx);
        assert!(process.finish(Ending::ConsumerGone).await.is_none());
        let status = Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .await
            .unwrap();
        let state = String::from_utf8_lossy(&status.stdout);
        let stopped = state.trim().is_empty() || state.trim_start().starts_with('Z');
        if !stopped {
            // Clean up the fixture even when the cancellation assertion fails.
            let _ = Command::new("kill")
                .args(["-KILL", &pid.to_string()])
                .status()
                .await;
        }
        assert!(stopped, "cancelled CLI left child {pid} running: {state}");
    }
}
