//! Workspace navigation stays above the canvas, independent of the thread drawer.

use bruhs_models::SidebarTab;
use leptos::prelude::*;

use crate::state::{
    ActiveView, AppView, SetActiveView, SetSidebarOpen, SidebarOpen, WorkspaceMode,
};
use crate::ui::Icon;

#[component]
pub fn WorkspaceModes() -> impl IntoView {
    let WorkspaceMode(mode) = expect_context::<WorkspaceMode>();
    let ActiveView(active_view) = expect_context::<ActiveView>();
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();
    let SidebarOpen(sidebar_open) = expect_context::<SidebarOpen>();
    let SetSidebarOpen(set_sidebar_open) = expect_context::<SetSidebarOpen>();

    view! {
        <nav aria-label="Workspace mode" class="relative flex h-11 shrink-0 items-center gap-1 px-3">
            <span aria-hidden="true" class="pointer-events-none absolute inset-x-0 bottom-0 h-px bg-longan-800/50"></span>
            <button type="button" aria-label="Toggle sidebar (⌘B)"
                aria-expanded=move || sidebar_open.get().to_string()
                class="inline-flex size-11 shrink-0 items-center justify-center rounded-grape text-rambutan-400 hover:bg-longan-800/40 hover:text-rambutan-100"
                on:click=move |_| set_sidebar_open.set(!sidebar_open.get_untracked())>
                <Icon name="panel-left" size=16 />
            </button>
            {[SidebarTab::Chat, SidebarTab::Code].into_iter().map(|tab| view! {
                <button type="button"
                    aria-label=tab.label()
                    aria-pressed=move || (active_view.get() == AppView::Chat && mode.get() == tab).to_string()
                    class=move || format!(
                        "inline-flex h-11 shrink-0 items-center gap-2 border-b-2 px-3 text-label {}",
                        if active_view.get() == AppView::Chat && mode.get() == tab {
                            "border-blueberry-400 text-rambutan-100"
                        } else {
                            "border-transparent text-rambutan-500 hover:bg-longan-800/40 hover:text-rambutan-100"
                        }
                    )
                    on:click=move |_| {
                        if mode.get_untracked() != tab { mode.set(tab); }
                        // Avoid remounting ChatPage (and its composer) on repeated switches.
                        if active_view.get_untracked() != AppView::Chat {
                            set_active_view.set(AppView::Chat);
                        }
                    }>
                    <Icon name=if tab == SidebarTab::Chat { "message-square" } else { "terminal" } size=14 />
                    {tab.label()}
                </button>
            }).collect_view()}
        </nav>
    }
}
