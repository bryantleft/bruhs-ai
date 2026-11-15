//! Merge and delete confirmations. Nothing is sent until the user confirms.

use bruhs_models::people_copy;
use leptos::prelude::*;

use crate::state::PeopleSignal;
use crate::ui::{Dropdown, DropdownOption};

const PANEL: &str =
    "mt-1 space-y-2 rounded-seed bg-longan-800/80 p-2 inset-ring inset-ring-longan-700";
const QUIET: &str = "bruhs-control inline-flex items-center justify-center rounded-grape px-3 text-label-sm text-rambutan-400 hover:bg-longan-800 hover:text-rambutan-200 cursor-pointer";

/// Pick the surviving profile, then confirm the merge in words.
#[component]
pub fn MergePanel(
    source_id: String,
    source_name: Signal<String>,
    target: RwSignal<String>,
    confirming: RwSignal<bool>,
    on_confirm: Callback<String>,
    on_cancel: Callback<()>,
) -> impl IntoView {
    let PeopleSignal(people) = expect_context::<PeopleSignal>();
    let source_id = StoredValue::new(source_id);
    let target_name = move || {
        let selected = target.get();
        people.with(|people| {
            people
                .iter()
                .find(|person| person.id == selected)
                .map(|person| person.display_name.clone())
        })
    };
    view! {
        <section aria-label="Merge profile" class=PANEL>
            <Show
                when=move || confirming.get() && target_name().is_some()
                fallback=move || view! {
                    <div class="text-label-sm text-rambutan-400">"Merge into:"</div>
                    <Dropdown
                        options=Signal::derive(move || {
                            people.with(|people| {
                                people
                                    .iter()
                                    .filter(|person| person.id != source_id.get_value())
                                    .map(|person| DropdownOption::new(person.id.clone(), person.display_name.clone()))
                                    .collect::<Vec<_>>()
                            })
                        })
                        value=Signal::derive(move || target.get())
                        on_change=Callback::new(move |value: String| target.set(value))
                        placeholder="Pick a profile"
                        label="Merge into"
                        class="w-full"
                    />
                    <div class="flex justify-end gap-2">
                        <button type="button" class=QUIET on:click=move |_| on_cancel.run(())>{people_copy::CANCEL}</button>
                        <button
                            type="button"
                            class="bruhs-control inline-flex items-center justify-center rounded-grape px-3 text-label-sm text-persimmon-400 hover:bg-longan-800 hover:text-persimmon-200 disabled:opacity-40 cursor-pointer"
                            prop:disabled=move || target.get().is_empty()
                            on:click=move |_| confirming.set(true)
                        >"Merge"</button>
                    </div>
                }
            >
                <p role="alert" class="break-words text-body-sm text-rambutan-200">
                    {move || people_copy::merge_confirm(&source_name.get(), &target_name().unwrap_or_default())}
                </p>
                <div class="flex flex-wrap justify-end gap-2">
                    <button type="button" class=QUIET on:click=move |_| confirming.set(false)>{people_copy::CANCEL}</button>
                    <button
                        type="button"
                        class="bruhs-control inline-flex items-center justify-center rounded-grape bg-persimmon-500 px-3 text-label-sm text-longan-950 hover:bg-persimmon-400 cursor-pointer"
                        on:click=move |_| on_confirm.run(target.get_untracked())
                    >{people_copy::MERGE_CONFIRM_BUTTON}</button>
                </div>
            </Show>
        </section>
    }
}

/// Confirm a delete in words, with the cascade spelled out.
#[component]
pub fn DeletePanel(
    name: Signal<String>,
    busy: Signal<bool>,
    on_confirm: Callback<()>,
    on_cancel: Callback<()>,
) -> impl IntoView {
    view! {
        <section aria-label="Delete profile" class=PANEL>
            <p role="alert" class="break-words text-body-sm text-rambutan-200">
                {move || people_copy::delete_confirm(&name.get())}
            </p>
            <div class="flex flex-wrap justify-end gap-2">
                <button type="button" class=QUIET on:click=move |_| on_cancel.run(())>{people_copy::CANCEL}</button>
                <button
                    type="button"
                    class="bruhs-control inline-flex items-center justify-center rounded-grape bg-lychee-500/20 px-3 text-label-sm text-lychee-200 inset-ring inset-ring-lychee-500/40 hover:bg-lychee-500/30 disabled:opacity-40 cursor-pointer"
                    disabled=move || busy.get()
                    on:click=move |_| on_confirm.run(())
                >{people_copy::DELETE_BUTTON}</button>
            </div>
        </section>
    }
}
