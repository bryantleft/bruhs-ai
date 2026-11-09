use crate::{
    assets, theme,
    views::task_registry_subscription::TaskRegistrySubscription,
    views::web_button::{
        is_web_button_activation_key, web_button, ButtonState, WebButtonSize, WebButtonVariant,
    },
};
use bruhs_core::{AppState, BackgroundTask, TaskStatus};
use bruhs_models::time::{format_relative_ms, RelativeTimeStyle};
use bruhs_models::EpochMs;
use gpui::prelude::*;
use gpui::*;
use gpui_component::scroll::ScrollableElement;
use gpui_component::tooltip::Tooltip;
use std::sync::Arc;

pub struct TasksPanelView {
    app: Arc<AppState>,
    tasks: Vec<BackgroundTask>,
    expanded_task_id: Option<String>,
    _registry_subscription: TaskRegistrySubscription,
    _event_task: Task<()>,
}

pub enum TasksPanelEvent {
    CloseRequested,
    StartConversationRequested,
}

impl EventEmitter<TasksPanelEvent> for TasksPanelView {}

const TASKS_PANEL_EMPTY_LABEL: &str = "No background tasks";
const TASKS_PANEL_RUNNING_LABEL: &str = "Running";
const TASKS_PANEL_RECENT_LABEL: &str = "Recent";
const TASKS_PANEL_RUNNING_EMPTY_LOG: &str = "No output yet...";
const TASKS_PANEL_RECENT_EMPTY_LOG: &str = "No output.";
const TASKS_PANEL_DISMISS_TOOLTIP: &str = "Dismiss";
const TASKS_PANEL_CLOSE_LABEL: &str = "Close tasks panel";
const TASKS_PANEL_BODY_FONT_FAMILY: &str = "Monaspace Neon";
const TASKS_PANEL_LABEL_FONT_FAMILY: &str = "Monaspace Argon";
const TASKS_PANEL_LOG_FONT_FAMILY: &str = "Monaspace Neon";
const TASKS_PANEL_HEADER_TEXT_SIZE_PX: f32 = 13.0;
const TASKS_PANEL_EMPTY_TEXT_SIZE_PX: f32 = 12.0;
const TASKS_PANEL_SECTION_TEXT_SIZE_PX: f32 = 12.0;
const TASKS_PANEL_ROW_LABEL_TEXT_SIZE_PX: f32 = 14.0;
const TASKS_PANEL_TIME_TEXT_SIZE_PX: f32 = 12.0;
const TASKS_PANEL_STATUS_TEXT_SIZE_PX: f32 = 12.0;
const TASKS_PANEL_LOG_TEXT_SIZE_PX: f32 = 12.0;
const TASKS_PANEL_RADIUS_PX: f32 = 8.0;
const TASKS_PANEL_HEADER_HEIGHT_PX: f32 = 42.0;
const TASKS_PANEL_ICON_SIZE_PX: f32 = 14.0;
const TASKS_PANEL_CLOSE_BUTTON_SIZE_PX: f32 = 32.0;
const TASKS_PANEL_LOG_MAX_HEIGHT_PX: f32 = 128.0;
const TASKS_PANEL_DIVIDER_OPACITY: f32 = 0.50;
const TASKS_PANEL_HOVER_BG_OPACITY: f32 = 0.50;
const TASKS_PANEL_RUNNING_DOT_GAP_PX: f32 = 2.0;
const TASKS_PANEL_RUNNING_DOT_SIZE_PX: f32 = 4.0;

