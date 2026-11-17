//! Outbox, agent-written memory, follow-ups, notification settings and draft
//! decisions. Everything a completed run reports commits with its receipt.

use super::{
    decode, enabled_agent, encode, get, new_run, queue_capacity, required, run_record, save_run,
    text, valid_id, ReactiveError,
};
use crate::reactive::clock;
use bruhs_models::{reactive::*, EpochMs};
use rusqlite::{params, Connection, OptionalExtension};

/// Recent resolved updates kept in a snapshot, beside every open question.
const OUTBOX_PAGE: i64 = 50;
/// Recent fired or cancelled follow-ups kept in a snapshot.
const FOLLOW_UP_HISTORY: i64 = 20;
/// Automatic memory tidy-ups per agent are at most daily.
const CONSOLIDATION_GAP_MS: i64 = 24 * 60 * 60 * 1000;

pub(super) fn initialize(connection: &Connection) -> Result<(), ReactiveError> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS reactive_outbox(id TEXT PRIMARY KEY,agent_id TEXT NOT NULL,run_id TEXT NOT NULL UNIQUE,created INTEGER NOT NULL,payload TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS reactive_outbox_agent ON reactive_outbox(agent_id,created);
        CREATE TABLE IF NOT EXISTS reactive_memory(agent_id TEXT NOT NULL,key TEXT NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(agent_id,key));
        CREATE TABLE IF NOT EXISTS reactive_follow_ups(id TEXT PRIMARY KEY,agent_id TEXT NOT NULL,key TEXT NOT NULL,status TEXT NOT NULL,due INTEGER NOT NULL,created INTEGER NOT NULL,payload TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS reactive_follow_ups_due ON reactive_follow_ups(status,due);
        CREATE TABLE IF NOT EXISTS reactive_agent_state(agent_id TEXT PRIMARY KEY,memory_rev INTEGER NOT NULL DEFAULT 0,memory_seen_rev INTEGER NOT NULL DEFAULT -1,last_consolidation INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE IF NOT EXISTS reactive_settings(id TEXT PRIMARY KEY,payload TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS reactive_session_stamps(agent_id TEXT PRIMARY KEY,stamp TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS reactive_draft_outcomes(workflow_id TEXT NOT NULL,outcome TEXT NOT NULL,run_id TEXT NOT NULL,created INTEGER NOT NULL,PRIMARY KEY(workflow_id,outcome));",
    )?;
    Ok(())
}

// ── Settings ───────────────────────────────────────────────────────────────

pub(in crate::reactive) fn notification_settings(
    connection: &Connection,
) -> Result<NotificationSettings, ReactiveError> {
    Ok(get(connection, "reactive_settings", "notifications")?.unwrap_or_default())
}

pub(super) fn set_notification_settings(
    connection: &Connection,
    settings: &NotificationSettings,
) -> Result<(), ReactiveError> {
    if let Some(hours) = &settings.quiet_hours {
        if parse_hhmm(&hours.start).is_none() || parse_hhmm(&hours.end).is_none() {
            return Err(ReactiveError::Invalid(
                "Quiet hours use 24-hour HH:MM times.".into(),
            ));
        }
        clock::zone(&hours.time_zone)?;
    }
    super::save(connection, "reactive_settings", "notifications", settings)
}

// ── Outbox ─────────────────────────────────────────────────────────────────

/// Save an outbox item. The `run_id` column holds one update per run, so an
/// approval (a run can open several) is keyed by its own id there instead.
pub(super) fn save_item(connection: &Connection, item: &OutboxItem) -> Result<(), ReactiveError> {
    let run_key = if item.approval.is_some() {
        format!("approval:{}", item.id)
    } else {
        item.run_id.clone()
    };
    connection.execute(
        "INSERT INTO reactive_outbox(id,agent_id,run_id,created,payload) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload",
        params![item.id, item.agent_id, run_key, item.created_at.as_millis(), encode(item)?],
    )?;
    Ok(())
}

fn item(connection: &Connection, id: &str) -> Result<OutboxItem, ReactiveError> {
    required(connection, "reactive_outbox", id, "update")
}

pub(super) fn outbox(connection: &Connection) -> Result<Vec<OutboxItem>, ReactiveError> {
    let mut statement = connection.prepare(
        "SELECT payload FROM reactive_outbox WHERE json_extract(payload,'$.kind')='ask' AND json_extract(payload,'$.resolved_at') IS NULL
         UNION SELECT payload FROM (SELECT payload FROM reactive_outbox ORDER BY created DESC,rowid DESC LIMIT ?1)",
    )?;
    let rows = statement.query_map([OUTBOX_PAGE], |row| row.get::<_, String>(0))?;
    let mut items = rows
        .map(|row| decode::<OutboxItem>(row?))
        .collect::<Result<Vec<_>, _>>()?;
    items.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.id.cmp(&a.id))
    });
    Ok(items)
}

