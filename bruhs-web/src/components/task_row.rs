use crate::task_style::{PriorityStyle, WorkItemStatusStyle};
use leptos::prelude::*;

use crate::state::{format_due_date, WorkItem};

/// Render a 4-bar priority indicator — matches TS apps/web's PriorityIcon
/// (apps/web/src/components/tasks/priority-icon.tsx). Each bar's height
/// scales with index; bars at or below `level` are filled, others muted.
fn priority_bars(level: u8, color_class: &str) -> impl IntoView {
    let bars: Vec<_> = (1u8..=4u8)
        .map(|i| {
            let height_class = match i {
                1 => "h-1",
                2 => "h-1.5",
                3 => "h-2",
                _ => "h-2.5",
            };
            let fill_class = if i <= level {
                "bg-current"
            } else {
                "bg-current/20"
            };
            view! {
                <span class=format!("w-0.5 rounded-orb {height_class} {fill_class}")></span>
            }
        })
        .collect();
    let outer = format!("inline-flex items-end gap-px {color_class}");
    view! { <span class=outer>{bars}</span> }
}

#[component]
pub fn TaskRow(
    item: WorkItem,
    #[prop(into)] selected: Signal<bool>,
    on_select: Callback<()>,
) -> impl IntoView {
    let status_dot_class = format!(
        "inline-block size-2.5 rounded-orb {}",
        item.status.dot_bg_class()
    );
    let status_label = item.status.label();
    let priority_color = item.priority.color_class();
    let priority_label = item.priority.label();
    let priority_level = item.priority.bars();
    let title = item.title.clone();
    let due_meta = item.due_date.as_deref().and_then(format_due_date);

    let row_class = move || {
        let base =
            "flex items-center gap-2 py-2 px-3 rounded-grape cursor-pointer transition-colors";
        if selected.get() {
            format!("{base} bg-longan-800/40")
        } else {
            format!("{base} hover:bg-longan-800/30")
        }
    };

    view! {
        <div
            role="button"
            tabindex="0"
            aria-pressed=move || selected.get().to_string()
            class=row_class
            on:click=move |_| on_select.run(())
            on:keydown=move |ev| {
                if ev.key() == "Enter" || ev.key() == " " {
                    ev.prevent_default();
                    on_select.run(());
                }
            }
        >
            // Status dot — fixed-width slot to keep the title column aligned.
            <span class="flex items-center justify-center w-6 shrink-0">
                <span class=status_dot_class title=status_label></span>
            </span>

            // Title
            <span class="flex-1 truncate text-body-sm text-rambutan-200">
                {title}
            </span>

            // Priority bars
            <span class="flex items-center justify-center w-8 shrink-0" title=priority_label>
                {priority_bars(priority_level, priority_color)}
            </span>

            // Due date — color-coded relative format from format_due_date.
            {match due_meta {
                Some((text, class)) => view! {
                    <span class=format!("w-20 text-right text-label shrink-0 {class}")>
                        {text}
                    </span>
                }.into_any(),
                None => view! {
                    <span class="w-20 text-right text-label shrink-0 text-rambutan-500"></span>
                }.into_any(),
            }}
        </div>
    }
}
