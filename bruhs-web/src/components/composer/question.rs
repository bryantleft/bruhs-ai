//! A question draft is separate from the composer's outgoing message.
use bruhs_models::api::AnswerQuestionRequest;
use bruhs_models::logic::user_question::{
    validate_answers, QuestionReply, UserQuestion, CHOICE_ADVANCE_DELAY_MS,
};
use leptos::leptos_dom::helpers::window_event_listener;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::api::{answer_question, ApiError};
use crate::components::file_attachments::OfferPhase;

use crate::state::{ActiveThreadId, PendingPermissions, PermissionRequest, QuestionDraft};
use crate::ui::{Button, ButtonSize, ButtonVariant, Icon, Kbd, ResetTimer};

use super::{answer_outcome, AnswerOutcome};

mod files;

#[component]
pub(super) fn QuestionPanel(
    request: PermissionRequest,
    questions: Vec<UserQuestion>,
    draft: ArcRwSignal<QuestionDraft>,
    pending: RwSignal<PendingPermissions>,
) -> impl IntoView {
    let count = questions.len();
    let questions = StoredValue::new(questions);
    let request = StoredValue::new(request);
    let draft = RwSignal::from(draft);
    let index = Memo::new(move |_| draft.with(|draft| draft.position));
    let expanded =
        Memo::new(move |_| draft.with(|draft| draft.collapsed_question != Some(draft.position)));
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let panel = NodeRef::<leptos::html::Section>::new();
    let active_thread = expect_context::<ActiveThreadId>().0;
    let auto_advance = ResetTimer::new();
    let can_advance = move || {
        draft.with(|draft| {
            draft.answers[draft.position].is_answered()
                && draft.uploads.iter().all(|item| {
                    item.upload.question != draft.position
                        || matches!(item.phase, OfferPhase::Ready { .. })
                })
        })
    };
    let can_send = move || {
        draft.with(|draft| {
            draft
                .uploads
                .iter()
                .all(|item| matches!(item.phase, OfferPhase::Ready { .. }))
                && request
                    .with_value(|request| validate_answers(&request.input, &draft.answers).is_ok())
        })
    };
    let respond = move |skip: bool| {
        auto_advance.cancel();
        if busy.get_untracked() {
            return;
        }
        let request = request.get_value();
        let reply = if skip {
            QuestionReply::Skip
        } else {
            QuestionReply::Answer {
                answers: draft.with_untracked(|draft| draft.answers.clone()),
            }
        };
        busy.set(true);
        error.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            let result = match request.thread_id.parse() {
                Ok(thread_id) => {
                    answer_question(&AnswerQuestionRequest {
                        thread_id,
                        request_id: request.id.clone(),
                        reply,
                    })
                    .await
                }
                Err(_) => Err(ApiError::Decode(
                    "The active thread could not be identified.".into(),
                )),
            };
            match answer_outcome(&result) {
                AnswerOutcome::Answered | AnswerOutcome::Gone => {
                    pending.try_update(|pending| pending.remove(&request.id));
                }
                AnswerOutcome::Failed(message) => {
                    error.try_set(Some(message));
                }
            }
            busy.try_set(false);
        });
    };
    let advance = move || {
        auto_advance.cancel();
        if busy.get_untracked() || !untrack(can_advance) {
            return;
        }
        if index.get_untracked() + 1 < count {
            draft.update(|draft| draft.position += 1);
        } else if untrack(can_send) {
            respond(false);
        }
    };
    let select_option = move |position: usize, option: usize| {
        if busy.get_untracked() || index.get_untracked() != position {
            return;
        }
        auto_advance.cancel();
        let auto = questions
            .with_value(|questions| {
                draft.try_update(|draft| {
                    draft.answers[position].select(&questions[position], option)
                        && !draft
                            .uploads
                            .iter()
                            .any(|item| item.upload.question == position)
                })
            })
            .unwrap_or(false);
        if auto {
            auto_advance.after(CHOICE_ADVANCE_DELAY_MS, move || {
                if panel
                    .get_untracked()
                    .is_some_and(|panel| panel.is_connected())
                    && request.with_value(|request| {
                        active_thread.with_untracked(|thread| {
                            thread.as_deref() == Some(request.thread_id.as_str())
                        })
                    })
                    && index.get_untracked() == position
                    && expanded.get_untracked()
                    && draft.with_untracked(|draft| {
                        draft.answers[position].is_choice_only()
                            && !draft
                                .uploads
                                .iter()
                                .any(|item| item.upload.question == position)
                    })
                {
                    advance();
                }
            });
        }
    };
    let shortcut = window_event_listener(
        leptos::ev::keydown,
        move |event: web_sys::KeyboardEvent| {
            if event.default_prevented()
                || event.is_composing()
                || event.repeat()
                || event.meta_key()
                || event.ctrl_key()
                || event.alt_key()
                || event.shift_key()
                || busy.get_untracked()
                || !expanded.get_untracked()
            {
                return;
            }
            let Some(panel) = panel.get_untracked() else {
                return;
            };
            let Some(target) = event
                .target()
                .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
            else {
                return;
            };
            if target.closest("input, textarea, select, [contenteditable]:not([contenteditable='false']), [role='dialog'], [aria-modal='true'], [role='menu'], [role='combobox']").ok().flatten().is_some() {
            return;
        }
            // A second chat pane can have its own pending question. Keep digit
            // presses in that pane and use the main question for page-level focus.
            let owner = target
                .closest("[data-agent-question]")
                .ok()
                .flatten()
                .or_else(|| {
                    document()
                        .query_selector("[data-agent-question]")
                        .ok()
                        .flatten()
                });
            if owner.is_none_or(|owner| !panel.is_same_node(Some(&owner))) {
                return;
            }
            let position = index.get_untracked();
            if let Some(option) =
                questions.with_value(|questions| questions[position].shortcut_option(&event.key()))
            {
                event.prevent_default();
                select_option(position, option);
            }
        },
    );
    on_cleanup(move || {
        auto_advance.cancel();
        shortcut.remove();
    });

    view! {
        <section node_ref=panel data-agent-question="" role="group" aria-label="Question from the agent"
            class="border-b border-longan-800/60 px-4 py-3">
            <button
                type="button"
                data-agent-question-toggle=""
                aria-expanded=move || expanded.get().to_string()
                title=move || if expanded.get() { "Hide the question and its options" } else { "Show the question and its options" }
                class="mb-2 flex min-h-control w-full items-center gap-2 rounded-grape text-left text-label-sm text-rambutan-400 hover:bg-longan-800/50 focus-visible:outline-2 focus-visible:outline-persimmon-400"
                on:click=move |_| {
                    auto_advance.cancel();
                    draft.update(|draft| {
                        draft.collapsed_question = (draft.collapsed_question != Some(draft.position)).then_some(draft.position);
                    });
                }
            >
                <span class="max-w-1/2 truncate">
                    {move || questions.with_value(|questions| {
                        let header = &questions[index.get()].header;
                        if header.trim().is_empty() { "Question from the agent".to_owned() } else { header.clone() }
                    })}
                </span>
                <span class="min-w-0 flex-1 truncate text-rambutan-200">
                    {move || (!expanded.get()).then(|| questions.with_value(|questions| questions[index.get()].question.clone()))}
                </span>
                {(count > 1).then(|| view! { <span class="shrink-0">{move || format!("{} of {count}", index.get() + 1)}</span> })}
                {move || view! { <Icon name=if expanded.get() { "chevron-down" } else { "chevron-right" } size=12 class="shrink-0" /> }}
            </button>
            <div>
                {move || {
                    let position = index.get();
                    let question = questions.with_value(|items| items[position].clone());
                    let allow_custom = question.allow_custom;
                    let title = question.question.clone();
                    let options = question.options.iter().cloned().enumerate().map(|(i, option)| {
                        let checked = move || draft.with(|draft| draft.answers[position].selected.contains(&i));
                        let shortcut = i + 1;
                        view! {
                            <div class="min-w-0 shrink-0">
                            <button
                                type="button"
                                aria-pressed=move || checked().to_string()
                                aria-keyshortcuts=shortcut.to_string()
                                disabled=move || busy.get()
                                class="bruhs-control flex min-h-control w-full shrink-0 items-center gap-2 rounded-grape border px-3 py-2 text-left text-label-sm focus-visible:outline-2 focus-visible:outline-persimmon-400 disabled:opacity-60"
                                class:border-persimmon-400=move || checked()
                                class:bg-longan-800=move || checked()
                                class:border-longan-800=move || !checked()
                                on:click=move |_| select_option(position, i)
                            >
                                <span class="flex min-w-0 flex-1 flex-col">
                                    <span class="text-rambutan-100">{option.label}</span>
                                    {(!option.description.is_empty()).then(|| view! {
                                        <span class="text-rambutan-400">{option.description}</span>
                                    })}
                                </span>
                                <span aria-hidden="true" class="shrink-0"><Kbd>{shortcut.to_string()}</Kbd></span>
                            </button>
                            {option.preview.map(|preview| view! {
                                <div data-agent-question-preview="" class="prose-msg min-w-0 break-words px-3 py-1 text-label-sm text-rambutan-300"
                                    inner_html=crate::markdown::render_untrusted(&preview) />
                            })}
                            </div>
                        }
                    }).collect_view();
                    view! {
                        <div hidden=move || !expanded.get()>
                            <h3 class="mb-2 text-body-sm font-medium text-rambutan-100">{title}</h3>
                            {question.multi_select.then(|| view! { <p class="mb-2 text-label-sm text-rambutan-400">"Choose one or more"</p> })}
                            <div class="flex max-h-36 flex-col gap-1 overflow-y-auto [scrollbar-width:thin]">{options}</div>
                        </div>
                        {allow_custom.then(|| {
                          let textarea = view! {
                            <label class="mt-2 flex flex-col gap-1 text-label-sm text-rambutan-400">
                                "Custom answer"
                                <textarea
                                    rows=2
                                    maxlength=8192
                                    disabled=move || busy.get()
                                    class="min-h-control w-full resize-y rounded-grape border border-longan-800 bg-longan-950 px-3 py-2 text-body-sm text-rambutan-100 focus-visible:outline-2 focus-visible:outline-persimmon-400"
                                    prop:value=move || draft.with(|draft| draft.answers[position].text.clone())
                                    on:input=move |event| {
                                        auto_advance.cancel();
                                        draft.update(|draft| draft.answers[position].text = event_target_value(&event));
                                    }
                                />
                            </label>
                          }.into_any();
                          let request = request.get_value();
                          let answer = if let Some(uploads) = request.question_draft.clone().filter(|_| request.can_attach) {
                              view! { <files::QuestionFiles question=position request=request draft=uploads busy=busy error=error textarea=textarea/> }.into_any()
                          } else {
                              textarea
                          };
                          view! {
                              <div on:focusin=move |_| auto_advance.cancel()
                                  on:pointerdown=move |_| auto_advance.cancel()>
                                  {answer}
                              </div>
                          }
                        })}
                    }
                }}
            </div>
            {move || error.get().map(|message| view! {
                <p role="alert" class="mt-2 text-label-sm text-lychee-300">{message}</p>
            })}
            <div class="mt-2 flex flex-wrap items-center gap-2">
                <Button variant=ButtonVariant::Ghost size=ButtonSize::Small disabled=busy on_click=Callback::new(move |_| respond(true))>
                    "Skip"
                </Button>
                <div class="flex-1"></div>
                {(count > 1).then(|| view! {
                    <Button variant=ButtonVariant::Ghost size=ButtonSize::Small
                        disabled=Signal::derive(move || busy.get() || index.get() == 0)
                        on_click=Callback::new(move |_| {
                            auto_advance.cancel();
                            draft.update(|draft| draft.position = draft.position.saturating_sub(1));
                        })>
                        "Back"
                    </Button>
                })}
                {move || if index.get() + 1 < count {
                    view! {
                        <Button variant=ButtonVariant::Outline size=ButtonSize::Small
                            disabled=Signal::derive(move || busy.get() || !can_advance())
                            on_click=Callback::new(move |_| advance())>
                            "Next"
                        </Button>
                    }.into_any()
                } else {
                    view! {
                        <Button size=ButtonSize::Small
                            disabled=Signal::derive(move || busy.get() || !can_send())
                            on_click=Callback::new(move |_| advance())>
                            "Send answers"
                        </Button>
                    }.into_any()
                }}
            </div>
        </section>
    }
}
