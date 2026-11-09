//! Subscriber for the meeting frames of the live stream (`crate::live_events`),
//! the same events as /api/meetings/stream. Listens for
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
//! Reconnects (with backoff and a banner) belong to the shared stream; each
//! reopen sends `stream_opened`, which reloads the open transcripts here.

use bruhs_models::api::live_events::MEETINGS;
use bruhs_models::EpochMs;

use leptos::prelude::*;

use crate::components::banner::{BannerKind, BannerStackSignal};
use crate::live_events::LiveEvents;
use crate::state::{
    ActiveView, AppView, AudioLevelSignal, AudioPromptCooldownSignal, AutoRecordSettingsSignal,
    CalendarEvent, EventSource as CalEvtSource, LiveMeetingIdSignal, MeetingEvent,
    PendingAutoRecordSignal, PersonalitySettings, SetPendingAutoRecordSignal,
};

fn reconnect_targets(live: Option<String>, selected: Option<String>) -> Vec<String> {
    live.into_iter()
        .chain(selected)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// How long (ms) after any audio-driven modal dismissal before a new
/// audio-driven prompt may appear.
const AUDIO_PROMPT_COOLDOWN_MS: f64 = 5.0 * 60.0 * 1000.0; // 5 minutes

pub fn subscribe(
    live: &mut LiveEvents,
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

    live.on(MEETINGS, move |data| {
                match serde_json::from_str::<MeetingEvent>(&data) {
                    Ok(MeetingEvent::StreamOpened) => {
                        let selected = match active_view.get_untracked() {
                            AppView::MeetingDetail { meeting_id } => Some(meeting_id),
                            _ => None,
                        };
                        let live = live_meeting_id.and_then(|signal| signal.get_untracked());
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
                            audio_prompt_cooldown,
                            auto_record_settings,
                            live_meeting_id,
                            pending_auto_record,
                            set_pending_auto_record,
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
                        if let Some(sig) = audio_level_signal {
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
}

#[cfg(test)]
mod tests {
    use super::reconnect_targets;

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
}
