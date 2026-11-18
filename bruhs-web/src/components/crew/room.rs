//! A room's page: its members, its transcript with each bruh's face and
//! the chat's question and approval cards, older messages a page at a time,
//! and a composer whose `@` lists only the room's members.

use bruhs_models::crew::room::{
    author_name, member_states, members_line, merge_messages, room_composer_block,
    room_composer_hint, room_follow_key, room_line_key, room_lines, room_markdown, room_post,
    working_line, Room, RoomLine, RoomMessage, APPROVAL_DECIDED, APPROVAL_WANTS, EMPTY_ROOM,
    LOAD_EARLIER, ROOM_GONE, ROOM_LOADING, ROOM_SETTINGS,
};
use bruhs_models::crew::{may_have_more, BruhLook, BruhState, BruhStatus, RAIL_FACE_PX};
use leptos::html;
use leptos::prelude::*;

use super::chat::{approval_card, question_card, CARD};
use super::data::{use_crew_snapshot, CrewData};
use super::face::BruhFace;
use super::face_editor::still_view;
use super::prose::{install_crew_prose, PROSE_BUBBLE};
use super::room_dialog::RoomDialog;
use crate::components::mention_editor::MentionEditor;
use crate::state::{ChipRetargetGuard, ComposerChips};
use crate::ui::{BrailleBruh, Button, ButtonSize, ButtonVariant};

const ERROR: &str = "rounded-grape bg-lychee-500/10 p-3 text-body-sm text-lychee-200";
const OWNER: &str = "min-w-0 max-w-[80%] whitespace-pre-wrap break-words rounded-grape bg-longan-800 px-3 py-2 text-body text-rambutan-100";
const FAILED: &str = "min-w-0 whitespace-pre-wrap break-words rounded-grape bg-lychee-500/10 px-3 py-2 text-body text-lychee-200";

#[component]
pub fn RoomPage(room_id: String) -> impl IntoView {
    let data = use_crew_snapshot();
    // This page's composer keeps its own chips; the chat's draft is untouched.
    provide_context(ComposerChips(RwSignal::new(Vec::new())));
    provide_context(ChipRetargetGuard(StoredValue::new(false)));
    let id = StoredValue::new(room_id.clone());
    let settings_open = RwSignal::new(false);
    // `None` until the crew loads, then whether the room is still there.
    let room = Memo::new(move |_| {
        data.snapshot.with(|snapshot| {
            snapshot.as_ref().map(|snapshot| {
                id.with_value(|id| {
                    snapshot
                        .rooms
                        .iter()
                        .find(|state| state.room.id == *id)
                        .map(|state| state.room.clone())
                })
            })
        })
    });
    let bruhs = Memo::new(move |_| {
        data.snapshot
            .with(|snapshot| snapshot.as_ref().map(|snapshot| snapshot.bruhs.clone()))
            .unwrap_or_default()
    });
    let present = Memo::new(move |_| room.with(|room| matches!(room, Some(Some(_)))));

    // A room deleted from its own settings takes the button focus would go
    // back to, so focus lands on the notice instead of the page body. The
    // room can go before the dialog closes, so closing checks again.
    let gone_ref = NodeRef::<html::H1>::new();
    Effect::new(move |_| {
        let dialog_open = settings_open.get();
        if dialog_open || !room.with(|room| matches!(room, Some(None))) {
            return;
        }
        request_animation_frame(move || {
            let lost = document()
                .active_element()
                .is_none_or(|element| element.tag_name() == "BODY");
            if let (true, Some(Some(heading))) = (lost, gone_ref.try_get_untracked()) {
                let _ = heading.focus();
            }
        });
    });

    view! {
        <section class="flex min-h-0 flex-1 flex-col bg-longan-950 text-rambutan-300" aria-labelledby="room-title" data-room-page="">
            {move || match room.with(|room| room.as_ref().map(Option::is_some)) {
                None => match data.error.get() {
                    Some(message) => view! {
                        <div class="space-y-3 p-6">
                            <p id="room-title" role="alert" class=ERROR>{message}</p>
                            <Button variant=ButtonVariant::Outline size=ButtonSize::Small on_click=Callback::new(move |_| data.reload.run(()))>"Retry"</Button>
                        </div>
                    }.into_any(),
                    None => view! {
                        <p id="room-title" role="status" class="p-6 text-body-sm text-rambutan-400">{ROOM_LOADING}</p>
                    }.into_any(),
                },
                Some(false) => view! {
                    <div class="p-6">
                        <h1 id="room-title" node_ref=gone_ref tabindex="-1" class="text-h3 text-rambutan-100 focus:outline-none">{ROOM_GONE}</h1>
                    </div>
                }.into_any(),
                Some(true) => ().into_any(),
            }}
            <Show when=move || present.get()>
                {room_header(room, bruhs, settings_open)}
                {room_transcript(id, data, bruhs)}
                {room_composer(id, data, room, bruhs)}
            </Show>
            <RoomDialog open=settings_open room_id=room_id />
        </section>
    }
}

