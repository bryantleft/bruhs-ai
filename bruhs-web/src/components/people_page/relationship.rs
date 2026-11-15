//! Private relationship context: details, follow ups, and interactions,
//! commitments, reminders and notes, which can be added, edited, completed
//! and deleted with the same options on both surfaces.

use bruhs_models::{
    people_copy, suggested_follow_ups, RelationshipDue, RelationshipItem, RelationshipItemEdit,
    RelationshipItemKind, RELATIONSHIP_ITEM_KINDS,
};
use leptos::prelude::*;

use super::back::PersonOpener;
use crate::relationship_store::{profile_fields, EditorDraft, RelationshipStore};
use crate::state::{AppView, InboxConversationsSignal, RelationshipsSignal};
use crate::ui::Textarea;

const ITEM_ACTION: &str = "bruhs-control inline-flex shrink-0 items-center justify-center rounded-seed px-2 text-label-sm text-rambutan-500 hover:text-rambutan-300 cursor-pointer";

fn due_option(checked: bool) -> RelationshipDue {
    if checked {
        RelationshipDue::Tomorrow
    } else {
        RelationshipDue::None
    }
}

/// The four kind buttons, as a pressed group.
fn kind_buttons(selected: RwSignal<RelationshipItemKind>, on_pick: impl Fn() + Copy + 'static) -> impl IntoView {
    view! {
        <div role="group" aria-label="Item kind" class="flex flex-wrap gap-1">
            {RELATIONSHIP_ITEM_KINDS.into_iter().map(|kind| view! {
                <button
                    type="button"
                    aria-pressed=move || (selected.get() == kind).to_string()
                    class=move || if selected.get() == kind {
                        "bruhs-control inline-flex items-center justify-center rounded-seed bg-blueberry-500/15 px-2 text-label-sm text-blueberry-300 cursor-pointer"
                    } else {
                        "bruhs-control inline-flex items-center justify-center rounded-seed px-2 text-label-sm text-rambutan-500 hover:bg-longan-800 cursor-pointer"
                    }
                    on:click=move |_| {
                        selected.set(kind);
                        on_pick();
                    }
                >
                    {kind.label()}
                </button>
            }).collect_view()}
        </div>
    }
}

fn due_checkbox(checked: RwSignal<bool>, on_change: impl Fn() + Copy + 'static) -> impl IntoView {
    view! {
        <label class="bruhs-control flex shrink-0 items-center gap-1.5 text-label-sm text-rambutan-500">
            <input
                type="checkbox"
                prop:checked=move || checked.get()
                on:change=move |event| {
                    checked.set(event_target_checked(&event));
                    on_change();
                }
            />
            {people_copy::DUE_TOMORROW}
        </label>
    }
}

