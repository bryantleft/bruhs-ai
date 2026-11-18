//! Workflow review on Agent workflows, shared by web
//! (`components/mcp_review.rs`, `components/automations_page.rs`,
//! `components/workflow_status.rs`) and desktop (`automations_ui/review_ui.rs`):
//! which previewed workflows wait for review and in what order, which Write
//! grants each still needs, the app-wide workflow notice, and the words both
//! pages use.
//!
//! An MCP client's undo request and every workflow still waiting in preview
//! (an MCP proposal, or one left from an earlier session) get a card of their
//! own above Local suggestions. The page's own dry run is reviewed in Preview
//! changes. A started workflow goes on show in the workflow history
//! ([`super::workflow_history`]), which carries its controls.

use crate::{AgentAccess, AgentDomain, AutomationState, BackgroundJobStatus, WorkflowStatus};

/// An awaiting card's accessible name.
pub const AWAITING_REVIEW_LABEL: &str = "Workflow awaiting review";
/// Under an awaiting card's title, when the connected MCP client proposed it.
pub const PROPOSED_BY_CLIENT: &str = "Proposed by the connected MCP client. Nothing has run yet.";
/// Under an awaiting card's title otherwise (left from an earlier session).
pub const WAITING_IN_PREVIEW: &str = "Waiting in preview. Nothing has run yet.";
/// An undo request card's title.
pub const UNDO_REQUESTED: &str = "Undo requested";
/// The page's own dry run.
pub const PREVIEW_CHANGES: &str = "Preview changes";
/// Under [`PREVIEW_CHANGES`].
pub const NOTHING_RUN_YET: &str = "Nothing below has run yet.";
/// Previews a workflow again against the current grants.
pub const RECHECK_GRANTS: &str = "Recheck grants";
/// Starts a reviewed workflow.
pub const START_TRANSACTION: &str = "Start transaction";
/// Declines a waiting workflow.
pub const DECLINE_PROPOSAL: &str = "Decline proposal";
/// Declines an undo request.
pub const KEEP_CHANGES: &str = "Keep changes";
/// Above a workflow's operations.
pub const PAYLOAD_NOTE: &str = "Full payload stored locally. Review every field before starting.";
/// Shows a workflow's operations where they are folded away.
pub const REVIEW_WORKFLOW: &str = "Review workflow";
/// The jobs card.
pub const BACKGROUND_JOBS: &str = "Background jobs";
/// Under [`BACKGROUND_JOBS`].
pub const BACKGROUND_JOBS_NOTE: &str = "Each record and checkpoint commit together. Background runs continue across pages while the app is open and resume after reload. Run, pause, resume or undo a workflow from Workflow history.";
/// A finished job's mark.
pub const CHECKPOINTED: &str = "Checkpointed";
/// Jobs the jobs card lists, newest first.
pub const JOBS_SHOWN: usize = 8;
/// The audit card.
pub const AUDIT_TIMELINE: &str = "Audit timeline";
/// Audit events the audit card lists, newest first.
pub const AUDIT_SHOWN: usize = 20;
/// The grants card.
pub const CAPABILITY_GRANTS: &str = "Capability grants";
/// Under [`CAPABILITY_GRANTS`].
pub const CAPABILITY_GRANTS_NOTE: &str =
    "Read and Write are independent. Finance and Health start fully off.";
/// The safety card's label.
pub const SAFETY_CONTRACT: &str = "Safety contract";
/// The safety card's text.
pub const SAFETY_CONTRACT_NOTE: &str = "Writes never inherit Read access. Every workflow requires a dry run. Undo reverts completed steps, including a partial run, only if their records are unchanged.";
/// The history card's label.
pub const WORKFLOW_HISTORY: &str = "Workflow history";
/// The app-wide notice's button.
pub const OPEN_WORKFLOWS: &str = "Open workflows";
/// [`OPEN_WORKFLOWS`]'s neighbour on a notice that can be put away.
pub const DISMISS_WORKFLOW_STATUS: &str = "Dismiss workflow status";
/// The app-wide notice's region.
pub const WORKFLOW_STATUS_LABEL: &str = "Workflow status";
/// Pauses every running background workflow from the app-wide notice.
pub const PAUSE_BACKGROUND: &str = "Pause";
/// [`PAUSE_BACKGROUND`]'s accessible name.
pub const PAUSE_BACKGROUND_LABEL: &str = "Pause background workflows";
const NEEDS_ATTENTION: &str =
    "Workflow needs attention. Open workflows to review the error and recovery options.";

