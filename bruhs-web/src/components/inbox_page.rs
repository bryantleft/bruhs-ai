//! Provider-neutral, local-first communication triage.

use bruhs_models::{
    inbox_agent_prompt, InboxConversation, InboxMessage, InboxParticipant, InboxSource,
    MessageDirection, MessageSendState,
};
use leptos::prelude::*;

use crate::state::{
    AppView, ComposerDraft, InboxConversationsSignal, InboxCreateRequest, PeopleSignal,
    SetActiveView, SetInboxConversationsSignal,
};
use crate::storage;
use crate::ui::{Dialog, Dropdown, DropdownOption, Input, Textarea};

#[derive(Clone, Copy, PartialEq, Eq)]
enum InboxFilter {
    Inbox,
    Unread,
    Snoozed,
    Archived,
    All,
}

#[component]
pub fn InboxPage(#[prop(optional)] conversation_id: Option<String>) -> impl IntoView {
    let executor = expect_context::<crate::workflow_executor::WorkflowExecutor>();
    let content = match conversation_id {
        Some(id) => view! { <ConversationDetail conversation_id=id /> }.into_any(),
        None => view! { <InboxLibrary /> }.into_any(),
    };
    view! {
        <div class="flex h-full min-h-0 flex-col">
            <Show when=move || executor.load_error.get().is_some()>
                <section aria-label="Inbox loading status" class="shrink-0 border-b border-longan-800 px-4 py-3 text-body-sm text-rambutan-300">
                    <p role="alert">"Local records could not be loaded. Existing storage was left unchanged; conversation changes made here are kept."</p>
                    <button type="button" class="mt-2 min-h-11 rounded-grape bg-longan-800 px-3 py-2 text-label text-rambutan-200 hover:bg-longan-700 disabled:opacity-40 cursor-pointer"
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
    let executor = expect_context::<crate::workflow_executor::WorkflowExecutor>();
    let InboxConversationsSignal(conversations) = expect_context::<InboxConversationsSignal>();
    let SetInboxConversationsSignal(set_conversations) =
        expect_context::<SetInboxConversationsSignal>();
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();
    let InboxCreateRequest(create_request) = expect_context::<InboxCreateRequest>();

    let filter = RwSignal::new(InboxFilter::Inbox);
    let query = RwSignal::new(String::new());
    let import_open = RwSignal::new(false);
    let source = RwSignal::new(InboxSource::Email);
    let sender = RwSignal::new(String::new());
    let subject = RwSignal::new(String::new());
    let body = RwSignal::new(String::new());
    let error = RwSignal::new(Option::<String>::None);

    Effect::new(move |_| {
        if create_request.take() {
            import_open.set(true);
        }
    });

    let visible = Memo::new(move |_| {
        let now = crate::clock::now_ms();
        let needle = query.get().trim().to_lowercase();
        let filter = filter.get();
        let mut items = conversations.with(|all| {
            all.iter()
                .filter(|conversation| {
                    let in_filter = match filter {
                        InboxFilter::Inbox => conversation.is_visible_in_inbox(now),
                        InboxFilter::Unread => conversation.unread && !conversation.archived,
                        InboxFilter::Snoozed => {
                            conversation.snoozed_until_ms.is_some_and(|at| at > now)
                        }
                        InboxFilter::Archived => conversation.archived,
                        InboxFilter::All => true,
                    };
                    in_filter
                        && (needle.is_empty()
                            || conversation.subject.to_lowercase().contains(&needle)
                            || conversation.participants.iter().any(|participant| {
                                participant.display_name.to_lowercase().contains(&needle)
                                    || participant.address.to_lowercase().contains(&needle)
                            })
                            || conversation
                                .messages
                                .iter()
                                .any(|message| message.body.to_lowercase().contains(&needle)))
                })
                .cloned()
                .collect::<Vec<_>>()
        });
        items.sort_unstable_by_key(|conversation| std::cmp::Reverse(conversation.updated_at_ms));
        items
    });

    let save_import = move || {
        let sender_value = sender.get_untracked();
        let body_value = body.get_untracked();
        if sender_value.trim().is_empty() || body_value.trim().is_empty() {
            error.set(Some("A sender and message are required.".to_string()));
            return;
        }
        let now = crate::clock::now_ms();
        let participant = InboxParticipant {
            id: uuid::Uuid::new_v4().to_string(),
            display_name: sender_value.trim().to_string(),
            address: String::new(),
            person_id: None,
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
        <div class="h-full overflow-auto bg-longan-950 text-rambutan-300">
            <div class="mx-auto w-full max-w-5xl space-y-5 px-4 py-5 sm:px-6">
                <header class="flex flex-wrap items-start justify-between gap-3">
                    <div>
                        <p class="text-label-sm uppercase tracking-[0.16em] text-persimmon-300">"Inbox"</p>
                        <h1 class="mt-1 text-title text-rambutan-100">"Every conversation, one decisive queue"</h1>
                        <p class="mt-1 max-w-2xl text-body-sm text-rambutan-500">"Triage email, Slack, Discord, iMessage, and X in one fast local workflow. Import works now; connected adapters can sync into the same model."</p>
                    </div>
                    <button type="button" class="rounded-grape bg-persimmon-500 px-3 py-2 text-label text-longan-950 hover:bg-persimmon-400 cursor-pointer" on:click=move |_| import_open.set(true)>"+ Import conversation"</button>
                </header>

                <div class="flex flex-col gap-3 sm:flex-row sm:items-center">
                    <div class="min-w-0 flex-1"><Input aria_label="Search inbox" input_type="search".to_string() placeholder="Search people, subjects, and messages…".to_string() value=Signal::derive(move || query.get()) on_input=Callback::new(move |value| query.set(value)) /></div>
                    <div class="flex flex-wrap gap-1" role="group" aria-label="Inbox filter">
                        <FilterButton label="Inbox" value=InboxFilter::Inbox filter />
                        <FilterButton label="Unread" value=InboxFilter::Unread filter />
                        <FilterButton label="Snoozed" value=InboxFilter::Snoozed filter />
                        <FilterButton label="Archived" value=InboxFilter::Archived filter />
                        <FilterButton label="All" value=InboxFilter::All filter />
                    </div>
                </div>

                <div class="grid grid-cols-2 gap-2 sm:grid-cols-4">
                    <Metric label="Open" value=Signal::derive(move || conversations.with(|items| items.iter().filter(|item| item.is_visible_in_inbox(crate::clock::now_ms())).count())) />
                    <Metric label="Unread" value=Signal::derive(move || conversations.with(|items| items.iter().filter(|item| item.unread && !item.archived).count())) />
                    <Metric label="Needs reply" value=Signal::derive(move || conversations.with(|items| items.iter().filter(|item| item.needs_reply() && !item.archived).count())) />
                    <Metric label="Snoozed" value=Signal::derive(move || conversations.with(|items| items.iter().filter(|item| item.snoozed_until_ms.is_some_and(|at| at > crate::clock::now_ms())).count())) />
                </div>

                <section aria-label="Conversations" class="overflow-hidden rounded-grape bg-longan-900/45 inset-ring inset-ring-longan-800">
                    {move || {
                        let items = visible.get();
                        if items.is_empty() && !executor.ready.get() {
                            view! { <div class="px-5 py-14 text-center"><p role="status" class="text-label text-rambutan-300">{move || if executor.load_error.get().is_some() { "Conversations could not be loaded" } else { "Loading conversations…" }}</p><p class="mt-1 text-body-sm text-rambutan-500">"Your conversations will appear after local records load."</p></div> }.into_any()
                        } else if items.is_empty() {
                            view! { <div class="px-5 py-14 text-center"><p class="text-label text-rambutan-300">"Nothing waiting here."</p><p class="mt-1 text-body-sm text-rambutan-500">"A clear inbox is a very good place to be."</p></div> }.into_any()
                        } else {
                            view! { <div>{items.into_iter().map(|conversation| {
                                let id = conversation.id.clone();
                                let participant = conversation.participants.first().map_or("Unknown", |item| item.display_name.as_str()).to_string();
                                let preview = conversation.last_message().map_or("Draft only", |message| message.body.as_str()).replace('\n', " ");
                                let important = conversation.labels.iter().any(|label| label == "important");
                                view! {
                                    <button type="button" class="group flex w-full items-start gap-3 border-b border-longan-800/80 px-3 py-3 text-left last:border-b-0 hover:bg-longan-800/60 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-persimmon-400 cursor-pointer" on:click=move |_| set_active_view.set(AppView::ConversationDetail { conversation_id: id.clone() })>
                                        <span class=move || if conversation.unread { "mt-2 size-2 shrink-0 rounded-full bg-persimmon-400" } else { "mt-2 size-2 shrink-0 rounded-full bg-longan-700" }></span>
                                        <span class="min-w-0 flex-1">
                                            <span class="flex items-baseline justify-between gap-3"><span class="truncate text-label text-rambutan-100">{participant}</span><span class="shrink-0 text-label-sm text-rambutan-500">{conversation.source.label()}</span></span>
                                            <span class="mt-0.5 flex items-center gap-2"><span class="truncate text-body-sm text-rambutan-300">{conversation.subject}</span><Show when=move || important><span class="shrink-0 text-label-sm text-persimmon-300">"important"</span></Show></span>
                                            <span class="mt-0.5 block truncate text-body-sm text-rambutan-500">{preview}</span>
                                        </span>
                                    </button>
                                }
                            }).collect_view()}</div> }.into_any()
                        }
                    }}
                </section>

                <p class="text-label-sm text-rambutan-400">"Adapter contract: Email · Slack · Discord · iMessage · X. Until an account is connected, share or paste a message here—the triage, drafts, and relationship links remain fully local."</p>
            </div>
        </div>

        <Show when=move || import_open.get()>
            <Dialog open=Signal::derive(move || import_open.get()) label="Import conversation" on_close=Callback::new(move |_| import_open.set(false))>
                <div class="border-b border-longan-700/70 px-4 py-3"><h2 class="text-h4 text-rambutan-100">"Import conversation"</h2><p class="mt-1 text-body-sm text-rambutan-500">"Paste a message from any provider. It stays available offline."</p></div>
                <form class="space-y-3 px-4 py-3" on:submit=move |event| { event.prevent_default(); save_import(); }>
                    <div class="space-y-1"><label class="text-label-sm text-rambutan-400">"Provider"</label><div class="flex flex-wrap gap-1">{[InboxSource::Email, InboxSource::Slack, InboxSource::Discord, InboxSource::IMessage, InboxSource::X, InboxSource::Import].into_iter().map(|item| view! { <button type="button" class=move || if source.get() == item { "rounded-seed bg-persimmon-500/15 px-2.5 py-1.5 text-label-sm text-persimmon-300 inset-ring inset-ring-persimmon-500/30 cursor-pointer" } else { "rounded-seed px-2.5 py-1.5 text-label-sm text-rambutan-400 inset-ring inset-ring-longan-700 hover:bg-longan-800 cursor-pointer" } on:click=move |_| source.set(item)>{item.label()}</button> }).collect_view()}</div></div>
                    <div class="space-y-1"><label class="text-label-sm text-rambutan-400">"Sender"</label><Input value=Signal::derive(move || sender.get()) on_input=Callback::new(move |value| sender.set(value)) autofocus=true placeholder="Name or handle".to_string() /></div>
                    <div class="space-y-1"><label class="text-label-sm text-rambutan-400">"Subject"</label><Input value=Signal::derive(move || subject.get()) on_input=Callback::new(move |value| subject.set(value)) placeholder="Optional".to_string() /></div>
                    <div class="space-y-1"><label class="text-label-sm text-rambutan-400">"Message"</label><Textarea value=Signal::derive(move || body.get()) on_input=Callback::new(move |value| body.set(value)) rows=5 placeholder="Paste the message…".to_string() /></div>
                    <Show when=move || error.get().is_some()>{move || error.get().map(|message| view! { <p role="alert" class="text-body-sm text-lychee-300">{message}</p> })}</Show>
                    <div class="flex justify-end gap-2"><button type="button" class="rounded-grape px-3 py-2 text-label text-rambutan-300 hover:bg-longan-800 cursor-pointer" on:click=move |_| import_open.set(false)>"Cancel"</button><button type="submit" class="rounded-grape bg-persimmon-500 px-3 py-2 text-label text-longan-950 hover:bg-persimmon-400 cursor-pointer">"Import"</button></div>
                </form>
            </Dialog>
        </Show>
    }
}

#[component]
fn FilterButton(
    label: &'static str,
    value: InboxFilter,
    filter: RwSignal<InboxFilter>,
) -> impl IntoView {
    view! { <button type="button" aria-pressed=move || (filter.get() == value).to_string() class=move || if filter.get() == value { "rounded-seed bg-rambutan-100 px-2.5 py-1.5 text-label-sm text-longan-950 cursor-pointer" } else { "rounded-seed px-2.5 py-1.5 text-label-sm text-rambutan-400 hover:bg-longan-800 cursor-pointer" } on:click=move |_| filter.set(value)>{label}</button> }
}

#[component]
fn Metric(label: &'static str, value: Signal<usize>) -> impl IntoView {
    view! { <div class="rounded-grape bg-longan-900/45 px-3 py-2.5 inset-ring inset-ring-longan-800"><p class="text-label-sm text-rambutan-500">{label}</p><p class="mt-0.5 text-h3 tabular-nums text-rambutan-100">{move || value.get()}</p></div> }
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
                    <div class="flex h-full flex-col overflow-hidden bg-longan-950 text-rambutan-300">
                        <header class="flex flex-wrap items-center gap-2 border-b border-longan-800 px-3 py-2.5 sm:px-5">
                            <button type="button" aria-label="Back to Inbox" class="rounded-seed px-2 py-1.5 text-label text-rambutan-400 hover:bg-longan-800 cursor-pointer" on:click=move |_| set_active_view.set(AppView::Inbox)>"← Inbox"</button>
                            <div class="min-w-0 flex-1"><h1 class="truncate text-h4 text-rambutan-100">{conversation.subject.clone()}</h1><p class="truncate text-label-sm text-rambutan-500">{format!("{} · {}", conversation.source.label(), conversation.participants.iter().map(|item| item.display_name.as_str()).collect::<Vec<_>>().join(", "))}</p></div>
                            <div class="flex flex-wrap gap-1">
                                <ActionButton label=if unread { "Mark read" } else { "Mark unread" } on_click=Callback::new(move |_| update_conversation(set_conversations, &id.get_value(), |item| item.mark_read(unread))) />
                                <ActionButton label=if snoozed { "Unsnooze" } else { "Tomorrow" } on_click=Callback::new(move |_| update_conversation(set_conversations, &id.get_value(), |item| item.snooze_until(if snoozed { None } else { Some(crate::clock::now_ms() + 86_400_000) }))) />
                                <ActionButton label=if important { "Unstar" } else { "Important" } on_click=Callback::new(move |_| update_conversation(set_conversations, &id.get_value(), |item| item.toggle_label("important"))) />
                                <ActionButton label=if archived { "Unarchive" } else { "Archive" } on_click=Callback::new(move |_| update_conversation(set_conversations, &id.get_value(), |item| item.set_archived(!archived))) />
                                <ActionButton label="Ask agent" on_click=Callback::new(move |_| { composer_draft.set(inbox_agent_prompt(&conversation_for_agent)); set_active_view.set(AppView::Chat); }) />
                                <ActionButton label="Delete" on_click=Callback::new(move |_| {
                                    let conversation_id = id.get_value();
                                    let original = conversations.with_untracked(|items| items.iter().find(|item| item.id == conversation_id).cloned());
                                    set_conversations.update(|items| items.retain(|item| item.id != conversation_id));
                                    if let Some(original) = original {
                                        storage::forget_inbox_conversation(original);
                                    }
                                    set_active_view.set(AppView::Inbox);
                                }) />
                            </div>
                        </header>
                        <div class="flex items-center justify-between gap-3 border-b border-longan-800/70 px-3 py-2 sm:px-5">
                            <p class="text-label-sm text-rambutan-500">"Link this conversation to a person for relationship follow-ups."</p>
                            <Dropdown
                                options=Signal::derive(move || std::iter::once(DropdownOption::new("", "Unlinked")).chain(people.get().into_iter().map(|person| DropdownOption::new(person.id, person.display_name))).collect())
                                value=Signal::derive(move || conversations.with(|items| items.iter().find(|item| item.id == id.get_value()).and_then(|item| item.participants.first().and_then(|participant| participant.person_id.clone()))).unwrap_or_default())
                                on_change=Callback::new(move |person_id: String| update_conversation(set_conversations, &id.get_value(), |item| if let Some(participant) = item.participants.first_mut() { participant.person_id = (!person_id.is_empty()).then_some(person_id); }))
                                label="Linked person"
                                class="w-48"
                            />
                        </div>
                        <div class="min-h-0 flex-1 overflow-auto"><div class="mx-auto max-w-3xl space-y-3 px-4 py-5">{conversation.messages.into_iter().filter(|message| message.send_state != MessageSendState::Draft).map(|message| {
                            let outgoing = message.direction == MessageDirection::Outgoing;
                            view! { <article class=move || if outgoing { "ml-auto max-w-[88%] rounded-grape rounded-br-seed bg-persimmon-500/12 px-3 py-2.5 inset-ring inset-ring-persimmon-500/20" } else { "mr-auto max-w-[88%] rounded-grape rounded-bl-seed bg-longan-900 px-3 py-2.5 inset-ring inset-ring-longan-800" }><p class="text-label-sm text-rambutan-500">{message.sender_name}</p><p class="mt-1 whitespace-pre-wrap text-body text-rambutan-200">{message.body}</p></article> }
                        }).collect_view()}</div></div>
                        <form class="border-t border-longan-800 bg-longan-950 px-3 py-3 sm:px-5" on:submit=move |event| { event.prevent_default(); send(); }>
                            <div class="mx-auto max-w-3xl space-y-2"><Textarea id="inbox-reply".to_string() value=Signal::derive(move || draft.get()) on_input=Callback::new(move |value| draft.set(value)) rows=3 placeholder="Write a reply…".to_string() on_keydown=Callback::new(move |event: web_sys::KeyboardEvent| { if (event.meta_key() || event.ctrl_key()) && event.key() == "Enter" { event.prevent_default(); send(); } }) /><div class="flex items-center justify-between gap-3"><p class="text-label-sm text-rambutan-400">"⌘↵ log reply · drafts stay local"</p><div class="flex gap-2"><button type="button" class="rounded-grape px-3 py-2 text-label text-rambutan-300 inset-ring inset-ring-longan-700 hover:bg-longan-800 cursor-pointer" on:click=move |_| save_draft()>"Save draft"</button><button type="submit" disabled=move || draft.get().trim().is_empty() class="rounded-grape bg-persimmon-500 px-3 py-2 text-label text-longan-950 hover:bg-persimmon-400 disabled:cursor-not-allowed disabled:opacity-50 cursor-pointer">"Log reply"</button></div></div></div>
                        </form>
                    </div>
                }.into_any()
            }
        )}
    }
}

#[component]
fn ActionButton(label: &'static str, on_click: Callback<web_sys::MouseEvent>) -> impl IntoView {
    view! { <button type="button" class="rounded-seed px-2 py-1.5 text-label-sm text-rambutan-400 inset-ring inset-ring-longan-700 hover:bg-longan-800 hover:text-rambutan-200 cursor-pointer" on:click=move |event| on_click.run(event)>{label}</button> }
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
