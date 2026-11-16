//! The email editor is separate from imported Inbox drafts and the read pane.
use crate::{
    state::{MailComposeState, MailState},
    storage::mail_compose::Scope,
    ui::{Button, ButtonSize, ButtonVariant, Input},
};
use bruhs_models::{connections::ServiceProvider, mail::*, num::ToFloatLossy as _};
use leptos::prelude::*;

fn context(mail: MailState) -> Option<(Scope, ServiceProvider)> {
    let snapshot = mail.snapshot.get_untracked()?;
    let mailbox = mail.mailbox.get_untracked()?;
    let provider = snapshot
        .mailboxes
        .iter()
        .find(|row| row.mailbox_id == mailbox)?
        .provider;
    Some((Scope::current(&snapshot.host_id, &mailbox), provider))
}

#[component]
pub(super) fn MailComposeEntry() -> impl IntoView {
    let state = expect_context::<MailComposeState>();
    let mail = expect_context::<MailState>();
    view! {<div class="flex min-w-0 flex-wrap items-center gap-2" data-mail-compose-entry="">
        <Button size=ButtonSize::Small disabled=Signal::derive(move ||mail.mailbox.with(Option::is_none)) on_click=Callback::new(move |_|{if let Some((scope,provider))=context(mail){state.new_message(scope,provider,None,Default::default());}})>"New email"</Button>
        <Button variant=ButtonVariant::Ghost size=ButtonSize::Small disabled=Signal::derive(move ||mail.mailbox.with(Option::is_none)) on_click=Callback::new(move |_|{if let Some((scope,provider))=context(mail){state.show_drafts(scope,provider,0);}})>"Drafts"</Button>
    </div>}
}

#[component]
fn Field(label: &'static str, field: u8) -> impl IntoView {
    let state = expect_context::<MailComposeState>();
    view! {<label class="flex min-w-0 items-center gap-3 border-b border-longan-800 py-1.5">
        <span class="w-14 shrink-0 text-label-sm text-rambutan-400">{label}</span>
        <div class="min-w-0 flex-1"><Input aria_label=label input_type="text".to_string() value=Signal::derive(move ||state.active.with(|value|value.as_ref().map(|value|match field{0=>value.content.to.clone(),1=>value.content.cc.clone(),2=>value.content.bcc.clone(),_=>value.content.subject.clone()}).unwrap_or_default())) disabled=Signal::derive(move ||state.locked.get()||(field==3&&state.active.with(|value|value.as_ref().is_some_and(|value|value.reply.is_some())))) on_input=Callback::new(move |text|state.edit(move |content|match field{0=>content.to=text,1=>content.cc=text,2=>content.bcc=text,_=>content.subject=text})) /></div>
    </label>}
}

fn size(bytes: u64) -> String {
    if bytes >= 1 << 20 {
        format!("{:.1} MiB", bytes.to_f64_lossy() / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes.to_f64_lossy() / 1024.0)
    } else {
        format!("{bytes} {}", if bytes == 1 { "byte" } else { "bytes" })
    }
}

