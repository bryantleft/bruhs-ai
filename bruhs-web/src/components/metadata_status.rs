use crate::durable_metadata::SaveState;
use leptos::prelude::*;

/// Routine loads and saves stay silent: nothing renders until a save fails.
/// A failure stays visible with a retry and a local recovery copy.
#[component]
pub fn MetadataStatus(
    status: RwSignal<SaveState>,
    retry: Callback<()>,
    recovery: Signal<String>,
) -> impl IntoView {
    let recovery_open = RwSignal::new(false);
    Effect::new(move |_| {
        if !matches!(status.get(), SaveState::Error { .. }) {
            recovery_open.set(false);
        }
    });
    view! {
        <Show when=move || matches!(status.get(), SaveState::Error { .. })>
            <div class="shrink-0 border-b border-longan-800/40 px-4 py-2 text-label-sm text-rambutan-400 sm:px-6">
                <p>"Not saved"</p>
                <p role="alert" class="mt-1 text-lychee-300">{move || match status.get() {
                    SaveState::Error { message, .. } => message,
                    _ => String::new(),
                }}</p>
                <button type="button" class="mt-2 min-h-11 rounded-grape bg-longan-800 px-3 py-2 text-rambutan-200 disabled:opacity-40 cursor-pointer sm:min-h-6"
                    disabled=move || matches!(status.get(), SaveState::Error { conflict: true, .. })
                    on:click=move |_| retry.run(())>"Retry save"</button>
                <details class="mt-2" on:toggle=move |event| {
                    use wasm_bindgen::JsCast;
                    let open = event.target().and_then(|target| target.dyn_into::<web_sys::Element>().ok()).is_some_and(|element| element.has_attribute("open"));
                    recovery_open.set(open);
                }>
                    <summary class="cursor-pointer">"View recovery data (private)"</summary>
                    <p class="my-2">"Copy this data somewhere private before closing or reloading. It may contain sensitive records."</p>
                    <Show when=move || recovery_open.get()>
                        <textarea aria-label="Unsaved recovery data" readonly class="h-32 w-full rounded-grape bg-longan-950 p-2 text-rambutan-300" prop:value=move || recovery.get()></textarea>
                    </Show>
                </details>
            </div>
        </Show>
    }
}
