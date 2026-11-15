//! The owner's decision on a workflow made from an agent's draft flows back to
//! that agent, and new agent updates surface while the app is open.
use super::*;
use crate::views::meetings_hub::NoticeAction;
use crate::views::reactive_automations::ReactiveArrival;
use bruhs_models::reactive::{DraftOutcomeKind, ReactiveCommand};

/// One report to send: the workflow, the status it reports, and the command.
pub(super) fn draft_outcome_commands(
    state: &AutomationState,
) -> Vec<(String, WorkflowStatus, ReactiveCommand)> {
    state
        .draft_outcome_reports()
        .into_iter()
        .filter_map(|report| {
            let outcome = DraftOutcomeKind::from_workflow(report.status)?;
            Some((
                report.workflow_id.clone(),
                report.status,
                ReactiveCommand::DraftOutcome {
                    run_id: report.run_id,
                    workflow_id: report.workflow_id,
                    outcome,
                    applied_steps: report.applied_steps,
                    total_steps: report.total_steps,
                },
            ))
        })
        .collect()
}

/// Record what the server acknowledged; true when the state needs saving.
pub(super) fn apply_draft_outcome_acks(
    state: &mut AutomationState,
    acks: &[(String, WorkflowStatus)],
) -> bool {
    acks.iter().fold(false, |changed, (workflow_id, status)| {
        state.mark_draft_outcome_reported(workflow_id, *status) | changed
    })
}

impl Workspace {
    /// Runs after load and after every save. Failed sends retry on the next change.
    pub(super) fn reconcile_draft_outcomes(&mut self, cx: &mut Context<'_, Self>) {
        // gpui's test scheduler rejects wake-ups from the tokio threads.
        if cfg!(test) {
            return;
        }
        let commands = draft_outcome_commands(&self.automation_state);
        if commands.is_empty() {
            return;
        }
        let panel = self.reactive_automations.clone();
        if !panel.update(cx, |panel, _| panel.begin_draft_reporting()) {
            return;
        }
        let client = panel.read(cx).client();
        let operation = crate::app_state::spawn_tokio(cx, async move {
            let mut acks = Vec::new();
            let mut snapshot = None;
            for (workflow_id, status, command) in commands {
                match client.reactive_command(&command).await {
                    Ok(next) => {
                        acks.push((workflow_id, status));
                        snapshot = Some(next);
                    }
                    Err(error) => tracing::warn!("draft decision not reported yet: {error}"),
                }
            }
            (acks, snapshot)
        });
        cx.spawn(async move |this, cx| {
            let (acks, snapshot) = operation.await;
            let _ = this.update(cx, |this, cx| {
                panel.update(cx, |panel, cx| panel.finish_draft_reporting(snapshot, cx));
                if apply_draft_outcome_acks(&mut this.automation_state, &acks) {
                    this.persist_and_notify(cx);
                }
            });
        })
        .detach();
    }

    /// Show a notice for each new agent update; subscribed once per workspace.
    pub(super) fn start_reactive_awareness(&mut self, cx: &mut Context<'_, Self>) {
        let panel = self.reactive_automations.clone();
        if panel.read(cx).has_awareness() {
            return;
        }
        let subscription = cx.subscribe(&panel, |this, _, event: &ReactiveArrival, cx| {
            let message = event.message.clone();
            this.meetings_hub.update(cx, |hub, cx| {
                hub.show_notice_with_action(message, NoticeAction::OpenAutomations, cx)
            });
        });
        panel.update(cx, |panel, _| panel.set_awareness(subscription));
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_draft_outcome_acks, draft_outcome_commands, AutomationState};
    use bruhs_models::{
        reactive::{DraftOutcomeKind, ReactiveCommand},
        WorkflowOperation, WorkflowStatus,
    };

    #[test]
    fn settled_draft_workflows_report_once_per_status_and_plain_workflows_never_do() {
        let mut state = AutomationState::default();
        let operations = vec![WorkflowOperation::CreateTask {
            title: "Review the quote".into(),
        }];
        let draft = state.create_reactive_workflow("Draft", operations.clone(), "run-7", 1);
        let plain = state.create_workflow("Plain", operations, 1);
        assert!(
            draft_outcome_commands(&state).is_empty(),
            "nothing settled yet"
        );
        for workflow in &mut state.workflows {
            workflow.status = WorkflowStatus::Declined;
        }
        let commands = draft_outcome_commands(&state);
        assert_eq!(commands.len(), 1, "only the draft-made workflow reports");
        let (workflow_id, status, command) = &commands[0];
        assert_eq!(workflow_id, &draft);
        assert_ne!(workflow_id, &plain);
        assert_eq!(*status, WorkflowStatus::Declined);
        assert_eq!(
            command,
            &ReactiveCommand::DraftOutcome {
                run_id: "run-7".into(),
                workflow_id: draft.clone(),
                outcome: DraftOutcomeKind::Declined,
                applied_steps: 0,
                total_steps: 1,
            }
        );
        assert!(apply_draft_outcome_acks(
            &mut state,
            &[(draft.clone(), WorkflowStatus::Declined)]
        ));
        assert!(draft_outcome_commands(&state).is_empty(), "acknowledged");
        assert!(!apply_draft_outcome_acks(
            &mut state,
            &[(draft, WorkflowStatus::Declined)]
        ));
    }
}