#[component]
pub(super) fn MailComposer() -> impl IntoView {
    let state = expect_context::<MailComposeState>();
    let cc = RwSignal::new(false);
    let send_history = Memo::new(move |_| {
        state.active.with(|value| {
            value
                .as_ref()
                .is_some_and(|value| value.pending_send.is_some() || value.latest_send.is_some())
        })
    });
    let input: NodeRef<leptos::html::Input> = NodeRef::new();
    let body: NodeRef<leptos::html::Textarea> = NodeRef::new();
    Effect::new(move |_| {
        if let Some(value) = state.active.get() {
            if !value.content.cc.is_empty() || !value.content.bcc.is_empty() {
                cc.set(true);
            }
        }
    });
    Effect::new(move |_| {
        if let Some(element) = body.get() {
            let _ = element.focus();
        }
    });
    view! {<section class="flex h-full min-h-0 min-w-0 flex-col bg-longan-950 text-rambutan-200" aria-label="Email composer" data-mail-composer="" on:keydown=move |event|{
        if event.repeat() { return; }
        if (event.meta_key()||event.ctrl_key())&&event.key()=="Enter" {event.prevent_default();event.stop_propagation();if state.review.with(Option::is_some){state.send();}else{state.prepare_review();}}
        else if event.key()=="Escape"{event.prevent_default();event.stop_propagation();if state.review.with(Option::is_some){state.edit_review();}else{state.close();}}
    }>
        <crate::ui::PageHeader title="Compose email" subtitle="Your draft stays saved while you move around Bruhs.">
            <Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_|state.close())>"Close editor"</Button>
        </crate::ui::PageHeader>
        // Keep recovery actions in a fixed footer while status details scroll.
        // A busy message must not move another control under a second click.
        <div class=move ||if send_history.get(){"flex h-[128px] max-h-32 min-h-0 min-w-0 shrink-0 flex-col overflow-hidden border-b border-longan-800 px-4 py-2 [overflow-wrap:anywhere] focus-visible:outline-2 focus-visible:outline-persimmon-400 sm:h-[96px] sm:px-6"}else{"flex max-h-32 min-h-0 min-w-0 shrink-0 flex-col overflow-hidden border-b border-longan-800 px-4 py-2 [overflow-wrap:anywhere] focus-visible:outline-2 focus-visible:outline-persimmon-400 sm:px-6"} tabindex="0" aria-label="Draft status" data-mail-compose-status="">
            <div class=move ||if send_history.get(){"min-h-0 flex-1 overflow-y-auto focus-visible:outline-2 focus-visible:outline-persimmon-400"}else{"min-h-0 overflow-y-auto focus-visible:outline-2 focus-visible:outline-persimmon-400"} tabindex="0" role="region" aria-label="Draft status details">
                <p role="status" class="text-label-sm text-rambutan-400">{move ||state.status.get()}</p>
                {move ||state.error.get().map(|message|view!{<p role="alert" class="text-body-sm text-lychee-300">{message}</p>})}
            </div>
            <div class="grid shrink-0 grid-cols-[minmax(0,1fr)_auto] items-end gap-x-2">
                <div class="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
                    <Show when=move ||state.error.with(Option::is_some)&&!state.sent_check_failed()><Button variant=ButtonVariant::Ghost size=ButtonSize::Small disabled=Signal::derive(move ||state.busy.get()) on_click=Callback::new(move |_|state.retry())>"Retry saving"</Button></Show>
                    <Show when=move ||state.active.with(|value|value.as_ref().and_then(|value|value.latest_send.as_ref()).is_some_and(|status|status.state==MailDispatchState::Queued))><Button variant=ButtonVariant::Ghost size=ButtonSize::Small disabled=Signal::derive(move ||state.busy.get()) on_click=Callback::new(move |_|state.cancel_send())>"Cancel queued send"</Button></Show>
                    <Show when=move ||state.active.with(|value|value.as_ref().is_some_and(|value|value.pending_send.is_some()))><Button variant=ButtonVariant::Ghost size=ButtonSize::Small disabled=Signal::derive(move ||state.busy.get()) on_click=Callback::new(move |_|state.check_send())>"Check send status"</Button></Show>
                </div>
                <div class="flex items-end justify-end">
                    <Show when=move ||state.active.with(|value|value.as_ref().and_then(|value|value.latest_send.as_ref()).is_some_and(|status|status.state==MailDispatchState::Uncertain))><Button variant=ButtonVariant::Ghost size=ButtonSize::Small disabled=Signal::derive(move ||!state.can_check_sent()) on_click=Callback::new(move |_|state.check_sent())>"Check Sent"</Button></Show>
                </div>
            </div>
        </div>
        <Show when=move ||!state.loading.get() fallback=move ||view!{<p role="status" class="px-4 py-6 text-body-sm text-rambutan-400">"Loading the saved draft…"</p>}>
            <Show when=move ||state.review.with(Option::is_some) fallback=move ||view!{
                <div class="min-h-0 min-w-0 flex-1 overflow-y-auto px-4 py-3 sm:px-6">
                    <div class="mx-auto flex min-h-full w-full max-w-4xl min-w-0 flex-col">
                        <div class="flex min-w-0 items-center justify-between gap-2"><p class="text-label-sm text-rambutan-400">{move ||state.active.with(|value|value.as_ref().map(|value|format!("From {}",value.provider.name())).unwrap_or_default())}</p><button type="button" aria-expanded=move ||cc.get().to_string() class=crate::ui::button_class(ButtonVariant::Ghost,ButtonSize::Small,"") on:click=move |_|cc.update(|value| *value = !*value)>"Cc / Bcc"</button></div>
                        <Field label="To" field=0 /><Show when=move ||cc.get()><Field label="Cc" field=1 /><Field label="Bcc" field=2 /></Show><Field label="Subject" field=3 />
                        <label class="mt-3 flex min-h-[192px] min-w-0 flex-1 flex-col gap-2"><span class="text-label-sm text-rambutan-400">"Message"</span><textarea node_ref=body aria-label="Email message" class="min-h-[160px] w-full min-w-0 flex-1 resize-y rounded-grape bg-longan-900/40 p-3 text-body text-rambutan-100 inset-ring inset-ring-longan-800 focus-visible:outline-2 focus-visible:outline-persimmon-400 disabled:opacity-70" spellcheck="true" disabled=move ||state.locked.get() prop:value=move ||state.active.with(|value|value.as_ref().map(|value|value.content.body.clone()).unwrap_or_default()) on:input=move |event|{let text=event_target_value(&event);state.edit(move |content|content.body=text);} /></label>
                        <div class="mt-3 flex min-w-0 flex-col gap-2" aria-label="Selected email files">
                            {move ||state.active.get().map(|value|value.files.into_iter().map(|file|{let id=file.id;view!{<div class="flex min-w-0 items-center justify-between gap-3 rounded-grape bg-longan-900 px-3 py-2 inset-ring inset-ring-longan-800"><div class="min-w-0"><p class="break-all text-label-sm">{file.file_name}</p><p class="text-label-xs text-rambutan-400">{format!("{}  {}",size(file.size_bytes),if file.server.is_some(){"Uploaded"}else{"Waiting to upload"})}</p></div><Button variant=ButtonVariant::Ghost size=ButtonSize::Small disabled=Signal::derive(move ||state.files_locked.get()) on_click=Callback::new(move |_|state.remove_file(&id))>"Remove"</Button></div>}}).collect_view())}
                        </div>
                        {move ||state.active.get().and_then(|value|value.latest_send.map(|status|{let newer=value.dirty||value.server_revision!=status.draft_revision;view!{<SendStatus status newer />}}))}
                    </div>
                </div>
            }><ReviewPane /></Show>
        </Show>
        <footer class="flex min-w-0 shrink-0 items-center justify-between gap-2 border-t border-longan-800 px-4 py-3 sm:px-6">
            <input type="file" multiple node_ref=input class="hidden" aria-label="Choose email files" on:change=move |_|{if let Some(element)=input.get(){if let Some(files)=element.files(){state.add_files((0..files.length()).filter_map(|index|files.get(index)).collect());}element.set_value("");}} />
            <Button variant=ButtonVariant::Ghost disabled=Signal::derive(move ||state.files_locked.get()||state.review.with(Option::is_some)) on_click=Callback::new(move |_|{if let Some(element)=input.get(){element.click();}})>"Attach files"</Button>
            <Show when=move ||state.review.with(Option::is_some) fallback=move ||view!{<Button disabled=Signal::derive(move ||state.busy.get()||state.locked.get()||state.active.with(|value|value.as_ref().is_none_or(|value|value.dirty||value.server_revision==0))) on_click=Callback::new(move |_|state.prepare_review())>"Review email"</Button>}>
                <div class="flex items-center gap-2"><Button variant=ButtonVariant::Ghost disabled=Signal::derive(move ||state.busy.get()) on_click=Callback::new(move |_|state.edit_review())>"Edit"</Button><Button disabled=Signal::derive(move ||state.busy.get()) on_click=Callback::new(move |_|state.send())>"Send email"</Button></div>
            </Show>
        </footer>
    </section>}
}

