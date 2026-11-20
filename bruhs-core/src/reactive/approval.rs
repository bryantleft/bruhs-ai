//! A bruh's risky steps wait on the owner. The access gate decides each tool
//! call of an unattended run; a call that needs approval becomes an outbox
//! item, and the run waits for the answer, a timeout or Stop. While the
//! owner has the bruh's computer, every call waits first for it to come
//! back. A browser call is gated on the page it acts on, and an app or mail
//! call on the account or mailbox it goes out as, looked up live. Each one
//! the gate lets through leaves a one-shot allowance its route uses up.

use super::{store, ReactiveError, ReactiveRuntime};
use crate::PermissionDecision;
use bruhs_models::{
    agent::AgentProvider,
    crew::{
        app_card_input, app_gate, approval_fingerprint, apps_unavailable, browser_gate,
        is_app_tool, list_apps_gate, mail_card_input, mail_gate, network_host, owner_only_card,
        permission_gate, same_page_host, AppLookup, ApprovalState, BruhAccess, Gate, MailLookup,
        APP_REQUEST_TOOL, BRUH_BROWSER_TOOL, CONTROL_WAIT_EXPIRED, LINKED_FILE_DENIAL,
        LOCAL_HOST_DENIAL, MAIL_RATE_LIMITED, MAIL_SENDS_PER_HOUR, PAGE_CHANGED, PAGE_UNKNOWN,
        PROFILE_DENIAL, SEND_MAIL_TOOL, VAULT_COMMAND_RUNNING, VAULT_FILE_DENIAL,
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
/// How long an allowed step waits for the helper's request: the helper's
/// own request deadline.
const ALLOWANCE_TTL: Duration = crate::capture_client::REQUEST_TIMEOUT;

/// A bruh step the gate let through, kept until the bruh's helper sends it
/// to its route: a browser step, an app call or an email. One request uses
/// it up.
#[derive(Debug, Clone, PartialEq)]
pub struct StepAllowance {
    pub agent_id: String,
    pub run_id: String,
    /// The tool the gate let through.
    pub tool: String,
    /// The step exactly as the bruh sent it: a browser call's `command`,
    /// or an app or mail tool's whole input, never the card's.
    pub command: Value,
    /// The page the gate saw a browser step act on, when known.
    pub page_url: Option<String>,
    /// The vault entry an app call or the mailbox an email goes out as,
    /// as the gate looked it up (ruling F10). The route's live lookup must
    /// find the same one.
    pub target: Option<String>,
    /// The owner approved this call on its card (or a late grant they
    /// left), rather than the gate allowing it outright. The route's second
    /// gate takes an Approve only from such an allowance, so an access
    /// lowered just after the gate can't let a call through unasked (final
    /// review BC M2).
    pub owner_approved: bool,
    /// The attempt at the run that was allowed it ([`Allowances::claim`]).
    claim: u64,
    expires: Instant,
}

/// How long [`MailSends`] remembers a send.
const MAIL_SEND_WINDOW: Duration = Duration::from_secs(60 * 60);

/// When each bruh's emails of the last hour went out, so no bruh sends more
/// than [`MAIL_SENDS_PER_HOUR`] (final review A2 M7). Kept in memory: a
/// restart forgets it.
#[derive(Debug, Default)]
pub(super) struct MailSends(Mutex<std::collections::HashMap<String, Vec<Instant>>>);

impl MailSends {
    /// `agent_id`'s sends still inside the window at `now`.
    fn recent<'a>(
        sends: &'a mut std::collections::HashMap<String, Vec<Instant>>,
        agent_id: &str,
        now: Instant,
    ) -> &'a mut Vec<Instant> {
        let recent = sends.entry(agent_id.to_owned()).or_default();
        recent.retain(|sent| now.saturating_duration_since(*sent) < MAIL_SEND_WINDOW);
        recent
    }

    /// Whether `agent_id` may send another email at `now`.
    fn has_room(&self, agent_id: &str, now: Instant) -> bool {
        let mut sends = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        Self::recent(&mut sends, agent_id, now).len() < MAIL_SENDS_PER_HOUR
    }

    /// Count one email of `agent_id` at `now`, if it has room: false
    /// otherwise.
    fn take(&self, agent_id: &str, now: Instant) -> bool {
        let mut sends = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let recent = Self::recent(&mut sends, agent_id, now);
        let room = recent.len() < MAIL_SENDS_PER_HOUR;
        if room {
            recent.push(now);
        }
        drop(sends);
        room
    }
}

