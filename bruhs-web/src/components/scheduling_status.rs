use leptos::prelude::*;

use crate::state::SchedulingPreferencesSignal;

#[component]
pub fn SchedulingStatus() -> impl IntoView {
    let SchedulingPreferencesSignal(preferences) = expect_context();
    let recovery_open = RwSignal::new(false);
    view! {
        <Show when=move || preferences.loading()>
            <p role="status" class="px-4 py-3 text-body-sm text-rambutan-400">"Loading saved scheduling preferences…"</p>
        </Show>
        <Show when=move || preferences.busy() && !preferences.loading()>
            <p role="status" class="px-4 py-2 text-body-sm text-rambutan-400">"Saving scheduling preferences…"</p>
        </Show>
        <Show when=move || preferences.error().is_some()>
            <section aria-label="Scheduling preference recovery" class="shrink-0 border-t border-longan-700 bg-longan-900 px-4 py-3 text-body-sm text-rambutan-300">
                <h2 class="text-label text-lychee-300">"Scheduling preferences need attention"</h2>
                <p role="alert" class="mt-1">{move || preferences.error().map(|error| error.to_string())}</p>
                <button type="button" class="bruhs-control mt-2 rounded-grape bg-longan-800 px-3 py-2 cursor-pointer"
                    disabled=move || preferences.busy()
                    on:click=move |_| { wasm_bindgen_futures::spawn_local(async move {let _ = preferences.reload().await;}); }>
                    "Reload saved preferences"
                </button>
                <details class="mt-2" on:toggle=move |event| {
                    use wasm_bindgen::JsCast;
                    recovery_open.set(event.target().and_then(|target| target.dyn_into::<web_sys::Element>().ok()).is_some_and(|element| element.has_attribute("open")));
                }>
                    <summary class="cursor-pointer">"View saved preference data (private)"</summary>
                    <p class="my-2">"Keep a private copy before restoring browser data. IndexedDB holds the current policy; the unchanged legacy copy is labeled separately. Reload saved preferences, then repeat a failed change."</p>
                    <Show when=move || recovery_open.get()>
                        <textarea aria-label="Saved scheduling preference data" readonly class="h-28 w-full rounded-grape bg-longan-950 p-2" prop:value=move || preferences.recovery()></textarea>
                    </Show>
                </details>
            </section>
        </Show>
    }
}
