//! Tasks page: project sidebar, header, list and board views, and the work-item actions behind them.

use super::*;

/// GPUI-side colour resolution for shared status / priority enums. Lives in
/// the desktop crate (rather than bruhs-models) because `Hsla` is a GPUI
/// type and the colour values reference the desktop theme tokens.
pub(super) fn status_dot_color(status: DesktopWorkItemStatus) -> Hsla {
    match status {
        DesktopWorkItemStatus::Backlog => theme::rambutan_500(),
        DesktopWorkItemStatus::Todo => theme::rambutan_400(),
        DesktopWorkItemStatus::InProgress => theme::persimmon_400(),
        DesktopWorkItemStatus::Done => theme::pandan_400(),
        DesktopWorkItemStatus::Canceled => theme::rambutan_500(),
    }
}

pub(super) fn status_glyph_color(status: DesktopWorkItemStatus) -> Hsla {
    match status {
        DesktopWorkItemStatus::Backlog => theme::rambutan_500(),
        DesktopWorkItemStatus::Todo => theme::blueberry_400(),
        DesktopWorkItemStatus::InProgress => theme::durian_400(),
        DesktopWorkItemStatus::Done => theme::pandan_400(),
        DesktopWorkItemStatus::Canceled => theme::rambutan_500(),
    }
}

pub(super) fn priority_color(priority: DesktopPriority) -> Hsla {
    match priority {
        DesktopPriority::Urgent => theme::lychee_400(),
        DesktopPriority::High => theme::persimmon_400(),
        DesktopPriority::Medium => theme::durian_400(),
        DesktopPriority::Low => theme::blueberry_400(),
        DesktopPriority::None => theme::rambutan_500(),
    }
}

impl DesktopTaskDrag {
    pub(super) fn new(item: &DesktopWorkItem) -> Self {
        Self {
            item_id: item.id.clone(),
            title: item.title.clone(),
            priority: item.priority,
        }
    }
}

impl Render for DesktopTaskDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        div()
            .rounded(px(TASK_DRAG_PREVIEW_RADIUS_PX))
            .border_1()
            .border_color(theme::persimmon_400().opacity(TASK_DRAG_PREVIEW_BORDER_OPACITY))
            .bg(theme::longan_900().opacity(TASK_DRAG_PREVIEW_BG_OPACITY))
            .px_3()
            .py_2()
            .text_xs()
            .text_color(theme::rambutan_100())
            .child(format!("{}  {}", self.priority.glyph(), self.title))
    }
}

pub(super) fn work_item_due_on(item: &DesktopWorkItem, date: NaiveDate) -> bool {
    let due = date.format(DESKTOP_DATE_FORMAT).to_string();
    item.due_date.as_deref() == Some(due.as_str())
}

pub(super) fn format_desktop_due_date(date: &str, today: NaiveDate) -> Option<(String, Hsla)> {
    let due = NaiveDate::parse_from_str(date, DESKTOP_DATE_FORMAT).ok()?;
    if due == today {
        return Some(("today".to_string(), theme::persimmon_400()));
    }
    if due == today + Duration::days(1) {
        return Some(("tomorrow".to_string(), theme::rambutan_500()));
    }

    let label = due.format("%b %-d").to_string();
    let color = if due < today {
        theme::lychee_400()
    } else {
        theme::rambutan_500()
    };
    Some((label, color))
}

pub(super) fn project_task_count(items: &[DesktopWorkItem], project_id: &str) -> usize {
    items
        .iter()
        .filter(|item| item.project_id.as_deref() == Some(project_id))
        .count()
}

pub(super) fn selected_project_name_like_web(
    projects: &[DesktopProject],
    selected_project_id: Option<&str>,
) -> String {
    match selected_project_id {
        None => TASKS_ALL_PROJECTS_LABEL.to_string(),
        Some(project_id) => projects
            .iter()
            .find(|project| project.id == project_id)
            .map_or_else(|| "Project".to_string(), |project| project.name.clone()),
    }
}

pub(super) fn project_delete_prompt(project_name: &str, task_count: usize) -> String {
    format!(
        "Delete project \"{}\" and {} task{}?",
        project_name,
        task_count,
        if task_count == 1 { "" } else { "s" }
    )
}

pub(super) fn move_work_item_to_status(
    items: &mut [DesktopWorkItem],
    item_id: &str,
    status: DesktopWorkItemStatus,
) -> bool {
    let Some(item) = items.iter_mut().find(|item| item.id == item_id) else {
        return false;
    };
    if item.status == status {
        return false;
    }
    item.status = status;
    item.updated_at = EpochMs::now();
    true
}

pub(super) fn task_detail_menu_after_native_select_selection(
    current_menu: Option<TaskDetailMenu>,
    selected_menu: TaskDetailMenu,
) -> Option<TaskDetailMenu> {
    if current_menu == Some(selected_menu) {
        None
    } else {
        current_menu
    }
}

pub(super) fn click_task_detail_status_chip_like_web(
    item: &mut DesktopWorkItem,
    status: DesktopWorkItemStatus,
) {
    item.status = status;
    item.updated_at = EpochMs::now();
}

pub(super) fn next_recurring_desktop_item(item: &DesktopWorkItem) -> Option<DesktopWorkItem> {
    let rule = item.planning.recurrence?;
    let due_date = item.due_date.as_deref()?;
    let next_due_date = bruhs_models::next_recurrence_date(due_date, rule)?;
    let mut planning = item.planning.clone();
    planning.time_block_event_ids.clear();
    Some(DesktopWorkItem {
        id: uuid::Uuid::new_v4().to_string(),
        project_id: item.project_id.clone(),
        title: item.title.clone(),
        description: item.description.clone(),
        status: DesktopWorkItemStatus::Todo,
        priority: item.priority,
        due_date: Some(next_due_date),
        planning,
        updated_at: EpochMs::now(),
    })
}

pub(super) fn select_task_detail_status_option_like_web(
    item: &mut DesktopWorkItem,
    status: DesktopWorkItemStatus,
) -> bool {
    if item.status == status {
        return false;
    }
    item.status = status;
    item.updated_at = EpochMs::now();
    true
}

pub(super) fn select_task_detail_project_option_like_web(
    item: &mut DesktopWorkItem,
    project_id: Option<String>,
) -> bool {
    if item.project_id == project_id {
        return false;
    }
    item.project_id = project_id;
    item.updated_at = EpochMs::now();
    true
}

