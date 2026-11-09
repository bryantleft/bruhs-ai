//! Bounded, keyboard-first record selection without mounting the whole library.
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::ui::{Dialog, DropdownOption, Icon};

mod search;
use search::{Index, PAGE_SIZE};

#[component]
pub(super) fn RecordPicker(
    options: Signal<Vec<DropdownOption>>,
    label: &'static str,
    search_label: &'static str,
    placeholder: &'static str,
    on_change: Callback<String>,
) -> impl IntoView {
    let open = RwSignal::new(false);
    let query = RwSignal::new(String::new());
    let highlighted = RwSignal::new(0usize);
    let prefix = StoredValue::new(format!("record-picker-{}", uuid::Uuid::new_v4()));
    let list_id = prefix.with_value(|id| format!("{id}-list"));
    let input_id = prefix.with_value(|id| format!("{id}-input"));
    let list_ref = NodeRef::<leptos::html::Div>::new();
    let index = Memo::new(move |_| options.with(|items| Index::new(items)));
    let matches = Memo::new(move |_| index.with(|index| index.matching(&query.get())));
    let count = Memo::new(move |_| matches.with(Vec::len));
    let current = move || highlighted.get().min(count.get().saturating_sub(1));
    let visible = Memo::new(move |_| {
        index.with(|index| matches.with(|matches| index.page(matches, current())))
    });
    let choose = Callback::new(move |position: usize| {
        let value = index.with_untracked(|index| {
            matches.with_untracked(|matches| index.value(matches, position))
        });
        if let Some(value) = value {
            on_change.run(value);
            open.set(false);
        }
    });
    let keys = Callback::new(move |event: web_sys::KeyboardEvent| {
        if event.is_composing() || event.ctrl_key() || event.meta_key() || event.alt_key() {
            return;
        }
        if event.key() == "Enter" {
            event.prevent_default();
            choose.run(current());
        } else if let Some(next) = search::navigate(current(), count.get(), &event.key()) {
            event.prevent_default();
            highlighted.set(next);
        }
    });
    // Scroll only the bounded result list after its new page mounts; never
    // move focus away from the search input or scroll the underlying editor.
    Effect::new(move |_| {
        let position = current();
        if !open.get() {
            return;
        }
        queue_microtask(move || {
            let Some(Some(list)) = list_ref.try_get_untracked() else {
                return;
            };
            let element = list
                .query_selector(&format!("[data-picker-position='{position}']"))
                .ok()
                .flatten()
                .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok());
            if let Some(element) = element {
                let top = element.offset_top();
                let bottom = top + element.offset_height();
                if top < list.scroll_top() {
                    list.set_scroll_top(top);
                } else if bottom > list.scroll_top() + list.client_height() {
                    list.set_scroll_top(bottom - list.client_height());
                }
            }
        });
    });
    view! {
        <button type="button" aria-label=label aria-haspopup="dialog" aria-expanded=move || open.get().to_string()
            class="inline-flex min-h-11 w-full items-center justify-between gap-2 rounded-grape bg-white/5 px-3 py-2 text-left text-body-sm text-rambutan-300 inset-ring inset-ring-white/10 hover:bg-white/[0.07] focus-visible:outline-2 focus-visible:-outline-offset-1 focus-visible:outline-persimmon-400 cursor-pointer"
            on:click=move |_| { query.set(String::new()); highlighted.set(0); open.set(true); }>
            <span class="min-w-0 truncate">{placeholder}</span><Icon name="search" size=14 class="shrink-0 text-rambutan-500" />
        </button>
        <Dialog open=Signal::derive(move || open.get()) label=label max_width="max-w-[calc(100vw-2rem)] sm:max-w-xl" padding_top="2rem"
            on_close=Callback::new(move |_| open.set(false))>
            <div class="flex max-h-[calc(100dvh-4rem)] min-h-0 flex-col">
                <div class="shrink-0 border-b border-longan-700 px-3 pb-3">
                    <div class="flex min-h-11 items-center justify-between gap-3">
                        <label for=input_id.clone() class="text-label text-rambutan-200">{search_label}</label>
                        <button type="button" aria-label="Close record picker" class="flex size-11 shrink-0 items-center justify-center rounded-grape text-rambutan-400 hover:bg-longan-700 focus-visible:outline-2 focus-visible:outline-persimmon-400 cursor-pointer" on:click=move |_| open.set(false)><Icon name="x" size=16 /></button>
                    </div>
                    <input id=input_id.clone() type="search" role="combobox" aria-label=search_label data-autofocus=""
                        aria-controls=list_id.clone() aria-expanded="true" aria-autocomplete="list"
                        aria-activedescendant=move || (count.get() > 0).then(|| prefix.with_value(|id| format!("{id}-option-{}", current())))
                        class="block w-full rounded-grape bg-white/5 px-3 py-2 text-body text-rambutan-100 placeholder:text-rambutan-500 inset-ring inset-ring-white/10 focus-visible:outline-2 -outline-offset-1 focus-visible:outline-persimmon-400"
                        placeholder="Type a name…" prop:value=move || query.get() on:keydown=move |event| keys.run(event)
                        on:input=move |event| { query.set(event_target_value(&event)); highlighted.set(0); } />
                </div>
                <div class="shrink-0 px-3 py-2 text-label-sm tabular-nums text-rambutan-400" role="status" data-picker-result-count=move || count.get().to_string()>
                    {move || format!("{} matching {}", count.get(), if count.get() == 1 { "record" } else { "records" })}
                </div>
                <div id=list_id.clone() node_ref=list_ref role="listbox" aria-label=label class="relative min-h-0 flex-1 overflow-y-auto px-1 pb-1">
                    {move || visible.get().into_iter().map(|(position, option)| {
                        let accessible_label = option.label.clone();
                        view! {
                        <button type="button" role="option" tabindex="-1" aria-label=accessible_label
                            id=prefix.with_value(|id| format!("{id}-option-{position}")) data-picker-value=option.value data-picker-position=position
                            aria-selected=move || (current() == position).to_string() aria-posinset=position + 1 aria-setsize=move || count.get()
                            class=move || format!("flex min-h-11 w-full items-center justify-between gap-3 rounded-grape px-3 py-2 text-left text-body-sm cursor-pointer {}", if current() == position { "bg-blueberry-500/15 text-blueberry-200 inset-ring inset-ring-blueberry-400/30" } else { "text-rambutan-200 hover:bg-longan-700" })
                            on:click=move |_| choose.run(position)>
                            <span class="min-w-0 truncate">{option.label}</span>
                            {option.group.map(|group| view! { <span class="max-w-24 shrink-0 truncate text-label-sm text-rambutan-400">{group}</span> })}
                        </button>
                    } }).collect_view()}
                    <Show when=move || count.get() == 0><p class="px-3 py-5 text-body-sm text-rambutan-400">"No matching records. Try another name."</p></Show>
                </div>
                <div class="flex shrink-0 flex-wrap items-center justify-between gap-2 border-t border-longan-700 px-3 py-2 text-label-sm text-rambutan-400">
                    <span>"↑↓ to move · Enter to choose"</span>
                    <Show when=move || { count.get() > PAGE_SIZE }>
                        <div class="flex items-center gap-2">
                            <button type="button" aria-label="Previous results" disabled=move || { current() < PAGE_SIZE }
                                class="min-h-11 rounded-grape px-2 hover:bg-longan-700 disabled:opacity-40 focus-visible:outline-2 focus-visible:outline-persimmon-400 cursor-pointer"
                                on:click=move |_| highlighted.set((current() / PAGE_SIZE).saturating_sub(1) * PAGE_SIZE)>"Previous"</button>
                            <span class="tabular-nums">{move || format!("{} / {}", current() / PAGE_SIZE + 1, count.get().div_ceil(PAGE_SIZE))}</span>
                            <button type="button" aria-label="Next results" disabled=move || { current() / PAGE_SIZE + 1 >= count.get().div_ceil(PAGE_SIZE) }
                                class="min-h-11 rounded-grape px-2 hover:bg-longan-700 disabled:opacity-40 focus-visible:outline-2 focus-visible:outline-persimmon-400 cursor-pointer"
                                on:click=move |_| highlighted.set((current() / PAGE_SIZE + 1) * PAGE_SIZE)>"Next"</button>
                        </div>
                    </Show>
                </div>
            </div>
        </Dialog>
    }
}
