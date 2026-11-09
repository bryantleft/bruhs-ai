//! The browser clock and `js_sys::Date` fields as typed numbers.
//!
//! `Date.now()` is an `f64`; [`now`] reads it as an [`EpochMs`]. `js_sys::Date`
//! hands calendar fields back as `u32` but takes them as `i32` (and the year
//! as `u32`). The fields are tiny — a year, a 0-based month, a day of the
//! month, an hour — so [`field`] and [`year_arg`] never clamp a real date;
//! they exist so call sites carry no `as` casts.

use bruhs_models::EpochMs;

/// The browser clock (`Date.now()`).
pub fn now() -> EpochMs {
    EpochMs::saturating_from_f64(js_sys::Date::now())
}

/// The browser clock in whole epoch milliseconds.
pub fn now_ms() -> i64 {
    now().as_millis()
}

/// A `Date` calendar field (`get_full_year`, `get_month`, `get_date`,
/// `get_day`, `get_hours`) as the `i32` that `Date` constructors take.
pub fn field(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// A year as the `u32` that `Date::new_with_year_month_day*` takes; a year
/// before 0 reads as 0.
pub fn year_arg(year: i32) -> u32 {
    u32::try_from(year).unwrap_or(0)
}

/// Local midnight of the date `days` after the one containing `ms` (negative
/// goes back). Steps by calendar date, not 24 hours, so the 23- and 25-hour
/// days of a daylight-saving change still land on the right midnight.
pub fn add_local_days(ms: f64, days: i32) -> f64 {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms));
    js_sys::Date::new_with_year_month_day(
        d.get_full_year(),
        field(d.get_month()),
        field(d.get_date()).saturating_add(days),
    )
    .get_time()
}

/// Local midnight of the date containing `ms`.
pub fn local_day_start(ms: f64) -> f64 {
    add_local_days(ms, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_fields_convert_without_clamping_real_dates() {
        assert_eq!(field(0), 0);
        assert_eq!(field(2026), 2026);
        assert_eq!(field(u32::MAX), i32::MAX);
        assert_eq!(year_arg(2026), 2026);
        assert_eq!(year_arg(-1), 0);
    }
}
