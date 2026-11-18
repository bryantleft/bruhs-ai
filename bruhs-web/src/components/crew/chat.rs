//! Chat with one bruh: its runs as a conversation, with its questions and
//! approvals inline, older runs a page at a time, and a composer.

use bruhs_models::crew::room::{APPROVAL_WANTS, LOAD_EARLIER};
use bruhs_models::crew::{
    approval_outcome, chat_follow_key, chat_markdown, composer_block, line_key, may_have_more,
    merge_runs, transcript, BruhLook, BruhStatus, TranscriptItem, RAIL_FACE_PX,
};
use bruhs_models::reactive::{OutboxItem, ReactiveCommand, ReactiveRun};
use leptos::html;
use leptos::prelude::*;

use super::data::{use_crew_snapshot, CrewData};
use super::face_editor::still_view;
use super::prose::{install_crew_prose, PROSE_BUBBLE};
use crate::ui::{BrailleBruh, Button, ButtonSize, ButtonVariant, Textarea};

pub(super) const CARD: &str =
    "min-w-0 max-w-[80%] space-y-2 rounded-grape bg-longan-900 p-3 inset-ring inset-ring-longan-700";

#[component]
pub fn BruhChat(agent_id: String) -> impl IntoView {
    let data = use_crew_snapshot();
    let id = StoredValue::new(agent_id);
    let runs = RwSignal::new(Vec::<ReactiveRun>::new());
    let oldest = RwSignal::new(None::<String>);
    let more = RwSignal::new(false);
    let loading_earlier = RwSignal::new(false);
    let load_error = RwSignal::new(None::<String>);
    let revision = StoredValue::new(0u64);
    let alive = RwSignal::new(true);
    on_cleanup(move || alive.set(false));

    // The newest page, again whenever the crew changes.
    Effect::new(move |_| {
        data.snapshot.track();
        revision.update_value(|value| *value = value.wrapping_add(1));
        let requested = revision.get_value();
        let agent = id.get_value();
        wasm_bindgen_futures::spawn_local(async move {
            let result = crate::api::agent_runs(&agent, None).await;
            if alive.try_get_untracked() != Some(true)
                || revision.try_get_value() != Some(requested)
            {
                return;
            }
            match result {
                Ok(page) => {
                    if oldest.get_untracked().is_none() {
                        more.set(may_have_more(page.len()));
                        oldest.set(page.last().map(|run| run.id.clone()));
                    }
                    runs.update(|runs| merge_runs(runs, page));
                    load_error.set(None);
                }
                Err(error) => load_error.set(Some(error.to_string())),
            }
        });
    });
    let load_earlier = move |_| {
        let Some(before) = oldest.get_untracked() else {
            return;
        };
        loading_earlier.set(true);
        let agent = id.get_value();
        wasm_bindgen_futures::spawn_local(async move {
            let result = crate::api::agent_runs(&agent, Some(&before)).await;
            if alive.try_get_untracked() != Some(true) {
                return;
            }
            loading_earlier.set(false);
            match result {
                Ok(page) => {
                    more.set(may_have_more(page.len()));
                    if let Some(last) = page.last() {
                        oldest.set(Some(last.id.clone()));
                    }
                    runs.update(|runs| merge_runs(runs, page));
                }
                Err(error) => load_error.set(Some(error.to_string())),
            }
        });
    };

    let state = Memo::new(move |_| {
        data.snapshot.with(|snapshot| {
            snapshot.as_ref().and_then(|snapshot| {
                id.with_value(|id| {
                    snapshot
                        .bruhs
                        .iter()
                        .find(|bruh| bruh.agent_id == *id)
                        .cloned()
                })
            })
        })
    });
    let look = Memo::new(move |_| state.get().map(|state| state.look).unwrap_or_default());
    let lines = Memo::new(move |_| {
        let outbox = data.snapshot.with(|snapshot| {
            snapshot
                .as_ref()
                .map(|snapshot| snapshot.outbox.clone())
                .unwrap_or_default()
        });
        runs.with(|runs| id.with_value(|id| transcript(id, runs, &outbox)))
    });

    // Code, math and diagrams in the bruh's Markdown, and its links.
    install_crew_prose(move || {
        lines.with(|lines| {
            lines
                .iter()
                .filter_map(chat_markdown)
                .map(str::to_string)
                .collect()
        })
    });

    // Follow the conversation, as a room does: a new line scrolls it into
    // view, runs loading above an open card too, and Load earlier does not.
    let scroller = NodeRef::<html::Div>::new();
    Effect::new(move |previous: Option<(Option<String>, Option<String>)>| {
        let shown = lines.with(|lines| chat_follow_key(lines));
        if previous.as_ref() != Some(&shown) {
            if let Some(element) = scroller.get_untracked() {
                element.set_scroll_top(element.scroll_height());
            }
        }
        shown
    });

    let blocked = Memo::new(move |_| composer_block(state.get().map(|state| state.status)));
    let message = RwSignal::new(String::new());
    let send = move || {
        let text = message.get_untracked();
        if text.trim().is_empty() || blocked.get_untracked().is_some() {
            return;
        }
        data.send.run(ReactiveCommand::Chat {
            agent_id: id.get_value(),
            message: text,
            run_id: None,
        });
        message.set(String::new());
    };

    view! {
        <div class="flex min-h-0 flex-1 flex-col">
            <div node_ref=scroller class="min-h-0 flex-1 overflow-y-auto">
                <div class="mx-auto flex w-full max-w-3xl min-w-0 flex-col gap-3 p-4 sm:px-6" role="log" aria-label="Conversation">
                    <Show when=move || more.get()>
                        <div class="flex justify-center">
                            <Button variant=ButtonVariant::Ghost size=ButtonSize::Small disabled=Signal::derive(move || loading_earlier.get()) on_click=Callback::new(load_earlier)>{LOAD_EARLIER}</Button>
                        </div>
                    </Show>
                    {move || load_error.get().map(|message| view! { <p role="alert" class="rounded-grape bg-lychee-500/10 p-3 text-body-sm text-lychee-200">{message}</p> })}
                    <Show when=move || lines.with(Vec::is_empty) && load_error.get().is_none()>
                        <p class="text-body-sm text-rambutan-500">"Nothing here yet. Say hi."</p>
                    </Show>
                    <For
                        each=move || lines.get()
                        key=line_key
                        children=move |item| line(item, look, data)
                    />
                </div>
            </div>
            <div class="shrink-0 border-t border-longan-800/50">
                <div class="mx-auto w-full max-w-3xl min-w-0 space-y-2 p-4 sm:px-6">
                    {move || blocked.get().map(|reason| view! { <p class="text-label-sm text-rambutan-400">{reason}</p> })}
                    <div class="flex min-w-0 items-end gap-2">
                        <Textarea
                            aria_label="Message"
                            placeholder="Message"
                            rows=2
                            class="min-w-0 flex-1"
                            value=Signal::derive(move || message.get())
                            on_input=Callback::new(move |text| message.set(text))
                            disabled=Signal::derive(move || blocked.get().is_some())
                            on_keydown=Callback::new(move |event: web_sys::KeyboardEvent| {
                                if event.key() == "Enter" && !event.shift_key() && !event.is_composing() {
                                    event.prevent_default();
                                    send();
                                }
                            })
                        />
                        <Button
                            size=ButtonSize::Small
                            disabled=Signal::derive(move || blocked.get().is_some() || message.with(|text| text.trim().is_empty()))
                            on_click=Callback::new(move |_| send())
                        >"Send"</Button>
                    </div>
                </div>
            </div>
        </div>
    }
}

