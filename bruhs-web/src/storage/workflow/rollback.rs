use super::*;
use crate::relationship_store::ExpectedRelationships;
use crate::storage::STORE_RELATIONSHIPS;
use bruhs_models::PersonReminderRollback;

pub struct RolledBackRecords {
    pub targets: Vec<(AgentDomain, String)>,
    pub people: PersonReminderRollback,
}

pub async fn rollback(
    expected: &Metadata<AutomationState>,
    expected_life: Option<&Metadata<LifeContext>>,
    expected_relationships: Option<&ExpectedRelationships>,
    workflow_id: &str,
    now: i64,
) -> Result<WorkflowCommit<RolledBackRecords>, SaveError> {
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(
        &[
            STORE_METADATA,
            STORE_THREADS,
            STORE_MESSAGES,
            STORE_WORK_ITEMS,
            STORE_EVENTS,
            STORE_CAPTURES,
            STORE_KNOWLEDGE_DOCUMENTS,
            STORE_INBOX_CONVERSATIONS,
            STORE_MEETING_NOTES,
            STORE_RELATIONSHIPS,
        ],
        TransactionMode::ReadWrite,
    )?);
    let result = async {
        let mut current = checked_state(&tx, expected).await?;
        let mut life = life::checked(
            &tx,
            expected_life,
            rollback_needs_life(&current.value, workflow_id),
        )
        .await?;
        let workflow = current
            .value
            .workflows
            .iter()
            .find(|workflow| workflow.id == workflow_id)
            .ok_or(SaveError::Rejected("Workflow no longer exists."))?;
        if workflow.steps.len() > 128 {
            return Err(SaveError::Rejected(
                "Invalid workflow size. No records were changed.",
            ));
        }
        let people = people::rollback_plan(&tx, &current.value, workflow_id, expected_relationships).await?;
        // All lookups are issued together, under the same read/write lock as deletions.
        let records = join_all(
            workflow
                .steps
                .iter()
                .filter(|step| step.status == bruhs_models::WorkflowStepStatus::Completed)
                .map(|step| async {
                    let domain = step.operation.domain();
                    let id = step.target_id.as_deref().ok_or(SaveError::Rejected(
                        "Missing record receipt. No records were changed.",
                    ))?;
                    // Messages live separately from thread metadata. Never
                    // erase a conversation added after workflow creation.
                    if domain == AgentDomain::Chat {
                        if let Some(raw) = tx.object_store(STORE_MESSAGES)?.get(JsValue::from_str(id))?.await? {
                            let messages: crate::storage::MessagesRecord = from_js(raw)?;
                            if messages.thread_id != id || !messages.messages.is_empty() {
                                return Err(SaveError::Rejected("A created chat now contains messages. No records were changed."));
                            }
                        }
                    }
                    let saved = if domain == AgentDomain::People {
                        people.snapshot(id)
                    } else if is_life_domain(domain) {
                        life.as_ref()
                            .and_then(|life| life::snapshot(&life.value, domain, id))
                    } else {
                        let raw = tx
                            .object_store(domain_store(domain)?)?
                            .get(JsValue::from_str(id))?
                            .await?;
                        raw.map(|raw| snapshot(domain, raw)).transpose()?
                    };
                    Ok::<_, SaveError>(((domain, id.to_string()), saved))
                }),
        )
        .await
        .into_iter()
        .collect::<Result<HashMap<_, _>, SaveError>>()?;
        let targets = current
            .value
            .rollback_targets(workflow_id, |domain, id| {
                records.get(&(domain, id.to_string())).cloned().flatten()
            })
            .map_err(|error| SaveError::Rejected(error.message()))?;
        if !current.value.mark_rolled_back(workflow_id, now) {
            return Err(SaveError::Rejected("Workflow cannot be rolled back."));
        }
        if let Some(life) = &mut life {
            for (domain, id) in &targets {
                match domain {
                    AgentDomain::Finance => life
                        .value
                        .finance
                        .transactions
                        .retain(|record| record.id != *id),
                    AgentDomain::Health => {
                        life.value.health.workouts.retain(|record| record.id != *id)
                    }
                    _ => {}
                }
            }
        }
        let mut requests = targets
            .iter()
            .filter(|(domain, _)| !is_life_domain(*domain) && *domain != AgentDomain::People)
            .map(|(domain, id)| {
                Ok(tx
                    .object_store(domain_store(*domain)?)?
                    .delete(JsValue::from_str(id))?)
            })
            .collect::<Result<Vec<_>, SaveError>>()?;
        for (domain, id) in &targets {
            if *domain == AgentDomain::Chat {
                requests.push(tx.object_store(STORE_MESSAGES)?.delete(JsValue::from_str(id))?);
            }
        }
        let tx_ref = &tx;
        let (deleted, restored, saved) = futures_util::join!(
            join_all(requests.into_iter().map(|request| async { request.await })),
            join_all(people.restorations().map(|(id, profile)| async move {
                let store = tx_ref.object_store(STORE_RELATIONSHIPS)?;
                if let Some(profile) = profile {
                    store.put(&to_js(profile)?, None)?.await?;
                } else {
                    store.delete(JsValue::from_str(id))?.await?;
                }
                Ok::<_, SaveError>(())
            })),
            save_checkpoints(&tx, &current, life.as_ref())
        );
        for result in deleted {
            result?;
        }
        for result in restored { result?; }
        saved?;
        Ok(WorkflowCommit {
            automation: current,
            life,
            result: RolledBackRecords { targets, people },
        })
    }
    .await;
    finish(tx, result).await
}
