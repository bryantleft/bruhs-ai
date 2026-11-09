//! One-field, keyboard-first capture into every lightweight productivity domain.

use bruhs_models::{
    CalendarEvent, CaptureKind, CaptureRecord, EventSource, InboxConversation, InboxParticipant,
    InboxSource, KnowledgeDocument, PersonProfile, WorkItem,
};
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::KeyboardEvent;

use crate::state::{
    CaptureEditId, CaptureOpen, CapturesSignal, SetCalendarEventsSignal, SetCaptureEditId,
    SetCaptureOpen, SetCapturesSignal, SetInboxConversationsSignal, SetKnowledgeDocumentsSignal,
    SetPeopleSignal, SetWorkItemsSignal,
};
use crate::storage;
use crate::ui::{Button, ButtonSize, ButtonVariant, Dialog, Icon, Textarea};

const HALF_HOUR_MS: f64 = 30.0 * 60.0 * 1000.0;
const EVENT_DURATION_MS: f64 = 60.0 * 60.0 * 1000.0;

fn capture_icon(kind: CaptureKind) -> &'static str {
    match kind {
        CaptureKind::Task => "list-checks",
        CaptureKind::Note => "file-text",
        CaptureKind::Event => "calendar",
        CaptureKind::Contact => "user-plus",
        CaptureKind::MessageDraft => "send",
    }
}

fn next_half_hour_ms(now_ms: f64) -> f64 {
    (now_ms / HALF_HOUR_MS).ceil() * HALF_HOUR_MS
}

fn capture_task(record: &CaptureRecord) -> WorkItem {
    let mut item = WorkItem::new_at(&record.title, record.created_at_ms);
    item.updated_at = record.updated_at_ms;
    if !record.body.is_empty() {
        item.description = Some(record.body.clone());
    }
    item
}

fn capture_event(record: &CaptureRecord, now_ms: f64) -> CalendarEvent {
    let start_at_ms = next_half_hour_ms(now_ms);
    CalendarEvent {
        id: uuid::Uuid::new_v4().to_string(),
        title: record.title.clone(),
        start_at_ms,
        end_at_ms: start_at_ms + EVENT_DURATION_MS,
        all_day: false,
        color: Some("persimmon".to_string()),
        notes: (!record.body.is_empty()).then(|| record.body.clone()),
        created_at: record.created_at_ms,
        updated_at: record.updated_at_ms,
        source: EventSource::Local,
        google_event_id: None,
        calendar_id: None,
        attendees: Vec::new(),
        organizer: None,
        location: None,
        conferencing: None,
        recurring_event_id: None,
        html_link: None,
    }
}

fn capture_contact(record: &CaptureRecord) -> PersonProfile {
    let mut person = PersonProfile::new(&record.title);
    person.created_at = record.created_at_ms;
    person.updated_at = record.updated_at_ms;
    person
}

fn capture_document(record: &CaptureRecord) -> KnowledgeDocument {
    let mut document = KnowledgeDocument::new(
        record.title.clone(),
        record.body.clone(),
        record.created_at_ms,
    );
    document.updated_at_ms = record.updated_at_ms;
    document
}

fn capture_message_draft(record: &CaptureRecord) -> InboxConversation {
    let mut conversation = InboxConversation::new(
        InboxSource::Import,
        record.title.clone(),
        vec![InboxParticipant {
            id: uuid::Uuid::new_v4().to_string(),
            display_name: "Recipient".to_string(),
            address: String::new(),
            person_id: None,
        }],
        record.created_at_ms,
    );
    conversation.upsert_draft(record.body.clone(), record.updated_at_ms);
    conversation
}

