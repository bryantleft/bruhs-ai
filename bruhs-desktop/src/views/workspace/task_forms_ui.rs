//! The task detail panel and the new task dialog, with their native-style selects.

use super::*;
pub(super) const TASK_DIALOG_BODY_FONT_FAMILY: &str = "Monaspace Neon";
pub(super) const TASK_DIALOG_LABEL_FONT_FAMILY: &str = "Monaspace Argon";
pub(super) const TASK_DIALOG_TITLE_TEXT_SIZE_PX: f32 = 13.0;
pub(super) const TASK_FORM_LABEL_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const TASK_CREATE_OVERLAY_OPACITY: f32 = 0.70;
pub(super) const TASK_CREATE_HEADER_Y_PADDING_PX: f32 = 12.0;
pub(super) const TASK_CREATE_BODY_Y_PADDING_PX: f32 = 12.0;
pub(super) const TASK_CREATE_BODY_GAP_PX: f32 = 12.0;
pub(super) const TASK_CREATE_FOOTER_Y_PADDING_PX: f32 = 12.0;
pub(super) const TASK_CREATE_FOOTER_GAP_PX: f32 = 8.0;
pub(super) const TASK_CREATE_FOOTER_BG_OPACITY: f32 = 0.40;
pub(super) const TASK_DETAIL_PANEL_WIDTH_PX: f32 = 384.0;
pub(super) const TASK_DETAIL_TITLE_TEXT_SIZE_PX: f32 = 14.0;
pub(super) const TASK_DETAIL_HEADER_Y_PADDING_PX: f32 = 12.0;
pub(super) const TASK_DETAIL_BODY_Y_PADDING_PX: f32 = 16.0;
pub(super) const TASK_DETAIL_BODY_GAP_PX: f32 = 16.0;
pub(super) const TASK_DETAIL_FOOTER_Y_PADDING_PX: f32 = 12.0;
pub(super) const TASK_DETAIL_STATUS_CHIP_RADIUS_PX: f32 = 8.0;
pub(super) const TASK_DETAIL_STATUS_CHIP_X_PADDING_PX: f32 = 10.0;
pub(super) const TASK_DETAIL_STATUS_CHIP_Y_PADDING_PX: f32 = 4.0;
pub(super) const TASK_DETAIL_STATUS_CHIP_GAP_PX: f32 = 6.0;
pub(super) const TASK_DETAIL_STATUS_CHIP_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const TASK_DETAIL_STATUS_CHIP_ACTIVE_BORDER_OPACITY: f32 = 0.30;
pub(super) const TASK_CREATE_TITLE_PLACEHOLDER: &str = "What needs to be done?";
pub(super) const REQUIRED_TITLE_ERROR: &str = "Title is required.";
pub(super) const TASK_CREATE_DESCRIPTION_PLACEHOLDER: &str = "Add details (optional)";
pub(super) const TASK_CREATE_DUE_PLACEHOLDER: &str = "Select date";
pub(super) const TASK_CREATE_DESCRIPTION_ROWS: usize = 3;
pub(super) const TASK_DETAIL_CLOSE_LABEL: &str = "Close";
pub(super) const TASK_DETAIL_TITLE_PLACEHOLDER: &str = "Title";
pub(super) const TASK_DETAIL_DESCRIPTION_PLACEHOLDER: &str = "Add description";
pub(super) const TASK_DETAIL_DUE_PLACEHOLDER: &str = "";
pub(super) const TASK_DETAIL_DESCRIPTION_ROWS: usize = 5;

impl Workspace {
    pub(super) fn set_task_create_project(
        &mut self,
        project_id: Option<String>,
        cx: &mut Context<'_, Self>,
    ) {
        self.task_create_project_id = project_id;
        self.set_task_create_menu_open(None);
        cx.notify();
    }

    pub(super) fn set_task_create_status(
        &mut self,
        status: DesktopWorkItemStatus,
        cx: &mut Context<'_, Self>,
    ) {
        self.task_create_status = status;
        self.set_task_create_menu_open(None);
        cx.notify();
    }

    pub(super) fn set_task_create_priority(
        &mut self,
        priority: DesktopPriority,
        cx: &mut Context<'_, Self>,
    ) {
        self.task_create_priority = priority;
        self.set_task_create_menu_open(None);
        cx.notify();
    }

    pub(super) fn toggle_task_create_menu(
        &mut self,
        menu: TaskCreateMenu,
        cx: &mut Context<'_, Self>,
    ) {
        let open = if self.task_create_menu_open() == Some(menu) {
            None
        } else {
            Some(menu)
        };
        self.set_task_create_menu_open(open);
        cx.notify();
    }

    pub(super) fn toggle_task_detail_menu(
        &mut self,
        menu: TaskDetailMenu,
        cx: &mut Context<'_, Self>,
    ) {
        self.task_detail_menu_open = if self.task_detail_menu_open == Some(menu) {
            None
        } else {
            Some(menu)
        };
        cx.notify();
    }

    pub(super) fn open_task_create(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.active_view = AppView::Tasks;
        self.open_overlay(Overlay::TaskCreate { menu: None });
        self.task_create_project_id = self.selected_project_id.clone();
        self.task_create_status = DesktopWorkItemStatus::Backlog;
        self.task_create_priority = DesktopPriority::None;
        self.task_create_error = None;
        self.task_create_title_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.task_create_description_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.task_create_due_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        let input = self.task_create_title_input.clone();
        cx.defer_in(window, move |_this, window, cx| {
            input.update(cx, |input, cx| {
                input.focus(window, cx);
            });
        });
        self.persist_and_notify(cx);
    }

    pub(super) fn open_task_create_with_status(
        &mut self,
        status: DesktopWorkItemStatus,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.open_task_create(window, cx);
        self.task_create_status = status;
        cx.notify();
    }

    pub(super) fn create_task_from_title_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let title = self.task_create_title_input.read(cx).value().to_string();
        let title = title.trim();
        if title.is_empty() {
            self.task_create_error = Some(REQUIRED_TITLE_ERROR.to_string());
            self.task_create_title_input.update(cx, |input, cx| {
                input.focus(window, cx);
            });
            cx.notify();
            return;
        }
        self.task_create_error = None;

