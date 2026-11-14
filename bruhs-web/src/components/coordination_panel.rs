//! The task domain uses the same responsive view locally and on paired devices.
use leptos::prelude::*;

#[component]
pub fn CoordinationPanel() -> impl IntoView {
    let src = format!("{}/api/coordination/ui", crate::api::BASE_URL);
    view! {
        <iframe title="Task workspaces" src=src class="h-full min-h-[36rem] w-full border-0"
            allow="clipboard-write" />
    }
}
