use bruhs_models::WorkflowOperation;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

#[component]
pub fn WorkflowReview(workflow_id: String) -> impl IntoView {
    let crate::state::AutomationStateSignal(automation) = expect_context();
    let workflow_id = StoredValue::new(workflow_id);
    let opened = RwSignal::new(false);
    view! {
        <details class="w-full min-w-0" on:toggle=move |event: web_sys::Event| {
            opened.set(event.target().and_then(|target| target.dyn_into::<web_sys::HtmlDetailsElement>().ok()).is_some_and(|details| details.open()));
        }>
            <summary class="flex min-h-11 items-center text-label-sm text-rambutan-400 cursor-pointer">"Review workflow"</summary>
            <Show when=move || opened.get()>
                {move || automation.with(|state| workflow_id.with_value(|id| {
                    state.workflows.iter().find(|workflow| &workflow.id == id).map(|workflow| {
                        let operations = workflow.steps.iter().map(|step| step.operation.clone()).collect();
                        view! { <WorkflowPayload operations=operations /> }
                    })
                }))}
            </Show>
        </details>
    }
}

#[component]
pub fn WorkflowPayload(operations: Vec<WorkflowOperation>) -> impl IntoView {
    view! {
        <section aria-label="Workflow payload" class="min-w-0">
            <p class="mt-2 text-label-sm text-rambutan-400">"Full payload · stored locally. Review every field before starting."</p>
            <ol class="mt-3 space-y-2">
                {operations.into_iter().enumerate().map(|(index, operation)| {
                    let fields = operation.review_fields(|millis| {
                        let iso = js_sys::Date::new_0();
                        iso.set_time(millis);
                        if iso.get_time().is_finite() {
                            format!("{} ({millis} ms since Unix epoch)", String::from(iso.to_iso_string()))
                        } else {
                            format!("Invalid timestamp ({millis})")
                        }
                    });
                    view! {
                        <li class="min-w-0 rounded-grape bg-longan-950/60 px-3 py-2">
                            <h3 class="text-label text-rambutan-200">{format!("{} · {}", index + 1, operation.domain().label())}</h3>
                            <dl class="mt-2 space-y-2">
                                {fields.into_iter().map(|(label, value)| view! {
                                    <div class="min-w-0">
                                        <dt class="text-label-sm text-rambutan-500">{label}</dt>
                                        <dd class="m-0 whitespace-pre-wrap break-words text-body-sm text-rambutan-300" style="overflow-wrap:anywhere">{value}</dd>
                                    </div>
                                }).collect_view()}
                            </dl>
                        </li>
                    }
                }).collect_view()}
            </ol>
        </section>
    }
}
