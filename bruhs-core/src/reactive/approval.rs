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
        approval_fingerprint, browser_gate, network_host, permission_gate, same_page_host,
        ApprovalState, BruhAccess, Gate, BRUH_BROWSER_TOOL, CONTROL_WAIT_EXPIRED,
        LINKED_FILE_DENIAL, LOCAL_HOST_DENIAL, PAGE_CHANGED, PAGE_UNKNOWN, PROFILE_DENIAL,
    },
    local_tools::{BrowserAction, BrowserActionKind, Control},
    reactive::{ReactiveRun, RunStatus},
};
use rusqlite::Connection;
use serde_json::Value;
use std::{
    future::Future,
    net::IpAddr,
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
/// How long the gate waits for a network tool call's host to resolve.
const RESOLVE_WAIT: Duration = Duration::from_secs(2);
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
    /// The attempt at the run that was allowed it ([`Allowances::claim`]).
    claim: u64,
    expires: Instant,
}

/// The browser steps allowed and not yet sent, one entry per allowed call,
/// so identical calls in parallel each run once. Each belongs to one claim
/// of its run: a retry keeps the run's id, so an old attempt's worker that
/// hasn't noticed its cancel yet can neither leave a step for the new
/// attempt nor clear the new attempt's steps.
#[derive(Debug, Default)]
pub(super) struct Allowances(Mutex<AllowanceState>);

#[derive(Debug, Default)]
struct AllowanceState {
    list: Vec<BrowserAllowance>,
    /// Each running run's current claim.
    claims: std::collections::HashMap<String, u64>,
    last_claim: u64,
}

impl Allowances {
    fn state(&self) -> std::sync::MutexGuard<'_, AllowanceState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A new claim of `run_id`: earlier attempts' allowances go, and only
    /// this claim may record from now on.
    pub(super) fn claim(&self, run_id: &str) -> u64 {
        let mut state = self.state();
        state.last_claim = state.last_claim.wrapping_add(1);
        let claim = state.last_claim;
        state.claims.insert(run_id.to_owned(), claim);
        state.list.retain(|kept| kept.run_id != run_id);
        claim
    }

    /// The run's current claim, if it has one.
    #[cfg(test)]
    pub(super) fn current_claim(&self, run_id: &str) -> Option<u64> {
        self.state().claims.get(run_id).copied()
    }

    /// Keep `allowance`, unless its claim is no longer its run's current one.
    fn record(&self, allowance: BrowserAllowance) {
        let mut state = self.state();
        if state.claims.get(&allowance.run_id) != Some(&allowance.claim) {
            return;
        }
        let now = Instant::now();
        state.list.retain(|kept| kept.expires > now);
        state.list.push(allowance);
    }

    /// Take the newest allowance for `agent_id`'s exact `command` that is
    /// still good at `now`, from its run's current claim. Expired ones go.
    fn take(&self, agent_id: &str, command: &Value, now: Instant) -> Option<BrowserAllowance> {
        let mut state = self.state();
        let AllowanceState { list, claims, .. } = &mut *state;
        list.retain(|kept| kept.expires > now && claims.get(&kept.run_id) == Some(&kept.claim));
        let taken = list
            .iter()
            .rposition(|kept| kept.agent_id == agent_id && &kept.command == command)
            .map(|index| list.remove(index));
        drop(state);
        taken
    }

    /// Claim `claim` of `run_id` has ended: its allowances go, and the run
    /// has no current claim unless a newer one took over.
    pub(super) fn forget_run(&self, run_id: &str, claim: u64) {
        let mut state = self.state();
        state
            .list
            .retain(|kept| kept.run_id != run_id || kept.claim != claim);
        if state.claims.get(run_id) == Some(&claim) {
            state.claims.remove(run_id);
        }
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

    /// Claim the next queued run as a worker would, without running it.
    /// For route tests outside this crate.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn claim_run_for_tests(&self) -> Result<Option<ReactiveRun>, ReactiveError> {
        Ok(self.claim_next().await?.map(|claim| claim.run))
    }