pub(super) fn select_task_detail_priority_option_like_web(
    item: &mut DesktopWorkItem,
    priority: DesktopPriority,
) -> bool {
    if item.priority == priority {
        return false;
    }
    item.priority = priority;
    item.updated_at = EpochMs::now();
    true
}
pub(super) const TASKS_ALL_PROJECTS_LABEL: &str = "All Tasks";
pub(super) const TASKS_NEW_PROJECT_LABEL: &str = "New project";
pub(super) const TASKS_NEW_TASK_LABEL: &str = "New task";
pub(super) const TASKS_HEADER_BODY_FONT_FAMILY: &str = "Monaspace Neon";
pub(super) const TASKS_HEADER_MIN_HEIGHT_PX: f32 = 40.0;
pub(super) const TASKS_HEADER_X_PADDING_PX: f32 = 16.0;
pub(super) const TASKS_HEADER_Y_PADDING_PX: f32 = 8.0;
pub(super) const TASKS_HEADER_BORDER_OPACITY: f32 = 0.40;
pub(super) const TASKS_HEADER_CONTROL_GAP_PX: f32 = 4.0;
pub(super) const TASKS_HEADER_CONTROL_RADIUS_PX: f32 = 8.0;
pub(super) const TASKS_HEADER_CONTROL_X_PADDING_PX: f32 = 10.0;
pub(super) const TASKS_HEADER_CONTROL_Y_PADDING_PX: f32 = 4.0;
pub(super) const TASKS_HEADER_MODE_ACTIVE_BG_OPACITY: f32 = 0.70;
pub(super) const TASKS_HEADER_TITLE_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const TASKS_HEADER_MODE_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const TASKS_HEADER_NEW_TASK_LEFT_MARGIN_PX: f32 = 4.0;
pub(super) const TASKS_HEADER_NEW_TASK_BORDER_OPACITY: f32 = 0.20;
pub(super) const TASKS_HEADER_NEW_TASK_BG_OPACITY: f32 = 0.15;
pub(super) const TASKS_HEADER_NEW_TASK_HOVER_BG_OPACITY: f32 = 0.25;
pub(super) const TASKS_HEADER_NEW_TASK_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const PROJECT_SIDEBAR_BODY_FONT_FAMILY: &str = "Monaspace Neon";
pub(super) const PROJECT_SIDEBAR_LABEL_FONT_FAMILY: &str = "Monaspace Argon";
pub(super) const PROJECT_SIDEBAR_BORDER_OPACITY: f32 = 0.50;
pub(super) const PROJECT_SIDEBAR_CONTENT_X_PADDING_PX: f32 = 8.0;
pub(super) const PROJECT_SIDEBAR_CONTENT_Y_PADDING_PX: f32 = 8.0;
pub(super) const PROJECT_SIDEBAR_ROW_STACK_GAP_PX: f32 = 2.0;
pub(super) const PROJECT_SIDEBAR_ROW_HEIGHT_PX: f32 = 32.0;
pub(super) const PROJECT_SIDEBAR_ROW_RADIUS_PX: f32 = 8.0;
pub(super) const PROJECT_SIDEBAR_ROW_GAP_PX: f32 = 6.0;
pub(super) const PROJECT_SIDEBAR_ROW_X_PADDING_PX: f32 = 8.0;
pub(super) const PROJECT_SIDEBAR_ROW_ACTIVE_BG_OPACITY: f32 = 0.40;
pub(super) const PROJECT_SIDEBAR_ROW_HOVER_BG_OPACITY: f32 = 0.30;
pub(super) const PROJECT_SIDEBAR_ROW_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const PROJECT_SIDEBAR_FOOTER_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const PROJECT_SIDEBAR_FOOTER_ICON_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const PROJECT_SIDEBAR_FOOTER_HEIGHT_PX: f32 = 36.0;
pub(super) const PROJECT_SIDEBAR_FOOTER_CONTAINER_X_PADDING_PX: f32 = 12.0;
pub(super) const PROJECT_SIDEBAR_FOOTER_CONTAINER_BOTTOM_PADDING_PX: f32 = 12.0;
pub(super) const PROJECT_SIDEBAR_FOOTER_GAP_PX: f32 = 6.0;
pub(super) const PROJECT_SIDEBAR_FOOTER_X_PADDING_PX: f32 = 12.0;
pub(super) const PROJECT_SIDEBAR_FOOTER_Y_PADDING_PX: f32 = 8.0;
pub(super) const PROJECT_SIDEBAR_FOOTER_BORDER_OPACITY: f32 = 0.50;
pub(super) const PROJECT_SIDEBAR_FOOTER_HOVER_BORDER_OPACITY: f32 = 0.50;
pub(super) const PROJECT_SIDEBAR_DELETE_ICON_SIZE_PX: f32 = 14.0;
pub(super) const PROJECT_SIDEBAR_ROW_ACTION_RADIUS_PX: f32 = 4.0;
pub(super) const PROJECT_SIDEBAR_ROW_ACTION_PADDING_PX: f32 = 2.0;
pub(super) const PROJECT_SIDEBAR_ROW_ACTION_HOVER_BG_OPACITY: f32 = 0.50;
pub(super) const TASK_LIST_BODY_FONT_FAMILY: &str = "Monaspace Neon";
pub(super) const TASK_LIST_HEADER_HEIGHT_PX: f32 = 34.0;
pub(super) const TASK_LIST_HEADER_X_PADDING_PX: f32 = 12.0;
pub(super) const TASK_LIST_HEADER_GAP_PX: f32 = 8.0;
pub(super) const TASK_LIST_HEADER_BORDER_OPACITY: f32 = 0.50;
pub(super) const TASK_LIST_HEADER_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const TASK_LIST_BODY_Y_PADDING_PX: f32 = 4.0;
pub(super) const TASK_LIST_EMPTY_Y_PADDING_PX: f32 = 48.0;
pub(super) const TASK_LIST_EMPTY_TEXT_SIZE_PX: f32 = 13.0;
pub(super) const TASK_LIST_STATUS_SLOT_WIDTH_PX: f32 = 24.0;
pub(super) const TASK_LIST_PRIORITY_COLUMN_WIDTH_PX: f32 = 32.0;
pub(super) const TASK_LIST_DUE_COLUMN_WIDTH_PX: f32 = 80.0;
pub(super) const TASK_ROW_BODY_FONT_FAMILY: &str = "Monaspace Neon";
pub(super) const TASK_ROW_RADIUS_PX: f32 = 8.0;
pub(super) const TASK_ROW_X_PADDING_PX: f32 = 12.0;
pub(super) const TASK_ROW_Y_PADDING_PX: f32 = 8.0;
pub(super) const TASK_ROW_GAP_PX: f32 = 8.0;
pub(super) const TASK_ROW_SELECTED_BG_OPACITY: f32 = 0.40;
pub(super) const TASK_ROW_HOVER_BG_OPACITY: f32 = 0.30;
pub(super) const TASK_ROW_STATUS_DOT_SIZE_PX: f32 = 10.0;
pub(super) const TASK_ROW_STATUS_DOT_RADIUS_PX: f32 = 999.0;
pub(super) const TASK_ROW_TITLE_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const TASK_ROW_DUE_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const TASK_PRIORITY_BAR_WIDTH_PX: f32 = 2.0;
pub(super) const TASK_PRIORITY_BAR_GAP_PX: f32 = 1.0;
pub(super) const TASK_PRIORITY_BAR_RADIUS_PX: f32 = 999.0;
pub(super) const TASK_PRIORITY_BAR_HEIGHTS_PX: [f32; 4] = [4.0, 6.0, 8.0, 10.0];
pub(super) const TASK_PRIORITY_EMPTY_OPACITY: f32 = 0.20;
pub(super) const TASK_BOARD_BODY_FONT_FAMILY: &str = "Monaspace Neon";
pub(super) const TASK_BOARD_PADDING_PX: f32 = 16.0;
pub(super) const TASK_BOARD_GAP_PX: f32 = 12.0;
pub(super) const TASK_BOARD_COLUMN_MIN_WIDTH_PX: f32 = 190.0;
pub(super) const TASK_BOARD_COLUMN_RADIUS_PX: f32 = 8.0;
pub(super) const TASK_BOARD_DRAG_OVER_RING_PX: f32 = 2.0;
pub(super) const TASK_BOARD_DRAG_OVER_RING_OPACITY: f32 = 0.40;
pub(super) const TASK_BOARD_COLUMN_COMPLETED_OPACITY: f32 = 0.70;
pub(super) const TASK_BOARD_COLUMN_COMPLETED_BG_OPACITY: f32 = 0.50;
pub(super) const TASK_BOARD_COLUMN_ACTIVE_BG_OPACITY: f32 = 0.30;
pub(super) const TASK_BOARD_HEADER_X_PADDING_PX: f32 = 12.0;
pub(super) const TASK_BOARD_HEADER_Y_PADDING_PX: f32 = 10.0;
pub(super) const TASK_BOARD_HEADER_GAP_PX: f32 = 8.0;
pub(super) const TASK_BOARD_HEADER_DOT_SIZE_PX: f32 = 8.0;
pub(super) const TASK_BOARD_HEADER_DOT_RADIUS_PX: f32 = 999.0;
pub(super) const TASK_BOARD_COUNT_RADIUS_PX: f32 = 999.0;
pub(super) const TASK_BOARD_COUNT_X_PADDING_PX: f32 = 6.0;
pub(super) const TASK_BOARD_COUNT_BG_OPACITY: f32 = 0.50;
pub(super) const TASK_BOARD_HEADER_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const TASK_BOARD_BODY_PADDING_PX: f32 = 8.0;
pub(super) const TASK_BOARD_BODY_GAP_PX: f32 = 8.0;
pub(super) const TASK_BOARD_EMPTY_Y_PADDING_PX: f32 = 16.0;
pub(super) const TASK_BOARD_COUNT_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const TASK_BOARD_EMPTY_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const TASK_CARD_RADIUS_PX: f32 = 8.0;
pub(super) const TASK_CARD_PADDING_PX: f32 = 12.0;
pub(super) const TASK_CARD_GAP_PX: f32 = 8.0;
pub(super) const TASK_CARD_TOP_ROW_GAP_PX: f32 = 8.0;
pub(super) const TASK_CARD_BORDER_OPACITY: f32 = 0.30;
pub(super) const TASK_CARD_SELECTED_BG_OPACITY: f32 = 0.50;
pub(super) const TASK_CARD_BG_OPACITY: f32 = 0.60;
pub(super) const TASK_CARD_HOVER_BORDER_OPACITY: f32 = 0.50;
pub(super) const TASK_DRAG_PREVIEW_RADIUS_PX: f32 = 6.0;
pub(super) const TASK_DRAG_PREVIEW_BORDER_OPACITY: f32 = 0.45;
pub(super) const TASK_DRAG_PREVIEW_BG_OPACITY: f32 = 0.92;
pub(super) const TASK_CARD_PRIORITY_MARGIN_TOP_PX: f32 = 2.0;
pub(super) const TASK_CARD_TITLE_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const TASK_CARD_DUE_TEXT_SIZE_PX: f32 = 12.0;

