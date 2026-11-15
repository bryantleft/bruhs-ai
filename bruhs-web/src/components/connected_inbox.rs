//! Connected mail uses the canonical server cache. No imported record grants
//! provider access, and no remote HTML, image or attachment is rendered here.
use crate::{
    state::MailState,
    ui::{Button, ButtonSize, ButtonVariant, Input},
};
use bruhs_models::{
    connections::ServiceProvider, mail::*, InboxFilter, InboxMessage, INBOX_PAGE_SIZE,
};
use leptos::prelude::*;
use wasm_bindgen::JsCast;

#[component]
pub(super) fn ConnectedInbox() -> impl IntoView {
    let compose = expect_context::<crate::state::MailComposeState>();
    view! {<Show when=move ||compose.visible.get() fallback=move ||view!{<Show when=move ||compose.list_visible.get() fallback=move ||view!{<ConnectedInboxRead />}><super::mail_compose::MailDrafts /></Show>}><super::mail_compose::MailComposer /></Show>}
}

#[component]
fn ConnectedInboxRead() -> impl IntoView {
    let state = expect_context::<MailState>();
    let set_view = expect_context::<crate::state::SetActiveView>().0;
    Effect::new(move |_| state.refresh());
    view! {
        <div class="flex h-full min-h-0 min-w-0 flex-col bg-longan-950 text-rambutan-300" data-connected-inbox="">
            <crate::ui::PageHeader title="Email" subtitle="Connected Gmail and Outlook, saved on this server.">
                <Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| state.visible.set(false))>"Imported conversations"</Button>
                <Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| set_view.set(crate::state::AppView::Settings))>"Connections"</Button>
                <Button variant=ButtonVariant::Ghost size=ButtonSize::Small disabled=Signal::derive(move || state.accounts_loading.get()) on_click=Callback::new(move |_| state.refresh())>"Refresh"</Button>
                <Button variant=ButtonVariant::Outline size=ButtonSize::Small disabled=Signal::derive(move || state.syncing.get() || state.accounts_loading.get() || state.snapshot.with(Option::is_none)) on_click=Callback::new(move |_| state.sync_provider(ServiceProvider::Gmail,false))>{move || if state.syncing.get() { "Syncing…" } else { "Sync Gmail" }}</Button>
                <Button variant=ButtonVariant::Outline size=ButtonSize::Small disabled=Signal::derive(move || state.syncing.get() || state.accounts_loading.get() || state.snapshot.with(Option::is_none)) on_click=Callback::new(move |_| state.sync_provider(ServiceProvider::Outlook,false))>"Sync Outlook"</Button>
                <Show when=move || state.syncing.get()>
                    <Button variant=ButtonVariant::Ghost size=ButtonSize::Small disabled=Signal::derive(move || state.stopping.get()) on_click=Callback::new(move |_| state.stop())>"Stop sync"</Button>
                </Show>
            </crate::ui::PageHeader>
            <Show when=move || state.accounts_loading.get() || state.error.with(Option::is_some) || state.sync_error.with(Option::is_some) || state.download_error.with(Option::is_some) || state.downloading.with(Option::is_some) || state.notice.with(Option::is_some)>
                <section data-mail-feedback="" aria-label="Email status" tabindex="0" class="max-h-32 min-h-0 min-w-0 shrink-0 space-y-1 overflow-y-auto border-b border-longan-800 px-4 py-2 [overflow-wrap:anywhere] focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-persimmon-400 sm:px-6">
                    <Show when=move || state.accounts_loading.get()><p role="status" class="text-label-sm text-rambutan-400">"Refreshing saved accounts…"</p></Show>
                    {move || state.error.get().map(|message| view! { <p role="alert" class="text-body-sm text-lychee-300">{message}</p> })}
                    {move || state.sync_error.get().map(|message| view! { <p role="alert" data-mail-sync-error="" class="text-body-sm text-lychee-300">{message}</p> })}
                    {move || state.download_error.get().map(|message| view! { <p role="alert" data-mail-download-error="" class="text-body-sm text-lychee-300">{message}</p> })}
                    <Show when=move || state.downloading.with(Option::is_some)><p role="status" class="text-label-sm text-rambutan-400">"Downloading the attachment…"</p></Show>
                    {move || state.notice.get().map(|message| view! { <p role="status" data-mail-notice="" class="text-label-sm text-rambutan-400">{message}</p> })}
                </section>
            </Show>
            <div class="min-h-0 min-w-0 flex-1 overflow-y-auto">
                <div class="mx-auto flex w-full max-w-5xl min-w-0 flex-col gap-4 px-4 py-5 sm:px-6">
                    <div class="flex min-w-0 flex-wrap gap-1" role="group" aria-label="Saved mail accounts">
                        {move || state.snapshot.get().map(|snapshot| snapshot.mailboxes.into_iter().map(|mailbox| {
                            let selected_id = mailbox.mailbox_id.clone();
                            let id = mailbox.mailbox_id;
                            let label = if mailbox.account_label.is_empty() { mailbox.provider.name().to_string() } else { mailbox.account_label };
                            view! { <button type="button" aria-pressed=move || (state.mailbox.get().as_ref() == Some(&selected_id)).to_string() class="bruhs-control inline-flex min-h-8 min-w-0 max-w-full items-start gap-2 whitespace-normal rounded-grape px-2.5 py-1.5 text-left text-label-sm text-rambutan-300 hover:bg-longan-800 aria-pressed:bg-blueberry-500/15 aria-pressed:text-blueberry-400 focus-visible:outline-2 focus-visible:outline-persimmon-400 disabled:opacity-60 cursor-pointer" disabled=move || state.syncing.get() on:click=move |_| state.select_mailbox(id.clone())><span class="min-w-0 break-all">{label}</span><span class="shrink-0 whitespace-nowrap">{format!("{} threads", mailbox.threads)}</span></button> }
                        }).collect_view())}
                    </div>
                    <Show when=move || state.selected_thread.with(Option::is_some) fallback=move || view! { <MailRows /> }>
                        <ThreadPane />
                    </Show>
                    <div class="space-y-2 border-t border-longan-800 pt-3">
                        <p class="text-label-sm text-rambutan-400">"Sync reads Gmail or Outlook into this server's private cache. Imported conversations remain on this device."</p>
                        <Button variant=ButtonVariant::Ghost size=ButtonSize::Small disabled=Signal::derive(move || state.syncing.get() || state.accounts_loading.get() || state.snapshot.with(Option::is_none)) on_click=Callback::new(move |_| state.sync(true))>"Restart sync"</Button>
                    </div>
                </div>
            </div>
        </div>
    }
}

