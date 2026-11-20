//! Wall-clock arithmetic for calendar grids and provider dates.
//!
//! Events are stored as UTC instants. Grids, availability rules and all-day
//! dates are wall-clock concepts, and a day is 23 or 25 hours long when
//! daylight saving starts or ends. These helpers convert between the two
//! through an `offset_at` function: the zone's offset east of UTC, in
//! minutes, at a UTC instant in milliseconds. The web reads it from
//! `Date.getTimezoneOffset()` (negated); native code reads it from chrono's
//! `Local`. Tests pass a synthetic zone, so no helper reads the host zone.

const MINUTE_MS: i64 = 60_000;
const DAY_MS: i64 = 86_400_000;
/// Offsets in use stay within ±14 hours of UTC.
pub(crate) const MAX_OFFSET_MS: i64 = 14 * 60 * MINUTE_MS;

/// The wall-clock reading of `instant_ms`, expressed as milliseconds since
/// 1970-01-01T00:00 *on the wall clock* (so `div_euclid(DAY_MS)` is the
/// local date and the remainder is the local time of day).
pub fn wall_clock_ms(instant_ms: i64, offset_at: impl Fn(i64) -> i32) -> i64 {
    instant_ms.saturating_add(i64::from(offset_at(instant_ms)).saturating_mul(MINUTE_MS))
}

/// The instant whose wall clock reads `wall_ms` (see [`wall_clock_ms`]).
///
/// Matches JavaScript's `new Date(y, m, d, h, min)`: a wall time skipped by
/// a spring-forward change resolves with the offset before the change (so
/// 02:30 becomes 03:30), and a wall time repeated by a fall-back change
/// resolves to its first occurrence.
pub fn instant_from_wall_clock(wall_ms: i64, offset_at: impl Fn(i64) -> i32) -> i64 {
    let offset_ms = |instant: i64| i64::from(offset_at(instant)).saturating_mul(MINUTE_MS);
    // At most one transition is assumed within ±14 hours of the wall time.
    let before = offset_ms(wall_ms.saturating_sub(MAX_OFFSET_MS));
    let after = offset_ms(wall_ms.saturating_add(MAX_OFFSET_MS));
    let with_before = wall_ms.saturating_sub(before);
    let with_after = wall_ms.saturating_sub(after);
    let valid = |instant: i64, offset: i64| offset_ms(instant) == offset;
    match (valid(with_before, before), valid(with_after, after)) {
        (true, true) => with_before.min(with_after),
        (true, false) => with_before,
        (false, true) => with_after,
        // Skipped wall time: keep the offset in force before the change.
        (false, false) => with_before,
    }
}

/// Local midnight of the date containing `instant_ms`.
pub fn local_day_start(instant_ms: i64, offset_at: impl Fn(i64) -> i32 + Copy) -> i64 {
    let wall = wall_clock_ms(instant_ms, offset_at);
    instant_from_wall_clock(wall.div_euclid(DAY_MS) * DAY_MS, offset_at)
}

/// Wall-clock milliseconds between local midnight `day_start_ms` and
/// `instant_ms`: the position on a 24-row day grid. On a spring-forward day
/// 10:00 is still ten rows down although only nine hours have elapsed.
pub fn wall_offset_in_day(
    instant_ms: i64,
    day_start_ms: i64,
    offset_at: impl Fn(i64) -> i32 + Copy,
) -> i64 {
    wall_clock_ms(instant_ms, offset_at) - wall_clock_ms(day_start_ms, offset_at)
}

/// The instant `wall_offset_ms` of wall-clock time after local midnight
/// `day_start_ms`: the inverse of [`wall_offset_in_day`], used when a grid
/// position becomes an event time.
pub fn instant_at_wall_offset(
    day_start_ms: i64,
    wall_offset_ms: i64,
    offset_at: impl Fn(i64) -> i32 + Copy,
) -> i64 {
    instant_from_wall_clock(
        wall_clock_ms(day_start_ms, offset_at).saturating_add(wall_offset_ms),
        offset_at,
    )
}

/// Providers send an all-day event as a date (`2026-05-15`), which the sync
/// engine carries as that date's UTC midnight. Returns the local midnight
/// of the same date, which is how local all-day events are stored, so the
/// event lands on its own day in every zone.
pub fn provider_all_day_to_local(utc_midnight_ms: i64, offset_at: impl Fn(i64) -> i32) -> i64 {
    instant_from_wall_clock(utc_midnight_ms, offset_at)
}