    /// Let `command` through for `run`, on `page_url`, as the gate does for
    /// an allowed browser call. For route tests outside this crate.
    #[cfg(any(test, feature = "test-support"))]
    pub fn allow_browser_step_for_tests(
        &self,
        run: &ReactiveRun,
        command: Value,
        page_url: Option<String>,
    ) {
        let Some(claim) = self.allowances.state().claims.get(&run.id).copied() else {
            return;
        };
        self.allowances.record(BrowserAllowance {
            agent_id: run.agent_id.clone(),
            run_id: run.id.clone(),
            command,
            page_url,
            claim,
            expires: Instant::now() + ALLOWANCE_TTL,
        });
    }

    /// Claim the next queued run as a new attempt, forgetting the
    /// allowances its earlier attempts left, under the same database lock.
    pub(super) async fn claim_next(&self) -> Result<Option<store::Claim>, ReactiveError> {
        let allowances = Arc::clone(&self.allowances);
        self.call(move |connection| {
            let mut claim = store::claim_next(connection)?;
            if let Some(claim) = &mut claim {
                claim.generation = allowances.claim(&claim.run.id);
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

/// Whether a browser call is input, which acts on the page it lands on.
fn is_input(call: &Value) -> bool {
    serde_json::from_value::<BrowserAction>(call["command"].clone())
        .is_ok_and(|action| action.kind() == BrowserActionKind::Input)
}

/// Why an approved input can't go now: the page moved off the host the
/// owner approved it on, or no longer says where it is.
async fn page_moved(
    runtime: &ReactiveRuntime,
    agent_id: &str,
    call: &Value,
) -> Option<&'static str> {
    if !is_input(call) {
        return None;
    }
    match page_url(runtime, agent_id).await {
        None => Some(PAGE_UNKNOWN),
        Some(now) if !same_page_host(call["page_url"].as_str(), Some(&now)) => Some(PAGE_CHANGED),
        Some(_) => None,
    }
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
/// computer. `None` means the run stopped while it waited. `claim` is the
/// worker's attempt at `run`, which an allowed browser step belongs to.
///
/// The owner's hold and the approval are separate waits, each bounded by
/// `timeout`, and an approved browser step waits for the hold once more,
/// so one call waits at most three times `timeout`: an approval asked
/// after a long hold still gets its whole time.
///
/// A file tool call is refused first when it reaches a browser profile by
/// another name (`private_file_refusal`). Claude itself allows Read,
/// Grep and Glob at Ask and Full, so only the calls it asks about reach
/// this check.
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
    claim: u64,
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
        // The flag is from the run's start: a Computer turned off since has
        // no slot, and its steps are refused at once instead of asked about.
        let computer = computer
            && runtime
                .computers()
                .is_some_and(|host| host.has_computer(&run.agent_id));
        let call = browser_call(input, page_url(runtime, &run.agent_id).await);
        let gate = browser_gate(access, provider, computer, &call, call["page_url"].as_str());
        // Input on a page the browser doesn't name could land anywhere, so
        // nobody is asked about it, and the route couldn't check it: a
        // snapshot names the page first.
        if matches!(gate, Gate::Allow | Gate::Approve)
            && call.get("page_url").is_none()
            && is_input(&call)
        {
            return Ok(Some(deny(PAGE_UNKNOWN)));
        }
        (call, gate)
    } else {
        // The gate reads only the name, and a name can point here.
        if let Some(host) = network_host(tool, input) {
            if reaches_this_computer(&host, system_lookup).await {
                return Ok(Some(deny(LOCAL_HOST_DENIAL)));
            }
        }
        if let Some(reason) = private_file_refusal(&runtime.home, project_path, tool, input) {
            return Ok(Some(deny(reason)));
        }
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
                claim,
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
        ApprovalWait::Approved if browser => {
            // The owner may have taken the computer before approving: the
            // step waits for it again, or the route would refuse it and the
            // retry would ask twice.
            match wait_for_computer(runtime, run, timeout).await? {
                ComputerWait::Free => {}
                ComputerWait::Stopped => return Ok(None),
                ComputerWait::StillHeld => return Ok(Some(deny(CONTROL_WAIT_EXPIRED))),
            }
            // The page may have moved during the waits: input goes only
            // where the owner approved it.
            match page_moved(runtime, &run.agent_id, &input).await {
                Some(reason) => Some(deny(reason)),
                None => allow(&input),
            }
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

/// Whether a network tool call's `host` is this computer: an address
/// literal, or a name `lookup` resolves within [`RESOLVE_WAIT`], counts when
/// any address is local or on one of this computer's interfaces
/// ([`crate::local_tools::is_this_computer`]). A name that doesn't resolve
/// in time passes on to the gate. The connection resolves again later, so
/// a name whose DNS changes in between still gets through (rebinding).
async fn reaches_this_computer<F, Fut>(host: &str, lookup: F) -> bool
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = std::io::Result<Vec<IpAddr>>>,
{
    if let Ok(ip) = host.parse::<IpAddr>() {
        return crate::local_tools::is_this_computer(ip);
    }
    match tokio::time::timeout(RESOLVE_WAIT, lookup(host.to_owned())).await {
        Ok(Ok(addresses)) => addresses
            .into_iter()
            .any(crate::local_tools::is_this_computer),
        _ => false,
    }
}

async fn system_lookup(host: String) -> std::io::Result<Vec<IpAddr>> {
    Ok(tokio::net::lookup_host((host.as_str(), 0))
        .await?
        .map(|address| address.ip())
        .collect())
}

fn deny(reason: &str) -> PermissionDecision {
    PermissionDecision::Deny {
        reason: Some(reason.into()),
    }
}

/// File tools, whose `file_path`, `notebook_path` or `path` names a target.
const FILE_TOOLS: [&str; 8] = [
    "Read",
    "Edit",
    "Write",
    "MultiEdit",
    "NotebookEdit",
    "Grep",
    "Glob",
    "LS",
];
/// The file tools that open the one file they name.
const OPENING_TOOLS: [&str; 5] = ["Read", "Edit", "Write", "MultiEdit", "NotebookEdit"];

/// Why a file tool call must not reach its target. The provider's deny
/// rules (`computers::hidden_folders`) match a path by name, so this looks
/// at what the path really is:
///
/// - Resolved (symlinks followed, a relative path taken from `project`),
///   a path in any `<home>/bruhs/*/browser*` folder, compared without case
///   as case-insensitive volumes do, gets [`PROFILE_DENIAL`].
/// - A tool that opens one file gets [`LINKED_FILE_DENIAL`] for a regular
///   file under `<home>/bruhs` with more than one name: a hard link to a
///   profile file can sit anywhere in a bruh's folder.
///
/// A path that can't be resolved is left to the gate and the provider.
fn private_file_refusal(
    home: &Path,
    project: &str,
    tool: &str,
    input: &Value,
) -> Option<&'static str> {
    if !FILE_TOOLS.contains(&tool) {
        return None;
    }
    let trees = home.join(super::computers::TREES);
    for key in ["file_path", "notebook_path", "path"] {
        let Some(named) = input[key].as_str().filter(|path| !path.is_empty()) else {
            continue;
        };
        // Joining an absolute path replaces `project`.
        let named = Path::new(project).join(named);
        if !named.is_absolute() {
            continue;
        }
        let Some(real) = resolve(&named, 0) else {
            continue;
        };
        let Some(mut inside) = below(&trees, &real) else {
            continue;
        };
        let in_profile = inside.nth(1).is_some_and(|folder| {
            folder
                .as_os_str()
                .to_string_lossy()
                .to_ascii_lowercase()
                .starts_with("browser")
        });
        if in_profile {
            return Some(PROFILE_DENIAL);
        }
        if OPENING_TOOLS.contains(&tool) && has_other_names(&real) {
            return Some(LINKED_FILE_DENIAL);
        }
    }
    None
}

/// The rest of `path` below `folder`, both compared by component and
/// without case; `None` when `path` isn't inside it.
fn below<'a>(folder: &Path, path: &'a Path) -> Option<std::path::Components<'a>> {
    let mut rest = path.components();
    for want in folder.components() {
        let have = rest.next()?;
        if !have
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&want.as_os_str().to_string_lossy())
        {
            return None;
        }
    }
    Some(rest)
}

/// Whether `path` is a regular file with more than one hard link.
#[cfg(unix)]
fn has_other_names(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.nlink() > 1)
}

/// Off Unix, no bruh browser runs (`COMPUTER_UNSUPPORTED`), so no profile
/// file can be linked.
#[cfg(not(unix))]
fn has_other_names(_path: &Path) -> bool {
    false
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
        let claim = allowances.claim("r");
        let allowance = |expires| BrowserAllowance {
            agent_id: "a".into(),
            run_id: "r".into(),
            command: command.clone(),
            page_url: None,
            claim,
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

    /// An old attempt's worker, still running after a cancel and a retry
    /// claimed the run again, can neither leave a step for the new attempt
    /// nor clear the new attempt's steps when it ends.
    #[test]
    fn an_old_attempt_never_records_into_or_forgets_a_new_one() {
        let allowances = Allowances::default();
        let command = serde_json::json!({"action": "snapshot"});
        let allowance = |claim| BrowserAllowance {
            agent_id: "a".into(),
            run_id: "r".into(),
            command: command.clone(),
            page_url: None,
            claim,
            expires: Instant::now() + ALLOWANCE_TTL,
        };
        let old = allowances.claim("r");
        allowances.record(allowance(old));
        let new = allowances.claim("r");
        assert!(
            allowances.take("a", &command, Instant::now()).is_none(),
            "a claim forgets earlier attempts"
        );
        allowances.record(allowance(old));
        assert!(allowances.take("a", &command, Instant::now()).is_none());
        allowances.record(allowance(new));
        allowances.forget_run("r", old);
        assert_eq!(allowances.current_claim("r"), Some(new));
        let taken = allowances.take("a", &command, Instant::now()).unwrap();
        assert_eq!(taken.claim, new);
        // Once the new attempt ends, nothing records for the run.
        allowances.forget_run("r", new);
        assert_eq!(allowances.current_claim("r"), None);
        allowances.record(allowance(new));
        assert!(allowances.take("a", &command, Instant::now()).is_none());
    }

    /// A name that resolves here is this computer, whatever it reads as;
    /// one that doesn't resolve in time passes on to the gate.
    #[tokio::test(start_paused = true)]
    async fn a_name_that_resolves_here_reaches_this_computer() {
        let answer = |ips: &'static [&'static str]| {
            move |_: String| async move {
                Ok(ips
                    .iter()
                    .map(|ip| ip.parse().unwrap())
                    .collect::<Vec<IpAddr>>())
            }
        };
        assert!(reaches_this_computer("rebind.example", answer(&["127.0.0.1"])).await);
        assert!(reaches_this_computer("split.example", answer(&["93.184.216.34", "::1"])).await);
        assert!(reaches_this_computer("mapped.example", answer(&["::ffff:127.0.0.1"])).await);
        assert!(!reaches_this_computer("cdn.example", answer(&["93.184.216.34"])).await);
        let failed = |_: String| async { Err(std::io::Error::other("no such name")) };
        assert!(!reaches_this_computer("nowhere.example", failed).await);
        let never = |_: String| std::future::pending::<std::io::Result<Vec<IpAddr>>>();
        assert!(!reaches_this_computer("slow.example", never).await);
        assert!(reaches_this_computer("::1", never).await);
        // This computer's own interface address, by literal.
        if let Some(own) = own_address() {
            assert!(
                reaches_this_computer(&own.to_string(), never).await,
                "{own}"
            );
        }
    }

    /// One of this computer's interface addresses that isn't loopback or
    /// link-local, if it has one.
    fn own_address() -> Option<IpAddr> {
        crate::local_tools::interface_addresses_for_tests()
            .into_iter()
            .find(|ip| match ip {
                IpAddr::V4(ip) => !ip.is_loopback() && !ip.is_link_local(),
                IpAddr::V6(_) => false,
            })
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
