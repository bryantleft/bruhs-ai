use super::*;
use bruhs_models::WorkflowStepStatus;

pub fn is_life_domain(domain: AgentDomain) -> bool {
    matches!(domain, AgentDomain::Finance | AgentDomain::Health)
}

pub fn rollback_needs_life(state: &AutomationState, workflow_id: &str) -> bool {
    state
        .workflows
        .iter()
        .find(|workflow| workflow.id == workflow_id)
        .is_some_and(|workflow| {
            workflow.steps.iter().any(|step| {
                step.status == WorkflowStepStatus::Completed
                    && is_life_domain(step.operation.domain())
            })
        })
}

pub(super) async fn checked(
    tx: &idb::Transaction,
    expected: Option<&Metadata<LifeContext>>,
    needed: bool,
) -> Result<Option<Metadata<LifeContext>>, SaveError> {
    if !needed {
        return Ok(None);
    }
    let expected = expected.ok_or(SaveError::Unavailable)?;
    checked_state(tx, expected).await.map(Some)
}

pub(super) fn snapshot(life: &LifeContext, domain: AgentDomain, id: &str) -> Option<String> {
    match domain {
        AgentDomain::Finance => life
            .finance
            .transactions
            .iter()
            .find(|record| record.id == id)
            .and_then(|record| serde_json::to_string(record).ok()),
        AgentDomain::Health => life
            .health
            .workouts
            .iter()
            .find(|record| record.id == id)
            .and_then(|record| serde_json::to_string(record).ok()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollback_freezes_life_only_for_completed_life_steps() {
        let mut state = AutomationState::default();
        let id = state.create_workflow(
            "Mixed",
            vec![
                WorkflowOperation::CreateTask {
                    title: "Task".into(),
                },
                WorkflowOperation::LogWorkout {
                    activity: "Walk".into(),
                    minutes: 37,
                },
            ],
            1,
        );
        assert!(!rollback_needs_life(&state, &id));
        state.workflows[0].steps[0].status = WorkflowStepStatus::Completed;
        assert!(!rollback_needs_life(&state, &id));
        state.workflows[0].steps[1].status = WorkflowStepStatus::Completed;
        assert!(rollback_needs_life(&state, &id));
        assert!(!rollback_needs_life(&state, "other"));
    }

    #[test]
    fn life_snapshots_are_exact_and_domain_scoped() {
        let record = FinanceTransaction::manual("Receipt", "Other", i64::MAX, 1);
        let id = record.id.clone();
        let expected = serde_json::to_string(&record).unwrap();
        let mut context = LifeContext::default();
        context.finance.transactions.push(record);
        assert_eq!(
            snapshot(&context, AgentDomain::Finance, &id),
            Some(expected)
        );
        assert_eq!(snapshot(&context, AgentDomain::Health, &id), None);
        context.finance.transactions[0].note = Some("Edit".into());
        assert!(snapshot(&context, AgentDomain::Finance, &id)
            .unwrap()
            .contains("Edit"));
    }
}
