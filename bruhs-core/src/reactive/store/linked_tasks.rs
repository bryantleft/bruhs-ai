//! Scheduled-task edits from a paired project environment.
use super::*;
use sha2::{Digest, Sha256};

fn load(db: &Connection, id: &str) -> Result<AutomationSetup, ReactiveError> {
    let subscription: Subscription = required(db, "reactive_subscriptions", id, "scheduled task")?;
    let source: EventSource = required(db, "reactive_sources", &subscription.source_id, "trigger")?;
    if !source.kind.is_scheduled_task() {
        return Err(ReactiveError::NotFound(
            "Scheduled task unavailable.".into(),
        ));
    }
    let agent = required(db, "reactive_agents", &subscription.agent_id, "agent")?;
    Ok(AutomationSetup {
        agent,
        source,
        subscription,
    })
}

pub(in crate::reactive) fn definition(
    db: &Connection,
    id: &str,
) -> Result<LinkedTaskDefinition, ReactiveError> {
    let mut setup = load(db, id)?;
    reports::guard_managed_identity(db, id)?;
    setup.agent.memory.clear();
    let binding = task_conversations::binding(db, id)?;
    let conversation_chosen = binding.as_ref().is_some_and(|(_, chosen)| *chosen);
    let agent_locked = conversation_chosen || db.query_row(
        "SELECT EXISTS(SELECT 1 FROM reactive_subscriptions WHERE id != ?1 AND json_extract(payload, '$.agent_id')=?2)",
        params![id, setup.agent.id], |row| row.get::<_, bool>(0),
    )?;
    if conversation_chosen {
        setup.agent.instructions.clear();
    }
    let revision_data = match &binding {
        Some((choice, chosen)) => encode(&(&setup, agent_locked, &choice.thread_id, chosen))?,
        None => encode(&(&setup, agent_locked))?,
    };
    let revision = format!("{:x}", Sha256::digest(revision_data.as_bytes()));
    Ok(LinkedTaskDefinition {
        setup,
        revision,
        agent_locked,
        conversation: binding.map(|(choice, _)| choice),
        conversation_chosen,
    })
}

