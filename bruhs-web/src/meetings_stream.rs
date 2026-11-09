//! Subscriber for the /api/meetings/stream SSE endpoint. Listens for
//! MeetingEvents emitted by the bruhs-capture daemon (forwarded by
//! bruhs-server) and updates the per-meeting transcript signal.
//!
//! Also handles server-injected events such as `PersonObservationsAppended`
//! which are emitted by the summarize route (not the daemon).
//!
//! Phase 4c: handles `AudioActivityDetected` — when the daemon's activity
//! monitor fires, we check whether the audio-driven sub-toggle is on and
//! whether the cooldown has elapsed, then populate `PendingAutoRecordSignal`
//! with a synthetic CalendarEvent so the existing modal pipeline fires.
//!
//! TranscriptSync owns bounded, acknowledged live writes and read-only reconnect recovery.
//!
//! Polish v2: SSE reconnect with exponential backoff. On EventSource error,
//! closes the connection, waits `backoff_ms` (starting at 1s, doubling up
//! to 30s), and reopens. BannerSignal shows "Reconnecting…" while retrying.

use bruhs_models::EpochMs;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{EventSource, MessageEvent};

use crate::components::banner::{BannerKind, BannerStackSignal};
use crate::state::{
    ActiveView, AppView, AudioLevelSignal, AudioPromptCooldownSignal, AutoRecordSettingsSignal,
    CalendarEvent, EventSource as CalEvtSource, LiveMeetingIdSignal, MeetingEvent,
    PendingAutoRecordSignal, PersonalitySettings, SetPendingAutoRecordSignal,
};

fn stream_url(api_base: &str, override_url: Option<&str>) -> String {
    override_url.map_or_else(
        || format!("{}/api/meetings/stream", api_base.trim_end_matches('/')),
        str::to_owned,
    )
}

