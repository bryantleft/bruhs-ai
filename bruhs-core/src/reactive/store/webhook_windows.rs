//! Durable replay identities and coalescing windows are scoped to a subscription.
use super::*;

const WEEK_MS: i64 = 7 * 24 * 60 * 60 * 1000;

pub(super) fn initialize(db: &Connection) -> Result<(), ReactiveError> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS webhook_replay(
        task TEXT NOT NULL, key BLOB NOT NULL, expires INTEGER NOT NULL, window_delivery TEXT, PRIMARY KEY(task,key));
        CREATE INDEX IF NOT EXISTS webhook_replay_expiry ON webhook_replay(expires);
        CREATE TABLE IF NOT EXISTS webhook_windows(
        task TEXT NOT NULL, agent_id TEXT NOT NULL, key BLOB NOT NULL, due INTEGER NOT NULL,
        delivery TEXT NOT NULL, event_id TEXT NOT NULL, PRIMARY KEY(task,delivery));
        CREATE INDEX IF NOT EXISTS webhook_windows_clock ON webhook_windows(due);
        CREATE INDEX IF NOT EXISTS webhook_windows_key ON webhook_windows(task,key,due);
        CREATE TRIGGER IF NOT EXISTS webhook_replay_forget_task AFTER DELETE ON reactive_subscriptions BEGIN
        DELETE FROM webhook_replay WHERE task=OLD.id; DELETE FROM webhook_windows WHERE task=OLD.id; END;")?;
    if !db.column_exists(None, "webhook_windows", "prompt")? {
        db.execute("ALTER TABLE webhook_windows ADD COLUMN prompt TEXT", [])?;
    }
    if !db.column_exists(None, "webhook_replay", "window_delivery")? {
        db.execute(
            "ALTER TABLE webhook_replay ADD COLUMN window_delivery TEXT",
            [],
        )?;
    }
    db.execute("CREATE INDEX IF NOT EXISTS webhook_replay_window ON webhook_replay(task,window_delivery) WHERE window_delivery IS NOT NULL", [])?;
    Ok(())
}

pub(super) fn duplicate(
    db: &Connection,
    task: &str,
    key: &[u8],
    now: EpochMs,
) -> Result<bool, ReactiveError> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM webhook_replay WHERE task=?1 AND key=?2 AND expires>?3)",
        params![task, key, now.as_millis()],
        |row| row.get(0),
    )?)
}

pub(super) fn remember(
    db: &Connection,
    task: &str,
    key: &[u8],
    window_delivery: Option<&str>,
    now: EpochMs,
) -> Result<(), ReactiveError> {
    db.execute(
        "DELETE FROM webhook_replay WHERE expires<=?1",
        [now.as_millis()],
    )?;
    db.execute(
        "INSERT INTO webhook_replay(task,key,expires,window_delivery) VALUES(?1,?2,?3,?4)",
        params![task, key, now.as_millis() + WEEK_MS, window_delivery],
    )?;
    Ok(())
}

