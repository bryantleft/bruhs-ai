use super::*;
use bruhs_models::PersonReminderReceipt;

impl Workspace {
    pub(super) fn run_person_reminder_step(
        &mut self,
        job_id: &str,
        operation: &WorkflowOperation,
        now: i64,
        cx: &mut Context<'_, Self>,
    ) {
        let result = self
            .prepare_person_reminder(operation, now, cx)
            .and_then(|receipt| {
                let snapshot = serde_json::to_string(&receipt).map_err(|_| {
                    "The reminder receipt could not be encoded. No records were changed."
                })?;
                if !self.automation_state.complete_step_with_snapshot(
                    job_id,
                    receipt.target_id(),
                    snapshot,
                    now,
                ) {
                    return Err("The reviewed workflow changed. No records were changed.");
                }
                let after = receipt.into_after();
                if let Some(existing) = self
                    .relationships
                    .iter_mut()
                    .find(|profile| profile.person_id == after.person_id)
                {
                    *existing = after;
                } else {
                    self.relationships.push(after);
                }
                Ok(())
            });
        match result {
            Ok(()) => self.persist_and_notify(cx),
            Err(message) => {
                self.automation_error = Some(message.into());
                cx.notify();
            }
        }
    }

    fn prepare_person_reminder(
        &self,
        operation: &WorkflowOperation,
        now: i64,
        cx: &App,
    ) -> Result<PersonReminderReceipt, &'static str> {
        let WorkflowOperation::CreatePersonReminder {
            person_id, person, ..
        } = operation
        else {
            return Err("This is not a person reminder.");
        };
        let mut people = self
            .meetings_hub
            .read(cx)
            .people
            .iter()
            .filter(|profile| profile.id == *person_id);
        if people
            .next()
            .is_none_or(|profile| profile.display_name != *person)
            || people.next().is_some()
        {
            return Err("The reviewed person is missing, renamed, or ambiguous. Review a new reminder with the current person ID and name. No records were changed.");
        }
        let mut profiles = self
            .relationships
            .iter()
            .filter(|profile| profile.person_id == *person_id);
        let before = profiles.next();
        if profiles.next().is_some() {
            return Err("Duplicate relationship profiles prevent safe reminder creation. No records were changed.");
        }
        PersonReminderReceipt::prepare(operation, before, now).map_err(|error| error.message())
    }
}
