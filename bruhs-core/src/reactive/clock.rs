//! Wall-clock schedules in named time zones: calendar sources, follow-up
//! deadlines and quiet hours.

use super::ReactiveError;
use bruhs_models::reactive::{parse_hhmm, QuietHours};
use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike, Utc};
use chrono_tz::Tz;

/// Occurrences counted when reporting missed slots after downtime.
const MISSED_CAP: u64 = 1_000;

pub(super) fn zone(name: &str) -> Result<Tz, ReactiveError> {
    name.trim().parse::<Tz>().map_err(|_| {
        ReactiveError::Invalid(format!(
            "“{name}” is not a known time zone. Use an IANA name such as America/Los_Angeles."
        ))
    })
}

/// The machine's own zone name, for defaults.
pub fn local_zone_name() -> String {
    iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".into())
}

fn instant(zone: Tz, date: NaiveDate, minute: u16) -> Option<i64> {
    let time = NaiveTime::from_hms_opt(u32::from(minute / 60), u32::from(minute % 60), 0)?;
    let local = NaiveDateTime::new(date, time);
    // A time skipped by a daylight-saving jump fires at the first valid instant after it.
    let resolved = zone.from_local_datetime(&local).earliest().or_else(|| {
        (1..=4).find_map(|hours| {
            zone.from_local_datetime(&(local + Duration::hours(hours)))
                .earliest()
        })
    })?;
    Some(resolved.timestamp_millis())
}

