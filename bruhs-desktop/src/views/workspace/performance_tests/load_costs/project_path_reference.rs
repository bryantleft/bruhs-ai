//! Frozen owned thread restoration with its discarded default chat project path.
use super::super::super::*;

pub(super) fn default_threads(
    state: Option<&mut PersistedWorkspaceState>,
    model: &str,
    permission: PermissionMode,
    window: &mut Window,
    cx: &mut Context<'_, Workspace>,
) -> (Vec<DesktopThread>, Option<String>) {
    Workspace::restore_default_project_path_threads(state, model, permission, window, cx)
}

impl Workspace {
    pub(super) fn create_default_project_path_thread_from_persisted(
        persisted: PersistedThread,
        current_model: &str,
        current_permission_mode: PermissionMode,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> DesktopThread {
        let id = persisted.id;
        let chat_id = id.clone();
        let title = persisted.title;
        let model_id = if persisted.model_id.trim().is_empty() {
            current_model.to_string()
        } else {
            persisted.model_id
        };
        let project_path = persisted
            .project_path
            .filter(|path| !path.is_empty())
            .unwrap_or_else(current_dir_string);
        let permission_mode =
            restored_thread_permission_mode(current_permission_mode, persisted.permission_mode);
        let (session_id, session_provider) =
            restored_owned_session(persisted.session_id, persisted.session_provider.as_deref());
        let messages = persisted
            .messages
            .into_iter()
            .map(|message| Message {
                role: message.role,
                content: message.content,
                model: message.model,
                thinking: message.thinking,
                usage: message.usage,
                duration_ms: message.duration_ms,
                started_at_ms: message.started_at_ms,
                completed_at_ms: message.completed_at_ms,
                tool_calls: message.tool_calls,
                files: message.files,
                state: TurnState::from_persisted(message.stopped, message.error),
                checkpoint: message.checkpoint.filter(|tree| !tree.is_empty()),
            })
            .collect::<Vec<_>>();

        let card_model_id = model_id.clone();
        let chat = cx.new(|cx| {
            let mut view = ChatView::new(window, cx);
            view.start_thread(chat_id, title, cx);
            view.fresh_session = persisted.fresh_session;
            (view.session_id, view.session_provider) = (session_id, session_provider);
            view.import_context = persisted.import_context;
            view.goal = persisted.goal;
            view.worktree_path = persisted.worktree_path;
            view.effort = persisted.effort;
            view.run_locked = persisted.run_locked;
            view.role = persisted.role;
            view.instructions_stamp = persisted.instructions_stamp;
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
                    .into_iter()
                    .filter(|prompt| prompt.thread_id == id)
                    .collect(),
            );
            view.scroll_messages_to_bottom();
            view
        });
        let _chat_observer = Self::observe_thread_chat(&chat, cx);
        let _chat_event_subscription = Self::subscribe_thread_chat_events(&chat, window, cx);

        DesktopThread {
            id,
            project_name: persisted.project_name,
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
            settled_at_ms: persisted.settled_at_ms,
            idle_settle_resumed_at: persisted.idle_settle_resumed_at,
            pin_rank: persisted.pin_rank,
            active_rank: persisted.active_rank,
            snoozed_until_ms: persisted.snoozed_until_ms,
            branch: persisted.branch,
            model_id: card_model_id,
            chat,
            _chat_observer,
            _chat_event_subscription,
        }
    }

    pub(super) fn restore_default_project_path_threads(
        persisted: Option<&mut PersistedWorkspaceState>,
        current_model: &str,
        permission_mode: PermissionMode,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> (Vec<DesktopThread>, Option<String>) {
        let saved_active_thread_id = persisted
            .as_deref()
            .and_then(|state| state.active_thread_id.clone());
        let mut threads = persisted
            .map(|state| {
                std::mem::take(&mut state.threads)
                    .into_iter()
                    .map(|thread| {
                        Self::create_default_project_path_thread_from_persisted(
                            thread,
                            current_model,
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
        let active_thread_id =
            initial_active_thread_id_after_start(saved_active_thread_id, &thread_ids);

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
