//! Provider-neutral, local-first communication triage.

use bruhs_models::{
    inbox_agent_prompt, unique_person_named, InboxCard, InboxConversation, InboxDirectory,
    InboxFilter, InboxMessage, InboxParticipant, InboxSource, MessageDirection, MessageSendState,
    DELETE_CONVERSATION_DETAIL, DELETE_CONVERSATION_TITLE, INBOX_PAGE_SIZE, REPLY_HINT,
};
use leptos::prelude::*;

use crate::state::{
    AppView, ComposerDraft, InboxBrowseState, InboxConversationsSignal, InboxCreateRequest,
    PeopleSignal, SetActiveView, SetInboxConversationsSignal,
};
use crate::storage;
use crate::ui::{
    button_class, ButtonSize, ButtonVariant, Dialog, Dropdown, DropdownOption, Input, Textarea,
};

#[component]
pub fn InboxPage(#[prop(optional)] conversation_id: Option<String>) -> impl IntoView {
    let executor = expect_context::<crate::workflow_executor::WorkflowExecutor>();
    let mail = expect_context::<crate::state::MailState>();
    let content = match conversation_id {
        Some(id) => view! { <ConversationDetail conversation_id=id /> }.into_any(),
        None => view! {
            <div class="h-full min-h-0">
                <div class="h-full min-h-0" class:hidden=move || mail.visible.get()><InboxLibrary /></div>
                <Show when=move || mail.visible.get()><super::connected_inbox::ConnectedInbox /></Show>
            </div>
        }.into_any(),
    };
    view! {
        <div class="flex h-full min-h-0 flex-col">
            <Show when=move || executor.load_error.get().is_some()>
                <section aria-label="Inbox loading status" class="shrink-0 border-b border-longan-800 px-4 py-3 text-body-sm text-rambutan-300">
                    <p role="alert">"Local records could not be loaded. Existing storage was left unchanged; conversation changes made here are kept."</p>
                    <button type="button" class="bruhs-control mt-2 min-h-control rounded-grape bg-longan-800 px-3 py-2 text-label text-rambutan-200 hover:bg-longan-700 disabled:opacity-40 cursor-pointer"
                        disabled=move || executor.loading.get()
                        on:click=move |_| executor.load()>"Retry loading records"</button>
                </section>
            </Show>
            <div class="min-h-0 flex-1">{content}</div>
        </div>
    }
}