fn minutes(times: &[String]) -> Result<Vec<u16>, ReactiveError> {
    let mut values = times
        .iter()
        .map(|time| {
            parse_hhmm(time)
                .ok_or_else(|| ReactiveError::Invalid(format!("“{time}” is not a HH:MM time.")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    values.sort_unstable();
    values.dedup();
    if values.is_empty() {
        return Err(ReactiveError::Invalid(
            "Choose at least one time of day.".into(),
        ));
    }
    Ok(values)
}

/// The first scheduled instant strictly after `after_ms`.
pub(super) fn next_calendar_due(
    times: &[String],
    weekdays: &[u8],
    time_zone: &str,
    after_ms: i64,
) -> Result<i64, ReactiveError> {
    let zone = zone(time_zone)?;
    let times = minutes(times)?;
    let start = Utc
        .timestamp_millis_opt(after_ms)
        .single()
        .ok_or_else(|| ReactiveError::Invalid("The schedule time is outside the clock.".into()))?
        .with_timezone(&zone)
        .date_naive();
    // Every weekday occurs within eight local days, including DST transitions.
    for offset in 0..9 {
        let date = start + Duration::days(offset);
        let weekday = u8::try_from(date.weekday().number_from_monday()).unwrap_or(0);
        if !weekdays.is_empty() && !weekdays.contains(&weekday) {
            continue;
        }
        if let Some(due) = times
            .iter()
            .filter_map(|minute| instant(zone, date, *minute))
            .filter(|due| *due > after_ms)
            .min()
        {
            return Ok(due);
        }
    }
    Err(ReactiveError::Invalid(
        "This calendar schedule never comes due.".into(),
    ))
}

/// Scheduled instants in `(from_ms, to_ms]`, capped for long downtime.
pub(super) fn missed_between(
    times: &[String],
    weekdays: &[u8],
    time_zone: &str,
    from_ms: i64,
    to_ms: i64,
) -> Result<u64, ReactiveError> {
    let mut count = 0;
    let mut cursor = from_ms;
    while count < MISSED_CAP {
        let due = next_calendar_due(times, weekdays, time_zone, cursor)?;
        if due > to_ms {
            break;
        }
        count += 1;
        cursor = due;
    }
    Ok(count)
}

/// Whether `now_ms` falls inside the owner's quiet hours.
pub(super) fn in_quiet_hours(hours: &QuietHours, now_ms: i64) -> Result<bool, ReactiveError> {
    let zone = zone(&hours.time_zone)?;
    let local = Utc
        .timestamp_millis_opt(now_ms)
        .single()
        .ok_or_else(|| ReactiveError::Invalid("The clock is out of range.".into()))?
        .with_timezone(&zone);
    let minute = u16::try_from(local.hour() * 60 + local.minute()).unwrap_or(0);
    hours
        .contains_minute(minute)
        .ok_or_else(|| ReactiveError::Invalid("Quiet hours use HH:MM times.".into()))
}

pub(super) fn parse_rfc3339(text: &str) -> Result<i64, ReactiveError> {
    chrono::DateTime::parse_from_rfc3339(text.trim())
        .map(|time| time.timestamp_millis())
        .map_err(|_| {
            ReactiveError::Invalid(format!(
                "“{text}” is not an RFC 3339 time with an offset, such as 2026-11-20T09:00:00-08:00."
            ))
        })
}

/// Human-readable local time for prompts, e.g. `2026-11-14 08:00 PST`.
pub(super) fn describe(ms: i64, time_zone: &str) -> String {
    let Ok(zone) = zone(time_zone) else {
        return format!("{ms} ms since the Unix epoch");
    };
    Utc.timestamp_millis_opt(ms).single().map_or_else(
        || format!("{ms} ms since the Unix epoch"),
        |time| {
            time.with_timezone(&zone)
                .format("%Y-%m-%d %H:%M %Z")
                .to_string()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(text: &str) -> i64 {
        parse_rfc3339(text).unwrap()
    }

    #[test]
    fn weekday_mornings_skip_weekends_in_the_named_zone() {
        let times = vec!["08:00".to_owned()];
        // Friday 2026-11-13 09:00 in Los Angeles -> next is Monday 08:00 local.
        let after = ms("2026-11-13T09:00:00-08:00");
        let due =
            next_calendar_due(&times, &[1, 2, 3, 4, 5], "America/Los_Angeles", after).unwrap();
        assert_eq!(due, ms("2026-11-16T08:00:00-08:00"));
        // Same instant, every day -> Saturday 08:00.
        let due = next_calendar_due(&times, &[], "America/Los_Angeles", after).unwrap();
        assert_eq!(due, ms("2026-11-14T08:00:00-08:00"));
    }

    #[test]
    fn dst_gaps_fire_after_the_jump_and_offsets_follow_the_zone() {
        // 2026-03-08 02:30 does not exist in New York; it fires at 03:30 EDT.
        let after = ms("2026-03-08T00:00:00-05:00");
        let due = next_calendar_due(&["02:30".into()], &[], "America/New_York", after).unwrap();
        assert_eq!(due, ms("2026-03-08T03:30:00-04:00"));
        // After the change, 08:00 local is UTC-4.
        let due = next_calendar_due(&["08:00".into()], &[], "America/New_York", due).unwrap();
        assert_eq!(due, ms("2026-03-08T08:00:00-04:00"));
    }

    #[test]
    fn missed_slots_are_counted_and_bad_zones_rejected() {
        let from = ms("2026-11-09T00:00:00Z");
        let to = ms("2026-11-16T00:00:00Z");
        assert_eq!(
            missed_between(
                &["09:00".into(), "17:00".into()],
                &[1, 2, 3, 4, 5],
                "UTC",
                from,
                to
            )
            .unwrap(),
            10
        );
        assert!(next_calendar_due(&["09:00".into()], &[], "Mars/Olympus", from).is_err());
    }

    #[test]
    fn quiet_hours_use_the_owner_zone() {
        let hours = QuietHours {
            start: "22:00".into(),
            end: "07:00".into(),
            time_zone: "Asia/Tokyo".into(),
        };
        // 14:00 UTC is 23:00 in Tokyo.
        assert!(in_quiet_hours(&hours, ms("2026-11-14T14:00:00Z")).unwrap());
        assert!(!in_quiet_hours(&hours, ms("2026-11-14T03:00:00Z")).unwrap());
    }
}
