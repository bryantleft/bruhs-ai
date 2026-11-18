//! A bruh's risky steps wait on the owner. The access gate decides each tool
//! call of an unattended run; a call that needs approval becomes an outbox
//! item, and the run waits for the answer, a timeout or Stop. While the
//! owner has the bruh's computer, every call waits first for it to come
//! back. A browser call is gated on the page it acts on, and each one the
//! gate lets through leaves a one-shot allowance the command route uses up.

use super::{store, ReactiveError, ReactiveRuntime};
use crate::PermissionDecision;
use bruhs_models::{
    agent::AgentProvider,
    crew::{
        approval_fingerprint, browser_gate, permission_gate, same_page_host, ApprovalState,
        BruhAccess, Gate, BRUH_BROWSER_TOOL, CONTROL_WAIT_EXPIRED, PAGE_CHANGED,
    },
    local_tools::{BrowserAction, BrowserActionKind, Control},
    reactive::{ReactiveRun, RunStatus},
};
use rusqlite::Connection;
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};
use tokio::time::Instant;

const DENIED: &str = "The owner denied this. Continue without it.";
const NOT_ANSWERED: &str =
    "The owner has not answered yet. Stop here and say you are waiting on their approval.";
/// Symlinks followed while resolving one path, as in most kernels.
const MAX_LINKS: u8 = 40;
/// How long an allowed browser step waits for the helper's request: the
/// helper's own request deadline.
const ALLOWANCE_TTL: Duration = crate::capture_client::REQUEST_TIMEOUT;

/// A browser step the gate let through, kept until the bruh's helper sends
/// it to the command route. One request uses it up.
#[derive(Debug, Clone, PartialEq)]
pub struct BrowserAllowance {
    pub agent_id: String,
    pub run_id: String,
    /// The step exactly as the gate saw it: the tool input's `command`.
    pub command: Value,
    /// The page the gate saw it act on, when known.
    pub page_url: Option<String>,
    expires: Instant,
}

/// The browser steps allowed and not yet sent, one entry per allowed call,
/// so identical calls in parallel each run once.
#[derive(Debug, Default)]
pub(super) struct Allowances(Mutex<Vec<BrowserAllowance>>);

impl Allowances {
    fn list(&self) -> std::sync::MutexGuard<'_, Vec<BrowserAllowance>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn record(&self, allowance: BrowserAllowance) {
        let mut list = self.list();
        let now = Instant::now();
        list.retain(|kept| kept.expires > now);
        list.push(allowance);
    }

    /// Take the newest allowance for `agent_id`'s exact `command` that is
    /// still good at `now`. Expired ones go.
    fn take(&self, agent_id: &str, command: &Value, now: Instant) -> Option<BrowserAllowance> {
        let mut list = self.list();
        list.retain(|kept| kept.expires > now);
        let index = list
            .iter()
            .rposition(|kept| kept.agent_id == agent_id && &kept.command == command)?;
        Some(list.remove(index))
    }

    /// Drop every allowance of `run_id`'s attempts so far. A claim calls it
    /// as the run starts (again), since a retry keeps the run's id.
    pub(super) fn forget_run(&self, run_id: &str) {
        self.list().retain(|kept| kept.run_id != run_id);
    }
}

impl ReactiveRuntime {
    /// Use up the allowance the gate left for this exact browser step of a
    /// running run of `agent_id`. `None` when the gate never let it
    /// through, it already ran or expired, or its run stopped.
    pub async fn consume_browser_allowance(
        &self,
        agent_id: &str,
        command: &Value,
    ) -> Result<Option<BrowserAllowance>, ReactiveError> {
        let (allowances, agent_id, command) = (
            Arc::clone(&self.allowances),
            agent_id.to_owned(),
            command.clone(),
        );
        // Taken and checked under the database lock, which a claim holds
        // while it forgets a retried run's old allowances: an old one can't
        // be taken before that and found Running after it.
        self.call(move |connection| {
            while let Some(allowance) = allowances.take(&agent_id, &command, Instant::now()) {
                let status = match store::run_record(connection, &allowance.run_id) {
                    Ok(run) => Some(run.status),
                    Err(ReactiveError::NotFound(_)) => None,
                    Err(error) => return Err(error),
                };
                if status == Some(RunStatus::Running) {
                    return Ok(Some(allowance));
                }
            }
            Ok(None)
        })
        .await
    }

