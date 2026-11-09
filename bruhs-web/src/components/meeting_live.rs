//! Compact, movable in-page meeting dock. Expanded previews are bounded;
//! capture must acknowledge Stop before the meeting is finalized.

use bruhs_models::num::ToIntSaturating;
use gloo_timers::callback::Interval;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::api;
use crate::components::meeting_detail::{
    group_by_speaker, speaker_colors, speaker_label, SpeakerChip,
};
use crate::state::{
    AudioLevelSignal, LiveMeetingIdSignal, Meeting, MeetingSegmentsSignal, MeetingsSignal,
    SetLiveMeetingIdSignal, SetMeetingsSignal,
};
use crate::storage;

/// Format elapsed milliseconds as MM:SS if < 1h, else HH:MM:SS.
fn format_elapsed(elapsed_ms: f64) -> String {
    let total_secs = (elapsed_ms / 1000.0).to_u64_saturating();
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

#[component]
pub fn MeetingLive() -> impl IntoView {
    let LiveMeetingIdSignal(live_id) = expect_context::<LiveMeetingIdSignal>();
    let SetLiveMeetingIdSignal(set_live_id) = expect_context::<SetLiveMeetingIdSignal>();
    let MeetingsSignal(meetings) = expect_context::<MeetingsSignal>();
    let SetMeetingsSignal(set_meetings) = expect_context::<SetMeetingsSignal>();
    let MeetingSegmentsSignal(meeting_segments) = expect_context::<MeetingSegmentsSignal>();
    let AudioLevelSignal(audio_level) = expect_context::<AudioLevelSignal>();
    let expanded = RwSignal::new(false);
    let position = RwSignal::new(None::<(f64, f64)>);
    let dragging = StoredValue::new(None::<(i32, f64, f64)>);
    let dock = NodeRef::<leptos::html::Div>::new();
    let stopping = RwSignal::new(false);
    let capture_stopped = RwSignal::new(false);
    let stop_error = RwSignal::new(None::<String>);
    let transcript_sync = expect_context::<crate::transcript_sync::TranscriptSync>();
    let selected_chunk = RwSignal::new(None::<String>);
    let chunk_note = RwSignal::new(None::<String>);

    Effect::new(move |_| {
        let _ = live_id.get();
        expanded.set(false);
        position.set(None);
        dragging.set_value(None);
        stop_error.set(None);
        capture_stopped.set(false);
        stopping.set(false);
        selected_chunk.set(None);
        chunk_note.set(None);
    });

    let dock_style = move || {
        let (width, height) = if expanded.get() {
            (480, 340)
        } else {
            (280, 52)
        };
        let (left, top) = position.get().map_or_else(
            || {
                (
                    format!("calc(50vw - min({width}px, 100vw - 16px) / 2)"),
                    "calc(100dvh - 144px)".to_string(),
                )
            },
            |(x, y)| (format!("{x}px"), format!("{y}px")),
        );
        format!("width:min({width}px,calc(100vw - 16px));height:min({height}px,calc(100dvh - 16px));left:clamp(8px,{left},max(8px,calc(100vw - {width}px - 8px)));top:clamp(8px,{top},max(8px,calc(100dvh - {height}px - 8px)))")
    };

    let current: Memo<Option<Meeting>> = Memo::new(move |_| {
        let id = live_id.get()?;
        meetings.with(|meetings| meetings.iter().find(|m| m.id == id).cloned())
    });

    // The dock is a preview, not a second full transcript. Closed pills do not
    // subscribe to transcript changes or clone another meeting's history.
    let segments_memo: Memo<Vec<crate::state::TranscriptSegment>> = Memo::new(move |_| {
        if !expanded.get() {
            return Vec::new();
        }
        let Some(id) = live_id.get() else {
            return Vec::new();
        };
        meeting_segments.with(|map| {
            map.get(&id)
                .map(|segments| segments[segments.len().saturating_sub(50)..].to_vec())
                .unwrap_or_default()
        })
    });

    Effect::new(move |_| {
        let visible = segments_memo.get();
        if selected_chunk.with_untracked(|selected| {
            selected
                .as_ref()
                .is_some_and(|id| !visible.iter().any(|segment| segment.id == *id))
        }) {
            selected_chunk.set(None);
        }
    });

    // Elapsed timer: ticks every second and stores the current wall-clock ms.
    let (now_ms, set_now_ms) = signal(js_sys::Date::now());
    let timer = StoredValue::new_local(Option::<Interval>::None);
    Effect::new(move |_| {
        timer.set_value(None);
        if live_id.get().is_some() {
            set_now_ms.set(js_sys::Date::now());
            timer.set_value(Some(Interval::new(1000, move || {
                set_now_ms.set(js_sys::Date::now());
            })));
        }
    });
    on_cleanup(move || {
        // Drop the interval (stops ticking) when the component is unmounted.
        timer.set_value(None);
    });

    let elapsed_label = move || {
        current.get().map(|m| {
            let elapsed = (now_ms.get() - m.started_at_ms).max(0.0);
            format_elapsed(elapsed)
        })
    };

    let on_stop = move |_: web_sys::MouseEvent| {
        if stopping.get_untracked() {
            return;
        }
        let Some(m) = current.get() else { return };
        let id = m.id;
        stopping.set(true);
        stop_error.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            // Keep the controls visible until the capture daemon acknowledges stop.
            if !capture_stopped.get_untracked() {
                if let Err(e) = api::capture_stop().await {
                    if live_id.get_untracked().as_deref() == Some(&id) {
                        stop_error.set(Some(format!("Could not stop capture: {e}. Retry Stop.")));
                        expanded.set(true);
                        stopping.set(false);
                    }
                    return;
                }
                if live_id.get_untracked().as_deref() == Some(&id) {
                    capture_stopped.set(true);
                }
            }
            let body = api::StopMeetingBody {
                ended_at_ms: api::epoch_ms(js_sys::Date::now()),
            };
            match api::stop_meeting(&id, &body).await {
                Ok(updated) => {
                    let updated_clone = updated.clone();
                    set_meetings.update(|ms| {
                        if let Some(slot) = ms.iter_mut().find(|m| m.id == updated_clone.id) {
                            *slot = updated_clone.clone();
                        }
                    });
                    storage::persist_meeting(updated);
                    if live_id.get_untracked().as_deref() == Some(&id) {
                        set_live_id.set(None);
                    }
                }
                Err(e) => {
                    if live_id.get_untracked().as_deref() == Some(&id) {
                        stop_error.set(Some(format!(
                            "Capture stopped; meeting could not be saved: {e}. Retry Stop."
                        )));
                        expanded.set(true);
                    }
                }
            }
            if live_id.get_untracked().as_deref() == Some(&id) {
                stopping.set(false);
            }
        });
    };

    view! {
        <Show
            when=move || current.get().is_some()
            fallback=|| view! { <></> }
        >
            <div node_ref=dock data-testid="meeting-dock" role="region" aria-label="Live meeting dock"
                class="fixed z-40 flex flex-col justify-end gap-2" style=dock_style>
                // Live segments scroller (shows only when there are segments)
                {move || {
                    if !expanded.get() { return ().into_any(); }
                    let Some(m) = current.get() else { return ().into_any(); };
                    let segments = segments_memo.get();
                    let speakers = m.speakers.clone();
                    let groups = group_by_speaker(&segments);
                    view! {
                        <div class="min-h-0 flex-1 overflow-auto rounded-grape bg-longan-900 inset-ring inset-ring-longan-700 px-3 py-2 space-y-3">
                            <p class="text-label text-rambutan-100 break-words">{m.title.clone()}</p>
                            <p class="text-label-sm text-rambutan-400">"Screen context not connected"</p>
                            <p class="text-label-sm text-rambutan-400">"Latest 50 segments · full transcript in Meetings"</p>
                            {move || {
                                let (mic_rms, sys_rms) = audio_level.get();
                                let mic = mic_rms.clamp(0.0, 1.0);
                                let mic_percent = mic * 100.0;
                                view! {
                                    <div class="flex items-center gap-3 text-label-sm text-rambutan-400">
                                        <span>"Mic"</span>
                                        <div role="meter" aria-label="Microphone level" aria-valuemin="0" aria-valuemax="100" aria-valuenow=mic_percent class="h-1 w-12 overflow-hidden rounded-orb bg-longan-700">
                                            <div class="h-full origin-left bg-dragonfruit-500" style=format!("transform:scaleX({mic})")></div>
                                        </div>
                                        {sys_rms.map(|level| {
                                            let level = level.clamp(0.0, 1.0);
                                            let percent = level * 100.0;
                                            view! {
                                                <span>"System"</span>
                                                <div role="meter" aria-label="System audio level" aria-valuemin="0" aria-valuemax="100" aria-valuenow=percent class="h-1 w-12 overflow-hidden rounded-orb bg-longan-700">
                                                    <div class="h-full origin-left bg-pandan-500" style=format!("transform:scaleX({level})")></div>
                                                </div>
                                            }
                                        })}
                                    </div>
                                }
                            }}
                            {move || stop_error.get().map(|error| view! { <p role="alert" class="text-body-sm text-lychee-400 break-words">{error}</p> })}
                            {move || chunk_note.get().map(|note| view! { <p role="status" class="text-body-sm text-lychee-400 break-words">{note}</p> })}
                            {segments.is_empty().then(|| view! { <p class="text-body-sm text-rambutan-400">"Waiting for transcript…"</p> })}
                            {groups.into_iter().map(|(speaker_id, segs)| {
                                let label = speaker_label(&speakers, speaker_id.as_deref());
                                let (text_class, ring_class) = speaker_colors(speaker_id.as_deref());
                                let meeting_id = m.id.clone();
                                let chip_speaker_id = speaker_id.clone();
                                view! {
                                    <div class="space-y-1">
                                        // Speaker chip — full opacity for clarity
                                        <div class="flex items-center [&_button]:min-h-11 [&_button]:min-w-11 [&_input]:min-h-11">
                                            <SpeakerChip
                                                meeting_id=meeting_id
                                                speaker_id=chip_speaker_id
                                                label=label
                                                text_class=text_class
                                                ring_class=ring_class
                                            />
                                        </div>
                                        // Segment texts — partial at 60%, final at full opacity
                                        <div class="space-y-0.5">
                                            {segs.into_iter().map(|seg| {
                                                let opacity_class = if seg.partial {
                                                    "text-body-sm text-rambutan-300 leading-snug pl-3 opacity-60"
                                                } else {
                                                    "text-body-sm text-rambutan-300 leading-snug pl-3 opacity-100"
                                                };
                                                let text = seg.text;
                                                let segment_id = seg.id;
                                                let selected = selected_chunk.get().as_deref() == Some(segment_id.as_str());
                                                let select_class = format!(
                                                    "{opacity_class} w-full whitespace-normal rounded-seed px-1 text-left hover:bg-longan-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400{}",
                                                    if selected { " bg-longan-800" } else { "" }
                                                );
                                                let select_id = segment_id.clone();
                                                let copy_text = text.clone();
                                                let delete_meeting = m.id.clone();
                                                let delete_id = segment_id.clone();
                                                let key_meeting = m.id.clone();
                                                let key_id = segment_id.clone();
                                                let key_text = text.clone();
                                                view! {
                                                    <div class="space-y-1">
                                                        <button
                                                            type="button"
                                                            aria-pressed=if selected { "true" } else { "false" }
                                                            data-testid="live-transcript-chunk"
                                                            data-chunk-id=segment_id
                                                            class=select_class
                                                            on:click=move |_| {
                                                                selected_chunk.update(|current| {
                                                                    *current = if current.as_deref() == Some(select_id.as_str()) {
                                                                        None
                                                                    } else {
                                                                        Some(select_id.clone())
                                                                    };
                                                                });
                                                            }
                                                            on:keydown=move |ev: web_sys::KeyboardEvent| {
                                                                let key = ev.key();
                                                                if key == "Escape" {
                                                                    ev.prevent_default();
                                                                    selected_chunk.set(None);
                                                                } else if matches!(key.as_str(), "Backspace" | "Delete")
                                                                    && !ev.meta_key()
                                                                    && !ev.ctrl_key()
                                                                    && !ev.alt_key()
                                                                    && !ev.shift_key()
                                                                {
                                                                    ev.prevent_default();
                                                                    selected_chunk.set(None);
                                                                    delete_live_chunk(transcript_sync, key_meeting.clone(), key_id.clone(), chunk_note);
                                                                } else if key.eq_ignore_ascii_case("c")
                                                                    && (ev.meta_key() || ev.ctrl_key())
                                                                    && !ev.shift_key()
                                                                    && !ev.alt_key()
                                                                {
                                                                    ev.prevent_default();
                                                                    copy_live_chunk(key_text.clone(), chunk_note);
                                                                }
                                                            }
                                                        >
                                                            {text}
                                                        </button>
                                                        {selected.then(|| view! {
                                                            <div class="flex gap-2 pl-3">
                                                                <button
                                                                    type="button"
                                                                    data-testid="live-chunk-copy"
                                                                    class="min-h-11 rounded-seed px-3 text-label-sm text-rambutan-200 hover:bg-longan-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400"
                                                                    on:click=move |_| copy_live_chunk(copy_text.clone(), chunk_note)
                                                                >
                                                                    "Copy"
                                                                </button>
                                                                <button
                                                                    type="button"
                                                                    data-testid="live-chunk-delete"
                                                                    class="min-h-11 rounded-seed px-3 text-label-sm text-lychee-300 hover:bg-longan-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400"
                                                                    on:click=move |_| {
                                                                        selected_chunk.set(None);
                                                                        delete_live_chunk(transcript_sync, delete_meeting.clone(), delete_id.clone(), chunk_note);
                                                                    }
                                                                >
                                                                    "Delete"
                                                                </button>
                                                            </div>
                                                        })}
                                                    </div>
                                                }
                                            }).collect_view()}
                                        </div>
                                    </div>
                                }
                            }).collect_view()}
                        </div>
                    }.into_any()
                }}
                // One quiet surface; controls do not move when Stop is pending.
                <div class="flex shrink-0 h-[52px] w-full items-center gap-1 rounded-orb bg-longan-900 inset-ring inset-ring-longan-700 p-1">
                    <button type="button" aria-label="Move meeting dock" title="Drag to move. Arrow keys move; Home resets position."
                        class="flex size-11 shrink-0 items-center justify-center cursor-grab active:cursor-grabbing touch-none select-none rounded-orb text-rambutan-400 hover:bg-longan-800 active:bg-longan-700 focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400 focus-visible:outline-offset-[-2px]"
                        on:pointerdown=move |ev: web_sys::PointerEvent| {
                            if ev.button() != 0 { return; }
                            let Some(el) = dock.get() else { return; };
                            let rect = el.get_bounding_client_rect();
                            if let Some(target) = ev.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
                                if target.set_pointer_capture(ev.pointer_id()).is_err() { return; }
                                dragging.set_value(Some((ev.pointer_id(), f64::from(ev.client_x()) - rect.x(), f64::from(ev.client_y()) - rect.y())));
                            }
                        }
                        on:pointermove=move |ev: web_sys::PointerEvent| {
                            if let Some((id, dx, dy)) = dragging.get_value() {
                                if id == ev.pointer_id() { position.set(Some((f64::from(ev.client_x()) - dx, f64::from(ev.client_y()) - dy))); }
                            }
                        }
                        on:pointerup=move |_| dragging.set_value(None)
                        on:pointercancel=move |_| dragging.set_value(None)
                        on:lostpointercapture=move |_| dragging.set_value(None)
                        on:keydown=move |ev: web_sys::KeyboardEvent| {
                            if ev.ctrl_key() || ev.alt_key() || ev.meta_key() { return; }
                            let step = if ev.shift_key() { 40.0 } else { 10.0 };
                            let delta = match ev.key().as_str() {
                                "ArrowLeft" => (-step, 0.0), "ArrowRight" => (step, 0.0),
                                "ArrowUp" => (0.0, -step), "ArrowDown" => (0.0, step),
                                "Home" => { ev.prevent_default(); position.set(None); return; },
                                _ => return,
                            };
                            ev.prevent_default();
                            if let Some(el) = dock.get() {
                                let rect = el.get_bounding_client_rect();
                                position.set(Some((rect.x() + delta.0, rect.y() + delta.1)));
                            }
                        }
                    >
                        <svg aria-hidden="true" width="7" height="12" viewBox="0 0 7 12" fill="currentColor">
                            <circle cx="1" cy="1" r="1"/><circle cx="6" cy="1" r="1"/>
                            <circle cx="1" cy="6" r="1"/><circle cx="6" cy="6" r="1"/>
                            <circle cx="1" cy="11" r="1"/><circle cx="6" cy="11" r="1"/>
                        </svg>
                    </button>
                    <div class="flex min-w-0 flex-1 items-center gap-2">
                        <span aria-hidden="true" class=move || if stopping.get() || capture_stopped.get() {
                            "hidden min-[300px]:block size-1.5 shrink-0 rounded-orb bg-rambutan-400"
                        } else { "hidden min-[300px]:block size-1.5 shrink-0 rounded-orb bg-persimmon-500" }></span>
                        <div class="min-w-0 flex-1">
                            {move || elapsed_label().map(|label| {
                                let title = label.clone();
                                view! {
                                    <div class="truncate text-[13px] leading-[17px] text-rambutan-100 tabular-nums" title=title>{label}</div>
                                }
                            })}
                            <div role="status" class=move || if stop_error.get().is_some() { "truncate text-label-sm leading-4 text-durian-400" } else { "truncate text-label-sm leading-4 text-rambutan-400" }>
                                {move || if stopping.get() { "Stopping" } else if stop_error.get().is_some() { "Needs review" } else { "Live" }}
                            </div>
                        </div>
                    </div>
                    <button type="button" aria-label=move || if expanded.get() { "Collapse meeting dock" } else { "Expand meeting dock" }
                        aria-expanded=move || expanded.get().to_string()
                        class="h-11 w-[52px] shrink-0 rounded-orb text-label-sm text-rambutan-200 hover:bg-longan-800 active:bg-longan-700 focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400 focus-visible:outline-offset-[-2px]"
                        on:click=move |_| expanded.update(|v| *v = !*v)>
                        {move || if expanded.get() { "Less" } else { "Open" }}
                    </button>
                    <button type="button"
                        on:click=on_stop
                        disabled=move || stopping.get()
                        aria-label=move || if stopping.get() { "Stopping recording" } else { "Stop recording" }
                        class="flex h-11 w-16 shrink-0 items-center justify-center gap-1.5 rounded-orb bg-longan-950 text-label-sm text-rambutan-200 enabled:hover:bg-longan-800 enabled:active:bg-longan-700 disabled:opacity-60 disabled:cursor-not-allowed focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400 focus-visible:outline-offset-[-2px]"
                    >
                        <span aria-hidden="true" class="size-[7px] shrink-0 rounded-[2px] bg-rambutan-200"></span>
                        "Stop"
                    </button>
                </div>
            </div>
        </Show>
    }
}

