//! The desktop's port of the web's meetings / people / Google-sync state:
//! `state.rs` signals, `meetings_stream.rs`, `calendar_sync_stream.rs`,
//! `auto_record.rs` and the mutations the pages call. One gpui entity owns
//! the data, talks to bruhs-server through `ServerClient` on the tokio
//! runtime, and applies results on the UI thread through a job channel.

use bruhs_models::num::ToIntSaturating;
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use bruhs_models::api::{ActiveRecording, RunningCapture, SystemAudioPermission};
use bruhs_models::ids::MeetingId;
use bruhs_models::time::EpochMs;
use bruhs_models::{
    AutoRecordSettings, Calendar, CalendarEvent, CalendarSyncEvent, DiarizeSettings, EventSource,
    GoogleCalendarSyncState, Meeting, MeetingEvent, MeetingStatus, PersonProfile,
    PersonalitySettings, TranscriptSegment, WhisperModel,
};
use gpui::{AppContext as _, AsyncApp, Context, Entity, EventEmitter};
use serde::{Deserialize, Serialize};
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

use crate::server_client::{
    CaptureStartBody, CaptureStatus, CreateMeetingBody, ServerClient, ServerError, SseMessage,
};
mod people_cache;
pub use people_cache::{PeopleMergeArchive, PeopleSnapshot};
mod meeting_cache;
pub use meeting_cache::MeetingCache;
mod activity_monitor;
pub mod audio_meter;
mod local_focus;
mod meeting_actions;
mod stream_recovery;
#[cfg(test)]
pub(crate) use meeting_actions::test_support::ActionServer;
pub use meeting_actions::{ActionCheckpoints, MeetingActionStatus};

/// Web `auto_record.rs` `TICK_INTERVAL_MS`.
pub const AUTO_RECORD_TICK_MS: u64 = 60_000;
/// Web `meetings_stream.rs` `AUDIO_PROMPT_COOLDOWN_MS`.
pub const AUDIO_PROMPT_COOLDOWN_MS: f64 = 5.0 * 60.0 * 1000.0;
/// Web `auto_record_prompt.rs` `COUNTDOWN_SECS`.
pub const PROMPT_COUNTDOWN_SECS: i32 = 30;
/// Web `search_page.rs` `SEARCH_LIMIT`.
pub const SEARCH_LIMIT: usize = 50;
/// Web `google_calendar_settings.rs`: poll the sync state every 2s, 60×.
const OAUTH_POLL_ATTEMPTS: u32 = 60;
const OAUTH_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Preferences the web keeps in `localStorage` (`bruhs.whisper_model`,
/// `bruhs.whisper_model_live`, `bruhs.diarize`, `bruhs.auto_record`,
/// `bruhs.auto_record.suppressed_events`, `bruhs.personality`); the desktop
/// persists them in `desktop-state.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MeetingPrefs {
    pub whisper_model: WhisperModel,
    pub live_whisper_model: WhisperModel,
    pub diarize: DiarizeSettings,
    pub auto_record: AutoRecordSettings,
    pub personality: PersonalitySettings,
    pub suppressed_events: Vec<String>,
}

/// Transient notice (web `push_banner`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HubNotice {
    pub message: String,
    pub warning: bool,
}

/// What the hub tells the workspace.
#[derive(Clone, Debug, PartialEq)]
pub enum HubEvent {
    /// Prefs changed; persist the workspace snapshot.
    PrefsChanged,
    PeopleChanged,
    /// The workspace contributes private-reference IDs before a directory refresh.
    RefreshPeople,
    PersonMergeResolved(Box<bruhs_models::PersonMergeResolution>),
    PersonMergesResolved(Vec<bruhs_models::PersonMergeResolution>),
    MeetingsChanged,
    /// Only these transcript rows need new geometry; `None` means a full snapshot.
    TranscriptChanged {
        meeting_id: String,
        range: Option<std::ops::Range<usize>>,
    },
    /// "Chat with this meeting" resolved a server thread; open a chat.
    OpenMeetingChat {
        meeting_title: String,
        thread_id: String,
    },
    /// A meeting was created or opened; navigate to it.
    ShowMeeting {
        meeting_id: String,
    },
    /// Capture acknowledged start; now it is safe to compact the workspace.
    EnterMeetingDock,
    CalendarConflict {
        local: Box<CalendarEvent>,
        remote: Box<CalendarEvent>,
    },
}

use bruhs_models::auto_record::auto_record_event_key as event_key;

/// Shown when `/api/capture/status` is 502: the capture daemon answered but
/// bruhs-server could not use it, most often because it refused the server's
/// token (the server logs the reason; the reply body is never shown).
const CAPTURE_STATUS_BAD_GATEWAY_NOTICE: &str = "Capture status unavailable: the capture daemon \
     answered but bruhs-server could not use it. Restart the capture daemon or check \
     BRUHS_CAPTURE_TOKEN_FILE.";

#[cfg(test)]
use bruhs_models::auto_record::is_auto_start_candidate;
pub use bruhs_models::auto_record::{decide, AutoRecordDecision, TickInput};

/// Web `search_page::build_snippet`: `(snippet, match_start, match_end)`
/// around the first case-insensitive match, on char boundaries.
#[cfg(test)]
pub fn build_snippet(text: &str, query: &str, half_window: usize) -> (String, usize, usize) {
    let text_lc = text.to_lowercase();
    let q_lc = query.to_lowercase();
    let match_start = text_lc.find(&q_lc).unwrap_or(0);
    let match_end = match_start + query.len();
    let snip_start = match_start.saturating_sub(half_window);
    let snip_end = (match_end + half_window).min(text.len());
    let snip_start = text
        .char_indices()
        .map(|(i, _)| i)
        .rev()
        .find(|&i| i <= snip_start)
        .unwrap_or(0);
    let snip_end = text
        .char_indices()
        .map(|(i, _)| i)
        .find(|&i| i >= snip_end)
        .unwrap_or(text.len());
    let snippet = text[snip_start..snip_end].to_string();
    let rel_start = align_to_char_boundary(&snippet, match_start.saturating_sub(snip_start), true);
    let rel_end = align_to_char_boundary(
        &snippet,
        match_start.saturating_sub(snip_start) + query.len(),
        false,
    );
    (snippet, rel_start, rel_end)
}

#[cfg(test)]
fn align_to_char_boundary(s: &str, idx: usize, floor: bool) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    let mut i = idx;
    if floor {
        while i > 0 && !s.is_char_boundary(i) {
            i -= 1;
        }
    } else {
        while i < s.len() && !s.is_char_boundary(i) {
            i += 1;
        }
    }
    i
}

/// Web `storage::search_segments`: case-insensitive substring match over
/// every cached segment, grouped by meeting, capped at `limit`.
#[cfg(test)]
pub fn search_segments(
    segments: &BTreeMap<String, Vec<TranscriptSegment>>,
    query: &str,
    limit: usize,
) -> Vec<TranscriptSegment> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for segs in segments.values() {
        for seg in segs {
            if seg.text.to_lowercase().contains(&needle) {
                out.push(seg.clone());
                if out.len() >= limit {
                    return out;
                }
            }
        }
    }
    out
}

/// Web `meeting_detail::group_by_speaker`.
#[cfg(test)]
pub fn group_by_speaker(
    segments: &[TranscriptSegment],
) -> Vec<(Option<String>, Vec<TranscriptSegment>)> {
    let mut groups: Vec<(Option<String>, Vec<TranscriptSegment>)> = Vec::new();
    for seg in segments {
        if let Some(last) = groups.last_mut() {
            if last.0 == seg.speaker_id {
                last.1.push(seg.clone());
                continue;
            }
        }
        groups.push((seg.speaker_id.clone(), vec![seg.clone()]));
    }
    groups
}

