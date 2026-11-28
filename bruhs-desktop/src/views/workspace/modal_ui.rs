//! The open dialogs (a form dialog and, above it, the command bar or project
//! picker): which they are, closing them, keeping Tab inside, and the shared
//! modal shell.

use super::*;
use bruhs_ui::press::on_press_view;
use bruhs_ui::zoom::px;
pub(super) const DESKTOP_MODAL_FOCUS_TRAP_KEY: &str = "tab";
pub(super) const DESKTOP_MODAL_FOCUS_START_TAB_INDEX: isize = 0;
pub(super) const DESKTOP_MODAL_FOCUS_END_TAB_INDEX: isize = 999;

/// Tallest a centered dialog may grow: the window less a gap above and below.
pub(super) fn dialog_max_height(window: &Window) -> Pixels {
    (window.viewport_size().height - px(DESKTOP_DIALOG_EDGE_GAP_PX * 2.0)).max(px(0.0))
}

/// Tallest a top-anchored picker may grow below its anchor.
pub(super) fn picker_max_height(window: &Window) -> Pixels {
    (window.viewport_size().height
        - px(DESKTOP_DIALOG_TOP_PADDING_PX + DESKTOP_DIALOG_BOTTOM_GAP_PX))
    .max(px(0.0))
}

/// Height a form dialog takes: its preferred height, or what the window
/// allows around a centered dialog.
pub(super) fn form_dialog_height(window: &Window) -> f32 {
    (bruhs_ui::zoom::css_px(window.viewport_size().height) - DESKTOP_DIALOG_EDGE_GAP_PX * 2.0)
        .clamp(
            DESKTOP_DIALOG_MIN_AVAILABLE_HEIGHT_PX,
            FORM_DIALOG_PREFERRED_HEIGHT_PX,
        )
}

pub(super) const PROJECT_DELETE_LABEL: &str = "Delete project";
pub(super) const PROJECT_DELETE_CANCEL_LABEL: &str = "Cancel";
pub(super) const PROJECT_DELETE_CONFIRM_LABEL: &str = "Delete";
pub(super) const PROJECT_DELETE_BODY_PADDING_PX: f32 = 16.0;
pub(super) const PROJECT_DELETE_BODY_GAP_PX: f32 = 16.0;
pub(super) const PROJECT_DELETE_ACTION_GAP_PX: f32 = 8.0;
pub(super) const PROJECT_DELETE_PROMPT_TEXT_SIZE_PX: f32 = 14.0;
pub(super) const TASK_DELETE_LABEL: &str = "Delete task";

impl Workspace {
    fn cancel_picker_tasks(&mut self) {
        self.clone_editor.generation = self.clone_editor.generation.wrapping_add(1);
        self.clone_editor.open = false;
        self.clone_editor.linked_prompt = None;
        self.project_picker_task = None;
        self.import_picker.cancel();
    }

    pub(super) fn close_transient_surfaces(&mut self) {
        self.license_browser = None;
        self.settings_dropdown_trigger = None;
        self.details.close();
        self.provider_editor = None;
        self.usage_hubs.close();
        self.reset_generator_dialog();
        if self.settings_visible() {
            self.microphone_test.clear();
            self.voice_cloud.dismiss();
            self.voice_models.close();
        }
        self.cancel_picker_tasks();
        self.action_editor.open = false;
        self.keybinding_recording = None;
        self.keybinding_draft = None;
        self.linked_keybindings.dismiss_form();
        self.linked_projects.dismiss();
        self.keybinding_when_open = None;
        self.date_picker = None;
        if self.event_create_visible() {
            self.selected_calendar_event_id = None;
            self.event_create_error = None;
        }
        self.theme_navigation = Default::default();
        self.overlay = None;
        self.leave_import_dialog();
        self.top_layer = None;
        self.capture_edit_id = None;
        self.capture_error = None;
        self.command_selected_index = 0;
        self.palette_mode = PaletteMode::Commands;
        self.task_create_error = None;
        self.task_detail_menu_open = None;
        self.inbox_person_menu_open = false;
        self.close_sidebar_menus();
    }

