use leptos::prelude::*;

/// Display a keyboard shortcut, e.g. `<Kbd>"⌘K"</Kbd>`.
#[component]
pub fn Kbd(#[prop(optional, into)] class: String, children: Children) -> impl IntoView {
    let final_class = format!(
        "inline-flex items-center justify-center min-w-[1.25rem] px-1.5 py-0.5 rounded-seed bg-longan-800 inset-ring inset-ring-white/10 text-code-sm text-rambutan-300 {class}"
    );
    view! { <kbd class=final_class>{children()}</kbd> }
}
