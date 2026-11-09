//! Meetings list — top-level surface reached via the sidebar entry or
//! `nav-meetings` in the command bar.

use bruhs_models::num::ToIntSaturating;
use leptos::prelude::*;

use crate::api;
use crate::components::capture_status_banner::CaptureStatusBanner;
use crate::state::{
    AppView, CaptureStatusSignal, Meeting, MeetingStatus, MeetingsSignal, SetActiveView,
    SetMeetingsSignal,
};
use crate::storage;
use crate::ui::{Button, ButtonSize, ButtonVariant};

fn format_started_at(ms: f64) -> String {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms));
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        d.get_full_year(),
        d.get_month() + 1,
        d.get_date(),
        d.get_hours(),
        d.get_minutes(),
    )
}

/// Format a duration in milliseconds as "Xh Ym" or "Ym Zs".
fn format_duration(ms: f64) -> String {
    let total_secs = (ms / 1000.0).to_u64_saturating();
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

/// Return the first sentence of `text`, truncated to `max_chars`.
fn summary_preview(text: &str, max_chars: usize) -> String {
    // Find a sentence boundary: period/exclamation/question followed by space or end.
    let sentence_end = text
        .char_indices()
        .find_map(|(i, c)| {
            if matches!(c, '.' | '!' | '?') {
                Some(i + c.len_utf8())
            } else {
                None
            }
        })
        .unwrap_or(text.len());
    let first_sentence = &text[..sentence_end];
    match first_sentence.char_indices().nth(max_chars) {
        None => first_sentence.trim_end().to_string(),
        Some((end, _)) => format!("{}…", first_sentence[..end].trim_end()),
    }
}

fn status_label(s: &MeetingStatus) -> &'static str {
    match s {
        MeetingStatus::Live => "Live",
        MeetingStatus::Ended => "Ended",
        MeetingStatus::Failed => "Failed",
    }
}

#[cfg(test)]
mod tests {
    use super::summary_preview;

    #[test]
    fn summary_preview_truncates_characters_without_splitting_utf8() {
        assert_eq!(summary_preview("🗒é中abc", 2), "🗒é…");
        assert_eq!(summary_preview("é! Next sentence", 2), "é!");
        assert_eq!(summary_preview("中", 0), "…");
        assert_eq!(summary_preview("", 0), "");
    }
}

fn status_class(s: &MeetingStatus) -> &'static str {
    match s {
        MeetingStatus::Live => "text-persimmon-400",
        MeetingStatus::Ended => "text-rambutan-400",
        MeetingStatus::Failed => "text-lychee-400",
    }
}

