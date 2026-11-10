//! Chat threads: creating and restoring them, their chat events, selection, pin / settle, and sidebar order.

use super::*;
use crate::views::sidebar_order::SidebarOrder;
use bruhs_models::logic::sidebar::FoldOpen;
pub(super) const THREAD_TITLE_GENERAL: &str = "New thread";
pub(super) const THREAD_TITLE_PROJECT_SESSION: &str = "New session";
pub(super) const THREAD_TITLE_PROJECT_GROUP_NEW: &str = "New thread";

pub(super) fn thread_selection_after_delete(
    current_active_thread_id: Option<String>,
    deleted_thread_id: &str,
) -> Option<String> {
    if current_active_thread_id.as_deref() == Some(deleted_thread_id) {
        None
    } else {
        current_active_thread_id
    }
}

/// Index of the thread `delta` steps from `active` in `order`, wrapping at
/// both ends; with no active thread, ↓ starts at the top and ↑ at the bottom.
pub(super) fn step_thread_index(
    order: &[&str],
    active: Option<&str>,
    delta: isize,
) -> Option<usize> {
    if order.is_empty() {
        return None;
    }
    let len = order.len();
    let current = active.and_then(|id| order.iter().position(|t| *t == id));
    Some(match current {
        // `(i + delta).rem_euclid(len)` in unsigned arithmetic.
        Some(i) => {
            let step = delta.unsigned_abs() % len;
            if delta >= 0 {
                (i + step) % len
            } else {
                (i + len - step) % len
            }
        }
        None if delta >= 0 => 0,
        None => len - 1,
    })
}

/// Reference copied by ⌘⇧C. Nothing handles the scheme yet; it names the
/// thread unambiguously for notes and hand-offs.
pub(super) fn thread_link(thread_id: &str) -> String {
    format!("bruhs://thread/{thread_id}")
}

pub(super) fn default_thread_title(project_name: &str) -> &'static str {
    if project_name.is_empty() {
        THREAD_TITLE_GENERAL
    } else {
        THREAD_TITLE_PROJECT_SESSION
    }
}

