//! Periodic tick that drives calendar-driven auto-record / auto-stop.
//! Fires every TICK_INTERVAL_MS; on each tick it hands the calendar events,
//! meetings, live meeting and capture status to the shared policy
//! ([`bruhs_models::auto_record::decide`]) and acts on its answer.

use std::collections::HashSet;

use gloo_timers::callback::Interval;
use leptos::prelude::*;

use bruhs_models::auto_record::{auto_record_event_key, decide, AutoRecordDecision, TickInput};
use bruhs_models::ids::{IdError, MeetingId};

use crate::api;
use crate::components::banner::{BannerKind, BannerStackSignal};
use crate::state::{
    AutoRecordSettings, AutoRecordSettingsSignal, CalendarEvent, CalendarEventsSignal,
    CaptureStatusSignal, DiarizeSettings, DiarizeSettingsSignal, LiveMeetingIdSignal,
    LiveWhisperModelSignal, Meeting, MeetingStatus, MeetingsSignal, PendingAutoRecordSignal,
    SessionSkippedEventsSignal, SetLiveMeetingIdSignal, SetMeetingsSignal,
    SetPendingAutoRecordSignal, SuppressedEventsSignal, WhisperModel, WhisperModelSignal,
};

const TICK_INTERVAL_MS: u32 = 60_000;

/// One recording start or stop at a time, whoever asks: the watcher, the
/// prompt's Record button or countdown, or the Meetings page's Record button.
#[derive(Clone, Copy)]
pub struct RecordingInFlight(StoredValue<bool>);

impl RecordingInFlight {
    pub fn new() -> Self {
        Self(StoredValue::new(false))
    }

    /// Whether a start or stop is running.
    pub fn is_busy(self) -> bool {
        self.0.try_get_value().unwrap_or(false)
    }

    /// Claim the slot; `None` while another start or stop holds it. The slot
    /// frees when the guard drops.
    #[must_use]
    pub fn try_begin(self) -> Option<InFlightGuard> {
        if self.is_busy() {
            return None;
        }
        self.0.set_value(true);
        Some(InFlightGuard(self.0))
    }
}

impl Default for RecordingInFlight {
    fn default() -> Self {
        Self::new()
    }
}

/// Holds [`RecordingInFlight`] until dropped.
pub struct InFlightGuard(StoredValue<bool>);

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        // The app (and its arena) may already be gone; then nothing waits.
        self.0.try_set_value(false);
    }
}

// ---------------------------------------------------------------------------
// Watcher
// ---------------------------------------------------------------------------

/// The signals a recording start or stop updates, and where a failure is
/// reported. Read from context once, by a component, so the async work that
/// outlives it never looks context up.
#[derive(Clone, Copy)]
pub struct RecordingTargets {
    pub set_meetings: WriteSignal<Vec<Meeting>>,
    pub set_live_meeting_id: WriteSignal<Option<String>>,
    /// `None` only outside the app shell (where nothing can show a notice).
    pub banners: Option<BannerStackSignal>,
    pub in_flight: RecordingInFlight,
}

impl RecordingTargets {
    /// The targets the app shell provides.
    pub fn from_context() -> Self {
        let SetMeetingsSignal(set_meetings) = expect_context::<SetMeetingsSignal>();
        let SetLiveMeetingIdSignal(set_live_meeting_id) =
            expect_context::<SetLiveMeetingIdSignal>();
        Self {
            set_meetings,
            set_live_meeting_id,
            banners: use_context::<BannerStackSignal>(),
            in_flight: expect_context::<RecordingInFlight>(),
        }
    }

    fn notify(self, message: String) {
        match self.banners {
            Some(banners) => banners.push(BannerKind::Error, message, Some(12_000)),
            None => log::warn!("{message}"),
        }
    }
}

/// Transcription and diarization options for a new recording.
#[derive(Clone, Debug)]
pub struct CaptureOptions {
    pub whisper_model: WhisperModel,
    pub live_whisper_model: WhisperModel,
    pub diarize: DiarizeSettings,
}

/// Where [`CaptureOptions`] come from: the settings signals, read from
/// context once by a component.
#[derive(Clone, Copy)]
pub struct CaptureSettings {
    whisper_model: ReadSignal<WhisperModel>,
    live_whisper_model: ReadSignal<WhisperModel>,
    diarize: ReadSignal<DiarizeSettings>,
}