fn line(item: TranscriptItem, look: Memo<BruhLook>, data: CrewData) -> AnyView {
    match item {
        // A message from a thread's agent names its sender, as thread chats do.
        TranscriptItem::Owner {
            text,
            pending,
            caption,
            ..
        } => view! {
            <div class="flex min-w-0 flex-col items-end gap-1">
                {caption.map(|caption| view! { <span data-from-thread="" class="text-label-sm text-blueberry-400">{caption}</span> })}
                <p class="min-w-0 max-w-[80%] whitespace-pre-wrap break-words rounded-grape bg-longan-800 px-3 py-2 text-body text-rambutan-100">{text}</p>
                {pending.then(|| view! { <span class="text-label-sm text-rambutan-500">"Waiting for a reply"</span> })}
            </div>
        }
        .into_any(),
        TranscriptItem::Bruh {
            text,
            trigger,
            failed,
            ..
        } => view! {
            <div class="flex min-w-0 items-start gap-2">
                <BrailleBruh size=f64::from(RAIL_FACE_PX) view=Signal::derive(move || look.with(|look| still_view(look, BruhStatus::Working))) />
                <div class="flex min-w-0 max-w-[80%] flex-col gap-1">
                    {trigger.map(|trigger| view! { <span class="text-label-sm text-rambutan-500">{trigger}</span> })}
                    {if failed {
                        view! { <p class="min-w-0 whitespace-pre-wrap break-words rounded-grape bg-lychee-500/10 px-3 py-2 text-body text-lychee-200">{text}</p> }.into_any()
                    } else {
                        view! { <div class=PROSE_BUBBLE inner_html=crate::markdown::render_untrusted(&text)></div> }.into_any()
                    }}
                </div>
            </div>
        }
        .into_any(),
        TranscriptItem::Card { item } if item.approval.is_some() => approval_card(item, data),
        TranscriptItem::Card { item } => question_card(item, data),
        TranscriptItem::Review {
            run_id,
            trigger,
            prompt,
            ..
        } => review_card(run_id, trigger, prompt, data),
    }
}