    /// Focus for a dialog that just opened, as the web `Dialog` places it:
    /// its first control, or the dialog itself when it has none. Web
    /// dialogs have no close button, so the shared shell's is skipped. The
    /// dialog takes focus at once, so Escape and Tab reach it; its first
    /// control follows once the dialog has drawn
    /// ([`Self::schedule_modal_first_control`]), since Tab order only knows
    /// drawn elements.
    pub(super) fn focus_first_modal_tab_stop(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.focus_modal_dialog(window, cx);
        self.modal_focus.first_control_pending = true;
    }

    /// Focus the open dialog itself. Settings keeps it there: on the web it
    /// is a page that focuses nothing until the first Tab.
    pub(super) fn focus_modal_dialog(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let start = self.modal_start_focus_handle.clone();
        cx.defer_in(window, move |_this, window, cx| start.focus(window, cx));
    }

    /// Focus the shared shell's close button: Tasks' Projects, whose web
    /// dialog begins with its own Close.
    pub(super) fn focus_modal_close_button(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let close = self.modal_focus.close.clone();
        cx.defer_in(window, move |_this, window, cx| close.focus(window, cx));
    }

    /// Render's half of [`Self::focus_first_modal_tab_stop`]: a step that
    /// runs right after the frame that draws the new dialog.
    pub(super) fn schedule_modal_first_control(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !std::mem::take(&mut self.modal_focus.first_control_pending)
            || (self.overlay.is_none() && self.top_layer.is_none())
        {
            return;
        }
        cx.defer_in(window, |this, window, cx| {
            this.focus_modal_first_control(window, cx);
        });
    }

    fn focus_modal_first_control(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let dialog = self.modal_start_focus_handle.clone();
        // A click or a key moved focus meanwhile, or the dialog closed.
        if !dialog.is_focused(window) {
            return;
        }
        // Begin after a form dialog's header and its close button. The
        // command bar and pickers draw above any form dialog and have no
        // such header.
        if self.top_layer.is_none() {
            let content = self.modal_focus.content.clone();
            content.focus(window, cx);
            window.focus_next(cx);
            if dialog.contains_focused(window, cx) && !content.is_focused(window) {
                return;
            }
        }
        dialog.focus(window, cx);
        window.focus_next(cx);
        if !dialog.contains_focused(window, cx) {
            dialog.focus(window, cx);
        }
    }

    pub(super) fn trap_modal_tab_focus(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if event.keystroke.key.as_str() != DESKTOP_MODAL_FOCUS_TRAP_KEY {
            return;
        }

        window.prevent_default();
        cx.stop_propagation();
        self.step_modal_focus(event.keystroke.modifiers.shift, window, cx);
    }

    pub(super) fn scheduled_task_dialog_visible(&self, cx: &App) -> bool {
        self.top_layer.is_none()
            && self.settings_visible()
            && self.settings_section == SettingsSection::ScheduledTasks
            && self.scheduled_panel().read(cx).scheduled_editor_open()
    }

    pub(super) fn secret_provider_dialog_visible(&self, cx: &App) -> bool {
        self.top_layer.is_none()
            && self.settings_visible()
            && self.settings_section == SettingsSection::Vault
            && self
                .vault_settings
                .read(cx)
                .secret_sources()
                .read(cx)
                .dialog_open()
    }

    pub(super) fn active_modal_focus_bounds(&self, cx: &App) -> (FocusHandle, FocusHandle) {
        if let Some(bounds) = self.usage_hubs.bounds() {
            return bounds;
        }
        if let Some(editor) = &self.provider_editor {
            return (editor.start.clone(), editor.end.clone());
        }
        if self.secret_provider_dialog_visible(cx) {
            return self
                .vault_settings
                .read(cx)
                .secret_sources()
                .read(cx)
                .focus_bounds();
        }
        if let Some(bounds) = self.generators.focus_bounds() {
            return bounds;
        }
        if self.scheduled_task_dialog_visible(cx) {
            self.scheduled_panel().read(cx).scheduled_focus_bounds()
        } else {
            (
                self.modal_start_focus_handle.clone(),
                self.modal_end_focus_handle.clone(),
            )
        }
    }

    pub(super) fn modal_contains_focus(&self, window: &Window, cx: &App) -> bool {
        self.active_modal_focus_bounds(cx)
            .0
            .contains_focused(window, cx)
            || self.accent_panel_has_focus(window, cx)
            || self.theme_picker_has_focus(window, cx)
    }