/// Previewed workflows newest first, apart from `shown` (the page's own dry
/// run, reviewed in Preview changes).
pub fn awaiting_review(state: &AutomationState, shown: Option<&str>) -> Vec<String> {
    state
        .workflows
        .iter()
        .rev()
        .filter(|workflow| {
            workflow.status == WorkflowStatus::Previewed && Some(workflow.id.as_str()) != shown
        })
        .map(|workflow| workflow.id.clone())
        .collect()
}

/// Domains of `workflow_id` whose Write grant is missing, once each, in step
/// order.
pub fn blocked_domains(state: &AutomationState, workflow_id: &str) -> Vec<AgentDomain> {
    let mut blocked = Vec::new();
    for step in state
        .workflows
        .iter()
        .filter(|workflow| workflow.id == workflow_id)
        .flat_map(|workflow| &workflow.steps)
    {
        let domain = step.operation.domain();
        if !state.permissions.allows(domain, AgentAccess::Write) && !blocked.contains(&domain) {
            blocked.push(domain);
        }
    }
    blocked
}

/// "Grant Write for: Tasks, Calendar." for a workflow that cannot start.
pub fn grant_write_for(blocked: &[AgentDomain]) -> Option<String> {
    (!blocked.is_empty()).then(|| {
        format!(
            "Grant Write for: {}.",
            blocked
                .iter()
                .map(|domain| domain.label())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })
}

/// The line under an awaiting card's title.
pub const fn origin(proposed_by_client: bool) -> &'static str {
    if proposed_by_client {
        PROPOSED_BY_CLIENT
    } else {
        WAITING_IN_PREVIEW
    }
}

/// A job's progress in the jobs card: "1 of 2 steps".
pub fn job_progress(next_step: usize, total: usize) -> String {
    format!("{next_step} of {total} steps")
}

/// The safety card's tool and workflow count.
pub fn tools_and_workflows(tools: usize, workflows: usize) -> String {
    format!(
        "{tools} typed tools, {workflows} {}",
        if workflows == 1 {
            "workflow"
        } else {
            "workflows"
        }
    )
}

/// The safety card's record count.
pub fn local_records(count: usize) -> String {
    format!(
        "{count} local {} in connected workflow domains",
        if count == 1 { "record" } else { "records" }
    )
}

/// The app-wide workflow notice, shown above every page but Agent workflows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowNotice {
    pub message: String,
    /// Read out as an alert, in the error color.
    pub error: bool,
    /// Set when the notice can be put away; a changed key shows it again.
    pub dismiss_key: Option<String>,
}

/// What the app-wide notice says, if anything. `failed`: the workflow store
/// could not load or save. `saved`: every change is saved. `pending_review`:
/// an MCP proposal or undo request waits. No workflow title, operation or
/// recovery detail ever reaches this app-wide surface.
pub fn workflow_notice(
    state: &AutomationState,
    failed: bool,
    saved: bool,
    pending_review: bool,
) -> Option<WorkflowNotice> {
    let notice = |message: String, error: bool, dismiss_key: Option<String>| {
        Some(WorkflowNotice {
            message,
            error,
            dismiss_key,
        })
    };
    if failed {
        return notice(NEEDS_ATTENTION.into(), true, None);
    }
    if pending_review {
        // Its payload stays in the review.
        return notice(
            crate::workspace_bridge::PENDING_REVIEW_NOTICE.into(),
            false,
            None,
        );
    }
    if let Some(job) = state
        .jobs
        .iter()
        .find(|job| job.status == BackgroundJobStatus::Running)
    {
        let mode = if job.auto_run {
            "Background workflow running"
        } else {
            "Workflow ready for next step"
        };
        let steps = if job.next_step == 1 { "step" } else { "steps" };
        return notice(
            format!("{mode}, {} {steps} saved", job.next_step),
            false,
            None,
        );
    }
    if state
        .jobs
        .iter()
        .any(|job| job.status == BackgroundJobStatus::Paused)
    {
        return notice(
            "Workflow paused, saved steps are retained".into(),
            false,
            None,
        );
    }
    if state
        .jobs
        .iter()
        .any(|job| job.status == BackgroundJobStatus::Failed)
    {
        return notice(NEEDS_ATTENTION.into(), true, None);
    }
    let job = state.jobs.iter().max_by_key(|job| job.updated_at_ms)?;
    if !saved {
        return None;
    }
    let label = match job.status {
        BackgroundJobStatus::Completed => "Workflow completed, saved locally",
        BackgroundJobStatus::RolledBack => "Workflow undone, saved locally",
        _ => return None,
    };
    notice(
        label.into(),
        false,
        Some(format!("{}:{}:{:?}", job.id, job.updated_at_ms, job.status)),
    )
}