fn reconnect_targets(live: Option<String>, selected: Option<String>) -> Vec<String> {
    live.into_iter()
        .chain(selected)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Holds the two JS closures registered on each `EventSource` instance.
/// Stored in an `Rc<RefCell<Option<…>>>` so both closures drop deterministically
/// when the next reconnect fires — prevents heap growth from `.forget()`.
struct LiveClosures {
    /// `onmessage` handler for the EventSource. Never read: it owns the
    /// `Closure` so the JS callback stays alive until this struct drops.
    _on_message: Box<dyn std::any::Any>,
    /// `onerror` handler for the EventSource. Same lifetime rationale.
    _on_error: Box<dyn std::any::Any>,
}

/// Reconnect backoff: start at 1s, double on each failure, cap at 30s.
const BACKOFF_INITIAL_MS: u32 = 1_000;
const BACKOFF_MAX_MS: u32 = 30_000;

/// How long (ms) after any audio-driven modal dismissal before a new
/// audio-driven prompt may appear.
const AUDIO_PROMPT_COOLDOWN_MS: f64 = 5.0 * 60.0 * 1000.0; // 5 minutes

pub fn subscribe(
    workflow_executor: crate::workflow_executor::WorkflowExecutor,
    personality: ReadSignal<PersonalitySettings>,
) {
    let transcript_sync = expect_context::<crate::transcript_sync::TranscriptSync>();
    let banners = expect_context::<BannerStackSignal>();
    let active_view = expect_context::<ActiveView>().0;
    let meeting_actions = expect_context::<crate::meeting_actions::MeetingActions>();
    // Phase 4c: read context signals needed for audio-driven auto-record.
    let auto_record_settings = use_context::<AutoRecordSettingsSignal>().map(|s| s.0);
    let live_meeting_id = use_context::<LiveMeetingIdSignal>().map(|s| s.0);
    let pending_auto_record = use_context::<PendingAutoRecordSignal>().map(|s| s.0);
    let set_pending_auto_record = use_context::<SetPendingAutoRecordSignal>().map(|s| s.0);
    let audio_prompt_cooldown = use_context::<AudioPromptCooldownSignal>().map(|s| s.0);
    // Polish v2: audio level signal updated on AudioLevel events.
    let audio_level_signal = use_context::<AudioLevelSignal>().map(|s| s.0);

    // Current backoff delay (ms). Doubles on each error up to BACKOFF_MAX_MS.
    let backoff_ms: Rc<Cell<u32>> = Rc::new(Cell::new(BACKOFF_INITIAL_MS));

    // Each EventSource open registers exactly two JS closures (on_message + on_error).
    // Storing them in a named struct ensures they drop deterministically when a
    // reconnect fires (borrow_mut().take() below), instead of leaking via .forget().
    // WASM is single-threaded so Rc<RefCell<…>> is safe.
    let live_closures: Rc<RefCell<Option<LiveClosures>>> = Rc::new(RefCell::new(None));

    // open_event_source is a recursive Rc<RefCell<dyn Fn()>> so the error
    // handler can schedule a retry that calls back into itself.
    type OpenFn = Rc<RefCell<Box<dyn Fn()>>>;
    let open_fn: OpenFn = Rc::new(RefCell::new(Box::new(|| {})));

    {
        let open_fn_clone = Rc::clone(&open_fn);
        let backoff_clone = Rc::clone(&backoff_ms);
        let live_closures_clone = Rc::clone(&live_closures);

        *open_fn.borrow_mut() = Box::new(move || {
            // Drop the previous EventSource's closures before registering new ones.
            // This is the deterministic replacement that prevents the unbounded heap
            // growth that .forget() causes over reconnects.
            live_closures_clone.borrow_mut().take();

            let url = stream_url(crate::api::BASE_URL, option_env!("BRUHS_STREAM_URL"));
            let src = match EventSource::new(&url) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("meetings EventSource failed to open: {e:?}");
                    return;
                }
            };
            // Wrap in Rc so both the message-setup code and the error
            // handler can hold a reference.
            let src = Rc::new(src);

            // ── Message handler ─────────────────────────────────────────────
            // Capture everything needed for the message handler.
            let auto_rec = auto_record_settings;
            let live_mid = live_meeting_id;
            let pending_rec = pending_auto_record;
            let set_pending_rec = set_pending_auto_record;
            let cooldown_sig = audio_prompt_cooldown;
            let audio_lvl = audio_level_signal;
            let backoff_reset = Rc::clone(&backoff_clone);

            let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |ev: MessageEvent| {
                let Some(data) = ev.data().as_string() else {
                    return;
                };
                // Successful message — reset backoff.
                backoff_reset.set(BACKOFF_INITIAL_MS);
                match serde_json::from_str::<MeetingEvent>(&data) {
                    Ok(MeetingEvent::StreamOpened) => {
                        let selected = match active_view.get_untracked() {
                            AppView::MeetingDetail { meeting_id } => Some(meeting_id),
                            _ => None,
                        };
                        let live = live_mid.and_then(|signal| signal.get_untracked());
                        for id in reconnect_targets(live, selected) {
                            transcript_sync.refresh(id);
                        }
                        if workflow_executor.people_hydrated.get_untracked() {
                            wasm_bindgen_futures::spawn_local(async move {
                                workflow_executor.refresh_people().await;
                            });
                        }
                    }
                    Ok(MeetingEvent::Segment {
                        meeting_id,
                        segment,
                    }) => {
                        transcript_sync.receive_segment(meeting_id, segment);
                    }
                    Ok(MeetingEvent::SegmentRemoved {
                        meeting_id,
                        segment_id,
                    }) => {
                        transcript_sync.forget_chunk(&meeting_id, &segment_id);
                        wasm_bindgen_futures::spawn_local(async move {
                            if let Err(error) = crate::storage::transcripts::forget_segment(
                                &meeting_id,
                                &segment_id,
                            )
                            .await
                            {
                                log::warn!(
                                    "could not drop a deleted transcript chunk from this device: {error}"
                                );
                            }
                        });
                    }
                    Ok(MeetingEvent::TranscriptionCompleted { meeting_id }) => {
                        let extract = personality.get_untracked().extract_observations;
                        wasm_bindgen_futures::spawn_local(async move {
                            match transcript_sync.flush_stream(&meeting_id).await {
                                Ok(()) => meeting_actions.generate(meeting_id, extract, false),
                                Err(error) => transcript_sync.report(&meeting_id, format!("Live speech could not be saved. {error} Retry loading the transcript before generating a summary.")),
                            }
                        });
                    }
                    Ok(MeetingEvent::SpeakersAssigned { meeting_id }) => {
                        log::info!("speakers assigned for meeting {meeting_id}");
                        transcript_sync.refresh(meeting_id);
                    }
                    Ok(MeetingEvent::SpeakersLinked { meeting_id }) => {
                        log::info!("speakers linked for meeting {meeting_id}; refetching meeting");
                        wasm_bindgen_futures::spawn_local(async move {
                            match crate::api::get_meeting(&meeting_id).await {
                                Ok(Some(updated)) => {
                                    if updated.id == meeting_id {
                                        transcript_sync.cache_meeting(updated).await;
                                    } else {
                                        transcript_sync.report(&meeting_id, "The server returned another meeting; retry loading the transcript.".into());
                                    }
                                }
                                Ok(None) => {
                                    log::warn!("SpeakersLinked: meeting {meeting_id} not found")
                                }
                                Err(e) => {
                                    log::warn!("SpeakersLinked: get_meeting failed: {e}")
                                }
                            }
                        });
                    }
                    Ok(MeetingEvent::PersonObservationsAppended { meeting_id: _ }) => {
                        log::info!("person observations appended; refetching people");
                        wasm_bindgen_futures::spawn_local(async move {
                            workflow_executor.refresh_people().await;
                        });
                    }
                    Ok(MeetingEvent::PeopleUpdated) => {
                        log::info!("people updated (rename/merge); refetching people");
                        wasm_bindgen_futures::spawn_local(async move {
                            workflow_executor.refresh_people().await;
                        });
                    }
                    Ok(MeetingEvent::PersonChanged {
                        person_id,
                        updated_at,
                    }) => {
                        workflow_executor.person_changed(person_id, updated_at);
                    }
                    Ok(MeetingEvent::PersonMerged { source_id }) => {
                        workflow_executor.remote_person_merged(source_id);
                    }
                    Ok(MeetingEvent::CaptureWarning {
                        meeting_id,
                        message,
                    }) => {
                        log::warn!("capture warning for {meeting_id}: {message}");
                        banners.push(
                            BannerKind::Warning,
                            format!("Capture warning: {message}"),
                            Some(8000),
                        );
                    }
                    Ok(MeetingEvent::AudioActivityDetected { started_at_ms }) => {
                        // Destructure the five optional signals individually so
                        // the nested-zip tuple-of-tuples is avoided.
                        if let (
                            Some(cooldown),
                            Some(settings),
                            Some(live),
                            Some(pending),
                            Some(set_pending),
                        ) = (
                            cooldown_sig,
                            auto_rec,
                            live_mid,
                            pending_rec,
                            set_pending_rec,
                        ) {
                            let cfg = settings.get_untracked();
                            let gate = cfg.enabled
                                && cfg.audio_driven_enabled
                                && live.get_untracked().is_none()
                                && pending.get_untracked().is_none();
                            if gate {
                                let last_prompt = cooldown.get_untracked();
                                let now_ms = js_sys::Date::now();
                                let cooldown_ok = last_prompt == 0.0
                                    || (now_ms - last_prompt) >= AUDIO_PROMPT_COOLDOWN_MS;
                                if cooldown_ok {
                                    let now = js_sys::Date::now();
                                    let synthetic = CalendarEvent {
                                        id: format!("audio-{started_at_ms}"),
                                        title: "Sustained audio activity".into(),
                                        start_at_ms: now,
                                        end_at_ms: now + 30.0 * 60.0 * 1000.0,
                                        all_day: false,
                                        color: None,
                                        notes: None,
                                        created_at: EpochMs::saturating_from_f64(now).as_millis(),
                                        updated_at: EpochMs::saturating_from_f64(now).as_millis(),
                                        source: CalEvtSource::Local,
                                        google_event_id: None,
                                        calendar_id: None,
                                        attendees: vec![],
                                        organizer: None,
                                        location: None,
                                        conferencing: None,
                                        recurring_event_id: None,
                                        html_link: None,
                                    };
                                    set_pending.set(Some(synthetic));
                                    log::info!(
                                        "audio_driven: queued auto-record prompt (activity started at {started_at_ms})"
                                    );
                                } else {
                                    log::debug!(
                                        "audio_driven: cooldown not elapsed ({:.0}s remaining)",
                                        (AUDIO_PROMPT_COOLDOWN_MS - (now_ms - last_prompt))
                                            / 1000.0
                                    );
                                }
                            }
                        }
                    }
                    Ok(MeetingEvent::AudioLevel {
                        mic_rms, sys_rms, ..
                    }) => {
                        if let Some(sig) = audio_lvl {
                            sig.set((mic_rms, sys_rms));
                        }
                    }
                    Ok(_) => {
                        // Other variants (Started/Ended/CaptureError) ignored here.
                    }
                    Err(_) => {
                        log::debug!("meetings_stream non-MeetingEvent data: {data}");
                    }
                }
            });
            src.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

            // ── Error handler (reconnect) ────────────────────────────────────
            let backoff_err = Rc::clone(&backoff_clone);
            let open_fn_retry = Rc::clone(&open_fn_clone);
            let src_err = Rc::clone(&src);

            let on_error = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
                // Close the broken source.
                src_err.close();

                let delay = backoff_err.get();
                // Double the backoff for the next failure (capped).
                backoff_err.set((delay * 2).min(BACKOFF_MAX_MS));

                log::warn!(
                    "meetings SSE error — reconnecting in {}ms (next backoff: {}ms)",
                    delay,
                    backoff_err.get()
                );
                banners.push(
                    BannerKind::Warning,
                    "Live transcript disconnected — reconnecting…".to_string(),
                    Some(delay + 500),
                );

                let retry = Rc::clone(&open_fn_retry);
                let timeout = gloo_timers::callback::Timeout::new(delay, move || {
                    (retry.borrow())();
                });
                // Timeout closures are one-shot and self-contained; .forget() is
                // acceptable here — they don't accumulate across reconnects.
                timeout.forget();
            });
            src.set_onerror(Some(on_error.as_ref().unchecked_ref()));

            // Store both closures as a named pair — they drop together on next reconnect
            // (.take() above). This replaces the previous misleading Vec<Box<dyn Any>>.
            *live_closures_clone.borrow_mut() = Some(LiveClosures {
                _on_message: Box::new(on_message),
                _on_error: Box::new(on_error),
            });
            // Keep `src` alive (Rc drop would close it).
            // The error handler holds a Rc clone; drop the outer one now.
            drop(src);
        });
    }

    // Kick off the first connection.
    (open_fn.borrow())();
}

#[cfg(test)]
mod tests {
    use super::{reconnect_targets, stream_url};

    #[test]
    fn reconnect_reads_only_distinct_live_and_open_recordings() {
        assert!(reconnect_targets(None, None).is_empty());
        assert_eq!(
            reconnect_targets(Some("live".into()), Some("live".into())),
            ["live"]
        );
        assert_eq!(
            reconnect_targets(Some("live".into()), Some("open".into())),
            ["live", "open"]
        );
    }

    #[test]
    fn meeting_stream_follows_configured_api_origin() {
        assert_eq!(
            stream_url("http://127.0.0.1:8097/", None),
            "http://127.0.0.1:8097/api/meetings/stream"
        );
    }

    #[test]
    fn meeting_stream_supports_same_origin_builds() {
        assert_eq!(stream_url("", None), "/api/meetings/stream");
    }

    #[test]
    fn explicit_stream_override_is_preserved() {
        assert_eq!(
            stream_url("http://127.0.0.1:8097", Some("/custom/events")),
            "/custom/events"
        );
    }
}
