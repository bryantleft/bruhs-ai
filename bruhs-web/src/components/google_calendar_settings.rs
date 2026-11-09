//! Connect / Disconnect Google Calendar — lives inside the SettingsDialog.
//! Phase 1 has no event fetching yet; this is the simplest possible UI:
//! - "Connect Google Calendar" button when disconnected
//! - "Connected as <email>" + "Disconnect" button when connected
//! - Error banner if the last action failed

use leptos::prelude::*;

use crate::api;
use crate::components::calendar_picker::CalendarPicker;
use crate::components::settings_page::{SettingsRow, SettingsRowBody};
use crate::state::{
    CalendarEvent, CalendarEventsSignal, EventSource, GoogleCalendarSyncState,
    GoogleSyncStateSignal, SetCalendarEventsSignal, SetGoogleSyncStateSignal,
};
use crate::ui::{Button, ButtonSize, ButtonVariant};

/// "just now", "12s ago", "5m ago", … for a sync time in epoch ms.
fn synced_ago(ts_ms: i64) -> String {
    let at = bruhs_models::EpochMs::new(ts_ms).unwrap_or(bruhs_models::EpochMs::UNIX_EPOCH);
    bruhs_models::time::format_relative_ms(
        at,
        crate::state::now_epoch_ms(),
        bruhs_models::time::RelativeTimeStyle::Ago,
    )
}

#[component]
pub fn GoogleCalendarSettings() -> impl IntoView {
    let GoogleSyncStateSignal(sync_state) = expect_context::<GoogleSyncStateSignal>();
    let SetGoogleSyncStateSignal(set_sync_state) = expect_context::<SetGoogleSyncStateSignal>();
    let CalendarEventsSignal(events) = expect_context::<CalendarEventsSignal>();
    let SetCalendarEventsSignal(set_events) = expect_context::<SetCalendarEventsSignal>();

    let (error, set_error) = signal::<Option<String>>(None);
    let (busy, set_busy) = signal(false);

    // Fetch the current sync state once on mount.
    Effect::new(move |_| {
        wasm_bindgen_futures::spawn_local(async move {
            match api::fetch_google_sync_state().await {
                Ok(s) => set_sync_state.set(s),
                Err(e) => log::warn!("fetch_google_sync_state: {e}"),
            }
        });
    });

    let on_connect = move |_: web_sys::MouseEvent| {
        if busy.get_untracked() {
            return;
        }
        set_busy.set(true);
        set_error.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            match api::start_google_oauth().await {
                Ok(resp) => {
                    // Open the consent URL in a new tab.
                    if let Some(win) = web_sys::window() {
                        let _ = win.open_with_url_and_target(&resp.auth_url, "_blank");
                    }
                    // Poll the state endpoint every 2s for up to ~2 minutes
                    // so we reflect connection promptly without WebSockets.
                    let attempts = 60u32;
                    for _ in 0..attempts {
                        gloo_timers::future::TimeoutFuture::new(2_000).await;
                        if let Ok(s) = api::fetch_google_sync_state().await {
                            if s.is_connected() {
                                set_sync_state.set(s);
                                break;
                            }
                            set_sync_state.set(s);
                        }
                    }
                    set_busy.set(false);
                }
                Err(e) => {
                    set_error.set(Some(format!("Could not start OAuth: {e}")));
                    set_busy.set(false);
                }
            }
        });
    };
    let on_connect_cb = Callback::new(on_connect);

    let on_disconnect = move |_: web_sys::MouseEvent| {
        if busy.get_untracked() {
            return;
        }
        set_busy.set(true);
        set_error.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            match api::disconnect_google().await {
                Ok(()) => {
                    set_sync_state.set(GoogleCalendarSyncState::default());
                    set_busy.set(false);
                }
                Err(e) => {
                    set_error.set(Some(format!("Disconnect failed: {e}")));
                    set_busy.set(false);
                }
            }
        });
    };
    let on_disconnect_cb = Callback::new(on_disconnect);

    let on_refresh = move |_: web_sys::MouseEvent| {
        if busy.get_untracked() {
            return;
        }
        set_busy.set(true);
        set_error.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            match api::force_google_sync().await {
                Ok(()) => set_busy.set(false),
                Err(e) => {
                    set_error.set(Some(format!("Refresh failed: {e}")));
                    set_busy.set(false);
                }
            }
        });
    };
    let on_refresh_cb = Callback::new(on_refresh);

    let on_clear_google = move |_: web_sys::MouseEvent| {
        let to_remove: Vec<CalendarEvent> = events.with_untracked(|evs| {
            evs.iter()
                .filter(|e| matches!(e.source, EventSource::Google))
                .cloned()
                .collect()
        });
        expect_context::<crate::task_saves::TaskSaves>().edit_events(
            to_remove
                .iter()
                .map(|event| (event.id.clone(), Some(event.clone()), None))
                .collect(),
        );
        let removed = to_remove
            .into_iter()
            .map(|event| event.id)
            .collect::<std::collections::HashSet<_>>();
        set_events.update(|list| {
            list.retain(|e| !removed.contains(&e.id));
        });
    };
    let on_clear_google_cb = Callback::new(on_clear_google);

    let status_text = Signal::derive(move || {
        let s = sync_state.get();
        if !s.is_connected() {
            return "Not connected. Connect to import events and enable auto-record.".to_string();
        }
        let who = match s.email() {
            Some(email) => format!("Connected as {email}"),
            None => "Connected".to_string(),
        };
        if let Some(err) = s.last_error {
            return format!("{who}. {err}");
        }
        match s.last_synced_at {
            Some(ts) => format!("{who}. Last synced {}.", synced_ago(ts)),
            None => who,
        }
    });
    let has_google_events = Signal::derive(move || {
        events.with(|evs| evs.iter().any(|e| matches!(e.source, EventSource::Google)))
    });

    view! {
        <SettingsRow title="Google Calendar" live_description=status_text>
            {move || {
                if sync_state.get().is_connected() {
                    view! {
                        <Button
                            variant=ButtonVariant::Ghost
                            size=ButtonSize::Small
                            on_click=on_refresh_cb
                            disabled=Signal::derive(move || busy.get())
                        >
                            "Refresh"
                        </Button>
                        <Button
                            variant=ButtonVariant::Outline
                            size=ButtonSize::Small
                            on_click=on_disconnect_cb
                            disabled=Signal::derive(move || busy.get())
                        >
                            "Disconnect"
                        </Button>
                    }.into_any()
                } else {
                    view! {
                        <Button
                            variant=ButtonVariant::Primary
                            size=ButtonSize::Small
                            on_click=on_connect_cb
                            disabled=Signal::derive(move || busy.get())
                        >
                            {move || if busy.get() { "Connecting…" } else { "Connect" }}
                        </Button>
                    }.into_any()
                }
            }}
        </SettingsRow>
        {move || error.get().map(|e| view! {
            <p class="px-4 py-2 text-label-sm text-lychee-300" role="alert">{e}</p>
        })}
        {move || sync_state.get().is_connected().then(|| view! {
            <SettingsRowBody>
                <CalendarPicker />
            </SettingsRowBody>
        })}
        {move || (!sync_state.get().is_connected() && has_google_events.get()).then(|| view! {
            <SettingsRow
                title="Imported events"
                description="Events that came from Google Calendar before it was disconnected."
            >
                <Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=on_clear_google_cb>
                    "Remove all"
                </Button>
            </SettingsRow>
        })}
    }
}
