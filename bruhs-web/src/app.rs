use crate::components::shell::Shell;
use crate::state::{
    provide_app_state, ActiveView, AppView, AutoRecordSettingsSignal, CalendarEventsSignal,
    GoogleSyncStateSignal, PersonalitySettingsSignal, SchedulingPreferencesSignal, SetActiveView,
    SetCalendarEventsSignal, SetCalendarsSignal, SetCaptureStatusSignal, SetCodexAuthSignal,
    SetGoogleSyncStateSignal,
};
use crate::storage;
use leptos::prelude::*;

/// Parse a routed hash into an AppView.
pub(crate) fn parse_hash(hash: &str) -> Option<AppView> {
    let h = hash.trim_start_matches('#');
    if let Some(rest) = h.strip_prefix("/meeting-notes/") {
        let note_id: String = js_sys::decode_uri_component(rest).ok()?.into();
        if !note_id.trim().is_empty() && note_id.len() <= 128 {
            return Some(AppView::MeetingNoteDetail { note_id });
        }
        return None;
    }
    if let Some(rest) = h.strip_prefix("/meetings/") {
        if !rest.is_empty() {
            return Some(AppView::MeetingDetail {
                meeting_id: rest.to_string(),
            });
        }
    }
    if let Some(rest) = h.strip_prefix("/knowledge/documents/") {
        if !rest.is_empty() {
            return Some(AppView::DocumentDetail {
                document_id: rest.to_string(),
            });
        }
    }
    if let Some(rest) = h.strip_prefix("/knowledge/clips/") {
        if !rest.is_empty() {
            return Some(AppView::ClipDetail {
                clip_id: rest.to_string(),
            });
        }
    }
    if let Some(rest) = h.strip_prefix("/inbox/") {
        if !rest.is_empty() {
            return Some(AppView::ConversationDetail {
                conversation_id: rest.to_string(),
            });
        }
    }
    if let Some(rest) = h.strip_prefix("/book/") {
        if !rest.is_empty() {
            return Some(AppView::Booking {
                slug: rest.to_string(),
            });
        }
    }
    match h {
        "/today" => Some(AppView::Today),
        "/inbox" => Some(AppView::Inbox),
        "/knowledge" => Some(AppView::Knowledge),
        "/chat" => Some(AppView::Chat),
        "/tasks" => Some(AppView::Tasks),
        "/calendar" => Some(AppView::Calendar),
        "/life" => Some(AppView::Life),
        "/automations" => Some(AppView::Automations),
        "/meetings" => Some(AppView::Meetings),
        "/people" => Some(AppView::People),
        "/search" => Some(AppView::Search),
        "/terminal" => Some(AppView::Terminal),
        "/settings" => Some(AppView::Settings),
        _ => None,
    }
}

