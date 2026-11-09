use leptos::prelude::*;

use crate::components::task_pagination::{page_start, TaskPagination};
use crate::components::task_row::TaskRow;
use crate::state::{SelectedWorkItem, SetSelectedWorkItem, WorkItem};

#[component]
pub fn TaskList(
    /// Filtered list of items already scoped to the active project.
    #[prop(into)]
    items: Signal<Vec<WorkItem>>,
) -> impl IntoView {
    let SelectedWorkItem(selected) = expect_context::<SelectedWorkItem>();
    let SetSelectedWorkItem(set_selected) = expect_context::<SetSelectedWorkItem>();
    const PAGE_SIZE: usize = 100;
    let page = RwSignal::new(0usize);
    let total = Signal::derive(move || items.with(Vec::len));
    let visible = Memo::new(move |_| {
        items.with(|items| {
            let start = page_start(items.len(), page.get(), PAGE_SIZE);
            items
                .iter()
                .skip(start)
                .take(PAGE_SIZE)
                .cloned()
                .collect::<Vec<_>>()
        })
    });
    // Initial hydration or a filter change can relocate the selected task
    // without changing its ID. Ordinary edits must not undo manual paging.
    let selected_location = Memo::new(move |_| {
        let id = selected.get()?;
        items.with(|items| {
            items
                .iter()
                .position(|item| item.id == id)
                .map(|index| (id, index))
        })
    });
    Effect::new(move |_| {
        // Explicitly selecting the same ID again still reveals its page.
        let _ = selected.get();
        if let Some((_, index)) = selected_location.get() {
            page.set(index / PAGE_SIZE);
        }
    });

    view! {
        <div class="flex min-w-0 flex-1 flex-col overflow-hidden">
          <div class="min-h-0 flex-1 overflow-y-auto">
            // Sticky column header — TITLE / PRI / DUE in uppercase muted
            // text. (No LABELS column: the data model has no labels yet —
            // reserving a header for it just reads as a broken feature.)
            <div class="flex items-center gap-2 px-3 py-2 border-b border-longan-800/50 sticky top-0 bg-longan-950 z-10">
                <span class="w-6 shrink-0"></span>
                <span class="flex-1 text-label uppercase tracking-widest text-rambutan-500">
                    "Title"
                </span>
                <span class="w-8 shrink-0 text-center text-label uppercase tracking-widest text-rambutan-500">
                    "Pri"
                </span>
                <span class="w-20 shrink-0 text-right text-label uppercase tracking-widest text-rambutan-500">
                    "Due"
                </span>
            </div>

            // Body
            {move || if total.get() == 0 {
                view! {
                    <div class="flex items-center justify-center py-12">
                        <span class="text-body-sm text-rambutan-500">"no tasks yet"</span>
                    </div>
                }.into_any()
            } else {
                view! {
                    <div class="py-1">
                        <For
                            each=move || visible.get()
                            key=|item: &WorkItem| item.id.clone()
                            children=move |item| {
                                let id = item.id.clone();
                                let id_for_select = id.clone();
                                let id_for_match = id;
                                let on_select = Callback::new(move |_| {
                                    set_selected.set(Some(id_for_select.clone()));
                                });
                                let is_selected = Signal::derive(move || {
                                    selected.get().as_deref() == Some(id_for_match.as_str())
                                });
                                view! {
                                    <TaskRow
                                        item=item
                                        selected=is_selected
                                        on_select=on_select
                                    />
                                }
                            }
                        />
                    </div>
                }.into_any()
            }}
          </div>
          <TaskPagination total page page_size=PAGE_SIZE label="Task list pages" />
        </div>
    }
}