fn room_header(
    room: Memo<Option<Option<Room>>>,
    bruhs: Memo<Vec<BruhState>>,
    settings_open: RwSignal<bool>,
) -> impl IntoView {
    let current = move || room.get().flatten();
    let members = Memo::new(move |_| {
        current()
            .map(|room| bruhs.with(|bruhs| member_states(&room, bruhs)))
            .unwrap_or_default()
    });
    let line = move || {
        current()
            .map(|room| bruhs.with(|bruhs| members_line(&room, bruhs)))
            .unwrap_or_default()
    };
    view! {
        <header class="flex shrink-0 flex-wrap items-center justify-between gap-3 border-b border-longan-800/50 px-4 py-3 sm:px-6">
            <div class="min-w-0 flex-1">
                <h1 id="room-title" class="break-words text-h3 text-rambutan-100">{move || current().map(|room| room.name).unwrap_or_default()}</h1>
                <div class="mt-1 flex min-w-0 items-center gap-2">
                    // The line beside the faces names every member for a screen reader.
                    <div aria-hidden="true" class="flex shrink-0 items-center gap-1">
                        <For
                            each=move || members.get()
                            key=|bruh| bruh.agent_id.clone()
                            children=move |bruh: BruhState| {
                                let id = StoredValue::new(bruh.agent_id.clone());
                                let state = Memo::new(move |_| {
                                    id.with_value(|id| members.with(|members| members.iter().find(|bruh| bruh.agent_id == *id).cloned()))
                                });
                                let look = Signal::derive(move || state.get().map(|state| state.look).unwrap_or_default());
                                let status = Signal::derive(move || state.get().map_or(bruh.status, |state| state.status));
                                view! {
                                    <span data-room-face=move || id.get_value() title=move || state.get().map(|state| state.name).unwrap_or_default() class="inline-flex">
                                        <BruhFace size=f64::from(RAIL_FACE_PX) look=look status=status />
                                    </span>
                                }
                            }
                        />
                    </div>
                    <p class="min-w-0 truncate text-label-sm text-rambutan-500">{line}</p>
                </div>
            </div>
            <Button variant=ButtonVariant::Outline size=ButtonSize::Small on_click=Callback::new(move |_| settings_open.set(true))>{ROOM_SETTINGS}</Button>
        </header>
    }
}

