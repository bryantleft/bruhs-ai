//! The local booking page for a saved scheduling link, shared by web and
//! desktop: which times to offer, how a time reads, and the local event a
//! booking writes. Moved from the web crate's `booking_page.rs`.

use crate::calendar::zone::wall_clock_ms;
use crate::{CalendarEvent, EpochMs, EventSource, SchedulingLink, SchedulingPreferences};

const MINUTE_MS: i64 = 60_000;
const DAY_MS: i64 = 86_400_000;
/// Times a booking page offers at once.
pub const BOOKING_SLOT_COUNT: usize = 6;
/// Space after one offered time's reserved span before the next is sought.
const SLOT_SPACING_MINUTES: i64 = 15;

pub const BOOKING_EYEBROW: &str = "Bruhs scheduling";
pub const BOOKING_BACK: &str = "← Back to calendar";
pub const BOOKING_UNAVAILABLE_TITLE: &str = "Link unavailable";
pub const BOOKING_UNAVAILABLE_DETAIL: &str =
    "This local scheduling link was removed, disabled, or belongs to another workspace.";
pub const BOOKING_NO_SLOTS: &str =
    "No times are currently available inside this link’s booking window.";
pub const BOOKING_NAME_PLACEHOLDER: &str = "Your name";
/// The status while a booking is written, and when it could not be.
pub const BOOKING_PREPARING: &str = "Preparing your booking…";
pub const BOOKING_SAVING: &str = "Saving your booking…";
pub const BOOKING_NOT_SAVED: &str =
    "Booking not saved. Use the save recovery notice to retry; your draft is kept here.";
/// The event color a booking gets.
const BOOKING_COLOR: &str = "persimmon";

/// The enabled link a booking page for `slug` shows.
pub fn booking_link<'a>(
    preferences: &'a SchedulingPreferences,
    slug: &str,
) -> Option<&'a SchedulingLink> {
    preferences
        .links
        .iter()
        .find(|link| link.enabled && link.slug == slug)
}

/// The line under the link's title.
pub fn booking_duration_line(link: &SchedulingLink) -> String {
    format!(
        "{} minutes, protected by your availability and buffers",
        link.duration_minutes
    )
}

/// Up to [`BOOKING_SLOT_COUNT`] meeting times `(start, end)` in UTC ms, from
/// the link's minimum notice through its booking window. Each reserves the
/// link's buffers around the meeting inside the availability rules and
/// clear of `events`. `offset_at` gives the zone's offset per instant, so a
/// window that crosses a daylight-saving change stays right.
pub fn available_booking_slots(
    preferences: &SchedulingPreferences,
    link: &SchedulingLink,
    events: &[CalendarEvent],
    now_ms: i64,
    offset_at: impl Fn(i64) -> i32 + Copy,
) -> Vec<(i64, i64)> {
    let mut slots = Vec::new();
    let mut cursor = now_ms + i64::from(link.minimum_notice_hours) * 60 * MINUTE_MS;
    let through = now_ms + i64::from(link.booking_window_days.max(1)) * DAY_MS;
    let reserved_minutes = link
        .buffer_before_minutes
        .saturating_add(link.duration_minutes)
        .saturating_add(link.buffer_after_minutes);
    while slots.len() < BOOKING_SLOT_COUNT {
        let Some((reserved_start, reserved_end)) = preferences.first_available_slot_in_zone(
            events,
            cursor,
            through,
            reserved_minutes,
            offset_at,
        ) else {
            break;
        };
        let meeting_start = reserved_start + i64::from(link.buffer_before_minutes) * MINUTE_MS;
        let meeting_end = meeting_start + i64::from(link.duration_minutes) * MINUTE_MS;
        slots.push((meeting_start, meeting_end));
        cursor = reserved_end + SLOT_SPACING_MINUTES * MINUTE_MS;
    }
    slots
}

/// A time as the page shows it, such as `Nov 17, 9:15 AM`, in the zone
/// `offset_at` describes.
pub fn booking_slot_label(instant_ms: i64, offset_at: impl Fn(i64) -> i32) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let wall = wall_clock_ms(instant_ms, offset_at);
    let (_, month, day) = civil_from_days(wall.div_euclid(DAY_MS));
    let minute_of_day = wall.rem_euclid(DAY_MS) / MINUTE_MS;
    let (hour, minute) = (minute_of_day / 60, minute_of_day % 60);
    let period = if hour >= 12 { "PM" } else { "AM" };
    let display_hour = match hour % 12 {
        0 => 12,
        value => value,
    };
    let month_name = usize::try_from(month.saturating_sub(1))
        .ok()
        .and_then(|index| MONTHS.get(index))
        .copied()
        .unwrap_or("Jan");
    format!("{month_name} {day}, {display_hour}:{minute:02} {period}")
}

