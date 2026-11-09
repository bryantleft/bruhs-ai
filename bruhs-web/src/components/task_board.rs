use crate::task_style::WorkItemStatusStyle;
use leptos::prelude::*;

use crate::components::task_card::TaskCard;
use crate::components::task_pagination::{page_start, TaskPagination};
use crate::state::{
    SelectedWorkItem, SetSelectedWorkItem, SetWorkItemsSignal, WorkItem, WorkItemStatus,
};
use crate::storage;

const BOARD_COLUMNS: &[WorkItemStatus] = &[
    WorkItemStatus::Backlog,
    WorkItemStatus::Todo,
    WorkItemStatus::InProgress,
    WorkItemStatus::Done,
];

#[component]
pub fn TaskBoard(
    /// Filtered list of items already scoped to the active project.
    #[prop(into)]
    items: Signal<Vec<WorkItem>>,
) -> impl IntoView {
    let SelectedWorkItem(selected) = expect_context::<SelectedWorkItem>();
    let SetSelectedWorkItem(set_selected) = expect_context::<SetSelectedWorkItem>();
    // For drop handlers — when a card is dropped on a column we mutate the
    // task's status and persist it.
    let SetWorkItemsSignal(set_work_items) = expect_context::<SetWorkItemsSignal>();

    // Move a task to a new status: update the in-memory list and persist.
    // Called by every column's drop handler.
    let move_to_status = move |task_id: String, new_status: WorkItemStatus| {
        let mut moved = None;
        set_work_items.update(|items| {
            for item in items.iter_mut() {
                if item.id == task_id && item.status != new_status {
                    let original = item.clone();
                    item.status = new_status;
                    item.updated_at = crate::clock::now_ms();
                    moved = Some((original, item.clone()));
                    break;
                }
            }
        });
        if let Some((original, item)) = moved {
            storage::persist_work_item(Some(original), item);
        }
    };

    view! {
        // Mirrors apps/web tasks/task-board.tsx: flex gap-3 p-4 across the
        // viewport, each column a flex-1 rounded-grape shell, completed
        // statuses muted to 70% opacity over the canvas tone.
        <div
            role="region"
            aria-label="Task board columns"
            tabindex="0"
            class="flex flex-1 gap-3 p-4 h-full overflow-x-auto"
        >
            {BOARD_COLUMNS.iter().map(|&status| {
                let column_items = Memo::new(move |_| {
                    items.with(|items| items.iter().filter(|w| w.status == status).cloned().collect::<Vec<_>>())
                });
                const PAGE_SIZE: usize = 40;
                let page = RwSignal::new(0usize);
                let total = Signal::derive(move || column_items.with(Vec::len));
                let visible = Memo::new(move |_| column_items.with(|items| {
                    let start = page_start(items.len(), page.get(), PAGE_SIZE);
                    items.iter().skip(start).take(PAGE_SIZE).cloned().collect::<Vec<_>>()
                }));
                // Follow a selected task into its destination column, even when
                // its ID hasn't changed. Memoize location so ordinary edits do
                // not undo the user's manual pagination.
                let selected_location = Memo::new(move |_| {
                    let id = selected.get()?;
                    column_items.with(|items| {
                        items.iter().position(|item| item.id == id).map(|index| (id, index))
                    })
                });
                Effect::new(move |_| {
                    // Explicitly selecting the same task again should reveal
                    // it after the user browsed another page.
                    let _ = selected.get();
                    if let Some((_, index)) = selected_location.get() {
                        page.set(index / PAGE_SIZE);
                    }
                });
                let label = status.label();
                let dot_bg = status.dot_bg_class();
                // Completed columns mute via opacity, NOT via a darker bg —
                // longan-950-on-longan-950 rendered as no panel at all.
                let base_class = if status.is_completed() {
                    "flex flex-1 min-w-[220px] flex-col rounded-grape overflow-hidden bg-longan-900/30 opacity-70"
                } else {
                    "flex flex-1 min-w-[220px] flex-col rounded-grape overflow-hidden bg-longan-900/30"
                };
                let (drag_over, set_drag_over) = signal(false);
                let column_class = move || {
                    if drag_over.get() {
                        format!("{base_class} inset-ring inset-ring-persimmon-400/40")
                    } else {
                        base_class.to_string()
                    }
                };
                view! {
                    <div
                        class=column_class
                        // Drop target: prevent default on dragover so the
                        // browser allows the drop, then handle it.
                        on:dragover=move |ev: web_sys::DragEvent| {
                            ev.prevent_default();
                            if !drag_over.get_untracked() {
                                set_drag_over.set(true);
                            }
                        }
                        on:dragleave=move |_| set_drag_over.set(false)
                        on:drop=move |ev: web_sys::DragEvent| {
                            ev.prevent_default();
                            set_drag_over.set(false);
                            let id = ev.data_transfer()
                                .and_then(|dt| dt.get_data("text/plain").ok())
                                .unwrap_or_default();
                            if !id.is_empty() {
                                move_to_status(id, status);
                            }
                        }
                    >
                        // Column header — colored dot + status name + count chip
                        <div class="px-3 py-2.5 text-label text-rambutan-400 flex items-center gap-2">
                            <span class=format!("size-2 rounded-orb {dot_bg}")></span>
                            <span>{label}</span>
                            <span class="text-rambutan-500 bg-longan-800/50 rounded-orb px-1.5 text-code-sm tabular-nums">
                                {move || total.get()}
                            </span>
                        </div>

                        // Column body — droppable in TS via @dnd-kit; no DnD here yet.
                        <div class="flex-1 overflow-y-auto space-y-2 p-2">
                            {move || if total.get() == 0 {
                                view! {
                                    <p class="text-rambutan-500 text-body-sm text-center py-4">
                                        "no tasks"
                                    </p>
                                }.into_any()
                            } else {
                                view! {
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
                                                <TaskCard
                                                    item=item
                                                    selected=is_selected
                                                    on_select=on_select
                                                />
                                            }
                                        }
                                    />
                                }.into_any()
                            }}
                        </div>
                        <TaskPagination total page page_size=PAGE_SIZE label=format!("{} task pages", label) />
                    </div>
                }
            }).collect_view()}
        </div>
    }
}