#[component]
fn ReviewPane() -> impl IntoView {
    let state = expect_context::<MailComposeState>();
    let area: NodeRef<leptos::html::Section> = NodeRef::new();
    Effect::new(move |_| {
        if let Some(element) = area.get() {
            let _ = element.focus();
        }
    });
    view! {<section class="min-h-0 min-w-0 flex-1 overflow-y-auto px-4 py-4 sm:px-6" aria-label="Review email before sending" data-mail-send-review="" tabindex="0" node_ref=area>
        {move ||state.review.get().map(|review|{
            let recipients=|values:Vec<MailAddress>|values.into_iter().map(|value|if value.name.is_empty(){value.address}else{format!("{} <{}>",value.name,value.address)}).collect::<Vec<_>>().join(", ");
            view!{<div class="mx-auto max-w-4xl min-w-0 space-y-4 [overflow-wrap:anywhere]">
                <h2 class="text-title-sm text-rambutan-100">"Review email"</h2>
                <dl class="grid grid-cols-[4rem_minmax(0,1fr)] gap-x-3 gap-y-2 text-body-sm"><dt class="text-rambutan-400">"From"</dt><dd>{review.from.address}</dd><dt class="text-rambutan-400">"To"</dt><dd>{recipients(review.recipients.to)}</dd><dt class="text-rambutan-400">"Cc"</dt><dd>{recipients(review.recipients.cc)}</dd><dt class="text-rambutan-400">"Bcc"</dt><dd>{recipients(review.recipients.bcc)}</dd><dt class="text-rambutan-400">"Subject"</dt><dd>{if review.content.subject.is_empty(){"No subject".into()}else{review.content.subject}}</dd></dl>
                <pre class="whitespace-pre-wrap break-words rounded-grape bg-longan-900 p-4 text-body inset-ring inset-ring-longan-800">{review.content.body}</pre>
                <ul class="space-y-2" aria-label="Reviewed email files">{review.files.into_iter().map(|file|view!{<li class="rounded-grape bg-longan-900 px-3 py-2 text-body-sm inset-ring inset-ring-longan-800"><p class="break-all">{file.file_name}</p><p class="text-label-xs text-rambutan-400">{size(file.size_bytes)}</p></li>}).collect_view()}</ul>
            </div>}
        })}
    </section>}
}

