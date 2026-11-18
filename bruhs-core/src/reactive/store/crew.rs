//! Bruh profiles: how each agent looks, what it may do and its effort, kept
//! beside `reactive_agents` so a cosmetic edit never touches the agent row,
//! its runs or its provider session.

use super::{
    always_on, cancel_runs, decode, enabled_agent, encode, get, new_run, queue_capacity, records,
    required, rooms, run_record, save_run, ReactiveError,
};
use crate::reactive::clock;
use bruhs_models::{
    agent::AgentProvider,
    crew::{
        approval_detail, approval_fingerprint, approval_summary, effective_level,
        level_unavailable, rail_order, status_for, ApprovalRequest, ApprovalState, BruhAccess,
        BruhLook, BruhProfile, BruhState,
    },
    logic::run_place::claude_effort,
    reactive::{
        EventSource, OutboxItem, OutboxKind, ReactiveAgent, ReactiveRun, RunStatus, RunTrigger,
        SourceKind, Subscription, SystemDelivery,
    },
    weekly_reports::managed_preset_id,
    EpochMs,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

const ACCESS_CHANGED: &str = "Access changed. Review the new access before starting another run.";
/// How long a late approval's grant stays usable.
pub(in crate::reactive) const GRANT_TTL_MS: i64 = 24 * 60 * 60 * 1000;

pub(in crate::reactive) fn initialize(connection: &Connection) -> Result<(), ReactiveError> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS reactive_bruhs(agent_id TEXT PRIMARY KEY,payload TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS reactive_approval_grants(agent_id TEXT NOT NULL,fingerprint TEXT NOT NULL,created INTEGER NOT NULL,PRIMARY KEY(agent_id,fingerprint));",
    )?;
    Ok(())
}

