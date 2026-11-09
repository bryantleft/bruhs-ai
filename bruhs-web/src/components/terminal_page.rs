//! Terminal view (`AppView::Terminal`): terminals that exist on their own,
//! not under a chat thread. Hosts the shared panel in page mode; the panel
//! keeps the tab strip, the "+" menu, and the empty state.

use leptos::prelude::*;

use crate::components::terminal_panel::{TerminalPanel, TerminalPanelMode};
use crate::state::ActiveProjectSignal;

#[component]
pub fn TerminalPage() -> impl IntoView {
    let ActiveProjectSignal(active_project) = expect_context::<ActiveProjectSignal>();
    let context_project = Signal::derive(move || active_project.get().map(|p| (p.name, p.path)));

    view! {
        <div class="flex h-full min-h-0 flex-1 flex-col" data-terminal-page="">
            <header class="flex h-[52px] shrink-0 items-center gap-2 px-4 text-label text-rambutan-100">
                <span>"Terminal"</span>
                <span class="text-label-sm text-rambutan-500">"— shells on this machine, independent of any thread"</span>
            </header>
            <TerminalPanel mode=TerminalPanelMode::Page context_project=context_project />
        </div>
    }
}
