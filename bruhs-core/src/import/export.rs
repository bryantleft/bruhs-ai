//! Sending a bruhs thread to T3 Code: T3 cannot take over a CLI session,
//! so its CLI starts a new thread in the project whose first message is
//! the transcript. Claude Code and Codex need no help here (the thread's
//! own session resumes there; see `portable::resume_command`), and Cursor
//! takes the Markdown as pasted text.

use super::ImportError;
use bruhs_models::agent::AgentProvider;
use bruhs_models::logic::portable::t3_export_prompt;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// T3's CLI: `t3` on `PATH`, else where its installer puts it.
fn t3_binary() -> Option<PathBuf> {
    let on_path = std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join("t3"))
            .find(|path| path.is_file())
    });
    on_path.or_else(|| {
        directories::BaseDirs::new()
            .map(|dirs| dirs.home_dir().join(".t3/bin/t3"))
            .filter(|path| path.is_file())
    })
}

fn run(binary: &Path, args: &[&str], stdin: Option<&str>) -> Result<String, String> {
    let mut child = Command::new(binary)
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        // A CLI that exits early (bad arguments) closes its end before
        // reading it all; its own error, read below, says why.
        if let Err(error) = pipe.write_all(text.as_bytes()) {
            if error.kind() != std::io::ErrorKind::BrokenPipe {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.to_string());
            }
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        Err(format!("{} {}", stderr.trim(), stdout.trim())
            .trim()
            .to_string())
    }
}

/// T3's provider instance id for `provider`.
fn t3_provider(provider: AgentProvider) -> &'static str {
    match provider {
        AgentProvider::Claude => "claudeAgent",
        AgentProvider::Codex => "codex",
    }
}

/// Start a T3 thread in `project` carrying `markdown`, on `model` (T3 needs
/// one when the project has no default), and return its id. A project T3
/// does not know yet is added first. Needs T3 running (its CLI talks to the
/// app's server).
///
/// # Errors
///
/// When T3 is not installed or its CLI fails (the message says why).
pub fn export_to_t3(
    project: &Path,
    title: &str,
    model: &str,
    markdown: &str,
) -> Result<String, ImportError> {
    let failed = |message: String| ImportError::Database("T3 Code", message);
    let binary = t3_binary().ok_or_else(|| failed("the t3 CLI is not installed".into()))?;
    let project = project.to_string_lossy();
    // A title starting with `-` would read as an option.
    let title = Some(title.trim().trim_start_matches('-').trim_start())
        .filter(|title| !title.is_empty())
        .unwrap_or("bruhs thread");
    let message = t3_export_prompt(markdown);
    let provider = t3_provider(AgentProvider::for_model(model));
    let start = || {
        run(
            &binary,
            &[
                "thread",
                "start",
                "--json",
                "--title",
                title,
                "--provider",
                provider,
                "--model",
                model,
                &project,
                "-",
            ],
            Some(&message),
        )
    };
    let output = match start() {
        Ok(output) => output,
        Err(first) => {
            // Most likely a project T3 does not know; adding one it already
            // has fails harmlessly, and the retry reports the real error.
            let _ = run(&binary, &["project", "add", &project], None);
            start().map_err(|again| failed(if again.is_empty() { first } else { again }))?
        }
    };
    serde_json::from_str::<serde_json::Value>(output.trim())
        .ok()
        .and_then(|json| json.get("threadId")?.as_str().map(str::to_string))
        .ok_or_else(|| failed(format!("unexpected reply from t3: {}", output.trim())))
}
