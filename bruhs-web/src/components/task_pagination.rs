use leptos::prelude::*;

pub fn page_start(total: usize, page: usize, page_size: usize) -> usize {
    page.min(total.saturating_sub(1) / page_size) * page_size
}

#[component]
pub fn TaskPagination(
    total: Signal<usize>,
    page: RwSignal<usize>,
    page_size: usize,
    #[prop(into)] label: String,
) -> impl IntoView {
    let start = Memo::new(move |_| page_start(total.get(), page.get(), page_size));
    view! {
        <Show when=move || { total.get() > page_size }>
            <nav aria-label=label.clone() class="flex shrink-0 flex-wrap items-center justify-between gap-2 border-t border-longan-800/40 px-3 py-2 text-label-sm text-rambutan-400">
                <button type="button" disabled=move || start.get() == 0
                    class="rounded-grape px-2 py-2 hover:bg-longan-800/50 disabled:opacity-40 cursor-pointer"
                    on:click=move |_| page.set((start.get_untracked() / page_size).saturating_sub(1))>"Previous"</button>
                <span role="status" aria-live="polite">{move || format!("{}–{} of {}", start.get() + 1, (start.get() + page_size).min(total.get()), total.get())}</span>
                <button type="button" disabled=move || { start.get() + page_size >= total.get() }
                    class="rounded-grape px-2 py-2 hover:bg-longan-800/50 disabled:opacity-40 cursor-pointer"
                    on:click=move |_| page.set(start.get_untracked() / page_size + 1)>"Next"</button>
            </nav>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use super::page_start;

    #[test]
    fn page_window_clamps_after_deletions_and_filter_changes() {
        assert_eq!(page_start(10_000, 99, 100), 9_900);
        assert_eq!(page_start(125, 99, 100), 100);
        assert_eq!(page_start(0, usize::MAX, 100), 0);
    }
}