    pub(super) fn step_modal_focus(
        &mut self,
        reverse: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.step_theme_picker_focus(reverse, window, cx)
            || self.step_accent_focus(reverse, window, cx)
        {
            return;
        }
        if self.scheduled_task_dialog_visible(cx) {
            self.scheduled_panel()
                .clone()
                .update(cx, |panel, cx| panel.close_task_picker_for_tab(window, cx));
        }
        if self.settings_visible() && self.settings_section == SettingsSection::ScheduledTasks {
            self.reactive_automations
                .update(cx, |panel, cx| panel.dismiss_task_menu(window, cx));
        }
        let (start, end) = self.active_modal_focus_bounds(cx);
        if reverse {
            window.focus_prev(cx);
            if !start.contains_focused(window, cx) {
                end.focus(window, cx);
                window.focus_prev(cx);
            }
        } else {
            window.focus_next(cx);
            if !start.contains_focused(window, cx) {
                start.focus(window, cx);
                window.focus_next(cx);
            }
        }
        if self.scheduled_task_dialog_visible(cx) {
            self.reactive_automations.update(cx, |panel, cx| {
                panel.dismiss_unfocused_schedule_picker(window, cx)
            });
        }
    }

    pub(super) fn render_modal_focus_sentinel(&self) -> AnyElement {
        div()
            .id("desktop-modal-focus-end")
            .size(px(0.0))
            .overflow_hidden()
            .track_focus(&self.modal_end_focus_handle)
            .into_any_element()
    }

    /// Where a dialog's own controls begin, after its header and close
    /// button: the web dialog's first focusable comes after this.
    pub(super) fn render_modal_content_start(&self) -> AnyElement {
        div()
            .id("desktop-modal-content-start")
            .size(px(0.0))
            .overflow_hidden()
            .track_focus(&self.modal_focus.content)
            .into_any_element()
    }

    /// Anything `close_transient_surfaces` would close: a form dialog, the
    /// command bar or project picker, the task detail's open menu, or a date
    /// picker.
    pub(super) fn overlay_open(&self) -> bool {
        self.overlay.is_some()
            || self.top_layer.is_some()
            || self.task_detail_menu_open.is_some()
            || self.date_picker.is_some()
    }

    /// Opens a form dialog, replacing any other and closing the command bar
    /// or project picker it was started from.
    pub(super) fn open_overlay(&mut self, overlay: Overlay) {
        self.details.close();
        self.cancel_picker_tasks();
        self.close_sidebar_menus();
        self.modal_scroll.set_offset(Point::default());
        self.leave_import_dialog();
        self.top_layer = None;
        self.theme_navigation.collapse();
        self.overlay = Some(overlay);
    }

    /// Opens the command bar or project picker above any form dialog. Settings
    /// is not a form behind which either belongs, so it closes.
    pub(super) fn open_top_layer(&mut self, top_layer: TopLayer) {
        self.cancel_picker_tasks();
        self.close_sidebar_menus();
        self.dismiss_overlay(|overlay| {
            matches!(overlay, Overlay::Settings { .. } | Overlay::Welcome)
        });
        self.leave_import_dialog();
        self.top_layer = Some(top_layer);
    }

    /// A dialog or the command bar takes over: sidebar menus (and their
    /// backdrops) must not stay above it.
    fn close_sidebar_menus(&mut self) {
        self.thread_menu_id = None;
        self.sidebar_apps_open = false;
    }

    /// Closes the dialog if `is_open` says it is the one showing.
    pub(super) fn dismiss_overlay(&mut self, is_open: impl Fn(&Overlay) -> bool) {
        if self.overlay.as_ref().is_some_and(is_open) {
            self.overlay = None;
        }
    }

    pub(super) fn command_bar_visible(&self) -> bool {
        matches!(self.top_layer, Some(TopLayer::CommandBar))
    }

    pub(super) fn settings_visible(&self) -> bool {
        matches!(self.overlay, Some(Overlay::Settings { .. }))
    }

    pub(super) fn project_picker_visible(&self) -> bool {
        matches!(self.top_layer, Some(TopLayer::ProjectPicker { .. }))
    }

    #[cfg(test)]
    pub(super) fn task_create_visible(&self) -> bool {
        matches!(self.overlay, Some(Overlay::TaskCreate { .. }))
    }