pub(super) fn set_task_detail_inputs(
    inputs: [&Entity<InputState>; 5],
    item: &DesktopWorkItem,
    window: &mut Window,
    cx: &mut Context<'_, Workspace>,
) {
    let values = [
        item.title.clone(),
        item.description.clone().unwrap_or_default(),
        item.due_date.clone().unwrap_or_default(),
        item.planning.cycle.clone().unwrap_or_default(),
        item.planning.waiting_on.clone().unwrap_or_default(),
    ];

    for (input, value) in inputs.into_iter().zip(values) {
        input.update(cx, |input, cx| input.set_value(value, window, cx));
    }
}

pub(super) fn task_create_selected_work_item_after_submit(
    current_selection: Option<String>,
    _created_item_id: &str,
) -> Option<String> {
    current_selection
}

pub(super) fn project_filter_selection_after_change(
    next_project_id: Option<String>,
    current_selected_work_item_id: Option<String>,
) -> (Option<String>, Option<String>) {
    (next_project_id, current_selected_work_item_id)
}

impl Workspace {
    pub(super) fn selected_project_name(&self) -> String {
        selected_project_name_like_web(&self.projects, self.selected_project_id.as_deref())
    }

    pub(super) fn filtered_work_items(&self) -> Vec<DesktopWorkItem> {
        self.work_items
            .iter()
            .filter(|item| match self.selected_project_id.as_deref() {
                Some(project_id) => item.project_id.as_deref() == Some(project_id),
                None => true,
            })
            .cloned()
            .collect()
    }

    pub(super) fn selected_work_item(&self) -> Option<DesktopWorkItem> {
        let selected_id = self.selected_work_item_id.as_deref()?;
        self.work_items
            .iter()
            .find(|item| item.id == selected_id)
            .cloned()
    }

    pub(super) fn select_project(
        &mut self,
        project_id: Option<String>,
        cx: &mut Context<'_, Self>,
    ) {
        let (selected_project_id, selected_work_item_id) =
            project_filter_selection_after_change(project_id, self.selected_work_item_id.clone());
        self.selected_project_id = selected_project_id;
        self.selected_work_item_id = selected_work_item_id;
        self.persist_and_notify(cx);
    }

    pub(super) fn create_project(&mut self, cx: &mut Context<'_, Self>) {
        let project = DesktopProject {
            id: uuid::Uuid::new_v4().to_string(),
            name: format!("Project {}", self.projects.len() + 1),
        };
        let (selected_project_id, selected_work_item_id) = project_filter_selection_after_change(
            Some(project.id.clone()),
            self.selected_work_item_id.clone(),
        );
        self.selected_project_id = selected_project_id;
        self.projects.push(project);
        self.selected_work_item_id = selected_work_item_id;
        self.persist_and_notify(cx);
    }

    pub(super) fn delete_project_by_id(&mut self, project_id: &str, cx: &mut Context<'_, Self>) {
        self.dismiss_overlay(|overlay| matches!(overlay, Overlay::ProjectDeleteConfirm { .. }));
        self.projects.retain(|project| project.id != project_id);
        self.work_items
            .retain(|item| item.project_id.as_deref() != Some(project_id));
        if self.selected_project_id.as_deref() == Some(project_id) {
            self.selected_project_id = None;
        }
        if let Some(selected_item_id) = self.selected_work_item_id.as_deref() {
            if !self
                .work_items
                .iter()
                .any(|item| item.id == selected_item_id)
            {
                self.selected_work_item_id = None;
                self.task_detail_menu_open = None;
            }
        }
        self.persist_and_notify(cx);
    }

    pub(super) fn select_work_item_by_id(
        &mut self,
        item_id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(item) = self
            .work_items
            .iter()
            .find(|item| item.id == item_id)
            .cloned()
        {
            self.selected_work_item_id = Some(item_id.to_string());
            self.task_detail_menu_open = None;
            set_task_detail_inputs(
                [
                    &self.task_detail_title_input,
                    &self.task_detail_description_input,
                    &self.task_detail_due_input,
                    &self.task_detail_cycle_input,
                    &self.task_detail_waiting_input,
                ],
                &item,
                window,
                cx,
            );
            self.persist_and_notify(cx);
        }
    }

