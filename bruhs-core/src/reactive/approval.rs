//! A bruh's risky steps wait on the owner. The access gate decides each tool
//! call of an unattended run; a call that needs approval becomes an outbox
//! item, and the run waits for the answer, a timeout or Stop.

use super::{store, ReactiveError, ReactiveRuntime};
use crate::PermissionDecision;
use bruhs_models::{
    agent::AgentProvider,
    crew::{approval_fingerprint, permission_gate, ApprovalState, BruhAccess, Gate},
    reactive::{ReactiveRun, RunStatus},
};
use rusqlite::Connection;
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

const DENIED: &str = "The owner denied this. Continue without it.";
const NOT_ANSWERED: &str =
    "The owner has not answered yet. Stop here and say you are waiting on their approval.";
/// Symlinks followed while resolving one path, as in most kernels.
const MAX_LINKS: u8 = 40;

/// How a wait on an approval ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ApprovalWait {
    Approved,
    Denied {
        reply: Option<String>,
    },
    Expired,
    /// The run is no longer running (Stop, cancel or a restart).
    Stopped,
}

/// Where the approval stands. The run comes first: Stop denies the item and
/// cancels the run together, and a cancelled run gets no answer.
fn settled(
    connection: &Connection,
    run_id: &str,
    item_id: &str,
) -> Result<Option<ApprovalWait>, ReactiveError> {
    if store::run_record(connection, run_id)?.status != RunStatus::Running {
        store::crew::withdraw_approval(connection, item_id)?;
        return Ok(Some(ApprovalWait::Stopped));
    }
    let item = store::crew::approval(connection, item_id)?;
    Ok(match item.approval.map(|request| request.state) {
        Some(ApprovalState::Approved) => Some(ApprovalWait::Approved),
        Some(ApprovalState::Denied) => Some(ApprovalWait::Denied { reply: item.answer }),
        _ => None,
    })
}

/// Wait for the owner's answer to approval `item_id`, until `timeout`.
pub(super) async fn wait(
    runtime: &ReactiveRuntime,
    run_id: &str,
    item_id: &str,
    timeout: Duration,
) -> Result<ApprovalWait, ReactiveError> {
    let deadline = tokio::time::Instant::now() + timeout;
    // Subscribed before the first read, so no answer slips between them.
    let mut changes = runtime.watch_changes();
    loop {
        let (run, item) = (run_id.to_owned(), item_id.to_owned());
        let outcome = runtime
            .call(move |connection| {
                store::transaction(connection, |connection| settled(connection, &run, &item))
            })
            .await?;
        if let Some(outcome) = outcome {
            if outcome == ApprovalWait::Stopped {
                runtime.changed();
            }
            return Ok(outcome);
        }
        tokio::select! {
            changed = changes.changed() => {
                if changed.is_err() {
                    return Ok(ApprovalWait::Stopped);
                }
            }
            () = tokio::time::sleep_until(deadline) => {
                let (run, item) = (run_id.to_owned(), item_id.to_owned());
                let outcome = runtime
                    .call(move |connection| {
                        store::transaction(connection, |connection| {
                            if let Some(outcome) = settled(connection, &run, &item)? {
                                return Ok(outcome);
                            }
                            store::crew::expire_approval(connection, &item)?;
                            Ok(ApprovalWait::Expired)
                        })
                    })
                    .await?;
                runtime.changed();
                return Ok(outcome);
            }
        }
    }
}

/// What happened when the run asked to open an approval.
enum Opened {
    Stopped,
    Granted,
    Waiting(String),
}