/// The steps allowed and not yet sent, one entry per allowed call,
/// so identical calls in parallel each run once. Each belongs to one claim
/// of its run: a retry keeps the run's id, so an old attempt's worker that
/// hasn't noticed its cancel yet can neither leave a step for the new
/// attempt nor clear the new attempt's steps.
#[derive(Debug, Default)]
pub(super) struct Allowances(Mutex<AllowanceState>);

#[derive(Debug, Default)]
struct AllowanceState {
    list: Vec<StepAllowance>,
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

    /// Whether `claim` is still `run_id`'s current attempt.
    fn is_current(&self, run_id: &str, claim: u64) -> bool {
        self.state().claims.get(run_id) == Some(&claim)
    }

    /// Keep `allowance`, unless its claim is no longer its run's current one.
    fn record(&self, allowance: StepAllowance) {
        let mut state = self.state();
        if state.claims.get(&allowance.run_id) != Some(&allowance.claim) {
            return;
        }
        let now = Instant::now();
        state.list.retain(|kept| kept.expires > now);
        state.list.push(allowance);
    }

    /// Take the newest allowance for `agent_id`'s exact `command` of
    /// `tool` that is still good at `now`, from its run's current claim.
    /// Expired ones go.
    fn take(
        &self,
        agent_id: &str,
        tool: &str,
        command: &Value,
        now: Instant,
    ) -> Option<StepAllowance> {
        let mut state = self.state();
        let AllowanceState { list, claims, .. } = &mut *state;
        list.retain(|kept| kept.expires > now && claims.get(&kept.run_id) == Some(&kept.claim));
        let taken = list
            .iter()
            .rposition(|kept| {
                kept.agent_id == agent_id && kept.tool == tool && &kept.command == command
            })
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
    /// Count one email bruh `agent_id` is about to send: false, and nothing
    /// counted, when it has sent [`MAIL_SENDS_PER_HOUR`] in the last hour.
    /// The route calls it just before the send.
    pub fn take_mail_send(&self, agent_id: &str) -> bool {
        self.mail_sends.take(agent_id, Instant::now())
    }

    /// Use up the allowance the gate left for this exact browser step of a
    /// running run of `agent_id`. `None` when the gate never let it
    /// through, it already ran or expired, or its run stopped.
    pub async fn consume_browser_allowance(
        &self,
        agent_id: &str,
        command: &Value,
    ) -> Result<Option<StepAllowance>, ReactiveError> {
        self.consume_allowance(agent_id, BRUH_BROWSER_TOOL, command)
            .await
    }

    /// Use up the allowance the gate left for this exact call of app or
    /// mail `tool` (`input` as the helper sent it, never the card's) by a
    /// running run of `agent_id`. `None` when the gate never let it
    /// through, it already ran or expired, its run stopped, or `tool` is
    /// no app or mail tool. The allowance's `target` is the account or
    /// mailbox the gate picked, which the route checks against its own
    /// live lookup.
    pub async fn consume_app_allowance(
        &self,
        agent_id: &str,
        tool: &str,
        input: &Value,
    ) -> Result<Option<StepAllowance>, ReactiveError> {
        if !is_app_tool(tool) {
            return Ok(None);
        }
        self.consume_allowance(agent_id, tool, input).await
    }

    async fn consume_allowance(
        &self,
        agent_id: &str,
        tool: &str,
        command: &Value,
    ) -> Result<Option<StepAllowance>, ReactiveError> {
        let (allowances, agent_id, tool, command) = (
            Arc::clone(&self.allowances),
            agent_id.to_owned(),
            tool.to_owned(),
            command.clone(),
        );
        // Taken and checked under the database lock, which a claim holds
        // while it forgets a retried run's old allowances: an old one can't
        // be taken before that and found Running after it.
        self.call(move |connection| {
            while let Some(allowance) = allowances.take(&agent_id, &tool, &command, Instant::now())
            {
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

    /// Whether outbox item `id` is a card only the owner's own app may
    /// answer ([`owner_only_card`]): a use of a vault key, an app call or
    /// an email, each in the owner's name. An unknown item is not one.
    pub(super) async fn is_owner_only_approval(&self, id: &str) -> Result<bool, ReactiveError> {
        let id = id.to_owned();
        self.call(
            move |connection| match store::crew::approval(connection, &id) {
                Ok(item) => Ok(item
                    .approval
                    .is_some_and(|request| owner_only_card(&request.tool))),
                Err(ReactiveError::NotFound(_)) => Ok(false),
                Err(error) => Err(error),
            },
        )
        .await
    }

    /// Open an approval of `tool` with card `input` for running `run`, as
    /// the gate does. For route tests outside this crate.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn open_approval_for_tests(
        &self,
        run: &ReactiveRun,
        tool: &str,
        input: &Value,
    ) -> Result<String, ReactiveError> {
        match open(self, run, tool, input, false).await? {
            Opened::Waiting(id) => Ok(id),
            Opened::Stopped | Opened::Granted => {
                Err(ReactiveError::Conflict("The run is not running.".into()))
            }
        }
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
        self.allowances.record(StepAllowance {
            agent_id: run.agent_id.clone(),
            run_id: run.id.clone(),
            tool: BRUH_BROWSER_TOOL.to_owned(),
            command,
            page_url,
            target: None,
            owner_approved: false,
            claim,
            expires: Instant::now() + ALLOWANCE_TTL,
        });
    }

    /// The gate's answer to one call of `run`'s browser tool (`input` is
    /// the tool input, `{"command": …}`) for a Claude bruh at `access`
    /// whose Computer is on, as the worker asks it: an allowed step leaves
    /// its allowance. For route tests outside this crate.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn gate_browser_step_for_tests(
        &self,
        run: &ReactiveRun,
        access: &BruhAccess,
        input: &Value,
    ) -> Result<Option<PermissionDecision>, ReactiveError> {
        let claim = self
            .allowances
            .state()
            .claims
            .get(&run.id)
            .copied()
            .unwrap_or_default();
        decide(
            self,
            access,
            AgentProvider::Claude,
            true,
            run,
            claim,
            "",
            BRUH_BROWSER_TOOL,
            input,
            Duration::from_secs(5),
        )
        .await
    }

    /// The gate's answer to one call of app or mail `tool` by claimed run
    /// `run`, at its bruh's stored access and provider, as the worker asks
    /// it: an allowed call leaves its allowance. For route tests outside
    /// this crate.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn gate_app_call_for_tests(
        &self,
        run: &ReactiveRun,
        tool: &str,
        input: &Value,
    ) -> Result<Option<PermissionDecision>, ReactiveError> {
        let agent_id = run.agent_id.clone();
        let (profile, agent) = self
            .call(move |connection| {
                let profile = store::crew::profile(connection, &agent_id)?;
                let agent = store::records::<bruhs_models::reactive::ReactiveAgent>(
                    connection,
                    "reactive_agents",
                )?
                .into_iter()
                .find(|agent| agent.id == agent_id)
                .ok_or_else(|| ReactiveError::NotFound("The agent was not found.".into()))?;
                Ok((profile, agent))
            })
            .await?;
        let provider = AgentProvider::for_model(&agent.model);
        let claim = self
            .allowances
            .state()
            .claims
            .get(&run.id)
            .copied()
            .unwrap_or_default();
        decide(
            self,
            &profile.access,
            provider,
            bruhs_models::crew::computer_on(profile.computer, &profile.access, provider),
            run,
            claim,
            &agent.project_path,
            tool,
            input,
            Duration::from_secs(5),
        )
        .await
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

/// How a wait ended: for the owner to hand the bruh's computer back, or
/// for the run's vault command to end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ComputerWait {
    /// The bruh has its computer (no vault command runs), or no longer has
    /// one to wait for.
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

/// The login a browser call's vault fill names, if it is one.
fn fill_login_id(input: &Value) -> Option<String> {
    let action = serde_json::from_value::<BrowserAction>(input.get("command")?.clone()).ok()?;
    action
        .secret_fill()
        .map(|(_, secret, _)| secret.login_id.clone())
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

/// Open an approval of `tool` with card `input` for `run`, in one
/// transaction: none for a run no longer running, and none when `grants`
/// lets a late approval's one-shot grant answer it.
async fn open(
    runtime: &ReactiveRuntime,
    run: &ReactiveRun,
    tool: &str,
    input: &Value,
    grants: bool,
) -> Result<Opened, ReactiveError> {
    let fingerprint = approval_fingerprint(tool, input);
    let (opening, tool, card) = (run.clone(), tool.to_owned(), input.clone());
    runtime
        .call(move |connection| {
            store::transaction(connection, |connection| {
                if store::run_record(connection, &opening.id)?.status != RunStatus::Running {
                    return Ok(Opened::Stopped);
                }
                if grants && store::crew::take_grant(connection, &opening.agent_id, &fingerprint)? {
                    return Ok(Opened::Granted);
                }
                store::crew::open_approval(connection, &opening, &tool, &card).map(Opened::Waiting)
            })
        })
        .await
}

/// Ask the owner about one use of `tool` (card `input`) by attempt
/// `claim` of `run`, every time: no grant answers it, before or after
/// (D18). `Some(true)` approved, `Some(false)` denied or expired, `None`
/// when the run stopped or a newer attempt took it over.
pub(super) async fn ask_owner(
    runtime: &ReactiveRuntime,
    run: &ReactiveRun,
    claim: u64,
    tool: &str,
    input: &Value,
    timeout: Duration,
) -> Result<Option<bool>, ReactiveError> {
    if !runtime.allowances.is_current(&run.id, claim) {
        return Ok(None);
    }
    let item_id = match open(runtime, run, tool, input, false).await? {
        Opened::Waiting(id) => id,
        // Without grants, only a stopped run opens nothing.
        Opened::Stopped | Opened::Granted => return Ok(None),
    };
    runtime.changed();
    Ok(match wait(runtime, &run.id, &item_id, timeout).await? {
        ApprovalWait::Approved => Some(true),
        ApprovalWait::Denied { .. } | ApprovalWait::Expired => Some(false),
        ApprovalWait::Stopped => None,
    })
}

/// Wait while the run's turn runs an approved vault command: the
/// command's environment holds a key's value, which no other gated call
/// of the run may reach meanwhile (D18). Bounded by `timeout`, after which
/// the call is refused (F30a). Processes the bruh started earlier are not
/// held (F30c).
async fn wait_for_vault_command(
    runtime: &ReactiveRuntime,
    run: &ReactiveRun,
    timeout: Duration,
) -> Result<ComputerWait, ReactiveError> {
    let Some(mut running) = runtime
        .vault()
        .and_then(|vault| vault.running_in(&run.thread_id))
    else {
        return Ok(ComputerWait::Free);
    };
    if *running.borrow_and_update() == 0 {
        return Ok(ComputerWait::Free);
    }
    let deadline = Instant::now() + timeout;
    // Subscribed before the first read, so a Stop can't slip between them.
    let mut changes = runtime.watch_changes();
    loop {
        let id = run.id.clone();
        let status = runtime
            .call(move |connection| store::run_record(connection, &id).map(|run| run.status))
            .await?;
        if status != RunStatus::Running {
            return Ok(ComputerWait::Stopped);
        }
        if *running.borrow_and_update() == 0 {
            return Ok(ComputerWait::Free);
        }
        tokio::select! {
            changed = running.changed() => {
                // The turn's scope is gone, and its command with it.
                if changed.is_err() {
                    return Ok(ComputerWait::Free);
                }
            }
            changed = changes.changed() => {
                if changed.is_err() {
                    return Ok(ComputerWait::Stopped);
                }
            }
            () = tokio::time::sleep_until(deadline) => return Ok(ComputerWait::StillHeld),
        }
    }
}

/// The answer to one tool call of an unattended bruh run: the gate's own
/// answer, a late approval's one-shot grant, or the owner's answer to a new
/// approval. Every call first waits while an approved vault command of the
/// run runs, then while the owner has the bruh's computer. `None` means
/// the run stopped while it waited. `claim` is the worker's attempt at
/// `run`, which an allowed browser, app or mail step belongs to.
///
/// The bruh's app and mail tools take their own gate ([`app_call`]),
/// before `permission_gate` would refuse them as helper tools.
///
/// The vault command, the owner's hold and the approval are separate
/// waits, each bounded by `timeout`, and an approved browser step waits for
/// the hold once more, so one call waits at most four times `timeout`: an
/// approval asked after a long hold still gets its whole time.
///
/// A file tool call is refused first when it reaches a vault folder or a
/// browser profile by another name (`private_file_refusal`). Claude itself
/// allows Read, Grep and Glob at Ask and Full, so only the calls it asks
/// about reach this check.
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
    match wait_for_vault_command(runtime, run, timeout).await? {
        ComputerWait::Free => {}
        ComputerWait::Stopped => return Ok(None),
        ComputerWait::StillHeld => return Ok(Some(deny(VAULT_COMMAND_RUNNING))),
    }
    match wait_for_computer(runtime, run, timeout).await? {
        ComputerWait::Free => {}
        ComputerWait::Stopped => return Ok(None),
        ComputerWait::StillHeld => return Ok(Some(deny(CONTROL_WAIT_EXPIRED))),
    }
    let browser = tool == BRUH_BROWSER_TOOL;
    // The card's input, the gate's answer, and what an allowed call leaves
    // for its route, if anything.
    let (input, gate, step) = if is_app_tool(tool) {
        // Before `permission_gate`, whose `mcp__` rule refuses every other
        // tool of the helper.
        app_call(runtime, access, provider, &run.agent_id, tool, input).await
    } else if browser {
        // The flag is from the run's start: a Computer turned off since has
        // no slot, and its steps are refused at once instead of asked about.
        let computer = computer
            && runtime
                .computers()
                .is_some_and(|host| host.has_computer(&run.agent_id));
        // A vault fill is checked against the login as the vault has it
        // now, looked up before the page is asked so the page is fresh. A
        // lookup that fails or outlasts the vault wait finds none.
        let login = match fill_login_id(input) {
            Some(login_id) => runtime
                .login_target(&run.agent_id, &login_id)
                .await
                .unwrap_or_else(|error| {
                    tracing::warn!(%error, "A vault fill's login could not be read");
                    None
                }),
            None => None,
        };
        let call = browser_call(input, page_url(runtime, &run.agent_id).await);
        let gate = browser_gate(
            access,
            provider,
            computer,
            &call,
            call["page_url"].as_str(),
            login.as_ref(),
        );
        // Input on a page the browser doesn't name could land anywhere, so
        // nobody is asked about it, and the route couldn't check it: a
        // snapshot names the page first.
        if matches!(gate, Gate::Allow | Gate::Approve)
            && call.get("page_url").is_none()
            && is_input(&call)
        {
            return Ok(Some(deny(PAGE_UNKNOWN)));
        }
        let step = Step {
            command: call["command"].clone(),
            page_url: call["page_url"].as_str().map(str::to_owned),
            target: None,
        };
        (call, gate, Some(step))
    } else {
        // The gate reads only the name, and a name can point here.
        if let Some(host) = network_host(tool, input) {
            if reaches_this_computer(&host, system_lookup).await {
                return Ok(Some(deny(LOCAL_HOST_DENIAL)));
            }
        }
        if let Some(reason) = private_file_refusal(
            &runtime.home,
            runtime.private_roots(),
            project_path,
            tool,
            input,
        ) {
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
        (input, gate, None)
    };
    // Each allowed browser, app or mail call may reach its route once,
    // marked with whether the owner approved it (final review BC M2).
    let allow = |owner_approved: bool| {
        if let Some(step) = &step {
            runtime.allowances.record(StepAllowance {
                agent_id: run.agent_id.clone(),
                run_id: run.id.clone(),
                tool: tool.to_owned(),
                command: step.command.clone(),
                page_url: step.page_url.clone(),
                target: step.target.clone(),
                owner_approved,
                claim,
                expires: Instant::now() + ALLOWANCE_TTL,
            });
        }
        Some(PermissionDecision::Allow)
    };
    match gate {
        Gate::Allow => return Ok(allow(false)),
        Gate::Deny(reason) => return Ok(Some(deny(reason))),
        Gate::Approve => {}
    }
    let item_id = match open(runtime, run, tool, &input, true).await? {
        Opened::Stopped => return Ok(None),
        // The grant was made for this page, account or mailbox (the card
        // names it), so it needs no second look; the allowance keeps the
        // call as sent (ruling F25).
        Opened::Granted => return Ok(allow(true)),
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
                None => allow(true),
            }
        }
        ApprovalWait::Approved => allow(true),
        ApprovalWait::Denied { reply: None } => Some(deny(DENIED)),
        ApprovalWait::Denied { reply: Some(reply) } => {
            Some(deny(&format!("The owner denied this and replied: {reply}")))
        }
        ApprovalWait::Expired => Some(deny(NOT_ANSWERED)),
        ApprovalWait::Stopped => None,
    })
}

/// What an allowed call leaves for its route: see [`StepAllowance`].
struct Step {
    command: Value,
    page_url: Option<String>,
    target: Option<String>,
}

/// One call of the bruh's app or mail tools as the gate sees it: the card
/// input (the call plus the account or mailbox it goes out as), the gate's
/// answer, and the step an allowed call leaves, which keeps the input as
/// sent and the target the lookup found (ruling F10). The account or
/// mailbox is looked up live from the bruh's grants, so a revoked grant
/// stops the next call.
async fn app_call(
    runtime: &ReactiveRuntime,
    access: &BruhAccess,
    provider: AgentProvider,
    agent_id: &str,
    tool: &str,
    input: &Value,
) -> (Value, Gate, Option<Step>) {
    let step = |target: Option<String>| {
        Some(Step {
            command: input.clone(),
            page_url: None,
            target,
        })
    };
    // A bruh that can't use apps is refused without a lookup.
    if let Some(reason) = apps_unavailable(access.level, provider) {
        return (input.clone(), Gate::Deny(reason), None);
    }
    match tool {
        APP_REQUEST_TOOL => {
            let lookup = runtime.app_lookup(agent_id, input).await;
            let gate = app_gate(access, provider, input, &lookup);
            match lookup {
                AppLookup::Found(target) => (
                    app_card_input(input, &target),
                    gate,
                    step(Some(target.entry_id)),
                ),
                AppLookup::Missing(_) => (input.clone(), gate, None),
            }
        }
        SEND_MAIL_TOOL => {
            let lookup = runtime.mail_lookup(agent_id, input).await;
            let gate = match mail_gate(access, provider, input, &lookup) {
                // Past the hourly cap, refused before any card (final
                // review A2 M7); the route counts the send itself.
                Gate::Allow | Gate::Approve
                    if !runtime.mail_sends.has_room(agent_id, Instant::now()) =>
                {
                    Gate::Deny(MAIL_RATE_LIMITED)
                }
                gate => gate,
            };
            match lookup {
                MailLookup::Found(target) => (
                    mail_card_input(input, &target),
                    gate,
                    step(Some(target.mailbox_id)),
                ),
                MailLookup::Missing(_) => (input.clone(), gate, None),
            }
        }
        _ => (input.clone(), list_apps_gate(access, provider), step(None)),
    }
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
/// rules (`computers::hidden_folders`, the vault folders) match a path by
/// name, so this looks at what the path really is:
///
/// - Resolved (symlinks followed, a relative path taken from `project`),
///   a path in one of `roots` (the folders that hold the owner's saved
///   logins, already resolved), compared by component without case, gets
///   [`VAULT_FILE_DENIAL`].
/// - Resolved the same way, a path in any `<home>/bruhs/*/browser*`
///   folder, compared without case as case-insensitive volumes do, gets
///   [`PROFILE_DENIAL`].
/// - A tool that opens one file gets [`LINKED_FILE_DENIAL`] for a regular
///   file under `<home>/bruhs` with more than one name: a hard link to a
///   profile file can sit anywhere in a bruh's folder.
///
/// A path that can't be resolved is left to the gate and the provider.
fn private_file_refusal(
    home: &Path,
    roots: &[PathBuf],
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
        if roots.iter().any(|root| below(root, &real).is_some()) {
            return Some(VAULT_FILE_DENIAL);
        }
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
pub(super) fn resolve(path: &Path, links: u8) -> Option<PathBuf> {
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
    use bruhs_models::crew::BRUH_SECRET_TOOL;

    /// Final review A2 M7: 30 emails a bruh an hour, each bruh counted on
    /// its own; one that falls out of the hour makes room.
    #[test]
    fn email_sends_are_capped_per_bruh_per_hour() {
        let sends = MailSends::default();
        let start = Instant::now();
        for minute in 0..MAIL_SENDS_PER_HOUR {
            let at = start + Duration::from_secs(60 * u64::try_from(minute).unwrap());
            assert!(sends.has_room("a", at));
            assert!(sends.take("a", at));
        }
        let later = start + Duration::from_secs(59 * 60);
        assert!(!sends.has_room("a", later) && !sends.take("a", later));
        assert!(sends.take("b", later));
        // The first send is an hour old: one more fits, then none.
        let hour = start + MAIL_SEND_WINDOW;
        assert!(sends.take("a", hour));
        assert!(!sends.take("a", hour));
    }

    #[test]
    fn an_allowance_lasts_until_the_helpers_deadline() {
        let allowances = Allowances::default();
        let command = serde_json::json!({"action": "snapshot"});
        let claim = allowances.claim("r");
        let allowance = |expires| StepAllowance {
            agent_id: "a".into(),
            run_id: "r".into(),
            tool: BRUH_BROWSER_TOOL.into(),
            command: command.clone(),
            page_url: None,
            target: None,
            owner_approved: false,
            claim,
            expires,
        };
        let now = Instant::now();
        let deadline = now + ALLOWANCE_TTL;
        allowances.record(allowance(deadline));
        let just_before = deadline - Duration::from_millis(1);
        assert!(allowances
            .take("a", BRUH_BROWSER_TOOL, &command, just_before)
            .is_some());
        allowances.record(allowance(deadline));
        assert!(allowances
            .take("a", BRUH_BROWSER_TOOL, &command, deadline)
            .is_none());
        // An expired allowance is gone for good.
        assert!(allowances
            .take("a", BRUH_BROWSER_TOOL, &command, just_before)
            .is_none());
    }

    /// An old attempt's worker, still running after a cancel and a retry
    /// claimed the run again, can neither leave a step for the new attempt
    /// nor clear the new attempt's steps when it ends.
    #[test]
    fn an_old_attempt_never_records_into_or_forgets_a_new_one() {
        let allowances = Allowances::default();
        let command = serde_json::json!({"action": "snapshot"});
        let allowance = |claim| StepAllowance {
            agent_id: "a".into(),
            run_id: "r".into(),
            tool: BRUH_BROWSER_TOOL.into(),
            command: command.clone(),
            page_url: None,
            target: None,
            owner_approved: false,
            claim,
            expires: Instant::now() + ALLOWANCE_TTL,
        };
        let old = allowances.claim("r");
        allowances.record(allowance(old));
        let new = allowances.claim("r");
        assert!(
            allowances
                .take("a", BRUH_BROWSER_TOOL, &command, Instant::now())
                .is_none(),
            "a claim forgets earlier attempts"
        );
        allowances.record(allowance(old));
        assert!(allowances
            .take("a", BRUH_BROWSER_TOOL, &command, Instant::now())
            .is_none());
        allowances.record(allowance(new));
        allowances.forget_run("r", old);
        assert_eq!(allowances.current_claim("r"), Some(new));
        let taken = allowances
            .take("a", BRUH_BROWSER_TOOL, &command, Instant::now())
            .unwrap();
        assert_eq!(taken.claim, new);
        // Once the new attempt ends, nothing records for the run.
        allowances.forget_run("r", new);
        assert_eq!(allowances.current_claim("r"), None);
        allowances.record(allowance(new));
        assert!(allowances
            .take("a", BRUH_BROWSER_TOOL, &command, Instant::now())
            .is_none());
    }

    /// An allowance answers only its own tool's call: a browser step never
    /// stands in for an app call or an email with the same input, nor one
    /// app tool for another.
    #[test]
    fn an_allowance_is_taken_only_by_its_own_tool() {
        let allowances = Allowances::default();
        let input = serde_json::json!({"app": "github", "method": "GET", "path": "/user"});
        let claim = allowances.claim("r");
        allowances.record(StepAllowance {
            agent_id: "a".into(),
            run_id: "r".into(),
            tool: APP_REQUEST_TOOL.into(),
            command: input.clone(),
            page_url: None,
            target: Some("e".into()),
            owner_approved: false,
            claim,
            expires: Instant::now() + ALLOWANCE_TTL,
        });
        for tool in [BRUH_BROWSER_TOOL, SEND_MAIL_TOOL, BRUH_SECRET_TOOL] {
            assert!(
                allowances.take("a", tool, &input, Instant::now()).is_none(),
                "{tool}"
            );
        }
        assert!(allowances
            .take("b", APP_REQUEST_TOOL, &input, Instant::now())
            .is_none());
        let taken = allowances
            .take("a", APP_REQUEST_TOOL, &input, Instant::now())
            .unwrap();
        assert_eq!(taken.target.as_deref(), Some("e"));
        assert!(allowances
            .take("a", APP_REQUEST_TOOL, &input, Instant::now())
            .is_none());
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

    /// Both data roots' `backups` folders are private (Backup D21): a bruh
    /// can't read a backup through a link in its work folder, by an
    /// absolute or a relative path, at any level of the tree.
    #[test]
    fn bruh_runs_are_kept_out_of_backups() {
        let dir = tempfile::tempdir().unwrap();
        // The tempdir path is not canonical on macOS (/var is a symlink),
        // and the roots are resolved like the paths the tools name.
        let base = std::fs::canonicalize(dir.path()).unwrap();
        let home = base.join("home");
        let core = base.join("core");
        let desktop = base.join("desktop");
        let backups = [core.join("backups"), desktop.join("backups")];
        let outgoing = backups[0].join("outgoing");
        std::fs::create_dir_all(&outgoing).unwrap();
        std::fs::create_dir_all(backups[1].join("pre-restore-x")).unwrap();
        std::fs::write(outgoing.join("x.bruhsbak"), "sealed").unwrap();
        let roots =
            crate::secrets::private::private_folders(&home, &core.join("credentials"), &backups);
        let work = home.join("bruhs").join("a1").join("work");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(work.join("notes.md"), "mine").unwrap();
        std::os::unix::fs::symlink(&backups[0], work.join("out")).unwrap();
        std::os::unix::fs::symlink(&backups[1], work.join("kept")).unwrap();
        let project = work.to_string_lossy().into_owned();
        let at = |path: &Path| path.to_string_lossy().into_owned();
        let refused = [
            (
                "Read",
                serde_json::json!({"file_path": at(&work.join("out/outgoing/x.bruhsbak"))}),
            ),
            (
                "Read",
                serde_json::json!({"file_path": "out/outgoing/x.bruhsbak"}),
            ),
            (
                "Read",
                serde_json::json!({"file_path": at(&outgoing.join("x.bruhsbak"))}),
            ),
            ("LS", serde_json::json!({"path": "kept"})),
            (
                "Glob",
                serde_json::json!({"pattern": "**", "path": at(&backups[1])}),
            ),
        ];
        for (tool, input) in &refused {
            assert_eq!(
                private_file_refusal(&home, &roots, &project, tool, input),
                Some(VAULT_FILE_DENIAL),
                "{tool} {input}"
            );
        }
        let open = serde_json::json!({"file_path": at(&work.join("notes.md"))});
        assert_eq!(
            private_file_refusal(&home, &roots, &project, "Read", &open),
            None
        );
    }
}
