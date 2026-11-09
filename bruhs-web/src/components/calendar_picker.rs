//! Per-calendar toggle list inside GoogleCalendarSettings. Reads the
//! signal-side calendars list (populated by the CalendarsRefreshed
//! reducer); also hits the server once on mount to hydrate (for users
//! who connected before this component existed).

use leptos::prelude::*;

use crate::api;
use crate::state::{Calendar, CalendarsSignal, SetCalendarsSignal};
use crate::ui::Checkbox;

#[component]
pub fn CalendarPicker() -> impl IntoView {
    let CalendarsSignal(calendars) = expect_context::<CalendarsSignal>();
    let SetCalendarsSignal(set_calendars) = expect_context::<SetCalendarsSignal>();

    // Hydrate from the server on mount.
    Effect::new(move |_| {
        wasm_bindgen_futures::spawn_local(async move {
            if let Ok(cals) = api::fetch_google_calendars().await {
                set_calendars.set(cals);
            }
        });
    });

    let toggle = Callback::new(move |(id, next): (String, bool)| {
        // Optimistic update.
        set_calendars.update(|list| {
            if let Some(slot) = list.iter_mut().find(|c| c.id == id) {
                slot.sync_enabled = next;
            }
        });
        wasm_bindgen_futures::spawn_local(async move {
            let body = api::PatchCalendarBody { sync_enabled: next };
            if let Err(e) = api::patch_google_calendar(&id, &body).await {
                log::warn!("patch_google_calendar failed: {e}");
                // Best-effort rollback: re-fetch.
                if let Ok(cals) = api::fetch_google_calendars().await {
                    set_calendars.set(cals);
                }
            }
        });
    });

    view! {
        <div class="space-y-1">
            <div class="text-label text-rambutan-500">"Calendars"</div>
            {move || {
                let list = calendars.get();
                if list.is_empty() {
                    view! {
                        <div class="text-label text-rambutan-500 italic">"No calendars yet — wait for the next sync."</div>
                    }.into_any()
                } else {
                    view! {
                        <ul role="list" class="space-y-0.5">
                            {list.into_iter().map(|c: Calendar| {
                                let id_for_toggle = c.id.clone();
                                let enabled = c.sync_enabled;
                                let is_primary = c.primary;
                                let name = c.name;
                                let aria = format!("Sync {name}");
                                view! {
                                    <li class="flex items-center justify-between gap-2 text-label text-rambutan-300">
                                        <span class="flex items-center gap-1.5">
                                            {if is_primary {
                                                view! {
                                                    <span class="text-label-sm text-persimmon-400 uppercase tracking-wide">"primary"</span>
                                                }.into_any()
                                            } else {
                                                ().into_any()
                                            }}
                                            <span>{name}</span>
                                        </span>
                                        <Checkbox
                                            checked=enabled
                                            label=aria
                                            on_change=Callback::new(move |next: bool| {
                                                toggle.run((id_for_toggle.clone(), next));
                                            })
                                        />
                                    </li>
                                }
                            }).collect_view()}
                        </ul>
                    }.into_any()
                }
            }}
        </div>
    }
}