    /// Claim the next queued run, forgetting the allowances its earlier
    /// attempts left, under the same database lock.
    pub(super) async fn claim_next(&self) -> Result<Option<store::Claim>, ReactiveError> {
        let allowances = Arc::clone(&self.allowances);
        self.call(move |connection| {
            let claim = store::claim_next(connection)?;
            if let Some(claim) = &claim {
                allowances.forget_run(&claim.run.id);
            }
            Ok(claim)
        })
        .await
    }
}

/// How a wait for the owner to hand the bruh's computer back ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ComputerWait {
    /// The bruh has its computer, or no longer has one to wait for.
    Free,
    /// The run is no longer running (Stop, cancel or a restart).
    Stopped,
    StillHeld,
}

/// Wait while the owner holds the bruh's computer, until `timeout`. Free
/// at once without a host, a slot or the owner's control, and when the slot
/// goes (Computer off or a delete).
pub(super) async fn wait_for_computer(
    runtime: &ReactiveRuntime,
    run: &ReactiveRun,
    timeout: Duration,
) -> Result<ComputerWait, ReactiveError> {
    let Some(mut control) = runtime
        .computers()
        .and_then(|host| host.control_changes(&run.agent_id))
    else {
        return Ok(ComputerWait::Free);
    };
    let deadline = Instant::now() + timeout;
    // Subscribed before the first read, so a Stop can't slip between them.
    let mut changes = runtime.watch_changes();
    let mut read_run = true;
    loop {
        if control.borrow_and_update().control != Control::Owner {
            return Ok(ComputerWait::Free);
        }
        // Only after a store change: the state also changes as frames
        // arrive, which says nothing about the run.
        if read_run {
            let id = run.id.clone();
            let status = runtime
                .call(move |connection| store::run_record(connection, &id).map(|run| run.status))
                .await?;
            if status != RunStatus::Running {
                return Ok(ComputerWait::Stopped);
            }
            read_run = false;
        }
        tokio::select! {
            changed = control.changed() => {
                if changed.is_err() {
                    return Ok(ComputerWait::Free);
                }
            }
            changed = changes.changed() => {
                if changed.is_err() {
                    return Ok(ComputerWait::Stopped);
                }
                read_run = true;
            }
            () = tokio::time::sleep_until(deadline) => return Ok(ComputerWait::StillHeld),
        }
    }
}

/// The page the bruh's browser is on, asked of it. `None` when unknown: no
/// host or slot, or an open page that didn't answer in time.
async fn page_url(runtime: &ReactiveRuntime, agent_id: &str) -> Option<String> {
    match runtime.computers() {
        Some(host) => host.page_url(agent_id).await,
        None => None,
    }
}

/// A browser call as the gate, the card, the fingerprint and a grant see
/// it: the command and the page it acts on. Anything else in the tool's
/// input, a `page_url` the model wrote included, is dropped.
fn browser_call(input: &Value, page_url: Option<String>) -> Value {
    let command = input.get("command").cloned().unwrap_or(Value::Null);
    let mut call = serde_json::json!({ "command": command });
    if let Some(url) = page_url {
        call["page_url"] = Value::from(url);
    }
    call
}