#[component]
fn InboxLibrary() -> impl IntoView {
    let mail = expect_context::<crate::state::MailState>();
    let executor = expect_context::<crate::workflow_executor::WorkflowExecutor>();
    let InboxConversationsSignal(conversations) = expect_context::<InboxConversationsSignal>();
    let SetInboxConversationsSignal(set_conversations) =
        expect_context::<SetInboxConversationsSignal>();
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();
    let InboxCreateRequest(create_request) = expect_context::<InboxCreateRequest>();
    let PeopleSignal(people) = expect_context::<PeopleSignal>();

    let InboxBrowseState {
        filter,
        query,
        page,
    } = expect_context::<InboxBrowseState>();
    let import_open = RwSignal::new(false);
    let source = RwSignal::new(InboxSource::Email);
    let sender = RwSignal::new(String::new());
    let subject = RwSignal::new(String::new());
    let body = RwSignal::new(String::new());
    let error = RwSignal::new(Option::<String>::None);

    Effect::new(move |_| {
        if create_request.take() {
            mail.visible.set(false);
            import_open.set(true);
        }
    });

    let directory = Memo::new(move |_| conversations.with(|items| InboxDirectory::new(items)));
    let matching = Memo::new(move |_| {
        directory.with(|directory| {
            directory.matching(&query.get(), filter.get(), crate::clock::now_ms())
        })
    });
    let visible = Memo::new(move |_| {
        directory.with(|directory| matching.with(|matches| directory.page(matches, page.get())))
    });
    let counts =
        Memo::new(move |_| directory.with(|directory| directory.counts(crate::clock::now_ms())));

    let save_import = move || {
        let sender_value = sender.get_untracked();
        let body_value = body.get_untracked();
        if sender_value.trim().is_empty() || body_value.trim().is_empty() {
            error.set(Some("A sender and message are required.".to_string()));
            return;
        }
        let now = crate::clock::now_ms();
        let display_name = sender_value.trim().to_string();
        // Like desktop: display names and aliases, Unicode case folded; a
        // name two profiles answer to links to neither.
        let person_id = people.with_untracked(|people| {
            unique_person_named(people, &display_name).map(|person| person.id.clone())
        });
        let participant = InboxParticipant {
            id: uuid::Uuid::new_v4().to_string(),
            display_name,
            address: String::new(),
            person_id,
        };
        let mut conversation = InboxConversation::new(
            source.get_untracked(),
            subject.get_untracked(),
            vec![participant],
            now,
        );
        conversation.receive(InboxMessage::incoming(sender_value, body_value, now));
        let id = conversation.id.clone();
        set_conversations.update(|items| items.insert(0, conversation.clone()));
        storage::persist_inbox_conversation(None, conversation);
        sender.set(String::new());
        subject.set(String::new());
        body.set(String::new());
        error.set(None);
        import_open.set(false);
        set_active_view.set(AppView::ConversationDetail {
            conversation_id: id,
        });
    };

    view! {
        <div class="flex h-full flex-col bg-longan-950 text-rambutan-300">
            <crate::ui::PageHeader title="Inbox" subtitle="Imported conversations and local reply drafts.">
                <crate::ui::Button variant=crate::ui::ButtonVariant::Outline size=crate::ui::ButtonSize::Small on_click=Callback::new(move |_| mail.open())>
                    "Email"
                </crate::ui::Button>
                <crate::ui::Button size=crate::ui::ButtonSize::Small on_click=Callback::new(move |_| import_open.set(true))>
                    "Import conversation"
                </crate::ui::Button>
            </crate::ui::PageHeader>
            <div class="min-h-0 flex-1 overflow-auto">
            <div class="mx-auto w-full max-w-5xl space-y-5 px-4 py-5 sm:px-6">

                <div class="flex flex-col gap-3">
                    <div class="min-w-0 flex-1"><Input aria_label="Search inbox" input_type="search".to_string() placeholder="Search people, subjects, and messages…".to_string() value=Signal::derive(move || query.get()) on_input=Callback::new(move |value| { query.set(value); page.set(0); }) /></div>
                    <div class="flex flex-wrap gap-1" role="group" aria-label="Inbox filter">
                        {InboxFilter::ALL.into_iter().map(|value| view! { <FilterButton value filter page /> }).collect_view()}
                    </div>
                </div>

                <section aria-label="Conversations" class="overflow-hidden rounded-grape bg-longan-900/45 inset-ring inset-ring-longan-800">
                    <Show when=move || !matching.with(Vec::is_empty) fallback=move || {
                        if !executor.ready.get() {
                            view! { <div class="px-5 py-14 text-center"><p role="status" class="text-label text-rambutan-300">{move || if executor.load_error.get().is_some() { "Conversations could not be loaded" } else { "Loading conversations…" }}</p><p class="mt-1 text-body-sm text-rambutan-500">{move || if executor.load_error.get().is_some() { "Choose Retry loading records above. Stored conversations were left unchanged." } else { "Your conversations will appear after local records load." }}</p></div> }.into_any()
                        } else {
                            view! { <div class="px-5 py-14 text-center"><p role="status" class="text-label text-rambutan-300">{move || if !query.get().trim().is_empty() { "No matching conversations." } else { filter.get().empty_message() }}</p><p class="mt-1 text-body-sm text-rambutan-500">{move || if !query.get().trim().is_empty() { "Try another search or choose All." } else { "Import a message or choose another view." }}</p></div> }.into_any()
                        }
                    }>
                        <For each=move || visible.get() key=|card| card.clone()
                            children=move |card| view! { <InboxRow card /> } />
                    </Show>
                </section>
                <crate::components::task_pagination::TaskPagination
                    total=Signal::derive(move || matching.with(Vec::len)) page page_size=INBOX_PAGE_SIZE label="Inbox pages" />

                <div class="flex flex-wrap gap-x-5 gap-y-1 text-label-sm text-rambutan-400" role="status" aria-label="Inbox counts">
                    <span>{move || format!("{} open", counts.get().open)}</span>
                    <span>{move || format!("{} unread", counts.get().unread)}</span>
                    <span>{move || format!("{} need reply", counts.get().needs_reply)}</span>
                    <span>{move || format!("{} snoozed", counts.get().snoozed)}</span>
                </div>
                <p class="text-label-sm text-rambutan-400">"Imported messages and replies stay on this device. Choose Email to read and sync connected Gmail."</p>
            </div>
            </div>
        </div>

        <Show when=move || import_open.get()>
            <Dialog open=Signal::derive(move || import_open.get()) label="Import conversation" on_close=Callback::new(move |_| import_open.set(false))>
                <div class="border-b border-longan-700/70 px-4 py-3"><h2 class="text-h4 text-rambutan-100">"Import conversation"</h2><p class="mt-1 text-body-sm text-rambutan-500">"Paste a message from any provider. It stays available offline."</p></div>
                <form class="max-h-[calc(100dvh-8rem)] overflow-y-auto space-y-3 px-4 py-3" on:submit=move |event| { event.prevent_default(); save_import(); }>
                    <div class="space-y-1"><label class="text-label-sm text-rambutan-400">"Provider"</label><div class="flex flex-wrap gap-1">{[InboxSource::Email, InboxSource::Slack, InboxSource::Discord, InboxSource::IMessage, InboxSource::X, InboxSource::Import].into_iter().map(|item| view! { <button type="button" aria-pressed=move || (source.get() == item).to_string() class=move || if source.get() == item { "bruhs-control min-h-[32px] rounded-seed bg-blueberry-500/15 px-2.5 py-1.5 text-label-sm text-blueberry-400 inset-ring inset-ring-blueberry-500/30 cursor-pointer" } else { "bruhs-control min-h-[32px] rounded-seed px-2.5 py-1.5 text-label-sm text-rambutan-400 inset-ring inset-ring-longan-700 hover:bg-longan-800 cursor-pointer" } on:click=move |_| source.set(item)>{item.label()}</button> }).collect_view()}</div></div>
                    <div class="space-y-1"><label class="text-label-sm text-rambutan-400">"Sender"</label><Input aria_label="Sender" value=Signal::derive(move || sender.get()) on_input=Callback::new(move |value| sender.set(value)) autofocus=true placeholder="Name or handle".to_string() /></div>
                    <div class="space-y-1"><label class="text-label-sm text-rambutan-400">"Subject"</label><Input aria_label="Subject" value=Signal::derive(move || subject.get()) on_input=Callback::new(move |value| subject.set(value)) placeholder="Optional".to_string() /></div>
                    <div class="space-y-1"><label class="text-label-sm text-rambutan-400">"Message"</label><Textarea aria_label="Message" value=Signal::derive(move || body.get()) on_input=Callback::new(move |value| body.set(value)) rows=5 placeholder="Paste the message…".to_string() /></div>
                    <Show when=move || error.get().is_some()>{move || error.get().map(|message| view! { <p role="alert" class="text-body-sm text-lychee-300">{message}</p> })}</Show>
                    <div class="flex justify-end gap-2"><button type="button" class="bruhs-control min-h-[32px] rounded-grape px-3 py-2 text-label text-rambutan-300 hover:bg-longan-800 cursor-pointer" on:click=move |_| import_open.set(false)>"Cancel"</button><button type="submit" class="bruhs-control min-h-[32px] rounded-grape bg-persimmon-500 px-3 py-2 text-label text-longan-950 hover:bg-persimmon-400 cursor-pointer">"Import"</button></div>
                </form>
            </Dialog>
        </Show>
    }
}

