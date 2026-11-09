//! Meetings, capture, auto-record and people.

use super::*;

#[derive(Copy, Clone)]
pub struct AutoRecordSettingsSignal(pub ReadSignal<AutoRecordSettings>);
#[derive(Copy, Clone)]
pub struct SetAutoRecordSettingsSignal(pub WriteSignal<AutoRecordSettings>);

#[derive(Copy, Clone)]
pub struct PendingAutoRecordSignal(pub ReadSignal<Option<CalendarEvent>>);
#[derive(Copy, Clone)]
pub struct SetPendingAutoRecordSignal(pub WriteSignal<Option<CalendarEvent>>);

#[derive(Copy, Clone)]
pub struct SuppressedEventsSignal(pub ReadSignal<HashSet<String>>);
#[derive(Copy, Clone)]
pub struct SetSuppressedEventsSignal(pub WriteSignal<HashSet<String>>);

#[derive(Copy, Clone)]
pub struct SessionSkippedEventsSignal(pub ReadSignal<HashSet<String>>);
#[derive(Copy, Clone)]
pub struct SetSessionSkippedEventsSignal(pub WriteSignal<HashSet<String>>);

/// Phase 4c: tracks the last time an audio-driven prompt was dismissed
/// (any of Record / Skip / Always-skip). Prevents re-prompting within
/// `AUDIO_PROMPT_COOLDOWN_MS` after the user has already responded.
/// A value of 0.0 means "never prompted".
#[derive(Copy, Clone)]
pub struct AudioPromptCooldownSignal(pub RwSignal<f64>);

#[derive(Copy, Clone)]
pub struct SelectedLifeRecordSignal(pub RwSignal<Option<SelectedLifeRecord>>);
#[derive(Copy, Clone)]
pub struct SelectedPersonSignal(pub RwSignal<Option<String>>);

#[derive(Copy, Clone)]
pub struct CaptureStatusSignal(pub ReadSignal<crate::api::CaptureStatus>);
#[derive(Copy, Clone)]
pub struct SetCaptureStatusSignal(pub WriteSignal<crate::api::CaptureStatus>);

#[derive(Copy, Clone)]
pub struct MeetingSegmentsSignal(pub ReadSignal<BTreeMap<String, Vec<TranscriptSegment>>>);

#[derive(Copy, Clone)]
pub struct MeetingsSignal(pub ReadSignal<Vec<Meeting>>);
#[derive(Copy, Clone)]
pub struct SetMeetingsSignal(pub WriteSignal<Vec<Meeting>>);

#[derive(Copy, Clone)]
pub struct LiveMeetingIdSignal(pub ReadSignal<Option<String>>);
#[derive(Copy, Clone)]
pub struct SetLiveMeetingIdSignal(pub WriteSignal<Option<String>>);

#[derive(Copy, Clone)]
pub struct PeopleSignal(pub ReadSignal<Vec<PersonProfile>>);
#[derive(Copy, Clone)]
pub struct SetPeopleSignal(pub WriteSignal<Vec<PersonProfile>>);

/// Current live audio levels: `(mic_rms, sys_rms)` in [0, 1].
/// Updated by the `AudioLevel` SSE event handler in `meetings_stream`.
/// `sys_rms` is `None` when system audio capture is inactive.
#[derive(Copy, Clone)]
pub struct AudioLevelSignal(pub RwSignal<(f32, Option<f32>)>);

/// Meetings, transcripts, people, capture status, audio levels and the
/// auto-record watcher's state. Needs the banner stack (workspace).
pub(super) fn provide() {
    let (meetings, set_meetings) = signal(Vec::<Meeting>::new());
    let (live_meeting_id, set_live_meeting_id) = signal::<Option<String>>(None);
    let (people, set_people) = signal(Vec::<PersonProfile>::new());
    let (meeting_segments, set_meeting_segments) =
        signal::<BTreeMap<String, Vec<TranscriptSegment>>>(BTreeMap::new());
    let (capture_status, set_capture_status) = signal(crate::api::CaptureStatus::default());

    let mut initial_auto_record: AutoRecordSettings =
        load_json_pref("bruhs.auto_record").unwrap_or_default();
    initial_auto_record.audio_driven_enabled = false;
    let (auto_record, set_auto_record) = signal(initial_auto_record);
    Effect::new(move |_| {
        let current = auto_record.get();
        save_json_pref("bruhs.auto_record", &current);
    });
    let (pending_auto_record, set_pending_auto_record) = signal::<Option<CalendarEvent>>(None);
    let initial_suppressed: HashSet<String> =
        load_json_pref("bruhs.auto_record.suppressed_events").unwrap_or_default();
    let (suppressed_events, set_suppressed_events) = signal(initial_suppressed);
    Effect::new(move |_| {
        let current = suppressed_events.get();
        save_json_pref("bruhs.auto_record.suppressed_events", &current);
    });
    let (session_skipped, set_session_skipped) = signal::<HashSet<String>>(HashSet::new());

    provide_context(crate::activity_monitor::MonitorFeedback(RwSignal::new(
        String::new(),
    )));
    provide_context(MeetingsSignal(meetings));
    provide_context(SetMeetingsSignal(set_meetings));
    provide_context(LiveMeetingIdSignal(live_meeting_id));
    provide_context(SetLiveMeetingIdSignal(set_live_meeting_id));
    provide_context(PeopleSignal(people));
    provide_context(SetPeopleSignal(set_people));
    provide_context(SelectedPersonSignal(RwSignal::new(None)));
    provide_context(MeetingSegmentsSignal(meeting_segments));
    provide_context(crate::transcript_sync::TranscriptSync::new(
        meetings,
        set_meetings,
        meeting_segments,
        set_meeting_segments,
        live_meeting_id,
        set_live_meeting_id,
        expect_context::<crate::components::banner::BannerStackSignal>(),
    ));
    provide_context(CaptureStatusSignal(capture_status));
    provide_context(SetCaptureStatusSignal(set_capture_status));
    provide_context(AutoRecordSettingsSignal(auto_record));
    provide_context(SetAutoRecordSettingsSignal(set_auto_record));
    provide_context(PendingAutoRecordSignal(pending_auto_record));
    provide_context(SetPendingAutoRecordSignal(set_pending_auto_record));
    provide_context(SuppressedEventsSignal(suppressed_events));
    provide_context(SetSuppressedEventsSignal(set_suppressed_events));
    provide_context(SessionSkippedEventsSignal(session_skipped));
    provide_context(SetSessionSkippedEventsSignal(set_session_skipped));
    // Phase 4c: cooldown timestamp for audio-driven auto-record prompts.
    provide_context(AudioPromptCooldownSignal(RwSignal::new(0f64)));
    provide_context(AudioLevelSignal(RwSignal::new((0.0, None))));
    provide_context(crate::auto_record::RecordingInFlight::new());
}
