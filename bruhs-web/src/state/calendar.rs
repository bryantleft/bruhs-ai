//! Calendar events, Google sync and scheduling.

use super::*;

#[derive(Copy, Clone)]
pub struct CalendarModeSignal(pub ReadSignal<CalendarMode>);
#[derive(Copy, Clone)]
pub struct SetCalendarModeSignal(pub WriteSignal<CalendarMode>);

#[derive(Copy, Clone)]
pub struct InboxConversationsSignal(pub ReadSignal<Vec<InboxConversation>>);
#[derive(Copy, Clone)]
pub struct SetInboxConversationsSignal(pub WriteSignal<Vec<InboxConversation>>);
#[derive(Copy, Clone)]
pub struct RelationshipsSignal(pub ReadSignal<Vec<RelationshipProfile>>);
#[derive(Copy, Clone)]
pub struct SetRelationshipsSignal(pub WriteSignal<Vec<RelationshipProfile>>);
#[derive(Copy, Clone)]
pub struct SchedulingPreferencesSignal(pub RwSignal<SchedulingPreferences>);
#[derive(Copy, Clone)]
pub struct LifeContextSignal(pub DurableMetadata<LifeContext>);

// ── Calendar signal newtypes ─────────────────────────────────────────────────
// All the calendar surface's reactive state, namespaced with newtype wrappers
// so context lookups don't collide with chat/tasks signals of the same shape.
#[derive(Copy, Clone)]
pub struct CalendarEventsSignal(pub ReadSignal<Vec<CalendarEvent>>);
#[derive(Copy, Clone)]
pub struct SetCalendarEventsSignal(pub WriteSignal<Vec<CalendarEvent>>);

// Phase 0 scaffolding: CalendarsSignal and GoogleSyncState signals are wired
// in Phase 1+ (Google OAuth integration). The fields are intentionally
// unused until then.
#[derive(Copy, Clone)]
pub struct CalendarsSignal(pub ReadSignal<Vec<Calendar>>);
#[derive(Copy, Clone)]
pub struct SetCalendarsSignal(pub WriteSignal<Vec<Calendar>>);

#[derive(Copy, Clone)]
pub struct GoogleSyncStateSignal(pub ReadSignal<GoogleCalendarSyncState>);
#[derive(Copy, Clone)]
pub struct SetGoogleSyncStateSignal(pub WriteSignal<GoogleCalendarSyncState>);

/// Any timestamp (ms) inside the month the user is currently viewing.
/// Navigation buttons mutate this; the month grid derives its layout
/// from whatever month this timestamp falls in.
#[derive(Copy, Clone)]
pub struct CalendarAnchor(pub ReadSignal<f64>);
#[derive(Copy, Clone)]
pub struct SetCalendarAnchor(pub WriteSignal<f64>);

#[derive(Copy, Clone)]
pub struct EventCreateOpen(pub ReadSignal<bool>);
#[derive(Copy, Clone)]
pub struct SetEventCreateOpen(pub WriteSignal<bool>);

/// Exact requested event start. Month cells choose 09:00; time-grid clicks
/// choose their displayed hour, including midnight. None uses the next half-hour.
#[derive(Copy, Clone)]
pub struct EventCreateStart(pub ReadSignal<Option<f64>>);
#[derive(Copy, Clone)]
pub struct SetEventCreateStart(pub WriteSignal<Option<f64>>);

/// When `Some`, the editor opens in EDIT mode (prefilled from this event)
/// instead of CREATE mode. Cleared when the dialog closes.
#[derive(Copy, Clone)]
pub struct SelectedEventId(pub ReadSignal<Option<String>>);
#[derive(Copy, Clone)]
pub struct SetSelectedEventId(pub WriteSignal<Option<String>>);

/// Calendar events, calendars, Google sync, the visible month and mode, the
/// event dialog, and scheduling preferences.
pub(super) fn provide() {
    let (calendar_events, set_calendar_events) = signal(Vec::<CalendarEvent>::new());
    let (calendars, set_calendars) = signal(Vec::<Calendar>::new());
    let (google_sync_state, set_google_sync_state) = signal(GoogleCalendarSyncState::default());
    // Any timestamp inside the month on screen.
    let (calendar_anchor, set_calendar_anchor) = signal(js_sys::Date::now());
    let (event_create_open, set_event_create_open) = signal(false);
    // Exact start timestamp chosen by a month cell or a time-grid hour.
    let (event_create_start, set_event_create_start) = signal::<Option<f64>>(None);
    // When set, the editor dialog opens in EDIT mode and prefills from this
    // event id. None = the dialog opens in CREATE mode.
    let (selected_event_id, set_selected_event_id) = signal::<Option<String>>(None);
    // Which calendar grain to render — month grid by default, week / day
    // for narrower-time-horizon views.
    let (calendar_mode, set_calendar_mode) = signal(CalendarMode::Month);
    let scheduling_preferences = RwSignal::new(
        load_json_pref::<SchedulingPreferences>("bruhs.scheduling").unwrap_or_default(),
    );
    Effect::new(move |_| {
        save_json_pref("bruhs.scheduling", &scheduling_preferences.get());
    });

    provide_context(CalendarEventsSignal(calendar_events));
    provide_context(SetCalendarEventsSignal(set_calendar_events));
    provide_context(CalendarsSignal(calendars));
    provide_context(SetCalendarsSignal(set_calendars));
    provide_context(GoogleSyncStateSignal(google_sync_state));
    provide_context(SetGoogleSyncStateSignal(set_google_sync_state));
    provide_context(CalendarAnchor(calendar_anchor));
    provide_context(SetCalendarAnchor(set_calendar_anchor));
    provide_context(EventCreateOpen(event_create_open));
    provide_context(SetEventCreateOpen(set_event_create_open));
    provide_context(EventCreateStart(event_create_start));
    provide_context(SetEventCreateStart(set_event_create_start));
    provide_context(SelectedEventId(selected_event_id));
    provide_context(SetSelectedEventId(set_selected_event_id));
    provide_context(CalendarModeSignal(calendar_mode));
    provide_context(SetCalendarModeSignal(set_calendar_mode));
    provide_context(SchedulingPreferencesSignal(scheduling_preferences));
}