fn create_item(
    connection: &Connection,
    run: &ReactiveRun,
    kind: OutboxKind,
    title: String,
    message: String,
) -> Result<(), ReactiveError> {
    let settings = notification_settings(connection)?;
    let item = OutboxItem {
        id: uuid::Uuid::new_v4().to_string(),
        agent_id: run.agent_id.clone(),
        run_id: run.id.clone(),
        kind,
        title,
        message,
        created_at: EpochMs::now(),
        read_at: None,
        resolved_at: None,
        answer: None,
        system: if settings.wants_system(kind) {
            SystemDelivery::Pending
        } else {
            SystemDelivery::NotRequested
        },
        approval: None,
    };
    // A retried run reuses its receipt; its earlier update stays the record.
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM reactive_outbox WHERE run_id=?1)",
        [&run.id],
        |row| row.get(0),
    )?;
    if !exists {
        save_item(connection, &item)?;
    }
    Ok(())
}

fn read(item: &mut OutboxItem, now: EpochMs) {
    if item.read_at.is_none() {
        item.read_at = Some(now);
    }
    if item.system == SystemDelivery::Pending {
        item.system = SystemDelivery::Skipped;
    }
}

pub(super) fn mark_read(connection: &Connection, ids: &[String]) -> Result<(), ReactiveError> {
    let now = EpochMs::now();
    let items = if ids.is_empty() {
        let mut statement = connection.prepare(
            "SELECT payload FROM reactive_outbox WHERE json_extract(payload,'$.read_at') IS NULL",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|row| decode::<OutboxItem>(row?))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        ids.iter()
            .map(|id| item(connection, id))
            .collect::<Result<Vec<_>, _>>()?
    };
    for mut item in items {
        read(&mut item, now);
        save_item(connection, &item)?;
    }
    Ok(())
}

pub(super) fn dismiss(connection: &Connection, id: &str) -> Result<(), ReactiveError> {
    let mut item = item(connection, id)?;
    if item.resolved_at.is_some() {
        return Ok(());
    }
    let now = EpochMs::now();
    read(&mut item, now);
    item.resolved_at = Some(now);
    // Dismissing an approval says no, so a waiting run stops waiting.
    if let Some(request) = item.approval.as_mut() {
        request.state = bruhs_models::crew::ApprovalState::Denied;
    }
    save_item(connection, &item)
}

pub(super) fn answer(
    connection: &Connection,
    id: &str,
    message: String,
) -> Result<(), ReactiveError> {
    text(&message, "Reply", true)?;
    let mut item = item(connection, id)?;
    if item.resolved_at.is_some() {
        return Err(ReactiveError::Conflict(
            "This update was already answered or dismissed.".into(),
        ));
    }
    // A reply to an approval is a no with reasons. A run still waiting on it
    // relays the reply itself; any other run is resumed as for a question.
    let waiting = super::crew::run_waits_on(connection, &item)?;
    if let Some(request) = item.approval.as_mut() {
        request.state = bruhs_models::crew::ApprovalState::Denied;
    }
    if !waiting {
        let agent = enabled_agent(connection, &item.agent_id)?;
        queue_capacity(connection)?;
        let what = match item.kind {
            OutboxKind::Ask if item.approval.is_some() => "approval request",
            OutboxKind::Ask => "question",
            OutboxKind::Notify => "update",
        };
        let prompt = format!(
            "The owner replied to your {what} “{}”.\n\nYou wrote:\n{}\n\nTheir reply:\n{message}",
            item.title, item.message
        );
        let mut run = new_run(connection, &agent, prompt, None, None, RunStatus::Queued, 0)?;
        run.trigger = Some(RunTrigger::Answer {
            outbox_id: item.id.clone(),
        });
        save_run(connection, &run)?;
    }
    let now = EpochMs::now();
    read(&mut item, now);
    item.resolved_at = Some(now);
    item.answer = Some(message);
    save_item(connection, &item)
}

