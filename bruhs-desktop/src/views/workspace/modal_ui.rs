//! The open dialogs (a form dialog and, above it, the command bar or project
//! picker): which they are, closing them, keeping Tab inside, and the shared
//! modal shell.

use super::*;
pub(super) const DESKTOP_MODAL_FOCUS_TRAP_KEY: &str = "tab";
pub(super) const DESKTOP_MODAL_FOCUS_START_TAB_INDEX: isize = 0;
pub(super) const DESKTOP_MODAL_FOCUS_END_TAB_INDEX: isize = 999;

pub(super) fn dialog_max_height(window: &Window) -> Pixels {
    (window.viewport_size().height
        - px(DESKTOP_DIALOG_TOP_PADDING_PX + DESKTOP_DIALOG_BOTTOM_GAP_PX))
    .max(px(0.0))
}

impl Overlay {
    /// Every dialog but Settings dims and blurs the workspace behind it.
    pub(super) fn blurs_backdrop(&self) -> bool {
        !matches!(self, Self::Settings { .. })
    }
}
pub(super) const PROJECT_DELETE_LABEL: &str = "Delete project";
pub(super) const PROJECT_DELETE_CANCEL_LABEL: &str = "Cancel";
pub(super) const PROJECT_DELETE_CONFIRM_LABEL: &str = "Delete";
pub(super) const PROJECT_DELETE_BODY_PADDING_PX: f32 = 16.0;
pub(super) const PROJECT_DELETE_BODY_GAP_PX: f32 = 16.0;
pub(super) const PROJECT_DELETE_ACTION_GAP_PX: f32 = 8.0;
pub(super) const PROJECT_DELETE_PROMPT_TEXT_SIZE_PX: f32 = 14.0;

impl Workspace {
    fn cancel_picker_tasks(&mut self) {
        self.project_picker_task = None;
        self.import_picker.cancel();
    }

    pub(super) fn close_transient_surfaces(&mut self) {
        self.cancel_picker_tasks();
        self.date_picker = None;
        if self.event_create_visible() {
            self.selected_calendar_event_id = None;
            self.event_create_error = None;
        }
        self.overlay = None;
        self.top_layer = None;
        self.capture_edit_id = None;
        self.capture_error = None;
        self.command_selected_index = 0;
        self.palette_mode = PaletteMode::Commands;
        self.task_create_error = None;
        self.task_detail_menu_open = None;
    }

    pub(super) fn focus_first_modal_tab_stop(
        &self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let start = self.modal_start_focus_handle.clone();
        cx.defer_in(window, move |_this, window, cx| {
            start.focus(window, cx);
            window.focus_next(cx);
            if !start.contains_focused(window, cx) {
                start.focus(window, cx);
            }
        });
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

    pub(super) fn step_modal_focus(
        &self,
        reverse: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if reverse {
            window.focus_prev(cx);
            if !self.modal_start_focus_handle.contains_focused(window, cx) {
                self.modal_end_focus_handle.focus(window, cx);
                window.focus_prev(cx);
            }
        } else {
            window.focus_next(cx);
            if !self.modal_start_focus_handle.contains_focused(window, cx) {
                self.modal_start_focus_handle.focus(window, cx);
                window.focus_next(cx);
            }
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
        self.cancel_picker_tasks();
        self.modal_scroll.set_offset(Point::default());
        self.top_layer = None;
        self.overlay = Some(overlay);
    }

    /// Opens the command bar or project picker above any form dialog. Settings
    /// is not a form behind which either belongs, so it closes.
    pub(super) fn open_top_layer(&mut self, top_layer: TopLayer) {
        self.cancel_picker_tasks();
        self.dismiss_overlay(|overlay| matches!(overlay, Overlay::Settings { .. }));
        self.top_layer = Some(top_layer);
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

    pub(super) fn settings_open_dropdown(&self) -> Option<&'static str> {
        match self.overlay {
            Some(Overlay::Settings { open_dropdown }) => open_dropdown,
            _ => None,
        }
    }

    pub(super) fn settings_listbox(&self, dropdown: &'static str) -> Listbox {
        Listbox::open_if(self.settings_open_dropdown() == Some(dropdown))
    }

    pub(super) fn set_settings_open_dropdown(&mut self, dropdown: Option<&'static str>) {
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
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let had_overlay = self.overlay_open();
        self.close_transient_surfaces();
        if let Some(chat) = self.active_thread().map(|thread| thread.chat.clone()) {
            chat.update(cx, |chat, cx| chat.close_composer_menus(cx));
        }
        // Nothing was open, so the focused control keeps Escape: a speaker
        // rename, a text field, or the terminal.
        if !had_overlay {
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
                "Delete conversation",
                "This permanently deletes the local conversation and its messages.",
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
                            .on_click(cx.listener(|this, _ev, _window, cx| {
                                this.cancel_project_delete(cx);
                            }))
                            .on_key_down(cx.listener(
                                |this, event: &KeyDownEvent, window, cx| {
                                    if is_web_button_activation_key(event.keystroke.key.as_str()) {
                                        window.prevent_default();
                                        cx.stop_propagation();
                                        this.cancel_project_delete(cx);
                                    }
                                },
                            )),
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
                            .on_click(cx.listener(|this, _ev, _window, cx| {
                                this.confirm_project_delete(cx);
                            }))
                            .on_key_down(cx.listener(
                                |this, event: &KeyDownEvent, window, cx| {
                                    if is_web_button_activation_key(event.keystroke.key.as_str()) {
                                        window.prevent_default();
                                        cx.stop_propagation();
                                        this.confirm_project_delete(cx);
                                    }
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
        title: &'static str,
        content: Div,
        footer: Option<Div>,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        div()
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .left_0()
            .occlude()
            .id("desktop-modal-overlay")
            .bg(theme::longan_950().opacity(DESKTOP_MODAL_OVERLAY_OPACITY))
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
                    .id("desktop-modal-dialog")
                    .role(Role::Dialog)
                    .aria_label(title)
                    .child(self.blur_scrim.backdrop())
                    .w(px(DESKTOP_DIALOG_MD_WIDTH_PX))
                    .max_w(relative(DESKTOP_DIALOG_MAX_WIDTH_RATIO))
                    .max_h(dialog_max_height(window))
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
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .debug_selector(|| "desktop-modal-close".to_string())
                                .aria_label("Close dialog")
                                .child(
                                    svg()
                                        .path(assets::CLOSE_ICON_PATH)
                                        .size(px(14.0))
                                        .text_color(theme::rambutan_400()),
                                )
                                .tab_index(0)
                                .role(Role::Button)
                                .focus_visible(|el| el.border_color(theme::persimmon_400()))
                                .on_click(cx.listener(|this, _ev, _window, cx| {
                                    this.close_transient_surfaces();
                                    cx.notify();
                                }))
                                .on_key_down(cx.listener(
                                    |this, event: &KeyDownEvent, window, cx| {
                                        if is_web_button_activation_key(
                                            event.keystroke.key.as_str(),
                                        ) {
                                            window.prevent_default();
                                            cx.stop_propagation();
                                            this.close_transient_surfaces();
                                            cx.notify();
                                        }
                                    },
                                )),
                            ),
                    )
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