impl Workspace {
    pub(super) fn create_thread(
        project_name: String,
        project_path: String,
        model_id: String,
        permission_mode: PermissionMode,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> DesktopThread {
        let thread_title = default_thread_title(&project_name);
        Self::create_thread_with_title(
            project_name,
            project_path,
            model_id,
            permission_mode,
            thread_title,
            window,
            cx,
        )
    }

    pub(super) fn create_thread_with_title(
        project_name: String,
        project_path: String,
        model_id: String,
        permission_mode: PermissionMode,
        thread_title: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> DesktopThread {
        let id = uuid::Uuid::new_v4().to_string();
        let chat_id = id.clone();
        let project_scoped = !project_name.is_empty();
        let chat_project_path = project_path.clone();
        let chat_project_name = project_name.clone();
        let card_model_id = model_id.clone();
        let chat = cx.new(|cx| {
            let mut view = ChatView::new(window, cx);
            view.start_thread(chat_id, thread_title, cx);
            view.set_model(model_id, cx);
            view.permission_mode = permission_mode;
            view.set_project_context(chat_project_path, chat_project_name, project_scoped, cx);
            view
        });
        let _chat_observer = Self::observe_thread_chat(&chat, cx);
        let _chat_event_subscription = Self::subscribe_thread_chat_events(&chat, window, cx);

        DesktopThread {
            id,
            project_name,
            project_path,
            updated_at_ms: EpochMs::now(),
            pinned: false,
            settled: false,
            pin_rank: None,
            active_rank: None,
            snoozed_until_ms: None,
            branch: None,
            model_id: card_model_id,
            chat,
            _chat_observer,
            _chat_event_subscription,
        }
    }

    pub(super) fn create_thread_from_persisted(
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

    /// Redraw the sidebar when a chat changes (its streaming / approval
    /// dots), even for a chat that is not on screen. Saving is driven by the
    /// chat's typed events instead: a bare notify fires on every spinner
    /// frame and streaming delta, none of which changes what is saved.
    pub(super) fn observe_thread_chat(
        chat: &Entity<ChatView>,
        cx: &mut Context<'_, Self>,
    ) -> Subscription {
        cx.observe(chat, |_this, _chat, cx| cx.notify())
    }

    pub(super) fn subscribe_thread_chat_events(
        chat: &Entity<ChatView>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Subscription {
        cx.subscribe_in(
            chat,
            window,
            |this: &mut Self, chat, event: &ChatViewEvent, window, cx| match event {
                ChatViewEvent::OpenSettings => {
                    this.open_settings(&OpenSettings, window, cx);
                }
                ChatViewEvent::MessagesChanged => {
                    // A queued message another thread waits on may be gone.
                    this.settle_thread_waits(chat, cx);
                    this.persist_and_notify(cx);
                }
                ChatViewEvent::Notice(message) => {
                    let message = message.clone();
                    this.meetings_hub
                        .update(cx, |hub, cx| hub.show_notice(message, true, cx));
                }
                ChatViewEvent::TurnStarted { model_id, branch } => {
                    let chat_id = chat.entity_id();
                    if let Some(thread) = this
                        .threads
                        .iter_mut()
                        .find(|thread| thread.chat.entity_id() == chat_id)
                    {
                        thread.model_id = model_id.clone();
                        thread.branch = branch.clone();
                        thread.updated_at_ms = EpochMs::now();
                        // A user turn ends a snooze and a manual settle.
                        // The saved active rank stays.
                        thread.settled = false;
                        thread.snoozed_until_ms = None;
                        let id = thread.id.clone();
                        this.unread_thread_ids.remove(&id);
                    }
                    let refresh = this
                        .active_thread()
                        .is_some_and(|thread| thread.chat.entity_id() == chat.entity_id());
                    if refresh {
                        this.refresh_branch_pull(cx);
                    }
                    this.persist_and_notify(cx);
                }
                ChatViewEvent::TurnFinished => {
                    let chat_id = chat.entity_id();
                    let active = this.active_thread_id.clone();
                    if let Some(thread) = this
                        .threads
                        .iter_mut()
                        .find(|thread| thread.chat.entity_id() == chat_id)
                    {
                        thread.updated_at_ms = EpochMs::now();
                        if active.as_deref() != Some(thread.id.as_str()) {
                            let id = thread.id.clone();
                            this.unread_thread_ids.insert(id);
                        }
                    }
                    if this.diff_panel_visible {
                        this.sync_diff_panel_project(cx);
                    }
                    let chat_is_active = this
                        .active_thread()
                        .is_some_and(|thread| thread.chat.entity_id() == chat.entity_id());
                    if chat_is_active {
                        this.refresh_branch_pull(cx);
                    }
                    this.settle_thread_waits(chat, cx);
                    this.persist_and_notify(cx);
                }
                ChatViewEvent::SetPermissionMode(mode) => {
                    this.set_permission_mode(*mode, cx);
                }
                ChatViewEvent::TogglePanel => {
                    this.toggle_panel(&TogglePanel, window, cx);
                }
                ChatViewEvent::OpenDiff { message_index } => {
                    this.open_diff_for_turn(*message_index, cx);
                }
                ChatViewEvent::OpenProjectPicker => {
                    this.open_project_picker(&OpenProjectPicker, window, cx);
                }
                ChatViewEvent::Handoff { brief } => {
                    this.open_handoff(chat, brief.clone(), window, cx);
                }
                ChatViewEvent::ThreadRequest(request) => {
                    this.answer_thread_request(chat, request, window, cx);
                }
                ChatViewEvent::NewThreadInProject => {
                    let project = this
                        .threads
                        .iter()
                        .find(|thread| thread.chat.entity_id() == chat.entity_id())
                        .map(|thread| (thread.project_name.clone(), thread.project_path.clone()));
                    if let Some((name, path)) = project.filter(|(name, _)| !name.is_empty()) {
                        this.new_thread_in_project(name, path, window, cx);
                    }
                }
                ChatViewEvent::ToggleTasks => {
                    this.set_tasks_panel_visible(
                        tasks_panel_visibility_after_toggle(this.tasks_panel_visible),
                        cx,
                    );
                }
                ChatViewEvent::SelectModel(model_id) => {
                    this.set_current_model(*model_id, cx);
                }
                ChatViewEvent::RunPrefs {
                    choice,
                    effort,
                    previous,
                    base_branch,
                    composer,
                } => {
                    this.run_choice = *choice;
                    this.claude_effort = effort.clone();
                    this.previous_worktree = previous.clone();
                    this.base_branch = base_branch.clone();
                    this.follow_up_steer = composer.follow_up_steer;
                    this.claude_autocompact = composer.autocompact.clone();
                    this.compact_idle = composer.compact_idle;
                    this.thread_access = composer.thread_access;
                    this.task_models = composer.task_models.clone();
                    this.stamp_run_prefs_on_threads(cx);
                    this.persist_and_notify(cx);
                }
                ChatViewEvent::ThreadPlaced {
                    project_path,
                    branch,
                } => {
                    let chat_id = chat.entity_id();
                    if let Some(thread) = this
                        .threads
                        .iter_mut()
                        .find(|thread| thread.chat.entity_id() == chat_id)
                    {
                        thread.project_path = project_path.clone();
                        if branch.is_some() {
                            thread.branch = branch.clone();
                        }
                    }
                    let refresh = this
                        .active_thread()
                        .is_some_and(|thread| thread.chat.entity_id() == chat.entity_id());
                    if refresh {
                        this.refresh_branch_pull(cx);
                    }
                    this.persist_and_notify(cx);
                }
                ChatViewEvent::ToggleDiff => {
                    let (_, diff, _) = right_panel_after_toggle(
                        this.tasks_panel_visible,
                        this.diff_panel_visible,
                        this.preview_panel_visible,
                        RightPanelSurface::Diff,
                    );
                    this.set_diff_panel_visible(diff, cx);
                }
                ChatViewEvent::OpenFile {
                    path,
                    label,
                    project,
                    source,
                } => {
                    this.open_attached_file(
                        path.clone(),
                        label.clone(),
                        project.clone(),
                        *source,
                        cx,
                    );
                }
                ChatViewEvent::RevertFiles { index } => {
                    this.revert_thread_files(chat.clone(), *index, cx);
                }
            },
        )
    }

    /// Copy the shared run prefs onto one chat so its composer can draw them.
    pub(super) fn stamp_chat_run_prefs(&self, chat: &Entity<ChatView>, cx: &mut Context<'_, Self>) {
        let choice = self.run_choice;
        let effort = self.claude_effort.clone();
        let previous = self.previous_worktree.clone();
        let base = self.base_branch.clone();
        let composer = ComposerPrefs {
            follow_up_steer: self.follow_up_steer,
            autocompact: self.claude_autocompact.clone(),
            compact_idle: self.compact_idle,
            thread_access: self.thread_access,
            task_models: self.task_models.clone(),
        };
        // The thread's role as it runs in its project today: a role edited
        // or deleted in Settings takes effect on the thread's next turn.
        let role = {
            let view = chat.read(cx);
            view.role
                .as_deref()
                .and_then(|id| self.roles.resolve(id, &view.project_name))
        };
        chat.update(cx, |chat, cx| {
            chat.role_profile = role;
            chat.apply_run_prefs(choice, effort, previous, base, composer, cx);
        });
    }

    pub(super) fn stamp_run_prefs_on_threads(&self, cx: &mut Context<'_, Self>) {
        let chats: Vec<_> = self
            .threads
            .iter()
            .map(|thread| thread.chat.clone())
            .collect();
        for chat in chats {
            self.stamp_chat_run_prefs(&chat, cx);
        }
    }

    pub(super) fn active_thread(&self) -> Option<&DesktopThread> {
        let active_id = self.active_thread_id.as_deref()?;
        self.threads
            .iter()
            .find(|thread| thread.id.as_str() == active_id)
    }

    pub(super) fn new_thread_in_project(
        &mut self,
        project_name: String,
        project_path: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let thread = Self::create_thread_with_title(
            project_name,
            project_path,
            self.current_model.clone(),
            self.permission_mode,
            THREAD_TITLE_PROJECT_GROUP_NEW,
            window,
            cx,
        );
        self.stamp_chat_run_prefs(&thread.chat, cx);
        thread.chat.update(cx, |chat, cx| {
            chat.tasks_panel_visible = self.tasks_panel_visible;
            chat.diff_panel_visible = self.diff_panel_visible;
            cx.notify();
        });
        self.active_thread_id = Some(thread.id.clone());
        self.threads.push(thread);
        (self.active_view, self.sidebar_tab) =
            project_session_surface_after_create_like_web(self.active_view.clone());
        self.sidebar_filter = sidebar_ui::SidebarFilter::All;
        self.close_transient_surfaces();
        self.persist_and_notify(cx);
    }

    /// A `/handoff` finished in `source`: open a fresh thread in the same
    /// project with `brief` as its draft (web: `handoff_place`).
    pub(super) fn open_handoff(
        &mut self,
        source: &Entity<ChatView>,
        brief: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        // Never pull the user away from the thread they moved to; the brief
        // stays as the source thread's last reply.
        let active = self.active_thread().map(|thread| thread.chat.entity_id());
        if active != Some(source.entity_id()) {
            source.update(cx, |_, cx| {
                cx.emit(ChatViewEvent::Notice(
                    "The handoff brief is the last reply in its thread; copy it into a new thread."
                        .to_string(),
                ));
            });
            return;
        }
        let project = self
            .threads
            .iter()
            .find(|thread| thread.chat.entity_id() == source.entity_id())
            .map(|thread| (thread.project_name.clone(), thread.project_path.clone()));
        match project.filter(|(name, _)| !name.is_empty()) {
            Some((name, path)) => self.new_thread_in_project(name, path, window, cx),
            None => self.new_general_thread(window, cx),
        }
        if let Some(chat) = self.active_thread().map(|thread| thread.chat.clone()) {
            chat.update(cx, |chat, cx| {
                chat.seed_composer(brief);
                cx.emit(ChatViewEvent::Notice(
                    "Handoff ready: review the brief, then send.".to_string(),
                ));
                cx.notify();
            });
        }
    }

    /// Web "Chat with this meeting": the server binds a thread to the meeting;
    /// the desktop opens a fresh local thread named after it.
    pub(super) fn open_meeting_chat_thread(
        &mut self,
        meeting_title: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.new_general_thread(window, cx);
        if let Some(chat) = self.active_thread().map(|thread| thread.chat.clone()) {
            chat.update(cx, |chat, cx| {
                chat.thread_title = meeting_title;
                cx.notify();
            });
            self.persist_and_notify(cx);
        }
    }

    /// Thread ids in sidebar order for the current tab. Closed shelves are skipped.
    pub(super) fn sidebar_thread_ids(&self, cx: &App) -> Vec<String> {
        let shelves = self.sidebar_shelves(cx);
        if !self.sidebar_query(cx).trim().is_empty() {
            return shelves
                .active
                .iter()
                .map(|thread| thread.id.clone())
                .collect();
        }
        fn ids_of<'a>(threads: &[&'a DesktopThread]) -> Vec<&'a str> {
            threads.iter().map(|thread| thread.id.as_str()).collect()
        }
        let order = SidebarOrder {
            pinned: ids_of(&shelves.pinned),
            active: ids_of(&shelves.active),
            working: ids_of(&shelves.working),
            snoozed: ids_of(&shelves.snoozed),
            settled: ids_of(&shelves.settled),
        };
        order
            .flattened(FoldOpen {
                working: self.sidebar_working_open,
                snoozed: self.sidebar_snoozed_open,
                settled: self.sidebar_settled_open,
            })
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    pub(super) fn step_thread(&mut self, delta: isize, cx: &mut Context<'_, Self>) {
        let ids = self.sidebar_thread_ids(cx);
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        if let Some(index) = step_thread_index(&refs, self.active_thread_id.as_deref(), delta) {
            let id = ids[index].clone();
            self.select_thread_by_id(&id, cx);
        }
    }

    pub(super) fn jump_thread(&mut self, index: usize, cx: &mut Context<'_, Self>) {
        let ids = self.sidebar_thread_ids(cx);
        if let Some(id) = ids.get(index).cloned() {
            self.select_thread_by_id(&id, cx);
        }
    }

    pub(super) fn prev_thread(
        &mut self,
        _: &PrevThread,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.step_thread(-1, cx);
    }

    pub(super) fn next_thread(
        &mut self,
        _: &NextThread,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.step_thread(1, cx);
    }

    pub(super) fn jump_thread_action(
        &mut self,
        action: &JumpThread,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.jump_thread(action.0, cx);
    }

    /// ⌘N / ⌘⇧O / ⌘⇧N: a new thread in the project you
    /// are in; from a general chat, a general chat.
    pub(super) fn new_thread(
        &mut self,
        _: &NewThread,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.sidebar_tab == DesktopSidebarTab::Chat {
            self.new_general_thread(window, cx);
            return;
        }
        let current = self
            .active_thread()
            .filter(|thread| !thread.project_name.is_empty())
            .map(|thread| (thread.project_name.clone(), thread.project_path.clone()));
        match current {
            Some((name, path)) => self.new_thread_in_project(name, path, window, cx),
            None => self.open_project_picker(&OpenProjectPicker, window, cx),
        }
    }

    /// The sidebar's "New chat": a general (project-less) thread.
    pub(super) fn new_general_thread(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let active_project = self.active_project.as_ref();
        let project_path =
            general_thread_project_path_like_web(active_project, current_dir_string());
        let project_scoped = general_thread_project_scoped_like_web(active_project);
        let thread = Self::create_thread(
            String::new(),
            project_path.clone(),
            self.current_model.clone(),
            self.permission_mode,
            window,
            cx,
        );
        if project_scoped {
            thread.chat.update(cx, |chat, cx| {
                chat.set_project_context(project_path, String::new(), true, cx);
            });
        }
        self.stamp_chat_run_prefs(&thread.chat, cx);
        thread.chat.update(cx, |chat, cx| {
            chat.tasks_panel_visible = self.tasks_panel_visible;
            chat.diff_panel_visible = self.diff_panel_visible;
            cx.notify();
        });
        self.active_thread_id = Some(thread.id.clone());
        self.threads.push(thread);
        self.active_view = AppView::Chat;
        self.sidebar_tab = DesktopSidebarTab::Chat;
        self.close_transient_surfaces();
        self.persist_and_notify(cx);
    }

    pub(super) fn select_thread_by_id(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        self.thread_menu_id = None;
        self.open_shelf_for_thread(id, cx);
        if let Some(thread) = self.threads.iter().find(|thread| thread.id == id) {
            let sidebar_tab = if thread.project_name.is_empty() {
                DesktopSidebarTab::Chat
            } else {
                DesktopSidebarTab::Code
            };
            self.active_thread_id = Some(id.to_string());
            self.active_view = AppView::Chat;
            self.compact_sidebar_open = false;
            self.sidebar_tab = sidebar_tab;
            if sidebar_tab == DesktopSidebarTab::Code
                && !self.sidebar_filter.matches(&thread.project_name)
            {
                self.sidebar_filter = sidebar_ui::SidebarFilter::All;
            }
            self.unread_thread_ids.remove(id);
            thread.chat.update(cx, |chat, cx| {
                chat.tasks_panel_visible = self.tasks_panel_visible;
                chat.diff_panel_visible = self.diff_panel_visible;
                cx.notify();
            });
            if self.diff_panel_visible {
                self.sync_diff_panel_project(cx);
            }
            if self.files_panel_visible {
                self.load_files(cx);
            }
            self.refresh_branch_pull(cx);
            self.close_transient_surfaces();
            self.persist_and_notify(cx);
        }
    }

    pub(super) fn delete_thread_by_id(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        self.end_thread_waits_on(id, cx);
        self.threads.retain(|thread| thread.id != id);
        self.unread_thread_ids.remove(id);
        self.active_thread_id = thread_selection_after_delete(self.active_thread_id.clone(), id);
        self.persist_and_notify(cx);
    }

    /// Flip `pinned` on thread `id`; returns the new state. Unpinning can be undone.
    pub(super) fn toggle_pinned_by_id(
        &mut self,
        id: &str,
        cx: &mut Context<'_, Self>,
    ) -> Option<bool> {
        let previous = self.shelf_snapshot(id)?;
        let thread = self.threads.iter_mut().find(|thread| thread.id == id)?;
        set_thread_pinned(thread, !thread.pinned);
        let pinned = thread.pinned;
        if !pinned {
            self.push_sidebar_undo("Thread unpinned", SidebarUndoRestore::Shelf(previous), cx);
        }
        self.persist_and_notify(cx);
        Some(pinned)
    }

    /// Flip the manual `settled` flag on thread `id`; settling also marks it
    /// read. Returns the new state. Settling can be undone.
    pub(super) fn toggle_settled_by_id(
        &mut self,
        id: &str,
        cx: &mut Context<'_, Self>,
    ) -> Option<bool> {
        let previous = self.shelf_snapshot(id)?;
        let thread = self.threads.iter_mut().find(|thread| thread.id == id)?;
        set_thread_settled(thread, !thread.settled);
        let settled = thread.settled;
        if settled {
            self.unread_thread_ids.remove(id);
            self.push_sidebar_undo("Thread settled", SidebarUndoRestore::Shelf(previous), cx);
        }
        self.persist_and_notify(cx);
        Some(settled)
    }

    pub(super) fn snooze_thread_by_id(
        &mut self,
        id: &str,
        until_ms: i64,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(previous) = self.shelf_snapshot(id) else {
            return;
        };
        if let Some(thread) = self.threads.iter_mut().find(|thread| thread.id == id) {
            thread.snoozed_until_ms = Some(until_ms);
        }
        self.thread_menu_id = None;
        self.push_sidebar_undo("Thread snoozed", SidebarUndoRestore::Shelf(previous), cx);
        self.persist_and_notify(cx);
    }

    pub(super) fn wake_thread_by_id(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        if let Some(thread) = self.threads.iter_mut().find(|thread| thread.id == id) {
            thread.snoozed_until_ms = None;
            thread.settled = false;
            thread.active_rank = None;
        }
        self.thread_menu_id = None;
        self.persist_and_notify(cx);
    }

    /// Apply saved title instructions to the first user message, or the current title.
    pub(super) fn regenerate_title_by_id(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        let streaming = self
            .threads
            .iter()
            .find(|thread| thread.id == id)
            .is_some_and(|thread| thread.chat.read(cx).is_streaming);
        if streaming {
            return;
        }
        let Some(thread) = self.threads.iter().find(|thread| thread.id == id) else {
            return;
        };
        let chat = thread.chat.clone();
        let project = thread.project_name.clone();
        let current = chat.read(cx).thread_title.clone();
        let prompt = chat.read(cx).messages.iter().find_map(|message| {
            (message.role == MessageRole::User).then(|| message.content.clone())
        });
        let instructions = title_instructions_text(
            &self.title_instructions,
            &self.title_instructions_by_project,
            &project,
        );
        let title = titled_like_web(&current, prompt.as_deref(), &instructions);
        chat.update(cx, |chat, cx| {
            chat.thread_title = title;
            cx.notify();
        });
        self.thread_menu_id = None;
        self.persist_and_notify(cx);
    }

    pub(super) fn discard_active_draft(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(id) = self.active_thread_id.clone() else {
            return;
        };
        let Some(thread) = self.threads.iter().find(|thread| thread.id == id) else {
            return;
        };
        let chat = thread.chat.clone();
        let text = chat.read(cx).input_state.read(cx).value().to_string();
        if text.is_empty() {
            return;
        }
        chat.update(cx, |chat, cx| {
            chat.set_composer_draft(String::new(), window, cx)
        });
        self.thread_menu_id = None;
        self.push_sidebar_undo(
            "Draft discarded",
            SidebarUndoRestore::Draft {
                thread_id: id,
                text,
            },
            cx,
        );
        cx.notify();
    }

    pub(super) fn undo_sidebar(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(entry) = self.sidebar_undo.take() else {
            return;
        };
        match entry.restore {
            SidebarUndoRestore::Draft { thread_id, text } => {
                if let Some(thread) = self.threads.iter().find(|thread| thread.id == thread_id) {
                    let chat = thread.chat.clone();
                    chat.update(cx, |chat, cx| chat.set_composer_draft(text, window, cx));
                }
            }
            SidebarUndoRestore::Shelf(previous) => {
                if let Some(thread) = self
                    .threads
                    .iter_mut()
                    .find(|thread| thread.id == previous.id)
                {
                    thread.pinned = previous.pinned;
                    thread.settled = previous.settled;
                    thread.pin_rank = previous.pin_rank;
                    thread.active_rank = previous.active_rank;
                    thread.snoozed_until_ms = previous.snoozed_until_ms;
                }
                self.persist_and_notify(cx);
                return;
            }
        }
        cx.notify();
    }

    pub(super) fn toggle_pin_active(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(id) = self.active_thread_id.clone() {
            self.toggle_pinned_by_id(&id, cx);
        }
    }

    pub(super) fn toggle_settle_active(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(id) = self.active_thread_id.clone() {
            self.toggle_settled_by_id(&id, cx);
        }
    }

    /// ⌘⇧C: a `bruhs://thread/<id>` reference on the clipboard.
    pub(super) fn copy_active_thread_link(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(id) = self.active_thread_id.as_deref() {
            cx.write_to_clipboard(ClipboardItem::new_string(thread_link(id)));
        }
    }

    /// Copy the pull request the right panel is showing. Unbound until the
    /// user picks a key. `link` copies the http(s) URL; otherwise `#N`.
    pub(super) fn copy_open_pull(&mut self, link: bool, cx: &mut Context<'_, Self>) {
        let shown = self
            .pull_panel_visible
            .then(|| self.branch_pull.clone())
            .flatten();
        let result = match shown.as_ref() {
            Some(pull) => bruhs_models::logic::branch_pull::pull_copy_text(
                true,
                Some(pull.number),
                Some(pull.url.as_str()),
                link,
            ),
            None => bruhs_models::logic::branch_pull::pull_copy_text(false, None, None, link),
        };
        match result {
            Ok(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
            Err(message) => {
                let message = message.to_string();
                self.meetings_hub
                    .update(cx, |hub, cx| hub.show_notice(message, true, cx));
            }
        }
    }

    pub(super) fn copy_pull_link(
        &mut self,
        _: &CopyPullLink,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.copy_open_pull(true, cx);
    }

    pub(super) fn copy_pull_number(
        &mut self,
        _: &CopyPullNumber,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.copy_open_pull(false, cx);
    }

    /// Restore the worktree checkpoint for the user message at `index`, then
    /// rewind the conversation. A checkout thread only rewinds the conversation.
    pub(super) fn revert_thread_files(
        &mut self,
        chat: Entity<ChatView>,
        index: usize,
        cx: &mut Context<'_, Self>,
    ) {
        let chat_id = chat.entity_id();
        let (worktree, checkpoint, streaming) = {
            let view = chat.read(cx);
            let checkpoint = view
                .messages
                .get(index)
                .and_then(|message| message.checkpoint.clone());
            (view.worktree_path.clone(), checkpoint, view.is_streaming)
        };
        if streaming {
            return;
        }
        let Some(worktree) = worktree.filter(|path| !path.trim().is_empty()) else {
            chat.update(cx, |view, cx| {
                view.confirm_rewind(index, cx);
            });
            return;
        };
        let others: Vec<String> = self
            .threads
            .iter()
            .filter(|thread| thread.chat.entity_id() != chat_id)
            .filter_map(|thread| thread.chat.read(cx).worktree_path.clone())
            .collect();
        if let Some(reason) = bruhs_models::logic::rewind::file_revert_block(
            &worktree,
            checkpoint.as_deref(),
            others.iter().map(String::as_str),
        ) {
            let reason = reason.to_string();
            self.meetings_hub
                .update(cx, |hub, cx| hub.show_notice(reason, true, cx));
            return;
        }
        let Some(tree) = checkpoint.filter(|tree| !tree.is_empty()) else {
            return;
        };
        let dir = match bruhs_core::git::ensure_bruhs_worktree(Path::new(&worktree)) {
            Ok(dir) => dir,
            Err(error) => {
                let message = error.to_string();
                self.meetings_hub
                    .update(cx, |hub, cx| hub.show_notice(message, true, cx));
                return;
            }
        };
        let restore = crate::app_state::spawn_tokio(cx, async move {
            bruhs_core::git::restore_worktree(&dir, &tree).await
        });
        cx.spawn(async move |this, cx| {
            let result = restore.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(()) => {
                    let still_running = chat.update(cx, |view, cx| {
                        if view.is_streaming {
                            return true;
                        }
                        view.confirm_rewind(index, cx);
                        false
                    });
                    if still_running {
                        this.meetings_hub.update(cx, |hub, cx| {
                            hub.show_notice("The turn is still running.", true, cx);
                        });
                    }
                }
                Err(error) => {
                    this.meetings_hub.update(cx, |hub, cx| {
                        hub.show_notice(error.to_string(), true, cx);
                    });
                }
            });
        })
        .detach();
    }

    pub(super) fn pin_thread(
        &mut self,
        _: &PinThread,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.toggle_pin_active(cx);
    }

    pub(super) fn settle_thread(
        &mut self,
        _: &SettleThread,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.toggle_settle_active(cx);
    }

    pub(super) fn copy_thread_link(
        &mut self,
        _: &CopyThreadLink,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.copy_active_thread_link(cx);
    }

    pub(super) fn toggle_shelf(&mut self, section: SidebarSection, cx: &mut Context<'_, Self>) {
        match section {
            SidebarSection::Working => self.sidebar_working_open = !self.sidebar_working_open,
            SidebarSection::Snoozed => self.sidebar_snoozed_open = !self.sidebar_snoozed_open,
            SidebarSection::Settled => self.sidebar_settled_open = !self.sidebar_settled_open,
            SidebarSection::Pinned | SidebarSection::Active => return,
        }
        self.persist_and_notify(cx);
    }

    /// Shelves for the current tab, filter, and search. A search is one list.
    pub(super) fn sidebar_shelves(&self, cx: &App) -> SidebarShelves<'_> {
        let filter = self.sidebar_filter.clone();
        let query = self.sidebar_query(cx);
        let searching = !query.trim().is_empty();
        self.sidebar_buckets(
            |thread| {
                self.sidebar_tab.matches_project(&thread.project_name)
                    && sidebar_ui::thread_matches(
                        &thread.project_name,
                        &thread.chat.read(cx).thread_title,
                        if self.sidebar_tab == DesktopSidebarTab::Chat {
                            &sidebar_ui::SidebarFilter::All
                        } else {
                            &filter
                        },
                        &query,
                    )
            },
            searching,
            cx,
        )
    }

