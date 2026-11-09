use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RollbackReview {
    Pending,
    Declined,
    Approved,
}

impl WorkspaceBridge {
    pub(in crate::views::workspace) fn finish_rollback_review(&mut self, id: &str) {
        if let Some(review) = self.rollback_reviews.get_mut(id) {
            *review = RollbackReview::Approved;
        }
    }

    pub(in crate::views::workspace) fn rollback_pending(&self, id: &str) -> bool {
        self.rollback_reviews.get(id) == Some(&RollbackReview::Pending)
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
        let message = "An MCP client requests undo. Review this workflow's completed steps. Undo removes unchanged created records and restores prior relationship context for reminders; Keep changes declines the request.";
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
                    if this.mcp_bridge.rollback_pending(&id) {
                        this.mcp_bridge
                            .rollback_reviews
                            .insert(id.clone(), RollbackReview::Declined);
                        cx.notify();
                    }
                })),
            )
            .into_any_element()
    }
}
