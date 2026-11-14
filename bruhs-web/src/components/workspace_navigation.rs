//! Workspace navigation stays above the canvas, independent of the thread drawer.

use leptos::prelude::*;

use crate::state::{ActiveView, AppView, SetActiveView, SetSidebarOpen, SidebarOpen};
use crate::ui::Icon;

#[component]
pub fn WorkspaceNavigation() -> impl IntoView {
    let ActiveView(active_view) = expect_context::<ActiveView>();
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();
    let SidebarOpen(sidebar_open) = expect_context::<SidebarOpen>();
    let SetSidebarOpen(set_sidebar_open) = expect_context::<SetSidebarOpen>();

    view! {
        <nav aria-label="Workspace" class="relative flex h-11 shrink-0 items-center gap-1 px-3">
            <span aria-hidden="true" class="pointer-events-none absolute inset-x-0 bottom-0 h-px bg-longan-800/50"></span>
            <button type="button" aria-label="Toggle sidebar (⌘B)"
                aria-expanded=move || sidebar_open.get().to_string()
                class="inline-flex size-11 shrink-0 items-center justify-center rounded-grape text-rambutan-400 hover:bg-longan-800/40 hover:text-rambutan-100"
                on:click=move |_| set_sidebar_open.set(!sidebar_open.get_untracked())>
                <Icon name="panel-left" size=16 />
            </button>
            <button type="button" aria-label="Threads"
                aria-pressed=move || (active_view.get() == AppView::Chat).to_string()
                class=move || format!(
                    "inline-flex h-11 shrink-0 items-center gap-2 border-b-2 px-3 text-label {}",
                    if active_view.get() == AppView::Chat {
                        "border-blueberry-400 text-rambutan-100"
                    } else {
                        "border-transparent text-rambutan-500 hover:bg-longan-800/40 hover:text-rambutan-100"
                    }
                )
                on:click=move |_| {
                    if active_view.get_untracked() != AppView::Chat {
                        set_active_view.set(AppView::Chat);
                    }
                }>
                <Icon name="message-square" size=14 />
                "Threads"
            </button>
        </nav>
    }
}
