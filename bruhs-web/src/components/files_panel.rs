//! Files surface: one directory of the active thread's project.

use leptos::prelude::*;

use crate::api::list_children;
use crate::components::file_attachments::show_open_file;
use crate::state::{ActiveThreadId, OpenFileSignal, RightPanelSignal, ThreadsSignal};
use crate::ui::Icon;
use bruhs_models::logic::branch_pull::parent_dir;

fn project_path() -> Option<String> {
    let id = expect_context::<ActiveThreadId>().0.get_untracked()?;
    expect_context::<ThreadsSignal>()
        .0
        .with_untracked(|threads| {
            threads
                .iter()
                .find(|thread| thread.id == id)
                .and_then(|thread| {
                    thread
                        .worktree_path
                        .clone()
                        .filter(|path| !path.is_empty())
                        .or_else(|| thread.project_path.clone().filter(|path| !path.is_empty()))
                })
        })
}

#[component]
pub fn FilesPanel() -> impl IntoView {
    let open_file = expect_context::<OpenFileSignal>().0;
    let panel = expect_context::<RightPanelSignal>().0;
    let threads = expect_context::<ThreadsSignal>().0;
    let active = expect_context::<ActiveThreadId>().0;
    let dir = RwSignal::new(String::new());
    let entries = RwSignal::new(Vec::<crate::api::FileEntry>::new());
    let error = RwSignal::new(None::<String>);
    let loading = RwSignal::new(false);
    let ticket = StoredValue::new(0u32);
    let seen_project = StoredValue::new(None::<Option<String>>);

    Effect::new(move |_| {
        let _ = threads.get();
        let _ = active.get();
        let path = project_path();
        let project_changed = seen_project.with_value(|seen| seen.as_ref() != Some(&path));
        if project_changed {
            seen_project.set_value(Some(path.clone()));
            entries.set(Vec::new());
            if !dir.get_untracked().is_empty() {
                dir.set(String::new());
                return;
            }
        }
        let dir_now = dir.get();
        let mine = ticket.get_value().wrapping_add(1);
        ticket.set_value(mine);
        let Some(path) = path else {
            entries.set(Vec::new());
            error.set(Some("Open a project to browse files.".into()));
            loading.set(false);
            return;
        };
        error.set(None);
        entries.set(Vec::new());
        loading.set(true);
        leptos::task::spawn_local(async move {
            let result = list_children(&path, &dir_now).await;
            if ticket.get_value() != mine {
                return;
            }
            loading.set(false);
            match result {
                Ok(files) => entries.set(files),
                Err(err) => {
                    entries.set(Vec::new());
                    error.set(Some(err.to_string()));
                }
            }
        });
    });

    let row = "flex h-8 w-full items-center gap-2 rounded-grape px-2 text-left text-label-sm text-rambutan-300 hover:bg-longan-800/60 hover:text-rambutan-100";

    view! {
        <div data-files-panel class="flex h-full min-h-0 flex-col" aria-label="Files">
            <div class="flex h-8 shrink-0 items-center gap-2 border-b border-longan-800/60 px-3">
                {move || {
                    parent_dir(&dir.get()).map(|parent| {
                        let parent = parent.to_string();
                        view! {
                            <button
                                type="button"
                                data-files-up
                                class="inline-flex h-6 items-center rounded-grape px-1.5 text-label-sm text-rambutan-400 hover:bg-longan-800/50 hover:text-rambutan-100"
                                on:click=move |_| dir.set(parent.clone())
                            >
                                "Up"
                            </button>
                        }
                    })
                }}
                <span class="min-w-0 flex-1 truncate text-label-sm text-rambutan-500">
                    {move || {
                        let current = dir.get();
                        if current.is_empty() { "Project".to_string() } else { current }
                    }}
                </span>
            </div>
            <div class="min-h-0 flex-1 overflow-y-auto p-2">
                {move || error.get().map(|message| view! {
                    <p class="px-2 py-3 text-label-sm text-rambutan-400">{message}</p>
                })}
                {move || loading.get().then(|| view! {
                    <p class="px-2 py-3 text-label-sm text-rambutan-500">"Loading files…"</p>
                })}
                {move || {
                    let items = entries.get();
                    if items.is_empty() && error.get().is_none() && !loading.get() {
                        return view! { <p class="px-2 py-3 text-label-sm text-rambutan-500">"This folder is empty."</p> }.into_any();
                    }
                    items.into_iter().map(|entry| {
                        let is_dir = entry.is_dir;
                        let icon = if is_dir { "folder" } else { "file" };
                        let click_path = entry.path.trim_end_matches('/').to_string();
                        let label = entry.name.trim_end_matches('/').to_string();
                        let name_attr = label.clone();
                        let click_label = label.clone();
                        view! {
                            <button
                                type="button"
                                class=row
                                data-file-name=name_attr
                                on:click=move |_| {
                                    if is_dir {
                                        dir.set(click_path.clone());
                                    } else if let Some(project) = project_path() {
                                        show_open_file(
                                            open_file,
                                            panel,
                                            click_path.clone(),
                                            click_label.clone(),
                                            Some(project),
                                            false,
                                        );
                                    }
                                }
                            >
                                <Icon name=icon size=14 class="shrink-0 text-rambutan-400" />
                                <span class="truncate">{label}</span>
                            </button>
                        }
                    }).collect_view().into_any()
                }}
            </div>
        </div>
    }
}