impl TasksPanelView {
    pub fn new(app: Arc<AppState>, cx: &mut Context<'_, Self>) -> Self {
        let tasks = tasks_panel_snapshot_like_web(app.registry().get_all());
        let (registry_subscription, task_events) =
            TaskRegistrySubscription::new(app.registry().clone());
        let app_for_events = Arc::clone(&app);
        let _event_task = cx.spawn(async move |panel, cx| {
            while task_events.recv().await.is_ok() {
                let tasks = tasks_panel_snapshot_like_web(app_for_events.registry().get_all());
                if panel
                    .update(cx, |panel, cx| {
                        panel.tasks = tasks;
                        panel.prune_expanded_task();
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        Self {
            app,
            tasks,
            expanded_task_id: None,
            _registry_subscription: registry_subscription,
            _event_task,
        }
    }

    fn toggle_task_expansion(&mut self, task_id: String, cx: &mut Context<'_, Self>) {
        if self.expanded_task_id.as_deref() == Some(task_id.as_str()) {
            self.expanded_task_id = None;
        } else {
            self.expanded_task_id = Some(task_id);
        }
        cx.notify();
    }

    fn dismiss_task(&mut self, task_id: &str, cx: &mut Context<'_, Self>) {
        // The panel offers dismiss only for finished tasks; a task that is
        // already gone or still running just leaves the local list.
        if let Err(error) = self.app.registry().remove(task_id) {
            tracing::warn!("dismiss task: {error}");
        }
        self.tasks.retain(|task| task.id != task_id);
        if self.expanded_task_id.as_deref() == Some(task_id) {
            self.expanded_task_id = None;
        }
        cx.notify();
    }

    fn prune_expanded_task(&mut self) {
        if let Some(expanded_id) = self.expanded_task_id.as_deref() {
            if !self.tasks.iter().any(|task| task.id == expanded_id) {
                self.expanded_task_id = None;
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn expanded_task_id_for_test(&self) -> Option<&str> {
        self.expanded_task_id.as_deref()
    }

    #[cfg(test)]
    pub(crate) fn has_task_for_test(&self, task_id: &str) -> bool {
        self.tasks.iter().any(|task| task.id == task_id)
    }

    fn running_tasks(&self) -> Vec<BackgroundTask> {
        self.tasks
            .iter()
            .filter(|task| task.status == TaskStatus::Running)
            .cloned()
            .collect()
    }

    fn recent_tasks(&self) -> Vec<BackgroundTask> {
        self.tasks
            .iter()
            .filter(|task| task.status != TaskStatus::Running)
            .cloned()
            .collect()
    }

    fn render_task_sections(&self, cx: &Context<'_, Self>) -> AnyElement {
        let running = self.running_tasks();
        let recent = self.recent_tasks();

        if running.is_empty() && recent.is_empty() {
            return div()
                .h_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .py_8()
                .font_family(TASKS_PANEL_LABEL_FONT_FAMILY)
                .text_size(px(TASKS_PANEL_EMPTY_TEXT_SIZE_PX))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .text_color(theme::rambutan_500())
                .child(TASKS_PANEL_EMPTY_LABEL)
                .child(
                    web_button(
                        "desktop-background-tasks-empty-action",
                        WebButtonVariant::Outline,
                        WebButtonSize::Small,
                        ButtonState::Enabled,
                    )
                    .aria_label("Start conversation")
                    .on_click(cx.listener(|_this, _event, _window, cx| {
                        cx.emit(TasksPanelEvent::StartConversationRequested);
                    }))
                    .child("Start conversation"),
                )
                .into_any_element();
        }

        div()
            .flex()
            .flex_col()
            .gap_4()
            .when(!running.is_empty(), |el| {
                el.child(self.render_task_section(TASKS_PANEL_RUNNING_LABEL, running, false, cx))
            })
            .when(!recent.is_empty(), |el| {
                el.child(self.render_task_section(TASKS_PANEL_RECENT_LABEL, recent, true, cx))
            })
            .into_any_element()
    }

    fn render_task_section(
        &self,
        title: &'static str,
        tasks: Vec<BackgroundTask>,
        dismissible: bool,
        cx: &Context<'_, Self>,
    ) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .px_3()
                    .py_1p5()
                    .font_family(TASKS_PANEL_LABEL_FONT_FAMILY)
                    .text_size(px(TASKS_PANEL_SECTION_TEXT_SIZE_PX))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::rambutan_500())
                    .child(title),
            )
            .children(tasks.into_iter().map(|task| {
                self.render_task_row(task, dismissible, cx)
                    .into_any_element()
            }))
    }

    fn render_task_row(
        &self,
        task: BackgroundTask,
        dismissible: bool,
        cx: &Context<'_, Self>,
    ) -> Div {
        let is_expanded = self.expanded_task_id.as_deref() == Some(task.id.as_str());
        let task_id_for_expand = task.id.clone();
        let task_id_for_expand_key = task.id.clone();
        let task_id_for_dismiss = task.id.clone();
        let task_id_for_dismiss_key = task.id.clone();
        let task_key = element_key(&task.id);
        let hover_group: SharedString = format!("desktop-background-task-row-{task_key}").into();
        let timestamp = tasks_panel_timestamp_like_web(&task, EpochMs::now());
        let success = desktop_task_success_like_web(task.exit_code);
        let status = task.status.clone();
        let log_tail = if task.log_tail.is_empty() {
            if task.status == TaskStatus::Running {
                TASKS_PANEL_RUNNING_EMPTY_LOG.to_string()
            } else {
                TASKS_PANEL_RECENT_EMPTY_LOG.to_string()
            }
        } else {
            task.log_tail.clone()
        };
        let expand_label = format!("Toggle details for {}", task.label);

        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .group(hover_group.clone())
                    .debug_selector(move || format!("desktop-background-task-row-{task_key}"))
                    .flex()
                    .items_center()
                    .rounded(px(TASKS_PANEL_RADIUS_PX))
                    .px_3()
                    .py_2()
                    .hover(|el| el.bg(theme::longan_800().opacity(TASKS_PANEL_HOVER_BG_OPACITY)))
                    .child(
                        div()
                            .id(("desktop-task-expand-row", element_key(&task.id)))
                            .debug_selector(move || {
                                format!("desktop-background-task-expand-{task_key}")
                            })
                            .flex()
                            .items_center()
                            .gap_2()
                            .flex_1()
                            .min_w_0()
                            .tab_index(0)
                            .role(Role::Button)
                            .aria_label(expand_label)
                            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _ev, _window, cx| {
                                this.toggle_task_expansion(task_id_for_expand.clone(), cx);
                            }))
                            .on_key_down(cx.listener(
                                move |this, event: &KeyDownEvent, window, cx| {
                                    if is_web_button_activation_key(event.keystroke.key.as_str()) {
                                        window.prevent_default();
                                        cx.stop_propagation();
                                        this.toggle_task_expansion(
                                            task_id_for_expand_key.clone(),
                                            cx,
                                        );
                                    }
                                },
                            ))
                            .child(render_task_status_indicator(&status, success))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_family(TASKS_PANEL_BODY_FONT_FAMILY)
                                    .text_size(px(TASKS_PANEL_ROW_LABEL_TEXT_SIZE_PX))
                                    .text_color(theme::rambutan_200())
                                    .child(task.label.clone()),
                            )
                            .child(
                                div()
                                    .font_family(TASKS_PANEL_LABEL_FONT_FAMILY)
                                    .text_size(px(TASKS_PANEL_TIME_TEXT_SIZE_PX))
                                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                                    .text_color(theme::rambutan_500())
                                    .child(timestamp),
                            ),
                    )
                    .when(dismissible, |el| {
                        el.child(
                            div()
                                .ml_2()
                                .invisible()
                                .group_hover(hover_group, |el| el.visible())
                                .child(
                                    web_button(
                                        ("desktop-task-dismiss", task_key),
                                        WebButtonVariant::Ghost,
                                        WebButtonSize::IconDefault,
                                        ButtonState::Enabled,
                                    )
                                    .debug_selector(move || {
                                        format!("desktop-background-task-dismiss-{task_key}")
                                    })
                                    .aria_label(TASKS_PANEL_DISMISS_TOOLTIP)
                                    .tooltip(|window, cx| {
                                        Tooltip::new(TASKS_PANEL_DISMISS_TOOLTIP).build(window, cx)
                                    })
                                    .tab_index(0)
                                    .role(Role::Button)
                                    .focus_visible(|el| {
                                        el.border_2().border_color(theme::persimmon_400())
                                    })
                                    .on_click(cx.listener(move |this, _ev, _window, cx| {
                                        cx.stop_propagation();
                                        this.dismiss_task(&task_id_for_dismiss, cx);
                                    }))
                                    .on_key_down(cx.listener(
                                        move |this, event: &KeyDownEvent, window, cx| {
                                            if is_web_button_activation_key(
                                                event.keystroke.key.as_str(),
                                            ) {
                                                window.prevent_default();
                                                cx.stop_propagation();
                                                this.dismiss_task(&task_id_for_dismiss_key, cx);
                                            }
                                        },
                                    ))
                                    .child("✕"),
                                ),
                        )
                    }),
            )
            .when(is_expanded, |el| {
                el.child(
                    div()
                        .mx_3()
                        .debug_selector(move || format!("desktop-background-task-log-{task_key}"))
                        .rounded(px(TASKS_PANEL_RADIUS_PX))
                        .border_1()
                        .border_color(theme::ring_panel())
                        .bg(theme::longan_950())
                        .p_3()
                        .max_h(px(TASKS_PANEL_LOG_MAX_HEIGHT_PX))
                        .overflow_y_scrollbar()
                        .font_family(TASKS_PANEL_LOG_FONT_FAMILY)
                        .text_size(px(TASKS_PANEL_LOG_TEXT_SIZE_PX))
                        .text_color(theme::rambutan_400())
                        .child(log_tail),
                )
            })
    }
}

impl Render for TasksPanelView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        div()
            .debug_selector(|| "desktop-background-tasks-panel".to_string())
            .w_full()
            .h_full()
            .bg(theme::longan_950())
            .border_l_1()
            .border_color(theme::longan_800().opacity(TASKS_PANEL_DIVIDER_OPACITY))
            .text_color(theme::rambutan_300())
            .font_family(TASKS_PANEL_BODY_FONT_FAMILY)
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(TASKS_PANEL_HEADER_HEIGHT_PX))
                    .px_5()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(theme::longan_800().opacity(TASKS_PANEL_DIVIDER_OPACITY))
                    .flex_shrink_0()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .font_family(TASKS_PANEL_BODY_FONT_FAMILY)
                            .text_size(px(TASKS_PANEL_HEADER_TEXT_SIZE_PX))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(theme::rambutan_300())
                            .child(
                                svg()
                                    .path(assets::ACTIVITY_ICON_PATH)
                                    .size(px(TASKS_PANEL_ICON_SIZE_PX))
                                    .flex_shrink_0()
                                    .text_color(theme::rambutan_500()),
                            )
                            .child("Tasks"),
                    )
                    .child(
                        div()
                            .id("desktop-close-tasks-panel")
                            .debug_selector(|| "desktop-close-tasks-panel".to_string())
                            .size(px(TASKS_PANEL_CLOSE_BUTTON_SIZE_PX))
                            .rounded(px(TASKS_PANEL_RADIUS_PX))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(theme::rambutan_500())
                            .tab_index(0)
                            .role(Role::Button)
                            .aria_label(TASKS_PANEL_CLOSE_LABEL)
                            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                            .cursor_pointer()
                            .hover(|el| {
                                el.bg(theme::longan_800().opacity(TASKS_PANEL_HOVER_BG_OPACITY))
                                    .text_color(theme::rambutan_300())
                            })
                            .tooltip(|window, cx| {
                                Tooltip::new(TASKS_PANEL_CLOSE_LABEL).build(window, cx)
                            })
                            .on_click(cx.listener(|_this, _ev, _window, cx| {
                                cx.emit(TasksPanelEvent::CloseRequested);
                            }))
                            .on_key_down(cx.listener(|_this, event: &KeyDownEvent, window, cx| {
                                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                                    window.prevent_default();
                                    cx.stop_propagation();
                                    cx.emit(TasksPanelEvent::CloseRequested);
                                }
                            }))
                            .child(
                                svg()
                                    .path(assets::CLOSE_ICON_PATH)
                                    .size(px(TASKS_PANEL_ICON_SIZE_PX))
                                    .text_color(theme::rambutan_300()),
                            ),
                    ),
            )
            .child(
                div()
                    .id("tasks-body")
                    .flex_1()
                    .overflow_y_scrollbar()
                    .px_3()
                    .py_3()
                    .child(self.render_task_sections(cx)),
            )
    }
}