#[component]
pub fn RelationshipPanel(person_id: String) -> impl IntoView {
    let RelationshipsSignal(relationships) = expect_context::<RelationshipsSignal>();
    let store = expect_context::<RelationshipStore>();
    let InboxConversationsSignal(conversations) = expect_context::<InboxConversationsSignal>();
    let opener = PersonOpener::from_context();
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
    // One item at a time is edited in place.
    let editing = RwSignal::new(Option::<String>::None);
    let edit_kind = RwSignal::new(RelationshipItemKind::Interaction);
    let edit_text = RwSignal::new(String::new());
    let edit_due = RwSignal::new(false);
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
        let item = RelationshipItem::with_due(
            item_kind.get_untracked(),
            item_text.get_untracked(),
            due_option(due_tomorrow.get_untracked()),
            now,
        );
        if item.text.is_empty() {
            return;
        }
        if store.update(&id.get_value(), |profile| {
            profile.add_item(item, now);
        }) {
            item_text.set(String::new());
            due_tomorrow.set(false);
            remember();
        }
    };
    let begin_edit = move |item: &RelationshipItem| {
        edit_kind.set(item.kind);
        edit_text.set(item.text.clone());
        edit_due.set(item.due_at_ms.is_some());
        draft_error.set(None);
        editing.set(Some(item.id.clone()));
    };
    let save_edit = move || {
        let Some(item_id) = editing.get_untracked() else {
            return;
        };
        if edit_text.get_untracked().trim().is_empty() {
            draft_error.set(Some(people_copy::ITEM_BLANK));
            return;
        }
        let edit = RelationshipItemEdit {
            kind: edit_kind.get_untracked(),
            text: edit_text.get_untracked(),
            due: due_option(edit_due.get_untracked()),
        };
        if store.update(&id.get_value(), |profile| {
            profile.edit_item(&item_id, edit, crate::clock::now_ms());
        }) {
            draft_error.set(None);
            editing.set(None);
        }
    };
    let delete_item = move |item_id: String| {
        store.update(&id.get_value(), |profile| {
            profile.remove_item(&item_id, crate::clock::now_ms());
        });
        if editing.get_untracked().as_deref() == Some(item_id.as_str()) {
            editing.set(None);
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
                            opener.open(&id.get_value(), AppView::ConversationDetail { conversation_id });
                        }
                    };
                    view! {
                        <button
                            type="button"
                            class="bruhs-control block w-full truncate text-left text-body-sm text-rambutan-200 hover:text-blueberry-200 cursor-pointer"
                            on:click=open
                        >
                            {suggestion.reason}
                        </button>
                    }
                })
                .collect_view();
            view! {
                <div class="rounded-grape bg-blueberry-500/8 p-2.5 inset-ring inset-ring-blueberry-500/20">
                    <p class="text-label-sm uppercase tracking-wide text-blueberry-300">"Suggested follow ups"</p>
                    <div class="mt-1 space-y-1">{rows}</div>
                </div>
            }
        })
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
                        let item_id = StoredValue::new(item.id.clone());
                        let complete = item.completed_at_ms.is_some();
                        let text = item.text.clone();
                        let is_editing = move || editing.get().as_deref() == Some(item_id.get_value().as_str());
                        let toggle = move |_| {
                            store.update(&id.get_value(), |profile| {
                                profile.toggle_item(&item_id.get_value(), crate::clock::now_ms());
                            });
                        };
                        let original = StoredValue::new(item);
                        view! {
                            <li data-relationship-item="">
                                <Show
                                    when=is_editing
                                    fallback=move || {
                                        let item = original.get_value();
                                        let text = item.text.clone();
                                        view! {
                                            <div class="flex items-start gap-1">
                                                <button
                                                    type="button"
                                                    aria-label=format!("{} {}", if complete { "Reopen" } else { "Complete" }, text)
                                                    class="flex min-w-0 flex-1 items-start gap-2 rounded-seed px-2 py-1.5 text-left hover:bg-longan-800/70 cursor-pointer"
                                                    on:click=toggle
                                                >
                                                    <span class=move || if complete { "mt-0.5 text-pandan-300" } else { "mt-0.5 text-rambutan-400" }>
                                                        {if complete { "✓" } else { "○" }}
                                                    </span>
                                                    <span class="min-w-0 flex-1">
                                                        <span class=move || if complete {
                                                            "block break-words text-body-sm text-rambutan-400 line-through"
                                                        } else {
                                                            "block break-words text-body-sm text-rambutan-300"
                                                        }>{item.text.clone()}</span>
                                                        <span class="block text-label-sm text-rambutan-400">
                                                            {item.kind.label()}
                                                            {if item.due_at_ms.is_some() { ", due" } else { "" }}
                                                        </span>
                                                    </span>
                                                </button>
                                                <button type="button" class=ITEM_ACTION
                                                    aria-label=format!("Edit {text}")
                                                    on:click=move |_| begin_edit(&original.get_value())>"edit"</button>
                                                <button type="button" class="bruhs-control inline-flex shrink-0 items-center justify-center rounded-seed px-2 text-label-sm text-rambutan-500 hover:text-lychee-300 cursor-pointer"
                                                    aria-label=format!("Delete {text}")
                                                    on:click=move |_| delete_item(item_id.get_value())>"delete"</button>
                                            </div>
                                        }
                                    }
                                >
                                    <form aria-label=format!("Edit {text}") class="space-y-2 rounded-seed bg-longan-900/70 p-2 inset-ring inset-ring-longan-700"
                                        on:submit=move |event| { event.prevent_default(); save_edit(); }>
                                        {kind_buttons(edit_kind, || {})}
                                        <div class="flex flex-col gap-2 sm:flex-row sm:items-center">
                                            <input type="text" aria-label="Item text"
                                                class="bruhs-control min-w-0 flex-1 rounded-grape bg-white/5 px-3 text-body-sm text-rambutan-100 inset-ring inset-ring-white/10 focus-visible:outline-2 focus-visible:outline-persimmon-400"
                                                prop:value=move || edit_text.get()
                                                on:input=move |event| edit_text.set(event_target_value(&event))
                                                on:keydown=move |event: web_sys::KeyboardEvent| if event.key() == "Escape" { editing.set(None); }
                                            />
                                            {due_checkbox(edit_due, || {})}
                                            <div class="flex gap-2">
                                                <button type="button" class=ITEM_ACTION on:click=move |_| editing.set(None)>{people_copy::CANCEL}</button>
                                                <button type="submit" class="bruhs-control inline-flex items-center justify-center rounded-grape bg-blueberry-500/15 px-3 text-label-sm text-blueberry-300 hover:bg-blueberry-500/25 cursor-pointer">{people_copy::ITEM_SAVE}</button>
                                            </div>
                                        </div>
                                    </form>
                                </Show>
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
                    <p class="text-label-sm text-rambutan-500">"Private context, commitments, and timely follow ups."</p>
                </div>
                <button
                    type="button"
                    class="bruhs-control rounded-seed px-2 text-label-sm text-persimmon-300 hover:bg-longan-800 cursor-pointer"
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
                {kind_buttons(item_kind, remember)}
                <div class="mt-2 flex flex-col gap-2 sm:flex-row sm:items-center">
                    <input
                        type="text"
                        aria-label="New relationship item"
                        class="bruhs-control min-w-0 flex-1 rounded-grape bg-white/5 px-3 text-body-sm text-rambutan-100 placeholder:text-rambutan-500 inset-ring inset-ring-white/10 focus-visible:outline-2 focus-visible:outline-persimmon-400"
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
                    <Show when=move || RelationshipDue::applies_to(item_kind.get())>
                        {due_checkbox(due_tomorrow, remember)}
                    </Show>
                    <button
                        type="button"
                        class="bruhs-control rounded-grape bg-blueberry-500/15 px-3 text-label-sm text-blueberry-300 hover:bg-blueberry-500/25 cursor-pointer"
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
                class="bruhs-control block w-full rounded-seed bg-white/5 px-2.5 text-body-sm text-rambutan-100 placeholder:text-rambutan-500 inset-ring inset-ring-white/10 focus-visible:outline-2 focus-visible:outline-persimmon-400"
                prop:value=move || value.get()
                on:input=move |event| {
                    value.set(event_target_value(&event));
                    on_edit.run(());
                }
            />
        </label>
    }
}