#[component]
pub fn MeetingsPage() -> impl IntoView {
    let MeetingsSignal(meetings) = expect_context::<MeetingsSignal>();
    let SetMeetingsSignal(set_meetings) = expect_context::<SetMeetingsSignal>();
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();
    let CaptureStatusSignal(capture_status) = expect_context::<CaptureStatusSignal>();

    // Initial hydrate — server then IDB fallback. The server is the
    // source of truth for `Live` meetings; IDB carries the historical
    // log for offline reads.
    Effect::new(move |_| {
        wasm_bindgen_futures::spawn_local(async move {
            if let Ok(fetched) = api::fetch_meetings().await {
                if !fetched.is_empty() {
                    set_meetings.set(fetched);
                    return;
                }
            }
            if let Ok(local) = storage::load_meetings().await {
                set_meetings.set(local);
            }
        });
    });

    let targets = crate::auto_record::RecordingTargets::from_context();
    let capture = crate::auto_record::CaptureSettings::from_context();
    let on_new_meeting = move |_: web_sys::MouseEvent| {
        let now = js_sys::Date::now();
        let body = api::CreateMeetingBody {
            title: format!("Meeting at {}", format_started_at(now)),
            started_at_ms: api::epoch_ms(now),
            calendar_event_id: None,
        };
        wasm_bindgen_futures::spawn_local(crate::auto_record::create_and_record(
            body,
            capture.current(),
            targets,
        ));
    };
    let on_new_meeting_cb = Callback::new(on_new_meeting);

    let open_meeting = Callback::new(move |id: String| {
        set_active_view.set(AppView::MeetingDetail { meeting_id: id });
    });

    view! {
        <div class="flex flex-col h-full bg-longan-950 text-rambutan-300">
            <header class="flex shrink-0 flex-wrap items-center justify-between gap-3 inset-ring inset-ring-longan-800 px-4 py-4 sm:px-6">
                <h1 class="shrink-0 text-body tracking-tight text-rambutan-100">
                    "Meetings"
                </h1>
                <span class=move || {
                    if capture_status.get().is_running() {
                        "order-3 w-full text-label text-pandan-400 sm:order-none sm:w-auto"
                    } else {
                        "order-3 w-full text-label text-rambutan-500 sm:order-none sm:w-auto"
                    }
                }>
                    {move || if capture_status.get().is_running() {
                        "Capture daemon: ready".to_string()
                    } else {
                        "Capture daemon: not running".to_string()
                    }}
                </span>
                <Button
                    variant=ButtonVariant::Primary
                    size=ButtonSize::Default
                    on_click=on_new_meeting_cb
                    class="min-h-11 shrink-0 whitespace-nowrap"
                    disabled=Signal::derive(move || !capture_status.get().is_running())
                >
                    "+ Record"
                </Button>
            </header>

            <div class="px-6 pt-3">
                <CaptureStatusBanner />
            </div>

            <div class="flex-1 overflow-auto">
                <crate::components::meeting_notes::MeetingNotes />
                // <Show> only flips on the empty↔non-empty transition; <For>
                // diffs rows by key so an update to one meeting doesn't
                // rebuild the whole list. The key is a row fingerprint (id +
                // every displayed field) so in-place changes — Live→Ended,
                // summary arriving — re-render exactly that row.
                <Show
                    when=move || meetings.with(|m| !m.is_empty())
                    fallback=move || view! {
                        <div class="flex flex-col items-center justify-center h-full gap-2 px-8 text-center">
                            <div class="text-body text-rambutan-300">"No meetings yet"</div>
                            <div class="text-body-sm text-rambutan-500 max-w-md">
                                {move || if capture_status.get().is_running() {
                                    "Press + Record to start one manually."
                                } else {
                                    "Start the bruhs-capture daemon (cargo run from bruhs-capture/) to enable recording."
                                }}
                            </div>
                        </div>
                    }
                >
                    <ul role="list" class="divide-y divide-longan-800">
                        <For
                            each=move || meetings.get()
                            key=|m| format!(
                                "{}|{}|{}|{}|{}|{}",
                                m.id,
                                status_label(&m.status),
                                m.ended_at_ms.map_or(-1, |v| v.to_i64_saturating()),
                                m.title,
                                m.thread_id.is_some(),
                                m.summary.as_ref().map_or(0, |s| s.tldr.len() + s.action_items.len()),
                            )
                            children=move |m: Meeting| {
                                let id_for_click = m.id.clone();
                                let status_cls = status_class(&m.status);
                                let status_txt = status_label(&m.status);
                                let started_txt = format_started_at(m.started_at_ms);
                                // Duration: show elapsed if live, or ended-started if ended.
                                let duration_txt: Option<String> = match m.status {
                                    MeetingStatus::Live => None, // rendered as "live now"
                                    MeetingStatus::Ended | MeetingStatus::Failed => {
                                        m.ended_at_ms.map(|end| format_duration(end - m.started_at_ms))
                                    }
                                };
                                let is_live = matches!(m.status, MeetingStatus::Live);
                                // Action-item count.
                                let action_item_count = m.summary.as_ref()
                                    .map_or(0, |s| s.action_items.len());
                                // Summary preview — first sentence of tldr, max 80 chars.
                                let preview = m.summary.as_ref()
                                    .filter(|s| !s.tldr.is_empty())
                                    .map(|s| summary_preview(&s.tldr, 80));
                                let has_thread = m.thread_id.is_some();
                                view! {
                                    <li>
                                        // Real <button> so rows are keyboard-activable.
                                        <button
                                            type="button"
                                            class="flex w-full items-start justify-between px-6 py-3 text-left hover:bg-longan-900/60 cursor-pointer"
                                            on:click=move |_| open_meeting.run(id_for_click.clone())
                                        >
                                            <div class="flex flex-col gap-0.5 flex-1 min-w-0 pr-4">
                                                <div class="flex items-center gap-1.5">
                                                    <span class="text-body-sm text-rambutan-100 truncate">{m.title}</span>
                                                    {if has_thread {
                                                        view! { <span class="text-label text-rambutan-400 shrink-0">"💬"</span> }.into_any()
                                                    } else {
                                                        ().into_any()
                                                    }}
                                                </div>
                                                <div class="flex items-center gap-2 flex-wrap">
                                                    <span class="text-label text-rambutan-500">{started_txt}</span>
                                                    {if is_live {
                                                        view! {
                                                            <span class="text-label text-persimmon-400">"live now"</span>
                                                        }.into_any()
                                                    } else if let Some(dur) = duration_txt {
                                                        view! {
                                                            <span class="text-label text-rambutan-500">{dur}</span>
                                                        }.into_any()
                                                    } else {
                                                        ().into_any()
                                                    }}
                                                    {if action_item_count > 0 {
                                                        view! {
                                                            <span class="text-label text-rambutan-400">"📋 "{action_item_count.to_string()}</span>
                                                        }.into_any()
                                                    } else {
                                                        ().into_any()
                                                    }}
                                                </div>
                                                {if let Some(prev) = preview {
                                                    view! {
                                                        <span class="text-label text-rambutan-500 truncate">{prev}</span>
                                                    }.into_any()
                                                } else {
                                                    ().into_any()
                                                }}
                                            </div>
                                            <span class=move || format!("text-label {status_cls} shrink-0")>{status_txt}</span>
                                        </button>
                                    </li>
                                }
                            }
                        />
                    </ul>
                </Show>
            </div>
        </div>
    }
}
