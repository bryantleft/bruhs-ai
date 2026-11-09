//! Past-meeting detail view: transcript + summary. Phase 5 wires the
//! Summary section to render MeetingSummary (TLDR, action items, decisions,
//! topics) and adds a Re-summarize button.
//!
//! Phase 3 Task 6 adds speaker-labeled transcript bubbles with inline rename:
//! - Segments are grouped into contiguous same-speaker runs.
//! - Each run gets a SpeakerChip above it (dragonfruit for "me", pandan for
//!   "them", rambutan/longan for unknown/unassigned).
//! - Clicking a chip opens an inline text editor; blur/Enter commits via PATCH,
//!   Escape cancels. Optimistic update reverts on API failure.

use bruhs_models::num::ToIntSaturating;
use leptos::prelude::*;

use crate::ui::Checkbox;
use wasm_bindgen::JsCast;
use web_sys::{HtmlInputElement, KeyboardEvent};

use crate::api;
use crate::state::{
    AppView, ColorIndex, Meeting, MeetingSegmentsSignal, MeetingsSignal, PeopleSignal,
    PersonalitySettingsSignal, SetActiveView, SetMeetingsSignal, Speaker, SpeakerIdentity,
    TranscriptSegment,
};

// ── MeetingDetailPage ─────────────────────────────────────────────────────────

