//! Local-first booking surface for a saved scheduling link.

use bruhs_models::num::ToIntSaturating;
use bruhs_models::{CalendarEvent, EventSource, SchedulingLink, SchedulingPreferences};
use leptos::prelude::*;

use crate::state::{
    AppView, CalendarEventsSignal, SchedulingPreferencesSignal, SetActiveView,
    SetCalendarEventsSignal,
};
use crate::storage;
use crate::ui::Input;

const MINUTE_MS: i64 = 60_000;
const DAY_MS: i64 = 86_400_000;

fn available_slots(
    preferences: &SchedulingPreferences,
    link: &SchedulingLink,
    events: &[CalendarEvent],
    now_ms: i64,
    utc_offset_minutes: i16,
) -> Vec<(i64, i64)> {
    let mut slots = Vec::new();
    let mut cursor = now_ms + i64::from(link.minimum_notice_hours) * 60 * MINUTE_MS;
    let through = now_ms + i64::from(link.booking_window_days.max(1)) * DAY_MS;
    let reserved_minutes = link
        .buffer_before_minutes
        .saturating_add(link.duration_minutes)
        .saturating_add(link.buffer_after_minutes);
    while slots.len() < 6 {
        let Some((reserved_start, reserved_end)) = preferences.first_available_slot(
            events,
            cursor,
            through,
            reserved_minutes,
            utc_offset_minutes,
        ) else {
            break;
        };
        let meeting_start = reserved_start + i64::from(link.buffer_before_minutes) * MINUTE_MS;
        let meeting_end = meeting_start + i64::from(link.duration_minutes) * MINUTE_MS;
        slots.push((meeting_start, meeting_end));
        cursor = reserved_end + 15 * MINUTE_MS;
    }
    slots
}