pub(in crate::reactive) fn signal(connection: &Connection) -> Result<OutboxSignal, ReactiveError> {
    let (unread, open_questions): (i64, i64) = connection.query_row(
        "SELECT
            COALESCE(SUM(json_extract(payload,'$.read_at') IS NULL AND json_extract(payload,'$.resolved_at') IS NULL),0),
            COALESCE(SUM(json_extract(payload,'$.kind')='ask' AND json_extract(payload,'$.resolved_at') IS NULL),0)
         FROM reactive_outbox",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let latest: Option<String> = connection
        .query_row(
            "SELECT payload FROM reactive_outbox WHERE json_extract(payload,'$.read_at') IS NULL AND json_extract(payload,'$.resolved_at') IS NULL ORDER BY created DESC,rowid DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(OutboxSignal {
        unread: usize::try_from(unread).unwrap_or_default(),
        open_questions: usize::try_from(open_questions).unwrap_or_default(),
        latest_unread: latest.map(decode).transpose()?,
        bruhs: super::crew::bruh_states(connection, EpochMs::now())?,
        protocol: bruhs_models::crew::CREW_PROTOCOL,
    })
}

/// Items waiting for a system notification, oldest first.
pub(in crate::reactive) fn pending_deliveries(
    connection: &Connection,
) -> Result<Vec<OutboxItem>, ReactiveError> {
    let mut statement = connection.prepare(
        "SELECT payload FROM reactive_outbox WHERE json_extract(payload,'$.system.state')='pending' ORDER BY created,rowid",
    )?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    rows.map(|row| decode(row?)).collect()
}

pub(in crate::reactive) fn record_delivery(
    connection: &Connection,
    ids: &[String],
    delivery: &SystemDelivery,
) -> Result<(), ReactiveError> {
    for id in ids {
        let mut item = item(connection, id)?;
        // Read in the app meanwhile: nothing more to deliver.
        if item.system == SystemDelivery::Pending {
            item.system = delivery.clone();
            save_item(connection, &item)?;
        }
    }
    Ok(())
}

fn recent_told(connection: &Connection, agent_id: &str) -> Result<Vec<OutboxItem>, ReactiveError> {
    let mut statement = connection.prepare(
        "SELECT payload FROM reactive_outbox WHERE agent_id=?1 AND json_extract(payload,'$.approval') IS NULL ORDER BY created DESC,rowid DESC LIMIT ?2",
    )?;
    let limit = i64::try_from(RECENT_TOLD).unwrap_or(5);
    let rows = statement.query_map(params![agent_id, limit], |row| row.get::<_, String>(0))?;
    rows.map(|row| decode(row?)).collect()
}

// ── Memory ─────────────────────────────────────────────────────────────────

pub(super) fn memories(connection: &Connection) -> Result<Vec<MemoryEntry>, ReactiveError> {
    let mut statement =
        connection.prepare("SELECT payload FROM reactive_memory ORDER BY agent_id,key")?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    rows.map(|row| decode(row?)).collect()
}

fn agent_memories(
    connection: &Connection,
    agent_id: &str,
) -> Result<Vec<MemoryEntry>, ReactiveError> {
    let mut statement =
        connection.prepare("SELECT payload FROM reactive_memory WHERE agent_id=?1 ORDER BY key")?;
    let rows = statement.query_map([agent_id], |row| row.get::<_, String>(0))?;
    rows.map(|row| decode(row?)).collect()
}

fn ensure_state(connection: &Connection, agent_id: &str) -> Result<(), ReactiveError> {
    connection.execute(
        "INSERT OR IGNORE INTO reactive_agent_state(agent_id) VALUES(?1)",
        [agent_id],
    )?;
    Ok(())
}

