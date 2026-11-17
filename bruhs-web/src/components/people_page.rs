//! People page: create, search and page through profiles. Expand a card for
//! its aliases, linked meetings and conversations, relationship context and
//! observations grouped by kind. Rename, tracking, merge and delete each
//! offer undo right after, while the touched records are unchanged.

use leptos::prelude::*;
use std::cmp::Reverse;
use std::collections::BTreeMap;

use bruhs_models::{people_copy, people_page_count, people_page_of, PersonProfile};

use crate::api;
use crate::components::metadata_status::MetadataStatus;
use crate::components::voice_enroll::VoiceEnrollModal;
use crate::relationship_store::RelationshipStore;
use crate::state::{
    AppView, MeetingsSignal, ObservationKind, PeopleSignal, PersonObservation,
    SelectedPersonSignal, SetActiveView, SetMeetingsSignal, SetPeopleSignal,
};
mod back;
mod confirm;
mod directory;
mod draft_status;
mod merge_status;
mod profile;
mod relationship;
pub use back::BackToPeople;
use directory::Directory;

/// Which card has which inline editor or confirmation open. One of each at a
/// time across the page.
#[derive(Clone, Copy)]
struct CardState {
    renaming: RwSignal<Option<String>>,
    rename_value: RwSignal<String>,
    merge_open: RwSignal<Option<String>>,
    merge_target: RwSignal<String>,
    merge_confirming: RwSignal<bool>,
    delete_open: RwSignal<Option<String>>,
    enroll_open: RwSignal<Option<String>>,
}

