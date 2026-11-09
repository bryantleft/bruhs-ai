//! Launch-check hooks driven by `BRUHS_DESKTOP_*SMOKE*` environment
//! variables (see `src/smoke.rs`). Debug builds only: a release build has no
//! way to auto-drive the window.

use super::*;

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
            "terminal" => self.activate_primary_view(AppView::Terminal, window, cx),
            "command-bar" => self.focus_command_bar(&FocusCommandBar, window, cx),
            "model-picker" => self.toggle_model_picker(&ToggleModelPicker, window, cx),
            "project-picker" => self.open_project_picker(&OpenProjectPicker, window, cx),
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
            "updates" => {
                self.open_settings(&OpenSettings, window, cx);
                self.set_settings_section(SettingsSection::Updates, cx);
            }
            "search" => self.activate_sidebar_view(AppView::Search, window, cx),
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
            "transcript" | "transcript-diff" | "transcript-tasks" | "transcript-preview" => {
                // A throwaway thread, so the fixture never lands in a real one.
                self.smoke_previous_active_id = self.active_thread_id.clone();
                self.new_general_thread(window, cx);
                self.smoke_thread_id = self.active_thread_id.clone();
                if let Some(chat) = self.active_thread().map(|thread| thread.chat.clone()) {
                    chat.update(cx, |chat, cx| chat.smoke_seed_transcript(cx));
                }
                match scene {
                    "transcript-diff" => self.toggle_diff(&ToggleDiff, window, cx),
                    "transcript-tasks" => self.toggle_tasks(&ToggleTasks, window, cx),
                    "transcript-preview" => self.toggle_preview(&TogglePreview, window, cx),
                    _ => {}
                }
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