/// The saved profile, or today's look and read-only access when none was saved.
pub(in crate::reactive) fn profile(
    connection: &Connection,
    agent_id: &str,
) -> Result<BruhProfile, ReactiveError> {
    let payload: Option<String> = connection
        .query_row(
            "SELECT payload FROM reactive_bruhs WHERE agent_id=?1",
            [agent_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(payload.map(decode).transpose()?.unwrap_or_default())
}

fn save(
    connection: &Connection,
    agent_id: &str,
    profile: &BruhProfile,
) -> Result<(), ReactiveError> {
    connection.execute(
        "INSERT INTO reactive_bruhs(agent_id,payload) VALUES(?1,?2) ON CONFLICT(agent_id) DO UPDATE SET payload=excluded.payload",
        params![agent_id, encode(profile)?],
    )?;
    Ok(())
}

/// Look and effort edits write only the profile. An access change also stops
/// the bruh's running and pending runs; its session and pause stay.
pub(in crate::reactive) fn set_bruh(
    connection: &Connection,
    agent_id: &str,
    look: BruhLook,
    access: BruhAccess,
    effort: Option<String>,
) -> Result<(), ReactiveError> {
    let agent: ReactiveAgent = required(connection, "reactive_agents", agent_id, "agent")?;
    if effort
        .as_deref()
        .is_some_and(|effort| claude_effort(effort).is_none())
    {
        return Err(ReactiveError::Invalid("Choose a supported effort.".into()));
    }
    if let Some(reason) = level_unavailable(access.level, AgentProvider::for_model(&agent.model)) {
        return Err(ReactiveError::Invalid(reason.into()));
    }
    let old = profile(connection, agent_id)?;
    if old.access != access {
        delete_grants(connection, agent_id)?;
        // Compared at the level the runtime runs: a Codex bruh stored at Ask
        // already runs read only, so saving it as Read only changes nothing.
        let provider = AgentProvider::for_model(&agent.model);
        let effective = |access: &BruhAccess| BruhAccess {
            level: effective_level(access.level, provider),
            ..access.clone()
        };
        if effective(&old.access) != effective(&access) {
            cancel_runs(connection, "agent_id", agent_id, ACCESS_CHANGED)?;
        }
    }
    save(
        connection,
        agent_id,
        &BruhProfile {
            look,
            access,
            effort,
            ..old
        },
    )
}

const STOPPED: &str = "Stopped by the owner.";

fn agent_ids(connection: &Connection) -> Result<Vec<String>, ReactiveError> {
    Ok(records::<ReactiveAgent>(connection, "reactive_agents")?
        .into_iter()
        .map(|agent| agent.id)
        .collect())
}

/// Cancel pending and running runs and deny open approvals, for one agent or
/// every agent (report agents included), then pause the bruhs. Report agents
/// are never paused: their schedule is not the crew's.
pub(in crate::reactive) fn stop(
    connection: &Connection,
    agent_id: Option<&str>,
) -> Result<(), ReactiveError> {
    let targets = match agent_id {
        Some(id) => {
            required::<ReactiveAgent>(connection, "reactive_agents", id, "agent")?;
            vec![id.to_owned()]
        }
        None => agent_ids(connection)?,
    };
    let now = EpochMs::now();
    for id in &targets {
        cancel_runs(connection, "agent_id", id, STOPPED)?;
        delete_grants(connection, id)?;
        let mut statement = connection.prepare(
            "SELECT payload FROM reactive_outbox WHERE agent_id=?1 AND json_extract(payload,'$.approval') IS NOT NULL AND json_extract(payload,'$.resolved_at') IS NULL",
        )?;
        let rows = statement.query_map([id], |row| row.get::<_, String>(0))?;
        let items = rows
            .map(|row| decode::<OutboxItem>(row?))
            .collect::<Result<Vec<_>, _>>()?;
        for mut item in items {
            if let Some(approval) = item.approval.as_mut() {
                approval.state = ApprovalState::Denied;
            }
            item.resolved_at = Some(now);
            always_on::save_item(connection, &item)?;
        }
        if agent_id.is_some() || managed_preset_id(id).is_none() {
            let mut saved = profile(connection, id)?;
            saved.paused_at = Some(now);
            save(connection, id, &saved)?;
        }
    }
    Ok(())
}

/// Clear the pause on one bruh, or on every bruh.
pub(in crate::reactive) fn resume(
    connection: &Connection,
    agent_id: Option<&str>,
) -> Result<(), ReactiveError> {
    let targets = match agent_id {
        Some(id) => {
            required::<ReactiveAgent>(connection, "reactive_agents", id, "agent")?;
            vec![id.to_owned()]
        }
        None => agent_ids(connection)?,
    };
    for id in targets {
        let mut saved = profile(connection, &id)?;
        if saved.paused_at.take().is_some() {
            save(connection, &id, &saved)?;
        }
    }
    Ok(())
}

pub(in crate::reactive) fn is_paused(
    connection: &Connection,
    agent_id: &str,
) -> Result<bool, ReactiveError> {
    Ok(profile(connection, agent_id)?.paused_at.is_some())
}

pub(in crate::reactive) fn delete(
    connection: &Connection,
    agent_id: &str,
) -> Result<(), ReactiveError> {
    connection.execute("DELETE FROM reactive_bruhs WHERE agent_id=?1", [agent_id])?;
    delete_grants(connection, agent_id)
}

/// Drop every late approval grant the agent holds.
fn delete_grants(connection: &Connection, agent_id: &str) -> Result<(), ReactiveError> {
    connection.execute(
        "DELETE FROM reactive_approval_grants WHERE agent_id=?1",
        [agent_id],
    )?;
    Ok(())
}

// ── Approvals ──────────────────────────────────────────────────────────────

/// Open an outbox approval for a tool call the run paused on; returns its id.
/// `input` is the call as the gate saw it (paths already resolved).
pub(in crate::reactive) fn open_approval(
    connection: &Connection,
    run: &ReactiveRun,
    tool: &str,
    input: &Value,
) -> Result<String, ReactiveError> {
    let settings = always_on::notification_settings(connection)?;
    let summary = approval_summary(tool, input);
    let item = OutboxItem {
        id: uuid::Uuid::new_v4().to_string(),
        agent_id: run.agent_id.clone(),
        run_id: run.id.clone(),
        kind: OutboxKind::Ask,
        title: "Approval needed".into(),
        message: summary.clone(),
        created_at: EpochMs::now(),
        read_at: None,
        resolved_at: None,
        answer: None,
        system: if settings.wants_system(OutboxKind::Ask) {
            SystemDelivery::Pending
        } else {
            SystemDelivery::NotRequested
        },
        approval: Some(ApprovalRequest {
            tool: tool.into(),
            summary,
            fingerprint: approval_fingerprint(tool, input),
            state: ApprovalState::Waiting,
            detail: approval_detail(tool, input),
        }),
    };
    always_on::save_item(connection, &item)?;
    rooms::ask_opened(connection, run, &item)?;
    Ok(item.id)
}

/// One approval item.
pub(in crate::reactive) fn approval(
    connection: &Connection,
    id: &str,
) -> Result<OutboxItem, ReactiveError> {
    let item: OutboxItem = required(connection, "reactive_outbox", id, "approval")?;
    if item.approval.is_none() {
        return Err(ReactiveError::NotFound(
            "The approval was not found.".into(),
        ));
    }
    Ok(item)
}

/// The owner's answer to an approval. A run still waiting reads it directly.
/// A run that already ended is resumed with a new run; an approval also
/// leaves a one-shot grant so the resumed run's same call doesn't ask again.
pub(in crate::reactive) fn decide_approval(
    connection: &Connection,
    id: &str,
    approve: bool,
) -> Result<(), ReactiveError> {
    let mut item = approval(connection, id)?;
    let decided = if approve {
        ApprovalState::Approved
    } else {
        ApprovalState::Denied
    };
    let Some(request) = item.approval.as_mut() else {
        return Err(ReactiveError::NotFound(
            "The approval was not found.".into(),
        ));
    };
    if request.state == decided {
        return Ok(());
    }
    if matches!(
        request.state,
        ApprovalState::Approved | ApprovalState::Denied
    ) {
        return Err(ReactiveError::Conflict(
            "This request was already decided.".into(),
        ));
    }
    let (summary, fingerprint) = (request.summary.clone(), request.fingerprint.clone());
    let live = run_waits_on(connection, &item)?;
    if let Some(request) = item.approval.as_mut() {
        request.state = decided;
    }
    let now = EpochMs::now();
    if item.read_at.is_none() {
        item.read_at = Some(now);
    }
    skip_system(&mut item);
    item.resolved_at = Some(now);
    if !live {
        let agent = enabled_agent(connection, &item.agent_id)?;
        queue_capacity(connection)?;
        let prompt = if approve {
            format!("The owner approved your request: {summary}. Do it now.")
        } else {
            format!("The owner denied your request: {summary}. Continue without it.")
        };
        let mut run = new_run(connection, &agent, prompt, None, None, RunStatus::Queued, 0)?;
        // A room run resumes in its room while the bruh is still in it.
        let original = get::<ReactiveRun>(connection, "reactive_runs", &item.run_id)?;
        run.trigger = match original {
            Some(original) => rooms::resume_trigger_for(connection, &original)?,
            None => None,
        }
        .or(Some(RunTrigger::Answer {
            outbox_id: item.id.clone(),
        }));
        save_run(connection, &run)?;
        if approve {
            connection.execute(
                "INSERT INTO reactive_approval_grants(agent_id,fingerprint,created) VALUES(?1,?2,?3) ON CONFLICT(agent_id,fingerprint) DO UPDATE SET created=excluded.created",
                params![item.agent_id, fingerprint, now.as_millis()],
            )?;
        }
    }
    always_on::save_item(connection, &item)
}

/// Whether the item's run is still waiting on it: the approval is Waiting
/// and the run Running. An expired approval's run may still be wrapping up,
/// but nothing reads the answer, so it is resumed like an ended run.
pub(in crate::reactive) fn run_waits_on(
    connection: &Connection,
    item: &OutboxItem,
) -> Result<bool, ReactiveError> {
    Ok(item
        .approval
        .as_ref()
        .is_some_and(|request| request.state == ApprovalState::Waiting)
        && run_record(connection, &item.run_id)?.status == RunStatus::Running)
}

/// The run stopped waiting; the approval stays open so a late answer resumes it.
pub(in crate::reactive) fn expire_approval(
    connection: &Connection,
    id: &str,
) -> Result<(), ReactiveError> {
    let mut item = approval(connection, id)?;
    match item.approval.as_mut() {
        Some(request) if request.state == ApprovalState::Waiting => {
            request.state = ApprovalState::Expired;
            always_on::save_item(connection, &item)
        }
        _ => Ok(()),
    }
}

/// Close an approval whose run stopped while it waited (cancelled, merged
/// into a newer run or interrupted), so a late answer can't resume it.
pub(in crate::reactive) fn withdraw_approval(
    connection: &Connection,
    id: &str,
) -> Result<(), ReactiveError> {
    let mut item = approval(connection, id)?;
    match item.approval.as_mut() {
        Some(request) if request.state == ApprovalState::Waiting && item.resolved_at.is_none() => {
            request.state = ApprovalState::Denied;
            item.resolved_at = Some(EpochMs::now());
            skip_system(&mut item);
            always_on::save_item(connection, &item)
        }
        _ => Ok(()),
    }
}

/// A system notification not sent yet is no longer worth sending.
fn skip_system(item: &mut OutboxItem) {
    if item.system == SystemDelivery::Pending {
        item.system = SystemDelivery::Skipped;
    }
}

/// Expire the approvals a run was waiting on when it stopped without
/// answering them (a restart, or a retry that reuses the run id). They stay
/// open, so a late answer still resumes the bruh with a new run.
pub(in crate::reactive) fn expire_run_approvals(
    connection: &Connection,
    run_id: &str,
) -> Result<(), ReactiveError> {
    let mut statement = connection.prepare(
        "SELECT payload FROM reactive_outbox WHERE json_extract(payload,'$.run_id')=?1 AND json_extract(payload,'$.approval.state')='waiting' AND json_extract(payload,'$.resolved_at') IS NULL",
    )?;
    let rows = statement.query_map([run_id], |row| row.get::<_, String>(0))?;
    let items = rows
        .map(|row| decode::<OutboxItem>(row?))
        .collect::<Result<Vec<_>, _>>()?;
    for mut item in items {
        if let Some(request) = item.approval.as_mut() {
            request.state = ApprovalState::Expired;
        }
        skip_system(&mut item);
        always_on::save_item(connection, &item)?;
    }
    Ok(())
}

/// Withdraw every approval of a run that was cancelled, waiting or expired,
/// so a late answer can't resume it.
pub(in crate::reactive) fn withdraw_run_approvals(
    connection: &Connection,
    run_id: &str,
) -> Result<(), ReactiveError> {
    let mut statement = connection.prepare(
        "SELECT payload FROM reactive_outbox WHERE json_extract(payload,'$.run_id')=?1 AND json_extract(payload,'$.approval') IS NOT NULL AND json_extract(payload,'$.resolved_at') IS NULL",
    )?;
    let rows = statement.query_map([run_id], |row| row.get::<_, String>(0))?;
    let items = rows
        .map(|row| decode::<OutboxItem>(row?))
        .collect::<Result<Vec<_>, _>>()?;
    for mut item in items {
        if let Some(request) = item.approval.as_mut() {
            request.state = ApprovalState::Denied;
        }
        item.resolved_at = Some(EpochMs::now());
        skip_system(&mut item);
        always_on::save_item(connection, &item)?;
    }
    Ok(())
}

/// Use up a late approval's grant for this exact call, if there is one and
/// it is younger than [`GRANT_TTL_MS`]. An older grant is dropped unused.
pub(in crate::reactive) fn take_grant(
    connection: &Connection,
    agent_id: &str,
    fingerprint: &str,
) -> Result<bool, ReactiveError> {
    let cutoff = EpochMs::now().as_millis().saturating_sub(GRANT_TTL_MS);
    connection.execute(
        "DELETE FROM reactive_approval_grants WHERE created<?1",
        [cutoff],
    )?;
    Ok(connection.execute(
        "DELETE FROM reactive_approval_grants WHERE agent_id=?1 AND fingerprint=?2",
        params![agent_id, fingerprint],
    )? > 0)
}

fn count(connection: &Connection, sql: &str, agent_id: &str) -> Result<usize, ReactiveError> {
    let count: i64 = connection.query_row(sql, [agent_id], |row| row.get(0))?;
    Ok(usize::try_from(count).unwrap_or_default())
}

/// Earliest pending follow-up or enabled calendar subscription for the agent.
fn next_wake(
    connection: &Connection,
    agent_id: &str,
    subscriptions: &[Subscription],
    sources: &[EventSource],
    now: EpochMs,
) -> Result<Option<EpochMs>, ReactiveError> {
    let follow_up: Option<i64> = connection.query_row(
        "SELECT MIN(due) FROM reactive_follow_ups WHERE agent_id=?1 AND status='pending'",
        [agent_id],
        |row| row.get(0),
    )?;
    let calendar = subscriptions
        .iter()
        .filter(|subscription| subscription.enabled && subscription.agent_id == agent_id)
        .filter_map(|subscription| {
            sources
                .iter()
                .find(|source| source.id == subscription.source_id && source.enabled)
        })
        .filter_map(|source| match &source.kind {
            SourceKind::Calendar {
                times,
                weekdays,
                time_zone,
            } => clock::next_calendar_due(times, weekdays, time_zone, now.as_millis()).ok(),
            _ => None,
        });
    Ok(follow_up
        .into_iter()
        .chain(calendar)
        .min()
        .and_then(|due| EpochMs::new(due).ok()))
}

/// Every bruh as the rail shows it. Weekly report presets are not bruhs.
pub(in crate::reactive) fn bruh_states(
    connection: &Connection,
    now: EpochMs,
) -> Result<Vec<BruhState>, ReactiveError> {
    let subscriptions = records::<Subscription>(connection, "reactive_subscriptions")?;
    let sources = records::<EventSource>(connection, "reactive_sources")?;
    let mut states = Vec::new();
    for agent in records::<ReactiveAgent>(connection, "reactive_agents")?
        .into_iter()
        .filter(|agent| managed_preset_id(&agent.id).is_none())
    {
        let profile = profile(connection, &agent.id)?;
        let open_asks = count(
            connection,
            "SELECT COUNT(*) FROM reactive_outbox WHERE agent_id=?1 AND json_extract(payload,'$.kind')='ask' AND json_extract(payload,'$.resolved_at') IS NULL",
            &agent.id,
        )?;
        let awaiting_review = count(
            connection,
            "SELECT COUNT(*) FROM reactive_runs WHERE agent_id=?1 AND status='awaiting_review'",
            &agent.id,
        )?;
        let running = count(
            connection,
            "SELECT COUNT(*) FROM reactive_runs WHERE agent_id=?1 AND status='running'",
            &agent.id,
        )?;
        let unread = count(
            connection,
            "SELECT COUNT(*) FROM reactive_outbox WHERE agent_id=?1 AND json_extract(payload,'$.read_at') IS NULL AND json_extract(payload,'$.resolved_at') IS NULL",
            &agent.id,
        )?;
        let next_wake_ms = next_wake(connection, &agent.id, &subscriptions, &sources, now)?;
        states.push(BruhState {
            status: status_for(
                agent.enabled,
                profile.paused_at.is_some(),
                open_asks > 0 || awaiting_review > 0,
                running > 0,
            ),
            agent_id: agent.id,
            name: agent.name,
            look: profile.look,
            access: profile.access,
            effort: profile.effort,
            next_wake_ms,
            unread,
            computer: profile.computer,
        });
    }
    rail_order(&mut states);
    Ok(states)
}