    pub(super) fn event_create_visible(&self) -> bool {
        matches!(self.overlay, Some(Overlay::EventCreate { .. }))
    }

    pub(super) fn project_delete_confirm_id(&self) -> Option<&str> {
        match &self.overlay {
            Some(Overlay::ProjectDeleteConfirm { project_id }) => Some(project_id),
            _ => None,
        }
    }

    pub(super) fn project_picker_for_terminal(&self) -> bool {
        matches!(
            self.top_layer,
            Some(TopLayer::ProjectPicker {
                for_terminal: true,
                ..
            })
        )
    }

    pub(super) fn settings_open_dropdown(&self) -> Option<&str> {
        match &self.overlay {
            Some(Overlay::Settings { open_dropdown }) => open_dropdown.as_deref(),
            _ => None,
        }
    }

    pub(super) fn settings_listbox(&self, dropdown: &str) -> Listbox {
        if self.settings_open_dropdown() == Some(dropdown) {
            Listbox::Popover
        } else {
            Listbox::Closed
        }
    }

    pub(super) fn set_settings_open_dropdown(&mut self, dropdown: Option<SharedString>) {
        self.settings_dropdown_trigger = None;
        if let Some(Overlay::Settings { open_dropdown }) = &mut self.overlay {
            *open_dropdown = dropdown;
        }
    }

    pub(super) fn task_create_menu_open(&self) -> Option<TaskCreateMenu> {
        match self.overlay {
            Some(Overlay::TaskCreate { menu }) => menu,
            _ => None,
        }
    }

    pub(super) fn set_task_create_menu_open(&mut self, open: Option<TaskCreateMenu>) {
        if let Some(Overlay::TaskCreate { menu }) = &mut self.overlay {
            *menu = open;
        }
    }

    pub(super) fn event_create_color_menu_open(&self) -> bool {
        matches!(
            self.overlay,
            Some(Overlay::EventCreate {
                color_menu_open: true
            })
        )
    }

    pub(super) fn set_event_create_color_menu_open(&mut self, open: bool) {
        if let Some(Overlay::EventCreate { color_menu_open }) = &mut self.overlay {
            *color_menu_open = open;
        }
    }

