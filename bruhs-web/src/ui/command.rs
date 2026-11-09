use leptos::prelude::*;

use super::input::Input;

/// Stub: a command palette primitive — search input + filtered list.
///
/// Consumers provide:
/// - `query` / `on_query` for the search state
/// - children for the rendered (already-filtered) list items
#[component]
pub fn Command(
    #[prop(into)] query: Signal<String>,
    #[prop(optional)] on_query: Option<Callback<String>>,
    #[prop(optional, into)] placeholder: String,
    children: Children,
) -> impl IntoView {
    let ph = if placeholder.is_empty() {
        "Search...".to_string()
    } else {
        placeholder
    };

    view! {
        <div class="flex flex-col">
            <div class="px-4 py-3">
                <Input
                    value=query
                    on_input=on_query.unwrap_or(Callback::new(|_| {}))
                    placeholder=ph
                    autofocus=true
                />
            </div>
            <div class="py-1 max-h-72 overflow-y-auto">
                {children()}
            </div>
        </div>
    }
}

#[component]
pub fn CommandItem(
    #[prop(optional)] on_select: Option<Callback<()>>,
    #[prop(optional, into)] class: String,
    children: Children,
) -> impl IntoView {
    let final_class = format!(
        "w-full flex items-center gap-3 px-4 py-2.5 hover:bg-longan-800 cursor-pointer text-body text-rambutan-200 hover:text-rambutan-100 transition {class}"
    );
    view! {
        <button
            class=final_class
            on:click=move |_| { if let Some(cb) = on_select { cb.run(()); } }
        >
            {children()}
        </button>
    }
}

#[component]
pub fn CommandEmpty(#[prop(optional, into)] class: String, children: Children) -> impl IntoView {
    let final_class = format!(
        "px-4 py-3 text-label-sm text-rambutan-500 text-center {class}"
    );
    view! { <div class=final_class>{children()}</div> }
}
