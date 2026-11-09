use crate::task_style::{PriorityStyle, WorkItemStatusStyle};
use leptos::prelude::*;
use uuid::Uuid;

use crate::state::{
    Priority, ProjectsSignal, SelectedProject, SetTaskCreateOpen, SetWorkItemsSignal,
    TaskCreateOpen, WorkItem, WorkItemStatus,
};
use crate::storage;
use crate::ui::{
    Button, ButtonSize, ButtonVariant, DatePicker, Dialog, Dropdown, DropdownOption, Input,
    Textarea,
};

#[component]
pub fn TaskCreateDialog() -> impl IntoView {
    let TaskCreateOpen(open) = expect_context::<TaskCreateOpen>();
    let SetTaskCreateOpen(set_open) = expect_context::<SetTaskCreateOpen>();
    let SetWorkItemsSignal(set_work_items) = expect_context::<SetWorkItemsSignal>();
    let ProjectsSignal(projects) = expect_context::<ProjectsSignal>();
    let SelectedProject(selected_project) = expect_context::<SelectedProject>();

    let (title, set_title) = signal(String::new());
    let (description, set_description) = signal(String::new());
    let (project_id, set_project_id) = signal(String::new());
    let (priority, set_priority) = signal(Priority::None.as_str().to_string());
    let (status, set_status) = signal(WorkItemStatus::Backlog.as_str().to_string());
    let (due_date, set_due_date) = signal(String::new());

    // Reset form when dialog opens / preselect current project filter.
    Effect::new(move |_| {
        if open.get() {
            set_title.set(String::new());
            set_description.set(String::new());
            set_project_id.set(selected_project.get().unwrap_or_default());
            set_priority.set(Priority::None.as_str().to_string());
            set_status.set(WorkItemStatus::Backlog.as_str().to_string());
            set_due_date.set(String::new());
        }
    });

    let close = Callback::new(move |_| set_open.set(false));

    let submit = move || {
        let t = title.get().trim().to_string();
        if t.is_empty() {
            return;
        }
        let now = crate::clock::now_ms();
        let pid = project_id.get();
        let pid_opt = if pid.is_empty() { None } else { Some(pid) };
        let desc = description.get();
        let desc_opt = if desc.trim().is_empty() {
            None
        } else {
            Some(desc)
        };
        let due = due_date.get();
        let due_opt = if due.trim().is_empty() {
            None
        } else {
            Some(due)
        };
        let prio = priority.get().parse().unwrap_or(Priority::None);
        let st = status.get().parse().unwrap_or(WorkItemStatus::Backlog);

        let item = WorkItem {
            id: Uuid::new_v4().to_string(),
            project_id: pid_opt,
            title: t,
            description: desc_opt,
            status: st,
            priority: prio,
            due_date: due_opt,
            planning: Default::default(),
            created_at: now,
            updated_at: now,
        };
        set_work_items.update(|items| items.push(item.clone()));
        storage::persist_work_item(None, item);
        set_open.set(false);
    };

    let on_submit_btn = Callback::new(move |_: web_sys::MouseEvent| submit());
    let on_cancel = Callback::new(move |_: web_sys::MouseEvent| set_open.set(false));

    view! {
        // Slimmed to match modal density across the app: max-w-md (448px),
        // 13px title, tighter padding. Field labels + inputs already use
        // the bruhs Argon role utilities so no further changes there.
        <Dialog open=Signal::derive(move || open.get()) on_close=close max_width="max-w-md".to_string() label="New task">
            <div class="border-b border-longan-800 px-4 py-3">
                <h2 class="text-h4 text-rambutan-100">"New task"</h2>
            </div>

            <div class="px-4 py-3 space-y-3">
                <div class="space-y-1.5">
                    <label class="text-label-sm text-rambutan-400">"Title"</label>
                    <Input
                        value=Signal::derive(move || title.get())
                        on_input=Callback::new(move |v| set_title.set(v))
                        placeholder="What needs to be done?".to_string()
                        autofocus=true
                    />
                </div>

                <div class="space-y-1.5">
                    <label class="text-label-sm text-rambutan-400">"Description"</label>
                    <Textarea
                        value=Signal::derive(move || description.get())
                        on_input=Callback::new(move |v| set_description.set(v))
                        placeholder="Add details (optional)".to_string()
                        rows=3
                    />
                </div>

                <div class="grid grid-cols-2 gap-3">
                    <div class="space-y-1.5">
                        <label class="text-label-sm text-rambutan-400">"Project"</label>
                        <Dropdown
                            options=Signal::derive(move || {
                                let mut opts = vec![DropdownOption::new("", "No project")];
                                opts.extend(
                                    projects.get().into_iter().map(|p| DropdownOption::new(p.id, p.name)),
                                );
                                opts
                            })
                            value=Signal::derive(move || project_id.get())
                            on_change=Callback::new(move |v: String| set_project_id.set(v))
                            label="Project"
                            class="w-full"
                        />
                    </div>

                    <div class="space-y-1.5">
                        <label class="text-label-sm text-rambutan-400">"Status"</label>
                        <Dropdown
                            options={WorkItemStatus::all()
                                .iter()
                                .map(|s| DropdownOption::new(s.as_str(), format!("{} {}", s.glyph(), s.label())))
                                .collect::<Vec<_>>()}
                            value=Signal::derive(move || status.get())
                            on_change=Callback::new(move |v: String| set_status.set(v))
                            label="Status"
                            class="w-full"
                        />
                    </div>

                    <div class="space-y-1.5">
                        <label class="text-label-sm text-rambutan-400">"Priority"</label>
                        <Dropdown
                            options={Priority::all()
                                .iter()
                                .map(|p| DropdownOption::new(p.as_str(), format!("{} {}", p.glyph(), p.label())))
                                .collect::<Vec<_>>()}
                            value=Signal::derive(move || priority.get())
                            on_change=Callback::new(move |v: String| set_priority.set(v))
                            label="Priority"
                            class="w-full"
                        />
                    </div>

                    <div class="space-y-1.5">
                        <label class="text-label-sm text-rambutan-400">"Due date"</label>
                        <DatePicker
                            value=Signal::derive(move || due_date.get())
                            on_change=Callback::new(move |v| set_due_date.set(v))
                        />
                    </div>
                </div>
            </div>

            <div class="flex items-center justify-end gap-2 border-t border-longan-800 bg-longan-900/40 px-4 py-3">
                <Button
                    variant=ButtonVariant::Ghost
                    size=ButtonSize::Default
                    on_click=on_cancel
                >
                    "Cancel"
                </Button>
                <Button
                    variant=ButtonVariant::Primary
                    size=ButtonSize::Default
                    on_click=on_submit_btn
                    disabled=Signal::derive(move || title.get().trim().is_empty())
                >
                    "Create"
                </Button>
            </div>
        </Dialog>
    }
}
