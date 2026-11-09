//! People page: list of PersonProfiles. Click a card to expand and
//! view observations grouped by kind.
//!
//! Phase 6d additions:
//! - Click display_name → inline edit input (Enter saves, Escape cancels).
//! - Click ⋯ → inline merge dialog (select other profile + Confirm button).

use leptos::prelude::*;
use std::cmp::Reverse;
use std::collections::BTreeMap;

use crate::api;
use crate::components::metadata_status::MetadataStatus;
use crate::components::voice_enroll::VoiceEnrollModal;
use crate::relationship_store::{profile_fields, EditorDraft, RelationshipStore};
use crate::state::{
    AppView, InboxConversationsSignal, ObservationKind, PeopleSignal, PersonObservation,
    PersonProfile, RelationshipsSignal, SelectedPersonSignal, SetActiveView,
};
use crate::ui::{Dropdown, DropdownOption, Textarea};
mod directory;
mod draft_status;
mod merge_status;
use directory::{Directory, PAGE_SIZE};

#[component]
pub fn PeoplePage() -> impl IntoView {
    let saves = expect_context::<crate::task_saves::TaskSaves>();
    let relationship_store = expect_context::<RelationshipStore>();
    let workflow_executor = expect_context::<crate::workflow_executor::WorkflowExecutor>();
    let PeopleSignal(people) = expect_context::<PeopleSignal>();
    let SelectedPersonSignal(expanded) = expect_context::<SelectedPersonSignal>();
    let set_expanded = expanded;
    let directory =
        Memo::new(move |previous| people.with(|people| Directory::new(people, previous)));
    let page = RwSignal::new(0usize);
    let page_count = Memo::new(move |_| {
        directory.with(|directory| directory.order.len().div_ceil(PAGE_SIZE).max(1))
    });
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
            if let Some(index) = directory.with(|directory| {
                directory
                    .order
                    .iter()
                    .position(|candidate| candidate == &id)
            }) {
                page.set(index / PAGE_SIZE);
            }
            last_focused.set_value(Some(id.clone()));
            super::record_focus::focus_record(format!("person-record-{id}"));
        }
    });
    // Which profile is being renamed (Some(id)) or None.
    let (renaming_id, set_renaming_id) = signal::<Option<String>>(None);
    // Rename input value.
    let (rename_value, set_rename_value) = signal::<String>(String::new());
    // Which profile has the merge panel open.
    let (merge_open_id, set_merge_open_id) = signal::<Option<String>>(None);
    // Selected target for merge.
    let (merge_target_id, set_merge_target_id) = signal::<String>(String::new());
    // Which profile has the voice-enroll modal open (Some(id)) or None.
    let (enroll_open_id, set_enroll_open_id) = signal::<Option<String>>(None);

    view! {
        <div class="flex h-full flex-col bg-longan-950">
            <crate::ui::PageHeader title="People" subtitle="Profiles from your meetings, with notes and reminders." />
            <div aria-label="Relationship save status">
                <MetadataStatus status=relationship_store.status
                    retry=Callback::new(move |_| relationship_store.retry())
                    recovery=Signal::derive(move || relationship_store.recovery_json()) />
            </div>
            <div class="min-h-0 flex-1 space-y-3 overflow-y-auto px-4 py-4 sm:px-6">
            <merge_status::ContactMergeStatus />
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
            <Show
                when=move || people.with(|people| !people.is_empty())
                fallback=move || view! {
                    <div class="text-label text-rambutan-500">
                        {move || if workflow_executor.refreshing_people() || !workflow_executor.people_hydrated.get() {
                            "Loading contacts… Profiles will appear as they are saved locally."
                        } else {
                            "No people yet. Profiles populate after your first meeting summary completes."
                        }}
                    </div>
                }
            >
                <ul role="list" class="space-y-2">
                    <For
                        each=move || directory.with(|directory| directory.page(page.get()))
                        key=|id| id.clone()
                        let:person_id
                    >
                        {
                            let row_id = StoredValue::new(person_id.clone());
                            let summary = Memo::new(move |_| directory.with(|directory| directory.summaries.get(&row_id.get_value()).cloned().unwrap_or_default()));
                            let id = person_id.clone();
                            let id_for_expanded = id.clone();
                            let id_for_button = id.clone();
                            let id_for_toggle = id.clone();
                            let id_for_rename_commit = id.clone();
                            let id_for_rename_cancel = id.clone();
                            let id_for_rename_btn = id.clone();
                            let id_for_renaming_check = id.clone();
                            let id_for_merge_toggle = id.clone();
                            let id_for_merge_confirm = id.clone();
                            let id_for_merge_open_check = id.clone();
                            let id_for_enroll = id;
                            let id_for_relationship = id_for_enroll.clone();
                            let is_expanded = move || expanded.get().as_deref() == Some(id_for_expanded.as_str());
                            let is_renaming = move || renaming_id.get().as_deref() == Some(id_for_renaming_check.as_str());
                            let is_merge_open = move || merge_open_id.get().as_deref() == Some(id_for_merge_open_check.as_str());
                            let observations = Memo::new(move |_| {
                                if expanded.get().as_deref() != Some(row_id.get_value().as_str()) { return Vec::new(); }
                                people.with(|people| people.iter().find(|person| person.id == row_id.get_value()).map(|person| person.observations.clone()).unwrap_or_default())
                            });
                            let display_name = Signal::derive(move || summary.with(|summary| summary.name.clone()));
                            let display_name_for_rename = display_name;
                            let obs_count = Signal::derive(move || summary.with(|summary| summary.observations));
                            let tracked = Signal::derive(move || summary.with(|summary| summary.tracked));

                            let card_class = move || if tracked.get() {
                                "bg-longan-900/50 inset-ring inset-ring-longan-700 rounded-seed p-3 space-y-2"
                            } else {
                                "bg-longan-900/50 inset-ring inset-ring-longan-700 rounded-seed p-3 space-y-2 opacity-60"
                            };

                            view! {
                                <li class=card_class>
                                    <div class="flex items-center justify-between w-full">
                                        // Left side: expand toggle OR inline rename input
                                        <button
                                            id=format!("person-record-{person_id}")
                                            aria-expanded={let is_expanded = is_expanded.clone(); move || is_expanded().to_string()}
                                            class="flex items-center gap-2 flex-1 text-left cursor-pointer"
                                            on:click=move |_| {
                                                let id = id_for_button.clone();
                                                // Don't toggle expand if rename is active.
                                                if renaming_id.get_untracked().is_some() { return; }
                                                set_expanded.update(|e| {
                                                    *e = if e.as_deref() == Some(id.as_str()) {
                                                        None
                                                    } else {
                                                        Some(id)
                                                    };
                                                });
                                            }
                                        >
                                            <Show
                                                when=is_renaming
                                                fallback={
                                                    let dn = display_name;
                                                    move || view! {
                                                        <span class="text-body-sm text-rambutan-200">
                                                            {move || dn.get()}
                                                        </span>
                                                    }
                                                }
                                            >
                                                // Inline rename input — rendered inside the button area but
                                                // stops click propagation so the expand-toggle doesn't fire.
                                                <input
                                                    type="text"
                                                    class="bg-longan-800 inset-ring inset-ring-longan-600 rounded-seed px-1 py-0.5 text-body-sm text-rambutan-100 focus:outline-none w-full"
                                                    prop:value=rename_value
                                                    on:input=move |e| {
                                                        set_rename_value.set(leptos::prelude::event_target_value(&e));
                                                    }
                                                    on:keydown={
                                                        let id_commit = id_for_rename_commit.clone();
                                                        move |e: web_sys::KeyboardEvent| {
                                                            match e.key().as_str() {
                                                                "Enter" => {
                                                                    let pid = id_commit.clone();
                                                                    let name = rename_value.get_untracked();
                                                                    set_renaming_id.set(None);
                                                                    wasm_bindgen_futures::spawn_local(async move {
                                                                        match api::rename_person(&pid, &name).await {
                                                                            Ok(updated) => {
                                                                                workflow_executor.cache_people(vec![updated]).await;
                                                                            }
                                                                            Err(e) => log::warn!("rename_person failed: {e}"),
                                                                        }
                                                                    });
                                                                }
                                                                "Escape" => {
                                                                    set_renaming_id.set(None);
                                                                }
                                                                _ => {}
                                                            }
                                                        }
                                                    }
                                                    on:blur={
                                                        let id_blur = id_for_rename_cancel.clone();
                                                        move |_| {
                                                            // Blur commits (same as Enter). Only commit if
                                                            // still renaming this card (re-entry guard).
                                                            if renaming_id.get_untracked().as_deref() == Some(id_blur.as_str()) {
                                                                let pid = id_blur.clone();
                                                                let name = rename_value.get_untracked();
                                                                set_renaming_id.set(None);
                                                                wasm_bindgen_futures::spawn_local(async move {
                                                                    match api::rename_person(&pid, &name).await {
                                                                        Ok(updated) => {
                                                                            workflow_executor.cache_people(vec![updated]).await;
                                                                        }
                                                                        Err(e) => log::warn!("rename_person (blur) failed: {e}"),
                                                                    }
                                                                });
                                                            }
                                                        }
                                                    }
                                                    on:click=|e: web_sys::MouseEvent| e.stop_propagation()
                                                    autofocus=true
                                                />
                                            </Show>
                                            {move || if !tracked.get() {
                                                view! {
                                                    <span class="text-label-sm uppercase tracking-wide text-rambutan-300 bg-longan-800 inset-ring inset-ring-longan-600 rounded-seed px-1 py-0.5">
                                                        "untracked"
                                                    </span>
                                                }.into_any()
                                            } else {
                                                ().into_any()
                                            }}
                                        </button>
                                        <div class="flex items-center gap-2">
                                            <span class="text-label-sm text-rambutan-500">
                                                {move || obs_count.get().to_string()}" obs"
                                            </span>
                                            // Rename trigger button (pencil / edit name)
                                            <button
                                                type="button"
                                                title="Rename profile"
                                                class="text-label-sm text-rambutan-500 hover:text-rambutan-300 cursor-pointer"
                                                on:click={
                                                    let dn = display_name_for_rename;
                                                    let pid = id_for_rename_btn;
                                                    move |_| {
                                                        set_rename_value.set(dn.get_untracked());
                                                        set_renaming_id.set(Some(pid.clone()));
                                                        // Close merge panel if open for this profile.
                                                        if merge_open_id.get_untracked().as_deref() == Some(pid.as_str()) {
                                                            set_merge_open_id.set(None);
                                                        }
                                                    }
                                                }
                                            >
                                                "rename"
                                            </button>
                                            // Tracked toggle button
                                            <button
                                                type="button"
                                                title=move || if tracked.get() { "Disable observation tracking" } else { "Enable observation tracking" }
                                                class=move || if tracked.get() {
                                                    "text-label-sm text-pandan-400 hover:text-pandan-200 cursor-pointer"
                                                } else {
                                                    "text-label-sm text-rambutan-500 hover:text-rambutan-400 cursor-pointer"
                                                }
                                                on:click=move |_| {
                                                    let pid = id_for_toggle.clone();
                                                    let new_tracked = !tracked.get_untracked();
                                                    wasm_bindgen_futures::spawn_local(async move {
                                                        match api::set_person_tracked(&pid, new_tracked).await {
                                                            Ok(updated) => {
                                                                workflow_executor.cache_people(vec![updated]).await;
                                                            }
                                                            Err(e) => log::warn!("set_person_tracked failed: {e}"),
                                                        }
                                                    });
                                                }
                                            >
                                                {move || if tracked.get() { "track" } else { "untrack" }}
                                            </button>
                                            // ⋯ merge menu button
                                            <button
                                                type="button"
                                                title="Merge this profile into another"
                                                class="text-label-sm text-rambutan-500 hover:text-rambutan-300 cursor-pointer"
                                                on:click={
                                                    let pid = id_for_merge_toggle;
                                                    move |_| {
                                                        set_merge_open_id.update(|cur| {
                                                            *cur = if cur.as_deref() == Some(pid.as_str()) {
                                                                None
                                                            } else {
                                                                set_merge_target_id.set(String::new());
                                                                Some(pid.clone())
                                                            };
                                                        });
                                                    }
                                                }
                                            >
                                                "⋯"
                                            </button>
                                            // Voice enroll button
                                            {
                                                let enroll_btn_pid = id_for_enroll.clone();
                                                view! {
                                                    <button
                                                        type="button"
                                                        title="Record voice sample for this person"
                                                        class="text-label-sm text-rambutan-500 hover:text-pandan-300 cursor-pointer"
                                                        on:click=move |_| {
                                                            set_enroll_open_id.set(Some(enroll_btn_pid.clone()));
                                                        }
                                                    >
                                                        "enroll"
                                                    </button>
                                                }
                                            }
                                        </div>
                                    </div>
                                    // Inline merge panel
                                    <Show when=is_merge_open fallback=|| view! { <></> }>
                                        <MergePanel
                                            source_id=id_for_merge_confirm.clone()
                                            people=people
                                            merge_target_id=merge_target_id
                                            set_merge_target_id=set_merge_target_id
                                            on_confirm={
                                                let src = id_for_merge_confirm.clone();
                                                move || {
                                                    let src2 = src.clone();
                                                    let tgt = merge_target_id.get_untracked();
                                                    if tgt.is_empty() { return; }
                                                    set_merge_open_id.set(None);
                                                    workflow_executor.merge_people(src2, tgt);
                                                }
                                            }
                                            on_cancel=move || set_merge_open_id.set(None)
                                        />
                                    </Show>
                                    // Closed cards do not allocate recording state or effects.
                                    {
                                        let enroll_pid = id_for_enroll.clone();
                                        let enroll_pname = display_name;
                                        let show_enroll = Signal::derive(move || {
                                            enroll_open_id.get().as_deref() == Some(enroll_pid.as_str())
                                        });
                                        view! {
                                            <Show when=move || show_enroll.get()>
                                                <VoiceEnrollModal
                                                    person_id=id_for_enroll.clone()
                                                    person_name=enroll_pname
                                                    show=show_enroll
                                                    on_close=move || set_enroll_open_id.set(None)
                                                />
                                            </Show>
                                        }
                                    }
                                    <Show when=is_expanded fallback=|| view! { <></> }>
                                        {
                                            let relationship_id = id_for_relationship.clone();
                                            move || view! {
                                                <div class="space-y-4 border-t border-longan-700/70 pt-3">
                                                    <RelationshipPanel person_id=relationship_id.clone() />
                                                    {move || view! { <ObservationsByKind observations=observations.get() /> }}
                                                </div>
                                            }
                                        }
                                    </Show>
                                </li>
                            }
                        }
                    </For>
                </ul>
                <Show when=move || { page_count.get() > 1 }>
                    <nav aria-label="People pages" class="flex items-center justify-between gap-2 py-2 text-label-sm">
                        <button type="button" disabled=move || page.get() == 0 class="rounded-grape bg-longan-800 px-3 py-2 disabled:opacity-40 cursor-pointer" on:click=move |_| page.update(|page| *page = page.saturating_sub(1))>"Previous people"</button>
                        <span role="status">{move || format!("Page {} of {}", page.get() + 1, page_count.get())}</span>
                        <button type="button" disabled=move || { page.get() + 1 >= page_count.get() } class="rounded-grape bg-longan-800 px-3 py-2 disabled:opacity-40 cursor-pointer" on:click=move |_| page.update(|page| *page += 1)>"Next people"</button>
                    </nav>
                </Show>
            </Show>
            </div>
        </div>
    }
}

