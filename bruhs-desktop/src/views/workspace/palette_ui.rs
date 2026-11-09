//! The ⌘K command bar: actions, thread and record search, local servers, and running a pick.

use super::*;

pub(super) const COMMAND_ACTIONS: &[CommandAction] = &[
    CommandAction {
        id: "nav-today",
        group: "Navigate",
        label: "Today",
        hint: "plan & focus",
    },
    CommandAction {
        id: "nav-chat",
        group: "Navigate",
        label: "Chat",
        hint: "conversations",
    },
    CommandAction {
        id: "nav-inbox",
        group: "Navigate",
        label: "Inbox",
        hint: "messages & follow-ups",
    },
    CommandAction {
        id: "nav-knowledge",
        group: "Navigate",
        label: "Knowledge",
        hint: "documents & sources",
    },
    CommandAction {
        id: "nav-tasks",
        group: "Navigate",
        label: "Tasks",
        hint: "projects & issues",
    },
    CommandAction {
        id: "nav-calendar",
        group: "Navigate",
        label: "Calendar",
        hint: "schedule & events",
    },
    CommandAction {
        id: "nav-life",
        group: "Navigate",
        label: "Life",
        hint: "finance, health & habits",
    },
    CommandAction {
        id: "nav-automations",
        group: "Navigate",
        label: "Agent workflows",
        hint: "permissions, jobs & audit",
    },
    CommandAction {
        id: "nav-meetings",
        group: "Navigate",
        label: "Meetings",
        hint: "recorded conversations",
    },
    CommandAction {
        id: "nav-people",
        group: "Navigate",
        label: "People",
        hint: "meeting profiles",
    },
    CommandAction {
        id: "nav-search",
        group: "Navigate",
        label: "Search",
        hint: "everything, instantly",
    },
    CommandAction {
        id: "nav-terminal",
        group: "Navigate",
        label: "Terminal",
        hint: "shells, no thread needed",
    },
    CommandAction {
        id: "open-project",
        group: "Agent",
        label: "Open project...",
        hint: "coding agent",
    },
    CommandAction {
        id: "new-thread",
        group: "Agent",
        label: "New thread",
        hint: "⌘⇧O",
    },
    CommandAction {
        id: "import-thread",
        group: "Agent",
        label: "Import thread...",
        hint: "Claude Code, Codex, T3, Cursor",
    },
    CommandAction {
        id: "pin-thread",
        group: "Actions",
        label: "Pin / unpin thread",
        hint: "⌘⇧P",
    },
    CommandAction {
        id: "settle-thread",
        group: "Actions",
        label: "Settle / unsettle thread",
        hint: "⌘⇧S",
    },
    CommandAction {
        id: "copy-thread-link",
        group: "Actions",
        label: "Copy thread link",
        hint: "⌘⇧C",
    },
    CommandAction {
        id: "copy-resume-command",
        group: "Actions",
        label: "Copy resume command",
        hint: "Claude Code / Codex",
    },
    CommandAction {
        id: "copy-thread-markdown",
        group: "Actions",
        label: "Copy thread as Markdown",
        hint: "for Cursor and others",
    },
    CommandAction {
        id: "continue-in-t3",
        group: "Actions",
        label: "Continue in T3 Code",
        hint: "new T3 thread",
    },
    CommandAction {
        id: "copy-pull-link",
        group: "Actions",
        label: "Copy pull request link",
        hint: "open pull request",
    },
    CommandAction {
        id: "copy-pull-number",
        group: "Actions",
        label: "Copy pull request number",
        hint: "# and the number",
    },
    CommandAction {
        id: "new-task",
        group: "Tasks",
        label: "New task",
        hint: "create",
    },
    CommandAction {
        id: "plan-day",
        group: "Planning",
        label: "Plan my day",
        hint: "review focus blocks",
    },
    CommandAction {
        id: "smart-scheduling",
        group: "Planning",
        label: "Smart scheduling",
        hint: "availability, links & conflicts",
    },
    CommandAction {
        id: "new-document",
        group: "Knowledge",
        label: "New document",
        hint: "Markdown note",
    },
    CommandAction {
        id: "import-conversation",
        group: "Inbox",
        label: "Import conversation",
        hint: "email, Slack, Discord, iMessage, X",
    },
    CommandAction {
        id: "new-web-clip",
        group: "Knowledge",
        label: "Save web clip",
        hint: "source & excerpt",
    },
    CommandAction {
        id: "capture",
        group: "Actions",
        label: "Capture anything",
        hint: "⌘⇧Space",
    },
    CommandAction {
        id: "new-meeting-note",
        group: "Meetings",
        label: "New meeting note",
        hint: "local text",
    },
    CommandAction {
        id: "toggle-sidebar",
        group: "View",
        label: "Toggle sidebar",
        hint: "⌘B",
    },
    CommandAction {
        id: "toggle-diff",
        group: "View",
        label: "Toggle diff panel",
        hint: "⌘D",
    },
    CommandAction {
        id: "toggle-panel",
        group: "View",
        label: "Toggle right panel",
        hint: "⌘⌥B",
    },
    CommandAction {
        id: "toggle-tasks",
        group: "View",
        label: "Toggle tasks panel",
        hint: "⌘⇧T",
    },
    CommandAction {
        id: "toggle-terminal",
        group: "View",
        label: "Toggle terminal",
        hint: "⌘J",
    },
    CommandAction {
        id: "open-terminal",
        group: "View",
        label: "Open in system Terminal",
        hint: "app",
    },
    CommandAction {
        id: "local-servers",
        group: "View",
        label: "Open a local server…",
        hint: "servers",
    },
    CommandAction {
        id: "toggle-preview",
        group: "View",
        label: "Toggle preview",
        hint: "⌘⇧J",
    },
    CommandAction {
        id: "toggle-files",
        group: "View",
        label: "Toggle files panel",
        hint: "project tree",
    },
    CommandAction {
        id: "stash-prompt",
        group: "Actions",
        label: "Stash prompt",
        hint: "⌘S",
    },
    CommandAction {
        id: "model-picker",
        group: "Actions",
        label: "Model",
        hint: "switch provider",
    },
    CommandAction {
        id: "settings",
        group: "Actions",
        label: "Settings",
        hint: "preferences",
    },
];

