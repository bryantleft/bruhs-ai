//! Route scheduled runs independently of the agent's own thread and provider session.
use super::*;

pub(super) fn initialize(db: &mut Connection) -> Result<(), ReactiveError> {
    transaction(db, |db| {
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS task_conversations(
                thread_id TEXT PRIMARY KEY, agent_id TEXT NOT NULL, project TEXT NOT NULL,
                title TEXT NOT NULL, checkout TEXT NOT NULL, session_id TEXT, stamp TEXT,
                updated INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS deleted_task_runs(id TEXT PRIMARY KEY);
             CREATE INDEX IF NOT EXISTS reactive_runs_thread ON reactive_runs(json_extract(payload,'$.thread_id'));
             CREATE INDEX IF NOT EXISTS task_conversations_project ON task_conversations(project,updated DESC);
             CREATE TABLE IF NOT EXISTS task_destinations(subscription_id TEXT PRIMARY KEY,thread_id TEXT NOT NULL,chosen INTEGER NOT NULL DEFAULT 0);
             CREATE TRIGGER IF NOT EXISTS forget_task_destination AFTER DELETE ON reactive_subscriptions
             BEGIN DELETE FROM task_destinations WHERE subscription_id=OLD.id; END;
             CREATE TRIGGER IF NOT EXISTS forget_agent_conversations AFTER DELETE ON reactive_agents
             BEGIN
                DELETE FROM task_destinations WHERE thread_id IN (SELECT thread_id FROM task_conversations WHERE agent_id=OLD.id);
                DELETE FROM task_conversations WHERE agent_id=OLD.id;
             END;",
        )?;
        if db.table_exists(None, "reactive_task_threads")? {
            db.execute_batch(
                r#"INSERT INTO task_conversations(thread_id,agent_id,project,title,checkout,session_id,stamp,updated)
                   SELECT t.thread_id,t.agent_id,json_extract(a.payload,'$.project_path'),
                     coalesce(json_extract(s.payload,'$.task_title'),json_extract(a.payload,'$.name')),
                     coalesce(json_extract(s.payload,'$.task_checkout'),'{"type":"project"}'),
                     t.session_id,t.stamp,0
                   FROM reactive_task_threads t JOIN reactive_subscriptions s ON s.id=t.subscription_id
                   JOIN reactive_agents a ON a.id=t.agent_id;
                   INSERT INTO task_destinations(subscription_id,thread_id) SELECT t.subscription_id,t.thread_id FROM reactive_task_threads t
                   JOIN task_conversations c ON c.thread_id=t.thread_id;
                   DROP TRIGGER IF EXISTS discard_task_thread;
                   DROP TABLE reactive_task_threads;"#,
            )?;
        }
        Ok(())
    })
}

fn current(db: &Connection, task: &str) -> Result<Option<String>, ReactiveError> {
    Ok(db
        .query_row(
            "SELECT thread_id FROM task_destinations WHERE subscription_id=?1",
            [task],
            |row| row.get(0),
        )
        .optional()?)
}