fn delete_live_chunk(
    sync: crate::transcript_sync::TranscriptSync,
    meeting_id: String,
    segment_id: String,
    note: RwSignal<Option<String>>,
) {
    let (_changed, segment) = sync.forget_chunk(&meeting_id, &segment_id);
    let Some(segment) = segment else {
        note.set(Some("That part of the transcript is already gone.".into()));
        return;
    };
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(error) =
            crate::storage::transcripts::forget_segment(&meeting_id, &segment_id).await
        {
            sync.restore_chunk(segment);
            note.set(Some(format!(
                "Could not delete that part of the transcript. {error} It is still saved."
            )));
            return;
        }
        if let Err(error) = api::delete_transcript_segment(&meeting_id, &segment_id).await {
            sync.restore_chunk(segment.clone());
            let reload = crate::storage::transcripts::put_segment(&segment)
                .await
                .is_err();
            note.set(Some(if reload {
                format!("Could not delete that part of the transcript. {error} It is still saved. Reload the transcript to confirm what was saved.")
            } else {
                format!("Could not delete that part of the transcript. {error} It is still saved.")
            }));
        } else {
            note.set(None);
        }
    });
}

fn copy_live_chunk(text: String, note: RwSignal<Option<String>>) {
    wasm_bindgen_futures::spawn_local(async move {
        if write_clipboard(&text).await.is_err() {
            note.set(Some("Could not copy that part of the transcript.".into()));
        } else {
            note.set(None);
        }
    });
}

async fn write_clipboard(text: &str) -> Result<(), ()> {
    let window = web_sys::window().ok_or(())?;
    let clipboard =
        js_sys::Reflect::get(&window.navigator(), &"clipboard".into()).map_err(|_| ())?;
    if clipboard.is_null() || clipboard.is_undefined() {
        return Err(());
    }
    let write_text = js_sys::Reflect::get(&clipboard, &"writeText".into()).map_err(|_| ())?;
    let write_text = write_text.dyn_into::<js_sys::Function>().map_err(|_| ())?;
    let result = write_text
        .call1(&clipboard, &wasm_bindgen::JsValue::from_str(text))
        .map_err(|_| ())?;
    let promise = result.dyn_into::<js_sys::Promise>().map_err(|_| ())?;
    wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map(|_| ())
        .map_err(|_| ())
}
