//! Find bar shared by the thread and the diff. `Cmd+F` opens it; Enter and
//! Shift+Enter move; Escape closes. The shell decides which one opens.

use leptos::html::Input;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use bruhs_models::logic::find::cycle;

/// Bumped by the shell on `Cmd+F`. `0` means the user has not asked yet.
/// The open flags are shared so Escape in the shell can close the bar
/// when focus has left the input.
#[derive(Copy, Clone)]
pub struct FindCommands {
    pub thread: RwSignal<u32>,
    pub diff: RwSignal<u32>,
    pub thread_open: RwSignal<bool>,
    pub diff_open: RwSignal<bool>,
}

#[component]
pub fn FindBar(
    /// Placeholder, also the accessible name.
    #[prop(into)]
    label: String,
    query: RwSignal<String>,
    index: RwSignal<usize>,
    count: Signal<usize>,
    open: RwSignal<bool>,
    /// Changes when `Cmd+F` is pressed again, so an open bar refocuses.
    focus_gen: Signal<u32>,
) -> impl IntoView {
    let input_ref = NodeRef::<Input>::new();
    let label_for_input = label.clone();
    Effect::new(move |_| {
        let generation = focus_gen.get();
        if !open.get() || generation == 0 {
            return;
        }
        // The effect can run before the input is mounted. A zero-delay
        // timer focuses it after that paint, and again on the next Cmd+F.
        gloo_timers::callback::Timeout::new(0, move || {
            if let Some(input) = input_ref.get_untracked() {
                let _ = input.focus();
                input.select();
            }
        })
        .forget();
    });

    let step = move |forward: bool| {
        let len = count.get_untracked();
        index.set(cycle(len, index.get_untracked(), forward));
    };

    view! {
        <div
            data-find-bar=""
            class="z-40 flex h-9 shrink-0 items-center gap-2 border-b border-longan-800/60 bg-longan-900 px-3"
        >
            <input
                node_ref=input_ref
                type="text"
                aria-label=label_for_input
                placeholder=label
                class="h-6 min-w-0 flex-1 bg-transparent text-body-sm text-rambutan-100 placeholder:text-rambutan-500 focus:outline-none"
                prop:value=move || query.get()
                on:input=move |ev| {
                    query.set(event_target_value(&ev));
                    index.set(0);
                }
                on:keydown=move |ev: web_sys::KeyboardEvent| {
                    match ev.key().as_str() {
                        "Enter" => {
                            ev.prevent_default();
                            ev.stop_propagation();
                            step(!ev.shift_key());
                        }
                        "Escape" => {
                            ev.prevent_default();
                            ev.stop_propagation();
                            open.set(false);
                        }
                        _ => {}
                    }
                }
            />
            <span class="shrink-0 tabular-nums text-label-sm text-rambutan-400">
                {move || {
                    let n = count.get();
                    let q = query.get();
                    if q.is_empty() {
                        String::new()
                    } else if n == 0 {
                        "No matches".to_string()
                    } else {
                        let shown = index.get() % n + 1;
                        format!("{shown} of {n}")
                    }
                }}
            </span>
            <button
                type="button"
                aria-label="Previous match"
                class="inline-flex size-6 items-center justify-center rounded-seed text-rambutan-400 hover:bg-longan-800/60 hover:text-rambutan-100"
                on:click=move |_| step(false)
            >
                "↑"
            </button>
            <button
                type="button"
                aria-label="Next match"
                class="inline-flex size-6 items-center justify-center rounded-seed text-rambutan-400 hover:bg-longan-800/60 hover:text-rambutan-100"
                on:click=move |_| step(true)
            >
                "↓"
            </button>
            <button
                type="button"
                aria-label="Close find"
                class="inline-flex size-6 items-center justify-center rounded-seed text-rambutan-400 hover:bg-longan-800/60 hover:text-rambutan-100"
                on:click=move |_| open.set(false)
            >
                "×"
            </button>
        </div>
    }
}

/// Scroll the first element with `data-message-id` and mark it current.
pub fn reveal_message(id: &str) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    clear_marked(&doc, "[data-message-id]");
    let Ok(nodes) = doc.query_selector_all("[data-message-id]") else {
        return;
    };
    for i in 0..nodes.length() {
        let Some(node) = nodes.item(i) else { continue };
        let Ok(el) = node.dyn_into::<web_sys::Element>() else {
            continue;
        };
        if el.get_attribute("data-message-id").as_deref() != Some(id) {
            continue;
        }
        let _ = el.set_attribute("data-find-current", "true");
        scroll_center(&el);
        break;
    }
}

/// Scroll a diff line. `line` `None` scrolls the file header.
pub fn reveal_diff(path: &str, line: Option<usize>) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    clear_marked(&doc, "[data-diff-path], [data-diff-line]");
    let Ok(nodes) = doc.query_selector_all("[data-diff-path]") else {
        return;
    };
    for i in 0..nodes.length() {
        let Some(node) = nodes.item(i) else { continue };
        let Ok(section) = node.dyn_into::<web_sys::Element>() else {
            continue;
        };
        if section.get_attribute("data-diff-path").as_deref() != Some(path) {
            continue;
        }
        let target = line.and_then(|line| {
            section
                .query_selector(&format!("[data-diff-line=\"{line}\"]"))
                .ok()
                .flatten()
        });
        let el = target.unwrap_or(section);
        let _ = el.set_attribute("data-find-current", "true");
        scroll_center(&el);
        break;
    }
}

fn clear_marked(doc: &web_sys::Document, selector: &str) {
    let Ok(nodes) = doc.query_selector_all(selector) else {
        return;
    };
    for i in 0..nodes.length() {
        if let Some(node) = nodes.item(i) {
            if let Ok(el) = node.dyn_into::<web_sys::Element>() {
                let _ = el.remove_attribute("data-find-current");
            }
        }
    }
}

/// Drop the outline on a thread match.
pub fn clear_message_mark() {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    clear_marked(&doc, "[data-message-id]");
}

/// Drop the outline on a diff match.
pub fn clear_diff_mark() {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    clear_marked(&doc, "[data-diff-path], [data-diff-line]");
}

fn scroll_center(el: &web_sys::Element) {
    let opts = web_sys::ScrollIntoViewOptions::new();
    opts.set_block(web_sys::ScrollLogicalPosition::Center);
    opts.set_inline(web_sys::ScrollLogicalPosition::Nearest);
    el.scroll_into_view_with_scroll_into_view_options(&opts);
}

/// True when `activeElement` is inside `selector`.
pub fn focus_within(selector: &str) -> bool {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.active_element())
        .and_then(|el| el.closest(selector).ok().flatten())
        .is_some()
}
