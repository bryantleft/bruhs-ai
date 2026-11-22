use super::{sources, store, ReactiveError, ReactiveRuntime};
use crate::secrets::NOT_GRANTED;
use crate::workbench_tools::BruhTools;
use crate::{AgentTurn, AppState, OrchestratorEvent, PermissionDecision, QueryEvent};
use bruhs_models::{
    agent::{AgentProvider, PermissionMode},
    crew::{
        apps_unavailable, computer_on, effective_level, run_preamble, vault_key_card_fits,
        AccessLevel, BruhProfile, PreambleTools, APPROVAL_WAIT_MS, BRUH_SECRET_TOOL,
        BRUH_SEND_RULE, VAULT_KEY_TOO_LONG,
    },
    reactive::{ReactiveAgent, ReactiveRun, RunStatus, MAX_TEXT_CHARS, REPORT_PROTOCOL},
    secrets::{SecretDecision, SecretRequest},
    thread_link::ThreadAccess,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio_stream::StreamExt;

mod task_checkout;

struct AbortOnDrop(tokio::task::JoinHandle<()>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(super) async fn run(runtime: ReactiveRuntime, app: Arc<AppState>) {
    let _sources = AbortOnDrop(sources::start(runtime.clone()));
    let _notifications = AbortOnDrop(super::notify::start(runtime.clone()));
    let mut changes = runtime.watch_changes();
    let mut lanes = tokio::task::JoinSet::new();
    let halted = Arc::new(std::sync::atomic::AtomicBool::new(false));
    loop {
        if halted.load(std::sync::atomic::Ordering::Acquire) {
            // An uncertain storage outcome never starts more work in this process.
            while lanes.join_next().await.is_some() {}
            return;
        }
        match runtime.call(store::fire_follow_ups).await {
            Ok(0) => {}
            Ok(_) => runtime.changed(),
            Err(error) => tracing::error!("Reactive follow-ups could not be queued: {error}"),
        }
        match runtime.call(store::webhook_windows::flush_due).await {
            Ok(0) => {}
            Ok(_) => runtime.changed(),
            Err(error) => tracing::error!("Webhook windows could not be dispatched: {error}"),
        }
        // Fill free lanes; each claim takes one agent's queued work.
        loop {
            match runtime.claim_next().await {
                Ok(Some(claim)) => {
                    runtime.changed();
                    let (runtime, app, halted) = (runtime.clone(), app.clone(), halted.clone());
                    lanes.spawn(async move {
                        if let Err(error) = execute(&runtime, &app, &claim).await {
                            // Deletion can win any await in execute, including the final receipt.
                            // Dropping the turn stream has already stopped its provider.
                            let removed = matches!(
                                error,
                                ReactiveError::NotFound(_) | ReactiveError::Conflict(_)
                            ) && {
                                let id = claim.run.id.clone();
                                matches!(
                                    runtime
                                        .call(move |db| store::task_conversations::run_was_deleted(db, &id))
                                        .await,
                                    Ok(true)
                                )
                            };
                            if !removed {
                                tracing::error!("Reactive run could not be recorded: {error}");
                                halted.store(true, std::sync::atomic::Ordering::Release);
                            }
                        }
                        // The run has left Running: its steps can't be sent.
                        runtime
                            .allowances
                            .forget_run(&claim.run.id, claim.generation);
                        runtime.changed();
                    });
                }
                Ok(None) => break,
                Err(error) => {
                    tracing::error!("Reactive work queue stopped: {error}");
                    halted.store(true, std::sync::atomic::Ordering::Release);
                    break;
                }
            }
        }
        // One-second scheduling precision matches source configuration application.
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            changed = changes.changed() => { if changed.is_err() { return; } }
            Some(_) = lanes.join_next(), if !lanes.is_empty() => {}
        }
    }
}

/// The agent's fixed system instructions for every run. The name and access
/// stay out, so editing them keeps the provider session ([`turn`] sends them).
pub(super) fn system_instructions(agent: &ReactiveAgent) -> String {
    const WORKFLOW_DRAFTS: &str = r#"When the owner requests drafts for new tasks, notes, captures, inbox messages or calendar entries, return only a JSON proposal with this shape: {"summary":"What the proposal does","operations":[{"type":"create_task","title":"Task title"}]}. Supported operation shapes are create_task {title}, create_knowledge_note {title,body}, add_capture {content}, draft_inbox_message {recipient,body}, and create_calendar_event {title,start_at_ms,end_at_ms}. Calendar timestamps are epoch milliseconds. This JSON is a draft only: the user must explicitly import it into the app's workflow preview and approve its independently granted writes. Never claim that proposed operations were applied. Answer ordinary questions in plain text."#;
    format!(
        "You are a persistent Bruhs agent. You wake for events, follow-ups and owner messages, do the work, and go back to sleep. Each run starts with your name and your current access; stay within it. {BRUH_SEND_RULE}\n\n{WORKFLOW_DRAFTS}\n\n{REPORT_PROTOCOL}\n\nOwner instructions:\n{}\n\nOwner-approved memory:\n{}",
        agent.instructions, agent.memory
    )
}

/// Fingerprint of the system instructions a provider session started with.
/// Providers keep a resumed session's original instructions, so a change
/// (including an app update to the protocol text) needs a fresh session.
pub(super) fn instructions_stamp(agent: &ReactiveAgent) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(system_instructions(agent).as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// What a run's turn needs from the vault, built from the runtime before
/// each run.
#[derive(Debug, Clone, Default)]
pub(super) struct TurnVault {
    /// The folders that hold the owner's saved logins, as the deny rules
    /// name them (`ReactiveRuntime::private_rule_folders`).
    pub(super) private: Vec<PathBuf>,
    /// Whether the bruh has a vault login it may fill, which the preamble
    /// then tells it about (D26).
    pub(super) logins: bool,
    /// The names of the vault keys the bruh may use through
    /// `request_secret` (D18, ruling F28). Empty when the run can't have
    /// the tool.
    pub(super) keys: Vec<String>,
    /// The names of the apps the bruh may call (D19). Empty when the run
    /// can't have the app tools.
    pub(super) apps: Vec<String>,
    /// The labels of the mailboxes the bruh may send from. Empty when the
    /// run can't have `send_mail`.
    pub(super) mail: Vec<String>,
}

/// Whether a run on `provider` at `level` (the bruh's chosen level) may
/// have the vault key tool: Claude at Ask or Full (D18).
pub(super) fn keys_possible(level: AccessLevel, provider: AgentProvider) -> bool {
    provider == AgentProvider::Claude
        && matches!(
            effective_level(level, provider),
            AccessLevel::Ask | AccessLevel::Full
        )
}

/// One run's turn. The bruh's name and access ride in the run's own message,
/// so renaming a bruh or changing its access keeps the provider session.
pub(super) fn turn(
    agent: &ReactiveAgent,
    bruh: &BruhProfile,
    run: &ReactiveRun,
    context: &str,
    home: &Path,
    vault: &TurnVault,
) -> AgentTurn {
    let instructions = system_instructions(agent);
    let provider = AgentProvider::for_model(&agent.model);
    // Instructions stay identical across runs so providers can cache them;
    // the name, access, memory and history that change ride in the run's
    // own message.
    let computer = computer_on(bruh.computer, &bruh.access, provider);
    let keys: &[String] = if keys_possible(bruh.access.level, provider) {
        &vault.keys
    } else {
        &[]
    };
    // Only a Claude bruh at Ask or Full gets app or mail tools (D2), and
    // only for what it is granted; the preamble names no others.
    let (apps, mail): (&[String], &[String]) =
        if apps_unavailable(bruh.access.level, provider).is_none() {
            (&vault.apps, &vault.mail)
        } else {
            (&[], &[])
        };
    let tools = BruhTools {
        agent_id: agent.id.clone(),
        browser: computer,
        apps: !apps.is_empty(),
        mail: !mail.is_empty(),
    };
    let mut prompt = run_preamble(
        &agent.name,
        &bruh.access,
        effective_level(bruh.access.level, provider),
        &PreambleTools {
            computer,
            vault_logins: computer && vault.logins,
            vault_keys: keys,
            apps,
            mail,
        },
    );
    prompt.push_str("\n\n");
    if !context.is_empty() {
        prompt.push_str(context);
        prompt.push_str("\n\n---\n\n");
    }
    prompt.push_str(&run.prompt);
    let turn = AgentTurn::new(
        run.thread_id.clone(),
        &prompt,
        &agent.project_path,
        &agent.model,
        provider,
        PermissionMode::Plan,
    )
    .resume(run.session_id.clone())
    .thread_access(ThreadAccess::Off)
    .instructions(Some(instructions))
    .effort(bruh.effort.as_deref())
    .unattended_with(bruh.access.clone())
    .bruh_tools(tools.any().then_some(tools))
    .bruh_secrets(!keys.is_empty())
    // Every run, Computer on or off: the profile outlives Computer off. The
    // keychains folder stays out of the rules (`deny_rule_may_name`).
    .hide_folders(
        super::computers::hidden_folders(home, &agent.id)
            .into_iter()
            .chain(
                vault
                    .private
                    .iter()
                    .filter(|folder| crate::secrets::private::deny_rule_may_name(folder))
                    .cloned(),
            )
            .collect(),
    );
    // `resume(None)` otherwise falls back to the orchestrator's in-memory session.
    if run.session_id.is_none() {
        turn.fresh()
    } else {
        turn
    }
}

async fn execute(
    runtime: &ReactiveRuntime,
    app: &AppState,
    claim: &store::Claim,
) -> Result<(), ReactiveError> {
    let (agent, run) = (&claim.agent, &claim.run);
    let mut context = claim.context.clone();
    let mut changes = runtime.watch_changes();
    let mut effective_run = run.clone();
    let mut effective_agent=agent.clone();
    let _desktop_lock = if claim.desktop_thread {
        let lock = match super::desktop_turn_lock(&run.thread_id) {
            Ok(lock) => lock,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                let id = run.id.clone();
                runtime
                    .call(move |db| store::defer_desktop_turn(db, &id))
                    .await?;
                return Ok(());
            }
            Err(error) => {
                let id = run.id.clone();
                runtime
                    .call(move |db| {
                        store::finish_run(
                            db,
                            &id,
                            RunStatus::Failed,
                            None,
                            Some(error.to_string()),
                            None,
                        )
                    })
                    .await?;
                return Ok(());
            }
        };
        let mut agent = agent.clone();
        let id = run.id.clone();
        let prepared = runtime
            .call(move |db| {
                store::transaction(db, |db| {
                    let mut next = store::run_record(db, &id)?;
                    if next.status != RunStatus::Running {
                        return Ok(None);
                    }
                    let context = store::desktop_tasks::prepare_turn(db, &mut agent, &mut next)?;
                    next.fresh_session = next.session_id.is_none();
                    store::save_run(db, &next)?;
                    Ok(Some((next, agent, context)))
                })
            })
            .await;
        match prepared {
            Ok(Some((next, agent, history))) => {
                effective_agent=agent;
                effective_run = next;
                context = history;
            }
            Ok(None) => return Ok(()),
            Err(error) => {
                let id = run.id.clone();
                runtime
                    .call(move |db| {
                        store::finish_run(
                            db,
                            &id,
                            RunStatus::Failed,
                            None,
                            Some(error.to_string()),
                            None,
                        )
                    })
                    .await?;
                return Ok(());
            }
        }
        Some(lock)
    } else {
        None
    };
    let mut bruh = claim.bruh.clone();
    let requested_run = run.clone();
    let preparation = runtime
        .call(move |connection| store::reports::prepare(connection, &requested_run))
        .await;
    let preparation = match preparation {
        Ok(preparation) => preparation,
        Err(error @ ReactiveError::Storage(_)) => return Err(error),
        Err(error) => {
            let (id, message) = (run.id.clone(), error.to_string());
            runtime
                .call(move |connection| {
                    store::finish_run(
                        connection,
                        &id,
                        RunStatus::Failed,
                        None,
                        Some(message),
                        None,
                    )
                })
                .await?;
            return Ok(());
        }
    };
    if let Some(mut job) = preparation {
        if job.coverage.is_none() {
            let coverage = {
                let collection = super::report_sources::collect(&job);
                tokio::pin!(collection);
                loop {
                    tokio::select! {
                        coverage = &mut collection => break coverage,
                        changed = changes.changed() => {
                            if changed.is_err() {return Ok(());}
                            let id=run.id.clone();
                            if runtime.call(move |connection|store::run_record(connection,&id).map(|run|run.status)).await? != RunStatus::Running {return Ok(());}
                        }
                    }
                }
            };
            job.coverage = Some(coverage.clone());
            let id = run.id.clone();
            runtime
                .call(move |connection| store::reports::set_coverage(connection, &id, coverage))
                .await?;
        }
        effective_agent.project_path = job.preset.project_path.clone();
        effective_agent.model = job.preset.model.clone();
        effective_run.prompt = super::report_sources::prompt(&job);
        // A new report uses only the frozen window and evidence, not prior
        // sessions' stale source context. Its stable thread still reopens activity.
        effective_run.session_id = None;
        context.clear();
        // Weekly reports only read their frozen evidence.
        bruh = BruhProfile::default();
    }
    if !effective_run.task_checkout.is_project() {
        match task_checkout::resolve(Path::new(&agent.project_path), &effective_run).await {
            Ok(checkout) => effective_agent.project_path = checkout,
            Err(error) => {
                let id = run.id.clone();
                runtime
                    .call(move |connection| {
                        store::finish_run(
                            connection,
                            &id,
                            RunStatus::Failed,
                            None,
                            Some(error),
                            None,
                        )
                    })
                    .await?;
                return Ok(());
            }
        }
    }
    let id = run.id.clone();
    if runtime
        .call(move |connection| store::run_record(connection, &id).map(|run| run.status))
        .await?
        != RunStatus::Running
    {
        return Ok(());
    }
    let provider = AgentProvider::for_model(&effective_agent.model);
    let vault = runtime
        .turn_vault(
            &effective_agent.id,
            computer_on(bruh.computer, &bruh.access, provider),
            keys_possible(bruh.access.level, provider),
            apps_unavailable(bruh.access.level, provider).is_none(),
        )
        .await;
    let mut stream = match app
        .send_message(turn(
            &effective_agent,
            &bruh,
            &effective_run,
            &context,
            &runtime.home,
            &vault,
        ))
        .await
    {
        Ok(stream) => stream,
        Err(crate::SendMessageError::ThreadBusy(_)) => {
            let id = run.id.clone();
            runtime
                .call(move |connection| store::return_to_queue(connection, &id))
                .await?;
            return Ok(());
        }
        Err(error) => {
            let id=run.id.clone();
            runtime.call(move |db| store::finish_run(db,&id,RunStatus::Failed,None,Some(error.to_string()),None)).await?;
            return Ok(());
        }
    };
    let mut output = String::new();
    let mut output_chars = 0_usize;
    let mut output_truncated = false;
    let mut result = None;
    let mut codex_completed = false;
    let mut context_tokens = None;
    let mut failure = None;
    loop {
        tokio::select! {
            changed = changes.changed() => {
                if changed.is_err() { break; }
                let id = run.id.clone();
                let status = runtime.call(move |connection| store::run_record(connection, &id).map(|run| run.status)).await?;
                if status != RunStatus::Running {
                    let id = run.id.clone();
                    let partial = result.take().unwrap_or_else(|| std::mem::take(&mut output));
                    runtime.call(move |connection| store::cancelled_output(connection, &id, partial)).await?;
                    runtime.changed();
                    return Ok(());
                }
            }
            event = stream.next() => {
                let Some(event) = event else { break; };
                match event {
                    OrchestratorEvent::Query(QueryEvent::Text(text)) => {
                        let remaining = MAX_TEXT_CHARS.saturating_sub(output_chars);
                        let take = text.chars().count().min(remaining);
                        output_truncated |= take < text.chars().count();
                        output.extend(text.chars().take(take));
                        output_chars += take;
                    }
                    OrchestratorEvent::Query(QueryEvent::Init { session_id, .. }) => {
                        let id = run.id.clone();
                        let model=effective_agent.model.clone();
                        runtime.call(move |connection| store::record_session(connection, &id, &session_id,&model)).await?;
                    }
                    OrchestratorEvent::Query(QueryEvent::Result { result: text, session_id, is_error, .. }) => {
                        let id = run.id.clone();
                        let model=effective_agent.model.clone();
                        runtime.call(move |connection| store::record_session(connection, &id, &session_id,&model)).await?;
                        result = Some(text);
                        if is_error { failure = Some("The provider reported a failed turn.".into()); }
                    }
                    // Codex releases final usage after process settlement. A
                    // following process error still overrides this receipt.
                    OrchestratorEvent::Query(QueryEvent::Usage(usage)) => {
                        if let Some(tokens) = usage.context_tokens {
                            context_tokens = Some(tokens);
                        }
                        if AgentProvider::for_model(&effective_agent.model) == AgentProvider::Codex {
                            codex_completed = true;
                        }
                    }
                    OrchestratorEvent::Query(QueryEvent::Error(error)) => { failure = Some(error); }
                    OrchestratorEvent::Query(QueryEvent::PermissionRequest { id, tool, input, .. }) => {
                        // The bruh's access gate answers, or the owner through an outbox approval.
                        let provider = AgentProvider::for_model(&effective_agent.model);
                        let decision = super::approval::decide(
                            runtime,
                            &bruh.access,
                            provider,
                            computer_on(bruh.computer, &bruh.access, provider),
                            &effective_run,
                            claim.generation,
                            &effective_agent.project_path,
                            &tool,
                            &input,
                            Duration::from_millis(APPROVAL_WAIT_MS),
                        )
                        .await;
                        let decision = match decision {
                            Ok(Some(decision)) => decision,
                            Ok(None) => {
                                // Stopped while waiting: keep the partial output, as above.
                                let id = run.id.clone();
                                let partial = result.take().unwrap_or_else(|| std::mem::take(&mut output));
                                runtime.call(move |connection| store::cancelled_output(connection, &id, partial)).await?;
                                runtime.changed();
                                return Ok(());
                            }
                            Err(error @ ReactiveError::Storage(_)) => return Err(error),
                            Err(error) => {
                                failure = Some(error.to_string());
                                PermissionDecision::Deny { reason: Some("This step could not be sent for approval.".into()) }
                            }
                        };
                        if let Err(error) = app.resolve_permission(&id, decision).await {
                            failure = Some(error.to_string());
                        }
                    }
                    OrchestratorEvent::Query(QueryEvent::SecretRequest(request)) => {
                        let answer = answer_secret_request(runtime, app, &effective_run, claim.generation, &request).await?;
                        if answer == SecretAnswer::Stopped {
                            let id = run.id.clone();
                            let partial = result.take().unwrap_or_else(|| std::mem::take(&mut output));
                            runtime.call(move |connection| store::cancelled_output(connection, &id, partial)).await?;
                            runtime.changed();
                            return Ok(());
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    if output_truncated && result.as_ref().is_none_or(|text| text.is_empty()) {
        output.push_str("\n[Output truncated; retained prefix only.]");
        failure = Some(
            "The streamed output exceeded the saved result budget. The retained answer is partial."
                .into(),
        );
    }
    let completed = (result.is_some() || codex_completed) && failure.is_none();
    if result.is_none() && !codex_completed && failure.is_none() {
        failure = Some("The provider ended without a completion receipt. Review the partial output before retrying.".into());
    }
    let output = result
        .filter(|text| !text.is_empty())
        .or_else(|| (!output.is_empty()).then_some(output));
    let id = run.id.clone();
    runtime
        .call(move |connection| {
            store::finish_run(
                connection,
                &id,
                if completed {
                    RunStatus::Completed
                } else {
                    RunStatus::Failed
                },
                output,
                failure,
                context_tokens,
            )
        })
        .await
}

/// How the worker left a bruh's vault key request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SecretAnswer {
    /// Answered; the run goes on.
    Answered,
    /// The run stopped while the owner was asked; the request was declined.
    Stopped,
}

/// Answer a bruh's `request_secret` (D18): a key it may use runs the
/// command once the owner approves this one use, at every level; anything
/// else is `not_granted` with no card. A service connection is never a
/// bruh's (F27). The value goes only into the approved command's
/// environment, through the broker, never into the run.
pub(super) async fn answer_secret_request(
    runtime: &ReactiveRuntime,
    app: &AppState,
    run: &ReactiveRun,
    claim: u64,
    request: &SecretRequest,
) -> Result<SecretAnswer, ReactiveError> {
    let secrets = app.secrets();
    let (thread, id) = (&run.thread_id, request.id.as_str());
    let name = &request.operation.name;
    let entry = match request.connection {
        Some(_) => None,
        None => runtime.usable_value(&run.agent_id, name).await,
    };
    let Some(entry) = entry else {
        if let Err(error) = secrets.resolve_refused(thread, id, NOT_GRANTED).await {
            tracing::debug!(%error, "A refused vault key request was already gone");
        }
        return Ok(SecretAnswer::Answered);
    };
    // The card shows the operation and the folder it runs in, whole: a
    // command too long for that is refused, never cut.
    let mut card = serde_json::to_value(&request.operation).unwrap_or_default();
    card["cwd"] = serde_json::Value::from(request.cwd.as_str());
    if !vault_key_card_fits(&card) {
        if let Err(error) = secrets
            .resolve_refused(thread, id, VAULT_KEY_TOO_LONG)
            .await
        {
            tracing::debug!(%error, "A refused vault key request was already gone");
        }
        return Ok(SecretAnswer::Answered);
    }
    let approved = super::approval::ask_owner(
        runtime,
        run,
        claim,
        BRUH_SECRET_TOOL,
        &card,
        Duration::from_millis(APPROVAL_WAIT_MS),
    )
    .await?;
    // The grant is checked again: it may have gone during the wait.
    let entry = match approved {
        Some(true) => runtime
            .usable_value(&run.agent_id, name)
            .await
            .filter(|now| now == &entry),
        _ => None,
    };
    let resolved = match entry {
        Some(entry) => secrets.resolve_from_vault(thread, id, &entry).await,
        None if approved == Some(true) => secrets.resolve_refused(thread, id, NOT_GRANTED).await,
        None => secrets.resolve(thread, id, SecretDecision::Decline).await,
    };
    if let Err(error) = resolved {
        // A read that failed leaves the request open: decline it, so the
        // tool call ends. The error is fixed copy, never a value.
        tracing::warn!(%error, "A bruh's vault key request could not run");
        let _ = secrets.resolve(thread, id, SecretDecision::Decline).await;
    }
    Ok(if approved.is_none() {
        SecretAnswer::Stopped
    } else {
        SecretAnswer::Answered
    })
}
