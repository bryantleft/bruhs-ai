//! A profile's aliases and its linked meetings and conversations.

use bruhs_models::{people_copy, person_conversations, person_meetings, review_alias};
use leptos::prelude::*;

use super::back::PersonOpener;
use crate::state::{AppView, InboxConversationsSignal, MeetingsSignal, PeopleSignal};

const HEADING: &str = "text-label text-rambutan-200";
const EMPTY: &str = "text-label-sm text-rambutan-500";
const LINK: &str = "bruhs-control flex min-h-control-xs w-full min-w-0 items-center justify-between gap-2 rounded-seed px-2 text-left text-body-sm text-rambutan-200 hover:bg-longan-800/70 hover:text-blueberry-200 cursor-pointer";

/// Inspect, add and remove aliases. A new alias is reviewed against every
/// profile first; the server reviews it again before saving.
#[component]
pub fn AliasesPanel(person_id: String) -> impl IntoView {
    let executor = expect_context::<crate::workflow_executor::WorkflowExecutor>();
    let PeopleSignal(people) = expect_context::<PeopleSignal>();
    let id = StoredValue::new(person_id);
    let draft = RwSignal::new(String::new());
    let problem = RwSignal::new(Option::<String>::None);
    let aliases = Memo::new(move |_| {
        people.with(|people| {
            people
                .iter()
                .find(|person| person.id == id.get_value())
                .map(|person| person.aliases.clone())
                .unwrap_or_default()
        })
    });
    let add = move || {
        let reviewed = people.with_untracked(|people| {
            review_alias(people.iter(), &id.get_value(), &draft.get_untracked())
        });
        match reviewed {
            Err(refused) => problem.set(Some(refused.message())),
            Ok(alias) => {
                problem.set(None);
                let mut next = aliases.get_untracked();
                next.push(alias);
                executor.set_person_aliases(id.get_value(), next, move || {
                    draft.try_set(String::new());
                });
            }
        }
    };
    let remove = move |alias: String| {
        let next = aliases
            .get_untracked()
            .into_iter()
            .filter(|current| *current != alias)
            .collect();
        executor.set_person_aliases(id.get_value(), next, || {});
    };
    let input_id = move || format!("person-alias-{}", id.get_value());
    view! {
        <section aria-label="Aliases" class="space-y-2">
            <h3 class=HEADING>{people_copy::ALIASES_HEADING}</h3>
            {move || {
                let list = aliases.get();
                if list.is_empty() {
                    view! { <p class=EMPTY>{people_copy::ALIASES_EMPTY}</p> }.into_any()
                } else {
                    view! {
                        <ul role="list" class="flex flex-wrap gap-1">
                            {list.into_iter().map(|alias| {
                                let label = format!("Remove alias {alias}");
                                let removed = alias.clone();
                                view! {
                                    <li class="inline-flex min-w-0 max-w-full items-center gap-1 rounded-seed bg-longan-800 pl-2 text-label-sm text-rambutan-300 inset-ring inset-ring-longan-700">
                                        <span class="min-w-0 break-words [overflow-wrap:anywhere]">{alias}</span>
                                        <button type="button" aria-label=label
                                            class="bruhs-control inline-flex shrink-0 items-center justify-center rounded-seed px-2 text-label-sm text-rambutan-500 hover:text-lychee-300 cursor-pointer"
                                            on:click=move |_| remove(removed.clone())>"remove"</button>
                                    </li>
                                }
                            }).collect_view()}
                        </ul>
                    }.into_any()
                }
            }}
            <form class="flex gap-2" on:submit=move |event| { event.prevent_default(); add(); }>
                <input type="text" autocomplete="off" id=input_id
                    aria-label=people_copy::ALIAS_INPUT_LABEL
                    placeholder=people_copy::ALIAS_PLACEHOLDER
                    aria-invalid=move || problem.get().is_some().to_string()
                    class="bruhs-control min-h-control min-w-0 flex-1 rounded-grape bg-white/5 px-3 text-body-sm text-rambutan-100 placeholder:text-rambutan-500 inset-ring inset-ring-white/10 focus-visible:outline-2 focus-visible:outline-persimmon-400"
                    prop:value=move || draft.get()
                    on:input=move |event| {
                        draft.set(event_target_value(&event));
                        problem.set(None);
                    } />
                <button type="submit" class="bruhs-control inline-flex min-h-control shrink-0 items-center justify-center rounded-grape bg-blueberry-500/15 px-3 text-label-sm text-blueberry-300 hover:bg-blueberry-500/25 cursor-pointer">
                    {people_copy::ALIAS_BUTTON}
                </button>
            </form>
            {move || problem.get().map(|message| view! {
                <p role="alert" aria-label="Alias not added" class="text-body-sm text-lychee-300">{message}</p>
            })}
        </section>
    }
}

