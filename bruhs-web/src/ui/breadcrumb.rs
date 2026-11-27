//! Breadcrumb navigation row with consistent slash separators matching `bruhs_ui::breadcrumb`.

use leptos::prelude::*;

/// Horizontal breadcrumb trail with `aria-label="Breadcrumb"`.
#[component]
pub fn Breadcrumb(
    #[prop(optional, into)] aria_label: Option<String>,
    #[prop(optional, into)] class: String,
    children: Children,
) -> impl IntoView {
    let label = aria_label.unwrap_or_else(|| "Breadcrumb".to_string());
    let classes = format!("flex min-w-0 items-center gap-2 {class}");
    view! {
        <nav aria-label=label class=classes>
            {children()}
        </nav>
    }
}

/// Decorative slash separator between breadcrumb segments.
#[component]
pub fn BreadcrumbSep() -> impl IntoView {
    view! {
        <span class="shrink-0 text-label-sm text-rambutan-500" aria-hidden="true">"/"</span>
    }
}
