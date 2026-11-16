use super::*;

#[cfg(test)]
pub(in crate::views::workspace) use bruhs_models::workspace_bridge::RollbackReview;

impl WorkspaceBridge {
    pub(in crate::views::workspace) fn finish_rollback_review(&mut self, id: &str) {
        self.reviews.finish_rollback_review(id);
    }

    pub(in crate::views::workspace) fn rollback_pending(&self, id: &str) -> bool {
        self.reviews.rollback_pending(id)
    }
}

impl Workspace {
    pub(in crate::views::workspace) fn render_mcp_rollback_review(
        &self,
        id: &str,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        if !self.mcp_bridge.rollback_pending(id)
            || self.automation_state.workflows.iter().any(|workflow| {
                workflow.id == id && workflow.status == bruhs_models::WorkflowStatus::RolledBack
            })
        {
            return div().into_any_element();
        }
        let id = id.to_owned();
        let message = bruhs_models::workspace_bridge::ROLLBACK_REVIEW_NOTE;
        div()
            .id("mcp-rollback-review")
            .role(Role::Group)
            .aria_label(message)
            .debug_selector(|| "desktop-mcp-rollback-review".into())
            .mt_2()
            .text_size(px(11.0))
            .text_color(theme::lychee_300())
            .child(message)
            .child(
                web_button(
                    "mcp-decline-rollback",
                    WebButtonVariant::Outline,
                    WebButtonSize::Small,
                    ButtonState::Enabled,
                )
                .debug_selector(|| "desktop-decline-mcp-rollback".into())
                .aria_label("Keep changes and decline requested undo")
                .mt_2()
                .child("Keep changes")
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.mcp_bridge.reviews.decline_rollback(&id) {
                        cx.notify();
                    }
                })),
            )
            .into_any_element()
    }
}