/// The person's meetings and Inbox conversations. Opening one remembers the
/// profile, so that view offers a way back to it.
#[component]
pub fn LinkedRecords(person_id: String) -> impl IntoView {
    let PeopleSignal(people) = expect_context::<PeopleSignal>();
    let MeetingsSignal(meetings) = expect_context::<MeetingsSignal>();
    let InboxConversationsSignal(conversations) = expect_context::<InboxConversationsSignal>();
    let opener = PersonOpener::from_context();
    let id = StoredValue::new(person_id);
    let meeting_rows = Memo::new(move |_| {
        people.with(|people| {
            let Some(person) = people.iter().find(|person| person.id == id.get_value()) else {
                return Vec::new();
            };
            meetings.with(|meetings| {
                person_meetings(person, meetings)
                    .into_iter()
                    .map(|meeting| (meeting.id.clone(), meeting.title.clone()))
                    .collect::<Vec<_>>()
            })
        })
    });
    let conversation_rows = Memo::new(move |_| {
        conversations.with(|conversations| {
            person_conversations(&id.get_value(), conversations)
                .into_iter()
                .map(|conversation| {
                    (
                        conversation.id.clone(),
                        conversation.subject.clone(),
                        conversation.source.label(),
                    )
                })
                .collect::<Vec<_>>()
        })
    });
    view! {
        <div class="grid gap-3 sm:grid-cols-2">
            <section aria-label="Meetings with this person" class="min-w-0 space-y-1">
                <h3 class=HEADING>{people_copy::MEETINGS_HEADING}</h3>
                {move || {
                    let rows = meeting_rows.get();
                    if rows.is_empty() {
                        return view! { <p class=EMPTY>{people_copy::MEETINGS_EMPTY}</p> }.into_any();
                    }
                    view! {
                        <ul role="list" class="space-y-0.5">
                            {rows.into_iter().map(|(meeting_id, title)| {
                                let title = people_copy::meeting_link_title(&title).to_string();
                                let label = format!("Open meeting {title}");
                                view! {
                                    <li>
                                        <button type="button" class=LINK
                                            aria-label=label
                                            on:click=move |_| opener.open(&id.get_value(), AppView::MeetingDetail { meeting_id: meeting_id.clone() })>
                                            <span class="min-w-0 truncate">{title}</span>
                                        </button>
                                    </li>
                                }
                            }).collect_view()}
                        </ul>
                    }.into_any()
                }}
            </section>
            <section aria-label="Conversations with this person" class="min-w-0 space-y-1">
                <h3 class=HEADING>{people_copy::CONVERSATIONS_HEADING}</h3>
                {move || {
                    let rows = conversation_rows.get();
                    if rows.is_empty() {
                        return view! { <p class=EMPTY>{people_copy::CONVERSATIONS_EMPTY}</p> }.into_any();
                    }
                    view! {
                        <ul role="list" class="space-y-0.5">
                            {rows.into_iter().map(|(conversation_id, subject, source)| {
                                let label = format!("Open conversation {subject}");
                                view! {
                                    <li>
                                        <button type="button" class=LINK
                                            aria-label=label
                                            on:click=move |_| opener.open(&id.get_value(), AppView::ConversationDetail { conversation_id: conversation_id.clone() })>
                                            <span class="min-w-0 truncate">{subject}</span>
                                            <span class="shrink-0 text-label-sm text-rambutan-500">{source}</span>
                                        </button>
                                    </li>
                                }
                            }).collect_view()}
                        </ul>
                    }.into_any()
                }}
            </section>
        </div>
    }
}
