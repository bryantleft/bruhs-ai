use leptos::prelude::*;

/// A pure-CSS hover tooltip. Wraps its children and reveals `tip` text above
/// them on hover/focus.
///
/// Uses `group` + `group-hover` Tailwind utilities — works without JS positioning.
#[component]
pub fn Tooltip(
    /// Tooltip text to display.
    #[prop(into)]
    tip: String,
    #[prop(optional, into)] class: String,
    children: Children,
) -> impl IntoView {
    let pos_class = "left-1/2 -translate-x-1/2 bottom-full mb-1.5";

    let wrapper_class = format!("relative inline-flex group {class}");
    let bubble_class = format!(
        "pointer-events-none absolute {pos_class} z-50 whitespace-nowrap rounded-grape bg-longan-700 inset-ring inset-ring-white/15 px-2 py-1 text-label-sm text-rambutan-100 opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 transition-opacity"
    );

    view! {
        <span class=wrapper_class>
            {children()}
            <span class=bubble_class role="tooltip">
                {tip}
            </span>
        </span>
    }
}