impl PaletteItem {
    pub(super) fn group(&self) -> &'static str {
        match self {
            Self::Action(action) => action.group,
            Self::Thread { .. } => "Threads",
            Self::Server { .. } => "Local servers",
            Self::Record(_) => "Records",
        }
    }

    pub(super) fn label(&self) -> String {
        match self {
            Self::Action(action) => action.label.to_string(),
            Self::Thread { title, .. } => title.clone(),
            Self::Server { port, .. } => format!("localhost:{port}"),
            Self::Record(hit) => format!("{}: {}", hit.document.kind.label(), hit.document.title),
        }
    }

    pub(super) fn hint(&self) -> String {
        match self {
            Self::Action(action) => action.hint.to_string(),
            Self::Thread { hint, .. } => hint.clone(),
            Self::Server { command, .. } => command.clone(),
            Self::Record(hit) => hit.document.subtitle.clone(),
        }
    }

    pub(super) fn selector(&self) -> String {
        match self {
            Self::Action(action) => format!("desktop-command-action-{}", action.id),
            Self::Thread { id, .. } => format!("desktop-command-thread-{}", element_key(id)),
            Self::Server { port, .. } => format!("desktop-command-server-{port}"),
            Self::Record(hit) => format!(
                "desktop-command-record-{}-{}",
                element_key(hit.document.kind.label()),
                element_key(&hit.document.id)
            ),
        }
    }
}

pub(super) const PALETTE_THREAD_LIMIT_MIXED: usize = 6;
pub(super) const PALETTE_RECORD_LIMIT: usize = 20;
pub(super) const THREAD_LINK_PREFIX: &str = "bruhs://thread/";

/// The id inside a pasted `bruhs://thread/<id>` reference.
pub(super) fn thread_link_id(query: &str) -> Option<&str> {
    let rest = query.trim().strip_prefix(THREAD_LINK_PREFIX)?;
    let id = rest.trim_end_matches('/');
    (!id.is_empty()).then_some(id)
}

