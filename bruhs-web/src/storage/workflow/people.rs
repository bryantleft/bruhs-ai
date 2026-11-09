use bruhs_models::{
    PersonReminderReceipt, PersonReminderRollback, RelationshipProfile, WorkflowStepStatus,
};
use serde::Deserialize;

use super::*;
use crate::relationship_store::ExpectedRelationships;
use crate::storage::{relationships, STORE_PEOPLE, STORE_RELATIONSHIPS};

#[derive(Deserialize)]
struct ContactIdentity {
    id: String,
    display_name: String,
}

pub fn rollback_people_ids(state: &AutomationState, workflow_id: &str) -> Vec<String> {
    state
        .workflows
        .iter()
        .find(|workflow| workflow.id == workflow_id)
        .into_iter()
        .flat_map(|workflow| &workflow.steps)
        .filter(|step| step.status == WorkflowStepStatus::Completed)
        .filter_map(|step| match &step.operation {
            WorkflowOperation::CreatePersonReminder { person_id, .. } => Some(person_id.clone()),
            _ => None,
        })
        .collect()
}

async fn checked_profile(
    tx: &idb::Transaction,
    expected: &ExpectedRelationships,
    person_id: &str,
) -> Result<Option<RelationshipProfile>, SaveError> {
    crate::storage::people::merge::ensure_editable(tx, person_id).await?;
    let expected = expected.get(person_id).ok_or(SaveError::Unavailable)?;
    let current = relationships::read_in(tx, person_id).await?;
    if &current != expected {
        return Err(SaveError::Conflict);
    }
    Ok(current)
}

pub(super) async fn execute(
    expected: &Metadata<AutomationState>,
    expected_relationships: Option<&ExpectedRelationships>,
    operation: &WorkflowOperation,
    job_id: &str,
    now: i64,
) -> Result<WorkflowCommit<CreatedRecord>, SaveError> {
    let WorkflowOperation::CreatePersonReminder {
        person_id, person, ..
    } = operation
    else {
        return Err(SaveError::Rejected("Unexpected People operation."));
    };
    let expected_relationships = expected_relationships.ok_or(SaveError::Unavailable)?;
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(
        &[STORE_METADATA, STORE_PEOPLE, STORE_RELATIONSHIPS],
        TransactionMode::ReadWrite,
    )?);
    let result = async {
        let mut current = checked_state(&tx, expected).await?;
        let (identity, before) = futures_util::join!(
            async {
                let raw = tx.object_store(STORE_PEOPLE)?.get(JsValue::from_str(person_id))?.await?
                    .ok_or(SaveError::Rejected("The reviewed contact is not saved locally. Retry contact storage before running this reminder. No records were changed."))?;
                from_js::<ContactIdentity>(raw).map_err(|_| SaveError::Corrupt)
            },
            checked_profile(&tx, expected_relationships, person_id)
        );
        let identity = identity?;
        let before = before?;
        if identity.id != *person_id || identity.display_name != *person {
            return Err(SaveError::Rejected("The saved contact no longer matches the reviewed ID and name. Review the current person before running this reminder. No records were changed."));
        }
        let receipt = PersonReminderReceipt::prepare(operation, before.as_ref(), now)
            .map_err(|error| SaveError::Rejected(error.message()))?;
        let snapshot = serde_json::to_string(&receipt).map_err(|_| SaveError::Corrupt)?;
        if !current.value.complete_step_with_snapshot(job_id, receipt.target_id(), snapshot, now) {
            return Err(SaveError::Rejected("The job no longer permits this reminder. No records were changed."));
        }
        let value = to_js(receipt.after())?;
        let (written, saved) = futures_util::join!(
            async { tx.object_store(STORE_RELATIONSHIPS)?.put(&value, None)?.await?; Ok::<_, SaveError>(()) },
            save_checkpoints(&tx, &current, None)
        );
        written?; saved?;
        Ok(WorkflowCommit { automation: current, life: None, result: CreatedRecord::Relationship(Box::new(receipt.into_after())) })
    }.await;
    finish(tx, result).await
}

pub(super) async fn rollback_plan(
    tx: &idb::Transaction,
    state: &AutomationState,
    workflow_id: &str,
    expected: Option<&ExpectedRelationships>,
) -> Result<PersonReminderRollback, SaveError> {
    let ids = rollback_people_ids(state, workflow_id);
    if ids.is_empty() {
        return Ok(PersonReminderRollback::default());
    }
    let expected = expected.ok_or(SaveError::Unavailable)?;
    // Only targeted people are copied. The writer lock covers the whole plan.
    let profiles = join_all(expected.keys().map(|id| checked_profile(tx, expected, id)))
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let workflow = state
        .workflows
        .iter()
        .find(|workflow| workflow.id == workflow_id)
        .ok_or(SaveError::Rejected("Workflow no longer exists."))?;
    PersonReminderRollback::prepare(&workflow.steps, &profiles)
        .map_err(|error| SaveError::Rejected(error.message()))
}