    pub(super) fn close_overlays(
        &mut self,
        _: &CloseOverlays,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.cancel_sidebar_rename(window, cx) {
            cx.stop_propagation();
            return;
        }
        if self.escape_usage(window, cx) {
            cx.stop_propagation();
            return;
        }
        if self.settings_visible()
            && self.settings_section == SettingsSection::Keyboard
            && self.dismiss_binding_editor(window, cx)
        {
            cx.stop_propagation();
            return;
        }
        if self.top_layer.is_none() && self.stop_picking_theme_role(cx) {
            cx.stop_propagation();
            return;
        }
        if self.settings_visible()
            && self.license_browser.as_ref().is_some_and(|browser| {
                browser.update(cx, |panel, cx| panel.escape_search(window, cx))
            })
        {
            cx.stop_propagation();
            return;
        }
        if self.settings_visible() && self.usage_hubs.bounds().is_some() {
            self.dismiss_hub(window, cx);
            cx.stop_propagation();
            return;
        }
        if self.settings_visible() && self.provider_editor.is_some() {
            self.dismiss_provider_editor(window, cx);
            cx.stop_propagation();
            return;
        }
        if self.secret_provider_dialog_visible(cx) {
            self.vault_settings
                .read(cx)
                .secret_sources()
                .update(cx, |view, cx| view.dismiss(cx));
            cx.stop_propagation();
            return;
        }
        if self.settings_visible() && self.generators.focus_bounds().is_some() {
            if self.settings_open_dropdown().is_some() {
                self.set_settings_open_dropdown(None);
            } else if self.dismiss_generator() {
                self.focus_modal_dialog(window, cx);
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.settings_visible()
            && self.settings_section == SettingsSection::Worktrees
            && self.retention_editor.take().is_some()
        {
            self.focus_modal_dialog(window, cx);
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.settings_visible()
            && self.settings_section == SettingsSection::ScheduledTasks
            && self
                .reactive_automations
                .update(cx, |panel, cx| panel.dismiss_task_menu(window, cx))
        {
            cx.stop_propagation();
            return;
        }
        if self.scheduled_task_dialog_visible(cx) {
            self.scheduled_panel()
                .clone()
                .update(cx, |panel, cx| panel.close_scheduled_overlay(window, cx));
            cx.stop_propagation();
            return;
        }
        if self.top_layer.is_none() && self.settings_open_dropdown().is_some() {
            let trigger = self.settings_dropdown_trigger.take();
            self.set_settings_open_dropdown(None);
            if let Some(trigger) = trigger {
                trigger.focus(window, cx);
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.dismiss_thread_details(window, cx) || self.clear_focused_settings_query(window, cx)
        {
            cx.stop_propagation();
            return;
        }
        if self.top_layer.is_none()
            && (self.dismiss_theme_fan(window, cx) || self.dismiss_community_sort(window, cx))
        {
            cx.stop_propagation();
            return;
        }
        if self.top_layer.is_none() && self.cancel_theme_color_picker(window, cx) {
            cx.stop_propagation();
            return;
        }
        if self.top_layer.is_none()
            && (self.dismiss_theme_editor(window, cx)
                || self.dismiss_restart_confirm(window, cx)
                || self.dismiss_update_confirm(window, cx))
        {
            cx.stop_propagation();
            return;
        }
        let had_overlay = self.overlay_open();
        let thread_menu = self.thread_menu_id.clone();
        let had_apps_menu = self.sidebar_apps_open;
        let had_menu = thread_menu.is_some() || had_apps_menu;
        self.close_transient_surfaces();
        // A menu returns focus to the button that opened it.
        if let Some(id) = thread_menu {
            self.focus_thread_menu_button(&id, window, cx);
        } else if had_apps_menu {
            self.sidebar_apps_trigger_focus.focus(window, cx);
        }
        if let Some(chat) = self.active_thread().map(|thread| thread.chat.clone()) {
            chat.update(cx, |chat, cx| chat.close_composer_menus(cx));
        }
        // The Inbox import form closes like the web's import dialog.
        if !had_overlay
            && !had_menu
            && self.inbox_import_visible
            && self.active_view == AppView::Inbox
        {
            self.cancel_inbox_import(window, cx);
            return;
        }
        // Nothing was open, so the focused control keeps Escape: a speaker
        // rename, a text field, or the terminal.
        if !had_overlay && !had_menu {
            cx.propagate();
        }
        cx.notify();
    }

    fn current_record_delete_target(&self) -> Option<RecordDeleteTarget> {
        match &self.active_view {
            AppView::DocumentDetail { document_id } => {
                Some(RecordDeleteTarget::Document(document_id.clone()))
            }
            AppView::ClipDetail { clip_id } => {
                Some(RecordDeleteTarget::SourceClip(clip_id.clone()))
            }
            AppView::ConversationDetail { conversation_id } => {
                Some(RecordDeleteTarget::Conversation(conversation_id.clone()))
            }
            _ => None,
        }
    }

    pub(super) fn request_record_delete(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(target) = self.current_record_delete_target() else {
            return;
        };
        // Like the web dialog, Cancel or Escape returns focus to the
        // control that asked (the record's Delete).
        if !self.modal_start_focus_handle.contains_focused(window, cx) {
            self.palette_return_focus = window
                .focused(cx)
                .map(|focus| (self.active_view.clone(), focus));
        }
        self.open_overlay(Overlay::RecordDeleteConfirm { target });
        self.focus_first_modal_tab_stop(window, cx);
        cx.notify();
    }

    pub(super) fn confirm_record_delete(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(Overlay::RecordDeleteConfirm { target }) = self.overlay.clone() else {
            return;
        };
        self.close_transient_surfaces();
        self.workspace_focus_handle.focus(window, cx);
        if self.current_record_delete_target().as_ref() == Some(&target) {
            match target {
                RecordDeleteTarget::Document(_) => self.delete_current_document(cx),
                RecordDeleteTarget::SourceClip(_) => self.delete_current_source_clip(cx),
                RecordDeleteTarget::Conversation(_) => self.delete_current_conversation(cx),
            }
        }
        cx.notify();
    }

    pub(super) fn render_record_delete_confirm(
        &self,
        target: &RecordDeleteTarget,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let (title, message) = match target {
            RecordDeleteTarget::Document(_) => (
                "Delete document",
                "This permanently deletes the document, its history and attached files.",
            ),
            RecordDeleteTarget::SourceClip(_) => (
                "Delete source",
                "This permanently deletes the source and removes its citations from your documents.",
            ),
            RecordDeleteTarget::Conversation(_) => (
                bruhs_models::DELETE_CONVERSATION_TITLE,
                bruhs_models::DELETE_CONVERSATION_DETAIL,
            ),
        };
        self.render_modal_shell(
            title,
            div()
                .p_4()
                .flex()
                .flex_col()
                .gap_4()
                .child(div().text_color(theme::rambutan_300()).child(message))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            web_button(
                                "record-delete-cancel",
                                WebButtonVariant::Ghost,
                                WebButtonSize::Small,
                                ButtonState::Enabled,
                            )
                            .debug_selector(|| "desktop-record-delete-cancel".into())
                            .aria_label("Cancel deletion")
                            .child("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.close_transient_surfaces();
                                cx.notify();
                            })),
                        )
                        .child(
                            web_button(
                                "record-delete-confirm",
                                WebButtonVariant::Destructive,
                                WebButtonSize::Small,
                                ButtonState::Enabled,
                            )
                            .debug_selector(|| "desktop-record-delete-confirm".into())
                            .aria_label("Confirm deletion")
                            .child("Delete")
                            .on_click(cx.listener(
                                |this, _, window, cx| this.confirm_record_delete(window, cx),
                            )),
                        ),
                ),
            None,
            window,
            cx,
        )
    }

    pub(super) fn render_project_delete_confirm(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let project_id = self.project_delete_confirm_id().unwrap_or("");
        let project_name = self
            .projects
            .iter()
            .find(|project| project.id == project_id)
            .map_or("this project", |project| project.name.as_str());
        let task_count = project_task_count(&self.work_items, project_id);
        let prompt = project_delete_prompt(project_name, task_count);

        self.render_modal_shell(
            PROJECT_DELETE_LABEL,
            div()
                .debug_selector(|| "desktop-project-delete-dialog".to_string())
                .p(px(PROJECT_DELETE_BODY_PADDING_PX))
                .flex()
                .flex_col()
                .gap(px(PROJECT_DELETE_BODY_GAP_PX))
                .child(
                    div()
                        .debug_selector(|| "desktop-project-delete-prompt".to_string())
                        .text_size(px(PROJECT_DELETE_PROMPT_TEXT_SIZE_PX))
                        .text_color(crate::theme::rambutan_100())
                        .child(prompt),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(px(PROJECT_DELETE_ACTION_GAP_PX))
                        .child(
                            web_button(
                                "project-delete-cancel",
                                WebButtonVariant::Ghost,
                                WebButtonSize::Small,
                                ButtonState::Enabled,
                            )
                            .aria_label(PROJECT_DELETE_CANCEL_LABEL)
                            .debug_selector(|| "desktop-project-delete-cancel".to_string())
                            .child(PROJECT_DELETE_CANCEL_LABEL)
                            .tab_index(0)
                            .role(Role::Button)
                            .focus_visible(|el| el.border_color(theme::persimmon_400()))
                            .map(|el| {
                                on_press_view(el, cx, |this, _window, cx| {
                                    this.cancel_project_delete(cx);
                                })
                            }),
                        )
                        .child(
                            web_button(
                                "project-delete-confirm",
                                WebButtonVariant::Destructive,
                                WebButtonSize::Small,
                                ButtonState::Enabled,
                            )
                            .aria_label(PROJECT_DELETE_CONFIRM_LABEL)
                            .debug_selector(|| "desktop-project-delete-confirm".to_string())
                            .child(PROJECT_DELETE_CONFIRM_LABEL)
                            .tab_index(0)
                            .role(Role::Button)
                            .focus_visible(|el| el.border_color(theme::persimmon_400()))
                            .map(|el| {
                                on_press_view(el, cx, |this, _window, cx| {
                                    this.confirm_project_delete(cx);
                                })
                            }),
                        ),
                ),
            None,
            window,
            cx,
        )
    }

    /// Web task detail's confirmation: the question, "This can't be
    /// undone." and Cancel or Delete (`bruhs_models::logic::task_detail`).
    pub(super) fn render_task_delete_confirm(
        &self,
        item_id: &str,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let title = self
            .work_items
            .iter()
            .find(|item| item.id == item_id)
            .map_or("", |item| item.title.as_str());
        self.render_modal_shell(
            TASK_DELETE_LABEL,
            div()
                .debug_selector(|| "desktop-task-delete-dialog".to_string())
                .p(px(PROJECT_DELETE_BODY_PADDING_PX))
                .flex()
                .flex_col()
                .gap(px(PROJECT_DELETE_BODY_GAP_PX))
                .child(
                    div()
                        .debug_selector(|| "desktop-task-delete-prompt".to_string())
                        .text_size(px(PROJECT_DELETE_PROMPT_TEXT_SIZE_PX))
                        .text_color(theme::rambutan_100())
                        .child(bruhs_models::logic::task_detail::delete_task_prompt(title)),
                )
                .child(
                    div()
                        .font_family(theme::UI_SMALL_FONT_FAMILY)
                        .font_weight(FontWeight::MEDIUM)
                        .text_size(px(12.0))
                        .text_color(theme::rambutan_300())
                        .child(bruhs_models::logic::task_detail::DELETE_TASK_NOTE),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(px(PROJECT_DELETE_ACTION_GAP_PX))
                        .child(
                            web_button(
                                "task-delete-cancel",
                                WebButtonVariant::Ghost,
                                WebButtonSize::Small,
                                ButtonState::Enabled,
                            )
                            .aria_label("Cancel")
                            .debug_selector(|| "desktop-task-delete-cancel".to_string())
                            .child("Cancel")
                            .on_click(cx.listener(
                                |this, _ev, window, cx| {
                                    this.cancel_task_delete(window, cx);
                                },
                            )),
                        )
                        .child(
                            web_button(
                                "task-delete-confirm",
                                WebButtonVariant::Destructive,
                                WebButtonSize::Small,
                                ButtonState::Enabled,
                            )
                            .aria_label("Delete")
                            .debug_selector(|| "desktop-task-delete-confirm".to_string())
                            .child("Delete")
                            .on_click(cx.listener(
                                |this, _ev, window, cx| {
                                    this.confirm_task_delete(window, cx);
                                },
                            )),
                        ),
                ),
            None,
            window,
            cx,
        )
    }

    pub(super) fn render_modal_shell(
        &self,
        title: impl Into<SharedString>,
        content: Div,
        footer: Option<Div>,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        self.render_modal_shell_sized(
            title,
            content,
            footer,
            DESKTOP_DIALOG_MD_WIDTH_PX,
            window,
            cx,
        )
    }

    /// `render_modal_shell` at another width (web `Dialog`'s `max_width`).
    pub(super) fn render_modal_shell_sized(
        &self,
        title: impl Into<SharedString>,
        content: Div,
        footer: Option<Div>,
        width_px: f32,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        // The header carries the dialog's one title; the content does not
        // repeat it (web: the visible heading is the only title).
        let title: SharedString = title.into();
        bruhs_ui::dialog::dialog_backdrop(
            "desktop-modal-overlay",
            bruhs_ui::dialog::DialogPlacement::Centered,
        )
        .on_click(cx.listener(|this, _ev, window, cx| {
            if this.dismiss_theme_editor(window, cx)
                || this.dismiss_restart_confirm(window, cx)
                || this.dismiss_update_confirm(window, cx)
            {
                return;
            }
            this.close_transient_surfaces();
            cx.notify();
        }))
        .child(
            bruhs_ui::dialog::dialog_card(
                "desktop-modal-dialog",
                title.clone(),
                if matches!(self.overlay, Some(Overlay::RemoveThemeVariants(_))) {
                    Role::AlertDialog
                } else {
                    Role::Dialog
                },
                &self.blur_scrim,
                cx,
            )
            .debug_selector(|| "desktop-modal-dialog".to_string())
            .w(px(width_px))
            .max_h(dialog_max_height(window))
            .track_focus(&self.modal_start_focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.trap_modal_tab_focus(event, window, cx);
            }))
            .on_click(cx.listener(|_this, _ev, _window, cx| {
                cx.stop_propagation();
            }))
            .child(
                div()
                    .flex_shrink_0()
                    .px(px(DESKTOP_MODAL_HEADER_X_PADDING_PX))
                    .py(px(DESKTOP_MODAL_HEADER_Y_PADDING_PX))
                    .border_b_1()
                    .border_color(theme::longan_800())
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .font_family(theme::HEADING_FONT_FAMILY)
                            .text_size(px(DESKTOP_MODAL_TITLE_TEXT_SIZE_PX))
                            .font_weight(FontWeight::MEDIUM)
                            .line_height(relative(1.5))
                            .text_color(theme::rambutan_100())
                            .child(title),
                    )
                    .child(
                        web_button(
                            "close-modal",
                            WebButtonVariant::Ghost,
                            WebButtonSize::IconSmall,
                            ButtonState::Enabled,
                        )
                        .debug_selector(|| "desktop-modal-close".to_string())
                        .aria_label("Close dialog")
                        .child(
                            svg()
                                .path(assets::CLOSE_ICON_PATH)
                                .size(px(theme::ICON_PX))
                                .text_color(theme::rambutan_400()),
                        )
                        .track_focus(&self.modal_focus.close)
                        .tab_index(0)
                        .role(Role::Button)
                        .focus_visible(|el| el.border_color(theme::persimmon_400()))
                        .map(|el| {
                            on_press_view(el, cx, |this, window, cx| {
                                if this.dismiss_theme_editor(window, cx)
                                    || this.dismiss_restart_confirm(window, cx)
                                    || this.dismiss_update_confirm(window, cx)
                                {
                                    return;
                                }
                                this.close_transient_surfaces();
                                cx.notify();
                            })
                        }),
                    ),
            )
            .child(self.render_modal_content_start())
            .child(
                div()
                    .id("desktop-modal-body")
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.modal_scroll)
                    .child(content)
                    .vertical_scrollbar(&self.modal_scroll),
            )
            .when_some(footer, |dialog, footer| {
                dialog.child(
                    div()
                        .id("desktop-modal-footer")
                        .debug_selector(|| "desktop-modal-footer".into())
                        .flex_shrink_0()
                        .px(px(FORM_DIALOG_SECTION_X_PADDING_PX))
                        .pb_3()
                        .child(footer),
                )
            })
            .child(self.render_modal_focus_sentinel()),
        )
        .into_any_element()
    }
}