fn tasks_panel_snapshot_like_web(tasks: Vec<BackgroundTask>) -> Vec<BackgroundTask> {
    // The web tasks panel renders the `/api/tasks/stream` snapshot as-is and
    // only filters it into Running and Recent sections.
    tasks
}

/// `now`, `5m`, `3h`, `40d` since `millis`; empty for a timestamp no
/// clock could have produced.
fn relative_time_from(now: EpochMs, millis: i64) -> String {
    EpochMs::new(millis).map_or_else(
        |_| String::new(),
        |then| format_relative_ms(then, now, RelativeTimeStyle::CompactDays),
    )
}

fn tasks_panel_timestamp_like_web(task: &BackgroundTask, now: EpochMs) -> String {
    if task.status == TaskStatus::Running {
        relative_time_from(now, task.started_at)
    } else {
        task.ended_at
            .map(|ended_at| relative_time_from(now, ended_at))
            .unwrap_or_default()
    }
}

fn desktop_task_completion_mark(success: bool) -> &'static str {
    if success {
        "✓"
    } else {
        "✗"
    }
}

fn desktop_task_success_like_web(exit_code: Option<i32>) -> bool {
    exit_code == Some(0)
}

fn render_task_status_indicator(status: &TaskStatus, success: bool) -> AnyElement {
    if *status == TaskStatus::Running {
        return div()
            .flex()
            .items_center()
            .gap(px(TASKS_PANEL_RUNNING_DOT_GAP_PX))
            .flex_shrink_0()
            .child(
                div()
                    .size(px(TASKS_PANEL_RUNNING_DOT_SIZE_PX))
                    .rounded_full()
                    .bg(theme::pandan_500()),
            )
            .child(
                div()
                    .size(px(TASKS_PANEL_RUNNING_DOT_SIZE_PX))
                    .rounded_full()
                    .bg(theme::pandan_500()),
            )
            .child(
                div()
                    .size(px(TASKS_PANEL_RUNNING_DOT_SIZE_PX))
                    .rounded_full()
                    .bg(theme::pandan_500()),
            )
            .into_any_element();
    }

    let (label, color) = if success {
        (desktop_task_completion_mark(true), theme::pandan_400())
    } else {
        (desktop_task_completion_mark(false), theme::lychee_400())
    };

    div()
        .flex_shrink_0()
        .font_family(TASKS_PANEL_LABEL_FONT_FAMILY)
        .text_size(px(TASKS_PANEL_STATUS_TEXT_SIZE_PX))
        .font_family(theme::UI_SMALL_FONT_FAMILY)
        .text_color(color)
        .child(label)
        .into_any_element()
}