/// Memory changed by someone other than the agent; the next run repeats it.
fn bump_revision(connection: &Connection, agent_id: &str) -> Result<(), ReactiveError> {
    ensure_state(connection, agent_id)?;
    connection.execute(
        "UPDATE reactive_agent_state SET memory_rev=memory_rev+1 WHERE agent_id=?1",
        [agent_id],
    )?;
    Ok(())
}

fn budget_error(entries: &[MemoryEntry]) -> Option<String> {
    let chars: usize = entries.iter().map(|entry| entry.text.chars().count()).sum();
    (entries.len() > MAX_MEMORY_ENTRIES || chars > MAX_MEMORY_CHARS).then(|| {
        format!("Memory is full ({MAX_MEMORY_ENTRIES} entries or {MAX_MEMORY_CHARS} characters). Forget stale entries first.")
    })
}

fn write_entry(connection: &Connection, entry: &MemoryEntry) -> Result<(), ReactiveError> {
    connection.execute(
        "INSERT INTO reactive_memory(agent_id,key,payload) VALUES(?1,?2,?3) ON CONFLICT(agent_id,key) DO UPDATE SET payload=excluded.payload",
        params![entry.agent_id, entry.key, encode(entry)?],
    )?;
    Ok(())
}

pub(super) fn edit_memory(
    connection: &Connection,
    agent_id: &str,
    key: &str,
    kind: MemoryKind,
    value: String,
) -> Result<(), ReactiveError> {
    required::<ReactiveAgent>(connection, "reactive_agents", agent_id, "agent")?;
    if !valid_key(key) {
        return Err(ReactiveError::Invalid(
            "Memory keys are 1 to 64 characters: lowercase letters, digits, hyphens, underscores or dots.".into(),
        ));
    }
    if value.trim().is_empty() || value.chars().count() > MAX_MEMORY_ENTRY_CHARS {
        return Err(ReactiveError::Invalid(format!(
            "A memory entry needs 1 to {MAX_MEMORY_ENTRY_CHARS} characters."
        )));
    }
    let entry = MemoryEntry {
        agent_id: agent_id.into(),
        key: key.into(),
        kind,
        text: value,
        updated_at: EpochMs::now(),
        run_id: None,
    };
    let mut entries = agent_memories(connection, agent_id)?;
    entries.retain(|existing| existing.key != key);
    entries.push(entry.clone());
    if let Some(error) = budget_error(&entries) {
        return Err(ReactiveError::Conflict(error));
    }
    write_entry(connection, &entry)?;
    bump_revision(connection, agent_id)
}

pub(super) fn forget_memory(
    connection: &Connection,
    agent_id: &str,
    key: &str,
) -> Result<(), ReactiveError> {
    let removed = connection.execute(
        "DELETE FROM reactive_memory WHERE agent_id=?1 AND key=?2",
        params![agent_id, key],
    )?;
    if removed > 0 {
        bump_revision(connection, agent_id)?;
    }
    Ok(())
}

pub(super) fn delete_agent(connection: &Connection, agent_id: &str) -> Result<(), ReactiveError> {
    connection.execute(
        "DELETE FROM reactive_session_stamps WHERE agent_id=?1",
        [agent_id],
    )?;
    connection.execute("DELETE FROM reactive_memory WHERE agent_id=?1", [agent_id])?;
    connection.execute(
        "DELETE FROM reactive_agent_state WHERE agent_id=?1",
        [agent_id],
    )?;
    for mut follow_up in pending_for(connection, agent_id)? {
        follow_up.status = FollowUpStatus::Cancelled;
        save_follow_up(connection, &follow_up)?;
    }
    Ok(())
}