/// The answer to one tool call of an unattended bruh run: the gate's own
/// answer, a late approval's one-shot grant, or the owner's answer to a new
/// approval. `None` means the run stopped while it waited.
#[expect(
    clippy::too_many_arguments,
    reason = "one call site in the worker passes the run's fixed context"
)]
pub(super) async fn decide(
    runtime: &ReactiveRuntime,
    access: &BruhAccess,
    provider: AgentProvider,
    run: &ReactiveRun,
    project_path: &str,
    tool: &str,
    input: &Value,
    timeout: Duration,
) -> Result<Option<PermissionDecision>, ReactiveError> {
    let (input, resolved) = resolved_input(input);
    // A path that can't be resolved never counts as inside the folder.
    let project = if resolved {
        resolve(Path::new(project_path), 0)
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        String::new()
    };
    let confined = crate::agent::unattended_shell_confined(access, provider);
    match permission_gate(access, provider, tool, &input, &project, confined) {
        Gate::Allow => return Ok(Some(PermissionDecision::Allow)),
        Gate::Deny(reason) => return Ok(Some(deny(reason))),
        Gate::Approve => {}
    }
    let fingerprint = approval_fingerprint(tool, &input);
    let (opening, tool) = (run.clone(), tool.to_owned());
    let opened = runtime
        .call(move |connection| {
            store::transaction(connection, |connection| {
                if store::run_record(connection, &opening.id)?.status != RunStatus::Running {
                    return Ok(Opened::Stopped);
                }
                if store::crew::take_grant(connection, &opening.agent_id, &fingerprint)? {
                    return Ok(Opened::Granted);
                }
                store::crew::open_approval(connection, &opening, &tool, &input).map(Opened::Waiting)
            })
        })
        .await?;
    let item_id = match opened {
        Opened::Stopped => return Ok(None),
        Opened::Granted => return Ok(Some(PermissionDecision::Allow)),
        Opened::Waiting(id) => id,
    };
    runtime.changed();
    Ok(match wait(runtime, &run.id, &item_id, timeout).await? {
        ApprovalWait::Approved => Some(PermissionDecision::Allow),
        ApprovalWait::Denied { reply: None } => Some(deny(DENIED)),
        ApprovalWait::Denied { reply: Some(reply) } => {
            Some(deny(&format!("The owner denied this and replied: {reply}")))
        }
        ApprovalWait::Expired => Some(deny(NOT_ANSWERED)),
        ApprovalWait::Stopped => None,
    })
}

fn deny(reason: &str) -> PermissionDecision {
    PermissionDecision::Deny {
        reason: Some(reason.into()),
    }
}

/// `input` with its file paths resolved, so a symlink inside the folder
/// can't lead a write outside it; false when one couldn't be resolved.
fn resolved_input(input: &Value) -> (Value, bool) {
    let mut input = input.clone();
    let mut resolved = true;
    for key in ["file_path", "notebook_path"] {
        let Some(path) = input[key].as_str().map(Path::new) else {
            continue;
        };
        if !path.is_absolute() {
            // The gate never counts a relative path as inside.
            continue;
        }
        match resolve(path, 0) {
            Some(real) => input[key] = Value::from(real.to_string_lossy().into_owned()),
            None => resolved = false,
        }
    }
    (input, resolved)
}

/// The real path `path` names: symlinks resolved, including dangling ones,
/// and for a path that doesn't exist yet its nearest existing ancestor.
fn resolve(path: &Path, links: u8) -> Option<PathBuf> {
    if links > MAX_LINKS {
        return None;
    }
    for ancestor in path.ancestors() {
        let rest = path.strip_prefix(ancestor).ok()?;
        let join = |base: PathBuf| {
            if rest.as_os_str().is_empty() {
                base
            } else {
                base.join(rest)
            }
        };
        if let Ok(real) = std::fs::canonicalize(ancestor) {
            return Some(join(real));
        }
        // A link whose target doesn't exist yet: a write would create the target.
        if let Ok(target) = std::fs::read_link(ancestor) {
            let base = ancestor.parent().unwrap_or(ancestor).join(target);
            return resolve(&join(base), links + 1);
        }
    }
    None
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn dangling_links_resolve_to_their_target() {
        let (folder, outside) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let target = outside.path().join("later.txt");
        std::os::unix::fs::symlink(&target, folder.path().join("dangling")).unwrap();
        let real = std::fs::canonicalize(outside.path())
            .unwrap()
            .join("later.txt");
        assert_eq!(resolve(&folder.path().join("dangling"), 0), Some(real));
        std::os::unix::fs::symlink(folder.path().join("b"), folder.path().join("a")).unwrap();
        std::os::unix::fs::symlink(folder.path().join("a"), folder.path().join("b")).unwrap();
        assert_eq!(resolve(&folder.path().join("a").join("x"), 0), None);
        let (_, resolved) = resolved_input(
            &serde_json::json!({"file_path": folder.path().join("a").to_string_lossy()}),
        );
        assert!(!resolved);
    }
}