/// Web `meetings_page::format_started_at` (local time).
pub fn format_started_at(ms: f64) -> String {
    use chrono::{Local, TimeZone};
    Local
        .timestamp_millis_opt(EpochMs::saturating_from_f64(ms).as_millis())
        .single()
        .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

/// Web `meetings_page::format_duration`.
pub fn format_duration(ms: f64) -> String {
    let total_secs = (ms / 1000.0).max(0.0).to_u64_saturating();
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

/// Web `meeting_live::format_elapsed`.
pub fn format_elapsed(elapsed_ms: f64) -> String {
    let total_secs = (elapsed_ms / 1000.0).max(0.0).to_u64_saturating();
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

/// Web `meetings_page::summary_preview`.
pub fn summary_preview(text: &str, max_chars: usize) -> String {
    let sentence_end = text
        .char_indices()
        .find_map(|(i, c)| matches!(c, '.' | '!' | '?').then(|| i + c.len_utf8()))
        .unwrap_or(text.len());
    let first = &text[..sentence_end];
    if first.chars().count() <= max_chars {
        first.trim_end().to_string()
    } else {
        let cut: String = first.chars().take(max_chars).collect();
        format!("{}…", cut.trim_end())
    }
}

/// Web `voice_enroll::EnrollState`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnrollState {
    Idle,
    Countdown(i32),
    Recording,
    Uploading,
    Success,
    Error(String),
}

type Job = Box<dyn FnOnce(&mut MeetingsHub, &mut Context<'_, MeetingsHub>) + Send>;

pub struct MeetingsHub {
    pub local_focus: local_focus::FocusState,
    client: Arc<ServerClient>,
    runtime: Arc<Runtime>,
    jobs: UnboundedSender<Job>,
    started: bool,

    pub meetings: Vec<Meeting>,
    pub people: Vec<PersonProfile>,
    pending_person_creates: HashSet<String>,
    person_creates_in_flight: HashSet<String>,
    person_creates_attempted: HashSet<String>,
    person_merge_archives: BTreeMap<String, Arc<PeopleMergeArchive>>,
    person_merge_recoveries: BTreeMap<String, Vec<Arc<PeopleMergeArchive>>>,
    person_merges_waiting: HashSet<String>,
    people_refreshing: bool,
    people_refresh_again: bool,
    people_refresh_failed: bool,
    pub segments: BTreeMap<String, Vec<TranscriptSegment>>,
    /// Chunk ids the user or another client removed. Replays of these ids are ignored.
    removed_chunks: BTreeMap<String, HashSet<String>>,
    meeting_cache: MeetingCache,
    transcript_indexes: BTreeMap<String, meeting_cache::TranscriptIndex>,
    meeting_refresh_generation: u64,
    deleted_meeting_ids: HashSet<String>,
    segments_in_flight: HashSet<String>,
    stream_recovery: stream_recovery::Recovery,
    pub capture_status: CaptureStatus,
    pub live_meeting_id: Option<String>,
    /// Live input levels; updated without notifying the hub.
    pub audio_meter: Entity<audio_meter::AudioLevelMeter>,
    pub google_sync: GoogleCalendarSyncState,
    pub calendars: Vec<Calendar>,
    pub google_events: Vec<CalendarEvent>,
    pub prefs: MeetingPrefs,
    monitor_session: Option<Arc<bruhs_models::ActivityMonitorSession>>,
    monitor_generation: u64,
    pub monitor_feedback: String,
    pub pending_auto_record: Option<CalendarEvent>,
    pub prompt_countdown: i32,
    pub prompt_paused: bool,
    session_skipped: HashSet<String>,
    audio_prompt_cooldown_ms: f64,
    auto_record_in_flight: bool,
    capture_stopped_session: Option<String>,
    pub notice: Option<HubNotice>,
    /// Meetings whose segments have been fetched for search.
    search_cached: HashSet<String>,
    search_revision: u64,
    pub search_loading: bool,
    meeting_actions: BTreeMap<String, meeting_actions::Entry>,
    pub google_busy: bool,
    pub google_error: Option<String>,
    pub enroll: Option<(String, EnrollState)>,
    pub server_online: bool,
}

impl EventEmitter<HubEvent> for MeetingsHub {}

/// Shown when bruhs-server sends a live event this build cannot read.
const STALE_MEETINGS_NOTICE: &str =
    "Some live meeting updates could not be read; meetings may be out of date. Update bruhs-desktop to match bruhs-server.";
const STALE_CALENDAR_NOTICE: &str =
    "Some calendar sync updates could not be read; the calendar may be out of date. Update bruhs-desktop to match bruhs-server.";

impl MeetingsHub {
    /// Isolated native-window acceptance. No start(), network worker or capture
    /// action is available; the Stop control is deliberately held disabled.
    #[cfg(any(debug_assertions, test))]
    pub fn synthetic_dock_fixture(runtime: Arc<Runtime>, cx: &mut Context<'_, Self>) -> Self {
        let mut hub = Self::new(
            ServerClient::new("http://127.0.0.1:9", "http://127.0.0.1:9")
                .unwrap_or_else(|error| ServerClient::unavailable(&error)),
            runtime,
            MeetingPrefs::default(),
            cx,
        );
        let meeting = Meeting::new(
            "Synthetic dock acceptance — no capture",
            EpochMs::now().as_f64() - 65_000.0,
        );
        hub.live_meeting_id = Some(meeting.id.clone());
        hub.meetings.push(meeting);
        hub.auto_record_in_flight = true;
        hub
    }

    pub fn new(
        client: ServerClient,
        runtime: Arc<Runtime>,
        mut prefs: MeetingPrefs,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        // Persisted preferences are not permission to open a microphone.
        prefs.auto_record.audio_driven_enabled = false;
        let (jobs, mut rx) = unbounded_channel::<Job>();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            while let Some(job) = rx.recv().await {
                if this.update(cx, |hub, cx| job(hub, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            local_focus: local_focus::FocusState::default(),
            client: Arc::new(client),
            runtime,
            jobs,
            started: false,
            meetings: Vec::new(),
            people: Vec::new(),
            pending_person_creates: HashSet::new(),
            person_creates_in_flight: HashSet::new(),
            person_creates_attempted: HashSet::new(),
            person_merge_archives: BTreeMap::new(),
            person_merge_recoveries: BTreeMap::new(),
            person_merges_waiting: HashSet::new(),
            people_refreshing: false,
            people_refresh_again: false,
            people_refresh_failed: false,
            segments: BTreeMap::new(),
            removed_chunks: BTreeMap::new(),
            meeting_cache: MeetingCache::default(),
            transcript_indexes: BTreeMap::new(),
            meeting_refresh_generation: 0,
            deleted_meeting_ids: HashSet::new(),
            segments_in_flight: HashSet::new(),
            stream_recovery: stream_recovery::Recovery::default(),
            capture_status: CaptureStatus::default(),
            live_meeting_id: None,
            audio_meter: cx.new(|_| audio_meter::AudioLevelMeter::default()),
            google_sync: GoogleCalendarSyncState::default(),
            calendars: Vec::new(),
            google_events: Vec::new(),
            prefs,
            monitor_session: None,
            monitor_generation: 0,
            monitor_feedback: String::new(),
            pending_auto_record: None,
            prompt_countdown: PROMPT_COUNTDOWN_SECS,
            prompt_paused: false,
            session_skipped: HashSet::new(),
            audio_prompt_cooldown_ms: 0.0,
            auto_record_in_flight: false,
            capture_stopped_session: None,
            notice: None,
            search_cached: HashSet::new(),
            search_revision: 0,
            search_loading: false,
            meeting_actions: BTreeMap::new(),
            google_busy: false,
            google_error: None,
            enroll: None,
            server_online: false,
        }
    }

    /// Run `fut` on the tokio runtime and apply its result on the UI thread.
    fn run<T, F, A>(&self, fut: F, apply: A)
    where
        T: Send + 'static,
        F: std::future::Future<Output = T> + Send + 'static,
        A: FnOnce(&mut Self, T, &mut Context<'_, Self>) + Send + 'static,
    {
        let jobs = self.jobs.clone();
        self.runtime.spawn(async move {
            let value = fut.await;
            let _ = jobs.send(Box::new(move |hub, cx| apply(hub, value, cx)));
        });
    }

    /// Record what a failed request says about the server: only an
    /// unreachable server is offline; one that answered with an error (or an
    /// unexpected body) is up, and the failure is logged.
    fn note_request_failure(&mut self, error: &ServerError) {
        if error.is_transport() {
            self.server_online = false;
        } else {
            tracing::warn!("{error}");
            self.server_online = true;
        }
    }

    /// Show `message` in the workspace's transient banner (web
    /// `push_banner`).
    pub fn show_notice(
        &mut self,
        message: impl Into<String>,
        warning: bool,
        cx: &mut Context<'_, Self>,
    ) {
        self.notify_notice(message, warning, cx);
    }

    fn notify_notice(
        &mut self,
        message: impl Into<String>,
        warning: bool,
        cx: &mut Context<'_, Self>,
    ) {
        let message = message.into();
        self.notice = Some(HubNotice {
            message: message.clone(),
            warning,
        });
        cx.notify();
        // Web banners expire (8s); clear ours unless a newer one replaced it.
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            cx.background_executor().timer(Duration::from_secs(8)).await;
            let _ = this.update(cx, |hub, cx| {
                if hub.notice.as_ref().map(|n| n.message.as_str()) == Some(message.as_str()) {
                    hub.notice = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub fn dismiss_notice(&mut self, cx: &mut Context<'_, Self>) {
        self.notice = None;
        cx.notify();
    }

    /// Web `App` mount: hydrate, subscribe to both streams, start the
    /// auto-record watcher. Idempotent.
    pub fn start(&mut self, cx: &mut Context<'_, Self>) {
        // gpui's test scheduler rejects wake-ups from the tokio threads, and
        // tests must not touch the network; the pages render offline states.
        if self.started || cfg!(test) {
            return;
        }
        self.started = true;
        self.refresh_meetings(cx);
        self.refresh_people(cx);
        self.refresh_capture_status(cx);
        self.refresh_google(cx);
        self.start_meetings_stream();
        self.start_calendar_stream();
        self.start_auto_record_watcher(cx);
    }

    // ── Hydration ────────────────────────────────────────────────────────

    pub fn refresh_meetings(&mut self, _cx: &mut Context<'_, Self>) {
        let client = self.client.clone();
        let expected = self.meeting_cache();
        self.meeting_refresh_generation = self.meeting_refresh_generation.wrapping_add(1);
        let generation = self.meeting_refresh_generation;
        self.run(
            async move { client.fetch_meetings().await },
            move |hub, result, cx| {
                if hub.meeting_refresh_generation != generation {
                    return;
                }
                match result {
                    Ok(meetings) => {
                        hub.server_online = true;
                        hub.merge_meetings(meetings, &expected, cx);
                        if let Some(live) = hub
                            .meetings
                            .iter()
                            .find(|m| m.status == MeetingStatus::Live)
                        {
                            if hub.live_meeting_id.is_none()
                                && hub.capture_status.recording().is_some()
                            {
                                hub.live_meeting_id = Some(live.id.clone());
                            }
                        }
                    }
                    Err(error) => hub.note_request_failure(&error),
                }
                cx.notify();
            },
        );
    }

    pub fn refresh_people(&mut self, cx: &mut Context<'_, Self>) {
        cx.emit(HubEvent::RefreshPeople);
    }

    /// Inserts immediately for responsive capture, then syncs with the server.
    pub fn capture_person(&mut self, person: PersonProfile, cx: &mut Context<'_, Self>) {
        if self.person_merge_archives.contains_key(&person.id)
            || self.person_merges_waiting.contains(&person.id)
        {
            self.notify_notice("This contact identity was merged. Use the surviving contact; no retired identity was recreated.", true, cx);
            return;
        }
        if let Some(current) = self.people.iter_mut().find(|item| item.id == person.id) {
            *current = person.clone();
        } else {
            self.people.push(person.clone());
        }
        self.person_creates_attempted.remove(&person.id);
        self.pending_person_creates.insert(person.id);
        self.people_changed(cx);
        self.sync_pending_people(cx);
    }

    pub fn refresh_capture_status(&mut self, _cx: &mut Context<'_, Self>) {
        let client = self.client.clone();
        self.run(
            async move { client.fetch_capture_status().await },
            |hub, result, cx| {
                match result {
                    Ok(status) => {
                        let recording = status.recording().map(|r| r.meeting_id.as_str());
                        if recording.is_none() || recording != hub.live_meeting_id.as_deref() {
                            hub.stop_local_focus(
                                "Local app context is off: capture is not active for this meeting.",
                                cx,
                            );
                        }
                        hub.server_online = true;
                        hub.capture_status = status;
                    }
                    Err(error) => {
                        hub.note_request_failure(&error);
                        hub.capture_status = CaptureStatus::default();
                        hub.stop_local_focus(
                            "Local app context paused: capture status is unavailable.",
                            cx,
                        );
                        if error.is_bad_gateway() {
                            // The daemon answered but the server could not use
                            // it (say, it refused the server's token): say so,
                            // so "not running" is not the only word on it.
                            hub.notify_notice(CAPTURE_STATUS_BAD_GATEWAY_NOTICE, true, cx);
                        }
                    }
                }
                cx.notify();
            },
        );
    }

    pub fn refresh_google(&mut self, _cx: &mut Context<'_, Self>) {
        let client = self.client.clone();
        self.run(
            async move {
                let state = client.fetch_google_sync_state().await;
                let calendars = client.fetch_google_calendars().await;
                (state, calendars)
            },
            |hub, (state, calendars), cx| {
                if let Ok(state) = state {
                    hub.google_sync = state;
                }
                if let Ok(calendars) = calendars {
                    hub.calendars = calendars;
                }
                cx.notify();
            },
        );
    }

    // ── Streams ──────────────────────────────────────────────────────────

    fn start_meetings_stream(&self) {
        let (tx, mut rx) = unbounded_channel::<SseMessage<MeetingEvent>>();
        self.runtime.spawn(
            self.client
                .clone()
                .run_sse::<MeetingEvent>("/api/meetings/stream".to_string(), tx),
        );
        let jobs = self.jobs.clone();
        self.runtime.spawn(async move {
            while let Some(message) = rx.recv().await {
                let job: Job = match message {
                    SseMessage::Event(event) => {
                        Box::new(move |hub, cx| hub.apply_meeting_event(event, cx))
                    }
                    SseMessage::Undecodable => Box::new(|hub, cx| {
                        hub.notify_notice(STALE_MEETINGS_NOTICE, true, cx);
                    }),
                    SseMessage::Disconnected => Box::new(move |hub, cx| {
                        hub.stop_local_focus("Local app context paused: meeting connection lost. Enable again after reconnecting.", cx);
                        hub.server_online = false;
                        hub.notify_notice("Live transcript disconnected — reconnecting…", true, cx);
                    }),
                };
                if jobs.send(job).is_err() {
                    break;
                }
            }
        });
    }

    fn start_calendar_stream(&self) {
        let (tx, mut rx) = unbounded_channel::<SseMessage<CalendarSyncEvent>>();
        self.runtime.spawn(
            self.client
                .clone()
                .run_sse::<CalendarSyncEvent>("/api/calendar/events/stream".to_string(), tx),
        );
        let jobs = self.jobs.clone();
        self.runtime.spawn(async move {
            while let Some(message) = rx.recv().await {
                let job: Job = match message {
                    SseMessage::Event(event) => {
                        Box::new(move |hub, cx| hub.apply_calendar_event(event, cx))
                    }
                    SseMessage::Undecodable => Box::new(|hub, cx| {
                        hub.notify_notice(STALE_CALENDAR_NOTICE, true, cx);
                    }),
                    // The calendar shows its own sync state; reconnects are quiet.
                    SseMessage::Disconnected => continue,
                };
                if jobs.send(job).is_err() {
                    break;
                }
            }
        });
    }

    /// Web `meetings_stream.rs` reducer.
    pub fn apply_meeting_event(&mut self, event: MeetingEvent, cx: &mut Context<'_, Self>) {
        let was_online = self.server_online;
        self.server_online = true;
        let mut changed = true;
        match event {
            MeetingEvent::Segment {
                meeting_id,
                segment,
            } => {
                self.receive_segment(meeting_id, segment, cx);
            }
            MeetingEvent::SegmentRemoved {
                meeting_id,
                segment_id,
            } => {
                self.forget_transcript_chunk(&meeting_id, &segment_id, cx);
            }
            MeetingEvent::TranscriptionCompleted { meeting_id } => {
                self.ensure_segments(&meeting_id, cx);
                let extract = self.prefs.personality.extract_observations;
                if self
                    .meeting(&meeting_id)
                    .is_some_and(|meeting| meeting.summary.is_none())
                {
                    self.summarize(meeting_id, extract, false, cx);
                }
            }
            MeetingEvent::SpeakersAssigned { meeting_id } => {
                let client = self.client.clone();
                let id = meeting_id.clone();
                let receipt = self.transcript_read_receipt(&meeting_id);
                self.run(
                    async move { client.refresh_transcript(&id).await },
                    move |hub, result, cx| match result {
                        Ok(refresh) => hub.receive_authorized_transcript(
                            meeting_id,
                            receipt,
                            Ok(refresh.segments),
                            Ok(refresh.removed),
                            cx,
                        ),
                        Err(error) => {
                            hub.receive_transcript_since(meeting_id, receipt, Err(error), cx);
                        }
                    },
                );
            }
            MeetingEvent::SpeakersLinked { meeting_id } => {
                let client = self.client.clone();
                self.run(
                    async move { client.get_meeting(&meeting_id).await },
                    |hub, result, cx| {
                        if let Ok(Some(updated)) = result {
                            hub.replace_meeting(updated, cx);
                            cx.notify();
                        }
                    },
                );
            }
            MeetingEvent::StreamOpened => {
                self.refresh_people(cx);
                self.recover_meeting_stream(cx);
            }
            MeetingEvent::PersonChanged { .. }
            | MeetingEvent::PersonMerged { .. }
            | MeetingEvent::PersonObservationsAppended { .. }
            | MeetingEvent::PeopleUpdated => {
                self.refresh_people(cx);
            }
            MeetingEvent::CaptureWarning { message, .. } => {
                self.notify_notice(format!("Capture warning: {message}"), true, cx);
            }
            MeetingEvent::AudioActivityDetected { started_at_ms } => {
                let cfg = self.prefs.auto_record;
                let gate = cfg.enabled
                    && cfg.audio_driven_enabled
                    && self.live_meeting_id.is_none()
                    && self.pending_auto_record.is_none();
                if gate {
                    let now = EpochMs::now().as_f64();
                    let last = self.audio_prompt_cooldown_ms;
                    if last == 0.0 || (now - last) >= AUDIO_PROMPT_COOLDOWN_MS {
                        self.pending_auto_record = Some(synthetic_audio_event(started_at_ms, now));
                        self.prompt_countdown = PROMPT_COUNTDOWN_SECS;
                        self.prompt_paused = false;
                    }
                }
            }
            MeetingEvent::AudioLevel {
                mic_rms, sys_rms, ..
            } => {
                // Only the meter redraws for a level tick.
                self.audio_meter
                    .update(cx, |meter, cx| meter.set(mic_rms, sys_rms, cx));
                changed = !was_online;
            }
            MeetingEvent::Ended {
                meeting_id,
                ended_at_ms,
                status,
            } => {
                if let Some(m) = self.meetings.iter_mut().find(|m| m.id == meeting_id) {
                    m.ended_at_ms = Some(ended_at_ms);
                    m.status = status;
                    self.bump_search_revision();
                }
                if self.live_meeting_id.as_deref() == Some(meeting_id.as_str()) {
                    self.stop_local_focus("Local app context is off: meeting ended.", cx);
                    self.live_meeting_id = None;
                }
            }
            MeetingEvent::Summary { .. } => self.refresh_meetings(cx),
            MeetingEvent::Started { .. }
            | MeetingEvent::SpeakerLabelChanged { .. }
            | MeetingEvent::CaptureError { .. } => {}
        }
        if changed {
            cx.notify();
        }
    }

    /// Web `calendar_sync_stream.rs` reducer.
    pub fn apply_calendar_event(&mut self, event: CalendarSyncEvent, cx: &mut Context<'_, Self>) {
        match event {
            CalendarSyncEvent::SyncStarted { .. } => {
                self.google_sync.last_error = None;
            }
            CalendarSyncEvent::StreamOpened => {}
            CalendarSyncEvent::EventUpserted { event } => {
                let remote = *event;
                let existing = remote.google_event_id.as_ref().and_then(|google_id| {
                    self.google_events
                        .iter()
                        .find(|event| event.google_event_id.as_ref() == Some(google_id))
                        .cloned()
                });
                let locally_changed = existing.filter(|local| {
                    bruhs_models::should_defer_remote_event(
                        local,
                        &remote,
                        self.google_sync.last_synced_at,
                    )
                });
                if let Some(local) = locally_changed {
                    cx.emit(HubEvent::CalendarConflict {
                        local: Box::new(local),
                        remote: Box::new(remote),
                    });
                    return;
                }
                upsert_google_event(&mut self.google_events, remote);
                self.bump_search_revision();
            }
            CalendarSyncEvent::EventDeleted { google_event_id } => {
                self.google_events
                    .retain(|e| e.google_event_id.as_deref() != Some(google_event_id.as_str()));
                self.bump_search_revision();
            }
            CalendarSyncEvent::CalendarsRefreshed { calendars } => {
                self.calendars = calendars;
            }
            CalendarSyncEvent::SyncCompleted { at_ms, .. } => {
                self.google_sync.last_synced_at =
                    Some(EpochMs::saturating_from_f64(at_ms).as_millis());
                self.google_sync.last_error = None;
            }
            CalendarSyncEvent::SyncFailed { at_ms, message } => {
                self.google_sync.last_synced_at =
                    Some(EpochMs::saturating_from_f64(at_ms).as_millis());
                self.google_sync.last_error = Some(message);
            }
            CalendarSyncEvent::Disconnected => {
                self.google_sync = GoogleCalendarSyncState::default();
            }
        }
        cx.notify();
    }

    fn replace_meeting(&mut self, updated: Meeting, cx: &mut Context<'_, Self>) {
        let changed = match self.cache_meeting(&updated) {
            Ok(changed) => changed,
            Err(error) => {
                self.notify_notice(error, true, cx);
                return;
            }
        };
        if let Some(slot) = self.meetings.iter_mut().find(|m| m.id == updated.id) {
            *slot = updated;
        } else {
            self.meetings.insert(0, updated);
        }
        self.bump_search_revision();
        if changed {
            cx.emit(HubEvent::MeetingsChanged);
        }
    }

    // ── Meetings actions ────────────────────────────────────────────────

    /// Web `meetings_page::on_new_meeting` (+ Record).
    pub fn record_now(&mut self, cx: &mut Context<'_, Self>) {
        let now = EpochMs::now().as_f64();
        let Some(started_at_ms) = self.clock_reading(now, cx) else {
            return;
        };
        let body = CreateMeetingBody {
            title: format!("Meeting at {}", format_started_at(now)),
            started_at_ms,
            calendar_event_id: None,
        };
        self.create_and_capture(body, cx);
    }

    /// `ms` as a wire timestamp; a clock that reads garbage is reported.
    fn clock_reading(&mut self, ms: f64, cx: &mut Context<'_, Self>) -> Option<EpochMs> {
        match EpochMs::from_f64_floor(ms) {
            Ok(at) => Some(at),
            Err(error) => {
                self.notify_notice(
                    format!("Recording did not start: the system clock is unreadable ({error})."),
                    true,
                    cx,
                );
                None
            }
        }
    }

    /// A capture just started for `meeting_id`; show it as recording before
    /// the next status poll confirms it.
    pub(crate) fn mark_capture_recording(&mut self, meeting_id: &str, started_at_ms: f64) {
        let (Ok(meeting_id), Ok(started_at_ms)) = (
            MeetingId::try_from(meeting_id),
            EpochMs::from_f64_floor(started_at_ms),
        ) else {
            tracing::warn!("capture started for an unreadable meeting {meeting_id:?}");
            return;
        };
        let recording = Some(ActiveRecording {
            meeting_id,
            started_at_ms,
        });
        match &mut self.capture_status {
            CaptureStatus::Running(running) => running.recording = recording,
            CaptureStatus::NotRunning => {
                self.capture_status = CaptureStatus::Running(RunningCapture {
                    recording,
                    system_audio_permission: SystemAudioPermission::Unknown,
                    system_audio_capturing: false,
                });
            }
        }
    }

    fn create_and_capture(&mut self, body: CreateMeetingBody, cx: &mut Context<'_, Self>) {
        if self.auto_record_in_flight || self.live_meeting_id.is_some() {
            return;
        }
        let client = self.client.clone();
        let prefs_snapshot = self.prefs.clone();
        self.auto_record_in_flight = true;
        cx.notify();
        self.run(
            async move {
                let meeting = client
                    .create_meeting(body)
                    .await
                    .map_err(|error| error.to_string())?;
                let capture_result =
                    match capture_start_body(&prefs_snapshot, &meeting.id, meeting.started_at_ms) {
                        Ok(capture) => client.capture_start(capture).await,
                        Err(error) => Err(error),
                    };
                // Web `auto_record::create_and_record`: a meeting whose
                // capture never started is stopped, not left live.
                let meeting = match &capture_result {
                    Ok(()) => meeting,
                    Err(_) => match client
                        .stop_meeting(&meeting.id, EpochMs::now().as_f64())
                        .await
                    {
                        Ok(stopped) => stopped,
                        Err(error) => {
                            tracing::warn!(
                                "could not stop meeting {} after a failed capture start: {error}",
                                meeting.id
                            );
                            stopped_locally(meeting, EpochMs::now().as_f64())
                        }
                    },
                };
                Ok::<_, String>((meeting, capture_result))
            },
            |hub, result, cx| {
                hub.auto_record_in_flight = false;
                match result {
                    Ok((meeting, Ok(()))) => {
                        let id = meeting.id.clone();
                        let started_at_ms = meeting.started_at_ms;
                        hub.replace_meeting(meeting, cx);
                        hub.live_meeting_id = Some(id.clone());
                        hub.mark_capture_recording(&id, started_at_ms);
                        cx.emit(HubEvent::ShowMeeting { meeting_id: id });
                        cx.emit(HubEvent::EnterMeetingDock);
                    }
                    Ok((meeting, Err(e))) => {
                        // Nothing is left pretending to record.
                        hub.replace_meeting(meeting, cx);
                        hub.notify_notice(
                            format!("Recording did not start: {e}. The meeting was stopped."),
                            true,
                            cx,
                        );
                    }
                    Err(e) => hub.notify_notice(format!("create_meeting failed: {e}"), true, cx),
                }
                cx.notify();
            },
        );
    }

    /// Web `meeting_live::on_stop` / `auto_record::trigger_auto_stop`.
    pub fn stop_live(&mut self, cx: &mut Context<'_, Self>) {
        if self.auto_record_in_flight {
            return;
        }
        let Some(id) = self.live_meeting_id.clone() else {
            return;
        };
        self.stop_local_focus("Local app context is off.", cx);
        let client = self.client.clone();
        let already_stopped = self.capture_stopped_session.as_ref() == Some(&id);
        let stopped_id = id.clone();
        self.auto_record_in_flight = true;
        cx.notify();
        self.run(
            async move {
                if !already_stopped {
                    if let Err(e) = client.capture_stop().await {
                        return (
                            false,
                            Err(format!("Could not stop capture: {e}. Retry Stop.")),
                        );
                    }
                }
                (
                    true,
                    client
                        .stop_meeting(&id, EpochMs::now().as_f64())
                        .await
                        .map_err(|e| {
                            format!("Capture stopped; meeting could not be saved: {e}. Retry Stop.")
                        }),
                )
            },
            move |hub, (capture_stopped, result), cx| {
                let same_session = hub.live_meeting_id.as_ref() == Some(&stopped_id);
                if capture_stopped {
                    hub.capture_stopped_session = Some(stopped_id);
                }
                if same_session || hub.live_meeting_id.is_none() {
                    hub.auto_record_in_flight = false;
                }
                match result {
                    Ok(updated) => {
                        hub.replace_meeting(updated, cx);
                        if same_session {
                            hub.live_meeting_id = None;
                        }
                    }
                    Err(e) => hub.notify_notice(format!("stop_meeting failed: {e}"), true, cx),
                }
                cx.notify();
            },
        );
    }

    pub fn capture_action_in_flight(&self) -> bool {
        self.auto_record_in_flight
    }

    /// Web `SpeakerChip` commit: optimistic label, revert on failure.
    pub fn rename_speaker(
        &mut self,
        meeting_id: String,
        speaker_id: String,
        label: String,
        cx: &mut Context<'_, Self>,
    ) {
        let previous = self
            .meetings
            .iter()
            .find(|m| m.id == meeting_id)
            .and_then(|m| m.speakers.iter().find(|s| s.id == speaker_id))
            .map(|s| s.label.clone());
        self.set_speaker_label(&meeting_id, &speaker_id, label.clone());
        cx.notify();
        let client = self.client.clone();
        let (mid, sid) = (meeting_id.clone(), speaker_id.clone());
        self.run(
            async move { client.update_speaker_label(&mid, &sid, label).await },
            move |hub, result, cx| {
                match result {
                    Ok(updated) => hub.replace_meeting(updated, cx),
                    Err(e) => {
                        if let Some(previous) = previous {
                            hub.set_speaker_label(&meeting_id, &speaker_id, previous);
                        }
                        hub.notify_notice(format!("speaker rename failed: {e}"), true, cx);
                    }
                }
                cx.notify();
            },
        );
    }

    fn set_speaker_label(&mut self, meeting_id: &str, speaker_id: &str, label: String) {
        if let Some(m) = self.meetings.iter_mut().find(|m| m.id == meeting_id) {
            if let Some(s) = m.speakers.iter_mut().find(|s| s.id == speaker_id) {
                s.label = label;
            } else {
                m.speakers.push(bruhs_models::Speaker {
                    id: speaker_id.to_string(),
                    label,
                    person_id: None,
                    color: None,
                });
            }
        }
    }

    /// Web `SpeakerChip::do_link`.
    pub fn link_speaker(
        &mut self,
        meeting_id: String,
        speaker_id: String,
        person_id: String,
        person_name: String,
        cx: &mut Context<'_, Self>,
    ) {
        self.set_speaker_label(&meeting_id, &speaker_id, person_name);
        cx.notify();
        let client = self.client.clone();
        self.run(
            async move {
                let meeting = client
                    .link_speaker_to_person(&meeting_id, &speaker_id, &person_id)
                    .await
                    .map_err(|error| error.to_string())?;
                let people = client.list_people().await.ok();
                Ok::<_, String>((meeting, people))
            },
            |hub, result, cx| {
                match result {
                    Ok((meeting, people)) => {
                        hub.replace_meeting(meeting, cx);
                        if let Some(people) = people {
                            hub.merge_people_cache(people, cx);
                        }
                    }
                    Err(e) => {
                        hub.notify_notice(format!("link_speaker_to_person failed: {e}"), true, cx)
                    }
                }
                cx.notify();
            },
        );
    }

    /// Web "Chat with this meeting".
    pub fn open_meeting_chat(&mut self, meeting_id: String, _cx: &mut Context<'_, Self>) {
        let title = self
            .meetings
            .iter()
            .find(|m| m.id == meeting_id)
            .map(|m| m.title.clone())
            .unwrap_or_default();
        let client = self.client.clone();
        self.run(
            async move { client.create_or_get_meeting_chat(&meeting_id).await },
            move |hub, result, cx| match result {
                Ok(thread) => cx.emit(HubEvent::OpenMeetingChat {
                    meeting_title: title,
                    thread_id: thread.id.into_inner(),
                }),
                Err(e) => {
                    hub.notify_notice(format!("create_or_get_meeting_chat failed: {e}"), true, cx)
                }
            },
        );
    }

    // ── People actions ──────────────────────────────────────────────────

    pub fn rename_person(&mut self, person_id: String, name: String, _cx: &mut Context<'_, Self>) {
        let client = self.client.clone();
        self.run(
            async move { client.rename_person(&person_id, &name).await },
            |hub, result, cx| {
                match result {
                    Ok(updated) => {
                        if let Some(p) = hub.people.iter_mut().find(|p| p.id == updated.id) {
                            p.display_name = updated.display_name;
                            p.updated_at = updated.updated_at;
                        }
                        hub.people_changed(cx);
                    }
                    Err(e) => hub.notify_notice(format!("rename_person failed: {e}"), true, cx),
                }
                cx.notify();
            },
        );
    }

    pub fn set_person_tracked(
        &mut self,
        person_id: String,
        tracked: bool,
        _cx: &mut Context<'_, Self>,
    ) {
        let client = self.client.clone();
        self.run(
            async move { client.set_person_tracked(&person_id, tracked).await },
            |hub, result, cx| {
                match result {
                    Ok(updated) => {
                        if let Some(p) = hub.people.iter_mut().find(|p| p.id == updated.id) {
                            p.tracked = updated.tracked;
                        }
                        hub.people_changed(cx);
                    }
                    Err(e) => {
                        hub.notify_notice(format!("set_person_tracked failed: {e}"), true, cx)
                    }
                }
                cx.notify();
            },
        );
    }

    pub fn merge_people(
        &mut self,
        source_id: String,
        target_id: String,
        _cx: &mut Context<'_, Self>,
    ) {
        let client = self.client.clone();
        let source = source_id.clone();
        let expected_target = target_id.clone();
        self.run(
            async move { client.merge_people(&source, &target_id).await },
            move |hub, result, cx| {
                match result {
                    Ok(target) if target.id == expected_target => {
                        hub.publish_person_merge(
                            bruhs_models::PersonMergeResolution { source_id, target },
                            cx,
                        );
                    }
                    Ok(_) => {
                        hub.notify_notice("Contact merge returned an unexpected target. Local records were retained; retry contact sync to confirm the surviving identity.", true, cx);
                        hub.refresh_people(cx);
                    }
                    Err(e) => hub.notify_notice(format!("merge_people failed: {e}"), true, cx),
                }
                cx.notify();
            },
        );
    }

    // ── Google Calendar actions ─────────────────────────────────────────

    /// Web `on_connect`: start OAuth, open the consent URL, poll the state.
    pub fn connect_google(&mut self, cx: &mut Context<'_, Self>) {
        if self.google_busy {
            return;
        }
        self.google_busy = true;
        self.google_error = None;
        cx.notify();
        let client = self.client.clone();
        let jobs = self.jobs.clone();
        self.run(
            async move {
                let started = client
                    .start_google_oauth()
                    .await
                    .map_err(|error| error.to_string())?;
                let _ = jobs.send(Box::new({
                    let url = started.auth_url.clone();
                    move |_hub: &mut MeetingsHub, cx: &mut Context<'_, MeetingsHub>| {
                        cx.open_url(&url);
                    }
                }));
                for _ in 0..OAUTH_POLL_ATTEMPTS {
                    tokio::time::sleep(OAUTH_POLL_INTERVAL).await;
                    if let Ok(state) = client.fetch_google_sync_state().await {
                        let connected = state.is_connected();
                        let _ = jobs.send(Box::new(
                            move |hub: &mut MeetingsHub, cx: &mut Context<'_, MeetingsHub>| {
                                hub.google_sync = state;
                                cx.notify();
                            },
                        ));
                        if connected {
                            break;
                        }
                    }
                }
                Ok::<_, String>(())
            },
            |hub, result, cx| {
                hub.google_busy = false;
                if let Err(e) = result {
                    hub.google_error = Some(format!("Could not start OAuth: {e}"));
                }
                hub.refresh_google(cx);
                cx.notify();
            },
        );
    }

    pub fn disconnect_google(&mut self, cx: &mut Context<'_, Self>) {
        if self.google_busy {
            return;
        }
        self.google_busy = true;
        self.google_error = None;
        cx.notify();
        let client = self.client.clone();
        self.run(
            async move { client.disconnect_google().await },
            |hub, result, cx| {
                hub.google_busy = false;
                match result {
                    Ok(()) => hub.google_sync = GoogleCalendarSyncState::default(),
                    Err(e) => hub.google_error = Some(format!("Disconnect failed: {e}")),
                }
                cx.notify();
            },
        );
    }

    pub fn refresh_google_sync(&mut self, cx: &mut Context<'_, Self>) {
        if self.google_busy {
            return;
        }
        self.google_busy = true;
        self.google_error = None;
        cx.notify();
        let client = self.client.clone();
        self.run(
            async move { client.force_google_sync().await },
            |hub, result, cx| {
                hub.google_busy = false;
                if let Err(e) = result {
                    hub.google_error = Some(format!("Refresh failed: {e}"));
                }
                cx.notify();
            },
        );
    }

    /// Web `CalendarPicker::toggle`: optimistic, re-fetch on failure.
    pub fn set_calendar_sync(
        &mut self,
        calendar_id: String,
        enabled: bool,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(c) = self.calendars.iter_mut().find(|c| c.id == calendar_id) {
            c.sync_enabled = enabled;
        }
        cx.notify();
        let client = self.client.clone();
        self.run(
            async move {
                match client.patch_google_calendar(&calendar_id, enabled).await {
                    Ok(_) => None,
                    Err(_) => client.fetch_google_calendars().await.ok(),
                }
            },
            |hub, rollback, cx| {
                if let Some(calendars) = rollback {
                    hub.calendars = calendars;
                    cx.notify();
                }
            },
        );
    }

    /// Web "Remove all" imported Google events.
    pub fn clear_google_events(&mut self, cx: &mut Context<'_, Self>) {
        self.google_events.clear();
        cx.notify();
    }

    pub fn has_google_events(&self) -> bool {
        self.google_events
            .iter()
            .any(|e| matches!(e.source, EventSource::Google))
    }

    // ── Prefs ───────────────────────────────────────────────────────────

    pub fn update_prefs(
        &mut self,
        update: impl FnOnce(&mut MeetingPrefs),
        cx: &mut Context<'_, Self>,
    ) {
        let was_monitoring =
            self.prefs.auto_record.enabled && self.prefs.auto_record.audio_driven_enabled;
        update(&mut self.prefs);
        if !self.prefs.auto_record.enabled {
            self.prefs.auto_record.audio_driven_enabled = false;
        }
        let monitoring =
            self.prefs.auto_record.enabled && self.prefs.auto_record.audio_driven_enabled;
        if was_monitoring != monitoring {
            self.set_activity_monitor(monitoring, cx);
        }
        cx.emit(HubEvent::PrefsChanged);
        cx.notify();
    }

    pub fn suppressed_events(&self) -> HashSet<String> {
        self.prefs.suppressed_events.iter().cloned().collect()
    }

    /// Monotonic invalidation token for records searchable by the workspace.
    pub fn search_revision(&self) -> u64 {
        self.search_revision
    }

    fn bump_search_revision(&mut self) {
        self.search_revision = self.search_revision.wrapping_add(1);
    }

    pub fn remove_suppressed_event(&mut self, key: &str, cx: &mut Context<'_, Self>) {
        self.prefs.suppressed_events.retain(|k| k != key);
        cx.emit(HubEvent::PrefsChanged);
        cx.notify();
    }

    // ── Search ──────────────────────────────────────────────────────────

    /// Fetch segments for every meeting not cached yet (the web reads its
    /// IndexedDB copy); the page re-runs its query when they arrive.
    pub fn ensure_search_cache(&mut self, cx: &mut Context<'_, Self>) {
        let missing: Vec<_> = self
            .meetings
            .iter()
            .map(|m| &m.id)
            .filter(|id| !self.search_cached.contains(*id))
            .map(|id| (id.clone(), self.transcript_read_receipt(id)))
            .collect();
        if missing.is_empty() || self.search_loading {
            return;
        }
        self.search_loading = true;
        cx.notify();
        let client = self.client.clone();
        self.run(
            async move {
                let mut requests = tokio::task::JoinSet::new();
                let mut out = Vec::new();
                for (id, receipt) in missing {
                    if requests.len() == 8 {
                        if let Some(Ok(Some(segments))) = requests.join_next().await {
                            out.push(segments);
                        }
                    }
                    let client = client.clone();
                    requests.spawn(async move {
                        client
                            .refresh_transcript(&id)
                            .await
                            .ok()
                            .map(|refresh| (id, receipt, refresh))
                    });
                }
                while let Some(result) = requests.join_next().await {
                    if let Ok(Some(segments)) = result {
                        out.push(segments);
                    }
                }
                out
            },
            |hub, fetched, cx| {
                for (id, receipt, refresh) in fetched {
                    hub.receive_authorized_transcript(
                        id,
                        receipt,
                        Ok(refresh.segments),
                        Ok(refresh.removed),
                        cx,
                    );
                }
                hub.bump_search_revision();
                hub.search_loading = false;
                cx.notify();
            },
        );
    }

    pub fn meeting(&self, id: &str) -> Option<&Meeting> {
        self.meetings.iter().find(|m| m.id == id)
    }

    pub fn live_meeting(&self) -> Option<&Meeting> {
        let id = self.live_meeting_id.as_deref()?;
        self.meeting(id)
    }

    /// When a meeting page opens, make sure its transcript is loaded.
    pub fn ensure_segments(&mut self, meeting_id: &str, _cx: &mut Context<'_, Self>) {
        self.stream_recovery.last_requested = Some(meeting_id.to_owned());
        if !self.segments_in_flight.insert(meeting_id.to_string()) {
            return;
        }
        let client = self.client.clone();
        let id = meeting_id.to_string();
        let key = id.clone();
        let receipt = self.transcript_read_receipt(&key);
        self.run(
            async move { client.refresh_transcript(&id).await },
            move |hub, result, cx| {
                hub.segments_in_flight.remove(&key);
                match result {
                    Ok(refresh) => hub.receive_authorized_transcript(
                        key,
                        receipt,
                        Ok(refresh.segments),
                        Ok(refresh.removed),
                        cx,
                    ),
                    Err(error) => hub.receive_transcript_since(key, receipt, Err(error), cx),
                }
            },
        );
    }

    /// Delete one live chunk and keep a later capture replay from restoring it.
    pub fn delete_transcript_chunk(
        &mut self,
        meeting_id: &str,
        segment_id: &str,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(segment) = self
            .segments
            .get(meeting_id)
            .and_then(|rows| rows.iter().find(|row| row.id == segment_id))
            .cloned()
        else {
            self.show_notice("That part of the transcript is already gone.", true, cx);
            return;
        };
        if !self.forget_transcript_chunk(meeting_id, segment_id, cx) {
            self.show_notice("That part of the transcript is already gone.", true, cx);
            return;
        }
        let client = self.client.clone();
        let meeting_id = meeting_id.to_owned();
        let segment_id = segment_id.to_owned();
        self.run(
            async move {
                client
                    .remove_transcript_segment(&meeting_id, &segment_id)
                    .await
            },
            move |hub, result, cx| {
                if let Err(error) = result {
                    hub.restore_transcript_chunk(segment, cx);
                    hub.show_notice(
                        format!(
                            "Could not delete that part of the transcript. {error} It is still saved."
                        ),
                        true,
                        cx,
                    );
                }
            },
        );
    }

    // ── Auto-record ─────────────────────────────────────────────────────

    fn start_auto_record_watcher(&self, cx: &mut Context<'_, Self>) {
        cx.spawn(async move |this, cx: &mut AsyncApp| loop {
            cx.background_executor()
                .timer(Duration::from_millis(AUTO_RECORD_TICK_MS))
                .await;
            if this.update(cx, |hub, cx| hub.auto_record_tick(cx)).is_err() {
                break;
            }
        })
        .detach();
        cx.spawn(async move |this, cx: &mut AsyncApp| loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            if this.update(cx, |hub, cx| hub.prompt_tick(cx)).is_err() {
                break;
            }
        })
        .detach();
    }

    /// All events the watcher sees: Google events plus local ones handed in
    /// by the workspace (local events have no attendees, as on the web).
    pub fn auto_record_tick(&mut self, cx: &mut Context<'_, Self>) {
        let suppressed = self.suppressed_events();
        let decision = decide(&TickInput {
            now_ms: EpochMs::now().as_f64(),
            settings: &self.prefs.auto_record,
            capture_running: self.capture_status.is_running(),
            events: &self.google_events,
            meetings: &self.meetings,
            live_meeting_id: self.live_meeting_id.as_deref(),
            pending: self.pending_auto_record.as_ref(),
            suppressed: &suppressed,
            session_skipped: &self.session_skipped,
            in_flight: self.auto_record_in_flight,
        });
        match decision {
            AutoRecordDecision::Idle => {}
            AutoRecordDecision::AutoStop { .. } => self.stop_live(cx),
            AutoRecordDecision::AutoStart(event) => self.start_recording_for_event(event, cx),
            AutoRecordDecision::Prompt(event) => {
                self.pending_auto_record = Some(event);
                self.prompt_countdown = PROMPT_COUNTDOWN_SECS;
                self.prompt_paused = false;
                cx.notify();
            }
        }
    }

    /// Web `auto_record_prompt` countdown: one tick per second while a
    /// prompt is pending and not paused; at zero it records.
    fn prompt_tick(&mut self, cx: &mut Context<'_, Self>) {
        if self.live_meeting_id.is_some() {
            // The live pill's elapsed label ticks with this.
            cx.notify();
        }
        if self.pending_auto_record.is_none() || self.prompt_paused {
            return;
        }
        self.prompt_countdown -= 1;
        if self.prompt_countdown <= 0 {
            self.prompt_record(cx);
        } else {
            cx.notify();
        }
    }

    fn take_pending(&mut self) -> Option<CalendarEvent> {
        let event = self.pending_auto_record.take()?;
        self.session_skipped.insert(event_key(&event).to_string());
        self.audio_prompt_cooldown_ms = EpochMs::now().as_f64();
        self.prompt_countdown = PROMPT_COUNTDOWN_SECS;
        self.prompt_paused = false;
        Some(event)
    }

    pub fn prompt_record(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(event) = self.take_pending() {
            self.start_recording_for_event(event, cx);
        }
        cx.notify();
    }

    pub fn prompt_skip_once(&mut self, cx: &mut Context<'_, Self>) {
        self.take_pending();
        cx.notify();
    }

    pub fn prompt_always_skip(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(event) = self.pending_auto_record.take() {
            let key = event_key(&event).to_string();
            if !self.prefs.suppressed_events.contains(&key) {
                self.prefs.suppressed_events.push(key);
            }
            self.audio_prompt_cooldown_ms = EpochMs::now().as_f64();
            self.prompt_countdown = PROMPT_COUNTDOWN_SECS;
            self.prompt_paused = false;
            cx.emit(HubEvent::PrefsChanged);
        }
        cx.notify();
    }

    pub fn prompt_toggle_pause(&mut self, cx: &mut Context<'_, Self>) {
        self.prompt_paused = !self.prompt_paused;
        cx.notify();
    }

    /// Web `auto_record::start_recording_for_event`.
    pub fn start_recording_for_event(&mut self, event: CalendarEvent, cx: &mut Context<'_, Self>) {
        let link_key = event_key(&event).to_string();
        let Some(started_at_ms) = self.clock_reading(EpochMs::now().as_f64(), cx) else {
            return;
        };
        let body = CreateMeetingBody {
            title: event.title,
            started_at_ms,
            calendar_event_id: Some(link_key),
        };
        self.create_and_capture(body, cx);
    }

    // ── Voice enrollment ────────────────────────────────────────────────

    pub fn open_enroll(&mut self, person_id: String, cx: &mut Context<'_, Self>) {
        self.enroll = Some((person_id, EnrollState::Idle));
        cx.notify();
    }

    pub fn close_enroll(&mut self, cx: &mut Context<'_, Self>) {
        if let Some((_, state)) = &self.enroll {
            if matches!(state, EnrollState::Recording | EnrollState::Uploading) {
                return;
            }
        }
        self.enroll = None;
        cx.notify();
    }

    pub fn enroll_reset(&mut self, cx: &mut Context<'_, Self>) {
        if let Some((_, state)) = &mut self.enroll {
            *state = EnrollState::Idle;
        }
        cx.notify();
    }

    /// Web `start_enrollment`: 3-2-1, record 5 s, upload.
    pub fn start_enrollment(&mut self, cx: &mut Context<'_, Self>) {
        let Some((person_id, state)) = self.enroll.clone() else {
            return;
        };
        if state != EnrollState::Idle {
            return;
        }
        self.enroll = Some((person_id.clone(), EnrollState::Countdown(3)));
        cx.notify();
        let client = self.client.clone();
        let jobs = self.jobs.clone();
        let pid = person_id.clone();
        self.run(
            async move {
                for remaining in (1..=3i32).rev() {
                    let _ = jobs.send(Box::new(
                        move |hub: &mut MeetingsHub, cx: &mut Context<'_, MeetingsHub>| {
                            if let Some((_, state)) = &mut hub.enroll {
                                *state = EnrollState::Countdown(remaining);
                            }
                            cx.notify();
                        },
                    ));
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                let _ = jobs.send(Box::new(
                    |hub: &mut MeetingsHub, cx: &mut Context<'_, MeetingsHub>| {
                        if let Some((_, state)) = &mut hub.enroll {
                            *state = EnrollState::Recording;
                        }
                        cx.notify();
                    },
                ));
                let wav = tokio::task::spawn_blocking(|| {
                    crate::voice_capture::capture_voice_sample_wav(Duration::from_secs(5))
                })
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| format!("Microphone error: {e}"))?;
                let _ = jobs.send(Box::new(
                    |hub: &mut MeetingsHub, cx: &mut Context<'_, MeetingsHub>| {
                        if let Some((_, state)) = &mut hub.enroll {
                            *state = EnrollState::Uploading;
                        }
                        cx.notify();
                    },
                ));
                client
                    .enroll_voice(&pid, wav)
                    .await
                    .map_err(|e| format!("Upload failed: {e}"))
            },
            move |hub, result, cx| {
                match result {
                    Ok(updated) => {
                        if let Some(p) = hub.people.iter_mut().find(|p| p.id == updated.id) {
                            *p = updated;
                        }
                        hub.people_changed(cx);
                        hub.enroll = Some((person_id, EnrollState::Success));
                        cx.spawn(async move |this, cx: &mut AsyncApp| {
                            cx.background_executor().timer(Duration::from_secs(2)).await;
                            let _ = this.update(cx, |hub, cx| {
                                if matches!(hub.enroll, Some((_, EnrollState::Success))) {
                                    hub.enroll = None;
                                    cx.notify();
                                }
                            });
                        })
                        .detach();
                    }
                    Err(message) => hub.enroll = Some((person_id, EnrollState::Error(message))),
                }
                cx.notify();
            },
        );
    }
}

/// Web `meetings_stream.rs` `AudioActivityDetected` synthetic event.
fn synthetic_audio_event(started_at_ms: f64, now: f64) -> CalendarEvent {
    CalendarEvent {
        id: format!("audio-{started_at_ms}"),
        title: "Sustained audio activity".into(),
        start_at_ms: now,
        end_at_ms: now + 30.0 * 60.0 * 1000.0,
        all_day: false,
        color: None,
        notes: None,
        created_at: EpochMs::saturating_from_f64(now).as_millis(),
        updated_at: EpochMs::saturating_from_f64(now).as_millis(),
        source: EventSource::Local,
        google_event_id: None,
        calendar_id: None,
        attendees: vec![],
        organizer: None,
        location: None,
        conferencing: None,
        recurring_event_id: None,
        html_link: None,
    }
}

/// Web `calendar_sync_stream` upsert: replace by `google_event_id`.
/// Web `meetings_page::on_new_meeting` capture body: the whisper models and
/// diarization prefs ride along with every capture start.
/// `meeting` stopped at `now_ms`, as `POST /api/meetings/{id}/stop` leaves
/// it; used when the server cannot be told.
fn stopped_locally(mut meeting: Meeting, now_ms: f64) -> Meeting {
    meeting.status = MeetingStatus::Ended;
    meeting.ended_at_ms.get_or_insert(now_ms);
    meeting
}

/// The daemon request recording `meeting_id` with the user's settings.
///
/// # Errors
///
/// [`ServerError::Invalid`] when the server's meeting has an id or start
/// time the daemon would refuse.
pub fn capture_start_body(
    prefs: &MeetingPrefs,
    meeting_id: &str,
    started_at_ms: f64,
) -> Result<CaptureStartBody, ServerError> {
    let d = prefs.diarize;
    Ok(CaptureStartBody {
        meeting_id: MeetingId::try_from(meeting_id)
            .map_err(|_| ServerError::Invalid("The meeting has an invalid id."))?,
        started_at_ms: EpochMs::from_f64_floor(started_at_ms)
            .map_err(|_| ServerError::Invalid("The meeting has an invalid start time."))?,
        model: Some(prefs.whisper_model),
        live_model: Some(prefs.live_whisper_model),
        subspeaker_enabled: Some(d.subspeaker_enabled),
        cross_meeting_linking_auto: Some(d.cross_meeting_linking_auto),
        cross_meeting_linking_threshold: Some(d.cross_meeting_linking_threshold),
        max_speakers: Some(d.max_speakers),
        video_ocr_enabled: Some(d.video_ocr_enabled),
    })
}

pub fn upsert_google_event(list: &mut Vec<CalendarEvent>, event: CalendarEvent) {
    let existing = event.google_event_id.as_ref().and_then(|gid| {
        list.iter()
            .position(|e| e.google_event_id.as_ref() == Some(gid))
    });
    match existing {
        Some(idx) => list[idx] = event,
        None => list.push(event),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn a_server_that_answers_with_an_error_is_not_offline(cx: &mut gpui::TestAppContext) {
        use gpui::AppContext as _;
        let runtime = Arc::new(Runtime::new().unwrap());
        let hub = cx.new(|cx| {
            MeetingsHub::new(
                ServerClient::new("http://127.0.0.1:9", "http://127.0.0.1:9").unwrap(),
                runtime,
                MeetingPrefs::default(),
                cx,
            )
        });
        hub.update(cx, |hub, _| {
            hub.server_online = false;
            hub.note_request_failure(&ServerError::Status {
                request: "GET /api/meetings".into(),
                code: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
                body: String::new(),
            });
            assert!(hub.server_online);
        });
    }

    #[test]
    fn a_meeting_whose_capture_never_started_is_stopped() {
        let meeting = Meeting::new("Standup", 1_000.0);
        let ended = stopped_locally(meeting, 2_000.0);
        assert_eq!(ended.status, MeetingStatus::Ended);
        assert_eq!(ended.ended_at_ms, Some(2_000.0));
    }
    use bruhs_models::{AttendeeResponse, EventAttendee};

    fn make_event(id: &str, start: f64, end: f64, attendees: usize) -> CalendarEvent {
        CalendarEvent {
            id: id.into(),
            title: "Sync".into(),
            start_at_ms: start,
            end_at_ms: end,
            all_day: false,
            color: None,
            notes: None,
            created_at: 0,
            updated_at: 0,
            source: EventSource::Google,
            google_event_id: Some(id.into()),
            calendar_id: None,
            attendees: (0..attendees)
                .map(|i| EventAttendee {
                    email: format!("a{i}@x"),
                    display_name: None,
                    response_status: AttendeeResponse::NeedsAction,
                    organizer: false,
                    self_: false,
                    optional: false,
                })
                .collect(),
            organizer: None,
            location: None,
            conferencing: None,
            recurring_event_id: None,
            html_link: None,
        }
    }

    fn settings() -> AutoRecordSettings {
        AutoRecordSettings {
            enabled: true,
            min_attendees: 2,
            stop_delay_secs: 60,
            prompt_before_starting: true,
            audio_driven_enabled: false,
        }
    }

    #[test]
    fn decide_prompts_for_a_live_event_and_idles_when_gated() {
        let events = [make_event("e-1", 100.0, 1000.0, 3)];
        let cfg = settings();
        let none = HashSet::new();
        let base = TickInput {
            now_ms: 500.0,
            settings: &cfg,
            capture_running: true,
            events: &events,
            meetings: &[],
            live_meeting_id: None,
            pending: None,
            suppressed: &none,
            session_skipped: &none,
            in_flight: false,
        };
        assert!(matches!(decide(&base), AutoRecordDecision::Prompt(_)));
        let no_daemon = TickInput {
            capture_running: false,
            ..base
        };
        assert_eq!(decide(&no_daemon), AutoRecordDecision::Idle);
        let mut silent = settings();
        silent.prompt_before_starting = false;
        let auto = TickInput {
            settings: &silent,
            ..base
        };
        assert!(matches!(decide(&auto), AutoRecordDecision::AutoStart(_)));
    }

    #[test]
    fn decide_auto_stops_after_the_event_plus_delay() {
        let events = [make_event("g-1", 0.0, 1000.0, 2)];
        let mut meeting = Meeting::new("Sync", 0.0);
        meeting.calendar_event_id = Some("g-1".into());
        let meetings = [meeting];
        let cfg = settings();
        let none = HashSet::new();
        let input = TickInput {
            now_ms: 61_000.0,
            settings: &cfg,
            capture_running: true,
            events: &events,
            meetings: &meetings,
            live_meeting_id: Some(meetings[0].id.as_str()),
            pending: None,
            suppressed: &none,
            session_skipped: &none,
            in_flight: false,
        };
        assert!(matches!(
            decide(&input),
            AutoRecordDecision::AutoStop { .. }
        ));
        let early = TickInput {
            now_ms: 60_999.0,
            ..input
        };
        assert_eq!(decide(&early), AutoRecordDecision::Idle);
    }

    #[test]
    fn suppressed_and_recorded_events_are_not_candidates() {
        let event = make_event("g-1", 100.0, 1000.0, 3);
        let cfg = settings();
        let mut suppressed = HashSet::new();
        suppressed.insert("g-1".to_string());
        assert!(!is_auto_start_candidate(
            &event,
            500.0,
            &cfg,
            &[],
            &suppressed,
            &HashSet::new()
        ));
        let mut prior = Meeting::new("Sync", 0.0);
        prior.calendar_event_id = Some("g-1".into());
        assert!(!is_auto_start_candidate(
            &event,
            500.0,
            &cfg,
            &[prior],
            &HashSet::new(),
            &HashSet::new()
        ));
        assert!(!is_auto_start_candidate(
            &make_event("g-2", 100.0, 1000.0, 1),
            500.0,
            &cfg,
            &[],
            &HashSet::new(),
            &HashSet::new()
        ));
    }

    #[test]
    fn snippets_centre_the_match_and_stay_on_char_boundaries() {
        let (snippet, s, e) = build_snippet("the quick brown fox jumps", "brown", 4);
        assert_eq!(snippet, "ick brown fox");
        assert_eq!(&snippet[s..e], "brown");
        let (snippet, s, e) = build_snippet("héllo wörld", "wörld", 2);
        assert_eq!(&snippet[s..e], "wörld");
    }

    fn seg(meeting: &str, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            id: format!("{meeting}-{}", text.len()),
            meeting_id: meeting.into(),
            speaker_id: Some("me".into()),
            start_ms: 0,
            end_ms: 1,
            text: text.into(),
            confidence: 1.0,
            partial: false,
            speaker_confidence: None,
        }
    }

    #[test]
    fn search_is_case_insensitive_grouped_and_capped() {
        let mut map = BTreeMap::new();
        map.insert(
            "m1".to_string(),
            vec![seg("m1", "Ship the Flag"), seg("m1", "no")],
        );
        map.insert("m2".to_string(), vec![seg("m2", "flag day")]);
        let hits = search_segments(&map, "FLAG", 50);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].meeting_id, "m1");
        assert_eq!(search_segments(&map, "flag", 1).len(), 1);
        assert!(search_segments(&map, "  ", 50).is_empty());
    }

    #[test]
    fn grouping_and_formatting_match_web() {
        let mut a = seg("m", "one");
        a.speaker_id = Some("me".into());
        let mut b = seg("m", "two");
        b.speaker_id = Some("me".into());
        let mut c = seg("m", "three");
        c.speaker_id = Some("them".into());
        let groups = group_by_speaker(&[a, b, c]);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].1.len(), 2);
        assert_eq!(format_duration(3_723_000.0), "1h 2m");
        assert_eq!(format_duration(65_000.0), "1m 5s");
        assert_eq!(format_elapsed(3_723_000.0), "01:02:03");
        assert_eq!(format_elapsed(65_000.0), "01:05");
        assert_eq!(
            summary_preview("First sentence. Second.", 80),
            "First sentence."
        );
        assert_eq!(summary_preview("A very long first sentence", 6), "A very…");
    }

    #[test]
    fn google_event_upsert_replaces_by_google_id() {
        let mut list = vec![make_event("g-1", 0.0, 1.0, 0)];
        let mut updated = make_event("g-1", 5.0, 6.0, 0);
        updated.title = "New".into();
        upsert_google_event(&mut list, updated);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title, "New");
        upsert_google_event(&mut list, make_event("g-2", 0.0, 1.0, 0));
        assert_eq!(list.len(), 2);
    }
}