/// A run waiting for review: Run starts it, Dismiss cancels it. Both are
/// commands every service knows, so an older one takes them too.
pub(super) fn review_card(
    run_id: String,
    trigger: &'static str,
    prompt: String,
    data: CrewData,
) -> AnyView {
    let id = StoredValue::new(run_id);
    let disabled = Signal::derive(move || data.busy.get());
    view! {
        <article class=CARD aria-label="Run waiting for review" data-crew-review="">
            <p class="text-label-sm text-rambutan-400">{format!("{trigger}: waiting for your review")}</p>
            <pre class="max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-grape bg-longan-950 p-2 text-code-sm text-rambutan-300">{prompt}</pre>
            <div class="flex flex-wrap items-center gap-2">
                <Button size=ButtonSize::Small disabled=disabled on_click=Callback::new(move |_| data.send.run(ReactiveCommand::ApproveRun { id: id.get_value() }))>"Run"</Button>
                <Button variant=ButtonVariant::Outline size=ButtonSize::Small disabled=disabled on_click=Callback::new(move |_| data.send.run(ReactiveCommand::CancelRun { id: id.get_value() }))>"Dismiss"</Button>
            </div>
        </article>
    }
    .into_any()
}

pub(super) fn approval_card(item: OutboxItem, data: CrewData) -> AnyView {
    let (summary, detail) = item
        .approval
        .as_ref()
        .map(|approval| (approval.summary.clone(), approval.detail.clone()))
        .unwrap_or_default();
    let outcome = approval_outcome(&item);
    let id = StoredValue::new(item.id.clone());
    let decide = move |approve: bool| {
        data.send.run(ReactiveCommand::DecideApproval {
            id: id.get_value(),
            approve,
        });
    };
    let disabled = Signal::derive(move || data.busy.get() || !data.crew_ready());
    view! {
        <article class=CARD aria-label="Approval">
            <p class="text-label-sm text-rambutan-400">{APPROVAL_WANTS}</p>
            <p class="whitespace-pre-wrap break-words text-body text-rambutan-100">{summary}</p>
            {detail.map(|detail| view! {
                <pre data-approval-detail="" aria-label="Full request" class="max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-grape bg-longan-950 p-2 text-code-sm text-rambutan-300">{detail}</pre>
            })}
            {match outcome {
                Some(outcome) => view! { <p class="text-label-sm text-rambutan-400">{outcome}</p> }.into_any(),
                None => view! {
                    <div class="flex flex-wrap items-center gap-2">
                        <Button size=ButtonSize::Small disabled=disabled on_click=Callback::new(move |_| decide(true))>"Approve"</Button>
                        <Button variant=ButtonVariant::Outline size=ButtonSize::Small disabled=disabled on_click=Callback::new(move |_| decide(false))>"Deny"</Button>
                        {move || data.service_reason().map(|reason| view! { <p class="min-w-0 text-label-sm text-rambutan-400">{reason}</p> })}
                    </div>
                }
                .into_any(),
            }}
        </article>
    }
    .into_any()
}

pub(super) fn question_card(item: OutboxItem, data: CrewData) -> AnyView {
    let reply = RwSignal::new(String::new());
    let id = StoredValue::new(item.id.clone());
    let answer = move || {
        let text = reply.get_untracked();
        if text.trim().is_empty() {
            return;
        }
        data.send.run(ReactiveCommand::AnswerOutbox {
            id: id.get_value(),
            message: text,
        });
    };
    let resolved = item.resolved_at.is_some();
    let label = format!("Reply to {}", item.title);
    view! {
        <article class=CARD aria-label="Question">
            <p class="break-words text-label text-rambutan-100">{item.title.clone()}</p>
            {(!item.message.trim().is_empty()).then(|| view! { <p class="whitespace-pre-wrap break-words text-body-sm text-rambutan-300">{item.message.clone()}</p> })}
            {if resolved {
                match item.answer.clone() {
                    Some(answer) => view! { <p class="whitespace-pre-wrap break-words text-body-sm text-rambutan-400">{format!("You answered: {answer}")}</p> }.into_any(),
                    None => view! { <p class="text-label-sm text-rambutan-400">"Dismissed"</p> }.into_any(),
                }
            } else {
                view! {
                    <div class="space-y-2">
                        <Textarea aria_label=label rows=2 value=Signal::derive(move || reply.get()) on_input=Callback::new(move |text| reply.set(text)) />
                        <Button size=ButtonSize::Small disabled=Signal::derive(move || data.busy.get() || reply.with(|text| text.trim().is_empty())) on_click=Callback::new(move |_| answer())>"Reply"</Button>
                    </div>
                }
                .into_any()
            }}
        </article>
    }
    .into_any()
}