pub(super) fn consolidate(
    connection: &Connection,
    agent_id: &str,
    automatic: bool,
) -> Result<bool, ReactiveError> {
    let agent = enabled_agent(connection, agent_id)?;
    ensure_state(connection, agent_id)?;
    let pending: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM reactive_runs WHERE agent_id=?1 AND status IN ('awaiting_review','queued','running') AND json_extract(payload,'$.trigger.type')='consolidation')",
        [agent_id],
        |row| row.get(0),
    )?;
    if pending {
        return Ok(false);
    }
    let now = EpochMs::now().as_millis();
    if automatic {
        let last: i64 = connection.query_row(
            "SELECT last_consolidation FROM reactive_agent_state WHERE agent_id=?1",
            [agent_id],
            |row| row.get(0),
        )?;
        if now.saturating_sub(last) < CONSOLIDATION_GAP_MS {
            return Ok(false);
        }
    }
    queue_capacity(connection)?;
    let prompt = format!(
        "Tidy your memory. Merge duplicates, rewrite entries that are wrong or out of date, drop anything you can look up again, and keep it within {MAX_MEMORY_ENTRIES} entries and {MAX_MEMORY_CHARS} characters. Use remember and forget in your report to apply the changes. Look for patterns across runs worth keeping as one note. Stay silent unless you found something the owner must correct."
    );
    let mut run = new_run(connection, &agent, prompt, None, None, RunStatus::Queued, 0)?;
    run.trigger = Some(RunTrigger::Consolidation);
    save_run(connection, &run)?;
    connection.execute(
        "UPDATE reactive_agent_state SET last_consolidation=?2 WHERE agent_id=?1",
        params![agent_id, now],
    )?;
    Ok(true)
}

// ── Follow-ups ─────────────────────────────────────────────────────────────

fn follow_up_status(status: FollowUpStatus) -> &'static str {
    match status {
        FollowUpStatus::Pending => "pending",
        FollowUpStatus::Fired => "fired",
        FollowUpStatus::Cancelled => "cancelled",
    }
}

fn save_follow_up(connection: &Connection, follow_up: &FollowUp) -> Result<(), ReactiveError> {
    connection.execute(
        "INSERT INTO reactive_follow_ups(id,agent_id,key,status,due,created,payload) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(id) DO UPDATE SET status=excluded.status,due=excluded.due,payload=excluded.payload",
        params![follow_up.id, follow_up.agent_id, follow_up.key, follow_up_status(follow_up.status), follow_up.due_at.as_millis(), follow_up.created_at.as_millis(), encode(follow_up)?],
    )?;
    Ok(())
}

fn follow_ups_where(
    connection: &Connection,
    predicate: &str,
    argument: Option<&str>,
) -> Result<Vec<FollowUp>, ReactiveError> {
    let mut statement = connection.prepare(&format!(
        "SELECT payload FROM reactive_follow_ups WHERE {predicate} ORDER BY due,rowid"
    ))?;
    let rows = statement.query_map(rusqlite::params_from_iter(argument), |row| {
        row.get::<_, String>(0)
    })?;
    rows.map(|row| decode(row?)).collect()
}

fn pending_for(connection: &Connection, agent_id: &str) -> Result<Vec<FollowUp>, ReactiveError> {
    follow_ups_where(
        connection,
        "agent_id=?1 AND status='pending'",
        Some(agent_id),
    )
}

pub(super) fn follow_ups(connection: &Connection) -> Result<Vec<FollowUp>, ReactiveError> {
    let mut items = follow_ups_where(connection, "status='pending'", None)?;
    let mut statement = connection.prepare(
        "SELECT payload FROM reactive_follow_ups WHERE status<>'pending' ORDER BY due DESC,rowid DESC LIMIT ?1",
    )?;
    let rows = statement.query_map([FOLLOW_UP_HISTORY], |row| row.get::<_, String>(0))?;
    for row in rows {
        items.push(decode(row?)?);
    }
    Ok(items)
}

pub(super) fn cancel_follow_up(connection: &Connection, id: &str) -> Result<(), ReactiveError> {
    let mut follow_up = follow_up_record(connection, id)?
        .ok_or_else(|| ReactiveError::NotFound("The follow-up was not found.".into()))?;
    if follow_up.status == FollowUpStatus::Pending {
        follow_up.status = FollowUpStatus::Cancelled;
        save_follow_up(connection, &follow_up)?;
    }
    Ok(())
}