    pub(super) fn update_selected_work_item<F>(&mut self, update: F, cx: &mut Context<'_, Self>)
    where
        F: FnOnce(&mut DesktopWorkItem) -> bool,
    {
        let Some(item_id) = self.selected_work_item_id.as_deref() else {
            return;
        };
        if let Some(item) = self.work_items.iter_mut().find(|item| item.id == item_id) {
            if update(item) {
                item.updated_at = EpochMs::now();
                self.persist_and_notify(cx);
            }
        }
    }

    pub(super) fn update_selected_work_item_title(
        &mut self,
        title: String,
        cx: &mut Context<'_, Self>,
    ) {
        self.update_selected_work_item(
            |item| {
                if item.title == title {
                    false
                } else {
                    item.title = title;
                    true
                }
            },
            cx,
        );
    }

    pub(super) fn update_selected_work_item_description(
        &mut self,
        description: String,
        cx: &mut Context<'_, Self>,
    ) {
        let description = if description.trim().is_empty() {
            None
        } else {
            Some(description)
        };
        self.update_selected_work_item(
            |item| {
                if item.description == description {
                    false
                } else {
                    item.description = description;
                    true
                }
            },
            cx,
        );
    }

    pub(super) fn update_selected_work_item_due_date(
        &mut self,
        due_date: String,
        cx: &mut Context<'_, Self>,
    ) {
        let due_date = if due_date.trim().is_empty() {
            None
        } else {
            Some(due_date)
        };
        self.update_selected_work_item(
            |item| {
                if item.due_date == due_date {
                    false
                } else {
                    item.due_date = due_date;
                    true
                }
            },
            cx,
        );
    }

    pub(super) fn update_selected_work_item_planning_text(
        &mut self,
        cycle: bool,
        value: String,
        cx: &mut Context<'_, Self>,
    ) {
        let value = (!value.trim().is_empty()).then_some(value);
        self.update_selected_work_item(
            move |item| {
                let target = if cycle {
                    &mut item.planning.cycle
                } else {
                    &mut item.planning.waiting_on
                };
                if *target == value {
                    false
                } else {
                    *target = value;
                    true
                }
            },
            cx,
        );
    }

    pub(super) fn cycle_selected_work_item_estimate(&mut self, cx: &mut Context<'_, Self>) {
        self.update_selected_work_item(
            |item| {
                item.planning.estimate_minutes = match item.planning.estimate_minutes {
                    None => Some(15),
                    Some(15) => Some(30),
                    Some(30) => Some(45),
                    Some(45) => Some(60),
                    Some(60) => Some(90),
                    Some(90) => Some(120),
                    Some(120) => Some(240),
                    _ => None,
                };
                true
            },
            cx,
        );
    }

    pub(super) fn cycle_selected_work_item_recurrence(&mut self, cx: &mut Context<'_, Self>) {
        self.update_selected_work_item(
            |item| {
                item.planning.recurrence = match item.planning.recurrence {
                    None => Some(bruhs_models::RecurrenceRule::Daily),
                    Some(bruhs_models::RecurrenceRule::Daily) => {
                        Some(bruhs_models::RecurrenceRule::Weekdays)
                    }
                    Some(bruhs_models::RecurrenceRule::Weekdays) => {
                        Some(bruhs_models::RecurrenceRule::Weekly)
                    }
                    Some(bruhs_models::RecurrenceRule::Weekly) => {
                        Some(bruhs_models::RecurrenceRule::Monthly)
                    }
                    Some(bruhs_models::RecurrenceRule::Monthly) => None,
                };
                true
            },
            cx,
        );
    }

    pub(super) fn cycle_selected_work_item_dependency(&mut self, cx: &mut Context<'_, Self>) {
        let Some(selected_id) = self.selected_work_item_id.clone() else {
            return;
        };
        let candidates = self
            .work_items
            .iter()
            .filter(|item| item.id != selected_id)
            .map(|item| item.id.clone())
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return;
        }
        self.update_selected_work_item(
            move |item| {
                if let Some(candidate) = candidates
                    .iter()
                    .find(|candidate| !item.planning.dependency_ids.contains(candidate))
                {
                    item.planning.dependency_ids.push(candidate.clone());
                } else {
                    item.planning.dependency_ids.clear();
                }
                true
            },
            cx,
        );
    }

    pub(super) fn update_work_item_status(
        &mut self,
        item_id: &str,
        status: DesktopWorkItemStatus,
        cx: &mut Context<'_, Self>,
    ) {
        if move_work_item_to_status(&mut self.work_items, item_id, status) {
            self.task_detail_menu_open = None;
            self.persist_and_notify(cx);
        }
    }

    pub(super) fn click_task_detail_status_chip(
        &mut self,
        item_id: &str,
        status: DesktopWorkItemStatus,
        cx: &mut Context<'_, Self>,
    ) {
        let recurrence = self
            .work_items
            .iter()
            .find(|item| item.id == item_id)
            .filter(|item| status == DesktopWorkItemStatus::Done && item.status != status)
            .and_then(next_recurring_desktop_item);
        if let Some(item) = self.work_items.iter_mut().find(|item| item.id == item_id) {
            click_task_detail_status_chip_like_web(item, status);
            if let Some(next) = recurrence {
                self.work_items.push(next);
            }
            self.task_detail_menu_open = None;
            self.persist_and_notify(cx);
        }
    }

    pub(super) fn select_task_detail_status_option(
        &mut self,
        item_id: &str,
        status: DesktopWorkItemStatus,
        cx: &mut Context<'_, Self>,
    ) {
        let recurrence = self
            .work_items
            .iter()
            .find(|item| item.id == item_id)
            .filter(|item| status == DesktopWorkItemStatus::Done && item.status != status)
            .and_then(next_recurring_desktop_item);
        if let Some(item) = self.work_items.iter_mut().find(|item| item.id == item_id) {
            let changed = select_task_detail_status_option_like_web(item, status);
            self.task_detail_menu_open = task_detail_menu_after_native_select_selection(
                self.task_detail_menu_open,
                TaskDetailMenu::Status,
            );
            if changed {
                if let Some(next) = recurrence {
                    self.work_items.push(next);
                }
                self.persist_and_notify(cx);
            } else {
                cx.notify();
            }
        }
    }

    pub(super) fn update_work_item_project(
        &mut self,
        item_id: &str,
        project_id: Option<String>,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(item) = self.work_items.iter_mut().find(|item| item.id == item_id) {
            let changed = select_task_detail_project_option_like_web(item, project_id);
            self.task_detail_menu_open = task_detail_menu_after_native_select_selection(
                self.task_detail_menu_open,
                TaskDetailMenu::Project,
            );
            if changed {
                self.persist_and_notify(cx);
            } else {
                cx.notify();
            }
        }
    }

    pub(super) fn update_work_item_priority(
        &mut self,
        item_id: &str,
        priority: DesktopPriority,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(item) = self.work_items.iter_mut().find(|item| item.id == item_id) {
            let changed = select_task_detail_priority_option_like_web(item, priority);
            self.task_detail_menu_open = task_detail_menu_after_native_select_selection(
                self.task_detail_menu_open,
                TaskDetailMenu::Priority,
            );
            if changed {
                self.persist_and_notify(cx);
            } else {
                cx.notify();
            }
        }
    }

    pub(super) fn delete_work_item_by_id(&mut self, item_id: &str, cx: &mut Context<'_, Self>) {
        self.work_items.retain(|item| item.id != item_id);
        if self.selected_work_item_id.as_deref() == Some(item_id) {
            self.selected_work_item_id = None;
            self.task_detail_menu_open = None;
        }
        self.persist_and_notify(cx);
    }

    pub(super) fn request_delete_project_by_id(
        &mut self,
        project_id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if project_task_count(&self.work_items, project_id) == 0 {
            self.delete_project_by_id(project_id, cx);
            return;
        }

        self.open_overlay(Overlay::ProjectDeleteConfirm {
            project_id: project_id.to_string(),
        });
        self.focus_first_modal_tab_stop(window, cx);
        cx.notify();
    }