pub(super) fn hold(
    db: &Connection,
    task: &Subscription,
    key: &[u8],
    event: &ReactiveEvent,
    prompt: Option<&str>,
    receipt: &mut webhook_deliveries::Receipt,
) -> Result<String, ReactiveError> {
    let now = event.created_at.as_millis();
    let first: Option<String> = db
        .query_row(
            "SELECT delivery FROM webhook_windows WHERE task=?1 AND key=?2 AND due>?3",
            params![task.id, key, now],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(first) = first {
        db.execute(
            "UPDATE webhook_windows SET event_id=?1,prompt=?4 WHERE task=?2 AND delivery=?3",
            params![event.id, task.id, first, prompt],
        )?;
        receipt.record(db, task, WebhookOutcome::Coalesced, None)?;
        receipt.replace_open_preview(db, &task.id, &first)?;
        Ok(first)
    } else {
        queue_capacity(db)?;
        let window = i64::from(task.webhook_dedupe.coalesce_window_seconds.unwrap_or(120));
        db.execute("INSERT INTO webhook_windows(task,agent_id,key,due,delivery,event_id,prompt) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![task.id,task.agent_id,key,now + 1000*window,receipt.id(),event.id,prompt])?;
        receipt.record(db, task, WebhookOutcome::WindowOpened, None)?;
        Ok(receipt.id().to_owned())
    }
}

fn finish_history(
    db: &Connection,
    task: &str,
    delivery: &str,
    outcome: WebhookOutcome,
    reason: Option<&str>,
) -> Result<(), ReactiveError> {
    if outcome == WebhookOutcome::Rejected {
        // No run was dispatched. The sender can retry after the task is enabled or repaired.
        db.execute(
            "DELETE FROM webhook_replay WHERE task=?1 AND window_delivery=?2",
            params![task, delivery],
        )?;
    }
    let row: Option<String> = db
        .query_row(
            "SELECT detail FROM webhook_deliveries WHERE task=?1 AND id=?2",
            params![task, delivery],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(row) = row {
        let mut detail: WebhookDelivery = decode(row)?;
        detail.summary.outcome = outcome;
        detail.error = reason.map(str::to_owned);
        db.execute(
            "UPDATE webhook_deliveries SET summary=?3,detail=?4 WHERE task=?1 AND id=?2",
            params![task, delivery, encode(&detail.summary)?, encode(&detail)?],
        )?;
    }
    Ok(())
}

pub(super) fn cancel(
    db: &Connection,
    field: &str,
    id: &str,
    reason: &str,
) -> Result<(), ReactiveError> {
    let column = match field {
        "subscription_id" => "task",
        "agent_id" => "agent_id",
        _ => return Ok(()),
    };
    let mut query = db.prepare(&format!(
        "SELECT task,delivery FROM webhook_windows WHERE {column}=?1"
    ))?;
    let rows = query.query_map([id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (task, delivery) = row?;
        finish_history(db, &task, &delivery, WebhookOutcome::Rejected, Some(reason))?;
    }
    db.execute(
        &format!("DELETE FROM webhook_windows WHERE {column}=?1"),
        [id],
    )?;
    Ok(())
}

/// Closing a window enqueues precisely its latest event, including after a host restart.
/// The pending window already reserved its place in the queue.
pub(in crate::reactive) fn flush_due(db: &mut Connection) -> Result<usize, ReactiveError> {
    transaction(db, |db| {
        let mut query = db.prepare(
            "SELECT task,delivery,event_id,prompt FROM webhook_windows WHERE due<=?1 ORDER BY due,rowid",
        )?;
        let rows = query
            .query_map([EpochMs::now().as_millis()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let count = rows.len();
        for (task, delivery, event_id, prompt) in rows {
            let sub: Subscription = required(db, "reactive_subscriptions", &task, "task")?;
            let agent: ReactiveAgent = required(db, "reactive_agents", &sub.agent_id, "agent")?;
            let event: ReactiveEvent = required(db, "reactive_events", &event_id, "webhook event")?;
            let status = if sub.auto_run {
                RunStatus::Queued
            } else {
                RunStatus::AwaitingReview
            };
            let prompt = match prompt {
                Some(prompt) => prompt,
                None => event_prompt(&sub.source_id, &sub.instructions, &encode(&event.payload)?),
            };
            let mut run = new_run(db, &agent, prompt, Some(&event), Some(&sub), status, 0)?;
            run.trigger = Some(RunTrigger::WebhookWindow);
            let (outcome, reason) = match check_run_enabled(db, &run) {
                Ok(()) => {
                    save_run(db, &run)?;
                    (
                        if sub.auto_run {
                            WebhookOutcome::Queued
                        } else {
                            WebhookOutcome::AwaitingReview
                        },
                        None,
                    )
                }
                Err(ReactiveError::Conflict(message)) => (WebhookOutcome::Rejected, Some(message)),
                Err(other) => return Err(other),
            };
            finish_history(db, &task, &delivery, outcome, reason.as_deref())?;
            db.execute(
                "DELETE FROM webhook_windows WHERE task=?1 AND delivery=?2",
                params![task, delivery],
            )?;
        }
        Ok(count)
    })
}