pub(super) fn follow_up_record(
    connection: &Connection,
    id: &str,
) -> Result<Option<FollowUp>, ReactiveError> {
    let payload: Option<String> = connection
        .query_row(
            "SELECT payload FROM reactive_follow_ups WHERE id=?1",
            [id],
            |row| row.get(0),
        )
        .optional()?;
    payload.map(decode).transpose()
}

/// Queue a run for every due follow-up. A full queue leaves them pending.
pub(super) fn fire_due(connection: &Connection, now: EpochMs) -> Result<usize, ReactiveError> {
    let due = {
        let mut statement = connection.prepare(
            "SELECT payload FROM reactive_follow_ups WHERE status='pending' AND due<=?1 ORDER BY due,rowid",
        )?;
        let rows = statement.query_map([now.as_millis()], |row| row.get::<_, String>(0))?;
        rows.map(|row| decode::<FollowUp>(row?))
            .collect::<Result<Vec<_>, _>>()?
    };
    let mut fired = 0;
    let zone = clock::local_zone_name();
    for mut follow_up in due {
        let agent = match enabled_agent(connection, &follow_up.agent_id) {
            Ok(agent) => agent,
            // A paused agent's follow-ups wait for it.
            Err(ReactiveError::Conflict(_)) => continue,
            Err(error) => return Err(error),
        };
        if queue_capacity(connection).is_err() {
            break;
        }
        let prompt = format!(
            "A follow-up you scheduled is due.\nKey: {}\nScheduled for: {}\nSet: {}\n\nNote:\n{}",
            follow_up.key,
            clock::describe(follow_up.due_at.as_millis(), &zone),
            clock::describe(follow_up.created_at.as_millis(), &zone),
            follow_up.note
        );
        let status = if follow_up.auto_run {
            RunStatus::Queued
        } else {
            RunStatus::AwaitingReview
        };
        let mut run = new_run(connection, &agent, prompt, None, None, status, 0)?;
        run.trigger = Some(RunTrigger::FollowUp {
            follow_up_id: follow_up.id.clone(),
        });
        save_run(connection, &run)?;
        follow_up.status = FollowUpStatus::Fired;
        follow_up.fired_run_id = Some(run.id);
        save_follow_up(connection, &follow_up)?;
        fired += 1;
    }
    Ok(fired)
}

// ── Draft decisions ────────────────────────────────────────────────────────

pub(super) fn draft_outcome(
    connection: &Connection,
    run_id: &str,
    workflow_id: &str,
    outcome: DraftOutcomeKind,
    applied: u32,
    total: u32,
) -> Result<(), ReactiveError> {
    valid_id(workflow_id)?;
    let origin = run_record(connection, run_id)?;
    let outcome_name = serde_json::to_value(outcome)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default();
    let inserted = connection.execute(
        "INSERT OR IGNORE INTO reactive_draft_outcomes(workflow_id,outcome,run_id,created) VALUES(?1,?2,?3,?4)",
        params![workflow_id, outcome_name, run_id, EpochMs::now().as_millis()],
    )?;
    if inserted == 0 {
        return Ok(());
    }
    // The decision is recorded either way; a paused or deleted agent is not woken.
    let Ok(agent) = enabled_agent(connection, &origin.agent_id) else {
        return Ok(());
    };
    if queue_capacity(connection).is_err() {
        return Ok(());
    }
    let summary = origin
        .result
        .as_deref()
        .and_then(workflow_draft)
        .map_or_else(|| "your proposed changes".to_owned(), |draft| draft.summary);
    let steps = if total == 0 {
        String::new()
    } else {
        format!(" {applied} of {total} steps were applied.")
    };
    let prompt = format!(
        "The owner reviewed the workflow draft from your earlier run: “{summary}”. It was {}.{steps}\nUpdate your memory or follow-ups if this changes what you are tracking. Stay silent unless something needs the owner.",
        outcome.describe()
    );
    let mut run = new_run(connection, &agent, prompt, None, None, RunStatus::Queued, 0)?;
    run.trigger = Some(RunTrigger::DraftOutcome {
        run_id: run_id.into(),
    });
    save_run(connection, &run)
}

// ── Run context and report application ─────────────────────────────────────

