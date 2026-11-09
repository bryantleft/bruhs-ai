use crate::task_style::PriorityStyle;
use leptos::prelude::*;

use crate::state::{format_due_date, WorkItem};

/// 4-bar priority indicator (mirrors task_row.rs but inlined here so the
/// card is self-contained).
fn priority_bars(level: u8, color_class: &str) -> impl IntoView {
    let bars: Vec<_> = (1u8..=4u8)
        .map(|i| {
            let h = match i {
                1 => "h-1",
                2 => "h-1.5",
                3 => "h-2",
                _ => "h-2.5",
            };
            let fill = if i <= level {
                "bg-current"
            } else {
                "bg-current/20"
            };
            view! { <span class=format!("w-0.5 rounded-orb {h} {fill}")></span> }
        })
        .collect();
    let outer = format!("inline-flex items-end gap-px {color_class}");
    view! { <span class=outer>{bars}</span> }
}

/// Card on the Kanban board — mirrors apps/web tasks/task-card.tsx:
/// border-style (not filled), priority bars + title (line-clamp-2),
/// optional label/due bottom row.
#[component]
pub fn TaskCard(
    item: WorkItem,
    #[prop(into)] selected: Signal<bool>,
    on_select: Callback<()>,
) -> impl IntoView {
    let priority_color = item.priority.color_class();
    let priority_label = item.priority.label();
    let priority_level = item.priority.bars();
    let title = item.title.clone();
    let due_meta = item.due_date.as_deref().and_then(format_due_date);
    let drag_id = item.id.clone();

    let card_class = move || {
        let base = "rounded-grape border p-3 cursor-pointer transition-colors";
        if selected.get() {
            format!("{base} border-longan-600 bg-longan-800/50")
        } else {
            format!("{base} border-longan-700/30 bg-longan-900/60 hover:border-longan-600/50")
        }
    };

    view! {
        <div
            role="button"
            tabindex="0"
            aria-pressed=move || selected.get().to_string()
            class=card_class
            draggable="true"
            on:click=move |_| on_select.run(())
            on:keydown=move |ev| {
                if ev.key() == "Enter" || ev.key() == " " {
                    ev.prevent_default();
                    on_select.run(());
                }
            }
            // HTML5 DnD source: write the task id into DataTransfer so the
            // column's drop handler can match it back. We also set effect to
            // "move" so the cursor reflects the operation.
            on:dragstart=move |ev: web_sys::DragEvent| {
                if let Some(dt) = ev.data_transfer() {
                    let _ = dt.set_data("text/plain", &drag_id);
                    dt.set_effect_allowed("move");
                }
            }
        >
            // Top row: priority bars + title
            <div class="flex items-start gap-2">
                <span class="flex items-center mt-0.5 shrink-0" title=priority_label>
                    {priority_bars(priority_level, priority_color)}
                </span>
                <span class="text-body-sm text-rambutan-200 line-clamp-2 flex-1">{title}</span>
            </div>

            // Bottom row: due date (labels not yet in data model).
            {due_meta.map(|(text, class)| view! {
                <div class="flex items-center gap-1.5 mt-2 flex-wrap text-label-sm text-rambutan-500">
                    <span class=class>{text}</span>
                </div>
            })}
        </div>
    }
}