fn room_transcript(
    id: StoredValue<String>,
    data: CrewData,
    bruhs: Memo<Vec<BruhState>>,
) -> impl IntoView {
    let messages = RwSignal::new(Vec::<RoomMessage>::new());
    let loaded = RwSignal::new(false);
    let oldest = RwSignal::new(None::<String>);
    let more = RwSignal::new(false);
    let loading_earlier = RwSignal::new(false);
    let load_error = RwSignal::new(None::<String>);
    let revision = StoredValue::new(0u64);
    let alive = RwSignal::new(true);
    on_cleanup(move || alive.set(false));

    // A new message moves the room's `last_seq`: load the newest page again.
    let last_seq = Memo::new(move |_| {
        data.snapshot.with(|snapshot| {
            snapshot.as_ref().and_then(|snapshot| {
                id.with_value(|id| {
                    snapshot
                        .rooms
                        .iter()
                        .find(|state| state.room.id == *id)
                        .map(|state| state.last_seq)
                })
            })
        })
    });
    Effect::new(move |_| {
        // Read, not just tracked: a memo never read never subscribes to the
        // snapshot, so it would never change.
        let _ = last_seq.get();
        revision.update_value(|value| *value = value.wrapping_add(1));
        let requested = revision.get_value();
        let room = id.get_value();
        wasm_bindgen_futures::spawn_local(async move {
            let result = crate::api::room_messages(&room, None).await;
            if alive.try_get_untracked() != Some(true)
                || revision.try_get_value() != Some(requested)
            {
                return;
            }
            match result {
                Ok(page) => {
                    if oldest.get_untracked().is_none() {
                        more.set(may_have_more(page.len()));
                        oldest.set(page.last().map(|message| message.id.clone()));
                    }
                    messages.update(|shown| merge_messages(shown, page));
                    load_error.set(None);
                    loaded.set(true);
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
        let room = id.get_value();
        wasm_bindgen_futures::spawn_local(async move {
            let result = crate::api::room_messages(&room, Some(&before)).await;
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
                    messages.update(|shown| merge_messages(shown, page));
                }
                Err(error) => load_error.set(Some(error.to_string())),
            }
        });
    };

    let lines = Memo::new(move |_| {
        data.snapshot.with(|snapshot| {
            let (runs, outbox) = snapshot.as_ref().map_or((&[][..], &[][..]), |snapshot| {
                (&snapshot.runs[..], &snapshot.outbox[..])
            });
            messages.with(|messages| id.with_value(|id| room_lines(messages, runs, outbox, id)))
        })
    });

    // Code, math and diagrams in the bruhs' Markdown, and its links.
    install_crew_prose(move || {
        lines.with(|lines| {
            lines
                .iter()
                .filter_map(room_markdown)
                .map(str::to_string)
                .collect()
        })
    });

    // Follow the room: a new line scrolls it into view; Load earlier does not.
    let scroller = NodeRef::<html::Div>::new();
    Effect::new(move |previous: Option<(Option<String>, Option<String>)>| {
        let shown = lines.with(|lines| room_follow_key(lines));
        if previous.as_ref() != Some(&shown) {
            if let Some(element) = scroller.get_untracked() {
                element.set_scroll_top(element.scroll_height());
            }
        }
        shown
    });

    view! {
        <div node_ref=scroller class="min-h-0 flex-1 overflow-y-auto">
            <div class="mx-auto flex w-full max-w-3xl min-w-0 flex-col gap-3 p-4 sm:px-6" role="log" aria-label="Room messages">
                <Show when=move || more.get()>
                    <div class="flex justify-center">
                        <Button variant=ButtonVariant::Ghost size=ButtonSize::Small disabled=Signal::derive(move || loading_earlier.get()) on_click=Callback::new(load_earlier)>{LOAD_EARLIER}</Button>
                    </div>
                </Show>
                {move || load_error.get().map(|message| view! { <p role="alert" class=ERROR>{message}</p> })}
                <Show when=move || loaded.get() && lines.with(Vec::is_empty) && load_error.get().is_none()>
                    <p class="text-body-sm text-rambutan-500">{EMPTY_ROOM}</p>
                </Show>
                <For
                    each=move || lines.get()
                    key=room_line_key
                    children=move |line| room_line(line, bruhs, data)
                />
            </div>
        </div>
    }
}

/// A bruh's current look, or the default once it has left the crew.
fn look_of(agent_id: StoredValue<String>, bruhs: Memo<Vec<BruhState>>) -> Signal<BruhLook> {
    Signal::derive(move || {
        agent_id.with_value(|id| {
            bruhs.with(|bruhs| {
                bruhs
                    .iter()
                    .find(|bruh| bruh.agent_id == *id)
                    .map(|bruh| bruh.look.clone())
                    .unwrap_or_default()
            })
        })
    })
}

/// A line a bruh wrote: its face, its current name and `body`.
fn by_bruh(agent_id: String, bruhs: Memo<Vec<BruhState>>, body: AnyView) -> AnyView {
    let id = StoredValue::new(agent_id);
    let look = look_of(id, bruhs);
    let name = move || id.with_value(|id| bruhs.with(|bruhs| author_name(id, bruhs)));
    view! {
        <div class="flex min-w-0 items-start gap-2" data-room-author=move || id.get_value()>
            <BrailleBruh size=f64::from(RAIL_FACE_PX) view=Signal::derive(move || look.with(|look| still_view(look, BruhStatus::Working))) />
            // The column holds the chat's 80%; a card in it takes the column's width.
            <div class="flex min-w-0 max-w-[80%] flex-1 flex-col items-start gap-1 [&>article]:max-w-none">
                <span class="text-label-sm text-rambutan-400">{name}</span>
                {body}
            </div>
        </div>
    }
    .into_any()
}

fn room_line(line: RoomLine, bruhs: Memo<Vec<BruhState>>, data: CrewData) -> AnyView {
    match line {
        RoomLine::Owner { text, .. } => view! {
            <div class="flex min-w-0 flex-col items-end" data-room-line="owner">
                <p class=OWNER>{text}</p>
            </div>
        }
        .into_any(),
        RoomLine::Bruh {
            agent_id,
            text,
            failed: true,
            ..
        } => by_bruh(
            agent_id,
            bruhs,
            view! { <p data-room-line="bruh" class=FAILED>{text}</p> }.into_any(),
        ),
        RoomLine::Bruh { agent_id, text, .. } => by_bruh(
            agent_id,
            bruhs,
            view! { <div data-room-line="bruh" class=PROSE_BUBBLE inner_html=crate::markdown::render_untrusted(&text)></div> }
                .into_any(),
        ),
        RoomLine::System { text, .. } => view! {
            <p data-room-line="system" class="text-center text-label-sm text-rambutan-500">{text}</p>
        }
        .into_any(),
        RoomLine::Card { agent_id, item, .. } => {
            let card = if item.approval.is_some() {
                approval_card(item, data)
            } else {
                question_card(item, data)
            };
            by_bruh(agent_id, bruhs, card)
        }
        RoomLine::Asked {
            agent_id, summary, ..
        } => by_bruh(
            agent_id,
            bruhs,
            view! {
                <article class=CARD aria-label="Approval" data-room-line="asked">
                    <p class="text-label-sm text-rambutan-400">{APPROVAL_WANTS}</p>
                    <p class="whitespace-pre-wrap break-words text-body text-rambutan-100">{summary}</p>
                    <p class="text-label-sm text-rambutan-400">{APPROVAL_DECIDED}</p>
                </article>
            }
            .into_any(),
        ),
        RoomLine::Working { agent_id, .. } => {
            let id = StoredValue::new(agent_id);
            let look = look_of(id, bruhs);
            let text = move || id.with_value(|id| bruhs.with(|bruhs| working_line(&author_name(id, bruhs))));
            view! {
                // The log announces it; no live region of its own.
                <div class="flex min-w-0 items-center gap-2" data-room-line="working">
                    <BruhFace size=f64::from(RAIL_FACE_PX) look=look status=BruhStatus::Working />
                    <span class="text-label-sm text-rambutan-400">{text}</span>
                </div>
            }
            .into_any()
        }
    }
}

fn room_composer(
    id: StoredValue<String>,
    data: CrewData,
    room: Memo<Option<Option<Room>>>,
    bruhs: Memo<Vec<BruhState>>,
) -> impl IntoView {
    let ComposerChips(chips) = expect_context::<ComposerChips>();
    let ChipRetargetGuard(guard) = expect_context::<ChipRetargetGuard>();
    let (draft, set_draft) = signal(String::new());
    let members = Memo::new(move |_| {
        room.with(|room| room.as_ref().and_then(Option::as_ref).cloned())
            .map(|room| bruhs.with(|bruhs| member_states(&room, bruhs)))
            .unwrap_or_default()
    });
    let lead = Signal::derive(move || room.get().flatten().and_then(|room| room.lead));
    let blocked = Memo::new(move |_| {
        room_composer_block(
            data.rooms_ready(),
            data.rooms_reason(),
            members.with(Vec::len),
        )
    });
    let hint = move || {
        room.with(|room| {
            room.as_ref()
                .and_then(Option::as_ref)
                .and_then(room_composer_hint)
        })
        .filter(|_| blocked.get().is_none())
    };
    let send = move || {
        if blocked.get_untracked().is_some() {
            return;
        }
        let command = chips.with_untracked(|chips| {
            draft.with_untracked(|text| id.with_value(|id| room_post(id, text, chips)))
        });
        let Some(command) = command else {
            return;
        };
        data.send.run(command);
        guard.set_value(true);
        chips.set(Vec::new());
        set_draft.set(String::new());
    };

    view! {
        <div class="shrink-0 border-t border-longan-800/50">
            <div class="mx-auto w-full max-w-3xl min-w-0 space-y-2 p-4 sm:px-6">
                {move || data.error.get().map(|message| view! { <p role="alert" class=ERROR>{message}</p> })}
                {move || blocked.get().filter(|reason| !reason.is_empty()).map(|reason| view! { <p data-room-blocked="" class="text-label-sm text-rambutan-400">{reason}</p> })}
                <div
                    data-composer-surface=""
                    data-room-composer=""
                    class="relative rounded-[16px] bg-longan-900 inset-ring inset-ring-white/10 transition-[box-shadow] focus-within:inset-ring-persimmon-400/40"
                >
                    <MentionEditor
                        value=draft
                        set_value=set_draft
                        project_path=Signal::derive(|| None::<String>)
                        on_submit=Callback::new(move |()| send())
                        placeholder="Message"
                        chrome=false
                        disabled=Signal::derive(move || blocked.get().is_some())
                        members=members
                        lead=lead
                    />
                    <div class="flex items-center justify-end px-2 pb-2">
                        <Button
                            size=ButtonSize::Small
                            disabled=Signal::derive(move || blocked.get().is_some() || draft.with(|text| text.trim().is_empty()))
                            on_click=Callback::new(move |_| send())
                        >"Send"</Button>
                    </div>
                </div>
                {move || hint().map(|hint| view! { <p data-room-hint="" class="text-label-sm text-rambutan-500">{hint}</p> })}
            </div>
        </div>
    }
}