impl CaptureSettings {
    pub fn from_context() -> Self {
        let WhisperModelSignal(whisper_model) = expect_context();
        let LiveWhisperModelSignal(live_whisper_model) = expect_context();
        let DiarizeSettingsSignal(diarize) = expect_context();
        Self {
            whisper_model,
            live_whisper_model,
            diarize,
        }
    }

    /// The options as currently set.
    pub fn current(self) -> CaptureOptions {
        CaptureOptions {
            whisper_model: self.whisper_model.get_untracked(),
            live_whisper_model: self.live_whisper_model.get_untracked(),
            diarize: self.diarize.get_untracked(),
        }
    }
}

impl CaptureOptions {
    fn body(&self, meeting: &Meeting) -> Result<api::CaptureStartBody, IdError> {
        Ok(api::CaptureStartBody {
            transcription: None,
            meeting_id: MeetingId::try_from(meeting.id.as_str())?,
            started_at_ms: api::epoch_ms(meeting.started_at_ms),
            model: Some(self.whisper_model),
            live_model: Some(self.live_whisper_model),
            subspeaker_enabled: Some(self.diarize.subspeaker_enabled),
            cross_meeting_linking_auto: Some(self.diarize.cross_meeting_linking_auto),
            cross_meeting_linking_threshold: Some(self.diarize.cross_meeting_linking_threshold),
            max_speakers: Some(self.diarize.max_speakers),
            video_ocr_enabled: Some(self.diarize.video_ocr_enabled),
        })
    }
}

/// Clear `slot` if it still points at `id` (the user may have moved on).
fn release(slot: &mut Option<String>, id: &str) {
    if slot.as_deref() == Some(id) {
        *slot = None;
    }
}

/// `meeting` stopped at `now_ms`, as `POST /api/meetings/{id}/stop` leaves
/// it.
fn stopped(mut meeting: Meeting, now_ms: f64) -> Meeting {
    meeting.status = MeetingStatus::Ended;
    meeting.ended_at_ms.get_or_insert(now_ms);
    meeting
}

/// Create `body`'s meeting, make it the live and selected one, and start the
/// capture daemon on it. When the daemon does not start, nothing is left
/// pretending to record: the live/selected ids roll back, the meeting is
/// stopped (on the server when it answers, locally regardless) and a notice
/// says why. The desktop hub does the same (`MeetingsHub::create_and_capture`).
pub async fn create_and_record(
    body: api::CreateMeetingBody,
    options: CaptureOptions,
    targets: RecordingTargets,
) {
    let Some(_in_flight) = targets.in_flight.try_begin() else {
        log::info!("recording: a start or stop is already running; not starting another");
        return;
    };
    let meeting = match api::create_meeting(&body).await {
        Ok(meeting) => meeting,
        Err(error) => {
            targets.notify(format!("Could not create the meeting: {error}"));
            return;
        }
    };
    let id = meeting.id.clone();
    targets
        .set_meetings
        .update(|ms| ms.insert(0, meeting.clone()));
    crate::storage::persist_meeting(meeting.clone());
    targets.set_live_meeting_id.set(Some(id.clone()));

    let started = match options.body(&meeting) {
        Ok(capture) => api::capture_start(&capture)
            .await
            .map_err(|error| error.to_string()),
        Err(error) => Err(format!(
            "the server returned an unusable meeting id ({error})"
        )),
    };
    let Err(reason) = started else {
        return;
    };
    targets
        .set_live_meeting_id
        .update(|live| release(live, &id));
    let now = js_sys::Date::now();
    let stop = api::StopMeetingBody {
        ended_at_ms: api::epoch_ms(now),
    };
    let marked = match api::stop_meeting(&id, &stop).await {
        Ok(updated) => updated,
        Err(error) => {
            log::warn!("recording: could not stop meeting {id} on the server: {error}");
            stopped(meeting, now)
        }
    };
    targets.set_meetings.update(|ms| {
        if let Some(slot) = ms.iter_mut().find(|m| m.id == marked.id) {
            *slot = marked.clone();
        }
    });
    crate::storage::persist_meeting(marked);
    targets.notify(format!(
        "Recording did not start: {reason}. The meeting was stopped."
    ));
}

