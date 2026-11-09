use super::*;
use bruhs_models::PersonReminderRollback;

pub(super) struct AutomationRollback {
    pub(super) targets: Vec<(AgentDomain, String)>,
    pub(super) people: PersonReminderRollback,
}

impl Workspace {
    pub(in crate::views::workspace) fn automation_rollback_targets(
        &self,
        id: &str,
        cx: &App,
    ) -> Result<Vec<(AgentDomain, String)>, &'static str> {
        self.automation_rollback_plan(id, cx)
            .map(|plan| plan.targets)
    }

    pub(super) fn automation_rollback_plan(
        &self,
        id: &str,
        cx: &App,
    ) -> Result<AutomationRollback, &'static str> {
        if self.automation_pending {
            return Err("Wait for the scheduled workflow step before reviewing undo. No records were changed.");
        }
        if self.automation_state.jobs.iter().any(|job| {
            job.workflow_id == id && job.auto_run && job.status == BackgroundJobStatus::Running
        }) {
            return Err("Pause the background workflow in the app before reviewing undo. No records were changed.");
        }
        let workflow = self
            .automation_state
            .workflows
            .iter()
            .find(|workflow| workflow.id == id)
            .ok_or("Workflow no longer exists.")?;
        let people = PersonReminderRollback::prepare(&workflow.steps, &self.relationships)
            .map_err(|error| error.message())?;
        let targets =
            self.automation_state
                .rollback_targets(id, |domain, id| match domain {
                    AgentDomain::People => people.snapshot(id),
                    AgentDomain::Chat => self
                        .threads
                        .iter()
                        .find(|thread| thread.id == id)
                        .and_then(|thread| {
                            if self.active_thread_id.as_deref() == Some(id)
                                || thread.chat.read(cx).has_content_or_pending_work(cx)
                            {
                                return None;
                            }
                            serde_json::to_string(&thread.snapshot(cx)).ok()
                        }),
                    AgentDomain::Tasks => self
                        .work_items
                        .iter()
                        .find(|item| item.id == id)
                        .and_then(|item| serde_json::to_string(item).ok()),
                    AgentDomain::Calendar => self
                        .calendar_events
                        .iter()
                        .find(|item| item.id == id)
                        .and_then(|item| serde_json::to_string(item).ok()),
                    AgentDomain::Capture => self
                        .captures
                        .iter()
                        .find(|item| item.id == id)
                        .and_then(|item| serde_json::to_string(item).ok()),
                    AgentDomain::Knowledge => self
                        .knowledge_documents
                        .iter()
                        .find(|item| item.id == id)
                        .and_then(|item| serde_json::to_string(item).ok()),
                    AgentDomain::Inbox => self
                        .inbox_conversations
                        .iter()
                        .find(|item| item.id == id)
                        .and_then(|item| serde_json::to_string(item).ok()),
                    AgentDomain::Finance => self
                        .life_context
                        .finance
                        .transactions
                        .iter()
                        .find(|item| item.id == id)
                        .and_then(|item| serde_json::to_string(item).ok()),
                    AgentDomain::Health => self
                        .life_context
                        .health
                        .workouts
                        .iter()
                        .find(|item| item.id == id)
                        .and_then(|item| serde_json::to_string(item).ok()),
                    AgentDomain::Meetings => {
                        let mut matches = self.meeting_notes.iter().filter(|note| note.id == id);
                        let note = matches.next()?;
                        if matches.next().is_some() {
                            return None;
                        }
                        serde_json::to_string(note).ok()
                    }
                })
                .map_err(|error| error.message())?;
        Ok(AutomationRollback { targets, people })
    }
}
