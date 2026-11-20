//! Working-hour gaps in one local date, including offset changes inside a block.
use super::{valid_interval, DayPlanChangeError, DayPlanWindow};
use crate::{AvailabilityRule, EpochMs};

const MINUTE_MS: i64 = 60_000;
const DAY_MS: i64 = 24 * 60 * MINUTE_MS;

#[cfg(test)]
mod tests;

pub(in crate::models::planning) fn unavailable_time(
    window: DayPlanWindow,
    rules: &[AvailabilityRule],
    offset_at: impl Fn(i64) -> i32 + Copy,
) -> Result<Vec<(f64, f64)>, DayPlanChangeError> {
    if !valid_interval(window.start_at_ms, window.end_at_ms) {
        return Err(DayPlanChangeError::InvalidWindow);
    }
    let start = EpochMs::try_from(window.start_at_ms)
        .map_err(|_| DayPlanChangeError::InvalidWindow)?
        .as_millis();
    let end = EpochMs::try_from(window.end_at_ms)
        .map_err(|_| DayPlanChangeError::InvalidWindow)?
        .as_millis();
    // One wall-clock date plus the calendar zone contract's full offset span.
    // This bounds minute-level expansion without inventing a resource limit.
    if end - start > DAY_MS + 2 * crate::calendar::zone::MAX_OFFSET_MS {
        return Err(DayPlanChangeError::InvalidWindow);
    }
    if rules.iter().any(|rule| rule.enabled && !rule.is_valid()) {
        return Err(DayPlanChangeError::InvalidAvailability);
    }
    let wall = |instant| {
        let offset = i64::from(offset_at(instant)) * MINUTE_MS;
        if offset.abs() > crate::calendar::zone::MAX_OFFSET_MS {
            return Err(DayPlanChangeError::InvalidWindow);
        }
        instant
            .checked_add(offset)
            .ok_or(DayPlanChangeError::InvalidWindow)
    };
    let date = wall(start)?.div_euclid(DAY_MS);
    let weekday =
        u8::try_from((date + 4).rem_euclid(7)).map_err(|_| DayPlanChangeError::InvalidWindow)?;
    let mut gaps = Vec::new();
    let mut gap_start = None;
    let mut cursor = start;
    while cursor < end {
        let next = cursor.saturating_add(MINUTE_MS).min(end);
        let allowed = |local: i64| {
            let minute = local.rem_euclid(DAY_MS) / MINUTE_MS;
            local.div_euclid(DAY_MS) == date
                && rules.iter().any(|rule| {
                    rule.enabled
                        && rule.weekdays.contains(&weekday)
                        && minute >= i64::from(rule.start_minute)
                        && minute < i64::from(rule.end_minute)
                })
        };
        // Rule boundaries and zone offsets have minute granularity. Checking
        // both ends also covers a shifted boundary inside a sampled minute.
        if allowed(wall(cursor)?) && allowed(wall(next - 1)?) {
            if let Some(gap) = gap_start.take() {
                gaps.push((
                    EpochMs::new(gap)
                        .map_err(|_| DayPlanChangeError::InvalidWindow)?
                        .as_f64(),
                    EpochMs::new(cursor)
                        .map_err(|_| DayPlanChangeError::InvalidWindow)?
                        .as_f64(),
                ));
            }
        } else {
            gap_start.get_or_insert(cursor);
        }
        cursor = next;
    }
    if let Some(gap) = gap_start {
        gaps.push((
            EpochMs::new(gap)
                .map_err(|_| DayPlanChangeError::InvalidWindow)?
                .as_f64(),
            window.end_at_ms,
        ));
    }
    Ok(gaps)
}
