use super::*;

impl DesktopThread {
    pub(super) fn snapshot(&self, cx: &App) -> PersistedThread {
        let chat = self.chat.read(cx);
        PersistedThread {
            id: self.id.clone(),
            title: chat.thread_title.clone(),
            project_name: self.project_name.clone(),
            project_path: Some(self.project_path.clone()),
            updated_at_ms: self.updated_at_ms,
            pinned: self.pinned,
            settled: self.settled,
            pin_rank: self.pin_rank,
            active_rank: self.active_rank,
            snoozed_until_ms: self.snoozed_until_ms,
            branch: self.branch.clone(),
            model_id: chat.model_id.clone(),
            permission_mode: chat.permission_mode,
            prompt_queue: chat.prompt_queue.for_thread(&self.id),
            fresh_session: chat.fresh_session,
            session_id: chat.session_id.clone(),
            session_provider: chat
                .session_provider
                .map(|provider| provider.as_str().to_string()),
            import_context: chat.import_context.clone(),
            goal: chat.goal.clone(),
            worktree_path: chat.worktree_path.clone(),
            effort: chat.effort.clone(),
            run_locked: chat.run_locked,
            role: chat.role.clone(),
            messages: chat
                .messages
                .iter()
                .filter(|message| !message.state.is_streaming())
                .map(|message| PersistedMessage {
                    role: message.role,
                    content: message.content.clone(),
                    model: message.model.clone(),
                    thinking: message.thinking.clone(),
                    usage: message.usage.clone(),
                    duration_ms: message.duration_ms,
                    started_at_ms: message.started_at_ms,
                    completed_at_ms: message.completed_at_ms,
                    tool_calls: message.tool_calls.clone(),
                    files: message.files.clone(),
                    checkpoint: message.checkpoint.clone().filter(|tree| !tree.is_empty()),
                    stopped: message.state.is_stopped(),
                    error: message.state.error().map(str::to_string),
                })
                .collect(),
        }
    }
}
