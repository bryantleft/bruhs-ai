use super::*;

#[component]
pub(super) fn ExportResponse(
    markdown: Signal<String>,
    project_path: Signal<Option<String>>,
    scope: String,
    streaming: Signal<bool>,
) -> impl IntoView {
    let include_remote = RwSignal::new(false);
    let include_state = RwSignal::new(false);
    let open = RwSignal::new(false);
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let scope = StoredValue::new(scope);
    view! {
        <div class="mt-1 text-label-sm text-rambutan-500" data-response-export="">
            <button type="button" disabled=move || streaming.get() class=button_class(ButtonVariant::Ghost, ButtonSize::Small, "") aria-expanded=move || open.get().to_string() on:click=move |_| open.update(|open| *open = !*open)>"Export reply"</button>
            <Show when=move || open.get()>
                <div class="mt-1 flex flex-col gap-2 rounded-grape bg-longan-900/60 p-3 text-rambutan-300 inset-ring inset-ring-longan-800">
                    <label class="flex min-h-[28px] items-center gap-2"><input type="checkbox" prop:checked=move || include_remote.get() on:change=move |event| include_remote.set(event_target_checked(&event)) />"Include external media"</label>
                    <p class="text-rambutan-500">{move || if include_remote.get() { "Downloads linked media for this bundle." } else { "External media stays as links. Local media and interactive content are included." }}</p>
                    <label class="flex min-h-[28px] items-center gap-2"><input type="checkbox" prop:checked=move || include_state.get() on:change=move |event| include_state.set(event_target_checked(&event)) />"Include saved widget values"</label>
                    <p class="text-rambutan-500">"Saved values can include text entered in widget forms."</p>
                    <button type="button" disabled=move || busy.get() class=button_class(ButtonVariant::Outline, ButtonSize::Small, "self-start") on:click=move |_| {
                        busy.set(true); error.set(None);
                        let markdown=markdown.get_untracked(); let project=project_path.get_untracked(); let scope=scope.get_value();
                        let remote=include_remote.get_untracked(); let state=include_state.get_untracked();
                        leptos::task::spawn_local(async move {
                            if let Err(reason)=client::export(&markdown,project.as_deref(),&scope,remote,state).await { let _=error.try_set(Some(reason)); }
                            let _=busy.try_set(false);
                        });
                    }>{move || if busy.get() { "Preparing bundle…" } else { "Download ZIP" }}</button>
                    <Show when=move || error.get().is_some()><p class="text-lychee-300" role="status">{move || error.get()}</p></Show>
                </div>
            </Show>
        </div>
    }
}