        let description = self
            .task_create_description_input
            .read(cx)
            .value()
            .to_string();
        let description = if description.trim().is_empty() {
            None
        } else {
            Some(description)
        };
        let due_date = self.task_create_due_input.read(cx).value().to_string();
        let due_date = if due_date.trim().is_empty() {
            None
        } else {
            Some(due_date)
        };
        let item = DesktopWorkItem {
            id: uuid::Uuid::new_v4().to_string(),
            project_id: self.task_create_project_id.clone(),
            title: title.to_string(),
            description,
            status: self.task_create_status,
            priority: self.task_create_priority,
            due_date,
            planning: WorkItemPlanning::default(),
            updated_at: EpochMs::now(),
        };
        let next_selected_work_item_id = task_create_selected_work_item_after_submit(
            self.selected_work_item_id.clone(),
            &item.id,
        );
        self.work_items.push(item);
        self.selected_work_item_id = next_selected_work_item_id;
        self.dismiss_overlay(|overlay| matches!(overlay, Overlay::TaskCreate { .. }));
        self.task_create_project_id = self.selected_project_id.clone();
        self.task_create_status = DesktopWorkItemStatus::Backlog;
        self.task_create_priority = DesktopPriority::None;
        self.task_create_error = None;
        self.task_create_title_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.task_create_description_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.task_create_due_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.persist_and_notify(cx);
    }

    pub(super) fn task_detail_project_label(&self, item: &DesktopWorkItem) -> String {
        item.project_id
            .as_deref()
            .and_then(|id| self.projects.iter().find(|project| project.id == id))
            .map_or_else(|| "No project".to_string(), |project| project.name.clone())
    }

    pub(super) fn render_task_detail_status_switcher(
        &self,
        item: &DesktopWorkItem,
        cx: &Context<'_, Self>,
    ) -> Div {
        let rows = DesktopWorkItemStatus::all()
            .iter()
            .enumerate()
            .map(|(index, status)| {
                let item_id = item.id.clone();
                let item_id_for_key = item.id.clone();
                let status = *status;
                let is_active = item.status == status;
                div()
                    .id(("task-detail-status-chip", index))
                    .debug_selector(move || {
                        format!("desktop-task-detail-status-chip-{}", status.as_str())
                    })
                    .rounded(px(TASK_DETAIL_STATUS_CHIP_RADIUS_PX))
                    .border_1()
                    .border_color(if is_active {
                        theme::persimmon_400()
                            .opacity(TASK_DETAIL_STATUS_CHIP_ACTIVE_BORDER_OPACITY)
                    } else {
                        theme::ring_panel()
                    })
                    .bg(if is_active {
                        theme::longan_700()
                    } else {
                        theme::longan_800()
                    })
                    .px(px(TASK_DETAIL_STATUS_CHIP_X_PADDING_PX))
                    .py(px(TASK_DETAIL_STATUS_CHIP_Y_PADDING_PX))
                    .flex()
                    .items_center()
                    .gap(px(TASK_DETAIL_STATUS_CHIP_GAP_PX))
                    .font_family(TASK_DIALOG_LABEL_FONT_FAMILY)
                    .font_weight(FontWeight::MEDIUM)
                    .text_size(px(TASK_DETAIL_STATUS_CHIP_TEXT_SIZE_PX))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_label(format!("Set status to {}", status.label()))
                    .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .when(!is_active, |el| {
                        el.hover(|el| {
                            el.border_color(theme::ring_floating())
                                .text_color(theme::rambutan_100())
                        })
                    })
                    .on_click(cx.listener(move |this, _ev, _window, cx| {
                        this.click_task_detail_status_chip(&item_id, status, cx);
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.click_task_detail_status_chip(&item_id_for_key, status, cx);
                        }
                    }))
                    .child(
                        div()
                            .text_color(status_glyph_color(status))
                            .font_weight(FontWeight::MEDIUM)
                            .child(status.glyph()),
                    )
                    .child(
                        div()
                            .text_color(theme::rambutan_200())
                            .child(status.label()),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();

        div()
            .flex()
            .flex_wrap()
            .gap(px(TASK_DETAIL_STATUS_CHIP_GAP_PX))
            .children(rows)
    }

    pub(super) fn render_task_detail_select_shell(
        &self,
        id: &'static str,
        label: &'static str,
        value: String,
        menu: TaskDetailMenu,
        rows: Vec<AnyElement>,
        cx: &Context<'_, Self>,
    ) -> Div {
        let is_open = self.task_detail_menu_open == Some(menu);
        div()
            .relative()
            .debug_selector(move || format!("desktop-task-detail-field-{id}"))
            .min_w_0()
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(FORM_FIELD_GAP_PX))
            .child(self.task_create_field_label(label))
            .child(
                div()
                    .id(("task-detail-select", element_key(id)))
                    .debug_selector(move || format!("desktop-task-detail-select-{id}"))
                    .h(px(FORM_SELECT_HEIGHT_PX))
                    .w_full()
                    .rounded(px(FORM_CONTROL_RADIUS_PX))
                    .border_1()
                    .border_color(theme::ring_card())
                    .bg(theme::control_surface())
                    .px_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .font_family(FORM_SELECT_BODY_FONT_FAMILY)
                    .text_size(px(FORM_SELECT_BODY_TEXT_SIZE_PX))
                    .text_color(theme::rambutan_200())
                    .tab_index(0)
                    .role(Role::ComboBox)
                    .aria_label(label)
                    .aria_expanded(is_open)
                    .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .hover(|el| el.border_color(theme::ring_floating()))
                    .on_click(cx.listener(move |this, _ev, _window, cx| {
                        this.toggle_task_detail_menu(menu, cx);
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.toggle_task_detail_menu(menu, cx);
                        }
                    }))
                    .child(div().flex_1().min_w_0().truncate().child(value))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(theme::rambutan_500())
                            .child("⌄"),
                    ),
            )
            .when(is_open, |el| {
                el.child(
                    div()
                        .id(("task-detail-menu", element_key(id)))
                        .role(Role::ListBox)
                        .aria_label(label)
                        .debug_selector(move || format!("desktop-task-detail-menu-{id}"))
                        .w_full()
                        .max_h(px(FORM_SELECT_MENU_MAX_HEIGHT_PX))
                        .overflow_y_scrollbar()
                        .rounded(px(FORM_CONTROL_RADIUS_PX))
                        .border_1()
                        .border_color(theme::ring_floating())
                        .bg(theme::longan_700())
                        .py(px(FORM_SELECT_MENU_Y_PADDING_PX))
                        .children(rows),
                )
            })
    }

    pub(super) fn render_task_detail_project_select(
        &self,
        item: &DesktopWorkItem,
        cx: &Context<'_, Self>,
    ) -> Div {
        let mut rows: Vec<AnyElement> = Vec::new();
        let item_id = item.id.clone();
        let item_id_for_key = item.id.clone();
        let none_selected = item.project_id.is_none();
        rows.push(
            div()
                .id("task-detail-project-option-none")
                .debug_selector(|| "desktop-task-detail-project-option-none".to_string())
                .w_full()
                .flex()
                .items_center()
                .gap(px(FORM_SELECT_OPTION_GAP_PX))
                .px(px(FORM_SELECT_OPTION_X_PADDING_PX))
                .py(px(FORM_SELECT_OPTION_Y_PADDING_PX))
                .font_family(FORM_SELECT_BODY_FONT_FAMILY)
                .text_size(px(FORM_SELECT_BODY_TEXT_SIZE_PX))
                .text_color(if none_selected {
                    theme::rambutan_100()
                } else {
                    theme::rambutan_300()
                })
                .tab_index(0)
                .role(Role::ListBoxOption)
                .aria_label("No project")
                .aria_selected(none_selected)
                .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                .cursor_pointer()
                .hover(|el| el.bg(theme::longan_800()))
                .on_click(cx.listener(move |this, _ev, _window, cx| {
                    this.update_work_item_project(&item_id, None, cx);
                }))
                .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                    if is_web_button_activation_key(event.keystroke.key.as_str()) {
                        window.prevent_default();
                        cx.stop_propagation();
                        this.update_work_item_project(&item_id_for_key, None, cx);
                    }
                }))
                .child(div().min_w_0().truncate().child("No project"))
                .into_any_element(),
        );
        for (index, project) in self.projects.iter().enumerate() {
            let item_id = item.id.clone();
            let item_id_for_key = item.id.clone();
            let project_id = project.id.clone();
            let project_id_for_key = project.id.clone();
            let debug_project_id = project.id.clone();
            let is_selected = item.project_id.as_deref() == Some(project.id.as_str());
            rows.push(
                div()
                    .id(("task-detail-project-option", index))
                    .debug_selector(move || {
                        format!(
                            "desktop-task-detail-project-option-{}",
                            element_key(&debug_project_id)
                        )
                    })
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(FORM_SELECT_OPTION_GAP_PX))
                    .px(px(FORM_SELECT_OPTION_X_PADDING_PX))
                    .py(px(FORM_SELECT_OPTION_Y_PADDING_PX))
                    .font_family(FORM_SELECT_BODY_FONT_FAMILY)
                    .text_size(px(FORM_SELECT_BODY_TEXT_SIZE_PX))
                    .text_color(if is_selected {
                        theme::rambutan_100()
                    } else {
                        theme::rambutan_300()
                    })
                    .tab_index(0)
                    .role(Role::ListBoxOption)
                    .aria_label(project.name.clone())
                    .aria_selected(is_selected)
                    .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .hover(|el| el.bg(theme::longan_800()))
                    .on_click(cx.listener(move |this, _ev, _window, cx| {
                        this.update_work_item_project(&item_id, Some(project_id.clone()), cx);
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.update_work_item_project(
                                &item_id_for_key,
                                Some(project_id_for_key.clone()),
                                cx,
                            );
                        }
                    }))
                    .child(div().min_w_0().truncate().child(project.name.clone()))
                    .into_any_element(),
            );
        }

        self.render_task_detail_select_shell(
            "project",
            "Project",
            self.task_detail_project_label(item),
            TaskDetailMenu::Project,
            rows,
            cx,
        )
    }

    pub(super) fn render_task_detail_status_select(
        &self,
        item: &DesktopWorkItem,
        cx: &Context<'_, Self>,
    ) -> Div {
        let rows = DesktopWorkItemStatus::all()
            .iter()
            .enumerate()
            .map(|(index, status)| {
                let item_id = item.id.clone();
                let item_id_for_key = item.id.clone();
                let status = *status;
                let is_selected = item.status == status;
                div()
                    .id(("task-detail-status-option", index))
                    .debug_selector(move || {
                        format!("desktop-task-detail-status-option-{}", status.as_str())
                    })
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(FORM_SELECT_OPTION_GAP_PX))
                    .px(px(FORM_SELECT_OPTION_X_PADDING_PX))
                    .py(px(FORM_SELECT_OPTION_Y_PADDING_PX))
                    .font_family(FORM_SELECT_BODY_FONT_FAMILY)
                    .text_size(px(FORM_SELECT_BODY_TEXT_SIZE_PX))
                    .text_color(if is_selected {
                        theme::rambutan_100()
                    } else {
                        theme::rambutan_300()
                    })
                    .tab_index(0)
                    .role(Role::ListBoxOption)
                    .aria_label(status.label())
                    .aria_selected(is_selected)
                    .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .hover(|el| el.bg(theme::longan_800()))
                    .on_click(cx.listener(move |this, _ev, _window, cx| {
                        this.select_task_detail_status_option(&item_id, status, cx);
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.select_task_detail_status_option(&item_id_for_key, status, cx);
                        }
                    }))
                    .child(format!("{} {}", status.glyph(), status.label()))
                    .into_any_element()
            })
            .collect::<Vec<_>>();

        self.render_task_detail_select_shell(
            "status",
            "Status",
            format!("{} {}", item.status.glyph(), item.status.label()),
            TaskDetailMenu::Status,
            rows,
            cx,
        )
    }

    pub(super) fn render_task_detail_priority_select(
        &self,
        item: &DesktopWorkItem,
        cx: &Context<'_, Self>,
    ) -> Div {
        let rows = DesktopPriority::all()
            .iter()
            .enumerate()
            .map(|(index, priority)| {
                let item_id = item.id.clone();
                let item_id_for_key = item.id.clone();
                let priority = *priority;
                let is_selected = item.priority == priority;
                div()
                    .id(("task-detail-priority-option", index))
                    .debug_selector(move || {
                        format!("desktop-task-detail-priority-option-{}", priority.as_str())
                    })
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(FORM_SELECT_OPTION_GAP_PX))
                    .px(px(FORM_SELECT_OPTION_X_PADDING_PX))
                    .py(px(FORM_SELECT_OPTION_Y_PADDING_PX))
                    .font_family(FORM_SELECT_BODY_FONT_FAMILY)
                    .text_size(px(FORM_SELECT_BODY_TEXT_SIZE_PX))
                    .text_color(if is_selected {
                        theme::rambutan_100()
                    } else {
                        theme::rambutan_300()
                    })
                    .tab_index(0)
                    .role(Role::ListBoxOption)
                    .aria_label(priority.label())
                    .aria_selected(is_selected)
                    .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .hover(|el| el.bg(theme::longan_800()))
                    .on_click(cx.listener(move |this, _ev, _window, cx| {
                        this.update_work_item_priority(&item_id, priority, cx);
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.update_work_item_priority(&item_id_for_key, priority, cx);
                        }
                    }))
                    .child(format!("{} {}", priority.glyph(), priority.label()))
                    .into_any_element()
            })
            .collect::<Vec<_>>();

        self.render_task_detail_select_shell(
            "priority",
            "Priority",
            format!("{} {}", item.priority.glyph(), item.priority.label()),
            TaskDetailMenu::Priority,
            rows,
            cx,
        )
    }

    pub(super) fn render_task_detail(&self, item: DesktopWorkItem, cx: &Context<'_, Self>) -> Div {
        let delete_id = item.id.clone();
        let delete_id_for_key = item.id.clone();
        let close_key = element_key(&item.id);
        let estimate_label = item.planning.estimate_minutes.map_or_else(
            || "No estimate".to_string(),
            |minutes| format!("{minutes} minutes"),
        );
        let recurrence_label = item.planning.recurrence.map_or_else(
            || "Does not repeat".to_string(),
            |rule| rule.label().to_string(),
        );
        let dependency_count = item.planning.dependency_ids.len();
        let time_block_count = item.planning.time_block_event_ids.len();

        div()
            .w(px(TASK_DETAIL_PANEL_WIDTH_PX))
            .h_full()
            .flex_shrink_0()
            .debug_selector(|| "desktop-task-detail".to_string())
            .bg(theme::longan_900())
            .border_1()
            .border_color(theme::ring_panel())
            .text_color(theme::rambutan_300())
            .font_family(TASK_DIALOG_BODY_FONT_FAMILY)
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(FORM_DIALOG_SECTION_X_PADDING_PX))
                    .py(px(TASK_DETAIL_HEADER_Y_PADDING_PX))
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(theme::ring_panel())
                    .child(
                        div()
                            .font_family(TASK_DIALOG_LABEL_FONT_FAMILY)
                            .text_size(px(TASK_DETAIL_TITLE_TEXT_SIZE_PX))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::rambutan_100())
                            .child("Task"),
                    )
                    .child(
                        web_button(
                            ("task-detail-close", close_key),
                            WebButtonVariant::Ghost,
                            WebButtonSize::IconSmall,
                            ButtonState::Enabled,
                        )
                        .debug_selector(|| "desktop-task-detail-close".to_string())
                        .aria_label(TASK_DETAIL_CLOSE_LABEL)
                        .child("×")
                        .tab_index(0)
                        .role(Role::Button)
                        .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                        .tooltip(|window, cx| {
                            Tooltip::new(TASK_DETAIL_CLOSE_LABEL).build(window, cx)
                        })
                        .on_click(cx.listener(|this, _ev, _window, cx| {
                            this.selected_work_item_id = None;
                            this.task_detail_menu_open = None;
                            this.persist_and_notify(cx);
                        }))
                        .on_key_down(cx.listener(
                            |this, event: &KeyDownEvent, window, cx| {
                                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                                    window.prevent_default();
                                    cx.stop_propagation();
                                    this.selected_work_item_id = None;
                                    this.task_detail_menu_open = None;
                                    this.persist_and_notify(cx);
                                }
                            },
                        )),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_y_scrollbar()
                    .px(px(FORM_DIALOG_SECTION_X_PADDING_PX))
                    .py(px(TASK_DETAIL_BODY_Y_PADDING_PX))
                    .flex()
                    .flex_col()
                    .gap(px(TASK_DETAIL_BODY_GAP_PX))
                    .child(self.render_task_detail_status_switcher(&item, cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(FORM_FIELD_GAP_PX))
                            .child(self.task_create_field_label("Title"))
                            .child(
                                div()
                                    .debug_selector(|| {
                                        "desktop-task-detail-title-input".to_string()
                                    })
                                    .child(
                                        form_text_input(Input::new(&self.task_detail_title_input))
                                            .aria_label("Task title"),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(FORM_FIELD_GAP_PX))
                            .child(self.task_create_field_label("Description"))
                            .child(
                                div()
                                    .debug_selector(|| {
                                        "desktop-task-detail-description-input".to_string()
                                    })
                                    .child(
                                        form_textarea(
                                            Input::new(&self.task_detail_description_input),
                                            FORM_FIVE_ROW_TEXTAREA_HEIGHT_PX,
                                        )
                                        .aria_label("Task description"),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(FORM_GRID_GAP_PX))
                            .child(self.render_task_detail_project_select(&item, cx))
                            .child(self.render_task_detail_status_select(&item, cx)),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(FORM_GRID_GAP_PX))
                            .child(self.render_task_detail_priority_select(&item, cx))
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .gap(px(FORM_FIELD_GAP_PX))
                                    .child(self.task_create_field_label("Due date"))
                                    .child(
                                        div()
                                            .debug_selector(|| {
                                                "desktop-task-detail-due-input".to_string()
                                            })
                                            .child(self.render_date_field(
                                                date_picker_ui::DatePickerTarget::TaskDetailDue,
                                                cx,
                                            )),
                                    ),
                            ),
                    ),
            )
            .child(
                div()
                    .border_t_1()
                    .border_color(theme::ring_panel())
                    .pt_3()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .font_family(TASK_DIALOG_LABEL_FONT_FAMILY)
                            .text_size(px(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::rambutan_200())
                            .child("Planning"),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                web_button(
                                    "task-detail-estimate",
                                    WebButtonVariant::Ghost,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .aria_label("Cycle task estimate")
                                .child(estimate_label)
                                .on_click(cx.listener(
                                    |this, _event, _window, cx| {
                                        this.cycle_selected_work_item_estimate(cx);
                                    },
                                )),
                            )
                            .child(
                                web_button(
                                    "task-detail-recurrence",
                                    WebButtonVariant::Ghost,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .aria_label("Cycle task recurrence")
                                .child(recurrence_label)
                                .on_click(cx.listener(
                                    |this, _event, _window, cx| {
                                        this.cycle_selected_work_item_recurrence(cx);
                                    },
                                )),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(FORM_GRID_GAP_PX))
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .gap(px(FORM_FIELD_GAP_PX))
                                    .child(self.task_create_field_label("Cycle"))
                                    .child(
                                        form_text_input(Input::new(&self.task_detail_cycle_input))
                                            .aria_label("Task cycle"),
                                    ),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .gap(px(FORM_FIELD_GAP_PX))
                                    .child(self.task_create_field_label("Waiting on"))
                                    .child(
                                        form_text_input(Input::new(
                                            &self.task_detail_waiting_input,
                                        ))
                                        .aria_label("Task waiting on"),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                web_button(
                                    "task-detail-dependencies",
                                    WebButtonVariant::Ghost,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .aria_label("Add or clear a task dependency")
                                .child(format!("{dependency_count} dependencies"))
                                .on_click(cx.listener(
                                    |this, _event, _window, cx| {
                                        this.cycle_selected_work_item_dependency(cx);
                                    },
                                )),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(theme::rambutan_500())
                                    .child(format!("{time_block_count} focus blocks")),
                            ),
                    ),
            )
            .child(
                div()
                    .px(px(FORM_DIALOG_SECTION_X_PADDING_PX))
                    .py(px(TASK_DETAIL_FOOTER_Y_PADDING_PX))
                    .border_t_1()
                    .border_color(theme::ring_panel())
                    .child(
                        web_button(
                            "task-detail-delete",
                            WebButtonVariant::Destructive,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .aria_label("Delete task")
                        .debug_selector(|| "desktop-task-detail-delete".to_string())
                        .child("Delete task")
                        .tab_index(0)
                        .role(Role::Button)
                        .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                        .on_click(cx.listener(move |this, _ev, _window, cx| {
                            this.delete_work_item_by_id(&delete_id, cx);
                        }))
                        .on_key_down(cx.listener(
                            move |this, event: &KeyDownEvent, window, cx| {
                                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                                    window.prevent_default();
                                    cx.stop_propagation();
                                    this.delete_work_item_by_id(&delete_id_for_key, cx);
                                }
                            },
                        )),
                    ),
            )
    }

    pub(super) fn task_create_project_label(&self) -> String {
        self.task_create_project_id
            .as_deref()
            .and_then(|id| self.projects.iter().find(|project| project.id == id))
            .map_or_else(|| "No project".to_string(), |project| project.name.clone())
    }

    pub(super) fn task_create_field_label(&self, label: &'static str) -> Div {
        div()
            .font_family(TASK_DIALOG_LABEL_FONT_FAMILY)
            .text_size(px(TASK_FORM_LABEL_TEXT_SIZE_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(theme::rambutan_400())
            .child(label)
    }

    pub(super) fn render_task_create_select_shell(
        &self,
        id: &'static str,
        label: &'static str,
        value: String,
        menu: TaskCreateMenu,
        rows: Vec<AnyElement>,
        cx: &Context<'_, Self>,
    ) -> Div {
        let is_open = self.task_create_menu_open() == Some(menu);
        div()
            .relative()
            .debug_selector(move || format!("desktop-task-create-field-{id}"))
            .min_w_0()
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(FORM_FIELD_GAP_PX))
            .child(self.task_create_field_label(label))
            .child(
                div()
                    .id(("task-create-select", element_key(id)))
                    .debug_selector(move || format!("desktop-task-create-select-{id}"))
                    .h(px(FORM_SELECT_HEIGHT_PX))
                    .w_full()
                    .rounded(px(FORM_CONTROL_RADIUS_PX))
                    .border_1()
                    .border_color(theme::ring_card())
                    .bg(theme::control_surface())
                    .px_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .font_family(FORM_SELECT_BODY_FONT_FAMILY)
                    .text_size(px(FORM_SELECT_BODY_TEXT_SIZE_PX))
                    .text_color(theme::rambutan_200())
                    .tab_index(0)
                    .role(Role::ComboBox)
                    .aria_label(label)
                    .aria_expanded(is_open)
                    .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .hover(|el| el.border_color(theme::ring_floating()))
                    .on_click(cx.listener(move |this, _ev, _window, cx| {
                        this.toggle_task_create_menu(menu, cx);
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.toggle_task_create_menu(menu, cx);
                        }
                    }))
                    .child(div().flex_1().min_w_0().truncate().child(value))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(theme::rambutan_500())
                            .child("⌄"),
                    ),
            )
            .when(is_open, |el| {
                el.child(
                    div()
                        .id(("task-create-menu", element_key(id)))
                        .role(Role::ListBox)
                        .aria_label(label)
                        .w_full()
                        .max_h(px(FORM_SELECT_MENU_MAX_HEIGHT_PX))
                        .overflow_y_scrollbar()
                        .rounded(px(FORM_CONTROL_RADIUS_PX))
                        .border_1()
                        .border_color(theme::ring_floating())
                        .bg(theme::longan_700())
                        .py(px(FORM_SELECT_MENU_Y_PADDING_PX))
                        .children(rows),
                )
            })
    }

    pub(super) fn render_task_create_project_select(&self, cx: &Context<'_, Self>) -> Div {
        let mut rows: Vec<AnyElement> = Vec::new();
        let no_project_selected = self.task_create_project_id.is_none();
        rows.push(
            div()
                .id("task-create-project-option-none")
                .debug_selector(|| "desktop-task-create-project-option-none".to_string())
                .w_full()
                .flex()
                .items_center()
                .gap(px(FORM_SELECT_OPTION_GAP_PX))
                .px(px(FORM_SELECT_OPTION_X_PADDING_PX))
                .py(px(FORM_SELECT_OPTION_Y_PADDING_PX))
                .font_family(FORM_SELECT_BODY_FONT_FAMILY)
                .text_size(px(FORM_SELECT_BODY_TEXT_SIZE_PX))
                .text_color(if no_project_selected {
                    theme::rambutan_100()
                } else {
                    theme::rambutan_300()
                })
                .tab_index(0)
                .role(Role::ListBoxOption)
                .aria_label("No project")
                .aria_selected(no_project_selected)
                .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                .cursor_pointer()
                .hover(|el| el.bg(theme::longan_800()))
                .on_click(cx.listener(|this, _ev, _window, cx| {
                    this.set_task_create_project(None, cx);
                }))
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if is_web_button_activation_key(event.keystroke.key.as_str()) {
                        window.prevent_default();
                        cx.stop_propagation();
                        this.set_task_create_project(None, cx);
                    }
                }))
                .child(div().min_w_0().truncate().child("No project"))
                .into_any_element(),
        );
        for (index, project) in self.projects.iter().enumerate() {
            let project_id = project.id.clone();
            let project_id_for_key = project.id.clone();
            let debug_index = index;
            let is_selected = self.task_create_project_id.as_deref() == Some(project.id.as_str());
            rows.push(
                div()
                    .id(("task-create-project-option", index))
                    .debug_selector(move || {
                        format!("desktop-task-create-project-option-{debug_index}")
                    })
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(FORM_SELECT_OPTION_GAP_PX))
                    .px(px(FORM_SELECT_OPTION_X_PADDING_PX))
                    .py(px(FORM_SELECT_OPTION_Y_PADDING_PX))
                    .font_family(FORM_SELECT_BODY_FONT_FAMILY)
                    .text_size(px(FORM_SELECT_BODY_TEXT_SIZE_PX))
                    .text_color(if is_selected {
                        theme::rambutan_100()
                    } else {
                        theme::rambutan_300()
                    })
                    .tab_index(0)
                    .role(Role::ListBoxOption)
                    .aria_label(project.name.clone())
                    .aria_selected(is_selected)
                    .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .hover(|el| el.bg(theme::longan_800()))
                    .on_click(cx.listener(move |this, _ev, _window, cx| {
                        this.set_task_create_project(Some(project_id.clone()), cx);
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.set_task_create_project(Some(project_id_for_key.clone()), cx);
                        }
                    }))
                    .child(div().min_w_0().truncate().child(project.name.clone()))
                    .into_any_element(),
            );
        }

        self.render_task_create_select_shell(
            "project",
            "Project",
            self.task_create_project_label(),
            TaskCreateMenu::Project,
            rows,
            cx,
        )
    }

    pub(super) fn render_task_create_status_select(&self, cx: &Context<'_, Self>) -> Div {
        let rows = DesktopWorkItemStatus::all()
            .iter()
            .enumerate()
            .map(|(index, status)| {
                let status = *status;
                let debug_status = status.as_str();
                let is_selected = self.task_create_status == status;
                div()
                    .id(("task-create-status-option", index))
                    .debug_selector(move || {
                        format!("desktop-task-create-status-option-{debug_status}")
                    })
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(FORM_SELECT_OPTION_GAP_PX))
                    .px(px(FORM_SELECT_OPTION_X_PADDING_PX))
                    .py(px(FORM_SELECT_OPTION_Y_PADDING_PX))
                    .font_family(FORM_SELECT_BODY_FONT_FAMILY)
                    .text_size(px(FORM_SELECT_BODY_TEXT_SIZE_PX))
                    .text_color(if is_selected {
                        theme::rambutan_100()
                    } else {
                        theme::rambutan_300()
                    })
                    .tab_index(0)
                    .role(Role::ListBoxOption)
                    .aria_label(status.label())
                    .aria_selected(is_selected)
                    .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .hover(|el| el.bg(theme::longan_800()))
                    .on_click(cx.listener(move |this, _ev, _window, cx| {
                        this.set_task_create_status(status, cx);
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.set_task_create_status(status, cx);
                        }
                    }))
                    .child(format!("{} {}", status.glyph(), status.label()))
                    .into_any_element()
            })
            .collect::<Vec<_>>();

        self.render_task_create_select_shell(
            "status",
            "Status",
            format!(
                "{} {}",
                self.task_create_status.glyph(),
                self.task_create_status.label()
            ),
            TaskCreateMenu::Status,
            rows,
            cx,
        )
    }

    pub(super) fn render_task_create_priority_select(&self, cx: &Context<'_, Self>) -> Div {
        let rows = DesktopPriority::all()
            .iter()
            .enumerate()
            .map(|(index, priority)| {
                let priority = *priority;
                let debug_priority = priority.as_str();
                let is_selected = self.task_create_priority == priority;
                div()
                    .id(("task-create-priority-option", index))
                    .debug_selector(move || {
                        format!("desktop-task-create-priority-option-{debug_priority}")
                    })
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(FORM_SELECT_OPTION_GAP_PX))
                    .px(px(FORM_SELECT_OPTION_X_PADDING_PX))
                    .py(px(FORM_SELECT_OPTION_Y_PADDING_PX))
                    .font_family(FORM_SELECT_BODY_FONT_FAMILY)
                    .text_size(px(FORM_SELECT_BODY_TEXT_SIZE_PX))
                    .text_color(if is_selected {
                        theme::rambutan_100()
                    } else {
                        theme::rambutan_300()
                    })
                    .tab_index(0)
                    .role(Role::ListBoxOption)
                    .aria_label(priority.label())
                    .aria_selected(is_selected)
                    .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .hover(|el| el.bg(theme::longan_800()))
                    .on_click(cx.listener(move |this, _ev, _window, cx| {
                        this.set_task_create_priority(priority, cx);
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.set_task_create_priority(priority, cx);
                        }
                    }))
                    .child(format!("{} {}", priority.glyph(), priority.label()))
                    .into_any_element()
            })
            .collect::<Vec<_>>();

        self.render_task_create_select_shell(
            "priority",
            "Priority",
            format!(
                "{} {}",
                self.task_create_priority.glyph(),
                self.task_create_priority.label()
            ),
            TaskCreateMenu::Priority,
            rows,
            cx,
        )
    }

    pub(super) fn render_task_create_dialog(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let dialog_height = (f32::from(window.viewport_size().height)
            - DESKTOP_DIALOG_TOP_PADDING_PX
            - DESKTOP_DIALOG_BOTTOM_GAP_PX)
            .clamp(
                DESKTOP_DIALOG_MIN_AVAILABLE_HEIGHT_PX,
                FORM_DIALOG_PREFERRED_HEIGHT_PX,
            );

        div()
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .left_0()
            .occlude()
            .id("desktop-task-create-overlay")
            .debug_selector(|| "desktop-task-create-overlay".to_string())
            .bg(theme::longan_950().opacity(TASK_CREATE_OVERLAY_OPACITY))
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
                    .id("desktop-task-create-dialog")
                    .role(Role::Dialog)
                    .aria_label("New task")
                    .child(self.blur_scrim.backdrop())
                    .debug_selector(|| "desktop-task-create-dialog".to_string())
                    .w(px(DESKTOP_DIALOG_MD_WIDTH_PX))
                    .max_w(relative(DESKTOP_DIALOG_MAX_WIDTH_RATIO))
                    .max_h(px(dialog_height))
                    .overflow_hidden()
                    .rounded(px(DESKTOP_DIALOG_RADIUS_PX))
                    .border_1()
                    .border_color(theme::ring_modal())
                    .bg(theme::longan_800())
                    .text_color(cx.theme().popover_foreground)
                    .font_family(TASK_DIALOG_BODY_FONT_FAMILY)
                    .flex()
                    .flex_col()
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
                            .px(px(FORM_DIALOG_SECTION_X_PADDING_PX))
                            .py(px(TASK_CREATE_HEADER_Y_PADDING_PX))
                            .border_b_1()
                            .border_color(theme::longan_800())
                            .child(
                                div()
                                    .text_size(px(TASK_DIALOG_TITLE_TEXT_SIZE_PX))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::rambutan_100())
                                    .child("New task"),
                            ),
                    )
                    .child(
                        div()
                            .debug_selector(|| "desktop-task-create-body".to_string())
                            .px(px(FORM_DIALOG_SECTION_X_PADDING_PX))
                            .py(px(TASK_CREATE_BODY_Y_PADDING_PX))
                            .flex()
                            .flex_col()
                            .gap(px(TASK_CREATE_BODY_GAP_PX))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(FORM_FIELD_GAP_PX))
                                    .child(self.task_create_field_label("Title"))
                                    .child(
                                        form_text_input(Input::new(&self.task_create_title_input))
                                            .aria_label("Task title"),
                                    )
                                    .when_some(self.task_create_error.clone(), |el, error| {
                                        el.child(
                                            div()
                                                .id("task-create-title-error")
                                                .debug_selector(|| {
                                                    "desktop-task-create-title-error".to_string()
                                                })
                                                .role(Role::Alert)
                                                .aria_label(error.clone())
                                                .font_family(TASK_DIALOG_LABEL_FONT_FAMILY)
                                                .text_size(px(TASK_FORM_LABEL_TEXT_SIZE_PX))
                                                .font_family(theme::UI_SMALL_FONT_FAMILY)
                                                .text_color(theme::lychee_300())
                                                .child(error),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(FORM_FIELD_GAP_PX))
                                    .child(self.task_create_field_label("Description"))
                                    .child(
                                        form_textarea(
                                            Input::new(&self.task_create_description_input),
                                            FORM_THREE_ROW_TEXTAREA_HEIGHT_PX,
                                        )
                                        .aria_label("Task description"),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap(px(FORM_GRID_GAP_PX))
                                    .child(self.render_task_create_project_select(cx))
                                    .child(self.render_task_create_status_select(cx)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap(px(FORM_GRID_GAP_PX))
                                    .child(self.render_task_create_priority_select(cx))
                                    .child(
                                        div()
                                            .min_w_0()
                                            .flex_1()
                                            .flex()
                                            .flex_col()
                                            .gap(px(FORM_FIELD_GAP_PX))
                                            .child(self.task_create_field_label("Due date"))
                                            .child(self.render_date_field(
                                                date_picker_ui::DatePickerTarget::TaskDue,
                                                cx,
                                            )),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .border_t_1()
                            .border_color(theme::longan_800())
                            .bg(theme::longan_900().opacity(TASK_CREATE_FOOTER_BG_OPACITY))
                            .px(px(FORM_DIALOG_SECTION_X_PADDING_PX))
                            .py(px(TASK_CREATE_FOOTER_Y_PADDING_PX))
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap(px(TASK_CREATE_FOOTER_GAP_PX))
                            .child(
                                web_button(
                                    "task-create-cancel",
                                    WebButtonVariant::Ghost,
                                    WebButtonSize::Default,
                                    ButtonState::Enabled,
                                )
                                .aria_label("Cancel")
                                .debug_selector(|| "desktop-task-create-cancel".to_string())
                                .child("Cancel")
                                .tab_index(0)
                                .role(Role::Button)
                                .focus_visible(|el| {
                                    el.border_2().border_color(theme::persimmon_400())
                                })
                                .on_click(cx.listener(|this, _ev, _window, cx| {
                                    this.dismiss_overlay(|overlay| {
                                        matches!(overlay, Overlay::TaskCreate { .. })
                                    });
                                    this.task_create_error = None;
                                    cx.notify();
                                }))
                                .on_key_down(cx.listener(
                                    |this, event: &KeyDownEvent, window, cx| {
                                        if is_web_button_activation_key(
                                            event.keystroke.key.as_str(),
                                        ) {
                                            window.prevent_default();
                                            cx.stop_propagation();
                                            this.dismiss_overlay(|overlay| {
                                                matches!(overlay, Overlay::TaskCreate { .. })
                                            });
                                            this.task_create_error = None;
                                            cx.notify();
                                        }
                                    },
                                )),
                            )
                            .child(
                                web_button(
                                    "task-create-submit",
                                    WebButtonVariant::Primary,
                                    WebButtonSize::Default,
                                    ButtonState::Enabled,
                                )
                                .aria_label("Create task")
                                .debug_selector(|| "desktop-task-create-submit".to_string())
                                .child("Create")
                                .on_click(cx.listener(|this, _ev, window, cx| {
                                    this.create_task_from_title_input(window, cx);
                                }))
                                .on_key_down(cx.listener(
                                    |this, event: &KeyDownEvent, window, cx| {
                                        if is_web_button_activation_key(
                                            event.keystroke.key.as_str(),
                                        ) {
                                            window.prevent_default();
                                            cx.stop_propagation();
                                            this.create_task_from_title_input(window, cx);
                                        }
                                    },
                                )),
                            ),
                    )
                    .child(self.render_modal_focus_sentinel()),
            )
            .into_any_element()
    }
}

/// The new-task dialog and task-detail fields.
pub(super) struct TaskFormInputs {
    pub(super) task_create_title_input: Entity<InputState>,
    pub(super) task_create_description_input: Entity<InputState>,
    pub(super) task_create_due_input: Entity<InputState>,
    pub(super) task_detail_title_input: Entity<InputState>,
    pub(super) task_detail_description_input: Entity<InputState>,
    pub(super) task_detail_due_input: Entity<InputState>,
    pub(super) task_detail_cycle_input: Entity<InputState>,
    pub(super) task_detail_waiting_input: Entity<InputState>,
}

impl Workspace {
    /// The new-task dialog fields (Enter in the title creates the task) and the task-detail fields, which save as they change.
    pub(super) fn new_task_form_inputs(
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> TaskFormInputs {
        let task_create_title_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(TASK_CREATE_TITLE_PLACEHOLDER));
        subscriptions.push(cx.subscribe_in(
            &task_create_title_input,
            window,
            |this: &mut Self, _input, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => {
                    this.task_create_error = None;
                    cx.notify();
                }
                InputEvent::PressEnter { secondary, .. } if !*secondary => {
                    this.create_task_from_title_input(window, cx);
                }
                _ => {}
            },
        ));
        let task_create_description_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(TASK_CREATE_DESCRIPTION_PLACEHOLDER)
                .multi_line(true)
                .rows(TASK_CREATE_DESCRIPTION_ROWS)
        });
        subscriptions.push(redraw_on_edit(&task_create_description_input, window, cx));
        let task_create_due_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(TASK_CREATE_DUE_PLACEHOLDER));
        subscriptions.push(redraw_on_edit(&task_create_due_input, window, cx));
        let task_detail_title_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(TASK_DETAIL_TITLE_PLACEHOLDER));
        subscriptions.push(cx.subscribe_in(
            &task_detail_title_input,
            window,
            |this: &mut Self, _input, ev: &InputEvent, _window, cx| {
                if matches!(ev, InputEvent::Change) {
                    let title = this.task_detail_title_input.read(cx).value().to_string();
                    this.update_selected_work_item_title(title, cx);
                }
            },
        ));
        let task_detail_description_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(TASK_DETAIL_DESCRIPTION_PLACEHOLDER)
                .multi_line(true)
                .rows(TASK_DETAIL_DESCRIPTION_ROWS)
        });
        subscriptions.push(cx.subscribe_in(
            &task_detail_description_input,
            window,
            |this: &mut Self, _input, ev: &InputEvent, _window, cx| {
                if matches!(ev, InputEvent::Change) {
                    let description = this
                        .task_detail_description_input
                        .read(cx)
                        .value()
                        .to_string();
                    this.update_selected_work_item_description(description, cx);
                }
            },
        ));
        let task_detail_due_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(TASK_DETAIL_DUE_PLACEHOLDER));
        subscriptions.push(cx.subscribe_in(
            &task_detail_due_input,
            window,
            |this: &mut Self, _input, ev: &InputEvent, _window, cx| {
                if matches!(ev, InputEvent::Change) {
                    let due_date = this.task_detail_due_input.read(cx).value().to_string();
                    this.update_selected_work_item_due_date(due_date, cx);
                }
            },
        ));
        let task_detail_cycle_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("e.g. 2026 W37"));
        subscriptions.push(cx.subscribe_in(
            &task_detail_cycle_input,
            window,
            |this: &mut Self, _input, ev: &InputEvent, _window, cx| {
                if matches!(ev, InputEvent::Change) {
                    let value = this.task_detail_cycle_input.read(cx).value().to_string();
                    this.update_selected_work_item_planning_text(true, value, cx);
                }
            },
        ));
        let task_detail_waiting_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Person or condition"));
        subscriptions.push(cx.subscribe_in(
            &task_detail_waiting_input,
            window,
            |this: &mut Self, _input, ev: &InputEvent, _window, cx| {
                if matches!(ev, InputEvent::Change) {
                    let value = this.task_detail_waiting_input.read(cx).value().to_string();
                    this.update_selected_work_item_planning_text(false, value, cx);
                }
            },
        ));
        TaskFormInputs {
            task_create_title_input,
            task_create_description_input,
            task_create_due_input,
            task_detail_title_input,
            task_detail_description_input,
            task_detail_due_input,
            task_detail_cycle_input,
            task_detail_waiting_input,
        }
    }
}