/// Whether an approved input would now land on another page than the one
/// the owner approved it on.
async fn page_moved(runtime: &ReactiveRuntime, agent_id: &str, call: &Value) -> bool {
    let input = serde_json::from_value::<BrowserAction>(call["command"].clone())
        .is_ok_and(|action| action.kind() == BrowserActionKind::Input);
    if !input {
        return false;
    }
    let now = page_url(runtime, agent_id).await;
    !same_page_host(call["page_url"].as_str(), now.as_deref())
}

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
/// approval. Every call first waits while the owner has the bruh's
/// computer. `None` means the run stopped while it waited.
///
/// The owner's hold and the approval are separate waits, each bounded by
/// `timeout`, so one call waits at most twice `timeout`: an approval asked
/// after a long hold still gets its whole time.
#[expect(
    clippy::too_many_arguments,
    reason = "one call site in the worker passes the run's fixed context"
)]
pub(super) async fn decide(
    runtime: &ReactiveRuntime,
    access: &BruhAccess,
    provider: AgentProvider,
    computer: bool,
    run: &ReactiveRun,
    project_path: &str,
    tool: &str,
    input: &Value,
    timeout: Duration,
) -> Result<Option<PermissionDecision>, ReactiveError> {
    match wait_for_computer(runtime, run, timeout).await? {
        ComputerWait::Free => {}
        ComputerWait::Stopped => return Ok(None),
        ComputerWait::StillHeld => return Ok(Some(deny(CONTROL_WAIT_EXPIRED))),
    }
    let browser = tool == BRUH_BROWSER_TOOL;
    let (input, gate) = if browser {
        let call = browser_call(input, page_url(runtime, &run.agent_id).await);
        let gate = browser_gate(access, provider, computer, &call, call["page_url"].as_str());
        (call, gate)
    } else {
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
        let gate = permission_gate(access, provider, tool, &input, &project, confined);
        (input, gate)
    };
    // Each allowed browser call may reach the command route once.
    let allow = |input: &Value| {
        if browser {
            runtime.allowances.record(BrowserAllowance {
                agent_id: run.agent_id.clone(),
                run_id: run.id.clone(),
                command: input["command"].clone(),
                page_url: input["page_url"].as_str().map(str::to_owned),
                expires: Instant::now() + ALLOWANCE_TTL,
            });
        }
        Some(PermissionDecision::Allow)
    };
    match gate {
        Gate::Allow => return Ok(allow(&input)),
        Gate::Deny(reason) => return Ok(Some(deny(reason))),
        Gate::Approve => {}
    }
    let fingerprint = approval_fingerprint(tool, &input);
    let (opening, tool, card) = (run.clone(), tool.to_owned(), input.clone());
    let opened = runtime
        .call(move |connection| {
            store::transaction(connection, |connection| {
                if store::run_record(connection, &opening.id)?.status != RunStatus::Running {
                    return Ok(Opened::Stopped);
                }
                if store::crew::take_grant(connection, &opening.agent_id, &fingerprint)? {
                    return Ok(Opened::Granted);
                }
                store::crew::open_approval(connection, &opening, &tool, &card).map(Opened::Waiting)
            })
        })
        .await?;
    let item_id = match opened {
        Opened::Stopped => return Ok(None),
        // The grant was made for this page, so it needs no second look.
        Opened::Granted => return Ok(allow(&input)),
        Opened::Waiting(id) => id,
    };
    runtime.changed();
    Ok(match wait(runtime, &run.id, &item_id, timeout).await? {
        // The page may have moved during the wait: input goes only where
        // the owner approved it.
        ApprovalWait::Approved if browser && page_moved(runtime, &run.agent_id, &input).await => {
            Some(deny(PAGE_CHANGED))
        }
        ApprovalWait::Approved => allow(&input),
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
    fn an_allowance_lasts_until_the_helpers_deadline() {
        let allowances = Allowances::default();
        let command = serde_json::json!({"action": "snapshot"});
        let allowance = |expires| BrowserAllowance {
            agent_id: "a".into(),
            run_id: "r".into(),
            command: command.clone(),
            page_url: None,
            expires,
        };
        let now = Instant::now();
        let deadline = now + ALLOWANCE_TTL;
        allowances.record(allowance(deadline));
        let just_before = deadline - Duration::from_millis(1);
        assert!(allowances.take("a", &command, just_before).is_some());
        allowances.record(allowance(deadline));
        assert!(allowances.take("a", &command, deadline).is_none());
        // An expired allowance is gone for good.
        assert!(allowances.take("a", &command, just_before).is_none());
    }

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
