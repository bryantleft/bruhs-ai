//! Both app surfaces render the same narrowly scoped delivery workbench.
use leptos::prelude::*;

#[component]
pub fn DeliveryPanel(#[prop(optional, into)] initial_path: String) -> impl IntoView {
    let source = format!(
        "{}/api/delivery/ui?path={}",
        crate::api::BASE_URL,
        crate::api::segment(&initial_path)
    );
    view! { <iframe title="Code delivery" src=source class="h-full min-h-[36rem] w-full border-0" /> }
}