    /// Sidebar buckets for the threads matching `filter`.
    pub(super) fn sidebar_buckets<'a>(
        &'a self,
        filter: impl Fn(&DesktopThread) -> bool,
        searching: bool,
        cx: &App,
    ) -> SidebarShelves<'a> {
        let candidates: Vec<&DesktopThread> = self
            .threads
            .iter()
            .filter(|thread| filter(thread))
            .collect();
        if searching {
            let mut active = candidates;
            active.sort_by_key(|thread| std::cmp::Reverse(thread.updated_at_ms));
            return SidebarShelves {
                pinned: Vec::new(),
                active,
                working: Vec::new(),
                snoozed: Vec::new(),
                settled: Vec::new(),
            };
        }
        let metas: Vec<ThreadMeta<'_>> = candidates
            .iter()
            .map(|thread| ThreadMeta {
                id: thread.id.as_str(),
                updated_at_ms: thread.updated_at_ms,
                pinned: thread.pinned,
                settled: thread.settled,
                pin_rank: thread.pin_rank,
                active_rank: thread.active_rank,
                snoozed_until_ms: thread.snoozed_until_ms,
            })
            .collect();
        let working = |id: &str| {
            candidates
                .iter()
                .find(|thread| thread.id == id)
                .is_some_and(|thread| thread.chat.read(cx).is_streaming)
        };
        let needs_you = |id: &str| {
            candidates
                .iter()
                .find(|thread| thread.id == id)
                .is_some_and(|thread| {
                    let chat = thread.chat.read(cx);
                    chat.is_streaming && chat.has_pending_permission()
                })
        };
        let unread = |id: &str| self.unread_thread_ids.contains(id);
        let order = sidebar_order(
            &metas,
            EpochMs::now(),
            self.active_thread_id.as_deref(),
            &working,
            &needs_you,
            &unread,
            self.sidebar_working_section,
        );
        let lookup = |ids: &[&str]| {
            ids.iter()
                .filter_map(|id| candidates.iter().copied().find(|thread| thread.id == *id))
                .collect()
        };
        SidebarShelves {
            pinned: lookup(&order.pinned),
            active: lookup(&order.active),
            working: lookup(&order.working),
            snoozed: lookup(&order.snoozed),
            settled: lookup(&order.settled),
        }
    }

    fn open_shelf_for_thread(&mut self, id: &str, cx: &App) {
        let Some(section) = self.thread_section(id, cx) else {
            return;
        };
        match section {
            SidebarSection::Working => self.sidebar_working_open = true,
            SidebarSection::Snoozed => self.sidebar_snoozed_open = true,
            SidebarSection::Settled => self.sidebar_settled_open = true,
            SidebarSection::Pinned | SidebarSection::Active => {}
        }
    }

    fn thread_section(&self, id: &str, cx: &App) -> Option<SidebarSection> {
        let thread = self.threads.iter().find(|thread| thread.id == id)?;
        let chat = thread.chat.read(cx);
        let facts = bruhs_models::logic::sidebar::SidebarFacts {
            id: thread.id.as_str(),
            updated_at_ms: thread.updated_at_ms.as_millis(),
            pinned: thread.pinned,
            settled: thread.settled,
            pin_rank: thread.pin_rank,
            active_rank: thread.active_rank,
            snoozed_until_ms: thread.snoozed_until_ms,
            working: chat.is_streaming,
            needs_you: chat.is_streaming && chat.has_pending_permission(),
            active: self.active_thread_id.as_deref() == Some(id),
            unread: self.unread_thread_ids.contains(id),
        };
        Some(bruhs_models::logic::sidebar::section(
            &facts,
            EpochMs::now().as_millis(),
            self.sidebar_working_section,
        ))
    }

    fn shelf_snapshot(&self, id: &str) -> Option<ShelfSnapshot> {
        let thread = self.threads.iter().find(|thread| thread.id == id)?;
        Some(ShelfSnapshot {
            id: thread.id.clone(),
            pinned: thread.pinned,
            settled: thread.settled,
            pin_rank: thread.pin_rank,
            active_rank: thread.active_rank,
            snoozed_until_ms: thread.snoozed_until_ms,
        })
    }

    fn push_sidebar_undo(
        &mut self,
        message: &str,
        restore: SidebarUndoRestore,
        cx: &mut Context<'_, Self>,
    ) {
        self.sidebar_undo_generation = self.sidebar_undo_generation.wrapping_add(1);
        let generation = self.sidebar_undo_generation;
        self.sidebar_undo = Some(SidebarUndo {
            message: message.to_string(),
            restore,
        });
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(5_000))
                .await;
            let _ = this.update(cx, |workspace, cx| {
                if workspace.sidebar_undo_generation == generation {
                    workspace.sidebar_undo = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }
}

fn set_thread_pinned(thread: &mut DesktopThread, pinned: bool) {
    thread.pinned = pinned;
    if pinned {
        thread.pin_rank = None;
        thread.settled = false;
        thread.snoozed_until_ms = None;
    } else {
        thread.pin_rank = None;
    }
}

fn set_thread_settled(thread: &mut DesktopThread, settled: bool) {
    if settled {
        thread.settled = true;
        thread.pinned = false;
        thread.pin_rank = None;
        thread.active_rank = None;
        thread.snoozed_until_ms = None;
    } else {
        thread.settled = false;
        thread.active_rank = None;
    }
}

fn title_instructions_text(
    global: &str,
    by_project: &BTreeMap<String, String>,
    project: &str,
) -> String {
    let project_text = by_project
        .get(project)
        .map(|text| text.trim())
        .filter(|text| !text.is_empty());
    match (global.trim().is_empty(), project_text) {
        (true, None) => String::new(),
        (false, None) => global.to_string(),
        (true, Some(project_text)) => project_text.to_string(),
        (false, Some(project_text)) => format!("{}\n{project_text}", global.trim()),
    }
}

fn titled_like_web(current: &str, prompt: Option<&str>, instructions: &str) -> String {
    let from_prompt = prompt.and_then(|prompt| {
        let first = prompt.trim().lines().next().unwrap_or("").trim();
        if first.is_empty() {
            return None;
        }
        let mut titled: String = first.chars().take(48).collect();
        if first.chars().count() > 48 {
            titled.push('…');
        }
        Some(titled)
    });
    let base = from_prompt.unwrap_or_else(|| {
        let trimmed = current.split_whitespace().collect::<Vec<_>>().join(" ");
        if trimmed.is_empty() {
            "New thread".to_string()
        } else {
            trimmed
        }
    });
    bruhs_models::logic::sidebar::apply_title_instructions(&base, instructions)
}

pub(super) fn utc_offset_minutes_east() -> i32 {
    chrono::Local::now().offset().local_minus_utc() / 60
}

pub(super) struct SidebarShelves<'a> {
    pub pinned: Vec<&'a DesktopThread>,
    pub active: Vec<&'a DesktopThread>,
    pub working: Vec<&'a DesktopThread>,
    pub snoozed: Vec<&'a DesktopThread>,
    pub settled: Vec<&'a DesktopThread>,
}

impl Workspace {
    /// The saved threads (or one new general thread) and the one to show first.
    pub(super) fn restore_threads(
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
                        Self::create_thread_from_persisted(
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
