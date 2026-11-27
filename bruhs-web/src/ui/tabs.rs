//! W3C APG roving-focus tabs (`role="tablist"`, `role="tab"`, `role="tabpanel"`).
//!
//! Only the selected tab sits in the Tab order (`tabindex="0"`); ArrowLeft,
//! ArrowRight, Home, and End move focus and selection across enabled tabs.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

/// Compute the next tab index for horizontal roving-focus navigation.
pub fn next_tab_index(current: usize, count: usize, key: &str) -> Option<usize> {
    if count == 0 {
        return None;
    }
    match key {
        "ArrowRight" => Some((current + 1) % count),
        "ArrowLeft" => Some((current + count - 1) % count),
        "Home" => Some(0),
        "End" => Some(count - 1),
        _ => None,
    }
}

/// Shared `on:keydown` handler for any horizontal `role="tablist"`.
///
/// Moves focus and activates the target tab on `ArrowLeft`, `ArrowRight`,
/// `Home`, or `End` without interfering with modified shortcuts or IME input.
pub fn tablist_keydown(ev: &web_sys::KeyboardEvent) {
    if ev.default_prevented()
        || ev.is_composing()
        || ev.alt_key()
        || ev.ctrl_key()
        || ev.meta_key()
        || ev.shift_key()
    {
        return;
    }
    let key = ev.key();
    if !matches!(key.as_str(), "ArrowLeft" | "ArrowRight" | "Home" | "End") {
        return;
    }
    let Some(list) = ev
        .current_target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
    else {
        return;
    };
    let Ok(nodes) = list.query_selector_all("[role='tab']:not([disabled])") else {
        return;
    };
    let mut tabs = Vec::with_capacity(nodes.length() as usize);
    for i in 0..nodes.length() {
        if let Some(el) = nodes
            .item(i)
            .and_then(|node| node.dyn_into::<web_sys::HtmlElement>().ok())
        {
            tabs.push(el);
        }
    }
    let active = document().active_element();
    let Some(current) = tabs
        .iter()
        .position(|tab| active.as_ref().is_some_and(|a| a.is_same_node(Some(tab))))
    else {
        return;
    };
    let Some(next) = next_tab_index(current, tabs.len(), &key) else {
        return;
    };
    ev.prevent_default();
    ev.stop_propagation();
    let _ = tabs[next].focus();
    tabs[next].click();
}

#[derive(Clone, Copy)]
pub struct TabsContext {
    pub value: Signal<String>,
    pub on_change: Option<Callback<String>>,
}

#[component]
pub fn Tabs(
    #[prop(into)] value: Signal<String>,
    #[prop(optional)] on_change: Option<Callback<String>>,
    #[prop(optional, into)] class: String,
    children: Children,
) -> impl IntoView {
    provide_context(TabsContext { value, on_change });
    let classes = format!("flex flex-col {class}");
    view! { <div class=classes>{children()}</div> }
}

#[component]
pub fn TabsList(
    #[prop(optional, into)] aria_label: Option<String>,
    #[prop(optional, into)] class: String,
    children: Children,
) -> impl IntoView {
    let final_class = format!(
        "inline-flex items-center gap-1 rounded-grape bg-longan-800 inset-ring inset-ring-white/10 p-1 {class}"
    );
    view! {
        <div
            class=final_class
            role="tablist"
            aria-orientation="horizontal"
            aria-label=aria_label
            on:keydown=move |ev| tablist_keydown(&ev)
        >
            {children()}
        </div>
    }
}

#[component]
pub fn TabsTrigger(
    #[prop(into)] value: String,
    #[prop(optional, into)] class: String,
    children: Children,
) -> impl IntoView {
    let ctx = expect_context::<TabsContext>();
    let value_for_active = value.clone();
    let value_for_click = value;
    let is_active = Memo::new(move |_| ctx.value.get() == value_for_active);

    let class_dyn = move || {
        let active_class = if is_active.get() {
            "bg-persimmon-500 text-longan-950"
        } else {
            "text-rambutan-200 hover:text-rambutan-100"
        };
        format!(
            "px-3 py-1.5 rounded-grape text-button-sm transition focus-visible:outline-2 focus-visible:outline-persimmon-400 cursor-pointer {active_class} {class}"
        )
    };

    view! {
        <button
            type="button"
            class=class_dyn
            role="tab"
            aria-selected=move || is_active.get().to_string()
            tabindex=move || if is_active.get() { "0" } else { "-1" }
            on:click=move |_| {
                if let Some(cb) = ctx.on_change {
                    cb.run(value_for_click.clone());
                }
            }
        >
            {children()}
        </button>
    }
}

#[component]
pub fn TabsContent(
    #[prop(into)] value: String,
    #[prop(optional, into)] class: String,
    children: ChildrenFn,
) -> impl IntoView {
    let ctx = expect_context::<TabsContext>();
    let value_for_match = value;
    let final_class = format!("mt-2 {class}");
    view! {
        <Show when=move || ctx.value.get() == value_for_match fallback=|| ()>
            <div class=final_class.clone() role="tabpanel">{children()}</div>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use super::next_tab_index;

    #[test]
    fn next_tab_index_wraps_and_handles_home_and_end() {
        assert_eq!(next_tab_index(0, 3, "ArrowRight"), Some(1));
        assert_eq!(next_tab_index(2, 3, "ArrowRight"), Some(0));
        assert_eq!(next_tab_index(0, 3, "ArrowLeft"), Some(2));
        assert_eq!(next_tab_index(1, 3, "Home"), Some(0));
        assert_eq!(next_tab_index(1, 3, "End"), Some(2));
        assert_eq!(next_tab_index(0, 3, "ArrowDown"), None);
        assert_eq!(next_tab_index(0, 0, "ArrowRight"), None);
    }
}
