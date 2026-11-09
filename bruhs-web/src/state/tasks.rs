//! Work items and projects.

use super::*;

#[derive(Copy, Clone)]
pub struct TaskViewModeSignal(pub ReadSignal<TaskViewMode>);
#[derive(Copy, Clone)]
pub struct SetTaskViewModeSignal(pub WriteSignal<TaskViewMode>);

/// Format a `YYYY-MM-DD` date string relative to today.
/// Returns `(text, tailwind_color_class)`. Mirrors apps/web tasks/task-row.tsx
/// formatDueDate: "today" persimmon, "tomorrow" rambutan, past dates lychee,
/// future dates rambutan, all formatted as "Mmm D".
pub fn format_due_date(date: &str) -> Option<(String, &'static str)> {
    if date.is_empty() {
        return None;
    }
    let parts: Vec<&str> = date.split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    let year: i32 = parts[0].parse().ok()?;
    let month: u32 = parts[1].parse().ok()?;
    let day: u32 = parts[2].parse().ok()?;

    let now = js_sys::Date::new_0();
    let today_y = crate::clock::field(now.get_full_year());
    let today_m = now.get_month() + 1;
    let today_d = now.get_date();

    let due = js_sys::Date::new_with_year_month_day(
        crate::clock::year_arg(year),
        crate::clock::field(month) - 1,
        crate::clock::field(day),
    );
    let today_ms = js_sys::Date::new_with_year_month_day(
        crate::clock::year_arg(today_y),
        crate::clock::field(today_m) - 1,
        crate::clock::field(today_d),
    )
    .get_time();
    let due_ms = due.get_time();

    let one_day = 1000.0 * 60.0 * 60.0 * 24.0;
    let diff = due_ms - today_ms;

    if diff == 0.0 {
        return Some(("today".to_string(), "text-persimmon-400"));
    }
    if diff == one_day {
        return Some(("tomorrow".to_string(), "text-rambutan-500"));
    }
    let months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let label = format!(
        "{} {}",
        months.get((month - 1) as usize).copied().unwrap_or(""),
        day
    );
    let class = if diff < 0.0 {
        "text-lychee-400"
    } else {
        "text-rambutan-500"
    };
    Some((label, class))
}

#[derive(Copy, Clone)]
pub struct TaskCreateOpen(pub ReadSignal<bool>);
#[derive(Copy, Clone)]
pub struct SetTaskCreateOpen(pub WriteSignal<bool>);

#[derive(Copy, Clone)]
pub struct WorkItemsSignal(pub ReadSignal<Vec<WorkItem>>);
#[derive(Copy, Clone)]
pub struct SetWorkItemsSignal(pub WriteSignal<Vec<WorkItem>>);

#[derive(Copy, Clone)]
pub struct ProjectsSignal(pub ReadSignal<Vec<Project>>);
#[derive(Copy, Clone)]
pub struct SetProjectsSignal(pub WriteSignal<Vec<Project>>);

#[derive(Copy, Clone)]
pub struct SelectedWorkItem(pub ReadSignal<Option<String>>);
#[derive(Copy, Clone)]
pub struct SetSelectedWorkItem(pub WriteSignal<Option<String>>);
#[derive(Copy, Clone)]
pub struct SelectedProject(pub ReadSignal<Option<String>>);
#[derive(Copy, Clone)]
pub struct SetSelectedProject(pub WriteSignal<Option<String>>);

/// Work items, projects and the Tasks view's selection and mode.
pub(super) fn provide() {
    let (work_items, set_work_items) = signal(Vec::<WorkItem>::new());
    let (projects, set_projects) = signal(Vec::<Project>::new());
    let (selected_work_item, set_selected_work_item) = signal::<Option<String>>(None);
    let (selected_project, set_selected_project) = signal::<Option<String>>(None);
    // Default to List view to match TS apps/web's tasks-page default.
    let initial_task_view_mode = load_pref("bruhs.taskViewMode").map_or(TaskViewMode::List, |s| {
        if s == "board" {
            TaskViewMode::Board
        } else {
            TaskViewMode::List
        }
    });
    let (task_view_mode, set_task_view_mode) = signal(initial_task_view_mode);
    Effect::new(move |_| {
        save_pref(
            "bruhs.taskViewMode",
            match task_view_mode.get() {
                TaskViewMode::Board => "board",
                TaskViewMode::List => "list",
            },
        );
    });
    let (task_create_open, set_task_create_open) = signal(false);

    provide_context(WorkItemsSignal(work_items));
    provide_context(SetWorkItemsSignal(set_work_items));
    provide_context(ProjectsSignal(projects));
    provide_context(SetProjectsSignal(set_projects));
    provide_context(SelectedWorkItem(selected_work_item));
    provide_context(SetSelectedWorkItem(set_selected_work_item));
    provide_context(SelectedProject(selected_project));
    provide_context(SetSelectedProject(set_selected_project));
    provide_context(TaskViewModeSignal(task_view_mode));
    provide_context(SetTaskViewModeSignal(set_task_view_mode));
    provide_context(TaskCreateOpen(task_create_open));
    provide_context(SetTaskCreateOpen(set_task_create_open));
}
