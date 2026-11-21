//! Local-first booking surface for a saved scheduling link.

use bruhs_models::logic::booking::{
    available_booking_slots, booking_confirmation, booking_duration_line, booking_event,
    booking_link, booking_slot_label, BOOKING_BACK, BOOKING_EYEBROW, BOOKING_NAME_PLACEHOLDER,
    BOOKING_NOT_SAVED, BOOKING_NO_SLOTS, BOOKING_PREPARING, BOOKING_SAVING,
    BOOKING_UNAVAILABLE_DETAIL, BOOKING_UNAVAILABLE_TITLE,
};
use leptos::prelude::*;

use crate::state::{
    AppView, CalendarEventsSignal, SchedulingPreferencesSignal, SetActiveView,
    SetCalendarEventsSignal,
};
use crate::storage;
use crate::ui::Input;

/// A time as the page shows it, in this browser's zone.
fn slot_label(timestamp_ms: i64) -> String {
    booking_slot_label(timestamp_ms, crate::clock::offset_minutes_east)
}

#[component]
pub fn BookingPage(slug: String) -> impl IntoView {
    let SchedulingPreferencesSignal(preferences) = expect_context::<SchedulingPreferencesSignal>();
    let CalendarEventsSignal(events) = expect_context::<CalendarEventsSignal>();
    let SetCalendarEventsSignal(set_events) = expect_context::<SetCalendarEventsSignal>();
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();
    let guest_name = RwSignal::new(String::new());
    let booked = RwSignal::new(Option::<(String, String)>::None);
    let saves = expect_context::<crate::task_saves::TaskSaves>();
    let link = Signal::derive(move || booking_link(&preferences.get(), &slug).cloned());
    let slots = Signal::derive(move || {
        link.get().map_or_else(Vec::new, |link| {
            let now = crate::clock::now_ms();
            // Per-instant offsets: a booking window can cross a DST change.
            available_booking_slots(
                &preferences.get(),
                &link,
                &events.get(),
                now,
                crate::clock::offset_minutes_east,
            )
        })
    });

    view! {
        <div class="flex min-h-full items-start justify-center overflow-auto bg-longan-950 px-4 py-10 text-rambutan-300">
            <div class="w-full max-w-xl rounded-grape bg-longan-900/65 p-5 inset-ring inset-ring-longan-800">
                <button type="button" class="mb-5 text-label-sm text-rambutan-500 hover:text-rambutan-300 cursor-pointer" on:click=move |_| set_active_view.set(AppView::Calendar)>{BOOKING_BACK}</button>
                <Show when=move || !preferences.ready()>
                    <p role="status">"Booking is paused until scheduling preferences are restored."</p>
                </Show>
                <Show when=move || preferences.ready()>
                {move || match link.get() {
                    None => view! { <div role="status"><p class="text-h3 text-rambutan-100">{BOOKING_UNAVAILABLE_TITLE}</p><p class="mt-2 text-body-sm text-rambutan-500">{BOOKING_UNAVAILABLE_DETAIL}</p></div> }.into_any(),
                    Some(link) => {
                        let candidate_slots = slots.get();
                        let no_slots = candidate_slots.is_empty();
                        let duration_line = booking_duration_line(&link);
                        let slot_buttons = candidate_slots.into_iter().map(|(start, end)| {
                            let booked_link = link.clone();
                            view! { <button type="button" aria-label=format!("Book {}", slot_label(start)) class="rounded-seed bg-longan-800 px-3 py-3 text-left text-body-sm text-blueberry-300 inset-ring inset-ring-longan-700 hover:bg-blueberry-500/10 cursor-pointer" on:click=move |_| {
                                if !preferences.ready() {
                                    return;
                                }
                                let event = booking_event(
                                    &booked_link,
                                    &guest_name.get_untracked(),
                                    (start, end),
                                    uuid::Uuid::new_v4().to_string(),
                                    crate::clock::now_ms(),
                                );
                                set_events.update(|items| items.push(event.clone()));
                                let event_id = event.id.clone();
                                storage::persist_event(None, event);
                                booked.set(Some((event_id, slot_label(start))));
                            }>{slot_label(start)}</button> }
                        }).collect_view();
                        view! {
                            <div>
                                <p class="text-label-sm uppercase tracking-wide text-persimmon-400">{BOOKING_EYEBROW}</p>
                                <h1 class="mt-2 text-h2 text-rambutan-100">{link.title}</h1>
                                <p class="mt-1 text-body-sm text-rambutan-500">{duration_line}</p>
                                <div class="mt-5"><Input value=Signal::derive(move || guest_name.get()) on_input=Callback::new(move |value| guest_name.set(value)) placeholder=BOOKING_NAME_PLACEHOLDER.to_string() /></div>
                                {match booked.get() {
                                    Some((id, label)) => view! { <div role="status" class="mt-5 rounded-seed bg-longan-800 p-3 text-body-sm text-rambutan-300">{move || match saves.record_status(crate::task_saves::Kind::Event, &id) {
                                        crate::durable_metadata::SaveState::Saved => booking_confirmation(&label),
                                        crate::durable_metadata::SaveState::Saving => BOOKING_SAVING.to_string(),
                                        crate::durable_metadata::SaveState::Error { .. } => BOOKING_NOT_SAVED.to_string(),
                                        _ => BOOKING_PREPARING.to_string(),
                                    }}</div> }.into_any(),
                                    None => view! {
                                        <div>
                                            <div class="mt-5 grid gap-2 sm:grid-cols-2">{slot_buttons}</div>
                                            <Show when=move || no_slots><p role="status" class="mt-5 text-body-sm text-rambutan-500">{BOOKING_NO_SLOTS}</p></Show>
                                        </div>
                                    }.into_any(),
                                }}
                            </div>
                        }.into_any()
                    }
                }}
                </Show>
            </div>
        </div>
    }
}
