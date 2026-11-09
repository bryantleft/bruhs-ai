//! The header every full page opens with: a title, an optional one-line
//! subtitle, and the page's actions on the right, on a bar that spans the
//! page above its scrolling content (Today's layout).

use leptos::prelude::*;

#[component]
pub fn PageHeader(
    /// Page name, the page's `h1`.
    #[prop(into)]
    title: String,
    /// One short line under the title (static or reactive).
    #[prop(optional, into)]
    subtitle: MaybeProp<String>,
    /// `id` for the `h1`, so a section can be `aria-labelledby` it.
    #[prop(optional)]
    title_id: Option<&'static str>,
    /// Actions, right-aligned.
    #[prop(optional)]
    children: Option<Children>,
) -> impl IntoView {
    view! {
        <header class="flex shrink-0 flex-wrap items-center justify-between gap-3 border-b border-longan-800/50 px-4 py-3 sm:px-6">
            <div class="min-w-0">
                <h1 id=title_id class="text-h3 text-rambutan-100">{title}</h1>
                {move || subtitle.get().map(|subtitle| view! {
                    <p class="mt-0.5 text-body-sm text-rambutan-500">{subtitle}</p>
                })}
            </div>
            {children.map(|actions| view! {
                <div class="flex flex-wrap items-center gap-2">{actions()}</div>
            })}
        </header>
    }
}
