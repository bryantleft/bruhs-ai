//! Choose the current thread’s optional project folder, or a terminal folder.

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api::{list_dir, DirEntry};
use crate::components::terminal_panel::open_terminal_session;
use crate::state::{
    ActiveProject, ActiveTerminalId, ActiveView, AppView, ProjectPickerMode,
    ProjectPickerModeSignal, ProjectPickerOpen, RecentProjectsSignal, SetActiveProjectSignal,
    SetProjectPickerOpen, SetRecentProjectsSignal, TerminalOpen, TerminalSessions,
};
use crate::ui::{Dialog, Icon};

const MAX_RECENT: usize = 5;

/// Add a project to the recent-projects list, de-duplicating by path and
/// capping at `MAX_RECENT`. Most recent is at the front.
fn push_recent(list: Vec<ActiveProject>, project: &ActiveProject) -> Vec<ActiveProject> {
    let mut next: Vec<ActiveProject> = std::iter::once(project.clone())
        .chain(list.into_iter().filter(|p| p.path != project.path))
        .collect();
    next.truncate(MAX_RECENT);
    next
}

#[component]
pub fn ProjectPickerDialog() -> impl IntoView {
    let catalog = expect_context::<crate::thread_catalog::ThreadCatalog>();
    let ProjectPickerOpen(open) = expect_context::<ProjectPickerOpen>();
    let SetProjectPickerOpen(set_open) = expect_context::<SetProjectPickerOpen>();
    let SetActiveProjectSignal(set_active_project) = expect_context::<SetActiveProjectSignal>();
    let RecentProjectsSignal(recent_projects) = expect_context::<RecentProjectsSignal>();
    let SetRecentProjectsSignal(set_recent_projects) = expect_context::<SetRecentProjectsSignal>();
    let ProjectPickerModeSignal(picker_mode) = expect_context::<ProjectPickerModeSignal>();
    let TerminalSessions(terminal_sessions) = expect_context::<TerminalSessions>();
    let ActiveTerminalId(active_terminal_id) = expect_context::<ActiveTerminalId>();
    let TerminalOpen(terminal_open) = expect_context::<TerminalOpen>();
    let ActiveView(active_view) = expect_context::<ActiveView>();
    let set_threads = expect_context::<crate::state::SetThreadsSignal>().0;
    let active = expect_context::<crate::state::ActiveThreadId>().0;
    let messages = expect_context::<crate::state::MessagesSignal>().0;
    let tasks = expect_context::<crate::state::BackgroundTasksSignal>().0;
    let turns = expect_context::<crate::state::Turns>();
    let history = expect_context::<crate::thread_history::ThreadHistory>();
    let target = StoredValue::new(None::<String>);

    let (current_path, set_current_path) = signal(String::new());
    let (current_name, set_current_name) = signal(String::new());
    let (parent_path, set_parent_path) = signal::<Option<String>>(None);
    let (dirs, set_dirs) = signal::<Vec<DirEntry>>(vec![]);
    let (error, set_error) = signal::<Option<String>>(None);
    let (loading, set_loading) = signal(false);

    // Fetch directory listing for `path` (or $HOME if None) and populate state.
    let load_dir = move |path: Option<String>| {
        set_loading.set(true);
        spawn_local(async move {
            match list_dir(path.as_deref()).await {
                Ok(resp) => {
                    set_current_path.set(resp.current);
                    set_current_name.set(resp.name);
                    set_parent_path.set(resp.parent);
                    set_dirs.set(resp.dirs);
                    set_error.set(None);
                }
                Err(error) => set_error.set(Some(format!("Could not list directory: {error}"))),
            }
            set_loading.set(false);
        });
    };

    // Load $HOME on first open.
    Effect::new(move |_| {
        if open.get() {
            target.set_value(active.get_untracked());
            set_error.set(None);
            if current_path.get_untracked().is_empty() {
                load_dir(None);
            }
        } else {
            picker_mode.set(ProjectPickerMode::Thread);
        }
    });

    let close = move || {
        set_open.set(false);
        picker_mode.set(ProjectPickerMode::Thread);
    };
    let on_close = Callback::new(move |_: ()| close());

    let apply_thread_project = move |project: Option<ActiveProject>| {
        let name = project.as_ref().map(|p| p.name.clone()).unwrap_or_default();
        let path = project.as_ref().map(|p| p.path.clone());
        let mut project_changed = target.get_value().is_none();
        if let Some(id) = target.get_value() {
            if active.get_untracked().as_deref() != Some(id.as_str()) {
                set_error.set(Some(
                    "The active thread changed. Close the picker and try again.".into(),
                ));
                return false;
            }
            if turns.is_busy_untracked(&id)
                || tasks.with_untracked(|tasks| {
                    tasks.iter().any(|task| {
                        task.thread_id.as_str() == id
                            && task.status == bruhs_models::api::TaskStatus::Running
                    })
                })
            {
                set_error.set(Some(
                    "Wait for this thread to finish before changing its project.".into(),
                ));
                return false;
            }
            if history.blocked(&id) {
                set_error.set(Some(
                    "Load the saved conversation before changing its project.".into(),
                ));
                return false;
            }
            let mut changed = false;
            set_threads.update(|rows| {
                if let Some(thread) = rows.iter_mut().find(|thread| thread.id == id) {
                    let original = thread.clone();
                    let context = messages.with_untracked(|messages| {
                        bruhs_models::logic::portable::thread_context(
                            &thread.title,
                            &crate::components::import_thread_dialog::portable_messages(messages),
                        )
                    });
                    thread.set_project(name, path, context);
                    if *thread != original {
                        project_changed = true;
                        catalog.persist(Some(original), thread.clone());
                    }
                    changed = true;
                }
            });
            if !changed {
                set_error.set(Some(
                    "This thread is no longer available. Close the picker and choose a thread."
                        .into(),
                ));
                return false;
            }
        } else {
            crate::state::create_thread(
                catalog,
                set_threads,
                expect_context::<crate::state::SetActiveThreadId>().0,
                expect_context::<crate::state::SetMessagesSignal>().0,
                name,
                path,
            );
        }
        if project_changed {
            let crate::state::RunChoiceSignal(choice) = expect_context();
            choice.set(bruhs_models::logic::run_place::WorkspaceChoice::Checkout);
            let crate::state::BaseBranchSignal(base) = expect_context();
            base.set(String::new());
            let crate::state::PreviousWorktreeSignal(previous) = expect_context();
            previous.set(None);
            let crate::state::ChipRetargetGuard(guard) = expect_context();
            let crate::state::ComposerChips(chips) = expect_context();
            guard.set_value(true);
            chips.set(Vec::new());
        }
        if active_view.get_untracked() != AppView::Chat {
            expect_context::<crate::state::SetActiveView>()
                .0
                .set(AppView::Chat);
        }
        true
    };
    let select_project = move |path: String, name: String| {
        let project = ActiveProject { path, name };
        if picker_mode.get_untracked() == ProjectPickerMode::NewTerminal {
            open_terminal_session(
                terminal_sessions,
                active_terminal_id,
                Some(project.path.clone()),
                Some(project.name.clone()),
            );
            if active_view.get_untracked() != AppView::Terminal {
                terminal_open.set(true);
            }
        } else if !apply_thread_project(Some(project.clone())) {
            return;
        }
        set_recent_projects.set(push_recent(recent_projects.get_untracked(), &project));
        set_active_project.set(Some(project));
        close();
    };
    let clear_project = move || {
        if apply_thread_project(None) {
            close();
        }
    };

    view! {
        <Dialog open=open on_close=on_close max_width="max-w-md".to_string() label="Choose project folder">
            // ── Header: current path + Select-this-folder button ────────────
            // Mirrors apps/web/src/components/shell/project-picker-dialog.tsx
            <div class="flex items-center justify-between border-b border-longan-800 px-4 py-3">
                <div class="min-w-0 flex-1">
                    <h2 class="text-label-sm text-rambutan-200">
                        "Choose project folder"
                    </h2>
                    {move || (!current_path.get().is_empty()).then(|| view! {
                        <p class="mt-0.5 truncate text-code-sm text-rambutan-300">
                            {current_path.get()}
                        </p>
                    })}
                </div>
                {move || (!current_path.get().is_empty()).then(|| {
                    let path = current_path.get();
                    let name = current_name.get();
                    view! {
                        <button
                            type="button"
                            on:click=move |_| select_project(path.clone(), name.clone())
                            class="ml-3 shrink-0 rounded-grape bg-rambutan-600/20 px-3 py-1.5 text-label-sm text-rambutan-300 hover:bg-rambutan-600/30 transition-colors cursor-pointer"
                        >
                            "Select this folder"
                        </button>
                    }
                })}
            </div>

            <Show when=move || picker_mode.get() != ProjectPickerMode::NewTerminal>
                <div class="border-b border-longan-800/50 px-4 py-2">
                    <button type="button" class="flex min-h-11 w-full items-center gap-2 rounded-grape px-2 text-label-sm text-rambutan-300 hover:bg-longan-800/50"
                        on:click=move |_| clear_project()>
                        <Icon name="message-square" size=14 />
                        "No project"
                    </button>
                </div>
            </Show>

            // ── Recent projects (only when no listing yet) ───────────────────
            {move || {
                let recents = recent_projects.get();
                if recents.is_empty() || !current_path.get().is_empty() {
                    return ().into_any();
                }
                let rendered: Vec<_> = recents
                    .into_iter()
                    .map(|p| {
                        let path = p.path.clone();
                        let name = p.name.clone();
                        view! {
                            <button
                                type="button"
                                on:click=move |_| select_project(path.clone(), name.clone())
                                class="flex w-full items-center gap-2 truncate rounded-grape px-2 py-1.5 text-left text-label-sm text-rambutan-400 hover:bg-longan-800/50 hover:text-rambutan-200 transition-colors cursor-pointer"
                            >
                                <span class="shrink-0 text-rambutan-500">
                                    <Icon name="folder" />
                                </span>
                                <span class="truncate">{p.path}</span>
                            </button>
                        }
                    })
                    .collect();
                view! {
                    <div class="border-b border-longan-800/50 px-4 py-3">
                        <p class="mb-2 text-label uppercase tracking-widest text-rambutan-300">
                            "Recent"
                        </p>
                        <div class="space-y-0.5">{rendered}</div>
                    </div>
                }.into_any()
            }}

            // ── Directory browser ────────────────────────────────────────────
            <div class="max-h-72 overflow-y-auto">
                {move || {
                    if loading.get() {
                        return view! {
                            <div class="flex items-center justify-center py-8 text-label-sm text-rambutan-300">
                                "Loading…"
                            </div>
                        }.into_any();
                    }
                    if let Some(err) = error.get() {
                        return view! {
                            <div class="px-4 py-6 text-center text-label-sm text-lychee-400">
                                {err}
                            </div>
                        }.into_any();
                    }
                    let list = dirs.get();
                    let parent = parent_path.get();
                    let parent_button = parent.map(|p| view! {
                        <button
                            type="button"
                            on:click=move |_| load_dir(Some(p.clone()))
                            class="flex w-full items-center gap-2 px-4 py-2 text-label-sm text-rambutan-500 hover:bg-longan-800/40 transition-colors cursor-pointer"
                        >
                            <span class="shrink-0">"↑"</span>
                            <span>".."</span>
                        </button>
                    });

                    let dir_rows: Vec<_> = if list.is_empty() {
                        vec![view! {
                            <div class="px-4 py-6 text-center text-label-sm text-rambutan-300">
                                "No subdirectories"
                            </div>
                        }.into_any()]
                    } else {
                        list.into_iter()
                            .map(|d: DirEntry| {
                                let nav_path = d.path.clone();
                                let select_path = d.path.clone();
                                let select_name = d.name.clone();
                                view! {
                                    <button
                                        type="button"
                                        on:click=move |_| load_dir(Some(nav_path.clone()))
                                        on:dblclick=move |_| select_project(select_path.clone(), select_name.clone())
                                        class="flex w-full items-center gap-2 px-4 py-2 text-label-sm text-rambutan-400 hover:bg-longan-800/40 hover:text-rambutan-200 transition-colors cursor-pointer"
                                    >
                                        <span class="shrink-0 text-rambutan-500">
                                            <Icon name="folder" />
                                        </span>
                                        <span class="truncate">{d.name}</span>
                                    </button>
                                }.into_any()
                            })
                            .collect()
                    };

                    view! {
                        <div class="py-1">
                            {parent_button}
                            {dir_rows}
                        </div>
                    }.into_any()
                }}
            </div>

            // ── Footer ──────────────────────────────────────────────────────
            <div class="flex items-center justify-between border-t border-longan-800/50 px-4 py-2.5 text-label text-rambutan-300">
                <span>"Click to browse, double click to select"</span>
                <button
                    type="button"
                    on:click=move |_| close()
                    class="text-rambutan-500 hover:text-rambutan-300 transition-colors cursor-pointer"
                >
                    "Cancel"
                </button>
            </div>
        </Dialog>
    }
}