    pub(super) fn cancel_project_delete(&mut self, cx: &mut Context<'_, Self>) {
        self.dismiss_overlay(|overlay| matches!(overlay, Overlay::ProjectDeleteConfirm { .. }));
        cx.notify();
    }

    pub(super) fn confirm_project_delete(&mut self, cx: &mut Context<'_, Self>) {
        let Some(project_id) = self.project_delete_confirm_id().map(str::to_string) else {
            cx.notify();
            return;
        };
        self.delete_project_by_id(&project_id, cx);
    }

    pub(super) fn set_task_view_mode(
        &mut self,
        mode: DesktopTaskViewMode,
        cx: &mut Context<'_, Self>,
    ) {
        self.task_view_mode = mode;
        self.persist_and_notify(cx);
    }

    pub(super) fn render_tasks_page(&self, compact: bool, cx: &Context<'_, Self>) -> Div {
        div()
            .flex_1()
            .h_full()
            .min_w_0()
            .overflow_hidden()
            .bg(cx.theme().background)
            .flex()
            .flex_row()
            .when(self.sidebar_visible && !compact, |el| {
                el.child(self.render_project_sidebar(cx))
            })
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .child(self.render_tasks_header(cx))
                    .child(self.render_tasks_body(cx)),
            )
            .when_some(self.selected_work_item(), |el, item| {
                if compact {
                    el.child(
                        div()
                            .debug_selector(|| "desktop-compact-task-detail".to_string())
                            .absolute()
                            .right_0()
                            .top_0()
                            .h_full()
                            .max_w_full()
                            .occlude()
                            .child(self.render_task_detail(item, cx)),
                    )
                } else {
                    el.child(self.render_task_detail(item, cx))
                }
            })
    }

    pub(super) fn render_project_sidebar(&self, cx: &Context<'_, Self>) -> Div {
        let project_rows: Vec<AnyElement> = self
            .projects
            .iter()
            .map(|project| self.render_project_row(project, cx))
            .collect();

        div()
            .w(px(WORKSPACE_SIDE_PANEL_WIDTH_PX))
            .min_w(px(WORKSPACE_SIDE_PANEL_MIN_WIDTH_PX))
            .h_full()
            .flex_shrink_0()
            .debug_selector(|| "desktop-project-sidebar".to_string())
            .bg(theme::longan_950())
            .border_r_1()
            .border_color(theme::longan_800().opacity(PROJECT_SIDEBAR_BORDER_OPACITY))
            .text_color(theme::rambutan_300())
            .font_family(PROJECT_SIDEBAR_BODY_FONT_FAMILY)
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .overflow_y_scrollbar()
                    .px(px(PROJECT_SIDEBAR_CONTENT_X_PADDING_PX))
                    .py(px(PROJECT_SIDEBAR_CONTENT_Y_PADDING_PX))
                    .flex()
                    .flex_col()
                    .gap(px(PROJECT_SIDEBAR_ROW_STACK_GAP_PX))
                    .child(
                        div()
                            .id("tasks-all-projects")
                            .debug_selector(|| "desktop-project-all".to_string())
                            .h(px(PROJECT_SIDEBAR_ROW_HEIGHT_PX))
                            .w_full()
                            .flex()
                            .items_center()
                            .rounded(px(PROJECT_SIDEBAR_ROW_RADIUS_PX))
                            .px(px(PROJECT_SIDEBAR_ROW_X_PADDING_PX))
                            .font_family(PROJECT_SIDEBAR_LABEL_FONT_FAMILY)
                            .text_size(px(PROJECT_SIDEBAR_ROW_TEXT_SIZE_PX))
                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                            .text_color(if self.selected_project_id.is_none() {
                                theme::rambutan_200()
                            } else {
                                theme::rambutan_400()
                            })
                            .tab_index(0)
                            .role(Role::Button)
                            .aria_label(TASKS_ALL_PROJECTS_LABEL)
                            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                            .cursor_pointer()
                            .when(self.selected_project_id.is_none(), |el| {
                                el.bg(theme::longan_800()
                                    .opacity(PROJECT_SIDEBAR_ROW_ACTIVE_BG_OPACITY))
                            })
                            .when(self.selected_project_id.is_some(), |el| {
                                el.hover(|el| {
                                    el.bg(theme::longan_800()
                                        .opacity(PROJECT_SIDEBAR_ROW_HOVER_BG_OPACITY))
                                        .text_color(theme::rambutan_300())
                                })
                            })
                            .on_click(cx.listener(|this, _ev, _window, cx| {
                                this.select_project(None, cx);
                            }))
                            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                                    window.prevent_default();
                                    cx.stop_propagation();
                                    this.select_project(None, cx);
                                }
                            }))
                            .child(TASKS_ALL_PROJECTS_LABEL),
                    )
                    .children(project_rows),
            )
            .child(
                div()
                    .px(px(PROJECT_SIDEBAR_FOOTER_CONTAINER_X_PADDING_PX))
                    .pb(px(PROJECT_SIDEBAR_FOOTER_CONTAINER_BOTTOM_PADDING_PX))
                    .flex_shrink_0()
                    .child(
                        div()
                            .id("tasks-new-project")
                            .debug_selector(|| "desktop-project-new".to_string())
                            .h(px(PROJECT_SIDEBAR_FOOTER_HEIGHT_PX))
                            .w_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap(px(PROJECT_SIDEBAR_FOOTER_GAP_PX))
                            .rounded(px(PROJECT_SIDEBAR_ROW_RADIUS_PX))
                            .border_1()
                            .border_dashed()
                            .border_color(
                                theme::longan_700().opacity(PROJECT_SIDEBAR_FOOTER_BORDER_OPACITY),
                            )
                            .px(px(PROJECT_SIDEBAR_FOOTER_X_PADDING_PX))
                            .py(px(PROJECT_SIDEBAR_FOOTER_Y_PADDING_PX))
                            .font_family(PROJECT_SIDEBAR_BODY_FONT_FAMILY)
                            .text_size(px(PROJECT_SIDEBAR_FOOTER_TEXT_SIZE_PX))
                            .text_color(theme::rambutan_500())
                            .tab_index(0)
                            .role(Role::Button)
                            .aria_label(TASKS_NEW_PROJECT_LABEL)
                            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                            .cursor_pointer()
                            .hover(|el| {
                                el.border_color(
                                    theme::rambutan_500()
                                        .opacity(PROJECT_SIDEBAR_FOOTER_HOVER_BORDER_OPACITY),
                                )
                                .text_color(theme::rambutan_400())
                            })
                            .tooltip(|window, cx| {
                                Tooltip::new(TASKS_NEW_PROJECT_LABEL).build(window, cx)
                            })
                            .on_click(cx.listener(|this, _ev, _window, cx| {
                                this.create_project(cx);
                            }))
                            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                                    window.prevent_default();
                                    cx.stop_propagation();
                                    this.create_project(cx);
                                }
                            }))
                            .child(
                                div()
                                    .text_size(px(PROJECT_SIDEBAR_FOOTER_ICON_TEXT_SIZE_PX))
                                    .child("+"),
                            )
                            .child(div().child(TASKS_NEW_PROJECT_LABEL)),
                    ),
            )
    }

    pub(super) fn render_project_row(
        &self,
        project: &DesktopProject,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let is_active = self.selected_project_id.as_deref() == Some(project.id.as_str());
        let id_for_select = project.id.clone();
        let id_for_select_key = project.id.clone();
        let id_for_delete = project.id.clone();
        let id_for_delete_key = project.id.clone();
        let key = element_key(&project.id);
        let hover_group: SharedString = format!("desktop-project-row-{key}").into();

        div()
            .id(("tasks-project-row", key))
            .debug_selector(move || format!("desktop-project-row-{key}"))
            .group(hover_group.clone())
            .h(px(PROJECT_SIDEBAR_ROW_HEIGHT_PX))
            .w_full()
            .flex()
            .items_center()
            .gap(px(PROJECT_SIDEBAR_ROW_GAP_PX))
            .rounded(px(PROJECT_SIDEBAR_ROW_RADIUS_PX))
            .px(px(PROJECT_SIDEBAR_ROW_X_PADDING_PX))
            .font_family(PROJECT_SIDEBAR_LABEL_FONT_FAMILY)
            .text_size(px(PROJECT_SIDEBAR_ROW_TEXT_SIZE_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(if is_active {
                theme::rambutan_200()
            } else {
                theme::rambutan_400()
            })
            .tab_index(0)
            .role(Role::Button)
            .aria_label(format!("Select project {}", project.name))
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .when(is_active, |el| {
                el.bg(theme::longan_800().opacity(PROJECT_SIDEBAR_ROW_ACTIVE_BG_OPACITY))
            })
            .when(!is_active, |el| {
                el.hover(|el| {
                    el.bg(theme::longan_800().opacity(PROJECT_SIDEBAR_ROW_HOVER_BG_OPACITY))
                        .text_color(theme::rambutan_300())
                })
            })
            .on_click(cx.listener(move |this, _ev, _window, cx| {
                this.select_project(Some(id_for_select.clone()), cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.select_project(Some(id_for_select_key.clone()), cx);
                }
            }))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .child(project.name.clone()),
            )
            .child(
                div()
                    .id(("tasks-project-delete", key))
                    .debug_selector(move || format!("desktop-project-delete-{key}"))
                    .invisible()
                    .flex_shrink_0()
                    .rounded(px(PROJECT_SIDEBAR_ROW_ACTION_RADIUS_PX))
                    .p(px(PROJECT_SIDEBAR_ROW_ACTION_PADDING_PX))
                    .text_color(theme::rambutan_500())
                    .tab_index(0)
                    .role(Role::ListBoxOption)
                    .aria_label(PROJECT_DELETE_LABEL)
                    .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .group_hover(hover_group, |el| el.visible())
                    .hover(|el| {
                        el.bg(theme::longan_700()
                            .opacity(PROJECT_SIDEBAR_ROW_ACTION_HOVER_BG_OPACITY))
                            .text_color(theme::lychee_400())
                    })
                    .tooltip(|window, cx| Tooltip::new(PROJECT_DELETE_LABEL).build(window, cx))
                    .on_click(cx.listener(move |this, _ev, window, cx| {
                        cx.stop_propagation();
                        this.request_delete_project_by_id(&id_for_delete, window, cx);
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.request_delete_project_by_id(&id_for_delete_key, window, cx);
                        }
                    }))
                    .child(
                        Icon::new(IconName::Delete).size(px(PROJECT_SIDEBAR_DELETE_ICON_SIZE_PX)),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_tasks_header(&self, cx: &Context<'_, Self>) -> Div {
        div()
            .min_h(px(TASKS_HEADER_MIN_HEIGHT_PX))
            .px(px(TASKS_HEADER_X_PADDING_PX))
            .py(px(TASKS_HEADER_Y_PADDING_PX))
            .flex()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(theme::longan_800().opacity(TASKS_HEADER_BORDER_OPACITY))
            .flex_shrink_0()
            .font_family(TASKS_HEADER_BODY_FONT_FAMILY)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(TASKS_HEADER_TITLE_TEXT_SIZE_PX))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::rambutan_400())
                    .child(self.selected_project_name()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(TASKS_HEADER_CONTROL_GAP_PX))
                    .child(self.render_task_view_mode_button(DesktopTaskViewMode::List, cx))
                    .child(self.render_task_view_mode_button(DesktopTaskViewMode::Board, cx))
                    .child(
                        div()
                            .id("tasks-open-create")
                            .debug_selector(|| "desktop-task-open-create".to_string())
                            .ml(px(TASKS_HEADER_NEW_TASK_LEFT_MARGIN_PX))
                            .rounded(px(TASKS_HEADER_CONTROL_RADIUS_PX))
                            .border_1()
                            .border_color(
                                theme::persimmon_400()
                                    .opacity(TASKS_HEADER_NEW_TASK_BORDER_OPACITY),
                            )
                            .bg(theme::persimmon_500().opacity(TASKS_HEADER_NEW_TASK_BG_OPACITY))
                            .px(px(TASKS_HEADER_CONTROL_X_PADDING_PX))
                            .py(px(TASKS_HEADER_CONTROL_Y_PADDING_PX))
                            .text_size(px(TASKS_HEADER_NEW_TASK_TEXT_SIZE_PX))
                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::persimmon_200())
                            .tab_index(0)
                            .role(Role::Button)
                            .aria_label(TASKS_NEW_TASK_LABEL)
                            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                            .cursor_pointer()
                            .hover(|el| {
                                el.bg(theme::persimmon_500()
                                    .opacity(TASKS_HEADER_NEW_TASK_HOVER_BG_OPACITY))
                            })
                            .tooltip(|window, cx| {
                                Tooltip::new(TASKS_NEW_TASK_LABEL).build(window, cx)
                            })
                            .on_click(cx.listener(|this, _ev, window, cx| {
                                this.open_task_create(window, cx);
                            }))
                            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                                    window.prevent_default();
                                    cx.stop_propagation();
                                    this.open_task_create(window, cx);
                                }
                            }))
                            .child("+ New"),
                    ),
            )
    }

    pub(super) fn render_task_view_mode_button(
        &self,
        mode: DesktopTaskViewMode,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let is_active = self.task_view_mode == mode;
        div()
            .id(("tasks-view-mode", mode as usize))
            .debug_selector(move || format!("desktop-task-view-{}", mode.as_str()))
            .rounded(px(TASKS_HEADER_CONTROL_RADIUS_PX))
            .px(px(TASKS_HEADER_CONTROL_X_PADDING_PX))
            .py(px(TASKS_HEADER_CONTROL_Y_PADDING_PX))
            .font_family(TASKS_HEADER_BODY_FONT_FAMILY)
            .text_size(px(TASKS_HEADER_MODE_TEXT_SIZE_PX))
            .font_weight(FontWeight::MEDIUM)
            .text_color(if is_active {
                theme::rambutan_200()
            } else {
                theme::rambutan_500()
            })
            .tab_index(0)
            .role(Role::Tab)
            .aria_label(mode.label())
            .aria_selected(is_active)
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .when(is_active, |el| {
                el.bg(theme::longan_800().opacity(TASKS_HEADER_MODE_ACTIVE_BG_OPACITY))
            })
            .when(!is_active, |el| {
                el.hover(|el| el.text_color(theme::rambutan_400()))
            })
            .on_click(cx.listener(move |this, _ev, _window, cx| {
                this.set_task_view_mode(mode, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.set_task_view_mode(mode, cx);
                }
            }))
            .child(mode.label())
            .into_any_element()
    }

    pub(super) fn render_tasks_body(&self, cx: &Context<'_, Self>) -> AnyElement {
        match self.task_view_mode {
            DesktopTaskViewMode::List => self.render_task_list(cx),
            DesktopTaskViewMode::Board => self.render_task_board(cx),
        }
    }

    pub(super) fn render_task_list(&self, cx: &Context<'_, Self>) -> AnyElement {
        let items = self.filtered_work_items();
        let rows: Vec<AnyElement> = items
            .iter()
            .map(|item| self.render_task_row(item, cx))
            .collect();

        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(TASK_LIST_HEADER_HEIGHT_PX))
                    .px(px(TASK_LIST_HEADER_X_PADDING_PX))
                    .flex()
                    .items_center()
                    .gap(px(TASK_LIST_HEADER_GAP_PX))
                    .border_b_1()
                    .border_color(theme::longan_800().opacity(TASK_LIST_HEADER_BORDER_OPACITY))
                    .bg(theme::longan_950())
                    .child(div().w(px(TASK_LIST_STATUS_SLOT_WIDTH_PX)))
                    .child(
                        div()
                            .flex_1()
                            .font_family(TASK_LIST_BODY_FONT_FAMILY)
                            .text_size(px(TASK_LIST_HEADER_TEXT_SIZE_PX))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::rambutan_500())
                            .child("TITLE"),
                    )
                    .child(
                        div()
                            .w(px(TASK_LIST_PRIORITY_COLUMN_WIDTH_PX))
                            .flex_shrink_0()
                            .text_center()
                            .font_family(TASK_LIST_BODY_FONT_FAMILY)
                            .text_size(px(TASK_LIST_HEADER_TEXT_SIZE_PX))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::rambutan_500())
                            .child("PRI"),
                    )
                    .child(
                        div()
                            .w(px(TASK_LIST_DUE_COLUMN_WIDTH_PX))
                            .flex_shrink_0()
                            .text_right()
                            .font_family(TASK_LIST_BODY_FONT_FAMILY)
                            .text_size(px(TASK_LIST_HEADER_TEXT_SIZE_PX))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::rambutan_500())
                            .child("DUE"),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .when(rows.is_empty(), |el| {
                        el.child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .justify_center()
                                .gap_3()
                                .py(px(TASK_LIST_EMPTY_Y_PADDING_PX))
                                .font_family(TASK_LIST_BODY_FONT_FAMILY)
                                .text_size(px(TASK_LIST_EMPTY_TEXT_SIZE_PX))
                                .text_color(theme::rambutan_500())
                                .child("No tasks yet")
                                .child(
                                    web_button(
                                        "desktop-task-list-empty-action",
                                        WebButtonVariant::Outline,
                                        WebButtonSize::Small,
                                        ButtonState::Enabled,
                                    )
                                    .aria_label("Create task")
                                    .on_click(cx.listener(|this, _event, window, cx| {
                                        this.open_task_create(window, cx);
                                    }))
                                    .child("Create task"),
                                ),
                        )
                    })
                    .child(div().py(px(TASK_LIST_BODY_Y_PADDING_PX)).children(rows)),
            )
            .into_any_element()
    }

    pub(super) fn render_task_row(
        &self,
        item: &DesktopWorkItem,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let is_selected = self.selected_work_item_id.as_deref() == Some(item.id.as_str());
        let key = element_key(&item.id);
        let item_id = item.id.clone();
        let item_id_for_key = item.id.clone();
        let due = item
            .due_date
            .as_deref()
            .and_then(|date| format_desktop_due_date(date, today_date()));
        let status_label = item.status.label();
        let priority_label = item.priority.label();

        div()
            .id(("tasks-row", key))
            .debug_selector(move || format!("desktop-task-row-{key}"))
            .rounded(px(TASK_ROW_RADIUS_PX))
            .px(px(TASK_ROW_X_PADDING_PX))
            .py(px(TASK_ROW_Y_PADDING_PX))
            .flex()
            .items_center()
            .gap(px(TASK_ROW_GAP_PX))
            .tab_index(0)
            .role(Role::Button)
            .aria_label(format!("Open task {}", item.title))
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .when(is_selected, |el| {
                el.bg(theme::longan_800().opacity(TASK_ROW_SELECTED_BG_OPACITY))
            })
            .when(!is_selected, |el| {
                el.hover(|el| el.bg(theme::longan_800().opacity(TASK_ROW_HOVER_BG_OPACITY)))
            })
            .on_click(cx.listener(move |this, _ev, window, cx| {
                this.select_work_item_by_id(&item_id, window, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.select_work_item_by_id(&item_id_for_key, window, cx);
                }
            }))
            .child(
                div()
                    .w(px(TASK_LIST_STATUS_SLOT_WIDTH_PX))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .id(("tasks-row-status", key))
                            .size(px(TASK_ROW_STATUS_DOT_SIZE_PX))
                            .rounded(px(TASK_ROW_STATUS_DOT_RADIUS_PX))
                            .bg(status_dot_color(item.status))
                            .tooltip(move |window, cx| {
                                Tooltip::new(status_label).build(window, cx)
                            }),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(TASK_ROW_BODY_FONT_FAMILY)
                    .text_size(px(TASK_ROW_TITLE_TEXT_SIZE_PX))
                    .text_color(theme::rambutan_200())
                    .child(item.title.clone()),
            )
            .child(
                div()
                    .id(("tasks-row-priority", key))
                    .w(px(TASK_LIST_PRIORITY_COLUMN_WIDTH_PX))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .tooltip(move |window, cx| Tooltip::new(priority_label).build(window, cx))
                    .child(self.render_priority_bars(item.priority)),
            )
            .child(
                div()
                    .w(px(TASK_LIST_DUE_COLUMN_WIDTH_PX))
                    .flex_shrink_0()
                    .text_right()
                    .font_family(TASK_ROW_BODY_FONT_FAMILY)
                    .text_size(px(TASK_ROW_DUE_TEXT_SIZE_PX))
                    .text_color(
                        due.as_ref()
                            .map_or(theme::rambutan_500(), |(_, color)| *color),
                    )
                    .child(due.map_or_else(String::new, |(label, _)| label)),
            )
            .into_any_element()
    }

    pub(super) fn render_priority_bars(&self, priority: DesktopPriority) -> Div {
        let color = priority_color(priority);
        let filled = priority.bars();
        let bars = (1u8..=4)
            .map(|index| {
                let height = TASK_PRIORITY_BAR_HEIGHTS_PX[(index - 1) as usize];
                div()
                    .w(px(TASK_PRIORITY_BAR_WIDTH_PX))
                    .h(px(height))
                    .rounded(px(TASK_PRIORITY_BAR_RADIUS_PX))
                    .bg(if index <= filled {
                        color
                    } else {
                        color.opacity(TASK_PRIORITY_EMPTY_OPACITY)
                    })
                    .into_any_element()
            })
            .collect::<Vec<_>>();

        div()
            .flex()
            .items_end()
            .gap(px(TASK_PRIORITY_BAR_GAP_PX))
            .children(bars)
    }

    pub(super) fn render_task_board(&self, cx: &Context<'_, Self>) -> AnyElement {
        let columns: Vec<AnyElement> = DesktopWorkItemStatus::board_columns()
            .iter()
            .map(|status| self.render_task_board_column(*status, cx))
            .collect();

        div()
            .flex_1()
            .min_h_0()
            .overflow_x_scrollbar()
            .debug_selector(|| "desktop-task-board".to_string())
            .p(px(TASK_BOARD_PADDING_PX))
            .flex()
            .gap(px(TASK_BOARD_GAP_PX))
            .children(columns)
            .into_any_element()
    }

    pub(super) fn render_task_board_column(
        &self,
        status: DesktopWorkItemStatus,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let items: Vec<DesktopWorkItem> = self
            .filtered_work_items()
            .into_iter()
            .filter(|item| item.status == status)
            .collect();
        let cards: Vec<AnyElement> = items
            .iter()
            .map(|item| self.render_task_card(item, cx))
            .collect();
        let opacity = if status.is_completed() {
            TASK_BOARD_COLUMN_COMPLETED_OPACITY
        } else {
            1.0
        };

        div()
            .id(("tasks-board-column", element_key(status.as_str())))
            .debug_selector(move || format!("desktop-task-board-column-{}", status.as_str()))
            .flex_1()
            .min_w(px(TASK_BOARD_COLUMN_MIN_WIDTH_PX))
            .h_full()
            .rounded(px(TASK_BOARD_COLUMN_RADIUS_PX))
            .overflow_hidden()
            .bg(if status.is_completed() {
                theme::longan_950().opacity(TASK_BOARD_COLUMN_COMPLETED_BG_OPACITY)
            } else {
                theme::longan_900().opacity(TASK_BOARD_COLUMN_ACTIVE_BG_OPACITY)
            })
            .opacity(opacity)
            .flex()
            .flex_col()
            .drag_over::<DesktopTaskDrag>(|el, _drag, _window, _cx| {
                el.border(px(TASK_BOARD_DRAG_OVER_RING_PX))
                    .border_color(theme::persimmon_400().opacity(TASK_BOARD_DRAG_OVER_RING_OPACITY))
            })
            .on_drop(
                cx.listener(move |this, drag: &DesktopTaskDrag, _window, cx| {
                    this.update_work_item_status(&drag.item_id, status, cx);
                }),
            )
            .child(
                div()
                    .px(px(TASK_BOARD_HEADER_X_PADDING_PX))
                    .py(px(TASK_BOARD_HEADER_Y_PADDING_PX))
                    .flex()
                    .items_center()
                    .gap(px(TASK_BOARD_HEADER_GAP_PX))
                    .font_family(TASK_BOARD_BODY_FONT_FAMILY)
                    .text_size(px(TASK_BOARD_HEADER_TEXT_SIZE_PX))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::rambutan_400())
                    .child(
                        div()
                            .size(px(TASK_BOARD_HEADER_DOT_SIZE_PX))
                            .rounded(px(TASK_BOARD_HEADER_DOT_RADIUS_PX))
                            .bg(status_dot_color(status)),
                    )
                    .child(status.label())
                    .child(
                        div()
                            .rounded(px(TASK_BOARD_COUNT_RADIUS_PX))
                            .px(px(TASK_BOARD_COUNT_X_PADDING_PX))
                            .text_size(px(TASK_BOARD_COUNT_TEXT_SIZE_PX))
                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                            .bg(theme::longan_800().opacity(TASK_BOARD_COUNT_BG_OPACITY))
                            .text_color(theme::rambutan_500())
                            .font(theme::tabular_system_font())
                            .child(items.len().to_string()),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .p(px(TASK_BOARD_BODY_PADDING_PX))
                    .flex()
                    .flex_col()
                    .gap(px(TASK_BOARD_BODY_GAP_PX))
                    .when(cards.is_empty(), |el| {
                        el.child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap_2()
                                .py(px(TASK_BOARD_EMPTY_Y_PADDING_PX))
                                .text_center()
                                .font_family(TASK_BOARD_BODY_FONT_FAMILY)
                                .text_size(px(TASK_BOARD_EMPTY_TEXT_SIZE_PX))
                                .text_color(theme::rambutan_500())
                                .child("No tasks")
                                .child(
                                    web_button(
                                        ("desktop-task-board-empty-action", status as usize),
                                        WebButtonVariant::Ghost,
                                        WebButtonSize::Small,
                                        ButtonState::Enabled,
                                    )
                                    .aria_label(format!("Create {} task", status.label()))
                                    .on_click(cx.listener(move |this, _event, window, cx| {
                                        this.open_task_create_with_status(status, window, cx);
                                    }))
                                    .child("Create task"),
                                ),
                        )
                    })
                    .children(cards),
            )
            .into_any_element()
    }

    pub(super) fn render_task_card(
        &self,
        item: &DesktopWorkItem,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let is_selected = self.selected_work_item_id.as_deref() == Some(item.id.as_str());
        let key = element_key(&item.id);
        let item_id = item.id.clone();
        let item_id_for_key = item.id.clone();
        let debug_item_id = item.id.clone();
        let drag = DesktopTaskDrag::new(item);
        let priority_label = item.priority.label();
        let due = item
            .due_date
            .as_deref()
            .and_then(|date| format_desktop_due_date(date, today_date()));

        div()
            .id(("tasks-card", key))
            .debug_selector(move || format!("desktop-task-card-{}", element_key(&debug_item_id)))
            .rounded(px(TASK_CARD_RADIUS_PX))
            .border_1()
            .border_color(if is_selected {
                theme::longan_600()
            } else {
                theme::longan_700().opacity(TASK_CARD_BORDER_OPACITY)
            })
            .bg(if is_selected {
                theme::longan_800().opacity(TASK_CARD_SELECTED_BG_OPACITY)
            } else {
                theme::longan_900().opacity(TASK_CARD_BG_OPACITY)
            })
            .p(px(TASK_CARD_PADDING_PX))
            .flex()
            .flex_col()
            .gap(px(TASK_CARD_GAP_PX))
            .tab_index(0)
            .role(Role::Button)
            .aria_label(format!("Open task {}", item.title))
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .when(!is_selected, |el| {
                el.hover(|el| {
                    el.border_color(theme::longan_600().opacity(TASK_CARD_HOVER_BORDER_OPACITY))
                })
            })
            .on_drag(drag, |drag: &DesktopTaskDrag, _offset, _window, cx| {
                cx.stop_propagation();
                cx.new(|_| drag.clone())
            })
            .on_click(cx.listener(move |this, _ev, window, cx| {
                this.select_work_item_by_id(&item_id, window, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.select_work_item_by_id(&item_id_for_key, window, cx);
                }
            }))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(TASK_CARD_TOP_ROW_GAP_PX))
                    .child(
                        div()
                            .id(("tasks-card-priority", key))
                            .mt(px(TASK_CARD_PRIORITY_MARGIN_TOP_PX))
                            .flex_shrink_0()
                            .tooltip(move |window, cx| {
                                Tooltip::new(priority_label).build(window, cx)
                            })
                            .child(self.render_priority_bars(item.priority)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .line_clamp(2)
                            .font_family(TASK_BOARD_BODY_FONT_FAMILY)
                            .text_size(px(TASK_CARD_TITLE_TEXT_SIZE_PX))
                            .text_color(theme::rambutan_200())
                            .child(item.title.clone()),
                    ),
            )
            .when_some(due, |el, (label, color)| {
                el.child(
                    div()
                        .font_family(TASK_BOARD_BODY_FONT_FAMILY)
                        .text_size(px(TASK_CARD_DUE_TEXT_SIZE_PX))
                        .text_color(color)
                        .child(label),
                )
            })
            .into_any_element()
    }
}