#[component]
fn MailRows() -> impl IntoView {
    let state = expect_context::<MailState>();
    view! {
        <div class="flex min-w-0 flex-col gap-3">
            <super::mail_compose::MailComposeEntry />
            <form role="search" aria-label="Search saved email" class="flex items-center gap-2" on:submit=move |event| { event.prevent_default(); state.search(); }>
                <div class="min-w-0 flex-1"><Input aria_label="Search email" input_type="search".to_string() placeholder="Search people, subjects, and messages…".to_string() value=Signal::derive(move || state.query.get()) on_input=Callback::new(move |value| state.query.set(value)) /></div>
                <button type="submit" class=crate::ui::button_class(ButtonVariant::Ghost, ButtonSize::Default, "") disabled=move || state.mailbox.with(Option::is_none)>"Search"</button>
            </form>
            <div class="flex flex-wrap gap-1" role="group" aria-label="Email filter">
                {InboxFilter::ALL.into_iter().map(|filter| view! {
                    <button type="button" aria-pressed=move || (state.filter.get() == filter).to_string() class="bruhs-control min-h-8 rounded-grape px-2.5 py-1.5 text-label-sm text-rambutan-400 hover:bg-longan-800 aria-pressed:bg-blueberry-500/15 aria-pressed:text-blueberry-400 focus-visible:outline-2 focus-visible:outline-persimmon-400 disabled:opacity-60 cursor-pointer" disabled=move || state.mailbox.with(Option::is_none) on:click=move |_| state.select_filter(filter)>{filter.label()}</button>
                }).collect_view()}
            </div>
            <Show when=move || state.rows_loading.get()><p role="status" class="text-label-sm text-rambutan-400">"Loading cached email…"</p></Show>
            <section aria-label="Saved email threads" class="overflow-hidden rounded-grape bg-longan-900/45 inset-ring inset-ring-longan-800">
                {move || state.rows.get().map(|page| page.rows.into_iter().map(|row| view! { <MailRow row /> }).collect_view())}
                <Show when=move || !state.rows_loading.get() && state.rows.with(|page| page.as_ref().is_none_or(|page| page.rows.is_empty()))>
                    <p role="status" data-mail-empty="" class="px-4 py-12 text-center text-body-sm text-rambutan-300">{move || if state.mailbox.with(Option::is_none) { "Connect Gmail or Outlook, then sync to save mail on this server." } else { "No saved email matches this view. Try another search or choose All." }}</p>
                </Show>
            </section>
            {move || state.rows.get().map(|page| {
                let page_size = u64::try_from(INBOX_PAGE_SIZE).unwrap_or(u64::MAX);
                let start = page.page.saturating_mul(page_size);
                let end = start.saturating_add(u64::try_from(page.rows.len()).unwrap_or(u64::MAX));
                view! {
                    <nav aria-label="Email pages" class="flex flex-wrap items-center justify-between gap-2">
                        <Button variant=ButtonVariant::Ghost disabled=Signal::derive(move || page.page == 0 || state.rows_loading.get()) on_click=Callback::new(move |_| state.select_page(page.page.saturating_sub(1)))>"Previous"</Button>
                        <p role="status" data-mail-page="" class="text-label-sm tabular-nums text-rambutan-400">{if page.total_matches == 0 { "0 threads".into() } else { format!("{} to {end} of {}", start.saturating_add(1), page.total_matches) }}</p>
                        <Button variant=ButtonVariant::Ghost disabled=Signal::derive(move || end >= page.total_matches || state.rows_loading.get()) on_click=Callback::new(move |_| state.select_page(page.page.saturating_add(1)))>"Next"</Button>
                    </nav>
                }
            })}
        </div>
    }
}

