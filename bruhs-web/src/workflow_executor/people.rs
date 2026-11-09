use bruhs_models::{PersonProfile, WorkflowOperation};

use super::*;
use crate::relationship_store::RelationshipTransaction;

impl WorkflowExecutor {
    pub(super) fn prepare_person_step(
        self,
        job_id: &str,
    ) -> Result<Option<RelationshipTransaction>, SaveError> {
        self.automation.with(|state| {
            let Some(operation @ WorkflowOperation::CreatePersonReminder { person_id, .. }) = state.pending_operation(job_id) else {
                return Ok(None);
            };
            if !self.people_hydrated.get_untracked() {
                return Err(SaveError::Rejected("People have not loaded. Reload to retry before creating a workflow reminder. No records were changed."));
            }
            if self.people_cache_error.get_untracked().is_some() || self.contact_merging.get_untracked() {
                return Err(SaveError::Rejected("Contact cache needs attention. Open People and retry loading contacts before running this reminder. No records were changed."));
            }
            self.people.0.with_untracked(|people| check_person(operation, people))?;
            self.relationships.begin_transaction(std::slice::from_ref(person_id)).map(Some)
        })
    }
}

fn check_person(operation: &WorkflowOperation, people: &[PersonProfile]) -> Result<(), SaveError> {
    let WorkflowOperation::CreatePersonReminder {
        person_id, person, ..
    } = operation
    else {
        return Err(SaveError::Rejected("Unexpected People operation."));
    };
    let mut matching = people.iter().filter(|candidate| candidate.id == *person_id);
    if !operation.is_valid()
        || matching
            .next()
            .is_none_or(|candidate| candidate.display_name != *person)
        || matching.next().is_some()
    {
        return Err(SaveError::Rejected("The reviewed person is missing, renamed, or ambiguous. Review the current ID and name before running this reminder. No records were changed."));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn operation(id: &str, name: &str) -> WorkflowOperation {
        WorkflowOperation::CreatePersonReminder {
            person_id: id.into(),
            person: name.into(),
            reminder: "Call back".into(),
        }
    }
    #[test]
    fn exact_id_disambiguates_people_with_the_same_name() {
        let first = PersonProfile::new("Alex");
        let second = PersonProfile::new("Alex");
        assert!(check_person(&operation(&second.id, "Alex"), &[first, second]).is_ok());
    }
    #[test]
    fn renamed_and_missing_people_cannot_be_retargeted_by_name() {
        let person = PersonProfile::new("New name");
        assert!(check_person(
            &operation(&person.id, "Old name"),
            std::slice::from_ref(&person)
        )
        .is_err());
        assert!(check_person(&operation("missing", "New name"), &[person]).is_err());
    }
    #[test]
    fn duplicate_identity_and_legacy_name_only_proposals_fail_closed() {
        let person = PersonProfile::new("Alex");
        assert!(check_person(&operation("", "Alex"), std::slice::from_ref(&person)).is_err());
        assert!(check_person(&operation(&person.id, "Alex"), &[person.clone(), person]).is_err());
    }
}