#[component]
pub fn App() -> impl IntoView {
    provide_app_state();
    crate::updates::Updates::provide();
    crate::task_saves::TaskSaves::install();
    crate::thread_history::ThreadHistory::install();
    crate::relationship_store::RelationshipStore::install();
    crate::navigation_guard::install();
    crate::workflow_executor::WorkflowExecutor::install();
    crate::thread_catalog::ThreadCatalog::install();
    crate::thread_link::ThreadLink::install();
    crate::meeting_actions::MeetingActions::install();

    // ── URL hash routing ─────────────────────────────────────────────────────
    // On mount: read window.location.hash and set the initial AppView.
    // On AppView changes: write the hash.
    // On hashchange: the Shell's listener updates AppView.
    {
        let ActiveView(active_view) = expect_context::<ActiveView>();
        let SetActiveView(set_active_view) = expect_context::<SetActiveView>();

        // Read initial hash on mount.
        if let Some(window) = web_sys::window() {
            let hash = window.location().hash().unwrap_or_default();
            if let Some(view) = parse_hash(&hash) {
                set_active_view.set(view);
            }
        }

        // Write hash whenever AppView changes.
        Effect::new(move |_| {
            let view = active_view.get();
            let hash = match &view {
                AppView::Today => "#/today".to_string(),
                AppView::Inbox => "#/inbox".to_string(),
                AppView::Knowledge => "#/knowledge".to_string(),
                AppView::Chat => "#/chat".to_string(),
                AppView::Tasks => "#/tasks".to_string(),
                AppView::Calendar => "#/calendar".to_string(),
                AppView::Life => "#/life".to_string(),
                AppView::Automations => "#/automations".to_string(),
                AppView::Meetings => "#/meetings".to_string(),
                AppView::MeetingDetail { meeting_id } => format!("#/meetings/{meeting_id}"),
                AppView::MeetingNoteDetail { note_id } => {
                    let encoded: String = js_sys::encode_uri_component(note_id).into();
                    format!("#/meeting-notes/{encoded}")
                }
                AppView::DocumentDetail { document_id } => {
                    format!("#/knowledge/documents/{document_id}")
                }
                AppView::ClipDetail { clip_id } => format!("#/knowledge/clips/{clip_id}"),
                AppView::ConversationDetail { conversation_id } => {
                    format!("#/inbox/{conversation_id}")
                }
                AppView::Booking { slug } => format!("#/book/{slug}"),
                AppView::People => "#/people".to_string(),
                AppView::Search => "#/search".to_string(),
                AppView::Terminal => "#/terminal".to_string(),
                AppView::Settings => "#/settings".to_string(),
            };
            if let Some(window) = web_sys::window() {
                let _ = window.location().set_hash(&hash);
            }
        });

        // Back/forward and pasted links arrive as `hashchange`; the Shell's
        // one listener routes them (views and `#thread/<id>` links).
    }

    // Domain stores own their loading and recovery state.
    let SetCalendarsSignal(set_calendars) = expect_context::<SetCalendarsSignal>();
    let CalendarEventsSignal(events_read) = expect_context::<CalendarEventsSignal>();
    let SetCalendarEventsSignal(set_calendar_events_for_sync) =
        expect_context::<SetCalendarEventsSignal>();
    let GoogleSyncStateSignal(sync_state_read) = expect_context::<GoogleSyncStateSignal>();
    let SetGoogleSyncStateSignal(set_sync_state_for_stream) =
        expect_context::<SetGoogleSyncStateSignal>();
    let SetCalendarsSignal(set_calendars_for_stream) = expect_context::<SetCalendarsSignal>();
    let SetCaptureStatusSignal(set_capture_status) = expect_context::<SetCaptureStatusSignal>();
    let capture_banners = use_context::<crate::components::banner::BannerStackSignal>();
    let SchedulingPreferencesSignal(scheduling_preferences) =
        expect_context::<SchedulingPreferencesSignal>();

    // One live stream from the server carries background tasks, calendar
    // sync and meeting events; each subscriber handles its own frames.
    let mut live_events = crate::live_events::LiveEvents::default();

    // Background tasks, so the tasks panel reflects the server's registry.
    let set_tasks = expect_context::<crate::state::SetBackgroundTasksSignal>().0;
    crate::tasks_stream::subscribe_tasks_stream(&mut live_events, set_tasks);

    // Calendar sync, so Google Calendar events follow the server's feed.
    crate::calendar_sync_stream::subscribe(
        &mut live_events,
        events_read,
        set_calendar_events_for_sync,
        sync_state_read,
        set_sync_state_for_stream,
        set_calendars_for_stream,
        scheduling_preferences,
    );

    let PersonalitySettingsSignal(personality_for_stream) =
        expect_context::<PersonalitySettingsSignal>();

    // Meeting events, so MeetingDetail can render live transcript segments as
    // bruhs-capture emits them, trigger summarization when transcription
    // completes, and refresh the people list when PersonObservationsAppended
    // fires.
    crate::meetings_stream::subscribe(
        &mut live_events,
        expect_context::<crate::workflow_executor::WorkflowExecutor>(),
        personality_for_stream,
    );
    live_events.connect(use_context::<crate::components::banner::BannerStackSignal>());

    // Wire the calendar-driven auto-record watcher. Reads settings +
    // live calendar events + capture status each tick to decide whether
    // to start or stop a recording.
    let AutoRecordSettingsSignal(auto_record_settings) =
        expect_context::<AutoRecordSettingsSignal>();
    let crate::state::SetAutoRecordSettingsSignal(set_auto_record_settings) =
        expect_context::<crate::state::SetAutoRecordSettingsSignal>();
    crate::activity_monitor::start(auto_record_settings, set_auto_record_settings);
    crate::auto_record::start_watcher(
        crate::auto_record::WatcherInputs::from_context(),
        crate::auto_record::RecordingTargets::from_context(),
    );

    expect_context::<crate::thread_catalog::ThreadCatalog>().load();
    let workflow_executor = expect_context::<crate::workflow_executor::WorkflowExecutor>();

    // Fetch the current CLI auth status for both providers once on startup.
    let set_auth = expect_context::<crate::state::SetClaudeAuthSignal>().0;
    wasm_bindgen_futures::spawn_local(async move {
        let status = crate::auth::fetch_auth_status_for("claude").await;
        set_auth.set(status);
    });
    let SetCodexAuthSignal(set_codex_auth) = expect_context::<SetCodexAuthSignal>();
    wasm_bindgen_futures::spawn_local(async move {
        let status = crate::auth::fetch_auth_status_for("codex").await;
        set_codex_auth.set(status);
    });

    wasm_bindgen_futures::spawn_local(async move {
        workflow_executor.hydrate_people().await;
        match storage::load_calendars().await {
            Ok(cals) => {
                if !cals.is_empty() {
                    set_calendars.set(cals);
                }
            }
            Err(e) => log::warn!("idb hydrate calendars failed: {e:?}"),
        }
        match crate::api::fetch_capture_status().await {
            Ok(s) => set_capture_status.set(s),
            Err(e) => {
                log::warn!("fetch_capture_status: {e}");
                // 502: the daemon answered but the server could not use it
                // (it refused the server's token). Say why, so the page's
                // "not running" is not the only word on it.
                if let (true, Some(banners)) = (e.is_status(502), capture_banners) {
                    banners.push(
                        crate::components::banner::BannerKind::Error,
                        format!("Capture status unavailable: {e}"),
                        Some(12_000),
                    );
                }
            }
        }
        workflow_executor.refresh_people().await;
    });

    view! {
        <Shell />
        <crate::components::capture_dialog::CaptureDialog />
        <crate::components::auto_record_prompt::AutoRecordPromptModal />
    }
}