#[component]
fn RelationshipPanel(person_id: String) -> impl IntoView {
    use bruhs_models::{suggested_follow_ups, RelationshipItem, RelationshipItemKind};

    let RelationshipsSignal(relationships) = expect_context::<RelationshipsSignal>();
    let store = expect_context::<RelationshipStore>();
    let InboxConversationsSignal(conversations) = expect_context::<InboxConversationsSignal>();
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();
    let id = StoredValue::new(person_id);
    let company = RwSignal::new(String::new());
    let role = RwSignal::new(String::new());
    let email = RwSignal::new(String::new());
    let phone = RwSignal::new(String::new());
    let location = RwSignal::new(String::new());
    let notes = RwSignal::new(String::new());
    let item_kind = RwSignal::new(RelationshipItemKind::Interaction);
    let item_text = RwSignal::new(String::new());
    let due_tomorrow = RwSignal::new(false);
    let initialized = RwSignal::new(false);
    let fields = [company, role, email, phone, location, notes];
    let baseline = StoredValue::new(<[String; 6]>::default());
    let last_reset = StoredValue::new(0);
    let draft_error = RwSignal::new(Option::<&'static str>::None);
    let saved_fields = move || {
        relationships.with_untracked(|profiles| {
            profiles
                .iter()
                .find(|profile| profile.person_id == id.get_value())
                .map(profile_fields)
                .unwrap_or_default()
        })
    };
    let read_draft = move || EditorDraft {
        fields: fields.map(|field| field.get_untracked()),
        baseline: baseline.get_value(),
        item_kind: item_kind.get_untracked(),
        item_text: item_text.get_untracked(),
        due_tomorrow: due_tomorrow.get_untracked(),
    };
    let remember = move || {
        draft_error.set(None);
        store.keep_editor_draft(id.get_value(), read_draft());
    };

    Effect::new(move |_| {
        if !store.ready() {
            return;
        }
        relationships.track();
        let reset = store.draft_reset.get();
        let next = saved_fields();
        let mut draft = if !initialized.get_untracked() || last_reset.get_value() != reset {
            draft_error.set(None);
            store
                .editor_draft(&id.get_value())
                .unwrap_or_else(|| EditorDraft::new(next.clone()))
        } else {
            read_draft()
        };
        draft.reconcile(next);
        baseline.set_value(draft.baseline.clone());
        for (field, value) in fields.into_iter().zip(&draft.fields) {
            field.set(value.clone());
        }
        item_kind.set(draft.item_kind);
        item_text.set(draft.item_text.clone());
        due_tomorrow.set(draft.due_tomorrow);
        last_reset.set_value(reset);
        initialized.set(true);
        store.keep_editor_draft(id.get_value(), draft);
    });

    let save_details = move || {
        let Some([company, role, email, phone, location, notes]) =
            read_draft().details_to_save(saved_fields())
        else {
            draft_error.set(Some("Saved details changed while you were editing. Your draft is kept. Copy it before discarding to review the latest details."));
            return;
        };
        if store.update(&id.get_value(), |profile| {
            profile.company = company;
            profile.role = role;
            profile.email = email;
            profile.phone = phone;
            profile.location = location;
            profile.notes = notes;
            profile.updated_at_ms = crate::clock::now_ms();
        }) {
            // Submitted details now belong to the save queue. An unsubmitted
            // item remains a separate draft and must not be cleared by Save.
            let mut draft = read_draft();
            draft.reconcile(saved_fields());
            baseline.set_value(draft.baseline.clone());
            store.keep_editor_draft(id.get_value(), draft);
            draft_error.set(None);
        }
    };
    let add_item = move || {
        let now = crate::clock::now_ms();
        let mut item =
            RelationshipItem::new(item_kind.get_untracked(), item_text.get_untracked(), now);
        if item.text.is_empty() {
            return;
        }
        if due_tomorrow.get_untracked()
            && matches!(
                item.kind,
                RelationshipItemKind::Commitment | RelationshipItemKind::Reminder
            )
        {
            item.due_at_ms = Some(now + 86_400_000);
        }
        if store.update(&id.get_value(), |profile| {
            profile.add_item(item, now);
        }) {
            item_text.set(String::new());
            due_tomorrow.set(false);
            remember();
        }
    };

    let suggestions = move || {
        let suggestions = relationships.with(|profiles| {
            conversations.with(|conversations| {
                profiles
                    .iter()
                    .find(|profile| profile.person_id == id.get_value())
                    .map(|profile| {
                        suggested_follow_ups(
                            std::slice::from_ref(profile),
                            conversations,
                            crate::clock::now_ms(),
                        )
                    })
                    .unwrap_or_default()
            })
        });
        (!suggestions.is_empty()).then(|| {
            let rows = suggestions
                .into_iter()
                .map(|suggestion| {
                    let source = suggestion.source_record_id.clone();
                    let open = move |_| {
                        if let Some(conversation_id) = source.clone() {
                            set_active_view.set(AppView::ConversationDetail { conversation_id });
                        }
                    };
                    view! {
                        <button
                            type="button"
                            class="block w-full truncate text-left text-body-sm text-rambutan-200 hover:text-persimmon-200 cursor-pointer"
                            on:click=open
                        >
                            {suggestion.reason}
                        </button>
                    }
                })
                .collect_view();
            view! {
                <div class="rounded-grape bg-persimmon-500/8 p-2.5 inset-ring inset-ring-persimmon-500/20">
                    <p class="text-label-sm uppercase tracking-wide text-persimmon-300">"Suggested follow-ups"</p>
                    <div class="mt-1 space-y-1">{rows}</div>
                </div>
            }
        })
    };
    let kind_button = move |kind: RelationshipItemKind| {
        view! {
            <button
                type="button"
                aria-pressed=move || (item_kind.get() == kind).to_string()
                class=move || if item_kind.get() == kind {
                    "rounded-seed bg-blueberry-500/15 px-2 py-1 text-label-sm text-blueberry-300 cursor-pointer"
                } else {
                    "rounded-seed px-2 py-1 text-label-sm text-rambutan-500 hover:bg-longan-800 cursor-pointer"
                }
                on:click=move |_| {
                    item_kind.set(kind);
                    remember();
                }
            >
                {kind.label()}
            </button>
        }
    };
    let items = move || {
        relationships
            .with(|profiles| {
                profiles
                    .iter()
                    .find(|profile| profile.person_id == id.get_value())
                    .cloned()
            })
            .map(|profile| {
                let rows = profile
                    .items
                    .into_iter()
                    .map(|item| {
                        let item_id = item.id.clone();
                        let complete = item.completed_at_ms.is_some();
                        let toggle = move |_| {
                            store.update(&id.get_value(), |profile| {
                                profile.toggle_item(&item_id, crate::clock::now_ms());
                            });
                        };
                        view! {
                            <li>
                                <button
                                    type="button"
                                    class="flex w-full items-start gap-2 rounded-seed px-2 py-1.5 text-left hover:bg-longan-800/70 cursor-pointer"
                                    on:click=toggle
                                >
                                    <span class=move || if complete { "mt-0.5 text-pandan-300" } else { "mt-0.5 text-rambutan-400" }>
                                        {if complete { "✓" } else { "○" }}
                                    </span>
                                    <span class="min-w-0 flex-1">
                                        <span class=move || if complete {
                                            "block text-body-sm text-rambutan-400 line-through"
                                        } else {
                                            "block text-body-sm text-rambutan-300"
                                        }>{item.text}</span>
                                        <span class="block text-label-sm text-rambutan-400">
                                            {item.kind.label()}
                                            {if item.due_at_ms.is_some() { " · due" } else { "" }}
                                        </span>
                                    </span>
                                </button>
                            </li>
                        }
                    })
                    .collect_view();
                view! { <ol class="space-y-1">{rows}</ol> }
            })
    };

    view! {
        <section aria-label="Relationship details" class="space-y-3">
            {move || draft_error.get().map(|message| view! {
                <p role="alert" class="text-body-sm text-lychee-300">{message}</p>
            })}
            <fieldset disabled=move || !initialized.get() || !store.editable(&id.get_value()) class="min-w-0 space-y-3 disabled:opacity-60">
            <div class="flex items-baseline justify-between">
                <div>
                    <h3 class="text-label text-rambutan-200">"Relationship"</h3>
                    <p class="text-label-sm text-rambutan-500">"Private context, commitments, and timely follow-ups."</p>
                </div>
                <button
                    type="button"
                    class="rounded-seed px-2 py-1 text-label-sm text-persimmon-300 hover:bg-longan-800 cursor-pointer"
                    on:click=move |_| save_details()
                >
                    "Save details"
                </button>
            </div>
            <div class="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
                <RelationshipField label="Company" value=company on_edit=Callback::new(move |_| remember()) />
                <RelationshipField label="Role" value=role on_edit=Callback::new(move |_| remember()) />
                <RelationshipField label="Email" value=email on_edit=Callback::new(move |_| remember()) />
                <RelationshipField label="Phone" value=phone on_edit=Callback::new(move |_| remember()) />
                <RelationshipField label="Location" value=location on_edit=Callback::new(move |_| remember()) />
                <label class="block space-y-1 sm:col-span-2 lg:col-span-3">
                    <span class="text-label-sm text-rambutan-500">"Notes"</span>
                    <Textarea rows=4 value=Signal::derive(move || notes.get())
                        on_input=Callback::new(move |value| { notes.set(value); remember(); }) class="text-body-sm" />
                </label>
            </div>

            {suggestions}

            <div class="rounded-grape bg-longan-900/60 p-2.5 inset-ring inset-ring-longan-800">
                <div class="flex flex-wrap gap-1">
                    {[
                        RelationshipItemKind::Interaction,
                        RelationshipItemKind::Commitment,
                        RelationshipItemKind::Reminder,
                        RelationshipItemKind::Note,
                    ]
                    .into_iter()
                    .map(kind_button)
                    .collect_view()}
                </div>
                <div class="mt-2 flex flex-col gap-2 sm:flex-row">
                    <input
                        type="text"
                        class="min-w-0 flex-1 rounded-grape bg-white/5 px-3 py-2 text-body-sm text-rambutan-100 placeholder:text-rambutan-500 inset-ring inset-ring-white/10 focus-visible:outline-2 focus-visible:outline-persimmon-400"
                        placeholder="Add interaction, commitment, reminder, or note…"
                        prop:value=move || item_text.get()
                        on:input=move |event| {
                            item_text.set(event_target_value(&event));
                            remember();
                        }
                        on:keydown=move |event: web_sys::KeyboardEvent| {
                            if event.key() == "Enter" {
                                event.prevent_default();
                                add_item();
                            }
                        }
                    />
                    <label class="flex items-center gap-1.5 text-label-sm text-rambutan-500">
                        <input
                            type="checkbox"
                            prop:checked=move || due_tomorrow.get()
                            on:change=move |event| {
                                due_tomorrow.set(event_target_checked(&event));
                                remember();
                            }
                        />
                        "Due tomorrow"
                    </label>
                    <button
                        type="button"
                        class="rounded-grape bg-blueberry-500/15 px-3 py-2 text-label-sm text-blueberry-300 hover:bg-blueberry-500/25 cursor-pointer"
                        on:click=move |_| add_item()
                    >
                        "Add"
                    </button>
                </div>
            </div>

            {items}
            </fieldset>
        </section>
    }
}

#[component]
fn RelationshipField(
    label: &'static str,
    value: RwSignal<String>,
    on_edit: Callback<()>,
) -> impl IntoView {
    view! {
        <label class="space-y-1">
            <span class="text-label-sm text-rambutan-500">{label}</span>
            <input
                type="text"
                class="block w-full rounded-seed bg-white/5 px-2.5 py-1.5 text-body-sm text-rambutan-100 placeholder:text-rambutan-500 inset-ring inset-ring-white/10 focus-visible:outline-2 focus-visible:outline-persimmon-400"
                prop:value=move || value.get()
                on:input=move |event| {
                    value.set(event_target_value(&event));
                    on_edit.run(());
                }
            />
        </label>
    }
}

/// Inline merge selection panel rendered within the profile card.
#[component]
fn MergePanel(
    source_id: String,
    people: ReadSignal<Vec<PersonProfile>>,
    merge_target_id: ReadSignal<String>,
    set_merge_target_id: WriteSignal<String>,
    on_confirm: impl Fn() + 'static,
    on_cancel: impl Fn() + 'static,
) -> impl IntoView {
    // All profiles except the source.
    let options = move || {
        people.with(|ps| {
            ps.iter()
                .filter(|p| p.id != source_id)
                .cloned()
                .collect::<Vec<_>>()
        })
    };

    view! {
        <div class="mt-1 p-2 bg-longan-800/80 inset-ring inset-ring-longan-700 rounded-seed space-y-2">
            <div class="text-label-sm text-rambutan-400">"Merge into:"</div>
            <Dropdown
                options=Signal::derive(move || {
                    options()
                        .into_iter()
                        .map(|p| DropdownOption::new(p.id, p.display_name))
                        .collect::<Vec<_>>()
                })
                value=Signal::derive(move || merge_target_id.get())
                on_change=Callback::new(move |v: String| set_merge_target_id.set(v))
                placeholder="Pick a profile"
                label="Merge into"
                class="w-full"
            />
            <div class="flex gap-2 justify-end">
                <button
                    type="button"
                    class="text-label-sm text-rambutan-500 hover:text-rambutan-300 cursor-pointer"
                    on:click=move |_| on_cancel()
                >
                    "Cancel"
                </button>
                <button
                    type="button"
                    class="text-label-sm text-persimmon-400 hover:text-persimmon-200 cursor-pointer disabled:opacity-40"
                    prop:disabled=move || merge_target_id.get().is_empty()
                    on:click=move |_| on_confirm()
                >
                    "Merge"
                </button>
            </div>
        </div>
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
                            <ul role="list" class="space-y-1">
                                <For
                                    each=move || items.clone()
                                    key=|o| (o.at_meeting_id.clone(), o.created_at)
                                    let:obs
                                >
                                    <li class="text-label text-rambutan-300 leading-snug">
                                        "• "{obs.text}
                                    </li>
                                </For>
                            </ul>
                        </div>
                    }
                }
            </For>
        </div>
    }
}