pub(super) fn for_run(
    db: &Connection,
    agent: &ReactiveAgent,
    sub: Option<&Subscription>,
) -> Result<(String, Option<String>), ReactiveError> {
    let Some(sub) = sub.filter(|sub| !sub.task_conversation.is_agent()) else {
        return Ok(db.query_row(
            "SELECT thread_id,session_id FROM reactive_agents WHERE id=?1",
            [&agent.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?);
    };
    let persistent = sub.task_conversation.has_destination();
    if persistent {
        if let Some(id) = current(db, &sub.id)? {
            db.execute(
                "UPDATE task_conversations SET updated=?2 WHERE thread_id=?1",
                params![id, EpochMs::now().as_millis()],
            )?;
            return Ok((id, None));
        }
    }
    let id = ThreadId::new_v4().to_string();
    db.execute(
        "INSERT INTO task_conversations(thread_id,agent_id,project,title,checkout,updated)
        VALUES(?1,?2,?3,?4,?5,?6)",
        params![
            id,
            agent.id,
            agent.project_path,
            sub.title(agent),
            encode(&sub.task_checkout)?,
            EpochMs::now().as_millis()
        ],
    )?;
    if persistent {
        db.execute(
            "INSERT INTO task_destinations(subscription_id,thread_id) VALUES(?1,?2)",
            params![sub.id, id],
        )?;
    }
    Ok((id, None))
}

/// The server checks the saved destination again at commit time. A stale picker
/// must not redirect another project's schedules or modify a shared agent.
pub(super) fn validate_selection(
    db: &Connection,
    setup: &AutomationSetup,
    id: &ThreadId,
) -> Result<(), ReactiveError> {
    let row: Option<(String, String, String)> = db
        .query_row(
            "SELECT agent_id,project,checkout FROM task_conversations WHERE thread_id=?1",
            [id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let sub = &setup.subscription;
    let Some((agent, project, checkout)) = row else {
        return Err(ReactiveError::Conflict(
            "The selected thread is no longer available.".into(),
        ));
    };
    if !sub.task_conversation.has_destination()
        || agent != setup.agent.id
        || project != setup.agent.project_path
        || decode::<TaskCheckout>(checkout)? != sub.task_checkout
    {
        return Err(ReactiveError::Conflict(
            "Choose a thread in this project with its saved agent and checkout.".into(),
        ));
    }
    let saved: ReactiveAgent = required(db, "reactive_agents", &agent, "agent")?;
    if saved != setup.agent {
        return Err(ReactiveError::Conflict(
            "The thread's agent settings have changed. Reopen the task editor.".into(),
        ));
    }
    Ok(())
}

pub(super) fn bind(db: &Connection, task: &str, id: &ThreadId) -> Result<(), ReactiveError> {
    if current(db, task)?.as_deref() != Some(id.as_str()) {
        cancel_where(db, "subscription_id", task)?;
    }
    db.execute(
        "INSERT INTO task_destinations(subscription_id,thread_id,chosen) VALUES(?1,?2,1)
        ON CONFLICT(subscription_id) DO UPDATE SET thread_id=excluded.thread_id,chosen=1",
        params![task, id.as_str()],
    )?;
    Ok(())
}

pub(super) fn detach(
    db: &Connection,
    task: &str,
    previous: &ThreadId,
) -> Result<(), ReactiveError> {
    let Some(current) = current(db, task)? else {
        return Ok(());
    };
    if current != previous.as_str() {
        return Err(ReactiveError::Conflict(
            "The task’s destination changed. Reopen it before choosing another thread.".into(),
        ));
    }
    cancel_where(db, "subscription_id", task)?;
    db.execute(
        "DELETE FROM task_destinations WHERE subscription_id=?1",
        [task],
    )?;
    Ok(())
}

pub(in crate::reactive) fn search(
    db: &Connection,
    project: &str,
    query: &str,
) -> Result<Vec<TaskThreadChoice>, ReactiveError> {
    let query = query.trim().to_lowercase();
    let mut statement=db.prepare("SELECT c.thread_id,c.title,c.agent_id,c.checkout FROM task_conversations c
        JOIN reactive_agents a ON a.id=c.agent_id WHERE c.project=?1 AND json_extract(a.payload,'$.project_path')=c.project
        ORDER BY c.updated DESC,c.rowid DESC")?;
    let mut rows = statement.query([project])?;
    let mut choices = Vec::new();
    while let Some(row) = rows.next()? {
        let title: String = row.get(1)?;
        if !title.to_lowercase().contains(&query) {
            continue;
        }
        choices.push(choice(row, 0)?);
        if choices.len() == HISTORY_LIMIT {
            break;
        }
    }
    Ok(choices)
}
pub(super) fn uses_agent_thread(db: &Connection, run: &ReactiveRun) -> Result<bool, ReactiveError> {
    Ok(db.query_row(
        "SELECT thread_id=?2 FROM reactive_agents WHERE id=?1",
        params![run.agent_id, run.thread_id.as_str()],
        |row| row.get(0),
    )?)
}
fn stamp(agent: &ReactiveAgent) -> Result<String, ReactiveError> {
    encode(&(
        &agent.project_path,
        &agent.model,
        crate::reactive::worker::instructions_stamp(agent),
    ))
}
pub(super) fn session(
    db: &Connection,
    agent: &ReactiveAgent,
    run: &ReactiveRun,
) -> Result<Option<String>, ReactiveError> {
    let sub = run
        .subscription_id
        .as_deref()
        .map(|id| get::<Subscription>(db, "reactive_subscriptions", id))
        .transpose()?
        .flatten();
    let resume = sub.is_some_and(|sub| {
        sub.task_conversation == TaskConversation::Continue
            || (sub.task_conversation.has_destination()
                && matches!(
                    run.trigger,
                    Some(RunTrigger::Chat | RunTrigger::Answer { .. })
                ))
    });
    if !resume {
        return Ok(None);
    }
    let session: Option<Option<String>> = db.query_row(
        "SELECT c.session_id FROM task_conversations c JOIN task_destinations d ON d.thread_id=c.thread_id
         WHERE c.thread_id=?1 AND c.agent_id=?2 AND c.stamp=?3 AND d.subscription_id=?4",
        params![run.thread_id.as_str(),agent.id,stamp(agent)?,run.subscription_id],|r|r.get(0)).optional()?;
    Ok(session.flatten())
}
pub(super) fn record(
    db: &Connection,
    run: &ReactiveRun,
    model: &str,
    session: &str,
) -> Result<(), ReactiveError> {
    let agent: ReactiveAgent = required(db, "reactive_agents", &run.agent_id, "agent")?;
    if agent.model == model {
        db.execute(
            "UPDATE task_conversations SET session_id=?3, stamp=?4
             WHERE thread_id=?1 AND agent_id=?2",
            params![run.thread_id.as_str(), agent.id, session, stamp(&agent)?],
        )?;
    }
    Ok(())
}

pub(super) fn destinations(
    db: &Connection,
) -> Result<std::collections::BTreeMap<String, ThreadId>, ReactiveError> {
    let mut statement = db.prepare("SELECT subscription_id,thread_id FROM task_destinations")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    rows.map(|row| {
        let (task, thread) = row?;
        let thread = ThreadId::try_from(thread)
            .map_err(|_| ReactiveError::Storage("Invalid saved task thread ID.".into()))?;
        Ok((task, thread))
    })
    .collect()
}

pub(super) fn selected(
    db: &Connection,
) -> Result<std::collections::BTreeMap<String, TaskThreadChoice>, ReactiveError> {
    let mut statement = db.prepare("SELECT d.subscription_id,c.thread_id,c.title,c.agent_id,c.checkout FROM task_destinations d
        JOIN task_conversations c USING(thread_id) WHERE d.chosen=1")?;
    let mut rows = statement.query([])?;
    let mut result = std::collections::BTreeMap::new();
    while let Some(row) = rows.next()? {
        result.insert(row.get(0)?, choice(row, 1)?);
    }
    Ok(result)
}

fn choice(row: &rusqlite::Row<'_>, offset: usize) -> Result<TaskThreadChoice, ReactiveError> {
    let id: String = row.get(offset)?;
    Ok(TaskThreadChoice {
        thread_id: ThreadId::try_from(id)
            .map_err(|_| ReactiveError::Storage("Invalid saved task thread ID.".into()))?,
        title: row.get(offset + 1)?,
        agent_id: row.get(offset + 2)?,
        checkout: decode(row.get(offset + 3)?)?,
    })
}

pub(super) fn reply(
    db: &Connection,
    task: &str,
    thread: &ThreadId,
    message: &str,
) -> Result<(), ReactiveError> {
    text(message, "Thread message", true)?;
    let sub: Subscription = required(db, "reactive_subscriptions", task, "task")?;
    let current = current(db, task)?;
    if !sub.task_conversation.has_destination() || current.as_deref() != Some(thread.as_str()) {
        return Err(ReactiveError::Conflict(
            "This task's destination has changed.".into(),
        ));
    }
    let agent = enabled_agent(db, &sub.agent_id)?;
    queue_capacity(db)?;
    let mut run = new_run(
        db,
        &agent,
        message.to_owned(),
        None,
        Some(&sub),
        RunStatus::Queued,
        0,
    )?;
    run.source_id = Some(sub.source_id);
    run.trigger = Some(RunTrigger::Chat);
    run.owner_message = Some(message.to_owned());
    save_run(db, &run)
}

/// Run IDs can be supplied by a client. Keep only those IDs after deletion so
/// a late provider result or a repeated request cannot revive a deleted turn.
pub(in crate::reactive) fn run_was_deleted(
    db: &Connection,
    id: &str,
) -> Result<bool, ReactiveError> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM deleted_task_runs WHERE id=?1)",
        [id],
        |row| row.get(0),
    )?)
}

pub(super) fn delete(
    db: &Connection,
    thread: &ThreadId,
    expected: &std::collections::BTreeSet<String>,
) -> Result<(), ReactiveError> {
    let id = thread.as_str();
    let agent: Option<String> = db
        .query_row(
            "SELECT agent_id FROM task_conversations WHERE thread_id=?1",
            [id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(agent) = agent else { return Ok(()) };
    reports::guard_managed_identity(db, &agent)?;
    let mut statement =
        db.prepare("SELECT subscription_id FROM task_destinations WHERE thread_id=?1")?;
    let tasks = statement
        .query_map([id], |row| row.get::<_, String>(0))?
        .collect::<Result<std::collections::BTreeSet<_>, _>>()?;
    drop(statement);
    if &tasks != expected {
        return Err(ReactiveError::Conflict(
            "The schedules linked to this thread changed. Review them before deleting it.".into(),
        ));
    }
    for task in tasks {
        reports::guard_managed_identity(db, &task)?;
        apply(db, &ReactiveChange::DeleteSubscription { id: task })?;
    }
    // Approval outbox rows use a synthetic key in the indexed run_id column.
    // Their payload carries the originating turn just like ordinary updates.
    db.execute(
        "DELETE FROM reactive_outbox WHERE json_extract(payload,'$.run_id') IN
        (SELECT id FROM reactive_runs WHERE json_extract(payload,'$.thread_id')=?1)",
        [id],
    )?;
    db.execute(
        "DELETE FROM reactive_follow_ups WHERE json_extract(payload,'$.run_id') IN
        (SELECT id FROM reactive_runs WHERE json_extract(payload,'$.thread_id')=?1)",
        [id],
    )?;
    db.execute("INSERT INTO deleted_task_runs SELECT id FROM reactive_runs WHERE json_extract(payload,'$.thread_id')=?1",[id])?;
    db.execute(
        "DELETE FROM reactive_runs WHERE json_extract(payload,'$.thread_id')=?1",
        [id],
    )?;
    db.execute("DELETE FROM task_conversations WHERE thread_id=?1", [id])?;
    Ok(())
}
