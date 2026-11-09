use leptos::prelude::*;
use uuid::Uuid;

use crate::state::{
    Project, ProjectsSignal, SelectedProject, SetProjectsSignal, SetSelectedProject,
    SetWorkItemsSignal, WorkItemsSignal,
};
use crate::ui::{Button, ButtonSize, ButtonVariant, Dialog, Icon};

#[component]
pub fn ProjectSidebar() -> impl IntoView {
    let saves = expect_context::<crate::task_saves::TaskSaves>();
    let ProjectsSignal(projects) = expect_context::<ProjectsSignal>();
    let SetProjectsSignal(set_projects) = expect_context::<SetProjectsSignal>();
    let SelectedProject(selected_project) = expect_context::<SelectedProject>();
    let SetSelectedProject(set_selected_project) = expect_context::<SetSelectedProject>();
    // For cascading task deletion when the project goes.
    let WorkItemsSignal(work_items) = expect_context::<WorkItemsSignal>();
    let SetWorkItemsSignal(set_work_items) = expect_context::<SetWorkItemsSignal>();

    let on_all = move |_| set_selected_project.set(None);
    let on_new = Callback::new(move |_: web_sys::MouseEvent| {
        let id = Uuid::new_v4().to_string();
        let project = Project {
            id: id.clone(),
            name: format!("Project {}", projects.with_untracked(Vec::len) + 1),
            color: None,
        };
        set_projects.update(|ps| ps.push(project.clone()));
        saves.edit_project(None, project);
        set_selected_project.set(Some(id));
    });

    // Holds the (project_id, prompt) awaiting confirmation in the custom
    // delete dialog. `None` = no dialog open.
    let pending_delete = RwSignal::new(Option::<(String, String)>::None);

    // Optimistic local deletion, with one durable/recoverable project+task group.
    let perform_delete = move |id: String| {
        let Some(project) =
            projects.with_untracked(|items| items.iter().find(|project| project.id == id).cloned())
        else {
            return;
        };
        let tasks = work_items.with_untracked(|items| {
            items
                .iter()
                .filter(|w| w.project_id.as_deref() == Some(id.as_str()))
                .cloned()
                .collect::<Vec<_>>()
        });
        let removed: std::collections::HashSet<String> =
            tasks.iter().map(|task| task.id.clone()).collect();
        set_work_items.update(|ws| ws.retain(|w| !removed.contains(&w.id)));
        set_projects.update(|ps| ps.retain(|p| p.id != id));
        if selected_project.get_untracked().as_deref() == Some(id.as_str()) {
            set_selected_project.set(None);
        }
        saves.delete_project(project, tasks);
    };

    // Delete request: delete immediately when the project has no tasks; pop
    // the custom confirmation dialog when it does (destructive cascade).
    let delete_project: Callback<String> = Callback::new(move |id: String| {
        let task_count = work_items.with_untracked(|items| {
            items
                .iter()
                .filter(|w| w.project_id.as_deref() == Some(id.as_str()))
                .count()
        });
        if task_count == 0 {
            perform_delete(id);
            return;
        }
        let project_name = projects.with_untracked(|items| {
            items
                .iter()
                .find(|p| p.id == id)
                .map_or_else(|| "this project".to_string(), |p| p.name.clone())
        });
        let prompt = format!(
            "Delete project \"{}\" and {} task{}?",
            project_name,
            task_count,
            if task_count == 1 { "" } else { "s" }
        );
        pending_delete.set(Some((id, prompt)));
    });

    let confirm_delete = Callback::new(move |_: web_sys::MouseEvent| {
        if let Some((id, _)) = pending_delete.get_untracked() {
            perform_delete(id);
        }
        pending_delete.set(None);
    });
    let cancel_delete = Callback::new(move |_: web_sys::MouseEvent| pending_delete.set(None));

    view! {
        // Mirrors apps/web/src/components/tasks/project-sidebar.tsx — minimal
        // chrome: "All Tasks" entry at top doubles as the section header,
        // project list in the middle, "+ New project" at bottom.
        <div class="flex flex-col h-full w-full min-w-0 md:w-[280px] md:min-w-[208px] shrink-0 bg-longan-950 border-r border-longan-800/50 text-rambutan-300">
            <div class="flex-1 overflow-y-auto px-2 py-2 space-y-0.5">
                // "All Tasks" entry — also the de-facto sidebar heading.
                <button
                    on:click=on_all
                    class=move || format!(
                        "flex h-7 w-full items-center gap-1.5 rounded-grape px-2 text-left text-label-sm transition-colors {}",
                        if selected_project.get().is_none() {
                            "bg-longan-800/40 text-rambutan-200"
                        } else {
                            "text-rambutan-400 hover:bg-longan-800/30 hover:text-rambutan-300"
                        }
                    )
                >
                    "All Tasks"
                </button>

                <For
                    each=move || projects.get()
                    key=|p| p.id.clone()
                    children=move |project| {
                        let id = project.id.clone();
                        let id_for_click = id.clone();
                        let id_for_delete = id.clone();
                        let id_for_match = id;
                        let is_active = move || {
                            selected_project.get().as_deref() == Some(id_for_match.as_str())
                        };
                        let on_click = move |_| {
                            set_selected_project.set(Some(id_for_click.clone()));
                        };
                        let on_delete = move |ev: web_sys::MouseEvent| {
                            ev.stop_propagation();
                            delete_project.run(id_for_delete.clone());
                        };
                        let is_active_attr = is_active.clone();
                        view! {
                            // Row = plain flex wrapper; the select action and the
                            // delete action are sibling <button>s so both are real,
                            // keyboard-reachable controls (no interactive nesting).
                            <div
                                class=move || format!(
                                    "group/proj flex h-7 w-full items-center rounded-grape text-label-sm transition-colors {}",
                                    if is_active() {
                                        "bg-longan-800/40 text-rambutan-200"
                                    } else {
                                        "text-rambutan-400 hover:bg-longan-800/30 hover:text-rambutan-300"
                                    }
                                )
                            >
                                <button
                                    type="button"
                                    on:click=on_click
                                    aria-current=move || if is_active_attr() { Some("true") } else { None }
                                    class="flex h-full min-w-0 flex-1 items-center gap-1.5 px-2 text-left cursor-pointer"
                                >
                                    <span class="min-w-0 flex-1 truncate">{project.name}</span>
                                </button>
                                // Revealed on hover AND on keyboard focus within the
                                // row (opacity, not display, so it stays focusable).
                                <button
                                    type="button"
                                    on:click=on_delete
                                    title="Delete project"
                                    aria-label="Delete project"
                                    class="shrink-0 rounded-seed p-0.5 text-rambutan-500 opacity-0 transition-opacity hover:bg-longan-700/50 hover:text-lychee-400 group-hover/proj:opacity-100 group-focus-within/proj:opacity-100"
                                >
                                    <Icon name="trash" />
                                </button>
                            </div>
                        }
                    }
                />
            </div>

            // Footer — same dashed-border 11px pattern as the chat sidebar's
            // "+ New chat" button so the two footers feel like the same UI
            // primitive in different surfaces.
            <div class="shrink-0 px-3 pb-3">
                <button
                    type="button"
                    on:click=move |ev| on_new.run(ev)
                    title="New project"
                    class="flex w-full items-center justify-center gap-1.5 rounded-grape border border-dashed border-longan-700/50 px-3 py-2 text-label text-rambutan-500 transition-colors hover:border-rambutan-600/50 hover:text-rambutan-300 cursor-pointer"
                >
                    <span class="text-body-sm">"+"</span>
                    <span>"New project"</span>
                </button>
            </div>
        </div>

        // Custom confirmation dialog for destructive project deletion
        // (replaces the old native window.confirm()).
        <Dialog
            open=Signal::derive(move || pending_delete.get().is_some())
            on_close=Callback::new(move |_| pending_delete.set(None))
            max_width="max-w-sm"
            label="Confirm project deletion"
        >
            <div class="p-4 space-y-4">
                <div class="text-body text-rambutan-100">
                    {move || pending_delete.get().map(|(_, prompt)| prompt).unwrap_or_default()}
                </div>
                // rambutan-300: secondary text on the longan-800 modal surface
                // needs ≥300 to clear WCAG AA 4.5:1 (500 is only 4.0:1 here).
                <div class="text-label-sm text-rambutan-300">
                    "This can't be undone."
                </div>
                <div class="flex justify-end gap-2">
                    <Button
                        variant=ButtonVariant::Ghost
                        size=ButtonSize::Small
                        on_click=cancel_delete
                    >
                        "Cancel"
                    </Button>
                    <Button
                        variant=ButtonVariant::Destructive
                        size=ButtonSize::Small
                        on_click=confirm_delete
                    >
                        "Delete"
                    </Button>
                </div>
            </div>
        </Dialog>
    }
}
