//! Thread header: `project / title` breadcrumb with inline rename, plus the
//! right-panel toggles.

use leptos::html::Input;
use leptos::prelude::*;

use crate::state::{create_thread, PanelSurface, RightPanelSignal};
use crate::ui::Icon;

/// Trim a rename; `None` means "keep the old title" (empty or unchanged).
pub fn resolve_rename(draft: &str, original: &str) -> Option<String> {
    let trimmed = draft.trim();
    if trimmed.is_empty() || trimmed == original {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[component]
pub fn ChatHeader(thread_id: String) -> impl IntoView {
    let catalog = expect_context::<crate::thread_catalog::ThreadCatalog>();
    let threads = expect_context::<crate::state::ThreadsSignal>().0;
    let set_threads = expect_context::<crate::state::SetThreadsSignal>().0;
    let set_active_thread_id = expect_context::<crate::state::SetActiveThreadId>().0;
    let set_messages = expect_context::<crate::state::SetMessagesSignal>().0;
    let tasks = expect_context::<crate::state::BackgroundTasksSignal>().0;
    let RightPanelSignal(panel) = expect_context::<RightPanelSignal>();

    let id = StoredValue::new(thread_id);
    let thread = Signal::derive(move || {
        id.with_value(|id| threads.with(|ts| ts.iter().find(|t| &t.id == id).cloned()))
    });
    let title = Signal::derive(move || thread.get().map(|t| t.title).unwrap_or_default());
    let project = Signal::derive(move || {
        thread
            .get()
            .map(|t| t.project_name)
            .filter(|p| !p.is_empty())
    });
    // The thread's role, by its current name. A deleted role shows nothing.
    let roles = expect_context::<crate::state::RolesSignal>().0;
    let role = Signal::derive(move || {
        let id = thread.get()?.role?;
        roles.with(|book| book.find(&id).map(|role| role.name.clone()))
    });
    let running = Signal::derive(move || {
        tasks.with(|ts| {
            ts.iter()
                .filter(|t| t.status == bruhs_models::api::TaskStatus::Running)
                .count()
        })
    });

    // Inline rename
    let renaming = RwSignal::new(false);
    let draft = RwSignal::new(String::new());
    let input_ref = NodeRef::<Input>::new();
    let start_rename = move || {
        draft.set(title.get_untracked());
        renaming.set(true);
    };
    Effect::new(move |_| {
        if renaming.get() {
            if let Some(el) = input_ref.get() {
                let _ = el.focus();
                el.select();
            }
        }
    });
    let commit = move || {
        if !renaming.get_untracked() {
            return;
        }
        renaming.set(false);
        let original = title.get_untracked();
        if let Some(new_title) = resolve_rename(&draft.get_untracked(), &original) {
            let tid = id.get_value();
            set_threads.update(move |ts| {
                if let Some(t) = ts.iter_mut().find(|t| t.id == tid) {
                    let original = t.clone();
                    t.title = new_title;
                    catalog.persist(Some(original), t.clone());
                }
            });
        }
    };
    let cancel = move || renaming.set(false);

    let new_in_project = move || {
        if let Some(t) = thread.get_untracked() {
            create_thread(
                catalog,
                set_threads,
                set_active_thread_id,
                set_messages,
                t.project_name,
                t.project_path,
            );
        }
    };

    view! {
        <header class="group/header flex h-[52px] shrink-0 items-center gap-3 px-5">
            <nav aria-label="Thread breadcrumb" class="min-w-0 flex-1">
                <ol class="m-0 flex min-w-0 list-none items-center gap-2 p-0 text-label">
                    {move || project.get().map(|name| {
                        let label = format!("New thread in {name}");
                        view! {
                            <li class="flex shrink-0 items-center">
                                <button
                                    type="button"
                                    title=label.clone()
                                    aria-label=label
                                    class="inline-flex max-w-40 items-center gap-1.5 rounded-seed text-rambutan-400 transition-colors hover:text-rambutan-100"
                                    on:click=move |_| new_in_project()
                                >
                                    <Icon name="folder" size=14 class="shrink-0" />
                                    <span class="truncate">{name}</span>
                                </button>
                            </li>
                            <li aria-hidden="true" class="flex shrink-0 items-center text-rambutan-600">"/"</li>
                        }
                    })}
                    <li aria-current="page" class="flex min-w-0 flex-1 items-center gap-1 text-rambutan-100">
                        <Show
                            when=move || renaming.get()
                            fallback=move || view! {
                                <h1
                                    class="min-w-0 cursor-text truncate text-label"
                                    title="Double-click to rename"
                                    on:dblclick=move |_| start_rename()
                                >
                                    {move || bruhs_models::logic::text::plain_title(&title.get()).into_owned()}
                                </h1>
                                {move || role.get().map(|name| {
                                    let hint = format!("Role: {name}. /role changes it.");
                                    view! {
                                        <span
                                            data-thread-role=""
                                            title=hint
                                            class="ml-1 max-w-[8rem] shrink-0 truncate rounded-seed bg-longan-800/60 px-1.5 text-label-sm text-rambutan-300"
                                        >
                                            {name}
                                        </span>
                                    }
                                })}
                                <button
                                    type="button"
                                    aria-label="Rename thread"
                                    title="Rename thread"
                                    class="ml-1 inline-flex size-6 shrink-0 items-center justify-center rounded-seed text-rambutan-500 opacity-0 transition-colors hover:bg-longan-800/50 hover:text-rambutan-100 group-hover/header:opacity-100 focus-visible:opacity-100"
                                    on:click=move |_| start_rename()
                                >
                                    <Icon name="pen-line" size=12 />
                                </button>
                            }
                        >
                            <input
                                node_ref=input_ref
                                type="text"
                                aria-label="Thread title"
                                class="min-w-0 flex-1 rounded-seed bg-transparent px-1 text-label text-rambutan-100 inset-ring inset-ring-persimmon-400/40 focus:outline-none"
                                prop:value=move || draft.get()
                                on:input=move |ev| draft.set(event_target_value(&ev))
                                on:keydown=move |ev| {
                                    match ev.key().as_str() {
                                        "Enter" => { ev.prevent_default(); commit(); }
                                        "Escape" => { ev.prevent_default(); cancel(); }
                                        _ => {}
                                    }
                                }
                                on:blur=move |_| commit()
                            />
                        </Show>
                    </li>
                </ol>
            </nav>
            <div class="flex shrink-0 items-center gap-1">
                <PanelToggle surface=PanelSurface::Diff icon="file-diff" label="Diff" shortcut="⌘D" />
                <PanelToggle surface=PanelSurface::Tasks icon="activity" label="Tasks" shortcut="⌘⇧T" badge=running />
                <button
                    type="button"
                    aria-label="Toggle right panel (⌘⌥B)"
                    title="Toggle right panel (⌘⌥B)"
                    aria-pressed=move || panel.get().open.to_string()
                    class=move || {
                        let base = "inline-flex size-7 items-center justify-center rounded-grape transition-colors";
                        if panel.get().open {
                            format!("{base} bg-longan-800/60 text-rambutan-100")
                        } else {
                            format!("{base} text-rambutan-400 hover:bg-longan-800/40 hover:text-rambutan-100")
                        }
                    }
                    on:click=move |_| panel.update(|p| p.toggle())
                >
                    <Icon name="panel-right" size=14 />
                </button>
            </div>
        </header>
    }
}

#[component]
fn PanelToggle(
    surface: PanelSurface,
    icon: &'static str,
    label: &'static str,
    shortcut: &'static str,
    #[prop(optional, into)] badge: Signal<usize>,
) -> impl IntoView {
    let RightPanelSignal(panel) = expect_context::<RightPanelSignal>();
    let active = Signal::derive(move || {
        let p = panel.get();
        p.open && p.surface == Some(surface)
    });
    let title = format!("{label} ({shortcut})");
    view! {
        <button
            type="button"
            title=title.clone()
            aria-label=title
            aria-pressed=move || active.get().to_string()
            class=move || {
                let base = "inline-flex h-7 items-center gap-1 rounded-grape px-2 text-label-sm transition-colors";
                if active.get() {
                    format!("{base} bg-longan-800/60 text-rambutan-100")
                } else {
                    format!("{base} text-rambutan-400 hover:bg-longan-800/40 hover:text-rambutan-100")
                }
            }
            on:click=move |_| panel.update(|p| p.toggle_surface(surface))
        >
            <Icon name=icon size=14 />
            <span>{label}</span>
            {move || (badge.get() > 0).then(|| view! {
                <span class="rounded-seed bg-pandan-400/20 px-1 text-label-sm tabular-nums text-pandan-300">{badge.get()}</span>
            })}
        </button>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_rejects_empty_and_unchanged() {
        assert_eq!(resolve_rename("   ", "Old"), None);
        assert_eq!(resolve_rename("Old", "Old"), None);
        assert_eq!(
            resolve_rename("  New name ", "Old"),
            Some("New name".to_string())
        );
    }
}
