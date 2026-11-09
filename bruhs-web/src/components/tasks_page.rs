use leptos::prelude::*;

use crate::components::project_sidebar::ProjectSidebar;
use crate::components::task_board::TaskBoard;
use crate::components::task_create_dialog::TaskCreateDialog;
use crate::components::task_detail::TaskDetail;
use crate::components::task_list::TaskList;
use crate::state::{
    ProjectsSignal, SelectedProject, SetTaskCreateOpen, SetTaskViewModeSignal, SidebarOpen,
    TaskViewMode, TaskViewModeSignal, WorkItemsSignal,
};
use crate::ui::Dialog;

#[component]
pub fn TasksPage() -> impl IntoView {
    let executor = expect_context::<crate::workflow_executor::WorkflowExecutor>();
    let WorkItemsSignal(work_items) = expect_context::<WorkItemsSignal>();
    let ProjectsSignal(projects) = expect_context::<ProjectsSignal>();
    let SelectedProject(selected_project) = expect_context::<SelectedProject>();
    let TaskViewModeSignal(view_mode) = expect_context::<TaskViewModeSignal>();
    let SetTaskViewModeSignal(set_view_mode) = expect_context::<SetTaskViewModeSignal>();
    let SetTaskCreateOpen(set_task_create_open) = expect_context::<SetTaskCreateOpen>();
    // Same `sidebar_open` signal that gates the chat sidebar — ⌘B should
    // toggle whichever sidebar is contextually visible (chat sidebar on Chat
    // view, project sidebar here on Tasks view).
    let SidebarOpen(sidebar_open) = expect_context::<SidebarOpen>();
    let project_picker_open = RwSignal::new(false);
    Effect::new(move |_| {
        let _ = selected_project.get();
        project_picker_open.set(false);
    });

    // Filter items by the active project (or show all when none selected).
    let filtered_items = Memo::new(move |_| {
        let pid = selected_project.get();
        work_items.with(|items| {
            items
                .iter()
                .filter(|w| match &pid {
                    Some(p) => w.project_id.as_deref() == Some(p.as_str()),
                    None => true,
                })
                .cloned()
                .collect::<Vec<_>>()
        })
    });

    let header_label = Signal::derive(move || match selected_project.get() {
        None => "All Tasks".to_string(),
        Some(pid) => projects.with(|ps| {
            ps.iter()
                .find(|p| p.id == pid)
                .map_or_else(|| "Project".to_string(), |p| p.name.clone())
        }),
    });

    let on_new_task = Callback::new(move |_: web_sys::MouseEvent| {
        set_task_create_open.set(true);
    });
    let on_list = Callback::new(move |_: web_sys::MouseEvent| {
        set_view_mode.set(TaskViewMode::List);
    });
    let on_board = Callback::new(move |_: web_sys::MouseEvent| {
        set_view_mode.set(TaskViewMode::Board);
    });

    view! {
        <div class="flex flex-1 h-full overflow-hidden">
            <Show when=move || sidebar_open.get() fallback=|| view! { <></> }>
                <div class="hidden h-full shrink-0 md:flex"><ProjectSidebar /></div>
            </Show>

            // Main column
            <div class="flex min-w-0 flex-1 flex-col overflow-hidden">
                // View mode toggle bar — TS apps/web tasks-page header:
                // border-b only, no bg/ring; px-4 py-2; project name on
                // left as text-xs font-medium tracking-wide rambutan-400.
                <div class="flex flex-wrap items-center justify-between gap-2 border-b border-longan-800/40 px-4 py-2 shrink-0">
                    <h1 class="text-label tracking-wide text-rambutan-400 truncate">
                        {move || header_label.get()}
                    </h1>
                    <button type="button" class="rounded-grape px-2 py-2 text-button text-rambutan-300 inset-ring inset-ring-longan-700 md:hidden" on:click=move |_| project_picker_open.set(true)>"Projects"</button>
                    <div class="flex flex-wrap items-center gap-1">
                        {move || {
                            let mode = view_mode.get();
                            let cls = move |active: bool| if active {
                                "rounded-grape px-2.5 py-1 text-button bg-longan-800/70 text-rambutan-200 cursor-pointer"
                            } else {
                                "rounded-grape px-2.5 py-1 text-button text-rambutan-500 hover:text-rambutan-300 cursor-pointer"
                            };
                            view! {
                                <>
                                    <button
                                        type="button"
                                        class=cls(mode == TaskViewMode::List)
                                        on:click=move |ev| on_list.run(ev)
                                    >
                                        "List"
                                    </button>
                                    <button
                                        type="button"
                                        class=cls(mode == TaskViewMode::Board)
                                        on:click=move |ev| on_board.run(ev)
                                    >
                                        "Board"
                                    </button>
                                </>
                            }
                        }}
                        // "+ New" — opens TaskCreateDialog. Same dialog the
                        // command bar's "New task" entry triggers.
                        <button
                            type="button"
                            on:click=move |ev| on_new_task.run(ev)
                            title="New task"
                            class="ml-1 rounded-grape bg-persimmon-500/15 text-persimmon-200 inset-ring inset-ring-persimmon-400/20 px-2.5 py-1 text-button hover:bg-persimmon-500/25 transition-colors cursor-pointer"
                        >
                            "+ New"
                        </button>
                    </div>
                </div>

                // Body
                <Show when=move || executor.load_error.get().is_some()>
                    <section aria-label="Task loading status" class="shrink-0 border-b border-longan-800/40 px-4 py-2 text-label-sm text-rambutan-400">
                        <p role="alert">"Saved task or project records could not be loaded. Existing storage was left unchanged; changes made here are kept."</p>
                        <button type="button" class="mt-2 rounded-grape bg-longan-800 px-3 py-2 text-rambutan-200 disabled:opacity-40 cursor-pointer"
                            disabled=move || executor.loading.get()
                            on:click=move |_| executor.load()>"Retry loading records"</button>
                    </section>
                </Show>
                <div class="flex flex-1 overflow-hidden">
                    {move || match view_mode.get() {
                        TaskViewMode::Board => view! {
                            <TaskBoard items=filtered_items />
                        }.into_any(),
                        TaskViewMode::List => view! {
                            <TaskList items=filtered_items />
                        }.into_any(),
                    }}
                </div>
            </div>

            // Detail panel slides in when a task is selected
            <TaskDetail />

            // Create dialog overlay
            <TaskCreateDialog />
            <Dialog open=project_picker_open on_close=Callback::new(move |_| project_picker_open.set(false)) label="Task projects" max_width="max-w-sm" padding_top="2rem">
                <div class="flex items-center justify-between px-3 py-2 text-rambutan-200">
                    <h2 class="text-h4">"Projects"</h2>
                    <button type="button" aria-label="Close projects" class="rounded-grape px-3 py-2 text-button" on:click=move |_| project_picker_open.set(false)>"Close"</button>
                </div>
                <div class="h-[60vh]"><ProjectSidebar /></div>
            </Dialog>
        </div>
    }
}