#[component]
fn SendStatus(status: MailSendStatus, #[prop(optional)] newer: bool) -> impl IntoView {
    let message = match status.state {
        MailDispatchState::Queued => "Email is queued.",
        MailDispatchState::Dispatching => "Sending the reviewed email.",
        MailDispatchState::Accepted if newer => {
            "The reviewed version was accepted. Newer edits remain a draft; recipient delivery is not confirmed."
        }
        MailDispatchState::Accepted => {
            "The provider accepted this email. Recipient delivery is not confirmed."
        }
        MailDispatchState::Uncertain => {
            "Sending could not be confirmed. Check Sent mail before starting another send."
        }
        MailDispatchState::NeedsReview => {
            "The draft changed. Review its current version before sending."
        }
        MailDispatchState::Cancelled => "This send was cancelled. Your draft remains saved.",
        MailDispatchState::Rejected => {
            "The provider did not accept this email. Resolve the issue and review again."
        }
    };
    view! {<p role="status" class="mt-3 rounded-grape bg-blueberry-500/10 px-3 py-2 text-body-sm text-blueberry-300 inset-ring inset-ring-blueberry-400/15">{message}</p>}
}

#[component]
pub(super) fn MailDrafts() -> impl IntoView {
    let state = expect_context::<MailComposeState>();
    view! {<section class="flex h-full min-h-0 min-w-0 flex-col bg-longan-950 text-rambutan-200" data-mail-drafts="">
        <crate::ui::PageHeader title="Email drafts" subtitle="Continue unsent work on this device and mail server."><Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_|state.close())>"Back to email"</Button></crate::ui::PageHeader>
        <div class="min-h-0 flex-1 overflow-y-auto px-4 py-4 sm:px-6"><div class="mx-auto max-w-4xl min-w-0 space-y-3">
            <Show when=move ||state.loading.get()><p role="status" class="text-label-sm text-rambutan-400">"Loading saved drafts…"</p></Show>
            {move ||state.error.get().map(|message|view!{<p role="alert" class="text-body-sm text-lychee-300">{message}</p>})}
            {move ||{
                let device=state.local_rows.get();let mut ids=std::collections::HashSet::new();let mut rows=Vec::new();
                for row in device{ids.insert(row.client_id.clone());rows.push((row.client_id,row.subject,row.recipients));}
                if let Some(page)=state.server_rows.get(){for row in page.rows{if ids.insert(row.client_id.clone()){rows.push((row.client_id,row.subject,row.recipients));}}}
                if rows.is_empty(){view!{<p role="status" class="py-10 text-center text-body-sm text-rambutan-400">"No saved drafts in this view."</p>}.into_any()}else{rows.into_iter().map(|(id,subject,recipients)|view!{<button type="button" class="block w-full min-w-0 rounded-grape bg-longan-900 px-4 py-3 text-left inset-ring inset-ring-longan-800 hover:bg-longan-800 focus-visible:outline-2 focus-visible:outline-persimmon-400 cursor-pointer" on:click=move |_|{state.open_listed(id.clone());}><p class="truncate text-body-sm">{if subject.trim().is_empty(){"No subject".into()}else{subject}}</p><p class="mt-1 truncate text-label-sm text-rambutan-400">{recipients}</p></button>}).collect_view().into_any()}
            }}
            <Show when=move ||state.local_more.get()><Button variant=ButtonVariant::Ghost disabled=Signal::derive(move ||state.loading.get()) on_click=Callback::new(move |_|state.older_local())>"Older device drafts"</Button></Show>
            {move ||state.server_rows.get().map(|page|view!{<nav class="flex items-center justify-between gap-2" aria-label="Server draft pages"><Button variant=ButtonVariant::Ghost disabled=Signal::derive(move ||state.loading.get()||page.page==0) on_click=Callback::new(move |_|state.server_page(page.page.saturating_sub(1)))>"Previous"</Button><span class="text-label-sm text-rambutan-400">{format!("Page {} of {}",page.page.saturating_add(1),page.total_matches.div_ceil(50).max(1))}</span><Button variant=ButtonVariant::Ghost disabled=Signal::derive(move ||state.loading.get()||page.page.saturating_add(1).saturating_mul(50)>=page.total_matches) on_click=Callback::new(move |_|state.server_page(page.page.saturating_add(1)))>"Next"</Button></nav>})}
        </div></div>
    </section>}
}