pub(in crate::reactive) fn save_task(
    db: &mut Connection,
    id: &str,
    edit: LinkedTaskSave,
    paths: &[String],
) -> Result<ScheduledTaskSummary, ReactiveError> {
    transaction(db, |db| {
        let LinkedTaskSave {
            request_id,
            expected,
            mut setup,
            enabled,
            signing_secret,
            destination,
        } = edit;
        if setup.subscription.id != id
            || !setup.agent.memory.is_empty()
            || !setup.source.kind.is_scheduled_task()
        {
            return Err(ReactiveError::Invalid(
                "Choose an editable scheduled task.".into(),
            ));
        }
        reports::guard_managed_identity(db, id)?;
        let exists = get::<Subscription>(db, "reactive_subscriptions", id)?.is_some();
        // Retain the last receipt after deletion. A lost create acknowledgement must
        // not recreate a task that its owner has since deleted.
        if !exists
            && db.query_row(
                "SELECT EXISTS(SELECT 1 FROM linked_task_saves WHERE task=?1)",
                [id],
                |row| row.get::<_, bool>(0),
            )?
        {
            return Err(ReactiveError::Conflict(
                "This scheduled task has been removed on its host.".into(),
            ));
        }
        let current = exists.then(|| definition(db, id)).transpose()?;
        let summary = |db: &Connection| {
            schedule_summaries(db, Some(id))?
                .pop()
                .ok_or_else(|| ReactiveError::NotFound("Scheduled task unavailable.".into()))
        };
        if let Some(current) = &current {
            let replay: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM linked_task_saves WHERE task=?1 AND request=?2 AND revision=?3)", params![id, request_id.to_string(), current.revision], |r|r.get(0))?;
            if replay {
                return summary(db);
            }
        }
        if expected.as_deref() != current.as_ref().map(|c| c.revision.as_str()) {
            return Err(ReactiveError::Conflict(
                "This task changed on the host. Reopen it to review the latest settings.".into(),
            ));
        }
        let known = |p: &str| paths.iter().any(|s| s == p);
        if !known(&setup.agent.project_path)
            && !current
                .as_ref()
                .is_some_and(|c| c.setup.agent.project_path == setup.agent.project_path)
        {
            return Err(ReactiveError::Invalid(
                "Choose a project shared by this host.".into(),
            ));
        }
        let destination_agent =
            match &destination {
                Some(TaskDestinationEdit::Existing { thread_id }) => Some(
                    task_conversations::linked_agent(db, &setup.agent.project_path, thread_id)?,
                ),
                Some(TaskDestinationEdit::Desktop { thread_id }) => Some(
                    desktop_tasks::linked_agent(db, &setup.agent.project_path, thread_id)?,
                ),
                _ => None,
            };
        if let TaskCheckout::Existing { path } = &setup.subscription.task_checkout {
            if destination_agent.is_none()
                && !known(path)
                && !current.as_ref().is_some_and(|c| {
                    c.setup.subscription.task_checkout == setup.subscription.task_checkout
                })
            {
                return Err(ReactiveError::Invalid(
                    "Choose an existing checkout shared by this host.".into(),
                ));
            }
        }
        // Component IDs and activation of shared records belong to the host.
        if let Some(current) = current {
            let old = load(db, id)?;
            if setup.agent.id != old.agent.id || setup.source.id != old.source.id {
                return Err(ReactiveError::Invalid(
                    "This task’s agent or trigger changed.".into(),
                ));
            }
            setup.agent.memory = old.agent.memory.clone();
            if current.conversation_chosen && setup.agent.instructions.is_empty() {
                setup.agent.instructions.clone_from(&old.agent.instructions);
            }
            setup.agent.enabled = old.agent.enabled;
            setup.source.enabled = old.source.enabled;
            if current.agent_locked && setup.agent != old.agent {
                return Err(ReactiveError::Conflict(
                    "The task shares its agent with a conversation or another automation.".into(),
                ));
            }
            // Detaching starts a task-owned agent. Later edits to this task must never
            // rewrite the old conversation's prompt, model or memory.
            if current.conversation_chosen
                && (!setup.subscription.task_conversation.has_destination()
                    || matches!(destination, Some(TaskDestinationEdit::FirstRun { .. })))
            {
                setup.agent.id = uuid::Uuid::new_v4().to_string();
                setup.agent.name = setup.title().into();
                setup
                    .agent
                    .instructions
                    .clone_from(&setup.subscription.instructions);
                setup.agent.memory.clear();
                setup.agent.enabled = true;
            }
            if setup.source != old.source && db.query_row("SELECT EXISTS(SELECT 1 FROM reactive_subscriptions WHERE id != ?1 AND json_extract(payload, '$.source_id')=?2)", params![id, old.source.id], |r|r.get::<_,bool>(0))? {
                setup.source.id = uuid::Uuid::new_v4().to_string();
            }
        } else {
            uuid::Uuid::parse_str(id)
                .map_err(|_| ReactiveError::Invalid("Choose a new task identifier.".into()))?;
            setup.agent.id = uuid::Uuid::new_v4().to_string();
            setup.source.id = uuid::Uuid::new_v4().to_string();
            setup.agent.enabled = true;
            // Pausing a new task pauses its subscription. Its private trigger and
            // agent remain available for an explicit Run now, as in the local editor.
            setup.source.enabled = true;
        }
        if let Some(agent) = destination_agent {
            setup.agent = agent;
        }
        setup.subscription.source_id.clone_from(&setup.source.id);
        setup.subscription.agent_id.clone_from(&setup.agent.id);
        if let Some(secret) = signing_secret {
            webhook_security::replace_key(db, &setup.subscription, secret)?;
        }
        save_scheduled_task(db, Box::new(setup), enabled, destination.as_ref())?;
        let saved = definition(db, id)?;
        db.execute(
            "INSERT OR REPLACE INTO linked_task_saves(task,request,revision) VALUES(?1,?2,?3)",
            params![id, request_id.to_string(), saved.revision],
        )?;
        summary(db)
    })
}