/// What the watcher reads each tick, and the prompt it raises.
#[derive(Clone, Copy)]
pub struct WatcherInputs {
    settings: ReadSignal<AutoRecordSettings>,
    events: ReadSignal<Vec<CalendarEvent>>,
    capture_status: ReadSignal<api::CaptureStatus>,
    meetings: ReadSignal<Vec<Meeting>>,
    live_meeting_id: ReadSignal<Option<String>>,
    pending: ReadSignal<Option<CalendarEvent>>,
    set_pending: WriteSignal<Option<CalendarEvent>>,
    suppressed: ReadSignal<HashSet<String>>,
    session_skipped: ReadSignal<HashSet<String>>,
    capture: CaptureSettings,
}

impl WatcherInputs {
    pub fn from_context() -> Self {
        let AutoRecordSettingsSignal(settings) = expect_context();
        let CalendarEventsSignal(events) = expect_context();
        let CaptureStatusSignal(capture_status) = expect_context();
        let MeetingsSignal(meetings) = expect_context();
        let LiveMeetingIdSignal(live_meeting_id) = expect_context();
        let PendingAutoRecordSignal(pending) = expect_context();
        let SetPendingAutoRecordSignal(set_pending) = expect_context();
        let SuppressedEventsSignal(suppressed) = expect_context();
        let SessionSkippedEventsSignal(session_skipped) = expect_context();
        Self {
            settings,
            events,
            capture_status,
            meetings,
            live_meeting_id,
            pending,
            set_pending,
            suppressed,
            session_skipped,
            capture: CaptureSettings::from_context(),
        }
    }

    /// This tick's decision, read without copying the event or meeting lists.
    fn decide(self, in_flight: bool) -> AutoRecordDecision {
        let capture_running = self.capture_status.with_untracked(|s| s.is_running());
        self.settings.with_untracked(|settings| {
            self.events.with_untracked(|events| {
                self.meetings.with_untracked(|meetings| {
                    self.live_meeting_id.with_untracked(|live| {
                        self.pending.with_untracked(|pending| {
                            self.suppressed.with_untracked(|suppressed| {
                                self.session_skipped.with_untracked(|session_skipped| {
                                    decide(&TickInput {
                                        now_ms: js_sys::Date::now(),
                                        settings,
                                        capture_running,
                                        events,
                                        meetings,
                                        live_meeting_id: live.as_deref(),
                                        pending: pending.as_ref(),
                                        suppressed,
                                        session_skipped,
                                        in_flight,
                                    })
                                })
                            })
                        })
                    })
                })
            })
        })
    }
}

/// Start the watcher. Wires once at App mount; the returned Interval
/// is `forget()`-ed so it lives for the app's lifetime.
pub fn start_watcher(inputs: WatcherInputs, targets: RecordingTargets) {
    let interval = Interval::new(TICK_INTERVAL_MS, move || {
        match inputs.decide(targets.in_flight.is_busy()) {
            AutoRecordDecision::Idle => {}
            AutoRecordDecision::AutoStop { meeting_id } => trigger_auto_stop(meeting_id, targets),
            AutoRecordDecision::AutoStart(event) => {
                start_recording_for_event(event, inputs.capture.current(), targets);
            }
            AutoRecordDecision::Prompt(event) => inputs.set_pending.set(Some(event)),
        }
    });
    interval.forget();
}

// ---------------------------------------------------------------------------
// Public entrypoints (for callers outside the watcher tick loop)
// ---------------------------------------------------------------------------

/// Start recording `event` (the watcher, the prompt modal's Record button
/// and countdown, or the event dialog's Record meeting button) through
/// [`create_and_record`], so a capture that does not start rolls back and
/// says why. Shares [`RecordingInFlight`] with every other start and stop,
/// so a prompt answered while the watcher is starting the same meeting does
/// not record it twice.
pub fn start_recording_for_event(
    event: CalendarEvent,
    options: CaptureOptions,
    targets: RecordingTargets,
) {
    log::info!(
        "auto-record: starting meeting for event {} ({})",
        auto_record_event_key(&event),
        event.title
    );
    let body = meeting_body_for_event(event, js_sys::Date::now());
    wasm_bindgen_futures::spawn_local(create_and_record(body, options, targets));
}

