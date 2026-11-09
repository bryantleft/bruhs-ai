use crate::storage::people::merge::Phase;
use crate::workflow_executor::WorkflowExecutor;
use leptos::prelude::*;

#[component]
pub fn ContactMergeStatus() -> impl IntoView {
    let executor = expect_context::<WorkflowExecutor>();
    let recovery = RwSignal::new(None::<String>);
    // Never serialize private observations or voice data on an ordinary editor update.
    Effect::new(move |_| {
        executor.merge_record.track();
        recovery.set(None);
    });
    view! {
        <Show when=move || executor.merge_record.with(Option::is_some) || executor.merge_error.with(Option::is_some)>
            <section aria-label="Contact merge recovery" class="space-y-2 rounded-grape bg-longan-900 p-3 inset-ring inset-ring-longan-800">
                <p role="status" class="text-label-sm text-rambutan-400">{move || executor.merge_record.with(|record| {
                    record.as_ref().map(|record| {
                        let status = match record.phase {
                            Phase::Prepared => "Merge intent saved locally; awaiting server confirmation",
                            Phase::Confirmed { .. } => "Server confirmed; local merge still pending",
                            Phase::Completed { .. } => "Contact merge saved locally",
                        };
                        format!("{status}: {} → {}.", record.source.display_name, record.target.display_name)
                    }).unwrap_or_default()
                })}</p>
                {move || executor.merge_error.get().map(|message| view! { <p role="alert" class="text-body-sm text-lychee-300">{message}</p> })}
                <Show when=move || executor.merge_record.with(|record| record.as_ref().is_some_and(|record| record.pending()))>
                    <p class="text-label-sm text-rambutan-400">"The two affected relationships are locked until recovery finishes. Other contacts remain editable."</p>
                    <button type="button" class="rounded-grape bg-longan-800 px-3 py-2 text-label-sm cursor-pointer disabled:opacity-40"
                        disabled=move || executor.merge_busy() on:click=move |_| executor.retry_contact_merge()>"Retry contact merge"</button>
                </Show>
                <Show when=move || executor.merge_error.with(Option::is_some)>
                    <button type="button" class="rounded-grape bg-longan-800 px-3 py-2 text-label-sm cursor-pointer disabled:opacity-40"
                        disabled=move || executor.merge_busy() on:click=move |_| executor.load_merge()>"Reload contact merge recovery"</button>
                </Show>
                <Show when=move || executor.merge_record.with(Option::is_some)>
                    <p class="text-label-sm text-rambutan-400">"Both originals are retained privately on this device. Recovery data may contain sensitive notes and voice information."</p>
                    <button type="button" class="rounded-grape bg-longan-800 px-3 py-2 text-label-sm cursor-pointer" on:click=move |_| {
                        let result = executor.merge_record.with_untracked(serde_json::to_string_pretty);
                        match result {
                            Ok(value) => recovery.set(Some(value)),
                            Err(_) => executor.merge_error.set(Some("Private merge recovery data could not be encoded.".into())),
                        }
                    }>"View contact merge recovery (private)"</button>
                    <Show when=move || recovery.with(Option::is_some)>
                        <textarea aria-label="Contact merge recovery data" readonly class="h-32 w-full rounded-grape bg-longan-950 p-2 text-body-sm" prop:value=move || recovery.get().unwrap_or_default()></textarea>
                    </Show>
                </Show>
            </section>
        </Show>
    }
}