/// Full-width routed detail page for a single meeting. Accessed via
/// `AppView::MeetingDetail { meeting_id }`. Replaces the 420px fixed
/// sidebar overlay from previous phases.
#[component]
pub fn MeetingDetailPage(meeting_id: String) -> impl IntoView {
    let MeetingsSignal(meetings) = expect_context::<MeetingsSignal>();
    let MeetingSegmentsSignal(meeting_segments) = expect_context::<MeetingSegmentsSignal>();
    let set_active_thread_id = expect_context::<crate::state::SetActiveThreadId>().0;
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();
    let PersonalitySettingsSignal(personality) = expect_context::<PersonalitySettingsSignal>();
    let meeting_actions = expect_context::<crate::meeting_actions::MeetingActions>();
    let note_executor = expect_context::<crate::workflow_executor::WorkflowExecutor>();

    let mid = StoredValue::new(meeting_id);
    let transcript_sync = expect_context::<crate::transcript_sync::TranscriptSync>();
    Effect::new(move |_| transcript_sync.refresh(mid.get_value()));
    let transcript_status = Memo::new(move |_| transcript_sync.status(&mid.get_value()));
    let action_status = Memo::new(move |_| meeting_actions.status(&mid.get_value()));
    let actions_disabled = Memo::new(move |_| {
        let status = action_status.get();
        status.busy || status.recovery
    });

    let current: Memo<Option<Meeting>> = Memo::new(move |_| {
        let id = mid.get_value();
        meetings.with(|meetings| meetings.iter().find(|m| m.id == id).cloned())
    });
    // Summary/setting receipts must not recreate speaker editors or disturb
    // an in-progress transcript selection when the transcript itself is unchanged.
    let transcript_identity = Memo::new(move |_| {
        current.with(|meeting| {
            meeting
                .as_ref()
                .map(|meeting| (meeting.id.clone(), meeting.speakers.clone()))
        })
    });

    let segments_memo: Memo<Vec<TranscriptSegment>> = Memo::new(move |_| {
        let id = mid.get_value();
        meeting_segments.with(|map| map.get(&id).cloned().unwrap_or_default())
    });

    // Memoize the grouping so it only recomputes when segments change,
    // not on every render (chip-edit signals, busy flags, etc.).
    let groups_memo = Memo::new(move |_| group_by_speaker(&segments_memo.get()));

    let on_resummarize = move |_: web_sys::MouseEvent| {
        let Some(m) = current.get() else { return };
        let extract = personality.get_untracked().extract_observations;
        meeting_actions.generate(m.id, extract, false);
    };

    let on_toggle_skip = move |new_skip: bool| {
        let Some(m) = current.get() else { return };
        meeting_actions.set_skip(m.id, new_skip);
    };

    let on_reextract = move |_: web_sys::MouseEvent| {
        let Some(m) = current.get() else { return };
        let extract = personality.get_untracked().extract_observations;
        if !extract || m.skip_observations || m.summary.is_none() {
            return;
        }
        meeting_actions.generate(m.id, extract, true);
    };

    view! {
        <div class="flex flex-col h-full bg-longan-950 text-rambutan-300 overflow-auto">
            <div class="max-w-3xl mx-auto w-full px-6 py-4 space-y-6">
                // Back button + title header
                <header class="flex items-center gap-3">
                    <button
                        type="button"
                        class="text-label text-rambutan-400 hover:text-rambutan-200 inset-ring inset-ring-longan-700 rounded-seed px-2 py-0.5 cursor-pointer"
                        on:click=move |_| set_active_view.set(AppView::Meetings)
                    >
                        "← Back"
                    </button>
                    <h1 class="text-body-lg text-rambutan-100">
                        {move || current.get().map_or_else(|| "Meeting".into(), |m| m.title)}
                    </h1>
                </header>

                <button type="button" disabled=move || !note_executor.ready.get() || current.with(Option::is_none)
                    class="min-h-11 rounded-grape px-3 py-2 text-label-sm text-persimmon-300 hover:bg-longan-800 disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400"
                    on:click=move |_| { if let Some(meeting) = current.get_untracked() {
                        note_executor.note_editor.begin_linked(meeting.id);
                        set_active_view.set(AppView::Meetings);
                    } }>"New linked note"</button>

                // ── Transcript ──────────────────────────────────────────────
                <section>
                    <div class="text-label-sm text-rambutan-400 mb-2">"Transcript"</div>
                    <Show when=move || transcript_status.get().loading>
                        <p role="status" aria-label="Transcript loading" class="text-label-sm text-rambutan-400 mb-2">"Loading transcript… Saved content remains available."</p>
                    </Show>
                    {move || transcript_status.get().warning.map(|warning| view! {
                        <section aria-label="Transcript recovery" class="rounded-grape bg-longan-900 p-3 mb-2 inset-ring inset-ring-longan-800 space-y-2">
                            <p role="status" class="text-body-sm text-rambutan-300">{warning}</p>
                            <button type="button" class="rounded-grape bg-longan-800 px-3 py-2 min-h-11 sm:min-h-6 text-label-sm cursor-pointer disabled:opacity-40" disabled=move || transcript_status.get().loading on:click=move |_| transcript_sync.refresh(mid.get_value())>"Retry loading transcript"</button>
                        </section>
                    })}
                    {move || {
                        let Some((meeting_id, speakers)) = transcript_identity.get() else {
                            return ().into_any();
                        };
                        let groups = groups_memo.get();
                        if groups.is_empty() {
                            view! {
                                <div class="rounded-grape inset-ring inset-ring-longan-800 bg-longan-900/40 px-3 py-2 text-body-sm text-rambutan-500">
                                    {move || if transcript_status.get().loading { "Loading transcript…" } else if transcript_status.get().warning.is_some() { "Transcript is unavailable. Use Retry loading transcript to try again." } else { "No transcript yet — segments appear after Stop while Whisper processes the audio." }}
                                </div>
                            }.into_any()
                        } else {
                            view! {
                                <div class="space-y-3">
                                    {groups.into_iter().map(|(speaker_id, segs)| {
                                        let label = speaker_label(&speakers, speaker_id.as_deref());
                                        let (text_class, ring_class) = speaker_colors(speaker_id.as_deref());
                                        let chip_meeting_id = meeting_id.clone();
                                        let chip_speaker_id = speaker_id.clone();
                                        view! {
                                            <div class="space-y-1">
                                                <div class="flex items-center">
                                                    <SpeakerChip
                                                        meeting_id=chip_meeting_id
                                                        speaker_id=chip_speaker_id
                                                        label=label
                                                        text_class=text_class
                                                        ring_class=ring_class
                                                    />
                                                </div>
                                                <div class="space-y-1 pl-3">
                                                    {segs.into_iter().map(|seg| {
                                                        let opacity = if seg.partial { " opacity-50" } else { "" };
                                                        let conf_badge = seg.speaker_confidence
                                                            .filter(|&c| c < 0.85)
                                                            .map(|c| {
                                                                let pct = (c * 100.0).to_u32_saturating();
                                                                view! {
                                                                    <span class="text-label-sm text-rambutan-500 ml-1">
                                                                        {format!("{pct}%")}
                                                                    </span>
                                                                }
                                                            });
                                                        view! {
                                                            <div class=format!("flex items-baseline text-body-sm text-rambutan-200 leading-snug{opacity}")>
                                                                <span>{seg.text}</span>
                                                                {conf_badge}
                                                            </div>
                                                        }
                                                    }).collect_view()}
                                                </div>
                                            </div>
                                        }
                                    }).collect_view()}
                                </div>
                            }.into_any()
                        }
                    }}
                </section>

                // ── Summary ────────────────────────────────────────────────
                {move || action_status.get().message.map(|message| view! {
                    <section aria-label=move || if action_status.get().recovery { "Meeting recovery" } else { "Meeting operation status" }
                        class="rounded-grape bg-longan-900 p-3 inset-ring inset-ring-longan-800 space-y-2">
                        // Native announces recovery as an alert; keep the same semantics here.
                        <p role=move || if action_status.get().recovery { "alert" } else { "status" } class="text-body-sm text-rambutan-300">{message}</p>
                        <Show when=move || action_status.get().recovery>
                            <button type="button" class="rounded-grape bg-longan-800 px-3 py-2 min-h-11 sm:min-h-6 text-label-sm cursor-pointer disabled:opacity-40"
                                disabled=move || action_status.get().busy on:click=move |_| meeting_actions.retry(mid.get_value())>"Load latest meeting"</button>
                        </Show>
                        <Show when=move || action_status.get().cancellable>
                            <button type="button" class="rounded-grape bg-longan-800 px-3 py-2 min-h-11 sm:min-h-6 text-label-sm cursor-pointer disabled:opacity-40"
                                disabled=move || action_status.get().busy on:click=move |_| meeting_actions.cancel_generation(mid.get_value())>"Cancel saved generation"</button>
                        </Show>
                        <Show when=move || action_status.get().foreign_pending>
                            <button type="button" class="rounded-grape bg-longan-800 px-3 py-2 min-h-11 sm:min-h-6 text-label-sm cursor-pointer disabled:opacity-40"
                                disabled=move || action_status.get().busy on:click=move |_| meeting_actions.cancel_foreign(mid.get_value())>"Cancel unresolved server request"</button>
                        </Show>
                    </section>
                })}
                <section>
                    <div class="flex items-center justify-between mb-2">
                        <div class="text-label-sm text-rambutan-400">"Summary"</div>
                        {move || current.get().map(|m| {
                                view! {
                                    <div class="flex items-center gap-1">
                                        <button
                                            type="button"
                                            class="min-h-11 sm:min-h-6 px-2 text-label-sm text-rambutan-400 hover:text-rambutan-200 cursor-pointer disabled:opacity-40"
                                            disabled=move || action_status.get().busy
                                            on:click=move |_| meeting_actions.discover_pending(mid.get_value())
                                        >
                                            "Check server requests"
                                        </button>
                                        <button
                                            type="button"
                                            on:click=on_resummarize
                                            class="min-h-11 sm:min-h-6 px-2 text-label-sm text-rambutan-400 hover:text-rambutan-200 cursor-pointer disabled:opacity-40"
                                            disabled=move || actions_disabled.get()
                                        >
                                            {if m.summary.is_some() { "Re-summarize" } else { "Generate summary" }}
                                        </button>
                                    </div>
                                }
                        })}
                    </div>
                    {move || {
                        let Some(m) = current.get() else { return ().into_any(); };
                        match m.summary {
                            None => view! {
                                <div class="rounded-grape inset-ring inset-ring-longan-800 bg-longan-900/40 px-3 py-2 text-body-sm text-rambutan-500">
                                    {move || if action_status.get().busy { "Working on this meeting…" } else { "Generate a summary from the saved transcript." }}
                                </div>
                            }.into_any(),
                            Some(s) => view! {
                                <div class="space-y-2">
                                    <div class="rounded-grape inset-ring inset-ring-longan-800 bg-longan-900/30 px-3 py-2 text-body-sm text-rambutan-200">
                                        {s.tldr}
                                    </div>
                                    {if !s.action_items.is_empty() {
                                        view! {
                                            <div>
                                                <div class="text-label text-rambutan-500 mb-0.5">"Action items"</div>
                                                <ul role="list" class="space-y-0.5">
                                                    {s.action_items.into_iter().map(|a| {
                                                        let due = a.due_hint.clone();
                                                        view! {
                                                            <li class="text-label text-rambutan-300">
                                                                "• " {a.text}
                                                                {due.map(|h| view! { <span class="text-rambutan-500">" — " {h}</span> })}
                                                            </li>
                                                        }
                                                    }).collect_view()}
                                                </ul>
                                            </div>
                                        }.into_any()
                                    } else { ().into_any() }}
                                    {if !s.decisions.is_empty() {
                                        view! {
                                            <div>
                                                <div class="text-label text-rambutan-500 mb-0.5">"Decisions"</div>
                                                <ul role="list" class="space-y-0.5">
                                                    {s.decisions.into_iter().map(|d| {
                                                        view! { <li class="text-label text-rambutan-300">"• " {d}</li> }
                                                    }).collect_view()}
                                                </ul>
                                            </div>
                                        }.into_any()
                                    } else { ().into_any() }}
                                    {if !s.topics.is_empty() {
                                        view! {
                                            <div>
                                                <div class="text-label text-rambutan-500 mb-0.5">"Topics"</div>
                                                {s.topics.into_iter().map(|t| {
                                                    view! {
                                                        <div class="mb-1">
                                                            <div class="text-label text-rambutan-200">{t.title}</div>
                                                            <ul role="list" class="ml-2 space-y-0.5">
                                                                {t.bullets.into_iter().map(|b| {
                                                                    view! { <li class="text-label-sm text-rambutan-400">"– " {b}</li> }
                                                                }).collect_view()}
                                                            </ul>
                                                        </div>
                                                    }
                                                }).collect_view()}
                                            </div>
                                        }.into_any()
                                    } else { ().into_any() }}
                                </div>
                            }.into_any(),
                        }
                    }}
                </section>

                // ── Observation extraction ────────────────────────────────
                <section>
                    <div class="text-label-sm text-rambutan-400 mb-2">"Observation extraction"</div>
                    {move || {
                        let Some(m) = current.get() else { return ().into_any(); };
                        let skip = m.skip_observations;
                        let has_summary = m.summary.is_some();
                        let reextract_disabled = skip || !has_summary || actions_disabled.get() || !personality.get().extract_observations;
                        view! {
                            <div class="space-y-2">
                                <div class="flex items-center gap-2 cursor-pointer select-none">
                                    <Checkbox
                                        id="skip-obs-checkbox".to_string()
                                        checked=skip
                                        disabled=Signal::derive(move || actions_disabled.get())
                                        class="min-h-11 min-w-11 sm:min-h-6 sm:min-w-6"
                                        on_change=Callback::new(on_toggle_skip)
                                    />
                                    <label
                                        for="skip-obs-checkbox"
                                        class="text-label text-rambutan-400 cursor-pointer"
                                    >
                                        "Skip observation extraction for this meeting"
                                    </label>
                                </div>
                                <button
                                    type="button"
                                    class="min-h-11 sm:min-h-6 text-label-sm text-rambutan-400 hover:text-rambutan-200 cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed inset-ring inset-ring-longan-700 rounded-seed px-2 py-0.5"
                                    disabled=reextract_disabled
                                    on:click=on_reextract
                                >
                                    "Re-extract observations"
                                </button>
                                <Show when=move || !personality.get().extract_observations>
                                    <p class="text-label-sm text-rambutan-400">"Enable observation extraction in Settings to re-extract."</p>
                                </Show>
                            </div>
                        }.into_any()
                    }}
                </section>

                // ── Chat ──────────────────────────────────────────────────
                <section>
                    <div class="text-label-sm text-rambutan-400 mb-2">"Chat"</div>
                    {move || {
                        let Some(m) = current.get() else { return ().into_any(); };
                        let mid_val = m.id;
                        let on_chat_click = move |_: web_sys::MouseEvent| {
                            let mid_val = mid_val.clone();
                            wasm_bindgen_futures::spawn_local(async move {
                                match crate::api::create_or_get_meeting_chat(&mid_val).await {
                                    Ok(thread) => {
                                        set_active_thread_id.set(Some(thread.id.into_inner()));
                                        set_active_view.set(AppView::Chat);
                                    }
                                    Err(e) => log::warn!("create_or_get_meeting_chat failed: {e}"),
                                }
                            });
                        };
                        view! {
                            <button
                                type="button"
                                class="text-label text-rambutan-200 inset-ring inset-ring-longan-700 rounded-seed px-2 py-1 cursor-pointer hover:inset-ring-persimmon-500"
                                on:click=on_chat_click
                            >
                                "Chat with this meeting"
                            </button>
                        }.into_any()
                    }}
                </section>
            </div>
        </div>
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Bruhs color tokens for each speaker slot. Returns `(text_class, ring_class)`.
///
/// Delegates the semantic classification to [`SpeakerIdentity::resolve`] and
/// only owns the palette mapping. The cluster palette cycles modulo 4 so
/// multi-speaker meetings stay visually distinct.
pub fn speaker_colors(speaker_id: Option<&str>) -> (&'static str, &'static str) {
    let identity = SpeakerIdentity::resolve(speaker_id, &[]);
    match identity.color_index {
        ColorIndex::Me => ("text-dragonfruit-300", "inset-ring-dragonfruit-700"),
        ColorIndex::Them => ("text-pandan-300", "inset-ring-pandan-700"),
        ColorIndex::Cluster(n) => match n % 4 {
            0 => ("text-pandan-300", "inset-ring-pandan-700"),
            1 => ("text-lychee-300", "inset-ring-lychee-700"),
            2 => ("text-blueberry-400", "inset-ring-blueberry-500"),
            _ => ("text-durian-300", "inset-ring-durian-700"),
        },
        ColorIndex::Unknown => ("text-rambutan-300", "inset-ring-longan-700"),
    }
}

/// Resolve a display label for a speaker id from the meeting's speakers list.
/// Delegates to [`SpeakerIdentity::resolve`] — the prefix grammar
/// (`me` / `them` / `them_N` / `name:X`) lives in `bruhs-models::speaker_id`.
pub fn speaker_label(speakers: &[Speaker], id: Option<&str>) -> String {
    SpeakerIdentity::resolve(id, speakers).display
}

/// Group a flat segment list into contiguous runs of the same speaker.
/// Each entry is `(speaker_id, Vec<TranscriptSegment>)`.
pub fn group_by_speaker(
    segments: &[TranscriptSegment],
) -> Vec<(Option<String>, Vec<TranscriptSegment>)> {
    let mut groups: Vec<(Option<String>, Vec<TranscriptSegment>)> = Vec::new();
    for seg in segments {
        let id = seg.speaker_id.clone();
        if let Some(last) = groups.last_mut() {
            if last.0 == id {
                last.1.push(seg.clone());
                continue;
            }
        }
        groups.push((id, vec![seg.clone()]));
    }
    groups
}

// ── SpeakerChip ───────────────────────────────────────────────────────────────

/// A small clickable chip that shows a speaker label and switches to an inline
/// text editor on click. Commits via `api::update_speaker_label` with optimistic
/// update and revert on failure. Disabled (non-interactive) when `speaker_id`
/// is `None`.
///
/// Phase 3d: when interactive and speakers are clustered (them_N ids), a `▾`
/// menu icon appears. Clicking it reveals a "Link to person..." sub-list of
/// all PersonProfiles. Selecting one calls the POST link route and updates
/// the chip label optimistically.
#[component]
pub fn SpeakerChip(
    meeting_id: String,
    speaker_id: Option<String>,
    label: String,
    text_class: &'static str,
    ring_class: &'static str,
) -> impl IntoView {
    let (editing, set_editing) = signal(false);
    let (current_label, set_current_label) = signal(label.clone());
    let (draft, set_draft) = signal(label);
    // Phase 3d: link-to-person dropdown.
    let (menu_open, set_menu_open) = signal(false);

    let PeopleSignal(people) = expect_context::<PeopleSignal>();
    let SetMeetingsSignal(set_meetings) = expect_context::<SetMeetingsSignal>();
    let workflow_executor = expect_context::<crate::workflow_executor::WorkflowExecutor>();
    let transcript_sync = expect_context::<crate::transcript_sync::TranscriptSync>();

    // NodeRef so we can auto-focus the input when entering edit mode.
    let input_ref: NodeRef<leptos::html::Input> = NodeRef::new();
    // NodeRef for the dropdown container — used for outside-click detection.
    let menu_ref: NodeRef<leptos::html::Div> = NodeRef::new();

    // Auto-focus when the editing signal becomes true.
    Effect::new(move |_| {
        if editing.get() {
            if let Some(el) = input_ref.get() {
                let _ = el.focus();
                // Select all text so the user can immediately overtype.
                el.select();
            }
        }
    });

    // Outside-click: close the ▾ menu when the user clicks outside its container.
    // We store the raw JS function pointer so we can remove it on cleanup.
    {
        use std::cell::RefCell;
        use std::rc::Rc;
        use wasm_bindgen::closure::Closure;
        type MenuClosure = Closure<dyn FnMut(web_sys::MouseEvent)>;
        let shared: Rc<RefCell<Option<MenuClosure>>> = Rc::new(RefCell::new(None));
        let shared_cleanup = StoredValue::new_local(Rc::clone(&shared));
        let cl = Closure::<dyn FnMut(web_sys::MouseEvent)>::new(move |ev: web_sys::MouseEvent| {
            if !menu_open.get_untracked() {
                return;
            }
            let inside = menu_ref
                .get_untracked()
                .and_then(|container| {
                    ev.target()
                        .and_then(|t| t.dyn_into::<web_sys::Node>().ok())
                        .map(|node| container.contains(Some(&node)))
                })
                .unwrap_or(false);
            if !inside {
                set_menu_open.set(false);
            }
        });
        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
            let _ = doc.add_event_listener_with_callback("mousedown", cl.as_ref().unchecked_ref());
        }
        *shared.borrow_mut() = Some(cl);
        on_cleanup(move || {
            let rc = shared_cleanup.get_value();
            let cb_opt = rc.borrow();
            if let Some(cl) = cb_opt.as_ref() {
                if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                    let _ = doc.remove_event_listener_with_callback(
                        "mousedown",
                        cl.as_ref().unchecked_ref(),
                    );
                }
            }
        });
    }

    // Store meeting_id and speaker_id in StoredValue so event handlers can
    // clone them on every invocation without consuming the closure (Fn not
    // FnOnce requirement from the Leptos event-attribute macros).
    let mid_stored = StoredValue::new(meeting_id);
    let sid_stored = StoredValue::new(speaker_id.clone());

    let is_interactive = speaker_id.is_some();
    // Only show the link menu for clustered speaker ids (them_N pattern).
    let is_linkable = speaker_id
        .as_deref()
        .is_some_and(|id| id.starts_with("them_") || id == "them");

    let on_input = move |ev: web_sys::Event| {
        if let Some(target) = ev.target() {
            if let Ok(input) = target.dyn_into::<HtmlInputElement>() {
                set_draft.set(input.value());
            }
        }
    };

    // Commit a new label value: optimistic update → API call → revert on error.
    // Declared as a plain `move` closure that clones its StoredValues each call
    // so it implements `Fn` (not `FnOnce`).
    //
    // The re-entry guard (`!editing.get_untracked()`) prevents double-commits when
    // Enter → blur fires both commit_for_keydown and commit_for_blur, or when
    // Escape → blur fires similarly.
    let commit = move |new: String| {
        if !editing.try_get_untracked().unwrap_or(false) {
            return;
        }
        let mid = mid_stored.get_value();
        let sid = sid_stored.get_value();
        let new_clone = new.clone();
        let prev = current_label.get_untracked();
        set_current_label.set(new);
        set_editing.set(false);
        if let Some(sid) = sid {
            wasm_bindgen_futures::spawn_local(async move {
                match api::update_speaker_label(&mid, &sid, new_clone).await {
                    Ok(updated) => transcript_sync.cache_meeting(updated).await,
                    Err(e) => {
                        log::warn!("speaker rename failed: {e}");
                        let _ = set_current_label.try_set(prev);
                        transcript_sync.report(&mid, "Speaker rename could not be saved. The previous label was retained; retry when connected.".into());
                    }
                }
            });
        }
    };

    let commit_for_keydown = commit;
    let commit_for_blur = commit;

    let on_keydown = move |ev: KeyboardEvent| {
        match ev.key().as_str() {
            "Enter" => {
                ev.prevent_default();
                commit_for_keydown(draft.get_untracked());
            }
            "Escape" => {
                ev.prevent_default();
                // Revert draft to committed label and close editor.
                set_draft.set(current_label.get_untracked());
                set_editing.set(false);
            }
            _ => {}
        }
    };

    let on_blur = move |_: web_sys::FocusEvent| {
        // DOM removal can dispatch blur after this editor's reactive owner
        // was disposed. An unmounted editor must not read dead draft signals.
        if let Some(value) = draft.try_get_untracked() {
            commit_for_blur(value);
        }
    };

    // Phase 3d: link a speaker to a chosen PersonProfile.
    // All captures are Copy (signals/StoredValue), so this closure is Fn.
    let do_link = move |person_id: String, person_name: String| {
        let mid = mid_stored.get_value();
        let sid_opt = sid_stored.get_value();
        set_menu_open.set(false);
        // Optimistic label update.
        set_current_label.set(person_name);
        if let Some(sid) = sid_opt {
            wasm_bindgen_futures::spawn_local(async move {
                match api::link_speaker_to_person(&mid, &sid, &person_id).await {
                    Ok(updated_meeting) => {
                        // Update the meeting in the global signal.
                        set_meetings.update(|list| {
                            if let Some(slot) = list.iter_mut().find(|m| m.id == updated_meeting.id)
                            {
                                *slot = updated_meeting;
                            }
                        });
                        // Refresh people list (PeopleUpdated was emitted server-side).
                        workflow_executor.refresh_people().await;
                    }
                    Err(e) => {
                        log::warn!("link_speaker_to_person failed: {e}");
                    }
                }
            });
        }
    };

    view! {
        <Show
            when=move || editing.get()
            fallback=move || {
                let label_now = current_label.get();
                if is_interactive {
                    view! {
                        <div class="relative inline-flex items-center gap-0.5">
                            <button
                                type="button"
                                class=format!(
                                    "text-label {text_class} \
                                     inset-ring {ring_class} rounded-seed \
                                     px-2 py-0.5 min-h-11 min-w-11 sm:min-h-6 sm:min-w-6 cursor-pointer \
                                     hover:opacity-80 transition-opacity"
                                )
                                on:click=move |_| {
                                    set_menu_open.set(false);
                                    set_draft.set(current_label.get_untracked());
                                    set_editing.set(true);
                                }
                            >
                                {label_now}
                            </button>

                            // ▾ menu button (only for linkable speakers)
                            <Show when=move || is_linkable>
                                <button
                                    type="button"
                                    class=format!(
                                        "text-label-sm {text_class} opacity-60 \
                                         hover:opacity-100 transition-opacity px-0.5 min-h-11 min-w-11 sm:min-h-6 sm:min-w-6 cursor-pointer"
                                    )
                                    on:click=move |ev| {
                                        ev.stop_propagation();
                                        set_menu_open.update(|o| *o = !*o);
                                    }
                                    title="Link to person..."
                                >
                                    "▾"
                                </button>

                                // Dropdown
                                <Show when=move || menu_open.get()>
                                    <div
                                        node_ref=menu_ref
                                        class="absolute top-full left-0 mt-1 z-50 \
                                               min-w-[160px] rounded-grape \
                                               inset-ring inset-ring-longan-700 \
                                               bg-longan-900 py-1"
                                    >
                                        <div class="text-label-sm text-rambutan-500 px-2 pb-1">
                                            "Link to person..."
                                        </div>
                                        {move || {
                                            let ps = people.get();
                                            if ps.is_empty() {
                                                view! {
                                                    <div class="px-2 py-1 text-label text-rambutan-500">
                                                        "No profiles yet"
                                                    </div>
                                                }.into_any()
                                            } else {
                                                ps.into_iter().map(|p| {
                                                    let p_id = p.id.clone();
                                                    let p_name = p.display_name.clone();
                                                    let p_name_display = p.display_name;
                                                    view! {
                                                        <button
                                                            type="button"
                                                            class="w-full text-left px-2 py-1 min-h-11 sm:min-h-6 \
                                                                   text-label text-rambutan-200 \
                                                                   hover:bg-longan-800 transition-colors"
                                                            on:click=move |_| {
                                                                do_link(p_id.clone(), p_name.clone());
                                                            }
                                                        >
                                                            {p_name_display}
                                                        </button>
                                                    }
                                                }).collect_view().into_any()
                                            }
                                        }}
                                        <button
                                            type="button"
                                            class="w-full text-left px-2 py-1 min-h-11 sm:min-h-6 \
                                                   text-label text-rambutan-500 \
                                                   hover:bg-longan-800 transition-colors \
                                                   mt-1"
                                            on:click=move |_| set_menu_open.set(false)
                                        >
                                            "Cancel"
                                        </button>
                                    </div>
                                </Show>
                            </Show>
                        </div>
                    }.into_any()
                } else {
                    view! {
                        <span
                            class=format!(
                                "text-label {text_class} \
                                 inset-ring {ring_class} rounded-seed \
                                 px-2 py-0.5 opacity-60"
                            )
                        >
                            {label_now}
                        </span>
                    }.into_any()
                }
            }
        >
            <input
                type="text"
                node_ref=input_ref
                aria-label="Speaker name"
                class=format!(
                    "text-label {text_class} \
                     bg-transparent inset-ring {ring_class} rounded-seed \
                     px-2 py-0.5 min-h-11 sm:min-h-6 outline-none min-w-0 w-[120px]"
                )
                prop:value=move || draft.get()
                on:input=on_input
                on:keydown=on_keydown
                on:blur=on_blur
            />
        </Show>
    }
}
