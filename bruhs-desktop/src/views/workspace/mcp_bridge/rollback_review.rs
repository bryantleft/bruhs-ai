use super::*;

#[cfg(test)]
pub(in crate::views::workspace) use bruhs_models::workspace_bridge::RollbackReview;

impl WorkspaceBridge {
    pub(in crate::views::workspace) fn finish_rollback_review(&mut self, id: &str) {
        self.reviews.finish_rollback_review(id);
    }

    #[cfg(test)]
    pub(in crate::views::workspace) fn rollback_pending(&self, id: &str) -> bool {
        self.reviews.rollback_pending(id)
    }

    /// Keep changes: the client's undo request is declined.
    pub(in crate::views::workspace) fn decline_rollback(&mut self, id: &str) -> bool {
        self.reviews.decline_rollback(id)
    }
}