/// What the agent sees before the run's own request: its memory (on a fresh
/// session or after someone else changed it) and what it already told the owner.
pub(super) fn run_context(
    connection: &Connection,
    run: &ReactiveRun,
) -> Result<String, ReactiveError> {
    ensure_state(connection, &run.agent_id)?;
    let (rev, seen): (i64, i64) = connection.query_row(
        "SELECT memory_rev,memory_seen_rev FROM reactive_agent_state WHERE agent_id=?1",
        [&run.agent_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let fresh = run.session_id.is_none();
    let consolidation = matches!(run.trigger, Some(RunTrigger::Consolidation));
    let mut sections = Vec::new();
    if fresh || rev != seen || consolidation {
        let entries = agent_memories(connection, &run.agent_id)?;
        let body = if entries.is_empty() {
            "(empty)".to_owned()
        } else {
            entries
                .iter()
                .map(|entry| {
                    format!(
                        "- [{}] {}: {}",
                        entry.kind.label().to_lowercase(),
                        entry.key,
                        entry.text
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let note = if fresh {
            "This is a new conversation. Your saved memory:"
        } else {
            "Your saved memory changed since your last run (the owner edited or tidied it). Current memory:"
        };
        sections.push(format!("{note}\n{body}"));
        connection.execute(
            "UPDATE reactive_agent_state SET memory_seen_rev=memory_rev WHERE agent_id=?1",
            [&run.agent_id],
        )?;
    }
    if fresh {
        let told = recent_told(connection, &run.agent_id)?;
        if !told.is_empty() {
            let lines = told
                .iter()
                .map(|item| {
                    let answer = item
                        .answer
                        .as_deref()
                        .map(|answer| format!(" Owner replied: {answer}"))
                        .unwrap_or_default();
                    format!(
                        "- {}: {}{answer}",
                        item.title,
                        item.message.lines().next().unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            sections.push(format!("Recently told the owner:\n{lines}"));
        }
        let pending = pending_for(connection, &run.agent_id)?;
        if !pending.is_empty() {
            let zone = clock::local_zone_name();
            let lines = pending
                .iter()
                .map(|item| {
                    format!(
                        "- {} at {}: {}",
                        item.key,
                        clock::describe(item.due_at.as_millis(), &zone),
                        item.note
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            sections.push(format!("Your pending follow-ups:\n{lines}"));
        }
    }
    Ok(sections.join("\n\n"))
}

/// Apply a completed run's report: memory, follow-ups and its outbox item.
/// Returns warnings for parts that could not be applied.
pub(super) fn apply_report(
    connection: &Connection,
    run: &ReactiveRun,
    body: &str,
    report: Option<&RunReport>,
    is_weekly_report: bool,
) -> Result<Vec<String>, ReactiveError> {
    let mut warnings = Vec::new();
    if let Some(report) = report {
        let consolidation = matches!(run.trigger, Some(RunTrigger::Consolidation));
        let mut entries = agent_memories(connection, &run.agent_id)?;
        entries.retain(|entry| !report.forget.contains(&entry.key));
        let now = EpochMs::now();
        for write in &report.remember {
            entries.retain(|entry| entry.key != write.key);
            entries.push(MemoryEntry {
                agent_id: run.agent_id.clone(),
                key: write.key.clone(),
                kind: write.kind,
                text: write.text.trim().to_owned(),
                updated_at: now,
                run_id: Some(run.id.clone()),
            });
        }
        if let Some(error) = budget_error(&entries) {
            // Forgetting still applies; new writes wait for a tidy-up.
            warnings.push(format!("{error} New memory from this run was not saved."));
            for key in &report.forget {
                forget_memory(connection, &run.agent_id, key)?;
            }
            if !consolidation {
                consolidate(connection, &run.agent_id, true).ok();
            }
        } else {
            for key in &report.forget {
                connection.execute(
                    "DELETE FROM reactive_memory WHERE agent_id=?1 AND key=?2",
                    params![run.agent_id, key],
                )?;
            }
            for entry in entries
                .iter()
                .filter(|entry| entry.run_id.as_deref() == Some(run.id.as_str()))
            {
                write_entry(connection, entry)?;
            }
            if consolidation {
                // The tidied memory is new to every later session.
                bump_revision(connection, &run.agent_id)?;
            }
        }
        for key in &report.cancel_follow_ups {
            for mut follow_up in pending_for(connection, &run.agent_id)?
                .into_iter()
                .filter(|follow_up| follow_up.key == *key)
            {
                follow_up.status = FollowUpStatus::Cancelled;
                save_follow_up(connection, &follow_up)?;
            }
        }
        let auto_run = run_is_automatic(connection, run)?;
        for request in &report.follow_ups {
            match schedule_follow_up(connection, run, request, auto_run, now) {
                Ok(()) => {}
                Err(ReactiveError::Invalid(message) | ReactiveError::Conflict(message)) => {
                    warnings.push(message);
                }
                Err(error) => return Err(error),
            }
        }
    }
    let (outcome, mut title, mut message) = effective_outcome(body, report);
    let kind = match outcome {
        RunOutcome::Notify => Some(OutboxKind::Notify),
        RunOutcome::Ask => Some(OutboxKind::Ask),
        RunOutcome::Silent if is_weekly_report => {
            title = "Weekly engineering report ready".into();
            message = body
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or_default()
                .trim_start_matches('#')
                .trim()
                .to_owned();
            Some(OutboxKind::Notify)
        }
        RunOutcome::Silent => None,
    };
    if let Some(kind) = kind {
        if title.is_empty() {
            title = "Agent update".into();
        }
        create_item(connection, run, kind, title, message)?;
    }
    Ok(warnings)
}

/// Owner-started runs and opted-in subscriptions run without review.
fn run_is_automatic(connection: &Connection, run: &ReactiveRun) -> Result<bool, ReactiveError> {
    let Some(id) = &run.subscription_id else {
        return Ok(true);
    };
    Ok(
        get::<Subscription>(connection, "reactive_subscriptions", id)?
            .is_some_and(|subscription| subscription.auto_run),
    )
}

fn schedule_follow_up(
    connection: &Connection,
    run: &ReactiveRun,
    request: &FollowUpRequest,
    auto_run: bool,
    now: EpochMs,
) -> Result<(), ReactiveError> {
    let due = match (&request.at, request.in_minutes) {
        (Some(at), None) => clock::parse_rfc3339(at)?,
        (None, Some(minutes)) => i64::try_from(minutes)
            .ok()
            .and_then(|minutes| minutes.checked_mul(60_000))
            .and_then(|delay| now.as_millis().checked_add(delay))
            .ok_or_else(|| ReactiveError::Invalid("The follow-up is too far away.".into()))?,
        _ => {
            return Err(ReactiveError::Invalid(
                "A follow-up needs exactly one of in_minutes or at.".into(),
            ))
        }
    };
    let delay = due.saturating_sub(now.as_millis());
    if delay > MAX_FOLLOW_UP_MS {
        return Err(ReactiveError::Invalid(format!(
            "Follow-up “{}” is more than a year away and was not scheduled.",
            request.key
        )));
    }
    let due_at = EpochMs::new(due.max(now.as_millis()))
        .map_err(|_| ReactiveError::Invalid("The follow-up time is outside the clock.".into()))?;
    let mut pending = pending_for(connection, &run.agent_id)?;
    if let Some(existing) = pending.iter_mut().find(|item| item.key == request.key) {
        existing.due_at = due_at;
        existing.note = request.note.trim().to_owned();
        existing.run_id = Some(run.id.clone());
        existing.auto_run = auto_run;
        return save_follow_up(connection, existing);
    }
    if pending.len() >= MAX_FOLLOW_UPS {
        return Err(ReactiveError::Conflict(format!(
            "This agent already has {MAX_FOLLOW_UPS} pending follow-ups; “{}” was not scheduled.",
            request.key
        )));
    }
    save_follow_up(
        connection,
        &FollowUp {
            id: uuid::Uuid::new_v4().to_string(),
            agent_id: run.agent_id.clone(),
            key: request.key.clone(),
            note: request.note.trim().to_owned(),
            due_at,
            status: FollowUpStatus::Pending,
            created_at: now,
            run_id: Some(run.id.clone()),
            auto_run,
            fired_run_id: None,
        },
    )
}
