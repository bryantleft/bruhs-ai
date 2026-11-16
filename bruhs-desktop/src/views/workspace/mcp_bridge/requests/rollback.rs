use super::*;

impl Workspace {
    pub(super) fn request_bridge_rollback(
        &mut self,
        id: &str,
        dry_run: bool,
        cx: &mut Context<'_, Self>,
    ) -> Result<Value, String> {
        let request = self.mcp_bridge.reviews.begin_rollback_request(
            &self.automation_state,
            self.save_status.is_saved(),
            id,
        )?;
        let completed_steps = if request.needs_targets() {
            self.automation_rollback_targets(id, cx)?.len()
        } else {
            0
        };
        let (response, opened) = self.mcp_bridge.reviews.finish_rollback_request(
            &self.automation_state,
            request,
            id,
            dry_run,
            completed_steps,
        )?;
        if opened {
            self.reviewed_workflow_id = Some(id.into());
            cx.notify();
        }
        Ok(response)
    }
}