#[component]
fn MailRow(row: MailThreadRow) -> impl IntoView {
    let state = expect_context::<MailState>();
    let card = row.card;
    let (date, full_date) = super::inbox_page::inbox_date(card.message_at_ms);
    let id = row.external_id;
    let dom_id = format!("mail-thread-{id}");
    view! {
        <button type="button" id=dom_id data-mail-row="" class="flex w-full items-start gap-3 border-b border-longan-800/80 px-3 py-3 text-left last:border-b-0 hover:bg-longan-800/60 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-persimmon-400 cursor-pointer" on:click=move |_| state.open_thread(id.clone())>
            <span aria-hidden="true" class=if card.unread { "mt-2 size-2 shrink-0 rounded-full bg-blueberry-400" } else { "mt-2 size-2 shrink-0 rounded-full bg-longan-700" }></span>
            <span class="sr-only">{if card.unread { "Unread email: " } else { "Read email: " }}</span>
            <span class="min-w-0 flex-1">
                <span class="flex items-baseline justify-between gap-3"><span class=if card.unread { "truncate text-label font-semibold text-rambutan-100" } else { "truncate text-label text-rambutan-300" }>{card.sender}</span><span class="shrink-0 text-label-sm tabular-nums text-rambutan-500" title=full_date>{date}</span></span>
                <span class="mt-0.5 block truncate text-body-sm text-rambutan-300">{if card.subject.trim().is_empty() { "No subject".into() } else { card.subject }}</span>
                <span class="mt-0.5 block truncate text-body-sm text-rambutan-500">{card.preview}</span>
            </span>
        </button>
    }
}

