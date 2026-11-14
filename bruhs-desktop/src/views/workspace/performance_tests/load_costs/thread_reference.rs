//! Frozen pre-transfer thread restorers; retain for controlled comparisons.
use super::super::super::*;

impl Workspace {
    pub(super) fn create_original_thread_from_persisted(
        persisted: &PersistedThread,
        current_model: String,
        current_permission_mode: PermissionMode,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> DesktopThread {
        let id = persisted.id.clone();
        let chat_id = id.clone();
        let title = persisted.title.clone();
        let model_id = restored_thread_model_id(&current_model, &persisted.model_id);
        let project_path = persisted
            .project_path
            .clone()
            .filter(|path| !path.is_empty())
            .unwrap_or_else(current_dir_string);
        let permission_mode =
            restored_thread_permission_mode(current_permission_mode, persisted.permission_mode);
        let messages = persisted
            .messages
            .iter()
            .map(|message| Message {
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
                state: TurnState::from_persisted(message.stopped, message.error.clone()),
                checkpoint: message.checkpoint.clone().filter(|tree| !tree.is_empty()),
            })
            .collect::<Vec<_>>();

        let card_model_id = model_id.clone();
        let chat = cx.new(|cx| {
            let mut view = ChatView::new(window, cx);
            view.start_thread(chat_id, title, cx);
            view.fresh_session = persisted.fresh_session;
            (view.session_id, view.session_provider) = restored_session(persisted);
            view.import_context = persisted.import_context.clone();
            view.goal = persisted.goal.clone();
            view.worktree_path = persisted.worktree_path.clone();
            view.effort = persisted.effort.clone();
            view.run_locked = persisted.run_locked;
            view.role = persisted.role.clone();
            view.instructions_stamp = persisted.instructions_stamp.clone();
            view.model_id = model_id;
            view.set_project_context(
                project_path.clone(),
                persisted.project_name.clone(),
                !persisted.project_name.is_empty(),
                cx,
            );
            view.permission_mode = permission_mode;
            view.messages = messages;
            view.is_streaming = false;
            view.restore_prompt_queue(
                persisted
                    .prompt_queue
                    .iter()
                    .filter(|prompt| prompt.thread_id == persisted.id)
                    .cloned()
                    .collect(),
            );
            view.scroll_messages_to_bottom();
            view
        });
        let _chat_observer = Self::observe_thread_chat(&chat, cx);
        let _chat_event_subscription = Self::subscribe_thread_chat_events(&chat, window, cx);

        DesktopThread {
            id,
            project_name: persisted.project_name.clone(),
            project_path,
            // Records from before this field count as fresh so nothing jumps
            // into Settled on the first launch after the upgrade.
            updated_at_ms: if persisted.updated_at_ms > EpochMs::UNIX_EPOCH {
                persisted.updated_at_ms
            } else {
                EpochMs::now()
            },
            pinned: persisted.pinned,
            settled: persisted.settled,
            pin_rank: persisted.pin_rank,
            active_rank: persisted.active_rank,
            snoozed_until_ms: persisted.snoozed_until_ms,
            branch: persisted.branch.clone(),
            model_id: card_model_id,
            chat,
            _chat_observer,
            _chat_event_subscription,
        }
    }

    /// The saved threads (or one new general thread) and the one to show first.
    pub(super) fn restore_original_threads(
        persisted: Option<&PersistedWorkspaceState>,
        current_model: &str,
        permission_mode: PermissionMode,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> (Vec<DesktopThread>, Option<String>) {
        let mut threads = persisted
            .map(|state| {
                state
                    .threads
                    .iter()
                    .map(|thread| {
                        Self::create_original_thread_from_persisted(
                            thread,
                            current_model.to_string(),
                            permission_mode,
                            window,
                            cx,
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if threads.is_empty() {
            threads.push(Self::create_thread(
                String::new(),
                current_dir_string(),
                current_model.to_string(),
                permission_mode,
                window,
                cx,
            ));
        }
        let thread_ids = threads
            .iter()
            .map(|thread| thread.id.clone())
            .collect::<Vec<_>>();
        let active_thread_id = initial_active_thread_id_after_start(
            persisted.and_then(|state| state.active_thread_id.clone()),
            &thread_ids,
        );

        // `BRUHS_DESKTOP_SMOKE_PROMPT` auto-submits into a real thread under
        // its saved permission mode, so it exists only in debug builds.
        #[cfg(debug_assertions)]
        if let Some(prompt) = crate::views::chat::desktop_smoke_prompt() {
            if let Some(active_id) = active_thread_id.as_deref() {
                if let Some(thread) = threads.iter().find(|thread| thread.id == active_id) {
                    thread.chat.update(cx, |chat, cx| {
                        chat.submit_prompt(prompt, cx);
                    });
                }
            }
        }
        (threads, active_thread_id)
    }
}