/// Every whitespace-separated word of `query` appears in `haystack`
/// (case-insensitive). An empty query matches everything.
pub(super) fn text_matches_query(haystack: &str, query: &str) -> bool {
    let hay = haystack.to_lowercase();
    query
        .split_whitespace()
        .all(|word| hay.contains(&word.to_lowercase()))
}
pub(super) const COMMAND_BAR_INPUT_PLACEHOLDER: &str = "What do you need?";
pub(super) const COMMAND_BAR_BODY_FONT_FAMILY: &str = "Monaspace Neon";
pub(super) const COMMAND_BAR_LABEL_FONT_FAMILY: &str = "Monaspace Argon";
pub(super) const COMMAND_BAR_INPUT_TEXT_SIZE_PX: f32 = 14.0;
pub(super) const COMMAND_BAR_INPUT_X_PADDING_PX: f32 = 20.0;
pub(super) const COMMAND_BAR_INPUT_Y_PADDING_PX: f32 = 16.0;
pub(super) const COMMAND_BAR_OVERLAY_OPACITY: f32 = 0.70;
pub(super) const COMMAND_BAR_LIST_MAX_HEIGHT_PX: f32 = 576.0;
pub(super) const COMMAND_BAR_LIST_X_PADDING_PX: f32 = 8.0;
pub(super) const COMMAND_BAR_LIST_Y_PADDING_PX: f32 = 8.0;
pub(super) const COMMAND_BAR_EMPTY_X_PADDING_PX: f32 = 16.0;
pub(super) const COMMAND_BAR_EMPTY_Y_PADDING_PX: f32 = 32.0;
pub(super) const COMMAND_BAR_ACTION_RADIUS_PX: f32 = 8.0;
pub(super) const COMMAND_BAR_ACTION_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const COMMAND_BAR_ACTION_X_PADDING_PX: f32 = 12.0;
pub(super) const COMMAND_BAR_ACTION_Y_PADDING_PX: f32 = 10.0;
pub(super) const COMMAND_BAR_ACTION_HOVER_BG_OPACITY: f32 = 0.70;
/// Web group headers and footer hints are `text-label` (14px).
pub(super) const COMMAND_BAR_GROUP_TEXT_SIZE_PX: f32 = 14.0;
pub(super) const COMMAND_BAR_GROUP_X_PADDING_PX: f32 = 12.0;
pub(super) const COMMAND_BAR_GROUP_BOTTOM_PADDING_PX: f32 = 4.0;
pub(super) const COMMAND_BAR_FOOTER_TEXT_SIZE_PX: f32 = 14.0;
pub(super) const COMMAND_BAR_FOOTER_X_PADDING_PX: f32 = 20.0;
pub(super) const COMMAND_BAR_FOOTER_Y_PADDING_PX: f32 = 10.0;
pub(super) const COMMAND_BAR_FOOTER_BORDER_OPACITY: f32 = 0.50;
pub(super) const COMMAND_BAR_FOOTER_HINT_GAP_PX: f32 = 4.0;

