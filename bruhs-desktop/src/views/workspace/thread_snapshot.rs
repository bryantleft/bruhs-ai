use super::*;

impl DesktopThread {
    pub(super) fn snapshot(&self, cx: &App) -> PersistedThread {
        let chat = self.chat.read(cx);
        PersistedThread {
            model_options: chat.model_options.clone(),
            kiro_choices: chat.kiro_choices.clone(),
            id: self.id.clone(),
            title: chat.thread_title.clone(),
            project_name: self.project_name.clone(),
            project_path: Some(self.project_path.clone()),
            updated_at_ms: self.updated_at_ms,
            pinned: self.pinned,
            settled: self.settled,
            settled_at_ms: self.settled_at_ms.filter(|_| self.settled),
            cleanup_busy: chat.is_streaming
                || chat.handoff_pending
                || chat.has_pending_permission(),
            pin_rank: self.pin_rank,
            active_rank: self.active_rank,
            snoozed_until_ms: self.snoozed_until_ms,
            branch: self.branch.clone(),
            model_id: chat.model_id.clone(),
            permission_mode: chat.permission_mode,
            project_permission_override: chat.project_permission_override,
            prompt_queue: chat.prompt_queue.for_thread(&self.id),
            fresh_session: chat.fresh_session,
            session_id: chat.session_id.clone(),
            session_provider: chat
                .session_provider
                .map(|provider| provider.as_str().to_string()),
            import_context: chat.import_context.clone(),
            import_key: chat.import_key.clone(),
            fork: chat.fork.clone(),
            goal: chat.goal.clone(),
            worktree_path: chat.worktree_path.clone(),
            project_settings_root: chat.project_settings_root.clone(),
            effort: chat.effort.clone(),
            run_locked: chat.run_locked,
            draft_placement: chat.draft_placement.clone(),
            role: chat.role.clone(),
            instructions_stamp: chat.instructions_stamp.clone(),
            messages: chat
                .messages
                .iter()
                .filter(|message| !message.state.is_streaming())
                .map(PersistedMessage::from_message)
                .collect(),
        }
    }
}

impl PersistedMessage {
    pub(super) fn from_message(message: &Message) -> Self {
        Self {
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
            interrupted_by_restart: message.state == TurnState::InterruptedByRestart,
            error: message.state.error().map(str::to_string),
        }
    }
    pub(super) fn into_message(self) -> Message {
        Message {
            role: self.role,
            content: self.content,
            model: self.model,
            thinking: self.thinking,
            usage: self.usage,
            duration_ms: self.duration_ms,
            started_at_ms: self.started_at_ms,
            completed_at_ms: self.completed_at_ms,
            tool_calls: self.tool_calls,
            files: self.files,
            state: if self.interrupted_by_restart {
                TurnState::InterruptedByRestart
            } else {
                TurnState::from_persisted(self.stopped, self.error)
            },
            checkpoint: self.checkpoint.filter(|tree| !tree.is_empty()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Message, PersistedMessage, TurnState};

    #[test]
    fn a_saved_partial_reply_keeps_its_restart_status_and_content() {
        let mut reply = Message::assistant_streaming(123, Some("model".into()));
        reply.content = "Partial response".into();
        reply.state = TurnState::InterruptedByRestart;
        let value = serde_json::to_value(PersistedMessage::from_message(&reply)).unwrap();
        assert_eq!(value["interrupted_by_restart"], true);
        let restored = serde_json::from_value::<PersistedMessage>(value)
            .unwrap()
            .into_message();
        assert_eq!(restored.state, TurnState::InterruptedByRestart);
        assert_eq!(restored.content, reply.content);
        assert_eq!(restored.started_at_ms, Some(123));
    }
}