fn element_key(value: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::{
        desktop_task_completion_mark, desktop_task_success_like_web, relative_time_from,
        tasks_panel_snapshot_like_web, tasks_panel_timestamp_like_web, EpochMs,
        TASKS_PANEL_BODY_FONT_FAMILY, TASKS_PANEL_CLOSE_BUTTON_SIZE_PX, TASKS_PANEL_CLOSE_LABEL,
        TASKS_PANEL_DISMISS_TOOLTIP, TASKS_PANEL_DIVIDER_OPACITY, TASKS_PANEL_EMPTY_LABEL,
        TASKS_PANEL_EMPTY_TEXT_SIZE_PX, TASKS_PANEL_HEADER_HEIGHT_PX,
        TASKS_PANEL_HEADER_TEXT_SIZE_PX, TASKS_PANEL_HOVER_BG_OPACITY, TASKS_PANEL_ICON_SIZE_PX,
        TASKS_PANEL_LABEL_FONT_FAMILY, TASKS_PANEL_LOG_FONT_FAMILY, TASKS_PANEL_LOG_MAX_HEIGHT_PX,
        TASKS_PANEL_LOG_TEXT_SIZE_PX, TASKS_PANEL_RADIUS_PX, TASKS_PANEL_RECENT_EMPTY_LOG,
        TASKS_PANEL_RECENT_LABEL, TASKS_PANEL_ROW_LABEL_TEXT_SIZE_PX,
        TASKS_PANEL_RUNNING_DOT_GAP_PX, TASKS_PANEL_RUNNING_DOT_SIZE_PX,
        TASKS_PANEL_RUNNING_EMPTY_LOG, TASKS_PANEL_RUNNING_LABEL, TASKS_PANEL_SECTION_TEXT_SIZE_PX,
        TASKS_PANEL_STATUS_TEXT_SIZE_PX, TASKS_PANEL_TIME_TEXT_SIZE_PX,
    };
    use bruhs_core::{BackgroundTask, TaskKind, TaskStatus};

    fn task(id: &str, started_at: i64, ended_at: Option<i64>) -> BackgroundTask {
        BackgroundTask {
            id: id.to_string(),
            thread_id: "thread".to_string(),
            parent_task_id: None,
            kind: TaskKind::Shell,
            label: id.to_string(),
            status: if ended_at.is_some() {
                TaskStatus::Exited
            } else {
                TaskStatus::Running
            },
            started_at,
            ended_at,
            exit_code: ended_at.map(|_| 0),
            meta: serde_json::Value::Null,
            log_tail: String::new(),
        }
    }

    #[::core::prelude::v1::test]
    fn desktop_tasks_panel_completion_marks_should_match_web_panel() {
        assert_eq!(desktop_task_completion_mark(true), "✓");
        assert_eq!(desktop_task_completion_mark(false), "✗");
    }

    #[::core::prelude::v1::test]
    fn desktop_tasks_panel_success_condition_should_match_web_panel() {
        assert!(desktop_task_success_like_web(Some(0)));
        assert!(!desktop_task_success_like_web(Some(1)));
        assert!(!desktop_task_success_like_web(None));
    }

    #[::core::prelude::v1::test]
    fn desktop_tasks_panel_snapshot_order_should_match_web_stream_reference() {
        let tasks = tasks_panel_snapshot_like_web(vec![
            task("older-finished", 100, Some(200)),
            task("newer-running", 300, None),
            task("newer-finished", 100, Some(400)),
        ]);

        assert_eq!(
            tasks
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            vec!["older-finished", "newer-running", "newer-finished"]
        );
    }

    #[::core::prelude::v1::test]
    fn desktop_tasks_panel_timestamps_should_match_web_running_and_recent_rows() {
        let now = EpochMs::new(300_000).unwrap();
        let mut recent_without_end = task("failed-without-end", 60_000, None);
        recent_without_end.status = TaskStatus::Failed;

        assert_eq!(
            tasks_panel_timestamp_like_web(&task("running", 240_000, None), now),
            "1m"
        );
        assert_eq!(
            tasks_panel_timestamp_like_web(&task("finished", 60_000, Some(180_000)), now),
            "2m"
        );
        assert_eq!(tasks_panel_timestamp_like_web(&recent_without_end, now), "");
    }

    #[::core::prelude::v1::test]
    fn desktop_tasks_panel_copy_should_match_web_panel() {
        assert_eq!(TASKS_PANEL_EMPTY_LABEL, "No background tasks");
        assert_eq!(TASKS_PANEL_RUNNING_LABEL, "Running");
        assert_eq!(TASKS_PANEL_RECENT_LABEL, "Recent");
        assert_eq!(TASKS_PANEL_RUNNING_EMPTY_LOG, "No output yet...");
        assert_eq!(TASKS_PANEL_RECENT_EMPTY_LOG, "No output.");
        assert_eq!(TASKS_PANEL_CLOSE_LABEL, "Close tasks panel");
        assert_eq!(TASKS_PANEL_DISMISS_TOOLTIP, "Dismiss");
    }

    #[::core::prelude::v1::test]
    fn desktop_tasks_panel_typography_should_match_web_panel() {
        assert_eq!(TASKS_PANEL_BODY_FONT_FAMILY, "Monaspace Neon");
        assert_eq!(TASKS_PANEL_LABEL_FONT_FAMILY, "Monaspace Argon");
        assert_eq!(TASKS_PANEL_LOG_FONT_FAMILY, "Monaspace Neon");
        assert_eq!(TASKS_PANEL_HEADER_TEXT_SIZE_PX, 13.0);
        assert_eq!(TASKS_PANEL_EMPTY_TEXT_SIZE_PX, 12.0);
        assert_eq!(TASKS_PANEL_SECTION_TEXT_SIZE_PX, 12.0);
        assert_eq!(TASKS_PANEL_ROW_LABEL_TEXT_SIZE_PX, 14.0);
        assert_eq!(TASKS_PANEL_TIME_TEXT_SIZE_PX, 12.0);
        assert_eq!(TASKS_PANEL_STATUS_TEXT_SIZE_PX, 12.0);
        assert_eq!(TASKS_PANEL_LOG_TEXT_SIZE_PX, 12.0);
    }

    #[::core::prelude::v1::test]
    fn desktop_tasks_panel_metrics_should_match_web_panel() {
        assert_eq!(TASKS_PANEL_RADIUS_PX, 8.0);
        assert_eq!(TASKS_PANEL_HEADER_HEIGHT_PX, 42.0);
        assert_eq!(TASKS_PANEL_ICON_SIZE_PX, 14.0);
        assert_eq!(TASKS_PANEL_CLOSE_BUTTON_SIZE_PX, 32.0);
        assert_eq!(TASKS_PANEL_LOG_MAX_HEIGHT_PX, 128.0);
        assert_eq!(TASKS_PANEL_DIVIDER_OPACITY, 0.50);
        assert_eq!(TASKS_PANEL_HOVER_BG_OPACITY, 0.50);
        assert_eq!(TASKS_PANEL_RUNNING_DOT_GAP_PX, 2.0);
        assert_eq!(TASKS_PANEL_RUNNING_DOT_SIZE_PX, 4.0);
    }

    #[::core::prelude::v1::test]
    fn desktop_tasks_panel_relative_time_should_match_web_thresholds() {
        let now = EpochMs::new(100 * 24 * 60 * 60_000).unwrap();
        let ago = |ms: i64| now.as_millis() - ms;

        assert_eq!(relative_time_from(now, ago(59_000)), "now");
        assert_eq!(relative_time_from(now, ago(60_000)), "1m");
        assert_eq!(relative_time_from(now, ago(59 * 60_000)), "59m");
        assert_eq!(relative_time_from(now, ago(60 * 60_000)), "1h");
        assert_eq!(relative_time_from(now, ago(23 * 60 * 60_000)), "23h");
        assert_eq!(relative_time_from(now, ago(24 * 60 * 60_000)), "1d");
        // Days never roll up into weeks here, unlike the sidebar.
        assert_eq!(relative_time_from(now, ago(40 * 24 * 60 * 60_000)), "40d");
    }
}
