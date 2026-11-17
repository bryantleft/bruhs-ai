//! The workflow history on Agent workflows: every workflow, one at a time,
//! with Previous (older) and Next (newer). Shared by web
//! (`components/workflow_history.rs`) and desktop
//! (`automations_ui/history_ui.rs`). Moved from the desktop review panel,
//! which held the only history; the web showed just the latest workflow.
//!
//! The workflow on show carries its own controls ([`controls`]): Run next
//! step, Run in background, Pause, Resume and Undo transaction, whichever
//! its job and steps allow, for any workflow in the history.

use crate::{AutomationState, BackgroundJob, BackgroundJobStatus, Workflow};

/// What the history says when there is no workflow at all.
pub const NO_WORKFLOW_YET: &str =
    "No workflow yet. Preview a suggestion or submit an MCP proposal.";
/// The button that shows the next older workflow.
pub const PREVIOUS_WORKFLOW: &str = "Previous workflow";
/// [`PREVIOUS_WORKFLOW`]'s accessible name.
pub const PREVIOUS_WORKFLOW_LABEL: &str = "Review previous workflow";
/// The button that shows the next newer workflow.
pub const NEXT_WORKFLOW: &str = "Next workflow";
/// [`NEXT_WORKFLOW`]'s accessible name.
pub const NEXT_WORKFLOW_LABEL: &str = "Review next workflow";

/// The workflow under review: `selected` while it still exists, otherwise
/// the newest. `None` only when there is no workflow.
pub fn reviewed<'a>(state: &'a AutomationState, selected: Option<&str>) -> Option<&'a Workflow> {
    page(state, selected).map(|page| page.workflow)
}

/// The history at `selected` (see [`reviewed`]), or `None` with no workflow.
pub fn page<'a>(state: &'a AutomationState, selected: Option<&str>) -> Option<Page<'a>> {
    let workflows = &state.workflows;
    // Every workflow counts, in the order it was created: settled ones,
    // ones still waiting in preview and ones running.
    let index = selected
        .and_then(|id| workflows.iter().position(|workflow| workflow.id == id))
        .or_else(|| workflows.len().checked_sub(1))?;
    Some(Page {
        workflow: workflows.get(index)?,
        number: index + 1,
        total: workflows.len(),
        previous: index
            .checked_sub(1)
            .and_then(|older| workflows.get(older))
            .map(|workflow| workflow.id.as_str()),
        next: workflows
            .get(index + 1)
            .map(|workflow| workflow.id.as_str()),
    })
}

/// One stop in the history.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Page<'a> {
    pub workflow: &'a Workflow,
    /// Its place in the history, 1 for the oldest.
    pub number: usize,
    /// How many workflows the history holds.
    pub total: usize,
    /// The next older workflow's id, which Previous shows.
    pub previous: Option<&'a str>,
    /// The next newer workflow's id, which Next shows.
    pub next: Option<&'a str>,
}

impl Page<'_> {
    /// The history's line and accessible name, such as "Protect focus,
    /// Declined, workflow 2 of 5".
    pub fn label(&self) -> String {
        format!("{}, {}", self.workflow.title, self.status())
    }

    /// The line after the title, such as "Declined, workflow 2 of 5".
    pub fn status(&self) -> String {
        format!(
            "{}, workflow {} of {}",
            self.workflow.status.label(),
            self.number,
            self.total
        )
    }
}

/// Runs the next step of the workflow on show; also its accessible name.
pub const RUN_NEXT_STEP: &str = "Run next step";
/// Lets the job of the workflow on show run its steps on its own while the
/// app is open; also its accessible name, which has to contain the words
/// on the button (desktop's used to read "Run workflow in background").
pub const RUN_IN_BACKGROUND: &str = "Run in background";
/// Pauses the job of the workflow on show at its next step boundary.
pub const PAUSE: &str = "Pause";
/// [`PAUSE`]'s accessible name.
pub const PAUSE_LABEL: &str = "Pause workflow";
/// Resumes the paused job of the workflow on show.
pub const RESUME: &str = "Resume";
/// [`RESUME`]'s accessible name.
pub const RESUME_LABEL: &str = "Resume workflow";
/// Undoes the completed steps of the workflow on show; also its accessible
/// name.
pub const UNDO: &str = "Undo transaction";
/// Why Resume did nothing: the payload changed since review, or a Write
/// grant for a remaining step is off.
pub const RESUME_REFUSED: &str =
    "Recheck the reviewed payload and remaining Write grants before resuming.";
/// Why Run in background did nothing.
pub const BACKGROUND_REFUSED: &str =
    "Recheck the payload and Write grants before enabling background execution.";

/// The controls the workflow on show carries, the same on web and desktop.
/// Each acts on that workflow or its own job, never on another workflow's,
/// so an older workflow can be run, paused, resumed or undone from the
/// history just like the newest.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Controls<'a> {
    /// The job running the workflow: Run next step, Run in background and
    /// Pause.
    pub running: Option<&'a BackgroundJob>,
    /// The job paused on it: Resume.
    pub paused: Option<&'a BackgroundJob>,
    /// Undo transaction, while a completed step can still be undone
    /// ([`Workflow::can_rollback`]).
    pub undo: bool,
}

/// The controls `workflow` carries in `state` (see [`Controls`]).
pub fn controls<'a>(state: &'a AutomationState, workflow: &Workflow) -> Controls<'a> {
    // A workflow has at most one job; take the newest should that change.
    let job = |status| {
        state
            .jobs
            .iter()
            .rev()
            .find(|job| job.status == status && job.workflow_id == workflow.id)
    };
    Controls {
        running: job(BackgroundJobStatus::Running),
        paused: job(BackgroundJobStatus::Paused),
        undo: workflow.can_rollback(),
    }
}

impl Controls<'_> {
    /// Whether the workflow carries any control at all.
    pub fn any(&self) -> bool {
        self.running.is_some() || self.paused.is_some() || self.undo
    }

    /// Whether Run in background is on already, which leaves its button
    /// disabled.
    pub fn in_background(&self) -> bool {
        self.running.is_some_and(|job| job.auto_run)
    }

    /// The job's progress, such as "Running, next checkpoint 2" or "Paused,
    /// resumes at checkpoint 1"; `None` with no running or paused job.
    pub fn job_status(&self) -> Option<String> {
        if let Some(job) = self.running {
            return Some(format!(
                "Running, next checkpoint {}",
                job.next_step.saturating_add(1)
            ));
        }
        self.paused.map(|job| {
            format!(
                "Paused, resumes at checkpoint {}",
                job.next_step.saturating_add(1)
            )
        })
    }
}

#[cfg(test)]
mod tests;
