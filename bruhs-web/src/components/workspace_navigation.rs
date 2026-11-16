//! Workspace navigation stays above the canvas, independent of the thread drawer.
//!
//! The bar carries the sidebar toggle only: a thread's card (or ⌘K) returns
//! to the conversation from any view, so there is no Threads tab.

use leptos::prelude::*;

use crate::state::{SetSidebarOpen, SidebarOpen};
use crate::ui::Icon;

#[component]
pub fn WorkspaceNavigation() -> impl IntoView {
    let SidebarOpen(sidebar_open) = expect_context::<SidebarOpen>();
    let SetSidebarOpen(set_sidebar_open) = expect_context::<SetSidebarOpen>();

    view! {
        <nav aria-label="Workspace" class="relative flex h-11 shrink-0 items-center gap-1 px-3">
            <span aria-hidden="true" class="pointer-events-none absolute inset-x-0 bottom-0 h-px bg-longan-800/50"></span>
            // shadcn `SidebarTrigger`: a 28px ghost button; touch keeps 44px
            // through `.bruhs-control`.
            <button type="button" aria-label="Toggle sidebar (⌘B)"
                aria-expanded=move || sidebar_open.get().to_string()
                class="bruhs-control inline-flex size-control-sm shrink-0 items-center justify-center rounded-grape text-rambutan-400 hover:bg-longan-800/50 hover:text-rambutan-100"
                on:click=move |_| set_sidebar_open.set(!sidebar_open.get_untracked())>
                <Icon name="panel-left" size=16 />
            </button>
        </nav>
    }
}
