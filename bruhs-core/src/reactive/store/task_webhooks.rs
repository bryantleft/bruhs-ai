use super::*;

/// The owner may reveal an existing address or rotate the exact address being displayed.
pub(in crate::reactive) fn address(
    db: &Connection,
    task: &str,
    previous: Option<&str>,
) -> Result<String, ReactiveError> {
    let sub: Subscription = required(db, "reactive_subscriptions", task, "task")?;
    let source: EventSource = required(db, "reactive_sources", &sub.source_id, "source")?;
    if source.kind != SourceKind::Webhook {
        return Err(ReactiveError::Invalid(
            "Choose a webhook trigger first.".into(),
        ));
    }
    let fresh = uuid::Uuid::new_v4().simple().to_string();
    if let Some(previous) = previous {
        if db.execute(
            "UPDATE task_webhook_addresses SET token=?1 WHERE task=?2 AND token=?3",
            params![fresh, task, previous],
        )? == 0
        {
            return Err(ReactiveError::Conflict(
                "The webhook URL has changed. Reload it before rotating.".into(),
            ));
        }
        return Ok(fresh);
    }
    db.execute("INSERT INTO task_webhook_addresses(task,token,scope) VALUES(?1,?2,?3) ON CONFLICT(task) DO NOTHING",
        params![task, fresh, uuid::Uuid::new_v4().to_string()])?;
    Ok(db.query_row(
        "SELECT token FROM task_webhook_addresses WHERE task=?1",
        [task],
        |r| r.get(0),
    )?)
}

/// Delivery can only use a credential that was already issued by the owner.
pub(in crate::reactive) fn lookup(
    db: &Connection,
    task: &str,
) -> Result<(Subscription, String, String), ReactiveError> {
    db.query_row(
        "SELECT s.payload,a.token,a.scope FROM reactive_subscriptions s
        JOIN task_webhook_addresses a ON a.task=s.id WHERE s.id=?1",
        [task],
        |row| Ok((row.get::<_, String>(0)?, row.get(1)?, row.get(2)?)),
    )
    .optional()?
    .map(|(payload, token, scope)| Ok((decode(payload)?, token, scope)))
    .unwrap_or(Err(ReactiveError::Unauthorized))
}
