use super::*;
use bruhs_models::{WorkflowStatus, WorkflowStepStatus};

impl Workspace {
    pub(super) fn bridge_workflow_status(&self, id: &str) -> Result<Value, String> {
        let (_, proposal) = self
            .mcp_bridge
            .proposals
            .values()
            .find(|(workflow_id, _)| workflow_id == id)
            .ok_or("Unknown proposal for this connection session.")?;
        if proposal.operations.iter().any(|operation| {
            !self
                .automation_state
                .permissions
                .allows(operation.domain(), AgentAccess::Write)
        }) {
            return Err("Write access was revoked. Review this workflow in the app.".into());
        }
        let workflow = self
            .automation_state
            .workflows
            .iter()
            .find(|workflow| workflow.id == id)
            .ok_or("Workflow no longer exists.")?;
        let durable = self.save_status.is_saved();
        let completed = workflow
            .steps
            .iter()
            .filter(|step| step.status == WorkflowStepStatus::Completed)
            .count();
        let reverted = workflow
            .steps
            .iter()
            .filter(|step| step.status == WorkflowStepStatus::Reverted)
            .count();
        let job = self
            .automation_state
            .jobs
            .iter()
            .find(|job| job.workflow_id == id);
        // Progress is provisional until the same whole-workspace save which
        // contains the records and checkpoint is acknowledged. Never serialize
        // full target snapshots or unrelated workflows into an agent response.
        Ok(json!({
            "workflow_id": id,
            "workflow_status": workflow.status,
            "durable": durable,
            "requires_user_approval": workflow.status == WorkflowStatus::Previewed || (self.mcp_bridge.rollback_pending(id) && workflow.status != WorkflowStatus::RolledBack),
            "executed": durable && workflow.status == WorkflowStatus::Completed,
            "rolled_back": durable && workflow.status == WorkflowStatus::RolledBack,
            "rollback_review": self.mcp_bridge.rollback_reviews.get(id),
            "progress": {
                "total_steps": workflow.steps.len(),
                "completed_steps": completed,
                "reverted_steps": reverted,
                "durable_completed_steps": durable.then_some(completed),
                "durable_reverted_steps": durable.then_some(reverted),
            },
            "job": job.map(|job| json!({
                "status": job.status,
                "next_step": job.next_step,
                "background_enabled": job.auto_run,
                "can_advance": durable && self.automation_state.pending_operation(&job.id).is_some(),
            })),
        }))
    }
}