/// Focus handles of the shared dialog shell, and whether a dialog that just
/// opened still has to move focus to its first control.
pub(super) struct ModalFocus {
    /// After a dialog's header: where its own controls begin.
    pub(super) content: FocusHandle,
    /// The shared shell's close button.
    pub(super) close: FocusHandle,
    /// Set by [`Workspace::focus_first_modal_tab_stop`], taken by render.
    pub(super) first_control_pending: bool,
    pub(super) restart_row: FocusHandle,
    pub(super) restart_sidebar: FocusHandle,
    pub(super) update_all: FocusHandle,
    pub(super) update_cancel: FocusHandle,
}

impl ModalFocus {
    pub(super) fn new(cx: &mut Context<'_, Workspace>) -> Self {
        Self {
            // a11y-exempt: a focus-handle builder, not an element; nothing draws.
            content: cx.focus_handle().tab_stop(false).tab_index(0),
            // a11y-exempt: a focus-handle builder, not an element; the close
            // button that tracks it names itself.
            close: cx.focus_handle().tab_index(0).tab_stop(true),
            first_control_pending: false,
            restart_row: cx.focus_handle().tab_stop(true),
            restart_sidebar: cx.focus_handle().tab_stop(true),
            update_all: cx.focus_handle().tab_stop(true),
            update_cancel: cx.focus_handle().tab_stop(true),
        }
    }
}

impl Workspace {
    /// Sentinels before and after a dialog's controls that keep Tab inside it.
    pub(super) fn new_modal_focus_handles(
        cx: &mut Context<'_, Self>,
    ) -> (FocusHandle, FocusHandle) {
        // a11y-exempt: a focus-handle builder, not an element; nothing draws.
        let modal_start_focus_handle = cx
            .focus_handle()
            .tab_stop(false)
            .tab_index(DESKTOP_MODAL_FOCUS_START_TAB_INDEX);
        // a11y-exempt: a focus-handle builder, not an element; nothing draws.
        let modal_end_focus_handle = cx
            .focus_handle()
            .tab_stop(false)
            .tab_index(DESKTOP_MODAL_FOCUS_END_TAB_INDEX);
        (modal_start_focus_handle, modal_end_focus_handle)
    }
}
