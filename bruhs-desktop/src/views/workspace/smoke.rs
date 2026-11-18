//! Launch-check hooks driven by `BRUHS_DESKTOP_*SMOKE*` environment
//! variables (see `src/smoke.rs`). Debug builds only: a release build has no
//! way to auto-drive the window.

use super::*;
use gpui_component::zoom::px;

/// One key press after another, each once the window has drawn the last
/// one's effect, as typing gives it: a key down, then the key up that
/// clicks a focused button.
fn press_keys(keys: Vec<Keystroke>, window: &mut Window, cx: &mut App) {
    let mut keys = keys.into_iter();
    let Some(keystroke) = keys.next() else {
        return;
    };
    window.dispatch_keystroke(keystroke.clone(), cx);
    window.dispatch_event(PlatformInput::KeyUp(KeyUpEvent { keystroke }), cx);
    let rest: Vec<Keystroke> = keys.collect();
    if !rest.is_empty() {
        window.on_next_frame(move |window, _cx| {
            window.on_next_frame(move |window, cx| press_keys(rest, window, cx));
        });
    }
}

impl Workspace {
    /// `BRUHS_DESKTOP_COMMANDS_SMOKE`: a fresh project-less chat, made
    /// active. Returns its view.
    pub fn smoke_new_chat(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Option<Entity<ChatView>> {
        self.new_general_thread(window, cx);
        self.smoke_active_chat()
    }

    /// The active thread's chat view and id.
    pub fn smoke_active_chat(&self) -> Option<Entity<ChatView>> {
        self.active_thread().map(|thread| thread.chat.clone())
    }

    /// The active thread's id.
    pub fn smoke_active_id(&self) -> Option<String> {
        self.active_thread_id.clone()
    }

    /// `BRUHS_DESKTOP_THREADS_SMOKE`: never ask, so a message between threads
    /// needs no click.
    pub fn smoke_bypass_permissions(&mut self, cx: &mut Context<'_, Self>) {
        self.set_permission_mode(PermissionMode::BypassPermissions, cx);
    }

    /// `BRUHS_DESKTOP_THREADS_SMOKE`: a role for the agent to start a thread
    /// with, as Settings would save it.
    pub fn smoke_add_role(
        &mut self,
        name: &str,
        model: &str,
        instructions: &str,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(id) = self.roles.add(name) {
            self.roles.update(&id, |role| {
                role.model = Some(model.to_string());
                role.instructions = instructions.to_string();
            });
            self.stamp_run_prefs_on_threads(cx);
        }
    }

    /// `BRUHS_DESKTOP_THREADS_SMOKE`: the chat of the thread titled `title`.
    pub fn smoke_chat_titled(&self, title: &str, cx: &App) -> Option<Entity<ChatView>> {
        self.threads
            .iter()
            .map(|thread| &thread.chat)
            .find(|chat| chat.read(cx).thread_title == title)
            .cloned()
    }

    /// Id of the active terminal session (`BRUHS_DESKTOP_TERMINAL_SMOKE`).
    pub fn terminal_active_id(&self, cx: &App) -> Option<String> {
        self.terminal_panel.read(cx).active_id().map(str::to_string)
    }

    /// `BRUHS_DESKTOP_TERMINAL_SMOKE`: show the Settings page.
    pub fn smoke_open_settings(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.open_settings(&OpenSettings, window, cx);
    }

    /// End of a smoke run: drop the fixture thread a scene created.
    pub fn smoke_cleanup(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(id) = self.smoke_thread_id.take() {
            self.delete_thread_by_id(&id, cx);
            if let Some(previous) = self.smoke_previous_active_id.take() {
                if self.threads.iter().any(|thread| thread.id == previous) {
                    self.active_thread_id = Some(previous);
                    self.persist_and_notify(cx);
                }
            }
        }
    }

    /// Thread of the last card in the sidebar list, folds included.
    fn smoke_bottom_card_id(&self, cx: &App) -> Option<String> {
        let shelves = self.sidebar_shelves(cx);
        let bottom = [
            (&shelves.settled, self.sidebar_settled_open),
            (&shelves.snoozed, self.sidebar_snoozed_open),
            (&shelves.working, self.sidebar_working_open),
            (&shelves.active, true),
            (&shelves.pinned, true),
        ]
        .into_iter()
        .find_map(|(threads, open)| open.then(|| threads.last()).flatten())
        .map(|thread| thread.id.clone());
        bottom
    }

    /// The crew page on screen: view, page agent and shown name.
    pub fn smoke_crew_report(&self, cx: &App) -> String {
        let panel = self.reactive_automations.read(cx);
        format!(
            "view={:?} page={:?} name={:?}",
            self.active_view,
            panel.bruh_page_agent(),
            panel.smoke_bruh_name(),
        )
    }

    /// `BRUHS_DESKTOP_SMOKE_SCENE`: put the window in one named state for a
    /// comparison screenshot. Returns false for an unknown scene.
    pub fn smoke_scene(
        &mut self,
        scene: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        match scene {
            "today" => self.activate_sidebar_view(AppView::Today, window, cx),
            "tasks" => self.activate_primary_view(AppView::Tasks, window, cx),
            "calendar" => self.activate_primary_view(AppView::Calendar, window, cx),
            "calendar-week" | "calendar-day" => {
                self.activate_primary_view(AppView::Calendar, window, cx);
                let mode = if scene == "calendar-week" {
                    DesktopCalendarMode::Week
                } else {
                    DesktopCalendarMode::Day
                };
                self.set_calendar_mode(mode, cx);
            }
            "terminal" => self.activate_primary_view(AppView::Terminal, window, cx),
            "command-bar" => self.focus_command_bar(&FocusCommandBar, window, cx),
            "model-picker" => self.toggle_model_picker(&ToggleModelPicker, window, cx),
            "project-picker" => self.open_project_picker(&OpenProjectPicker, window, cx),
            "import-thread" => self.open_import_thread(&ImportThread, window, cx),
            "task-create" => self.open_task_create(window, cx),
            "event-create" => self.open_event_create(None, window, cx),
            "meetings" => self.open_meetings_list(cx),
            "meeting-detail" => {
                // The first meeting the hub has loaded, else the list.
                let first = self
                    .meetings_hub
                    .read(cx)
                    .meetings
                    .first()
                    .map(|m| m.id.clone());
                match first {
                    Some(id) => self.open_meeting_detail(id, cx),
                    None => self.open_meetings_list(cx),
                }
            }
            "people" => self.activate_sidebar_view(AppView::People, window, cx),
            "codex-login" => {
                if let Some(chat) = self.active_thread().map(|thread| thread.chat.clone()) {
                    chat.update(cx, |chat, cx| chat.smoke_start_codex_login(cx));
                }
            }
            "codex-report" => {
                if let Some(chat) = self.active_thread().map(|thread| thread.chat.clone()) {
                    eprintln!(
                        "BRUHS_DESKTOP_SMOKE codex {}",
                        chat.read(cx).smoke_codex_auth()
                    );
                }
            }
            // The first profile opened, as a click on its name does.
            "person-expand" => {
                self.activate_sidebar_view(AppView::People, window, cx);
                if let Some(id) = self
                    .meetings_hub
                    .read(cx)
                    .people
                    .first()
                    .map(|p| p.id.clone())
                {
                    self.expanded_person_id = Some(id.clone());
                    self.load_relationship_inputs(&id, window, cx);
                }
            }
            // Life opens on Finance; this is its Health tab, to compare with
            // the web's.
            "life-health" => {
                self.activate_sidebar_view(AppView::Life, window, cx);
                self.life_tab = DesktopLifeTab::Health;
                cx.notify();
            }
            "meeting-suggestions" => {
                self.open_meetings_list(cx);
                self.smoke_meeting_suggestions(cx);
            }
            "updates" => {
                self.open_settings(&OpenSettings, window, cx);
                self.set_settings_section(SettingsSection::Updates, cx);
            }
            "settings-about" => {
                self.open_settings(&OpenSettings, window, cx);
                self.set_settings_section(SettingsSection::About, cx);
            }
            "search" => self.activate_sidebar_view(AppView::Search, window, cx),
            // Agent workflows with the MCP connection on, to compare with the
            // web (PARITY-05); a client then connects through its file.
            "workbench" => self.activate_sidebar_view(AppView::Workbench, window, cx),
            // Verification's Use in composer pressed inside the embedded
            // Workbench page (PARITY-04): the draft must reach the composer
            // through the page's own `window.ipc` relay.
            "workbench-use-draft" => {
                let script = r#"(() => {
  location.hash = '#verification';
  const deadline = Date.now() + 20000;
  (function step() {
    const doc = document.getElementById('panel').contentDocument;
    const box = doc && doc.getElementById('draft');
    const use = doc && doc.getElementById('use-draft');
    if (box && use && !use.hidden) {
      box.value = 'Ship the launch post on Friday.';
      use.click();
      return;
    }
    if (Date.now() < deadline) setTimeout(step, 200);
  })();
})();"#;
                let result = self
                    .workbench_panel
                    .as_ref()
                    .ok_or_else(|| "Workbench is not open".to_string())
                    .and_then(|panel| panel.read(cx).smoke_evaluate(script, cx));
                eprintln!("BRUHS_DESKTOP_SMOKE workbench-use-draft {result:?}");
            }
            // A change that must be saved, and the save strip's Retry: for the
            // failed-save comparison (PARITY-04).
            "smoke-edit" => {
                let on = self.automation_state.permissions.allows(
                    bruhs_models::AgentDomain::Tasks,
                    bruhs_models::AgentAccess::Write,
                );
                self.automation_state.permissions.set(
                    bruhs_models::AgentDomain::Tasks,
                    bruhs_models::AgentAccess::Write,
                    !on,
                );
                self.persist_and_notify(cx);
            }
            "retry-save" => self.persist_and_notify(cx),
            // `toggle-grant:Calendar`: flip a domain's Write grant and save.
            scene if scene.starts_with("toggle-grant:") => {
                let label = scene.trim_start_matches("toggle-grant:");
                if let Some(domain) = bruhs_models::AgentDomain::ALL
                    .into_iter()
                    .find(|domain| domain.label() == label)
                {
                    let access = bruhs_models::AgentAccess::Write;
                    let on = self.automation_state.permissions.allows(domain, access);
                    self.automation_state.permissions.set(domain, access, !on);
                    self.persist_and_notify(cx);
                }
            }

            "save-report" => eprintln!(
                "BRUHS_DESKTOP_SMOKE save status={:?} tasks_write={}",
                self.save_status,
                self.automation_state.permissions.allows(
                    bruhs_models::AgentDomain::Tasks,
                    bruhs_models::AgentAccess::Write
                )
            ),
            "composer-report" => {
                let text = self
                    .smoke_active_chat()
                    .map(|chat| chat.read(cx).input_state.read(cx).value().to_string());
                eprintln!(
                    "BRUHS_DESKTOP_SMOKE composer view={:?} text={text:?}",
                    self.active_view
                );
            }
            // The foot of Agent workflows' cards (jobs, audit, safety and
            // history) where the web page ends, above desktop's Phone access.
            "automations-grid-bottom" => {
                let scroll = &self.automations_scroll;
                let grid = if self.automation_error.is_some() {
                    3
                } else {
                    2
                };
                // From the page's foot, back up past Phone access and its gap.
                if let Some(mobile) = scroll.bounds_for_item(grid + 1) {
                    let max = scroll.max_offset();
                    let up = mobile.size.height + px(12.0);
                    scroll.set_offset(point(px(0.0), -(max.y - up).max(px(0.0))));
                    cx.notify();
                }
            }
            "mcp-off" => {
                self.activate_sidebar_view(AppView::Automations, window, cx);
                self.automations_scroll.scroll_to_top_of_item(1);
            }
            #[cfg(unix)]
            "mcp-copied" => {
                self.activate_sidebar_view(AppView::Automations, window, cx);
                self.smoke_mcp_copied(cx);
                self.automations_scroll.scroll_to_top_of_item(1);
            }
            #[cfg(unix)]
            "mcp-undo-requested" => {
                self.activate_sidebar_view(AppView::Automations, window, cx);
                self.smoke_mcp_undo_requested(cx);
                self.automations_scroll.scroll_to_top_of_item(1);
            }
            #[cfg(unix)]
            "mcp-connected" => {
                self.activate_sidebar_view(AppView::Automations, window, cx);
                self.enable_mcp_bridge(cx);
                // The panel is the page's second item, after the crew. Repeat
                // the scene (`mcp-connected;mcp-connected`) once the crew has
                // loaded, since that moves the panel down.
                self.automations_scroll.scroll_to_top_of_item(1);
            }
            "search-query" => {
                self.activate_sidebar_view(AppView::Search, window, cx);
                self.search_query_input
                    .update(cx, |input, cx| input.set_value("fix", window, cx));
                self.meetings_hub
                    .update(cx, |hub, cx| hub.ensure_search_cache(cx));
            }
            "event-picker" => {
                self.open_event_create(None, window, cx);
                self.toggle_date_picker(date_picker_ui::DatePickerTarget::EventStart, window, cx);
            }
            "settings" => self.open_settings(&OpenSettings, window, cx),
            // Today's own Plan my day, Apply plan and Undo (TODAY-01), and
            // what they left: the plan, its Undo receipt, the focus blocks
            // and the tasks' links.
            "today-plan" => self.draft_desktop_day_plan(cx),
            "today-apply" => self.apply_desktop_day_plan(cx),
            "today-undo" => self.undo_desktop_day_plan(cx),
            "today-report" => {
                let blocks = self.calendar_events.len();
                let links: Vec<(String, usize)> = self
                    .work_items
                    .iter()
                    .map(|task| (task.title.clone(), task.planning.time_block_event_ids.len()))
                    .collect();
                eprintln!(
                    "BRUHS_DESKTOP_SMOKE today proposal={:?} applied={} events={blocks} links={links:?} error={:?} busy={} save={:?}",
                    self.day_plan_proposal.as_ref().map(|plan| plan.blocks.len()),
                    self.day_plan_undo.is_some(),
                    self.day_plan_error,
                    self.day_plan_busy(),
                    self.save_status,
                );
            }
            // Inbox (INBOX-06, INBOX-11): the list, the import form, a reply
            // draft saved as Save draft does, the delete dialog, and focus
            // in the search field or the reply box.
            "inbox" => self.open_inbox_view(window, cx),
            "inbox-import" => self.start_inbox_import(window, cx),
            "inbox-delete" => self.request_record_delete(window, cx),
            "inbox-focus-search" => {
                let input = self.inbox_search_input.clone();
                input.update(cx, |input, cx| input.focus(window, cx));
            }
            "inbox-focus-reply" => {
                let input = self.inbox_reply_input.clone();
                input.update(cx, |input, cx| input.focus(window, cx));
            }
            scene if scene.starts_with("inbox-draft:") => {
                let text = scene.trim_start_matches("inbox-draft:").to_string();
                self.inbox_reply_input
                    .update(cx, |input, cx| input.set_value(text, window, cx));
                self.save_inbox_draft(cx);
            }
            "inbox-report" => {
                let draft = match &self.active_view {
                    AppView::ConversationDetail { conversation_id } => self
                        .inbox_conversations
                        .iter()
                        .find(|conversation| conversation.id == *conversation_id)
                        .map(|conversation| conversation.draft_body().to_string()),
                    _ => None,
                };
                eprintln!(
                    "BRUHS_DESKTOP_SMOKE inbox view={:?} draft={draft:?} reply={:?} save={:?}",
                    self.active_view,
                    self.inbox_reply_input.read(cx).value(),
                    self.save_status,
                );
            }
            // `key:shift-tab`: key presses delivered through gpui's own
            // dispatch, inside this window (no OS event), as if typed: Tab
            // moves focus and shows its ring, Enter on a button presses it
            // (gpui clicks on the key up).
            scene if scene.starts_with("key:") => {
                let keys: Vec<Keystroke> = scene
                    .trim_start_matches("key:")
                    .split_whitespace()
                    .filter_map(|key| Keystroke::parse_with_os(key, keybindings_ui::host_os()).ok())
                    .collect();
                window.defer(cx, move |window, cx| press_keys(keys, window, cx));
            }
            // The crew: a bruh's page on its Chat or Settings tab, and the
            // New bruh dialog (web-vs-desktop crew comparison).
            "bruh-create" => self.open_create_bruh_dialog(window, cx),
            // A room's page, New room and a room's settings (web-vs-desktop
            // room page and dialog).
            "room-new" => self.open_room_dialog(None, window, cx),
            scene if scene.starts_with("room:") => {
                let room_id = scene.trim_start_matches("room:").to_string();
                self.activate_sidebar_view(AppView::Room { room_id }, window, cx);
            }
            // Text typed into the open room's composer (the `@` list).
            scene if scene.starts_with("room-draft:") => {
                let text = scene.trim_start_matches("room-draft:").to_string();
                self.reactive_automations
                    .update(cx, |panel, cx| panel.smoke_room_draft(&text, window, cx));
            }
            scene if scene.starts_with("room-settings:") => {
                let room_id = scene.trim_start_matches("room-settings:").to_string();
                self.open_room_dialog(Some(room_id), window, cx);
            }
            scene if scene.starts_with("bruh:") || scene.starts_with("bruh-settings:") => {
                let (kind, agent_id) = scene.split_once(':').unwrap_or_default();
                self.activate_sidebar_view(
                    AppView::Bruh {
                        agent_id: agent_id.to_string(),
                    },
                    window,
                    cx,
                );
                if kind == "bruh-settings" {
                    self.reactive_automations
                        .update(cx, |panel, cx| panel.smoke_show_bruh_settings(agent_id, cx));
                }
            }
            "notice" => self
                .meetings_hub
                .update(cx, |hub, cx| hub.smoke_post_notice(cx)),
            "export-reply" => {
                let Some(chat) = self.smoke_active_chat() else {
                    return false;
                };
                return chat.update(cx, |chat, cx| chat.smoke_open_export_panel(cx));
            }
            // The active card's menu, or the bottom card's (opens upward).
            "thread-menu" | "thread-menu-last" => {
                let id = if scene == "thread-menu" {
                    self.active_thread_id.clone()
                } else {
                    self.smoke_bottom_card_id(cx)
                };
                let Some(id) = id else {
                    return false;
                };
                self.toggle_thread_menu(&id, window, cx);
            }
            // The composer options panel, open at its top.
            "composer-options" => {
                let Some(chat) = self.smoke_active_chat() else {
                    return false;
                };
                chat.update(cx, |chat, cx| chat.smoke_open_composer_options(window, cx));
            }
            // The thread menu's "Custom…" snooze dialog (web "Snooze until").
            "snooze-custom" => {
                let Some(id) = self.active_thread_id.clone() else {
                    return false;
                };
                self.open_snooze_custom(&id, window, cx);
            }
            "settings-roles" => {
                // The role editor with a role open, scrolled into view, for
                // a comparison with the web's. Use a scratch state file: the
                // fixture role is saved.
                self.open_settings(&OpenSettings, window, cx);
                if self.roles.is_empty() {
                    self.smoke_add_role(
                        "Writer",
                        "claude-sonnet-4-6",
                        "End every reply with the token ROLE-MARK-2885.",
                        cx,
                    );
                }
                let first = self.roles.roles.first().map(|role| role.id.clone());
                self.select_role(first, window, cx);
                let down = std::env::var("BRUHS_DESKTOP_SMOKE_SCROLL_PX")
                    .ok()
                    .and_then(|value| value.parse::<f32>().ok())
                    .unwrap_or(0.0);
                self.settings_scroll.set_offset(point(px(0.0), px(-down)));
            }
            scene if scene.starts_with("settings-") => {
                let Some(section) = scene
                    .strip_prefix("settings-")
                    .and_then(SettingsSection::from_id)
                else {
                    return false;
                };
                self.open_settings(&OpenSettings, window, cx);
                self.set_settings_section(section, cx);
            }
            "transcript"
            | "transcript-diff"
            | "transcript-tasks"
            | "transcript-preview"
            | "transcript-rich"
            | "transcript-rich-save" => {
                // A throwaway thread, so the fixture never lands in a real one.
                self.smoke_previous_active_id = self.active_thread_id.clone();
                self.new_general_thread(window, cx);
                self.smoke_thread_id = self.active_thread_id.clone();
                if let Some(chat) = self.active_thread().map(|thread| thread.chat.clone()) {
                    chat.update(cx, |chat, cx| {
                        if scene.starts_with("transcript-rich") {
                            chat.smoke_seed_rich_response(cx);
                        } else {
                            chat.smoke_seed_transcript(cx);
                        }
                    });
                }
                match scene {
                    "transcript-diff" => self.toggle_diff(&ToggleDiff, window, cx),
                    "transcript-tasks" => self.toggle_tasks(&ToggleTasks, window, cx),
                    "transcript-preview" => self.toggle_preview(&TogglePreview, window, cx),
                    _ => {}
                }
                if scene == "transcript-rich-save" {
                    self.persist_and_notify(cx);
                    self.smoke_thread_id = None;
                }
            }
            "transcript-rich-reopen" => {
                let Some(id) = self
                    .threads
                    .iter()
                    .find(|thread| {
                        thread.chat.read(cx).messages.iter().any(|message| {
                            message.content.contains("tag:'bruhs-rich-response-smoke'")
                        })
                    })
                    .map(|thread| thread.id.clone())
                else {
                    return false;
                };
                self.select_thread_by_id(&id, cx);
                self.active_view = AppView::Chat;
                self.smoke_thread_id = Some(id);
                cx.notify();
            }
            _ => return false,
        }
        true
    }

    /// `BRUHS_DESKTOP_TERMINAL_SMOKE`: ⌘J without going through the keymap.
    pub fn smoke_toggle_terminal(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.toggle_terminal(&ToggleTerminal, window, cx);
    }

    /// `BRUHS_DESKTOP_TERMINAL_SMOKE`: show the Preview surface on `url`.
    pub fn preview_smoke_go(&mut self, url: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.active_view != AppView::Chat {
            self.open_chat_view(&OpenChatView, window, cx);
        }
        self.set_preview_panel_visible(true, cx);
        self.preview_panel
            .update(cx, |panel, cx| panel.go_to(url, window, cx));
    }

    /// `BRUHS_DESKTOP_TERMINAL_SMOKE`: force the native page view to exist.
    pub fn preview_smoke_place(
        &mut self,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) -> Result<(), String> {
        self.preview_panel
            .update(cx, |panel, cx| panel.smoke_place(window, cx))
    }

    /// One-line diagnostic for `BRUHS_DESKTOP_TERMINAL_SMOKE`: the preview's
    /// visibility, URL, whether the native view exists, and any host error.
    pub fn preview_smoke_report(&self, cx: &App) -> String {
        let panel = self.preview_panel.read(cx);
        format!(
            "visible={} shown={} attempts={} view={:?} overlay={} url={:?} live={} error={:?}",
            self.preview_panel_visible,
            panel.is_shown(),
            panel.place_attempts(),
            self.active_view,
            self.overlay_open(),
            panel.url(),
            panel.host_is_live(),
            panel.host_error()
        )
    }

    /// One-line diagnostic for `BRUHS_DESKTOP_TERMINAL_SMOKE`: the active
    /// session's status, grid size, cursor and non-empty rows.
    pub fn terminal_smoke_report(&self, cx: &App) -> String {
        let panel = self.terminal_panel.read(cx);
        let Some(view) = panel.active_session(cx) else {
            return format!(
                "no terminal session (drawer_open={} view={:?} sessions={} error={:?})",
                self.terminal_drawer_open,
                self.active_view,
                panel.has_sessions(),
                panel.last_error()
            );
        };
        let view = view.read(cx);
        let (rows, cursor) = view.snapshot();
        let lines: Vec<&str> = rows
            .iter()
            .filter(|row| !row.text.is_empty())
            .map(|row| row.text.as_str())
            .collect();
        format!(
            "id={} status={} size={}x{} cursor={:?} rows={:?}",
            view.info.id,
            view.status().label(),
            view.size().cols,
            view.size().rows,
            cursor.map(|c| (c.row, c.col)),
            lines
        )
    }
}