#[component]
fn InboxRow(card: InboxCard) -> impl IntoView {
    let set_active_view = expect_context::<SetActiveView>().0;
    let (date, full_date) = inbox_date(card.message_at_ms);
    let id = card.id;
    view! {
        <button type="button" data-inbox-row="" class="group flex w-full items-start gap-3 border-b border-longan-800/80 px-3 py-3 text-left last:border-b-0 hover:bg-longan-800/60 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-persimmon-400 cursor-pointer" on:click=move |_| set_active_view.set(AppView::ConversationDetail { conversation_id: id.clone() })>
            <span aria-hidden="true" class=if card.unread { "mt-2 size-2 shrink-0 rounded-full bg-blueberry-400" } else { "mt-2 size-2 shrink-0 rounded-full bg-longan-700" }></span>
            <span class="sr-only">{if card.unread { "Unread conversation: " } else { "Read conversation: " }}</span>
            <span class="min-w-0 flex-1">
                <span class="flex items-baseline justify-between gap-3">
                    <span class=if card.unread { "truncate text-label font-semibold text-rambutan-100" } else { "truncate text-label text-rambutan-300" }>{card.sender}</span>
                    <span class="shrink-0 text-label-sm tabular-nums text-rambutan-500" title=full_date>{date}</span>
                </span>
                <span class="mt-0.5 flex items-center gap-2"><span class="truncate text-body-sm text-rambutan-300">{if card.subject.trim().is_empty() { "(No subject)".to_string() } else { card.subject }}</span><Show when=move || card.important><span class="shrink-0 text-label-sm text-rambutan-300">"Important"</span></Show></span>
                <span class="mt-0.5 block truncate text-body-sm text-rambutan-500">{card.preview}</span>
                <span class="mt-1 block text-label-sm text-rambutan-500">{format!("{} import", card.source.label())}</span>
            </span>
        </button>
    }
}