/// Whether the notice's Pause applies: a background workflow is running.
pub fn background_running(state: &AutomationState) -> bool {
    state
        .jobs
        .iter()
        .any(|job| job.auto_run && job.status == BackgroundJobStatus::Running)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorkflowOperation;

    fn task(title: &str) -> WorkflowOperation {
        WorkflowOperation::CreateTask {
            title: title.into(),
        }
    }

    #[test]
    fn previews_wait_newest_first_and_the_pages_own_dry_run_is_left_out() {
        let mut state = AutomationState::default();
        state
            .permissions
            .set(AgentDomain::Tasks, AgentAccess::Write, true);
        let older = state.create_workflow("Older", vec![task("a")], 1);
        let shown = state.create_workflow("Shown", vec![task("b")], 2);
        let newer = state.create_workflow("Newer", vec![task("c")], 3);
        let declined = state.create_workflow("Declined", vec![task("d")], 4);
        for id in [&older, &shown, &newer, &declined] {
            state.preview(id, 5).unwrap();
        }
        assert!(state.decline_workflow(&declined, 6));
        assert_eq!(awaiting_review(&state, Some(&shown)), vec![newer, older]);
    }

    #[test]
    fn missing_write_grants_are_named_once_in_step_order() {
        let mut state = AutomationState::default();
        let id = state.create_workflow(
            "Mixed",
            vec![
                WorkflowOperation::AddCapture {
                    content: "a".into(),
                },
                task("b"),
                WorkflowOperation::AddCapture {
                    content: "c".into(),
                },
            ],
            1,
        );
        assert_eq!(
            blocked_domains(&state, &id),
            vec![AgentDomain::Capture, AgentDomain::Tasks]
        );
        assert_eq!(
            grant_write_for(&blocked_domains(&state, &id)).as_deref(),
            Some("Grant Write for: Capture, Tasks.")
        );
        state
            .permissions
            .set(AgentDomain::Capture, AgentAccess::Write, true);
        state
            .permissions
            .set(AgentDomain::Tasks, AgentAccess::Write, true);
        assert_eq!(grant_write_for(&blocked_domains(&state, &id)), None);
    }

    #[test]
    fn counts_read_like_the_page() {
        assert_eq!(job_progress(1, 2), "1 of 2 steps");
        assert_eq!(tools_and_workflows(20, 1), "20 typed tools, 1 workflow");
        assert_eq!(tools_and_workflows(20, 0), "20 typed tools, 0 workflows");
        assert_eq!(
            local_records(1),
            "1 local record in connected workflow domains"
        );
        assert_eq!(
            local_records(3),
            "3 local records in connected workflow domains"
        );
    }

    #[test]
    fn the_notice_never_names_a_workflow_and_failures_come_first() {
        let mut state = AutomationState::default();
        assert_eq!(workflow_notice(&state, false, true, false), None);
        let failed = workflow_notice(&state, true, true, true).unwrap();
        assert!(failed.error && failed.dismiss_key.is_none());
        let pending = workflow_notice(&state, false, true, true).unwrap();
        assert_eq!(
            pending.message,
            crate::workspace_bridge::PENDING_REVIEW_NOTICE
        );
        state
            .permissions
            .set(AgentDomain::Tasks, AgentAccess::Write, true);
        let id = state.create_workflow("Private title", vec![task("secret")], 1);
        state.preview(&id, 2).unwrap();
        state.start_job(&id, 3).unwrap();
        let running = workflow_notice(&state, false, false, false).unwrap();
        assert_eq!(
            running.message,
            "Workflow ready for next step, 0 steps saved"
        );
        assert!(!running.message.contains("Private"));
        assert!(!background_running(&state));
    }

    #[test]
    fn the_notice_counts_one_saved_step_in_the_singular() {
        let mut state = AutomationState::default();
        state
            .permissions
            .set(AgentDomain::Tasks, AgentAccess::Write, true);
        let id = state.create_workflow("Plan", vec![task("a"), task("b"), task("c")], 1);
        state.preview(&id, 2).unwrap();
        let job = state.start_job(&id, 3).unwrap();
        assert!(state.enable_background(&job));
        let message =
            |state: &AutomationState| workflow_notice(state, false, false, false).unwrap().message;
        assert_eq!(
            message(&state),
            "Background workflow running, 0 steps saved"
        );
        assert!(state.complete_step(&job, "task-1", 4));
        assert_eq!(message(&state), "Background workflow running, 1 step saved");
        assert!(state.complete_step(&job, "task-2", 5));
        assert_eq!(
            message(&state),
            "Background workflow running, 2 steps saved"
        );
    }
}