/// The meeting a recording of `event` that starts at `now_ms` creates.
fn meeting_body_for_event(event: CalendarEvent, now_ms: f64) -> api::CreateMeetingBody {
    // Use the stable Google event ID as the linkage key; fall back to the
    // local UUID only for locally-created events that have no google_event_id,
    // so a re-sync (which mints a fresh local id) keeps the already-recorded
    // guard working.
    let link_key = auto_record_event_key(&event).to_string();
    api::CreateMeetingBody {
        title: event.title,
        started_at_ms: api::epoch_ms(now_ms),
        calendar_event_id: Some(link_key),
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn trigger_auto_stop(meeting_id: String, targets: RecordingTargets) {
    let Some(in_flight) = targets.in_flight.try_begin() else {
        return;
    };
    log::info!("auto-record: stopping meeting {meeting_id}");
    wasm_bindgen_futures::spawn_local(async move {
        let _in_flight = in_flight;
        let body = api::StopMeetingBody {
            ended_at_ms: api::epoch_ms(js_sys::Date::now()),
        };
        match api::stop_meeting(&meeting_id, &body).await {
            Ok(updated) => {
                targets.set_meetings.update(|ms| {
                    if let Some(slot) = ms.iter_mut().find(|m| m.id == updated.id) {
                        *slot = updated.clone();
                    }
                });
                crate::storage::persist_meeting(updated);
                targets.set_live_meeting_id.set(None);
                if let Err(e) = api::capture_stop().await {
                    log::warn!("auto-record: capture_stop failed: {e}");
                }
            }
            Err(e) => log::warn!("auto-record: stop_meeting failed: {e}"),
        }
    });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A failed capture start gives the live/selected slot back only if it
    /// still holds that meeting.
    #[test]
    fn a_failed_start_releases_only_its_own_meeting() {
        let mut slot = Some("m1".to_string());
        release(&mut slot, "m1");
        assert_eq!(slot, None);
        let mut other = Some("m2".to_string());
        release(&mut other, "m1");
        assert_eq!(other.as_deref(), Some("m2"));
    }

    #[test]
    fn a_meeting_that_never_recorded_is_stopped_at_the_failure() {
        let meeting = Meeting::new("Standup", 1_000.0);
        assert_eq!(meeting.status, MeetingStatus::Live);
        let ended = stopped(meeting, 2_000.0);
        assert_eq!(ended.status, MeetingStatus::Ended);
        assert_eq!(ended.ended_at_ms, Some(2_000.0));
    }

    #[test]
    fn capture_options_build_the_shared_start_body() {
        let meeting = Meeting::new("Standup", 1_234.9);
        let options = CaptureOptions {
            whisper_model: WhisperModel::default(),
            live_whisper_model: WhisperModel::default(),
            diarize: DiarizeSettings::default(),
        };
        let body = options.body(&meeting).unwrap();
        assert_eq!(body.meeting_id.as_str(), meeting.id);
        assert_eq!(body.started_at_ms.as_millis(), 1_234);
        let mut bad = meeting;
        bad.id = "a/b".into();
        assert!(options.body(&bad).is_err());
    }

    fn event(id: &str, google_event_id: Option<&str>) -> CalendarEvent {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "title": "Standup",
            "start_at_ms": 1_000.0,
            "end_at_ms": 2_000.0,
            "all_day": false,
            "color": null,
            "notes": null,
            "created_at": 0,
            "updated_at": 0,
            "google_event_id": google_event_id,
        }))
        .unwrap()
    }

    /// A recording of an event links the meeting by the stable key and
    /// starts now (not at the event's scheduled start).
    #[test]
    fn an_event_recording_links_by_the_stable_key_and_starts_now() {
        let google = meeting_body_for_event(event("local-1", Some("g-1")), 5_000.4);
        assert_eq!(google.title, "Standup");
        assert_eq!(google.calendar_event_id.as_deref(), Some("g-1"));
        assert_eq!(google.started_at_ms.as_millis(), 5_000);
        let local = meeting_body_for_event(event("local-1", None), 5_000.0);
        assert_eq!(local.calendar_event_id.as_deref(), Some("local-1"));
    }

    /// One start or stop at a time across every caller; the slot frees
    /// when the work ends (the guard drops), even on an early return.
    #[test]
    fn recording_starts_and_stops_share_one_slot() {
        let owner = Owner::new();
        owner.with(|| {
            let slot = RecordingInFlight::new();
            let first = slot.try_begin().expect("free at first");
            assert!(slot.is_busy());
            assert!(slot.try_begin().is_none(), "a second caller waits its turn");
            drop(first);
            assert!(!slot.is_busy());
            assert!(slot.try_begin().is_some());
        });
    }
}
