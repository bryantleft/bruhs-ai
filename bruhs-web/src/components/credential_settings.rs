//! Metadata listing and write-only editing. Stored values never reach the browser.
use bruhs_models::secrets::{review_text, SavedCredential, SavedCredentialState};
use leptos::{html::Input, prelude::*};

#[derive(Clone)]
enum LoadState {
    Loading,
    Ready,
    Failed(String),
}

#[component]
pub fn CredentialSettings() -> impl IntoView {
    let rows = RwSignal::new(Vec::<SavedCredential>::new());
    let load = RwSignal::new(LoadState::Loading);
    let generation = RwSignal::new(0u64);
    let busy = RwSignal::new(false);
    let query = RwSignal::new(String::new());
    let refresh = Callback::new(move |_: ()| {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        load.set(LoadState::Loading);
        wasm_bindgen_futures::spawn_local(async move {
            match crate::api::saved_credentials().await {
                Ok(entries) => {
                    rows.try_set(entries);
                    generation.try_update(|g| *g = g.wrapping_add(1));
                    load.try_set(LoadState::Ready);
                }
                Err(error) => {
                    load.try_set(LoadState::Failed(error.to_string()));
                }
            }
            busy.try_set(false);
        });
    });
    Effect::new(move |_| refresh.run(()));
    let button = "min-h-11 rounded-seed px-3 text-label-sm inset-ring inset-ring-longan-700 hover:bg-longan-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400 disabled:opacity-50 sm:min-h-10";
    view! {
        <div class="p-4 text-body-sm text-rambutan-200">
            <super::service_connections::ServiceConnections />
            <div class="flex flex-col gap-2 sm:flex-row sm:items-center">
                <input type="search" aria-label="Search saved credentials" placeholder="Search by name or project" class="min-h-10 w-full min-w-0 rounded-seed bg-longan-950 px-3 inset-ring inset-ring-longan-700 focus:outline focus:outline-persimmon-400 sm:flex-1" on:input=move |event| query.set(event_target_value(&event)) />
                <button type="button" class=button disabled=move || busy.get() on:click=move |_| refresh.run(())>"Refresh credentials"</button>
            </div>
            <p class="mt-2 text-label-sm text-rambutan-400">"Values are never displayed. Replace a key to rotate it. Deleting removes the saved copy; running commands may finish with the value they already received."</p>
            <p class="mt-2 text-label-sm text-rambutan-400">"Older saved credentials appear here after their next thread request."</p>
            {move || match load.get() {
                LoadState::Loading => view! { <p role="status" class="mt-3 text-label-sm text-rambutan-400">"Loading saved credentials…"</p> }.into_any(),
                LoadState::Failed(message) => view! { <p role="alert" class="mt-3 text-label-sm text-lychee-300">{message}" Refresh to retry."</p> }.into_any(),
                LoadState::Ready if rows.with(|rows| rows.is_empty()) => view! { <p role="status" class="mt-3 text-label-sm text-rambutan-400">"No saved credentials. Choose Save securely and use in a thread request to add one."</p> }.into_any(),
                LoadState::Ready => ().into_any(),
            }}
            <div class="mt-3 divide-y divide-longan-800">
                <For
                    each=move || { let search = query.get().to_lowercase(); rows.get().into_iter().filter(|row| row.name.to_lowercase().contains(&search) || row.project_path.to_lowercase().contains(&search)).map(|row| (generation.get(), row)).collect::<Vec<_>>() }
                    key=|(generation, row)| (*generation, row.id.clone())
                    children=move |(_, row)| view! { <CredentialRow row=row rows=rows busy=busy /> }
                />
                {move || (!rows.with(|rows| rows.is_empty()) && !query.get().is_empty() && rows.with(|rows| !rows.iter().any(|row| row.name.to_lowercase().contains(&query.get().to_lowercase()) || row.project_path.to_lowercase().contains(&query.get().to_lowercase())))).then(|| view! { <p role="status" class="py-3 text-label-sm text-rambutan-400">"No credentials match your search."</p> })}
            </div>
        </div>
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Edit {
    None,
    Replace,
    Delete,
}

#[component]
fn CredentialRow(
    row: SavedCredential,
    rows: RwSignal<Vec<SavedCredential>>,
    busy: RwSignal<bool>,
) -> impl IntoView {
    let id = StoredValue::new(row.id.clone());
    let mode = RwSignal::new(Edit::None);
    let error = RwSignal::new(None::<String>);
    let notice = RwSignal::new(None::<String>);
    let input = NodeRef::<Input>::new();
    on_cleanup(move || {
        if let Some(input) = input.get_untracked() {
            input.set_value("");
        }
    });
    let reset = Callback::new(move |_: ()| {
        if let Some(input) = input.get_untracked() {
            input.set_value("");
        }
        mode.set(Edit::None);
        error.set(None);
    });
    let mutate = Callback::new(move |delete: bool| {
        if busy.get_untracked() {
            return;
        }
        let value = input
            .get_untracked()
            .map(|input| input.value())
            .unwrap_or_default();
        if !delete && value.is_empty() {
            error.set(Some("Enter a replacement credential.".into()));
            return;
        }
        if let Some(input) = input.get_untracked() {
            input.set_value("");
        }
        busy.set(true);
        error.set(None);
        notice.set(None);
        let id = id.get_value();
        wasm_bindgen_futures::spawn_local(async move {
            let result = if delete {
                crate::api::delete_credential(&id).await
            } else {
                crate::api::replace_credential(&id, value).await
            };
            match result {
                Ok(()) => {
                    if delete {
                        rows.try_update(|rows| rows.retain(|row| row.id != id));
                    } else {
                        rows.try_update(|rows| {
                            if let Some(row) = rows.iter_mut().find(|row| row.id == id) {
                                row.state = SavedCredentialState::Saved;
                            }
                        });
                        notice.try_set(Some("Credential updated.".into()));
                    }
                    mode.try_set(Edit::None);
                }
                Err(err) if err.is_status(404) => {
                    rows.try_update(|rows| rows.retain(|row| row.id != id));
                }
                Err(err) => {
                    error.try_set(Some(format!("{err} Re-enter the value or retry deletion.")));
                }
            }
            busy.try_set(false);
        });
    });
    let button = "min-h-11 rounded-seed px-3 text-label-sm inset-ring inset-ring-longan-700 hover:bg-longan-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400 disabled:opacity-50 sm:min-h-10";
    view! {
        <article aria-label=format!("Saved credential: {}", review_text(&row.name)) class="min-w-0 py-3">
            <div class="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
                <div class="min-w-0 flex-1">
                    <h3 class="break-words text-label text-rambutan-100">{review_text(&row.name)}</h3>
                    <p class="mt-1 break-all text-label-sm text-rambutan-400">{review_text(&row.project_path)}</p>
                    <p class="mt-1 text-label-sm text-rambutan-400">{move || if rows.with(|rows| rows.iter().any(|row| row.id == id.get_value() && row.state == SavedCredentialState::NeedsAttention)) { "Needs attention · replace or delete to finish a previous change." } else { "Stored in OS keychain · value hidden" }}</p>
                </div>
                <div class="flex flex-wrap gap-2">
                    <button type="button" class=button disabled=move || busy.get() on:click=move |_| { reset.run(()); notice.set(None); mode.set(Edit::Replace); }>"Replace"</button>
                    <button type="button" class=button disabled=move || busy.get() on:click=move |_| { reset.run(()); notice.set(None); mode.set(Edit::Delete); }>"Delete"</button>
                </div>
            </div>
            <Show when=move || mode.get() == Edit::Replace>
                <form class="mt-3 flex flex-wrap items-end gap-2" on:submit=move |event| { event.prevent_default(); mutate.run(false); }>
                    <label class="w-full min-w-0 text-label-sm sm:w-auto sm:flex-1">"New credential value"
                        <input node_ref=input type="password" autocomplete="off" autocapitalize="none" spellcheck="false" disabled=move || busy.get() on:keydown=move |event| event.stop_propagation() class="mt-1 block min-h-10 w-full min-w-0 rounded-seed bg-longan-950 px-3 text-body-sm inset-ring inset-ring-longan-700 focus:outline focus:outline-persimmon-400" />
                    </label>
                    <button type="submit" class=button disabled=move || busy.get()>"Save replacement"</button>
                    <button type="button" class=button disabled=move || busy.get() on:click=move |_| reset.run(())>"Cancel"</button>
                </form>
            </Show>
            <Show when=move || mode.get() == Edit::Delete>
                <div class="mt-3 flex flex-wrap items-center gap-2">
                    <p class="w-full text-label-sm">"Delete this saved credential from the OS keychain?"</p>
                    <button type="button" class=button disabled=move || busy.get() on:click=move |_| mutate.run(true)>"Delete credential"</button>
                    <button type="button" class=button disabled=move || busy.get() on:click=move |_| reset.run(())>"Cancel"</button>
                </div>
            </Show>
            {move || error.get().map(|message| view! { <p role="alert" class="mt-2 text-label-sm text-lychee-300">{message}</p> })}
            {move || notice.get().map(|message| view! { <p role="status" class="mt-2 text-label-sm text-pandan-300">{message}</p> })}
        </article>
    }
}
