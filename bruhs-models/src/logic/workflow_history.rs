//! The workflow history on Agent workflows: every workflow, one at a time,
//! with Previous (older) and Next (newer). Shared by web
//! (`components/workflow_history.rs`) and desktop
//! (`automations_ui/history_ui.rs`). Moved from the desktop review panel,
//! which held the only history; the web showed just the latest workflow.

use crate::{AutomationState, Workflow};

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

#[cfg(test)]
mod tests;