fn slot_label(timestamp_ms: i64) -> String {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(
        bruhs_models::EpochMs::saturating_from_i64(timestamp_ms).as_f64(),
    ));
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let hour = date.get_hours();
    let minute = date.get_minutes();
    let period = if hour >= 12 { "PM" } else { "AM" };
    let display_hour = match hour % 12 {
        0 => 12,
        value => value,
    };
    format!(
        "{} {} · {}:{:02} {}",
        MONTHS[date.get_month() as usize],
        date.get_date(),
        display_hour,
        minute,
        period
    )
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
    let link = Signal::derive(move || {
        preferences
            .get()
            .links
            .into_iter()
            .find(|link| link.enabled && link.slug == slug)
    });
    let slots = Signal::derive(move || {
        link.get().map_or_else(Vec::new, |link| {
            let now = crate::clock::now_ms();
            let offset = -js_sys::Date::new_0()
                .get_timezone_offset()
                .to_i16_saturating();
            available_slots(&preferences.get(), &link, &events.get(), now, offset)
        })
    });

    view! {
        <div class="flex min-h-full items-start justify-center overflow-auto bg-longan-950 px-4 py-10 text-rambutan-300">
            <div class="w-full max-w-xl rounded-grape bg-longan-900/65 p-5 inset-ring inset-ring-longan-800">
                <button type="button" class="mb-5 text-label-sm text-rambutan-500 hover:text-rambutan-300 cursor-pointer" on:click=move |_| set_active_view.set(AppView::Calendar)>"← Back to calendar"</button>
                {move || match link.get() {
                    None => view! { <div role="status"><p class="text-h3 text-rambutan-100">"Link unavailable"</p><p class="mt-2 text-body-sm text-rambutan-500">"This local scheduling link was removed, disabled, or belongs to another workspace."</p></div> }.into_any(),
                    Some(link) => {
                        let candidate_slots = slots.get();
                        let no_slots = candidate_slots.is_empty();
                        let slot_buttons = candidate_slots.into_iter().map(|(start, end)| {
                            let event_title = link.title.clone();
                            let link_slug = link.slug.clone();
                            let buffer_before = link.buffer_before_minutes;
                            let buffer_after = link.buffer_after_minutes;
                            view! { <button type="button" aria-label=format!("Book {}", slot_label(start)) class="rounded-seed bg-longan-800 px-3 py-3 text-left text-body-sm text-blueberry-300 inset-ring inset-ring-longan-700 hover:bg-blueberry-500/10 cursor-pointer" on:click=move |_| {
                                let name = guest_name.get_untracked();
                                let now = crate::clock::now_ms();
                                let event = CalendarEvent {
                                    id: uuid::Uuid::new_v4().to_string(),
                                    title: if name.trim().is_empty() { event_title.clone() } else { format!("{} · {}", event_title, name.trim()) },
                                    start_at_ms: bruhs_models::EpochMs::saturating_from_i64(start - i64::from(buffer_before) * MINUTE_MS).as_f64(),
                                    end_at_ms: bruhs_models::EpochMs::saturating_from_i64(end + i64::from(buffer_after) * MINUTE_MS).as_f64(),
                                    all_day: false,
                                    color: Some("persimmon".to_string()),
                                    notes: Some(format!("Booked locally via #/book/{link_slug}; meeting time {start}–{end}")),
                                    created_at: now,
                                    updated_at: now,
                                    source: EventSource::Local,
                                    google_event_id: None,
                                    calendar_id: None,
                                    attendees: Vec::new(), organizer: None, location: None, conferencing: None,
                                    recurring_event_id: None, html_link: None,
                                };
                                set_events.update(|items| items.push(event.clone()));
                                let event_id = event.id.clone();
                                storage::persist_event(None, event);
                                booked.set(Some((event_id, slot_label(start))));
                            }>{slot_label(start)}</button> }
                        }).collect_view();
                        view! {
                            <div>
                                <p class="text-label-sm uppercase tracking-wide text-persimmon-400">"Bruhs scheduling"</p>
                                <h1 class="mt-2 text-h2 text-rambutan-100">{link.title}</h1>
                                <p class="mt-1 text-body-sm text-rambutan-500">{format!("{} minutes · protected by your availability and buffers", link.duration_minutes)}</p>
                                <div class="mt-5"><Input value=Signal::derive(move || guest_name.get()) on_input=Callback::new(move |value| guest_name.set(value)) placeholder="Your name".to_string() /></div>
                                {match booked.get() {
                                    Some((id, label)) => view! { <div role="status" class="mt-5 rounded-seed bg-longan-800 p-3 text-body-sm text-rambutan-300">{move || match saves.record_status(crate::task_saves::Kind::Event, &id) {
                                        crate::durable_metadata::SaveState::Saved => format!("Booked for {label}. It is saved on your calendar."),
                                        crate::durable_metadata::SaveState::Saving => "Saving your booking…".to_string(),
                                        crate::durable_metadata::SaveState::Error { .. } => "Booking not saved. Use the save recovery notice to retry; your draft is kept here.".to_string(),
                                        _ => "Preparing your booking…".to_string(),
                                    }}</div> }.into_any(),
                                    None => view! {
                                        <div>
                                            <div class="mt-5 grid gap-2 sm:grid-cols-2">{slot_buttons}</div>
                                            <Show when=move || no_slots><p role="status" class="mt-5 text-body-sm text-rambutan-500">"No times are currently available inside this link’s booking window."</p></Show>
                                        </div>
                                    }.into_any(),
                                }}
                            </div>
                        }.into_any()
                    }
                }}
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn booking_slots_include_notice_and_reserve_buffers() {
        let mut preferences = SchedulingPreferences::default();
        preferences.availability[0].weekdays = (0..=6).collect();
        let mut link = SchedulingLink::new("Review", 30);
        link.minimum_notice_hours = 4;
        link.buffer_before_minutes = 15;
        link.buffer_after_minutes = 15;
        let monday = 4 * DAY_MS;
        let slots = available_slots(&preferences, &link, &[], monday + 4 * 60 * MINUTE_MS, 0);
        assert_eq!(slots[0].0, monday + 9 * 60 * MINUTE_MS + 15 * MINUTE_MS);
        assert_eq!(slots[0].1 - slots[0].0, 30 * MINUTE_MS);
    }
}
