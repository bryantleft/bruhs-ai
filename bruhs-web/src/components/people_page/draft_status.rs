use bruhs_models::people_copy;
use leptos::prelude::*;

use crate::relationship_store::RelationshipStore;
use crate::state::{PeopleSignal, SelectedPersonSignal};

#[component]
pub(super) fn RelationshipDraftStatus() -> impl IntoView {
    let store = expect_context::<RelationshipStore>();
    let PeopleSignal(people) = expect_context();
    let SelectedPersonSignal(selected) = expect_context();
    let show_private = RwSignal::new(false);
    Effect::new(move |_| {
        if store.editor_draft_count() == 0 {
            show_private.set(false);
        }
    });
    view! {
        <Show when=move || { store.editor_draft_count() > 0 }>
            <section aria-label=people_copy::DRAFTS_LABEL class="space-y-2 rounded-grape bg-longan-900 p-3 inset-ring inset-ring-longan-800">
                <p role="status" class="text-body-sm text-blueberry-300">
                    {move || people_copy::drafts_status(store.editor_draft_count())}
                </p>
                <p class="text-label-sm text-rambutan-400">{people_copy::DRAFTS_KEPT_TAB}</p>
                <ul class="space-y-1">
                    <For each=move || store.editor_draft_ids() key=|id| id.clone() let:id>
                        {
                            let id = StoredValue::new(id);
                            let name = Signal::derive(move || people.with(|people| {
                                people.iter().find(|person| person.id == id.get_value()).map_or_else(
                                    || people_copy::draft_unavailable_contact(&id.get_value()),
                                    |person| person.display_name.clone(),
                                )
                            }));
                            let exists = move || people.with(|people| people.iter().any(|person| person.id == id.get_value()));
                            view! {
                                <li class="flex flex-wrap items-center gap-2 text-label-sm">
                                    <span class="min-w-0 w-full break-words text-rambutan-300 sm:w-auto sm:flex-1">{move || name.get()}</span>
                                    <button type="button" disabled=move || !exists() aria-label=move || people_copy::draft_open_label(&name.get()) class="bruhs-control min-h-control-xs rounded-grape bg-longan-800 px-3 py-2 disabled:opacity-40 cursor-pointer" on:click=move |_| selected.set(Some(id.get_value()))>{people_copy::DRAFT_OPEN}</button>
                                    <button type="button" aria-label=move || people_copy::draft_discard_label(&name.get()) class="bruhs-control min-h-control-xs rounded-grape px-3 py-2 text-lychee-300 hover:bg-longan-800 cursor-pointer" on:click=move |_| store.discard_editor_draft(&id.get_value())>{people_copy::DRAFT_DISCARD}</button>
                                </li>
                            }
                        }
                    </For>
                </ul>
                <Show when=move || { store.editor_draft_count() > bruhs_models::DRAFT_LIST_LIMIT }>
                    <p class="text-label-sm text-rambutan-500">{people_copy::DRAFTS_TRUNCATED}</p>
                </Show>
                <button type="button" aria-expanded=move || show_private.get().to_string() class="bruhs-control min-h-control-xs rounded-seed px-2 py-1 text-label-sm text-rambutan-400 hover:text-rambutan-200 cursor-pointer" on:click=move |_| show_private.update(|show| *show = !*show)>
                    {move || if show_private.get() { people_copy::DRAFTS_HIDE } else { people_copy::DRAFTS_VIEW }}
                </button>
                <Show when=move || show_private.get()>
                    <p class="text-label-sm text-rambutan-400">{people_copy::DRAFTS_COPY_NOTE}</p>
                    <textarea aria-label=people_copy::DRAFTS_RECOVERY_LABEL readonly rows=8 class="block w-full min-w-0 resize-y rounded-grape bg-longan-950 p-2 text-label-sm text-rambutan-300" prop:value=move || store.editor_recovery_json() />
                </Show>
            </section>
        </Show>
    }
}