#[component]
fn ThreadPane() -> impl IntoView {
    let state = expect_context::<MailState>();
    let back: NodeRef<leptos::html::Button> = NodeRef::new();
    Effect::new(move |_| {
        if let Some(button) = back.get() {
            let _ = button.focus();
        }
    });
    view! {
        <div class="flex min-w-0 flex-col gap-4" data-mail-reader="">
            <div><button type="button" node_ref=back class=crate::ui::button_class(ButtonVariant::Ghost, ButtonSize::Default, "") on:click=move |_| {
                let previous = state.selected_thread.get_untracked();
                state.back();
                queue_microtask(move || {
                    if let Some(id) = previous {
                        if let Some(element) = document().get_element_by_id(&format!("mail-thread-{id}")).and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok()) { let _ = element.focus(); }
                    }
                });
            }>"Back to email"</button></div>
            <Show when=move || state.thread_loading.get()><p role="status" class="text-label-sm text-rambutan-400">"Loading saved thread…"</p></Show>
            {move || state.thread.get().map(|thread| view! { <ThreadReader thread /> })}
        </div>
    }
}

#[component]
fn ThreadReader(thread: ConnectedMailThread) -> impl IntoView {
    let state = expect_context::<MailState>();
    let compose = expect_context::<crate::state::MailComposeState>();
    let title = if thread.conversation.subject.is_empty() {
        "No subject".into()
    } else {
        thread.conversation.subject.clone()
    };
    let count = thread.conversation.messages.len();
    let thread = StoredValue::new(thread);
    view! {
        <h2 class="break-words text-h3 text-rambutan-100">{title}</h2>
        <nav aria-label="Messages in this thread" class="flex flex-wrap items-center justify-between gap-2">
            <Button variant=ButtonVariant::Ghost disabled=Signal::derive(move || state.message.get() == 0) on_click=Callback::new(move |_| state.message.update(|index| *index = index.saturating_sub(1)))>"Older message"</Button>
            <p role="status" data-mail-message-index="" class="text-label-sm text-rambutan-400">{move || format!("Message {} of {count}", state.message.get().saturating_add(1))}</p>
            <Button variant=ButtonVariant::Ghost disabled=Signal::derive(move || state.message.get().saturating_add(1) >= count) on_click=Callback::new(move |_| state.message.update(|index| *index = index.saturating_add(1)))>"Newer message"</Button>
        </nav>
        {move || thread.with_value(|thread| thread.conversation.messages.get(state.message.get()).map(|message| {
            let detail = thread.connected_mail.messages.iter().find(|detail| detail.id == message.id).cloned();
            view! { <MessageBody message=message.clone() detail provider=thread.connected_mail.provider /> }
        }))}
        <div class="flex flex-wrap gap-2" aria-label="Reply to this message">
            <Button variant=ButtonVariant::Outline on_click=Callback::new(move |_| thread.with_value(|thread| reply(thread, state, compose, false)))>"Reply"</Button>
            <Button variant=ButtonVariant::Ghost on_click=Callback::new(move |_| thread.with_value(|thread| reply(thread, state, compose, true)))>"Reply all"</Button>
        </div>
        <p class="text-label-sm text-rambutan-400">"Saved plain text. Remote images and HTML are not loaded."</p>
    }
}

fn reply(
    thread: &ConnectedMailThread,
    state: MailState,
    compose: crate::state::MailComposeState,
    all: bool,
) {
    let Some(message) = thread
        .conversation
        .messages
        .get(state.message.get_untracked())
    else {
        return;
    };
    let Some(detail) = thread
        .connected_mail
        .messages
        .iter()
        .find(|detail| detail.id == message.id)
    else {
        return;
    };
    let Some(snapshot) = state.snapshot.get_untracked() else {
        return;
    };
    let mailbox = &thread.connected_mail.mailbox_id;
    let own = snapshot
        .mailboxes
        .iter()
        .find(|row| &row.mailbox_id == mailbox)
        .map_or("", |row| row.account_label.as_str());
    let content = reply_content(detail, message.direction, own, all);
    compose.new_message(
        crate::storage::mail_compose::Scope::current(&snapshot.host_id, mailbox),
        thread.connected_mail.provider,
        Some(MailReplyTarget {
            external_thread_id: thread.connected_mail.external_thread_id.clone(),
            message_id: message.id.clone(),
        }),
        content,
    );
}

