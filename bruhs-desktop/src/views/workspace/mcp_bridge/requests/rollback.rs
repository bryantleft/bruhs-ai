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
        // The request gets its own Undo requested card, as on the web.
        if opened {
            cx.notify();
        }
        Ok(response)
    }
}