#[component]
pub fn CaptureDialog() -> impl IntoView {
    let CaptureOpen(open) = expect_context::<CaptureOpen>();
    let SetCaptureOpen(set_open) = expect_context::<SetCaptureOpen>();
    let CaptureEditId(edit_id) = expect_context::<CaptureEditId>();
    let SetCaptureEditId(set_edit_id) = expect_context::<SetCaptureEditId>();
    let CapturesSignal(captures) = expect_context::<CapturesSignal>();
    let SetCapturesSignal(set_captures) = expect_context::<SetCapturesSignal>();
    let SetWorkItemsSignal(set_work_items) = expect_context::<SetWorkItemsSignal>();
    let SetCalendarEventsSignal(set_events) = expect_context::<SetCalendarEventsSignal>();
    let SetPeopleSignal(set_people) = expect_context::<SetPeopleSignal>();
    let saves = expect_context::<crate::task_saves::TaskSaves>();
    let SetKnowledgeDocumentsSignal(set_documents) =
        expect_context::<SetKnowledgeDocumentsSignal>();
    let SetInboxConversationsSignal(set_conversations) =
        expect_context::<SetInboxConversationsSignal>();

    let kind = RwSignal::new(CaptureKind::Task);
    let content = RwSignal::new(String::new());
    let error = RwSignal::new(Option::<String>::None);

    Effect::new(move |_| {
        if !open.get() {
            return;
        }
        gloo_timers::callback::Timeout::new(0, || {
            if let Some(input) = web_sys::window()
                .and_then(|window| window.document())
                .and_then(|document| document.get_element_by_id("capture-input"))
                .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
            {
                let _ = input.focus();
            }
        })
        .forget();
        error.set(None);
        let current = edit_id.get().and_then(|id| {
            captures.with(|records| records.iter().find(|record| record.id == id).cloned())
        });
        if let Some(record) = current {
            kind.set(record.kind);
            content.set(record.content());
        } else {
            kind.set(CaptureKind::Task);
            content.set(String::new());
        }
    });

    let close = move || {
        set_open.set(false);
        set_edit_id.set(None);
        error.set(None);
    };

    let submit = Callback::new(move |_: ()| {
        let now_ms = crate::clock::now_ms();
        let Ok(mut record) =
            CaptureRecord::new(kind.get_untracked(), &content.get_untracked(), now_ms)
        else {
            error.set(Some("Capture text is required.".to_string()));
            return;
        };

        if let Some(id) = edit_id.get_untracked() {
            let mut saved = None;
            set_captures.update(|items| {
                if let Some(existing) = items.iter_mut().find(|item| item.id == id) {
                    let original = existing.clone();
                    existing.title = record.title.clone();
                    existing.body = record.body.clone();
                    existing.updated_at_ms = now_ms;
                    saved = Some((original, existing.clone()));
                }
            });
            if let Some((original, saved)) = saved {
                storage::persist_capture(Some(original), saved);
                close();
            }
            return;
        }

        let mut captured_task = None;
        let mut captured_inbox = None;
        let mut captured_event = None;
        let mut captured_document = None;
        let mut captured_person = None;
        match record.kind {
            CaptureKind::Task => {
                let item = capture_task(&record);
                record.target_id = Some(item.id.clone());
                set_work_items.update(|items| items.push(item.clone()));
                captured_task = Some(item);
            }
            CaptureKind::Event => {
                let event = capture_event(
                    &record,
                    bruhs_models::EpochMs::saturating_from_i64(now_ms).as_f64(),
                );
                record.target_id = Some(event.id.clone());
                set_events.update(|events| events.push(event.clone()));
                captured_event = Some(event);
            }
            CaptureKind::Contact => {
                let person = capture_contact(&record);
                record.target_id = Some(person.id.clone());
                set_people.update(|people| people.push(person.clone()));
                captured_person = Some(person);
            }
            CaptureKind::Note => {
                let document = capture_document(&record);
                record.target_id = Some(document.id.clone());
                set_documents.update(|documents| documents.insert(0, document.clone()));
                captured_document = Some(document);
            }
            CaptureKind::MessageDraft => {
                let conversation = capture_message_draft(&record);
                record.target_id = Some(conversation.id.clone());
                set_conversations.update(|items| items.insert(0, conversation.clone()));
                captured_inbox = Some(conversation);
            }
        }

        set_captures.update(|items| items.insert(0, record.clone()));
        if let Some(task) = captured_task {
            // The writer announces success only after the linked pair commits.
            saves.capture_task(record, task);
        } else if let Some(conversation) = captured_inbox {
            saves.capture_inbox(record, conversation);
        } else if let Some(event) = captured_event {
            saves.capture_event(record, event);
        } else if let Some(document) = captured_document {
            saves.capture_document(record, document);
        } else if let Some(person) = captured_person {
            saves.capture_person(record, person);
        } else {
            storage::persist_capture(None, record);
        }
        close();
    });

    let submit_click = Callback::new(move |_: web_sys::MouseEvent| submit.run(()));
    let close_callback = Callback::new(move |_: ()| close());
    let cancel_click = Callback::new(move |_: web_sys::MouseEvent| close());
    let on_keydown = move |event: KeyboardEvent| {
        if event.key() == "Enter" && (event.meta_key() || event.ctrl_key()) {
            event.prevent_default();
            submit.run(());
        }
    };

    view! {
        <Dialog
            open=Signal::derive(move || open.get())
            on_close=close_callback
            max_width="max-w-lg".to_string()
            label="Capture anything"
        >
            <div class="border-b border-longan-700/70 px-4 py-3">
                <div class="flex items-baseline justify-between gap-3">
                    <h2 class="text-h4 text-rambutan-100">
                        {move || if edit_id.get().is_some() { "Edit capture" } else { "Capture anything" }}
                    </h2>
                    <span class="text-label-sm text-rambutan-500">"⌘↵ save"</span>
                </div>
            </div>

            <div class="px-4 py-3 space-y-3">
                <div class="grid grid-cols-5 gap-1 rounded-grape bg-longan-900/50 p-1 inset-ring inset-ring-longan-700/70" role="radiogroup" aria-label="Capture type">
                    {CaptureKind::ALL.into_iter().map(|option| {
                        let icon = capture_icon(option);
                        view! {
                            <button
                                type="button"
                                role="radio"
                                disabled=move || edit_id.get().is_some()
                                aria-checked=move || if kind.get() == option { "true" } else { "false" }
                                class=move || format!(
                                    "flex min-w-0 flex-col items-center gap-1 rounded-seed px-1 py-2 text-label-sm transition-colors cursor-pointer focus-visible:outline-2 focus-visible:outline-persimmon-400 {}",
                                    if kind.get() == option {
                                        "bg-longan-700 text-rambutan-100 inset-ring inset-ring-white/10"
                                    } else if edit_id.get().is_some() {
                                        "text-rambutan-500 cursor-not-allowed"
                                    } else {
                                        "text-rambutan-500 hover:bg-longan-800 hover:text-rambutan-200"
                                    }
                                )
                                on:click=move |_| {
                                    kind.set(option);
                                    error.set(None);
                                }
                            >
                                <Icon name=icon size=14 />
                                <span class="truncate max-w-full">{option.label()}</span>
                            </button>
                        }
                    }).collect_view()}
                </div>

                <div class="space-y-1.5">
                    <label class="text-label-sm text-rambutan-400">
                        {move || kind.get().label()}
                    </label>
                    <div on:keydown=on_keydown>
                        <Textarea
                            id="capture-input".to_string()
                            value=Signal::derive(move || content.get())
                            on_input=Callback::new(move |value| {
                                content.set(value);
                                error.set(None);
                            })
                            placeholder="Start with a concise title, then add details on the next line…".to_string()
                            rows=5
                            autofocus=true
                        />
                    </div>
                    <Show when=move || error.get().is_some()>
                        <p class="text-label-sm text-lychee-400" role="alert">
                            {move || error.get().unwrap_or_default()}
                        </p>
                    </Show>
                    <p class="text-label-sm text-rambutan-500">
                        {move || match kind.get() {
                            CaptureKind::Task => "Creates a backlog task immediately.",
                            CaptureKind::Note => "Creates a local Markdown document you can link and cite.",
                            CaptureKind::Event => "Schedules the next available half-hour for one hour.",
                            CaptureKind::Contact => "Adds a private contact and syncs when the server is available.",
                            CaptureKind::MessageDraft => "Saved locally until you choose where to send it.",
                        }}
                    </p>
                </div>
            </div>

            <div class="flex items-center justify-end gap-2 border-t border-longan-700/70 bg-longan-900/40 px-4 py-3">
                <Button variant=ButtonVariant::Ghost size=ButtonSize::Default on_click=cancel_click>
                    "Cancel"
                </Button>
                <Button
                    variant=ButtonVariant::Primary
                    size=ButtonSize::Default
                    on_click=submit_click
                    disabled=Signal::derive(move || content.get().trim().is_empty())
                >
                    {move || if edit_id.get().is_some() { "Save" } else { "Capture" }}
                </Button>
            </div>
        </Dialog>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(kind: CaptureKind) -> CaptureRecord {
        CaptureRecord::new(kind, "Launch plan\nReview the rollout", 100).expect("valid capture")
    }

    #[test]
    fn capture_task_should_preserve_title_body_and_timestamp() {
        let item = capture_task(&record(CaptureKind::Task));

        assert_eq!(item.title, "Launch plan");
        assert_eq!(item.description.as_deref(), Some("Review the rollout"));
        assert_eq!(item.created_at, 100);
        assert_eq!(item.updated_at, 100);
    }

    #[test]
    fn capture_task_retains_distinct_creation_and_update_times() {
        let mut record = record(CaptureKind::Task);
        record.updated_at_ms = 250;
        let item = capture_task(&record);
        assert_eq!(item.created_at, 100);
        assert_eq!(item.updated_at, 250);
    }

    #[test]
    fn capture_event_should_round_up_and_last_one_hour() {
        let event = capture_event(&record(CaptureKind::Event), 1_000_001.0);

        assert_eq!(event.start_at_ms % HALF_HOUR_MS, 0.0);
        assert_eq!(event.end_at_ms - event.start_at_ms, EVENT_DURATION_MS);
    }

    #[test]
    fn capture_contact_should_preserve_client_generated_timestamps() {
        let person = capture_contact(&record(CaptureKind::Contact));

        assert_eq!(person.display_name, "Launch plan");
        assert_eq!(person.created_at, 100);
    }
}