/// Whether `ms` is a provider all-day boundary that has not been moved to
/// local midnight yet: a UTC midnight in a zone that is not on UTC then.
/// A local midnight is never a UTC midnight unless the offset is zero, so
/// normalizing twice is harmless.
pub fn is_unnormalized_provider_all_day(ms: i64, offset_at: impl Fn(i64) -> i32) -> bool {
    ms.rem_euclid(DAY_MS) == 0 && offset_at(ms) != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR_MS: i64 = 60 * MINUTE_MS;

    /// Days-from-civil (Howard Hinnant), so the tests need no date crate.
    fn utc(y: i64, m: i64, d: i64, h: i64, min: i64) -> i64 {
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = (m + 9) % 12;
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe - 719_468;
        days * DAY_MS + h * HOUR_MS + min * MINUTE_MS
    }

    /// US Pacific for 2026: PDT from 2026-03-08 10:00 UTC until
    /// 2026-11-01 09:00 UTC, PST otherwise.
    fn pacific(instant: i64) -> i32 {
        if (utc(2026, 3, 8, 10, 0)..utc(2026, 11, 1, 9, 0)).contains(&instant) {
            -7 * 60
        } else {
            -8 * 60
        }
    }

    /// Fixed UTC+5:45 (no daylight saving).
    fn kathmandu(_: i64) -> i32 {
        5 * 60 + 45
    }

    #[test]
    fn wall_clock_round_trips_outside_transitions() {
        let instant = utc(2026, 5, 15, 16, 30); // 09:30 PDT
        let wall = wall_clock_ms(instant, pacific);
        assert_eq!(wall, utc(2026, 5, 15, 9, 30));
        assert_eq!(instant_from_wall_clock(wall, pacific), instant);
        assert_eq!(
            instant_from_wall_clock(wall_clock_ms(instant, kathmandu), kathmandu),
            instant
        );
    }

    #[test]
    fn skipped_wall_time_moves_forward_like_javascript() {
        // 2026-03-08 02:30 does not exist in Pacific; JS gives 03:30 PDT.
        let got = instant_from_wall_clock(utc(2026, 3, 8, 2, 30), pacific);
        assert_eq!(got, utc(2026, 3, 8, 10, 30));
        assert_eq!(wall_clock_ms(got, pacific), utc(2026, 3, 8, 3, 30));
    }

    #[test]
    fn repeated_wall_time_resolves_to_first_occurrence() {
        // 2026-11-01 01:30 happens at 08:30 UTC (PDT) and 09:30 UTC (PST).
        assert_eq!(
            instant_from_wall_clock(utc(2026, 11, 1, 1, 30), pacific),
            utc(2026, 11, 1, 8, 30)
        );
    }

    #[test]
    fn local_days_are_23_and_25_hours_around_transitions() {
        let spring = local_day_start(utc(2026, 3, 8, 20, 0), pacific);
        assert_eq!(spring, utc(2026, 3, 8, 8, 0));
        let next = local_day_start(spring + 30 * HOUR_MS, pacific);
        assert_eq!(next - spring, 23 * HOUR_MS);
        let fall = local_day_start(utc(2026, 11, 1, 20, 0), pacific);
        assert_eq!(fall, utc(2026, 11, 1, 7, 0));
        let next = local_day_start(fall + 30 * HOUR_MS, pacific);
        assert_eq!(next - fall, 25 * HOUR_MS);
    }

    #[test]
    fn grid_position_uses_wall_clock_on_daylight_saving_days() {
        let spring = utc(2026, 3, 8, 8, 0); // local midnight
        let ten_am = utc(2026, 3, 8, 17, 0); // 10:00 PDT, nine hours later
        assert_eq!(wall_offset_in_day(ten_am, spring, pacific), 10 * HOUR_MS);
        assert_eq!(
            instant_at_wall_offset(spring, 10 * HOUR_MS, pacific),
            ten_am
        );

        let fall = utc(2026, 11, 1, 7, 0);
        let ten_am = utc(2026, 11, 1, 18, 0); // 10:00 PST, eleven hours later
        assert_eq!(wall_offset_in_day(ten_am, fall, pacific), 10 * HOUR_MS);
        assert_eq!(instant_at_wall_offset(fall, 10 * HOUR_MS, pacific), ten_am);
        // The end of the day is 24 wall hours down whatever its length.
        let next = local_day_start(fall + 30 * HOUR_MS, pacific);
        assert_eq!(wall_offset_in_day(next, fall, pacific), 24 * HOUR_MS);
    }

    #[test]
    fn provider_all_day_dates_land_on_local_midnight() {
        let may_15 = utc(2026, 5, 15, 0, 0);
        assert!(is_unnormalized_provider_all_day(may_15, pacific));
        let local = provider_all_day_to_local(may_15, pacific);
        assert_eq!(local, utc(2026, 5, 15, 7, 0)); // 00:00 PDT
        assert!(!is_unnormalized_provider_all_day(local, pacific));
        assert_eq!(
            provider_all_day_to_local(may_15, kathmandu),
            may_15 - (5 * 60 + 45) * MINUTE_MS
        );
        // In UTC itself the date is already local midnight.
        assert!(!is_unnormalized_provider_all_day(may_15, |_| 0));
        assert_eq!(provider_all_day_to_local(may_15, |_| 0), may_15);
        // A two-day event across the spring change keeps whole local days.
        let start = provider_all_day_to_local(utc(2026, 3, 8, 0, 0), pacific);
        let end = provider_all_day_to_local(utc(2026, 3, 9, 0, 0), pacific);
        assert_eq!(end - start, 23 * HOUR_MS);
        assert_eq!(local_day_start(end, pacific), end);
    }
}
