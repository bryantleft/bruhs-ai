use super::*;
use crate::views::workspace::mcp_bridge::RollbackReview;

impl Workspace {
    pub(super) fn request_bridge_rollback(
        &mut self,
        id: &str,
        dry_run: bool,
        cx: &mut Context<'_, Self>,
    ) -> Result<Value, String> {
        // The status boundary enforces session ownership and every current
        // Write grant before inspecting or revealing any target state.
        let status = self.bridge_workflow_status(id)?;
        if !self.save_status.is_saved() {
            return Err("Wait for a durable local save before requesting undo.".into());
        }
        let already_undone = status["rolled_back"] == true;
        let declined = self.mcp_bridge.rollback_reviews.get(id) == Some(&RollbackReview::Declined);
        let targets = if already_undone || declined {
            Vec::new()
        } else {
            self.automation_rollback_targets(id, cx)?
        };
        if !dry_run && !already_undone && !declined && !self.mcp_bridge.rollback_pending(id) {
            if self
                .mcp_bridge
                .pending_review_id(&self.automation_state)
                .is_some()
            {
                return Err(
                    "Review or decline the existing MCP request first. No records were changed."
                        .into(),
                );
            }
            self.mcp_bridge
                .rollback_reviews
                .insert(id.into(), RollbackReview::Pending);
            self.reviewed_workflow_id = Some(id.into());
            cx.notify();
        }
        Ok(json!({
            "workflow_id":id,
            "dry_run":dry_run,
            "eligible":!targets.is_empty(),
            "completed_steps":targets.len(),
            "rollback_review":self.mcp_bridge.rollback_reviews.get(id),
            "requires_user_approval":!already_undone && !declined,
            "rolled_back":already_undone,
            "checkpoint_durable":true,
        }))
    }
}