#[component]
pub fn PeoplePage() -> impl IntoView {
    let saves = expect_context::<crate::task_saves::TaskSaves>();
    let relationship_store = expect_context::<RelationshipStore>();
    let workflow_executor = expect_context::<crate::workflow_executor::WorkflowExecutor>();
    let PeopleSignal(people) = expect_context::<PeopleSignal>();
    let SetPeopleSignal(set_people) = expect_context::<SetPeopleSignal>();
    let SelectedPersonSignal(expanded) = expect_context::<SelectedPersonSignal>();
    let directory =
        Memo::new(move |previous| people.with(|people| Directory::new(people, previous)));
    let query = RwSignal::new(String::new());
    let matching = Memo::new(move |_| directory.with(|directory| directory.matching(&query.get())));
    let page = RwSignal::new(0usize);
    let page_count = Memo::new(move |_| matching.with(|ids| people_page_count(ids.len())));
    Effect::new(move |_| {
        if page.get() >= page_count.get() {
            page.set(page_count.get() - 1);
        }
    });
    let last_focused = StoredValue::new(Option::<String>::None);
    Effect::new(move |_| {
        let selected = expanded.get();
        if last_focused.get_value() == selected {
            return;
        }
        let Some(id) = selected else {
            last_focused.set_value(None);
            return;
        };
        if people.with(|profiles| profiles.iter().any(|profile| profile.id == id)) {
            // A profile opened from elsewhere is shown even when a search hides it.
            if !matching.with_untracked(|ids| ids.contains(&id)) {
                query.set(String::new());
            }
            if let Some(index) = directory.with(|directory| {
                directory
                    .order
                    .iter()
                    .position(|candidate| candidate == &id)
            }) {
                page.set(people_page_of(index));
            }
            last_focused.set_value(Some(id.clone()));
            super::record_focus::focus_record(format!("person-record-{id}"));
        }
    });
    let cards = CardState {
        renaming: RwSignal::new(None),
        rename_value: RwSignal::new(String::new()),
        merge_open: RwSignal::new(None),
        merge_target: RwSignal::new(String::new()),
        merge_confirming: RwSignal::new(false),
        delete_open: RwSignal::new(None),
        enroll_open: RwSignal::new(None),
    };
    // Observation provenance needs the meeting list, which is otherwise only
    // hydrated by the Meetings page. Load it the same way (server, then the
    // local cache) so a cold People visit never claims a stored meeting is gone.
    let MeetingsSignal(meetings) = expect_context::<MeetingsSignal>();
    let SetMeetingsSignal(set_meetings) = expect_context::<SetMeetingsSignal>();
    let meeting_sources_ready = RwSignal::new(meetings.with_untracked(|list| !list.is_empty()));
    if !meeting_sources_ready.get_untracked() {
        wasm_bindgen_futures::spawn_local(async move {
            match api::fetch_meetings().await {
                Ok(fetched) if !fetched.is_empty() => set_meetings.set(fetched),
                _ => {
                    if let Ok(local) = crate::storage::load_meetings().await {
                        if !local.is_empty() {
                            set_meetings.set(local);
                        }
                    }
                }
            }
            // The page may have closed while loading.
            meeting_sources_ready.try_set(true);
        });
    }
    provide_context(MeetingSourcesReady(meeting_sources_ready.read_only()));

    // Create keeps the typed name until a profile is actually created.
    let new_name = RwSignal::new(String::new());
    let create_error = RwSignal::new(Option::<&'static str>::None);
    let create = move || {
        let name = new_name.get_untracked().trim().to_string();
        if name.is_empty() {
            create_error.set(Some(people_copy::CREATE_BLANK));
            return;
        }
        create_error.set(None);
        let now = crate::clock::now_ms();
        let mut person = PersonProfile::new(name);
        person.created_at = now;
        person.updated_at = now;
        let id = person.id.clone();
        set_people.update(|people| people.push(person.clone()));
        saves.create_person(person);
        new_name.set(String::new());
        expanded.set(Some(id));
    };

    view! {
        <div class="flex h-full flex-col bg-longan-950">
            <crate::ui::PageHeader title="People" subtitle=people_copy::PAGE_SUBTITLE />
            <div aria-label="Relationship save status">
                <MetadataStatus status=relationship_store.status
                    retry=Callback::new(move |_| relationship_store.retry())
                    recovery=Signal::derive(move || relationship_store.recovery_json()) />
            </div>
            <div class="min-h-0 flex-1 space-y-3 overflow-y-auto px-4 py-4 sm:px-6">
            <merge_status::ContactMergeStatus />
            {move || workflow_executor.people_notice.get().map(|message| view! {
                <p role="alert" aria-label="Profile change failed" class="text-body-sm text-lychee-300">{message}</p>
            })}
            {move || workflow_executor.people_undo.get().map(|undo| {
                let label = undo.action_label();
                view! {
                    <div aria-label="Undo People change" data-people-undo="" class="flex flex-wrap items-center justify-between gap-2 rounded-grape bg-longan-900 px-3 py-2 inset-ring inset-ring-longan-800">
                        <p role="status" class="min-w-0 flex-1 break-words text-body-sm text-rambutan-300">{undo.summary()}</p>
                        <div class="flex shrink-0 gap-2">
                            <button type="button"
                                class="bruhs-control inline-flex items-center justify-center rounded-grape px-3 text-label-sm text-persimmon-300 hover:bg-longan-800 disabled:opacity-40 cursor-pointer"
                                disabled=move || workflow_executor.people_busy()
                                on:click=move |_| workflow_executor.undo_people()>{label}</button>
                            <button type="button" aria-label="Dismiss undo"
                                class="bruhs-control inline-flex items-center justify-center rounded-grape px-3 text-label-sm text-rambutan-500 hover:bg-longan-800 hover:text-rambutan-300 cursor-pointer"
                                on:click=move |_| workflow_executor.people_undo.set(None)>"Dismiss"</button>
                        </div>
                    </div>
                }
            })}
            <Show when=move || workflow_executor.loading_people()>
                <p role="status" aria-label="Local contact loading" class="text-label-sm text-rambutan-400">"Loading cached contacts…"</p>
            </Show>
            <Show when=move || workflow_executor.refreshing_people()>
                <p role="status" aria-label="Contact directory sync" class="text-label-sm text-rambutan-400">"Refreshing contacts… Cached contacts remain available."</p>
            </Show>
            <Show when=move || saves.contacts_saving()>
                <p role="status" class="text-label-sm text-rambutan-400">"Saving captured contacts locally… Keep this tab open."</p>
            </Show>
            {move || workflow_executor.people_cache_error.get().or_else(|| workflow_executor.people_refresh_error.get()).map(|message| view! {
                <section aria-label="Contact storage recovery" class="space-y-2 rounded-grape bg-longan-900 p-3 inset-ring inset-ring-longan-800">
                    <p role="alert" class="text-body-sm text-lychee-300">{message}</p>
                    <button type="button" class="rounded-grape bg-longan-800 px-3 py-2 text-label-sm cursor-pointer" on:click=move |_| workflow_executor.retry_people()>"Retry loading contacts"</button>
                </section>
            })}
            {move || workflow_executor.contact_sync_error.get().map(|message| view! {
                <div class="text-label-sm text-rambutan-400"><p role="status">{message}</p>
                    <button type="button" class="mt-1 rounded-grape bg-longan-800 px-3 py-2 cursor-pointer" on:click=move |_| workflow_executor.retry_contact_sync()>"Retry contact sync"</button>
                </div>
            })}
            <draft_status::RelationshipDraftStatus />
            <div class="grid gap-3 sm:grid-cols-2">
                <form aria-label="Create person" class="min-w-0 space-y-1" on:submit=move |event| { event.prevent_default(); create(); }>
                    <label for="people-create" class="block text-label-sm text-rambutan-500">{people_copy::CREATE_LABEL}</label>
                    <div class="flex gap-2">
                        <input id="people-create" type="text" autocomplete="off"
                            class="bruhs-control min-w-0 flex-1 rounded-grape bg-white/5 px-3 text-body-sm text-rambutan-100 placeholder:text-rambutan-500 inset-ring inset-ring-white/10 focus-visible:outline-2 focus-visible:outline-persimmon-400"
                            placeholder=people_copy::CREATE_PLACEHOLDER
                            aria-invalid=move || create_error.get().is_some().to_string()
                            aria-describedby="people-create-error"
                            prop:value=move || new_name.get()
                            on:input=move |event| {
                                new_name.set(event_target_value(&event));
                                create_error.set(None);
                            } />
                        <button type="submit" class="bruhs-control inline-flex shrink-0 items-center justify-center rounded-grape bg-persimmon-500 px-3 text-label-sm text-longan-950 hover:bg-persimmon-400 cursor-pointer">
                            {people_copy::CREATE_BUTTON}
                        </button>
                    </div>
                    <p id="people-create-error" role="alert" class="text-body-sm text-lychee-300 empty:hidden">{move || create_error.get().unwrap_or_default()}</p>
                </form>
                <div role="search" class="min-w-0 space-y-1">
                    <label for="people-search" class="block text-label-sm text-rambutan-500">{people_copy::SEARCH_LABEL}</label>
                    <input id="people-search" type="search" autocomplete="off"
                        class="bruhs-control w-full rounded-grape bg-white/5 px-3 text-body-sm text-rambutan-100 placeholder:text-rambutan-500 inset-ring inset-ring-white/10 focus-visible:outline-2 focus-visible:outline-persimmon-400"
                        placeholder=people_copy::SEARCH_PLACEHOLDER
                        prop:value=move || query.get()
                        on:input=move |event| {
                            query.set(event_target_value(&event));
                            page.set(0);
                        } />
                </div>
            </div>
            <Show
                when=move || people.with(|people| !people.is_empty())
                fallback=move || view! {
                    <div class="text-label text-rambutan-500" role="status">
                        {move || if workflow_executor.refreshing_people() || !workflow_executor.people_hydrated.get() {
                            people_copy::LOADING
                        } else {
                            people_copy::EMPTY
                        }}
                    </div>
                }
            >
                <Show when=move || matching.with(|ids| ids.is_empty())>
                    <p role="status" class="text-label text-rambutan-500">{people_copy::SEARCH_EMPTY}</p>
                </Show>
                <ul role="list" class="space-y-2">
                    <For
                        each=move || matching.with(|ids| bruhs_models::people_page(ids, page.get()).to_vec())
                        key=|id| id.clone()
                        let:person_id
                    >
                        <PersonCard person_id=person_id directory=directory cards=cards />
                    </For>
                </ul>
                <Show when=move || { page_count.get() > 1 }>
                    <nav aria-label="People pages" class="flex items-center justify-between gap-2 py-2 text-label-sm">
                        <button type="button" disabled=move || page.get() == 0 class="bruhs-control rounded-grape bg-longan-800 px-3 py-2 disabled:opacity-40 cursor-pointer" on:click=move |_| page.update(|page| *page = page.saturating_sub(1))>{people_copy::PREVIOUS_PAGE}</button>
                        <span role="status">{move || people_copy::page_status(page.get(), page_count.get())}</span>
                        <button type="button" disabled=move || { page.get() + 1 >= page_count.get() } class="bruhs-control rounded-grape bg-longan-800 px-3 py-2 disabled:opacity-40 cursor-pointer" on:click=move |_| page.update(|page| *page += 1)>{people_copy::NEXT_PAGE}</button>
                    </nav>
                </Show>
            </Show>
            </div>
        </div>
    }
}

#[component]
fn PersonCard(person_id: String, directory: Memo<Directory>, cards: CardState) -> impl IntoView {
    let workflow_executor = expect_context::<crate::workflow_executor::WorkflowExecutor>();
    let saves = expect_context::<crate::task_saves::TaskSaves>();
    let PeopleSignal(people) = expect_context::<PeopleSignal>();
    let SelectedPersonSignal(expanded) = expect_context::<SelectedPersonSignal>();
    let id = StoredValue::new(person_id);
    let summary = Memo::new(move |_| {
        directory.with(|directory| {
            directory
                .summaries
                .get(&id.get_value())
                .cloned()
                .unwrap_or_default()
        })
    });
    let display_name = Signal::derive(move || summary.with(|summary| summary.name.clone()));
    let obs_count = Signal::derive(move || summary.with(|summary| summary.observations));
    let tracked = Signal::derive(move || summary.with(|summary| summary.tracked));
    let is_expanded = move || expanded.get().as_deref() == Some(id.get_value().as_str());
    let is_renaming = move || cards.renaming.get().as_deref() == Some(id.get_value().as_str());
    let is_merge_open = move || cards.merge_open.get().as_deref() == Some(id.get_value().as_str());
    let is_delete_open =
        move || cards.delete_open.get().as_deref() == Some(id.get_value().as_str());
    let observations = Memo::new(move |_| {
        if !is_expanded() {
            return Vec::new();
        }
        people.with(|people| {
            people
                .iter()
                .find(|person| person.id == id.get_value())
                .map(|person| person.observations.clone())
                .unwrap_or_default()
        })
    });
    let commit_rename = move || {
        if cards.renaming.get_untracked().as_deref() != Some(id.get_value().as_str()) {
            return;
        }
        let name = cards.rename_value.get_untracked();
        cards.renaming.set(None);
        workflow_executor.rename_person(id.get_value(), name);
    };
    let card_class = move || {
        if tracked.get() {
            "bg-longan-900/50 inset-ring inset-ring-longan-700 rounded-seed p-3 space-y-2"
        } else {
            "bg-longan-900/50 inset-ring inset-ring-longan-700 rounded-seed p-3 space-y-2 opacity-60"
        }
    };
    let action = "bruhs-control inline-flex items-center justify-center text-label-sm text-rambutan-500 hover:text-rambutan-300 cursor-pointer";

    view! {
        <li class=card_class>
            // Narrow cards wrap the actions under the name instead of overflowing.
            <div class="flex w-full flex-wrap items-center justify-between gap-x-3 gap-y-1">
                <Show
                    when=is_renaming
                    fallback=move || view! {
                        <button
                            type="button"
                            id=format!("person-record-{}", id.get_value())
                            aria-expanded=move || is_expanded().to_string()
                            class="flex min-w-0 flex-1 basis-40 flex-wrap items-center gap-2 text-left cursor-pointer"
                            on:click=move |_| {
                                let current = id.get_value();
                                expanded.update(|open| {
                                    *open = if open.as_deref() == Some(current.as_str()) { None } else { Some(current) };
                                });
                            }
                        >
                            <span class="min-w-0 break-words [overflow-wrap:anywhere] text-body-sm text-rambutan-200">{move || display_name.get()}</span>
                            <Show when=move || !tracked.get()>
                                <span class="shrink-0 whitespace-nowrap text-label-sm uppercase tracking-wide text-rambutan-300 bg-longan-800 inset-ring inset-ring-longan-600 rounded-seed px-1 py-0.5">
                                    "Not tracked"
                                </span>
                            </Show>
                        </button>
                    }
                >
                    // The rename field sits beside the actions, never inside the expand button.
                    <div class="flex min-w-0 flex-1 basis-40 items-center">
                        <input
                            type="text"
                            aria-label="Profile name"
                            class="bruhs-control w-full rounded-seed bg-longan-800 px-2 text-body-sm text-rambutan-100 inset-ring inset-ring-longan-600 focus-visible:outline-2 focus-visible:outline-persimmon-400"
                            prop:value=move || cards.rename_value.get()
                            on:input=move |event| cards.rename_value.set(event_target_value(&event))
                            on:keydown=move |event: web_sys::KeyboardEvent| match event.key().as_str() {
                                "Enter" => {
                                    event.prevent_default();
                                    commit_rename();
                                }
                                "Escape" => cards.renaming.set(None),
                                _ => {}
                            }
                            on:blur=move |_| commit_rename()
                            autofocus=true
                        />
                    </div>
                </Show>
                <div class="flex flex-wrap items-center justify-end gap-x-2">
                    <span class="whitespace-nowrap text-label-sm text-rambutan-500">
                        {move || obs_count.get().to_string()}" obs"
                    </span>
                    <button
                        type="button"
                        title="Rename profile"
                        aria-label=move || format!("Rename {}", display_name.get())
                        class=action
                        on:click=move |_| {
                            cards.rename_value.set(display_name.get_untracked());
                            cards.renaming.set(Some(id.get_value()));
                            if is_merge_open() { cards.merge_open.set(None); }
                        }
                    >"rename"</button>
                    <button
                        type="button"
                        title=move || if tracked.get() { "Stop collecting observations about this person and hide saved ones from agents" } else { "Collect observations about this person again" }
                        aria-label=move || format!("{} {}", if tracked.get() { "Untrack" } else { "Track" }, display_name.get())
                        class=move || if tracked.get() {
                            "bruhs-control inline-flex items-center justify-center text-label-sm text-pandan-400 hover:text-pandan-200 cursor-pointer"
                        } else {
                            "bruhs-control inline-flex items-center justify-center text-label-sm text-rambutan-500 hover:text-rambutan-400 cursor-pointer"
                        }
                        on:click=move |_| workflow_executor.set_person_tracked(id.get_value(), !tracked.get_untracked())
                    >
                        {move || if tracked.get() { "untrack" } else { "track" }}
                    </button>
                    <button
                        type="button"
                        title="Merge this profile into another"
                        aria-label=move || format!("Merge {} into another profile", display_name.get())
                        aria-expanded=move || is_merge_open().to_string()
                        class=action
                        on:click=move |_| {
                            let current = id.get_value();
                            cards.merge_confirming.set(false);
                            cards.merge_open.update(|open| {
                                *open = if open.as_deref() == Some(current.as_str()) {
                                    None
                                } else {
                                    cards.merge_target.set(String::new());
                                    Some(current)
                                };
                            });
                        }
                    >"⋯"</button>
                    <button
                        type="button"
                        title="Record voice sample for this person"
                        aria-label=move || format!("Enroll voice for {}", display_name.get())
                        class="bruhs-control inline-flex items-center justify-center text-label-sm text-rambutan-500 hover:text-pandan-300 cursor-pointer"
                        on:click=move |_| cards.enroll_open.set(Some(id.get_value()))
                    >"enroll"</button>
                    <button
                        type="button"
                        title="Delete this profile"
                        aria-label=move || format!("Delete {}", display_name.get())
                        aria-expanded=move || is_delete_open().to_string()
                        class="bruhs-control inline-flex items-center justify-center text-label-sm text-rambutan-500 hover:text-lychee-300 cursor-pointer"
                        on:click=move |_| {
                            let current = id.get_value();
                            cards.delete_open.update(|open| {
                                *open = if open.as_deref() == Some(current.as_str()) { None } else { Some(current) };
                            });
                        }
                    >"delete"</button>
                </div>
            </div>
            <Show when=is_merge_open>
                <confirm::MergePanel
                    source_id=id.get_value()
                    source_name=display_name
                    target=cards.merge_target
                    confirming=cards.merge_confirming
                    on_confirm=Callback::new(move |target: String| {
                        cards.merge_open.set(None);
                        cards.merge_confirming.set(false);
                        workflow_executor.merge_people(id.get_value(), target);
                    })
                    on_cancel=Callback::new(move |_| {
                        cards.merge_open.set(None);
                        cards.merge_confirming.set(false);
                    })
                />
            </Show>
            <Show when=is_delete_open>
                <confirm::DeletePanel
                    name=display_name
                    busy=Signal::derive(move || workflow_executor.people_busy())
                    on_confirm=Callback::new(move |_| {
                        cards.delete_open.set(None);
                        workflow_executor.delete_person(id.get_value(), saves.saving_links());
                    })
                    on_cancel=Callback::new(move |_| cards.delete_open.set(None))
                />
            </Show>
            // Closed cards do not allocate recording state or effects.
            <Show when=move || cards.enroll_open.get().as_deref() == Some(id.get_value().as_str())>
                <VoiceEnrollModal
                    person_id=id.get_value()
                    person_name=display_name
                    show=Signal::derive(move || cards.enroll_open.get().as_deref() == Some(id.get_value().as_str()))
                    on_close=move || cards.enroll_open.set(None)
                />
            </Show>
            <Show when=is_expanded>
                <div class="space-y-4 border-t border-longan-700/70 pt-3">
                    <Show when=move || !tracked.get()>
                        <p class="text-label-sm text-rambutan-400" data-people-opt-out>
                            "Observation tracking is off. Summaries add no observations about this person, and agents cannot read the saved ones."
                        </p>
                    </Show>
                    <profile::AliasesPanel person_id=id.get_value() />
                    <profile::LinkedRecords person_id=id.get_value() />
                    <relationship::RelationshipPanel person_id=id.get_value() />
                    {move || view! { <ObservationsByKind observations=observations.get() /> }}
                </div>
            </Show>
        </li>
    }
}

/// Whether the People page has finished loading meetings for provenance.
#[derive(Clone, Copy)]
struct MeetingSourcesReady(ReadSignal<bool>);

/// Provenance for one observation: open the meeting it came from, or say that
/// the source meeting is no longer stored once meetings have loaded.
#[component]
fn ObservationSource(meeting_id: String) -> impl IntoView {
    let MeetingsSignal(meetings) = expect_context::<MeetingsSignal>();
    let ready = use_context::<MeetingSourcesReady>()
        .map_or_else(|| Signal::stored(true), |ready| ready.0.into());
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();
    let title = {
        let meeting_id = meeting_id.clone();
        move || {
            meetings.with(|meetings| {
                meetings
                    .iter()
                    .find(|meeting| meeting.id == meeting_id)
                    .map(|meeting| meeting.title.clone())
            })
        }
    };
    let meeting_id = StoredValue::new(meeting_id);
    move || {
        match title() {
        Some(title) => view! {
            <button
                type="button"
                class="bruhs-control inline-flex items-center justify-center ml-2 text-label-sm text-blueberry-300 hover:text-blueberry-200 underline-offset-2 hover:underline cursor-pointer"
                aria-label=format!("Open source meeting {title}")
                on:click=move |_| set_active_view.set(AppView::MeetingDetail { meeting_id: meeting_id.get_value() })
            >
                {format!("From {title}")}
            </button>
        }
        .into_any(),
        None if ready.get() => view! {
            <span class="ml-2 text-label-sm text-rambutan-500">"Source meeting is not stored"</span>
        }
        .into_any(),
        None => ().into_any(),
    }
    }
}

#[component]
fn ObservationsByKind(observations: Vec<PersonObservation>) -> impl IntoView {
    // Group by kind, sort within by created_at desc.
    let mut by_kind: BTreeMap<&'static str, Vec<PersonObservation>> = BTreeMap::new();
    for obs in observations {
        let key = match obs.kind {
            ObservationKind::Topic => "Topics",
            ObservationKind::Decision => "Decisions",
            ObservationKind::ActionItemOwner => "Action items",
            ObservationKind::Tone => "Tone",
            ObservationKind::Quote => "Quotes",
        };
        by_kind.entry(key).or_default().push(obs);
    }
    for (_k, v) in by_kind.iter_mut() {
        v.sort_by_key(|o| Reverse(o.created_at));
    }
    let buckets: Vec<(&'static str, Vec<PersonObservation>)> = by_kind.into_iter().collect();

    view! {
        <div class="space-y-3 pt-2">
            <For
                each=move || buckets.clone()
                key=|(k, _)| *k
                let:bucket
            >
                {
                    let (kind_label, items) = bucket;
                    view! {
                        <div class="space-y-1">
                            <div class="text-label-sm uppercase tracking-wide text-rambutan-500">{kind_label}</div>
                            // One summary stamps all its observations with one
                            // time, so (meeting, time) keys collided and rows
                            // vanished. The list is static: render it in order.
                            <ul role="list" class="space-y-1">
                                {items.into_iter().map(|obs| view! {
                                    <li class="text-label text-rambutan-300 leading-snug" data-person-observation>
                                        <span>{obs.text}</span>
                                        <ObservationSource meeting_id=obs.at_meeting_id />
                                    </li>
                                }).collect_view()}
                            </ul>
                        </div>
                    }
                }
            </For>
        </div>
    }
}