#[component]
fn MessageBody(
    message: InboxMessage,
    detail: Option<MailMessageDetail>,
    provider: ServiceProvider,
) -> impl IntoView {
    let state = expect_context::<MailState>();
    let (_, date) = super::inbox_page::inbox_date(message.sent_at_ms);
    let copy = message.body.clone();
    view! {
        <article class="min-w-0 space-y-3" aria-label="Saved message">
            <p class="text-label-sm text-rambutan-400">{date}</p>
            {detail.map(|detail| view! {
                <div class="space-y-1 text-label-sm text-rambutan-400 [overflow-wrap:anywhere]">
                    <p>{format!("From: {}", addresses(&detail.envelope.from))}</p>
                    <p>{format!("To: {}", addresses(&detail.envelope.to))}</p>
                    {(!detail.envelope.cc.is_empty()).then(|| view! { <p>{format!("Cc: {}", addresses(&detail.envelope.cc))}</p> })}
                    {detail.attachments.into_iter().map(|attachment| view! { <AttachmentRow attachment provider /> }).collect_view()}
                </div>
            })}
            <Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| {
                let text = copy.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let result = match web_sys::window() {
                        Some(window) => wasm_bindgen_futures::JsFuture::from(window.navigator().clipboard().write_text(&text)).await.map(|_| ()),
                        None => Err(wasm_bindgen::JsValue::NULL),
                    };
                    state.notice.try_set(Some(if result.is_ok() { "Message text copied." } else { "Message text could not be copied. Select the text and copy it manually." }.into()));
                });
            })>"Copy message text"</Button>
            <div data-mail-message-body="" class="select-text whitespace-pre-wrap rounded-grape bg-longan-900 p-4 text-body-sm leading-relaxed text-rambutan-200 inset-ring inset-ring-longan-800 [overflow-wrap:anywhere]">{message.body}</div>
        </article>
    }
}

#[component]
fn AttachmentRow(attachment: MailAttachment, provider: ServiceProvider) -> impl IntoView {
    let state = expect_context::<MailState>();
    let description = format!(
        "Attachment: {} ({} bytes)",
        attachment.file_name, attachment.size_bytes
    );
    let label = format!("Download attachment: {}", attachment.file_name);
    let downloadable = provider != ServiceProvider::Outlook || attachment.external_id.is_some();
    let attachment = StoredValue::new(attachment);
    view! {
        <div data-mail-attachment="" class="flex min-w-0 flex-wrap items-center justify-between gap-2">
            <p class="min-w-0 flex-1 [overflow-wrap:anywhere]">{description}</p>
            <button type="button" data-mail-attachment-download="" aria-label=label class=crate::ui::button_class(ButtonVariant::Ghost, ButtonSize::Default, "") disabled=move || !downloadable || state.downloading.with(Option::is_some) on:click=move |_| state.download(attachment.get_value())>"Download attachment"</button>
            {(!downloadable).then(||view!{<p class="w-full text-label-sm text-rambutan-400">"Linked or unsupported attachment. Open it in Outlook."</p>})}
        </div>
    }
}

pub(crate) fn download_blob(blob: &web_sys::Blob, name: &str) -> Result<(), &'static str> {
    let url = web_sys::Url::create_object_url_with_blob(blob)
        .map_err(|_| "The browser could not create an attachment download.")?;
    let result = (|| {
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or("The browser document is unavailable.")?;
        let anchor = document
            .create_element("a")
            .map_err(|_| "The browser could not create an attachment download.")?
            .dyn_into::<web_sys::HtmlAnchorElement>()
            .map_err(|_| "The browser could not create an attachment download.")?;
        anchor.set_href(&url);
        anchor.set_download(name);
        anchor.set_hidden(true);
        document
            .body()
            .ok_or("The browser document is unavailable.")?
            .append_child(&anchor)
            .map_err(|_| "The browser could not start the attachment download.")?;
        anchor.click();
        anchor.remove();
        Ok(())
    })();
    let _ = web_sys::Url::revoke_object_url(&url);
    result
}

fn addresses(addresses: &[MailAddress]) -> String {
    addresses
        .iter()
        .map(|address| {
            if address.name.is_empty() {
                address.address.clone()
            } else {
                format!("{} <{}>", address.name, address.address)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}