/// The local event booking `slot` writes, widened by the link's buffers so
/// the calendar keeps them free. `guest_name` joins the title when given.
pub fn booking_event(
    link: &SchedulingLink,
    guest_name: &str,
    slot: (i64, i64),
    id: String,
    now_ms: i64,
) -> CalendarEvent {
    let (start, end) = slot;
    let guest_name = guest_name.trim();
    CalendarEvent {
        id,
        title: if guest_name.is_empty() {
            link.title.clone()
        } else {
            format!("{}, {guest_name}", link.title)
        },
        start_at_ms: EpochMs::saturating_from_i64(
            start - i64::from(link.buffer_before_minutes) * MINUTE_MS,
        )
        .as_f64(),
        end_at_ms: EpochMs::saturating_from_i64(
            end + i64::from(link.buffer_after_minutes) * MINUTE_MS,
        )
        .as_f64(),
        all_day: false,
        color: Some(BOOKING_COLOR.to_string()),
        notes: Some(format!(
            "Booked locally via #/book/{}; meeting time {start} to {end}",
            link.slug
        )),
        created_at: now_ms,
        updated_at: now_ms,
        source: EventSource::Local,
        google_event_id: None,
        calendar_id: None,
        attendees: Vec::new(),
        organizer: None,
        location: None,
        conferencing: None,
        recurring_event_id: None,
        html_link: None,
    }
}

/// The status after a booking is saved.
pub fn booking_confirmation(label: &str) -> String {
    format!("Booked for {label}. It is saved on your calendar.")
}

/// `(year, month 1..=12, day 1..=31)` of `days` since 1970-01-01 (the
/// proleptic Gregorian calendar).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = if month_from_march < 10 {
        month_from_march + 3
    } else {
        month_from_march - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (
        year,
        u32::try_from(month).unwrap_or(1),
        u32::try_from(day).unwrap_or(1),
    )
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
        let slots =
            available_booking_slots(&preferences, &link, &[], monday + 4 * 60 * MINUTE_MS, |_| 0);
        assert_eq!(slots[0].0, monday + 9 * 60 * MINUTE_MS + 15 * MINUTE_MS);
        assert_eq!(slots[0].1 - slots[0].0, 30 * MINUTE_MS);
        assert!(slots.len() <= BOOKING_SLOT_COUNT);
    }

    #[test]
    fn labels_read_in_the_zone_and_dates_follow_the_calendar() {
        // 2026-11-17 14:15 UTC.
        let instant = 1_794_924_900_000;
        assert_eq!(booking_slot_label(instant, |_| 0), "Nov 17, 2:15 PM");
        assert_eq!(booking_slot_label(instant, |_| -8 * 60), "Nov 17, 6:15 AM");
        assert_eq!(booking_slot_label(instant, |_| 10 * 60), "Nov 18, 12:15 AM");
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn a_booking_writes_a_local_event_with_buffers_and_the_guest() {
        let mut preferences = SchedulingPreferences::default();
        let mut link = SchedulingLink::new("Review", 30);
        link.buffer_before_minutes = 10;
        link.buffer_after_minutes = 5;
        preferences.links.push(link.clone());
        assert_eq!(
            booking_link(&preferences, &link.slug).map(|found| found.id.as_str()),
            Some(link.id.as_str())
        );
        let event = booking_event(
            &link,
            "  Ada ",
            (60 * MINUTE_MS, 90 * MINUTE_MS),
            "e".into(),
            7,
        );
        assert_eq!(event.title, "Review, Ada");
        assert_eq!(event.start_at_ms, f64::from(50 * 60_000_i32));
        assert_eq!(event.end_at_ms, f64::from(95 * 60_000_i32));
        assert_eq!(event.source, EventSource::Local);
        assert_eq!(
            booking_event(&link, " ", (0, 0), "f".into(), 0).title,
            "Review"
        );
        link.enabled = false;
        preferences.links[0].enabled = false;
        assert!(booking_link(&preferences, &link.slug).is_none());
        assert_eq!(
            booking_confirmation("Nov 17, 2:15 PM"),
            "Booked for Nov 17, 2:15 PM. It is saved on your calendar."
        );
    }
}