impl Workspace {
    pub(super) fn reset_and_focus_command_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.modal_start_focus_handle.contains_focused(window, cx) {
            self.palette_return_focus = window
                .focused(cx)
                .map(|focus| (self.active_view.clone(), focus));
        }
        self.command_selected_index = 0;
        self.command_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        let input = self.command_input.clone();
        cx.defer_in(window, move |_this, window, cx| {
            input.update(cx, |input, cx| {
                input.focus(window, cx);
            });
        });
    }

    pub(super) fn command_query(&self, cx: &App) -> String {
        self.command_input.read(cx).value().to_lowercase()
    }

    pub(super) fn command_action_matches_query(action: &CommandAction, query: &str) -> bool {
        query.is_empty()
            || action.label.to_lowercase().contains(query)
            || action.group.to_lowercase().contains(query)
            || action.hint.to_lowercase().contains(query)
    }

    pub(super) fn clamp_command_selected_index(index: usize, count: usize) -> usize {
        if count == 0 {
            0
        } else {
            index.min(count - 1)
        }
    }

    pub(super) fn move_command_selected_index(
        index: usize,
        count: usize,
        key: &str,
    ) -> Option<usize> {
        if !matches!(
            key,
            "down" | "arrowdown" | "ArrowDown" | "up" | "arrowup" | "ArrowUp"
        ) {
            return None;
        }
        if count == 0 {
            return Some(0);
        }

        let index = Self::clamp_command_selected_index(index, count);
        match key {
            "down" | "arrowdown" | "ArrowDown" => Some((index + 1) % count),
            "up" | "arrowup" | "ArrowUp" => Some(if index == 0 { count - 1 } else { index - 1 }),
            _ => None,
        }
    }

    pub(super) fn filtered_command_actions(&self, cx: &App) -> Vec<&'static CommandAction> {
        let query = self.command_query(cx);
        let mut actions: Vec<_> = COMMAND_ACTIONS
            .iter()
            .filter(|action| Self::command_action_matches_query(action, &query))
            .collect();
        // Rendering groups rows; keyboard indices must use that same order.
        actions.sort_by_key(|action| {
            COMMAND_ACTIONS
                .iter()
                .position(|candidate| candidate.group == action.group)
        });
        actions
    }

    pub(super) fn execute_first_command(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let items = self.filtered_palette_items(cx);
        let selected_index =
            Self::clamp_command_selected_index(self.command_selected_index, items.len());
        if let Some(item) = items.get(selected_index).cloned() {
            self.execute_palette_item(item, window, cx);
        }
    }

    pub(super) fn move_command_selection(&mut self, key: &str, cx: &mut Context<'_, Self>) -> bool {
        if Self::move_command_selected_index(0, 0, key).is_none() {
            return false;
        }
        let items = self.filtered_palette_items(cx);
        let count = items.len();
        let Some(next_index) =
            Self::move_command_selected_index(self.command_selected_index, count, key)
        else {
            return false;
        };

        if self.command_selected_index != next_index {
            self.command_selected_index = next_index;
            let mut previous = None;
            let headers = items
                .iter()
                .take(next_index + 1)
                .filter(|item| {
                    let group = item.group();
                    let changed = previous != Some(group);
                    previous = Some(group);
                    changed
                })
                .count();
            self.command_scroll.scroll_to_item(next_index + headers);
            cx.notify();
        }
        true
    }

    pub(super) fn execute_command_action(
        &mut self,
        action_id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match action_id {
            "nav-today" => self.activate_sidebar_view(AppView::Today, window, cx),
            "nav-inbox" => self.open_inbox_view(window, cx),
            "nav-knowledge" => self.open_knowledge_view(window, cx),
            "nav-chat" => self.open_chat_view(&OpenChatView, window, cx),
            "nav-tasks" => self.open_tasks_view(&OpenTasksView, window, cx),
            "nav-calendar" => self.open_calendar_view(&OpenCalendarView, window, cx),
            "nav-life" => self.activate_sidebar_view(AppView::Life, window, cx),
            "nav-automations" => self.activate_sidebar_view(AppView::Automations, window, cx),
            "nav-terminal" => self.open_terminal_view(&OpenTerminalView, window, cx),
            "open-project" => self.open_project_picker(&OpenProjectPicker, window, cx),
            "new-thread" => self.new_thread(&NewThread, window, cx),
            "import-thread" => self.open_import_thread(&ImportThread, window, cx),
            "open-terminal" => self.open_terminal(&OpenTerminal, window, cx),
            "toggle-terminal" => self.toggle_terminal(&ToggleTerminal, window, cx),
            "local-servers" => self.open_local_servers(&OpenLocalServers, window, cx),
            "nav-meetings" => self.open_meetings_list(cx),
            "nav-people" => self.activate_sidebar_view(AppView::People, window, cx),
            "nav-search" => self.activate_sidebar_view(AppView::Search, window, cx),
            "new-task" => self.open_task_create(window, cx),
            "plan-day" => {
                self.active_view = AppView::Today;
                self.draft_desktop_day_plan(cx);
            }
            "smart-scheduling" => {
                self.active_view = AppView::Calendar;
                self.scheduling_visible = true;
                self.close_transient_surfaces();
                self.persist_and_notify(cx);
            }
            "new-document" => self.new_knowledge_document(window, cx),
            "new-web-clip" => self.start_source_clip(window, cx),
            "import-conversation" => self.start_inbox_import(window, cx),
            "capture" => self.open_capture_dialog(&OpenCapture, window, cx),
            "new-meeting-note" => {
                self.close_transient_surfaces();
                self.active_view = AppView::Meetings;
                self.begin_note_edit(None, window, cx);
            }
            "toggle-sidebar" => {
                self.close_transient_surfaces();
                self.toggle_sidebar_for_window(window, cx);
            }
            "settings" => self.open_settings(&OpenSettings, window, cx),
            "toggle-diff" => self.toggle_diff(&ToggleDiff, window, cx),
            "toggle-preview" => self.toggle_preview(&TogglePreview, window, cx),
            "toggle-files" => self.toggle_files(cx),
            "model-picker" => {
                self.close_transient_surfaces();
                self.toggle_model_picker(&ToggleModelPicker, window, cx);
            }
            "toggle-panel" => self.toggle_panel(&TogglePanel, window, cx),
            "toggle-tasks" => self.toggle_tasks(&ToggleTasks, window, cx),
            "stash-prompt" => {
                if let Some(chat) = self.active_thread().map(|thread| thread.chat.clone()) {
                    chat.update(cx, |chat, cx| chat.stash_current_draft(window, cx));
                }
            }
            "pin-thread" => self.toggle_pin_active(cx),
            "settle-thread" => self.toggle_settle_active(cx),
            "copy-thread-link" => self.copy_active_thread_link(cx),
            "copy-resume-command" => {
                self.close_transient_surfaces();
                self.copy_resume_command(cx);
            }
            "copy-thread-markdown" => {
                self.close_transient_surfaces();
                self.copy_thread_markdown(cx);
            }
            "continue-in-t3" => {
                self.close_transient_surfaces();
                self.continue_in_t3(cx);
            }
            "copy-pull-link" => self.copy_open_pull(true, cx),
            "copy-pull-number" => self.copy_open_pull(false, cx),
            _ => {}
        }
    }

    pub(super) fn focus_command_bar(
        &mut self,
        _: &FocusCommandBar,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        // Web ⌘K: toggles the bar alone; a form dialog under it stays open.
        let should_open = !self.command_bar_visible();
        if should_open {
            self.open_top_layer(TopLayer::CommandBar);
        } else {
            self.top_layer = None;
        }
        self.palette_mode = PaletteMode::Commands;
        if should_open {
            self.meetings_hub
                .update(cx, |hub, cx| hub.ensure_search_cache(cx));
            self.reset_and_focus_command_input(window, cx);
        }
        cx.notify();
    }

    pub(super) fn open_palette_in_mode(
        &mut self,
        mode: PaletteMode,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.palette_mode = mode;
        self.open_top_layer(TopLayer::CommandBar);
        self.reset_and_focus_command_input(window, cx);
        cx.notify();
    }

    /// ⌘⇧J: palette listing localhost listeners; picking one opens it.
    pub(super) fn open_local_servers(
        &mut self,
        _: &OpenLocalServers,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.open_palette_in_mode(PaletteMode::Servers, window, cx);
        self.refresh_local_servers(cx);
    }

    /// ⌘⇧F: open universal local search.
    pub(super) fn search_threads(
        &mut self,
        _: &SearchThreads,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.activate_sidebar_view(AppView::Search, window, cx);
    }

    #[cfg(test)]
    pub(super) fn refresh_local_servers(&mut self, cx: &mut Context<'_, Self>) {
        self.local_servers_loading = false;
        cx.notify();
    }

    #[cfg(not(test))]
    pub(super) fn refresh_local_servers(&mut self, cx: &mut Context<'_, Self>) {
        self.local_servers_generation += 1;
        let generation = self.local_servers_generation;
        self.local_servers_loading = true;
        let runtime = Arc::clone(&cx.global::<DesktopState>().runtime);
        let (tx, rx) = async_channel::bounded::<Vec<LocalServer>>(1);
        runtime.spawn(async move {
            let servers = bruhs_core::preview::scan_local_servers(&[]).await;
            let _ = tx.send(servers).await;
        });
        cx.spawn(async move |this, cx| {
            let Ok(servers) = rx.recv().await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if this.local_servers_generation != generation {
                    return;
                }
                this.local_servers = servers;
                this.local_servers_loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn thread_palette_item(&self, thread: &DesktopThread, cx: &App) -> PaletteItem {
        let chat = thread.chat.read(cx);
        let place = if thread.project_name.is_empty() {
            "Chat".to_string()
        } else {
            thread.project_name.clone()
        };
        PaletteItem::Thread {
            id: thread.id.clone(),
            title: chat.thread_title.clone(),
            hint: format!(
                "{place} · {}",
                relative_age(thread.updated_at_ms, EpochMs::now())
            ),
        }
    }

    pub(super) fn matching_thread_items(&self, query: &str, cx: &App) -> Vec<PaletteItem> {
        let mut threads: Vec<&DesktopThread> = self
            .threads
            .iter()
            .filter(|thread| {
                if query.is_empty() {
                    return true;
                }
                let chat = thread.chat.read(cx);
                text_matches_query(&chat.thread_title, query)
                    || text_matches_query(&thread.project_name, query)
                    || chat
                        .messages
                        .iter()
                        .any(|message| text_matches_query(&message.content, query))
            })
            .collect();
        threads.sort_by_key(|thread| std::cmp::Reverse(thread.updated_at_ms));
        threads
            .into_iter()
            .map(|thread| self.thread_palette_item(thread, cx))
            .collect()
    }

    pub(super) fn matching_server_items(&self, query: &str) -> Vec<PaletteItem> {
        self.local_servers
            .iter()
            .filter(|server| {
                text_matches_query(
                    &format!("localhost:{} {}", server.port, server.command),
                    query,
                )
            })
            .map(|server| PaletteItem::Server {
                port: server.port,
                command: server.command.clone(),
            })
            .collect()
    }

    pub(super) fn filtered_palette_items(&self, cx: &Context<'_, Self>) -> Vec<PaletteItem> {
        let query = self.command_query(cx);
        if let Some(id) = thread_link_id(&query) {
            return self
                .threads
                .iter()
                .filter(|thread| thread.id == id)
                .map(|thread| self.thread_palette_item(thread, cx))
                .collect();
        }
        match self.palette_mode {
            PaletteMode::Commands => {
                let mut items: Vec<PaletteItem> = self
                    .filtered_command_actions(cx)
                    .into_iter()
                    .map(PaletteItem::Action)
                    .collect();
                if !query.is_empty() {
                    items.extend(
                        self.matching_thread_items(&query, cx)
                            .into_iter()
                            .take(PALETTE_THREAD_LIMIT_MIXED),
                    );
                    let thread_ids: HashSet<_> = items
                        .iter()
                        .filter_map(|item| match item {
                            PaletteItem::Thread { id, .. } => Some(id.as_str()),
                            _ => None,
                        })
                        .collect();
                    let records: Vec<_> = self
                        .universal_search(&query, cx)
                        .1
                        .into_iter()
                        .filter(|hit| {
                            hit.document.kind != bruhs_models::SearchKind::Thread
                                || !thread_ids.contains(hit.document.id.as_str())
                        })
                        .take(PALETTE_RECORD_LIMIT)
                        .map(PaletteItem::Record)
                        .collect();
                    items.extend(records);
                    items.extend(self.matching_server_items(&query));
                }
                items
            }
            PaletteMode::Servers => self.matching_server_items(&query),
        }
    }

    pub(super) fn execute_palette_item(
        &mut self,
        item: PaletteItem,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match item {
            PaletteItem::Action(action) => self.execute_command_action(action.id, window, cx),
            PaletteItem::Thread { id, .. } => {
                self.close_transient_surfaces();
                self.select_thread_by_id(&id, cx);
            }
            PaletteItem::Server { port, .. } => {
                spawn_detached(browser_open_command(&format!("http://localhost:{port}")));
                self.close_transient_surfaces();
                cx.notify();
            }
            PaletteItem::Record(hit) => {
                self.close_transient_surfaces();
                self.open_universal_search_hit(hit, window, cx);
            }
        }
    }

    pub(super) fn render_palette_mode_chip(&self) -> Option<AnyElement> {
        let (label, hint) = match self.palette_mode {
            PaletteMode::Commands => return None,
            PaletteMode::Servers => (
                "Local servers",
                if self.local_servers_loading {
                    "scanning…"
                } else {
                    "↵ opens in your browser"
                },
            ),
        };
        Some(
            div()
                .debug_selector(|| "desktop-command-mode".to_string())
                .px(px(
                    COMMAND_BAR_LIST_X_PADDING_PX + COMMAND_BAR_ACTION_X_PADDING_PX
                ))
                .pt(px(COMMAND_BAR_LIST_Y_PADDING_PX))
                .flex()
                .items_center()
                .gap_2()
                .font_family(COMMAND_BAR_BODY_FONT_FAMILY)
                .text_size(px(COMMAND_BAR_GROUP_TEXT_SIZE_PX))
                .text_color(theme::rambutan_500())
                .child(
                    div()
                        .rounded(px(COMMAND_BAR_ACTION_RADIUS_PX))
                        .bg(theme::longan_700())
                        .px_2()
                        .py_0p5()
                        .text_color(theme::rambutan_200())
                        .child(label),
                )
                .child(hint)
                .into_any_element(),
        )
    }

    pub(super) fn render_command_action_list(&self, cx: &Context<'_, Self>) -> AnyElement {
        let items = self.filtered_palette_items(cx);
        if items.is_empty() {
            let label = match self.palette_mode {
                PaletteMode::Commands => "nothing found",
                PaletteMode::Servers if self.local_servers_loading => "scanning localhost…",
                PaletteMode::Servers => "nothing is listening on localhost",
            };
            return div()
                .debug_selector(|| "desktop-command-empty".to_string())
                .px(px(COMMAND_BAR_EMPTY_X_PADDING_PX))
                .py(px(COMMAND_BAR_EMPTY_Y_PADDING_PX))
                .text_center()
                .font_family(COMMAND_BAR_LABEL_FONT_FAMILY)
                .font_weight(FontWeight::MEDIUM)
                .text_size(px(COMMAND_BAR_ACTION_TEXT_SIZE_PX))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .text_color(theme::rambutan_500())
                .child(label)
                .into_any_element();
        }

        let selected_index =
            Self::clamp_command_selected_index(self.command_selected_index, items.len());
        let mut elements = Vec::new();
        let mut previous_group = None;
        for (index, item) in items.into_iter().enumerate() {
            let group = item.group();
            if previous_group != Some(group) {
                elements.push(
                    self.command_group_label(group)
                        .when(previous_group.is_some(), |label| label.mt_1())
                        .into_any_element(),
                );
                previous_group = Some(group);
            }
            elements.push(self.render_palette_item(item, index, index == selected_index, cx));
        }

        div()
            .id("desktop-command-menu")
            .debug_selector(|| "desktop-command-menu".to_string())
            .role(Role::Menu)
            .aria_label("Commands")
            .px(px(COMMAND_BAR_LIST_X_PADDING_PX))
            .py(px(COMMAND_BAR_LIST_Y_PADDING_PX))
            .max_h(px(COMMAND_BAR_LIST_MAX_HEIGHT_PX))
            .overflow_y_scroll()
            .track_scroll(&self.command_scroll)
            .children(elements)
            .vertical_scrollbar(&self.command_scroll)
            .into_any_element()
    }

    pub(super) fn render_palette_item(
        &self,
        item: PaletteItem,
        index: usize,
        is_selected: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        if let PaletteItem::Action(action) = item {
            return self.render_command_action(action, index, is_selected, cx);
        }
        let selector = item.selector();
        let debug_selector = selector.clone();
        let label = item.label();
        let hint = item.hint();
        let item_for_click = item.clone();
        let item_for_key = item;
        div()
            .id(ElementId::Name(selector.into()))
            .debug_selector(move || debug_selector)
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .rounded(px(COMMAND_BAR_ACTION_RADIUS_PX))
            .px(px(COMMAND_BAR_ACTION_X_PADDING_PX))
            .py(px(COMMAND_BAR_ACTION_Y_PADDING_PX))
            .font_family(COMMAND_BAR_LABEL_FONT_FAMILY)
            .text_size(px(COMMAND_BAR_ACTION_TEXT_SIZE_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(theme::rambutan_200())
            .tab_index(0)
            .role(Role::MenuItem)
            .aria_label(label.clone())
            .aria_selected(is_selected)
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .when(is_selected, |el| {
                el.bg(theme::longan_700().opacity(COMMAND_BAR_ACTION_HOVER_BG_OPACITY))
                    .text_color(theme::rambutan_100())
            })
            .hover(|el| {
                el.bg(theme::longan_800().opacity(COMMAND_BAR_ACTION_HOVER_BG_OPACITY))
                    .text_color(theme::rambutan_100())
            })
            .on_hover(cx.listener(move |this, hovered, _window, cx| {
                if *hovered && this.command_selected_index != index {
                    this.command_selected_index = index;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _ev, window, cx| {
                this.execute_palette_item(item_for_click.clone(), window, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.execute_palette_item(item_for_key.clone(), window, cx);
                }
            }))
            .child(div().min_w_0().truncate().child(label))
            .child(
                div()
                    .min_w_0()
                    .max_w(relative(0.4))
                    .truncate()
                    .text_size(px(COMMAND_BAR_ACTION_TEXT_SIZE_PX))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::rambutan_500())
                    .child(hint),
            )
            .into_any_element()
    }

    pub(super) fn render_command_action(
        &self,
        action: &'static CommandAction,
        action_index: usize,
        is_selected: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let action_id = action.id;
        div()
            .id(("desktop-command-action", element_key(action.id)))
            .debug_selector(move || format!("desktop-command-action-{action_id}"))
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .rounded(px(COMMAND_BAR_ACTION_RADIUS_PX))
            .px(px(COMMAND_BAR_ACTION_X_PADDING_PX))
            .py(px(COMMAND_BAR_ACTION_Y_PADDING_PX))
            .font_family(COMMAND_BAR_LABEL_FONT_FAMILY)
            .text_size(px(COMMAND_BAR_ACTION_TEXT_SIZE_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(theme::rambutan_200())
            .tab_index(0)
            .role(Role::MenuItem)
            .aria_label(action.label)
            .aria_selected(is_selected)
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .when(is_selected, |el| {
                el.bg(theme::longan_700().opacity(COMMAND_BAR_ACTION_HOVER_BG_OPACITY))
                    .text_color(theme::rambutan_100())
            })
            .hover(|el| {
                el.bg(theme::longan_800().opacity(COMMAND_BAR_ACTION_HOVER_BG_OPACITY))
                    .text_color(theme::rambutan_100())
            })
            .on_hover(cx.listener(move |this, hovered, _window, cx| {
                if *hovered && this.command_selected_index != action_index {
                    this.command_selected_index = action_index;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _ev, window, cx| {
                this.execute_command_action(action_id, window, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.execute_command_action(action_id, window, cx);
                }
            }))
            .child(div().child(action.label))
            .child(
                div()
                    .text_size(px(COMMAND_BAR_ACTION_TEXT_SIZE_PX))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::rambutan_500())
                    .child(action.hint),
            )
            .into_any_element()
    }

    pub(super) fn render_command_bar(&self, cx: &Context<'_, Self>) -> AnyElement {
        div()
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .left_0()
            .occlude()
            .id("desktop-command-bar-overlay")
            .debug_selector(|| "desktop-command-bar-overlay".to_string())
            .bg(theme::longan_950().opacity(COMMAND_BAR_OVERLAY_OPACITY))
            .flex()
            .items_start()
            .justify_center()
            .pt(px(DESKTOP_DIALOG_TOP_PADDING_PX))
            .on_click(cx.listener(|this, _ev, _window, cx| {
                this.close_transient_surfaces();
                cx.notify();
            }))
            .child(
                div()
                    .id("desktop-command-bar")
                    .role(Role::Dialog)
                    .aria_label("Command palette")
                    .child(self.blur_scrim.backdrop())
                    .debug_selector(|| "desktop-command-bar".to_string())
                    .w(px(DESKTOP_DIALOG_LG_WIDTH_PX))
                    .max_w(relative(DESKTOP_DIALOG_MAX_WIDTH_RATIO))
                    .rounded(px(DESKTOP_DIALOG_RADIUS_PX))
                    .border_1()
                    .border_color(theme::ring_modal())
                    .bg(theme::longan_800())
                    .text_color(cx.theme().popover_foreground)
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .track_focus(&self.modal_start_focus_handle)
                    .tab_group()
                    .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        let key = event.keystroke.key.as_str();
                        if event.keystroke.modifiers == Modifiers::default()
                            && (matches!(key, "escape" | "enter")
                                || Self::move_command_selected_index(0, 0, key).is_some())
                        {
                            let composing = this.command_input.update(cx, |input, cx| {
                                input.marked_text_range(window, cx).is_some()
                            });
                            if composing {
                                if key == "enter" {
                                    window.prevent_default();
                                    cx.stop_propagation();
                                }
                                return;
                            }
                            if key == "enter" {
                                // Consume activation before a command changes focus;
                                // otherwise Enter can be delivered as text to its editor.
                                window.prevent_default();
                                cx.stop_propagation();
                                this.execute_first_command(window, cx);
                                return;
                            }
                            if key == "escape" {
                                this.close_transient_surfaces();
                                cx.notify();
                            } else {
                                this.move_command_selection(key, cx);
                            }
                            window.prevent_default();
                            cx.stop_propagation();
                        }
                    }))
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        this.trap_modal_tab_focus(event, window, cx);
                    }))
                    .on_click(cx.listener(|_this, _ev, _window, cx| {
                        cx.stop_propagation();
                    }))
                    .child(
                        div()
                            .px(px(COMMAND_BAR_INPUT_X_PADDING_PX))
                            .py(px(COMMAND_BAR_INPUT_Y_PADDING_PX))
                            .border_b_1()
                            .border_color(theme::longan_800())
                            .child(
                                Input::new(&self.command_input)
                                    .aria_label("Command search")
                                    .appearance(false)
                                    .bordered(false)
                                    .focus_bordered(false)
                                    .cleanable(false)
                                    .font_family(COMMAND_BAR_BODY_FONT_FAMILY)
                                    .text_size(px(COMMAND_BAR_INPUT_TEXT_SIZE_PX)),
                            ),
                    )
                    .children(self.render_palette_mode_chip())
                    .child(self.render_command_action_list(cx))
                    .child(
                        div()
                            .px(px(COMMAND_BAR_FOOTER_X_PADDING_PX))
                            .py(px(COMMAND_BAR_FOOTER_Y_PADDING_PX))
                            .border_t_1()
                            .border_color(
                                theme::longan_800().opacity(COMMAND_BAR_FOOTER_BORDER_OPACITY),
                            )
                            .flex()
                            .items_center()
                            .justify_between()
                            .font_family(COMMAND_BAR_BODY_FONT_FAMILY)
                            .text_size(px(COMMAND_BAR_FOOTER_TEXT_SIZE_PX))
                            .text_color(theme::rambutan_500())
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(COMMAND_BAR_FOOTER_HINT_GAP_PX))
                                    .child(
                                        div()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(theme::rambutan_500())
                                            .child("↑↓"),
                                    )
                                    .child("navigate ·")
                                    .child(
                                        div()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(theme::rambutan_500())
                                            .child("↵"),
                                    )
                                    .child("select"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(COMMAND_BAR_FOOTER_HINT_GAP_PX))
                                    .child(
                                        div()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(theme::rambutan_500())
                                            .child("esc"),
                                    )
                                    .child("close"),
                            ),
                    )
                    .child(self.render_modal_focus_sentinel()),
            )
            .into_any_element()
    }

    pub(super) fn command_group_label(&self, label: &'static str) -> Div {
        div()
            .px(px(COMMAND_BAR_GROUP_X_PADDING_PX))
            .pb(px(COMMAND_BAR_GROUP_BOTTOM_PADDING_PX))
            .font_family(COMMAND_BAR_BODY_FONT_FAMILY)
            .text_size(px(COMMAND_BAR_GROUP_TEXT_SIZE_PX))
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme::rambutan_300())
            .child(label.to_ascii_uppercase())
    }
}

impl Workspace {
    /// The command bar input: typing re-filters from the top; Enter runs the first match.
    pub(super) fn new_command_input(
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> Entity<InputState> {
        let command_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(COMMAND_BAR_INPUT_PLACEHOLDER));
        subscriptions.push(cx.subscribe_in(
            &command_input,
            window,
            |this: &mut Self, _input, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => {
                    this.command_selected_index = 0;
                    this.command_scroll.set_offset(Point::default());
                    cx.notify();
                }
                InputEvent::PressEnter { secondary, .. } if !*secondary => {
                    let composing = this.command_input.update(cx, |input, cx| {
                        input.marked_text_range(window, cx).is_some()
                    });
                    if !composing {
                        this.execute_first_command(window, cx);
                    }
                }
                _ => {}
            },
        ));
        command_input
    }
}