pub(super) fn inbox_date(timestamp_ms: i64) -> (String, String) {
    let Ok(timestamp) = bruhs_models::EpochMs::new(timestamp_ms) else {
        return ("Unknown date".into(), "Unknown date".into());
    };
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(timestamp.as_f64()));
    if !date.get_time().is_finite() {
        return ("Unknown date".into(), "Unknown date".into());
    }
    let day = format!(
        "{:04}-{:02}-{:02}",
        date.get_full_year(),
        date.get_month() + 1,
        date.get_date()
    );
    let time = format!("{:02}:{:02}", date.get_hours(), date.get_minutes());
    let now = js_sys::Date::new_0();
    let today = date.get_full_year() == now.get_full_year()
        && date.get_month() == now.get_month()
        && date.get_date() == now.get_date();
    (
        if today { time.clone() } else { day.clone() },
        format!("{day} {time}"),
    )
}

#[component]
fn FilterButton(
    value: InboxFilter,
    filter: RwSignal<InboxFilter>,
    page: RwSignal<usize>,
) -> impl IntoView {
    view! { <button type="button" aria-pressed=move || (filter.get() == value).to_string() class=move || if filter.get() == value { "bruhs-control min-h-[32px] rounded-seed bg-blueberry-500/15 px-2.5 py-1.5 text-label-sm text-blueberry-400 inset-ring inset-ring-blueberry-500/30 cursor-pointer" } else { "bruhs-control min-h-[32px] rounded-seed px-2.5 py-1.5 text-label-sm text-rambutan-400 hover:bg-longan-800 cursor-pointer" } on:click=move |_| { filter.set(value); page.set(0); }>{value.label()}</button> }
}

#[component]
fn ConversationDetail(conversation_id: String) -> impl IntoView {
    let executor = expect_context::<crate::workflow_executor::WorkflowExecutor>();
    let InboxConversationsSignal(conversations) = expect_context::<InboxConversationsSignal>();
    let SetInboxConversationsSignal(set_conversations) =
        expect_context::<SetInboxConversationsSignal>();
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();
    let ComposerDraft(composer_draft) = expect_context::<ComposerDraft>();
    let PeopleSignal(people) = expect_context::<PeopleSignal>();
    let id = StoredValue::new(conversation_id);
    let draft = RwSignal::new(String::new());
    let initialized = RwSignal::new(false);
    // Deleting asks first, as on desktop: the conversation and its messages
    // are gone for good.
    let pending_delete = RwSignal::new(false);
    let confirm_delete = move || {
        pending_delete.set(false);
        let conversation_id = id.get_value();
        let original = conversations.with_untracked(|items| {
            items
                .iter()
                .find(|item| item.id == conversation_id)
                .cloned()
        });
        set_conversations.update(|items| items.retain(|item| item.id != conversation_id));
        if let Some(original) = original {
            storage::forget_inbox_conversation(original);
        }
        set_active_view.set(AppView::Inbox);
    };

    Effect::new(move |_| {
        if initialized.get() {
            return;
        }
        let selected = conversations
            .with(|items| items.iter().find(|item| item.id == id.get_value()).cloned());
        if let Some(conversation) = selected {
            initialized.set(true);
            draft.set(conversation.draft_body().to_string());
            if conversation.unread {
                update_conversation(set_conversations, &conversation.id, |item| {
                    item.mark_read(true)
                });
            }
        }
    });

    let save_draft = move || {
        let body = draft.get_untracked();
        update_conversation(set_conversations, &id.get_value(), |item| {
            item.upsert_draft(body, crate::clock::now_ms())
        });
    };
    let send = move || {
        let body = draft.get_untracked();
        update_conversation(set_conversations, &id.get_value(), |item| {
            item.upsert_draft(body, crate::clock::now_ms());
            item.send_draft(crate::clock::now_ms());
        });
        draft.set(String::new());
    };

    view! {
        {move || conversations.with(|items| items.iter().find(|item| item.id == id.get_value()).cloned()).map_or_else(
            || view! { <div class="flex h-full items-center justify-center bg-longan-950"><div class="text-center"><p role="status" class="text-label text-rambutan-300">{move || if executor.ready.get() { "Conversation not found" } else if executor.load_error.get().is_some() { "Conversation could not be loaded" } else { "Loading conversation…" }}</p><button type="button" class="mt-3 text-label text-persimmon-300 cursor-pointer" on:click=move |_| set_active_view.set(AppView::Inbox)>"Back to Inbox"</button></div></div> }.into_any(),
            |conversation| {
                let conversation_for_agent = conversation.clone();
                let archived = conversation.archived;
                let unread = conversation.unread;
                let important = conversation.labels.iter().any(|label| label == "important");
                let snoozed = conversation.snoozed_until_ms.is_some_and(|at| at > crate::clock::now_ms());
                view! {
                    <div class="flex h-full min-h-0 flex-col overflow-y-auto bg-longan-950 text-rambutan-300" data-inbox-detail="">
                        <header class="flex shrink-0 flex-wrap items-center gap-2 border-b border-longan-800 px-3 py-2.5 sm:px-5">
                            <button type="button" aria-label="Back to Inbox" class="bruhs-control min-h-[32px] rounded-seed px-2 py-1.5 text-label text-rambutan-400 hover:bg-longan-800 cursor-pointer" on:click=move |_| set_active_view.set(AppView::Inbox)>"← Inbox"</button>
                            <crate::components::people_page::BackToPeople />
                            <div class="min-w-0 flex-1"><h1 class="truncate text-h4 text-rambutan-100">{conversation.subject.clone()}</h1><p class="truncate text-label-sm text-rambutan-500">{conversation.source_and_participants()}</p></div>
                            <div class="flex flex-wrap gap-1">
                                <ActionButton label=if unread { "Mark read" } else { "Mark unread" } on_click=Callback::new(move |_| update_conversation(set_conversations, &id.get_value(), |item| item.mark_read(unread))) />
                                <ActionButton label=if snoozed { "Unsnooze" } else { "Tomorrow" } on_click=Callback::new(move |_| update_conversation(set_conversations, &id.get_value(), |item| item.snooze_until(if snoozed { None } else { Some(crate::clock::now_ms() + 86_400_000) }))) />
                                <ActionButton label=if important { "Unstar" } else { "Important" } on_click=Callback::new(move |_| update_conversation(set_conversations, &id.get_value(), |item| item.toggle_label("important"))) />
                                <ActionButton label=if archived { "Unarchive" } else { "Archive" } on_click=Callback::new(move |_| update_conversation(set_conversations, &id.get_value(), |item| item.set_archived(!archived))) />
                                <ActionButton label="Ask agent" on_click=Callback::new(move |_| { composer_draft.set(inbox_agent_prompt(&conversation_for_agent)); set_active_view.set(AppView::Chat); }) />
                                <ActionButton label="Delete" on_click=Callback::new(move |_| pending_delete.set(true)) />
                            </div>
                        </header>
                        <div class="flex shrink-0 flex-wrap items-center justify-between gap-3 border-b border-longan-800/70 px-3 py-2 sm:px-5">
                            <p class="text-label-sm text-rambutan-500">"Linked person"</p>
                            <Dropdown
                                options=Signal::derive(move || std::iter::once(DropdownOption::new("", "Unlinked")).chain(people.get().into_iter().map(|person| DropdownOption::new(person.id, person.display_name))).collect())
                                value=Signal::derive(move || conversations.with(|items| items.iter().find(|item| item.id == id.get_value()).and_then(|item| item.linked_person_id().map(str::to_string))).unwrap_or_default())
                                on_change=Callback::new(move |person_id: String| update_conversation(set_conversations, &id.get_value(), |item| { item.link_person(Some(person_id)); }))
                                label="Linked person"
                                class="w-full sm:w-48"
                            />
                        </div>
                        <section aria-label="Conversation messages" data-inbox-history="" class="min-h-[10rem] flex-1 overflow-auto"><div class="mx-auto max-w-3xl space-y-3 px-4 py-5">{conversation.messages.into_iter().filter(|message| message.send_state != MessageSendState::Draft).map(|message| {
                            let outgoing = message.direction == MessageDirection::Outgoing;
                            let (date, full_date) = inbox_date(message.sent_at_ms);
                            view! { <article class=move || if outgoing { "ml-auto min-w-0 max-w-[88%] rounded-grape rounded-br-seed bg-longan-800 px-3 py-2.5 inset-ring inset-ring-longan-700" } else { "mr-auto min-w-0 max-w-[88%] rounded-grape rounded-bl-seed bg-longan-900 px-3 py-2.5 inset-ring inset-ring-longan-800" }><div class="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1 text-label-sm text-rambutan-500"><p class="min-w-0 break-words">{message.sender_name}</p><span class="shrink-0 tabular-nums" title=full_date>{date}</span></div><p class="mt-1 whitespace-pre-wrap break-words [overflow-wrap:anywhere] text-body text-rambutan-200">{message.body}</p></article> }
                        }).collect_view()}</div></section>
                        <form class="shrink-0 border-t border-longan-800 bg-longan-950 px-3 py-3 sm:px-5" on:submit=move |event| { event.prevent_default(); send(); }>
                            <div class="mx-auto max-w-3xl space-y-2"><Textarea aria_label="Reply draft" id="inbox-reply".to_string() value=Signal::derive(move || draft.get()) on_input=Callback::new(move |value| draft.set(value)) rows=3 placeholder="Write a reply…".to_string() on_keydown=Callback::new(move |event: web_sys::KeyboardEvent| { if (event.meta_key() || event.ctrl_key()) && event.key() == "Enter" { event.prevent_default(); send(); } }) /><div class="flex flex-wrap items-center justify-between gap-3"><p class="text-label-sm text-rambutan-400">{REPLY_HINT}</p><div class="flex gap-2"><button type="button" class="bruhs-control min-h-[32px] rounded-grape px-3 py-2 text-label text-rambutan-300 inset-ring inset-ring-longan-700 hover:bg-longan-800 cursor-pointer" on:click=move |_| save_draft()>"Save draft"</button><button type="submit" disabled=move || draft.get().trim().is_empty() class="bruhs-control min-h-[32px] rounded-grape bg-persimmon-500 px-3 py-2 text-label text-longan-950 hover:bg-persimmon-400 disabled:cursor-not-allowed disabled:opacity-50 cursor-pointer">"Log reply"</button></div></div></div>
                        </form>
                    </div>
                }.into_any()
            }
        )}
        <Show when=move || pending_delete.get()>
            <Dialog open=Signal::derive(move || pending_delete.get()) on_close=Callback::new(move |_| pending_delete.set(false)) max_width="max-w-sm" label=DELETE_CONVERSATION_TITLE>
                <div class="space-y-4 p-4" data-inbox-delete-confirm="">
                    <h2 class="text-h4 text-rambutan-100">{DELETE_CONVERSATION_TITLE}</h2>
                    <p class="text-body-sm text-rambutan-300">{DELETE_CONVERSATION_DETAIL}</p>
                    <div class="flex justify-end gap-2">
                        <button type="button" aria-label="Cancel deletion" class=button_class(ButtonVariant::Ghost, ButtonSize::Small, "cursor-pointer") on:click=move |_| pending_delete.set(false)>"Cancel"</button>
                        <button type="button" aria-label="Confirm deletion" class=button_class(ButtonVariant::Destructive, ButtonSize::Small, "cursor-pointer") on:click=move |_| confirm_delete()>"Delete"</button>
                    </div>
                </div>
            </Dialog>
        </Show>
    }
}

#[component]
fn ActionButton(label: &'static str, on_click: Callback<web_sys::MouseEvent>) -> impl IntoView {
    view! { <button type="button" class="bruhs-control min-h-[32px] rounded-seed px-2 py-1.5 text-label-sm text-rambutan-400 inset-ring inset-ring-longan-700 hover:bg-longan-800 hover:text-rambutan-200 cursor-pointer" on:click=move |event| on_click.run(event)>{label}</button> }
}

fn update_conversation(
    set_conversations: WriteSignal<Vec<InboxConversation>>,
    id: &str,
    update: impl FnOnce(&mut InboxConversation),
) {
    let mut changed = None;
    set_conversations.update(|items| {
        if let Some(conversation) = items.iter_mut().find(|item| item.id == id) {
            let original = conversation.clone();
            update(conversation);
            if *conversation != original {
                changed = Some((original, conversation.clone()));
            }
        }
    });
    if let Some((original, conversation)) = changed {
        storage::persist_inbox_conversation(Some(original), conversation);
    }
}
