//! Millisecond timestamps and the pure time helpers every surface shares.
//!
//! [`EpochMs`] is the one timestamp type for new wire and storage fields. It
//! holds whole milliseconds since the Unix epoch and is confined to the range
//! an `f64` represents exactly (`±(2^53 - 1)`, JavaScript's
//! `Number.MAX_SAFE_INTEGER`), so converting to and from the browser's `f64`
//! clock never loses precision.

use std::fmt;

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

use crate::num::ToIntSaturating;

/// Largest magnitude an [`EpochMs`] may have: `2^53 - 1`, the largest integer
/// an `f64` represents exactly. About 285,000 years either side of 1970.
pub const MAX_EPOCH_MS: i64 = 9_007_199_254_740_991;

/// [`MAX_EPOCH_MS`] as an `f64` (exact).
const MAX_EPOCH_MS_F64: f64 = 9_007_199_254_740_991.0;

/// Why a number could not become an [`EpochMs`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EpochMsError {
    /// `NaN` or an infinity.
    #[error("timestamp is not a finite number")]
    NotFinite,
    /// Outside `±MAX_EPOCH_MS`, where `f64` can no longer hold every value.
    #[error("timestamp is outside the range an f64 holds exactly")]
    OutOfRange,
    /// Has a fractional millisecond; [`EpochMs::from_f64_floor`] accepts it.
    #[error("timestamp has a fractional millisecond")]
    Fractional,
}

/// Whole milliseconds since the Unix epoch, within `±MAX_EPOCH_MS`.
///
/// Serializes as a JSON integer. Deserializes from an integer or an integral
/// float (`1700000000000.0`, which `serde_json` writes for an `f64`); a
/// fractional float is floored, matching how the surfaces used to truncate
/// `f64` clocks into `i64` fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct EpochMs(i64);

impl EpochMs {
    /// 1970-01-01T00:00:00Z.
    pub const UNIX_EPOCH: Self = Self(0);

    /// The system clock. Native only: `SystemTime::now()` panics on
    /// `wasm32-unknown-unknown`, where callers read the browser clock. A clock
    /// set before 1970 reads as the epoch.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn now() -> Self {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| {
                i64::try_from(elapsed.as_millis()).unwrap_or(MAX_EPOCH_MS)
            });
        Self(ms.min(MAX_EPOCH_MS))
    }

    /// `ms` milliseconds since the epoch.
    pub const fn new(ms: i64) -> Result<Self, EpochMsError> {
        if ms > MAX_EPOCH_MS || ms < -MAX_EPOCH_MS {
            Err(EpochMsError::OutOfRange)
        } else {
            Ok(Self(ms))
        }
    }

    /// A JavaScript clock reading (`Date.now()`), flooring any fractional
    /// millisecond. Rejects `NaN`, infinities and out-of-range values.
    pub fn from_f64_floor(ms: f64) -> Result<Self, EpochMsError> {
        if !ms.is_finite() {
            return Err(EpochMsError::NotFinite);
        }
        let floored = ms.floor();
        if floored.abs() > MAX_EPOCH_MS_F64 {
            return Err(EpochMsError::OutOfRange);
        }
        // In range and integral, so the conversion is exact.
        Ok(Self(floored.to_i64_saturating()))
    }

    /// An `f64` clock reading or timestamp field that must become a
    /// timestamp no matter what: floors any fractional millisecond, clamps
    /// to `±MAX_EPOCH_MS`, and reads `NaN` as the epoch.
    ///
    /// Same result as the `ms as i64` it replaces for every non-negative
    /// timestamp below 2^53 (all real clocks); a pre-1970 fraction floors
    /// rather than truncating toward zero.
    pub fn saturating_from_f64(ms: f64) -> Self {
        if ms.is_nan() {
            return Self::UNIX_EPOCH;
        }
        let clamped = ms.floor().clamp(-MAX_EPOCH_MS_F64, MAX_EPOCH_MS_F64);
        Self(clamped.to_i64_saturating())
    }

    /// An `i64` epoch-milliseconds value (a chrono timestamp, a stored
    /// field) that must become a timestamp no matter what: clamps to
    /// `±MAX_EPOCH_MS`, which every real clock reading is far inside.
    pub const fn saturating_from_i64(ms: i64) -> Self {
        if ms > MAX_EPOCH_MS {
            Self(MAX_EPOCH_MS)
        } else if ms < -MAX_EPOCH_MS {
            Self(-MAX_EPOCH_MS)
        } else {
            Self(ms)
        }
    }

    /// Milliseconds since the epoch.
    pub const fn as_millis(self) -> i64 {
        self.0
    }

    /// The same instant as a JavaScript `f64`; exact for every `EpochMs`.
    #[expect(
        clippy::cast_precision_loss,
        reason = "an EpochMs is confined to ±(2^53 - 1), which f64 holds exactly"
    )]
    pub const fn as_f64(self) -> f64 {
        self.0 as f64
    }

    /// `self - earlier` in milliseconds. Cannot overflow: both sides are
    /// within `±2^53`.
    pub const fn millis_since(self, earlier: Self) -> i64 {
        self.0 - earlier.0
    }
}

impl TryFrom<i64> for EpochMs {
    type Error = EpochMsError;

    fn try_from(ms: i64) -> Result<Self, Self::Error> {
        Self::new(ms)
    }
}

/// Lossless: rejects `NaN`, infinities, out-of-range values and fractional
/// milliseconds. Use [`EpochMs::from_f64_floor`] to drop the fraction.
impl TryFrom<f64> for EpochMs {
    type Error = EpochMsError;

    fn try_from(ms: f64) -> Result<Self, Self::Error> {
        if ms.is_finite() && ms.fract() != 0.0 {
            return Err(EpochMsError::Fractional);
        }
        Self::from_f64_floor(ms)
    }
}

impl From<EpochMs> for i64 {
    fn from(ms: EpochMs) -> Self {
        ms.0
    }
}

impl From<EpochMs> for f64 {
    fn from(ms: EpochMs) -> Self {
        ms.as_f64()
    }
}

impl fmt::Display for EpochMs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Serialize for EpochMs {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_i64(self.0)
    }
}

impl<'de> Deserialize<'de> for EpochMs {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(EpochMsVisitor { legacy: false })
    }
}

/// Reads a timestamp that older records stored as a string. Accepts every
/// shape [`EpochMs`] does, plus numeric strings (`"1700000000000"`, written
/// by the web's `Date.now().to_string()`) and any other string (`"now"`,
/// written by the desktop), which reads as [`EpochMs::UNIX_EPOCH`] so the
/// record sorts as oldest, the way the web sidebar already treated it.
///
/// Use with `#[serde(deserialize_with = "bruhs_models::time::deserialize_legacy")]`.
pub fn deserialize_legacy<'de, D: Deserializer<'de>>(deserializer: D) -> Result<EpochMs, D::Error> {
    deserializer.deserialize_any(EpochMsVisitor { legacy: true })
}

struct EpochMsVisitor {
    legacy: bool,
}

impl<'de> de::Visitor<'de> for EpochMsVisitor {
    type Value = EpochMs;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.legacy {
            f.write_str("epoch milliseconds as a number or string")
        } else {
            f.write_str("epoch milliseconds as a number")
        }
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<EpochMs, E> {
        EpochMs::new(v).map_err(E::custom)
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<EpochMs, E> {
        i64::try_from(v)
            .map_err(|_| E::custom(EpochMsError::OutOfRange))
            .and_then(|v| self.visit_i64(v))
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<EpochMs, E> {
        EpochMs::from_f64_floor(v).map_err(E::custom)
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<EpochMs, E> {
        if !self.legacy {
            return Err(E::invalid_type(de::Unexpected::Str(v), &self));
        }
        Ok(v.trim()
            .parse::<f64>()
            .ok()
            .and_then(|ms| EpochMs::from_f64_floor(ms).ok())
            .unwrap_or(EpochMs::UNIX_EPOCH))
    }
}

/// Number of days in `month` (1 = January … 12 = December) of the proleptic
/// Gregorian `year`; `None` for a month outside `1..=12`.
pub const fn days_in_month(year: i32, month: u32) -> Option<u32> {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => Some(31),
        4 | 6 | 9 | 11 => Some(30),
        2 if is_leap_year(year) => Some(29),
        2 => Some(28),
        _ => None,
    }
}

/// Gregorian leap-year rule.
pub const fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Which relative-time vocabulary [`format_relative_ms`] speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelativeTimeStyle {
    /// Sidebar cards: `now`, `5m`, `3h`, `2d`, `5w`.
    Compact,
    /// Background-task rows: `now`, `5m`, `3h`, `40d` (no weeks).
    CompactDays,
    /// Sync status: `just now`, `12s ago`, `5m ago`, `3h ago`, `2d ago`.
    Ago,
}

/// How long before `now` the instant `then` was, in `style`. Instants in the
/// future read as `now` / `just now`. Every unit is floored.
pub fn format_relative_ms(then: EpochMs, now: EpochMs, style: RelativeTimeStyle) -> String {
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    const WEEK: i64 = 7 * DAY;

    let secs = now.millis_since(then) / 1000;
    match style {
        RelativeTimeStyle::Compact | RelativeTimeStyle::CompactDays => {
            let secs = secs.max(0);
            if secs < MINUTE {
                "now".to_string()
            } else if secs < HOUR {
                format!("{}m", secs / MINUTE)
            } else if secs < DAY {
                format!("{}h", secs / HOUR)
            } else if style == RelativeTimeStyle::CompactDays || secs < WEEK {
                format!("{}d", secs / DAY)
            } else {
                format!("{}w", secs / WEEK)
            }
        }
        RelativeTimeStyle::Ago => {
            if secs < 5 {
                "just now".to_string()
            } else if secs < MINUTE {
                format!("{secs}s ago")
            } else if secs < HOUR {
                format!("{}m ago", secs / MINUTE)
            } else if secs < DAY {
                format!("{}h ago", secs / HOUR)
            } else {
                format!("{}d ago", secs / DAY)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(v: i64) -> EpochMs {
        EpochMs::new(v).unwrap()
    }

    #[test]
    fn f64_conversions_are_lossless_and_reject_non_finite() {
        assert_eq!(
            EpochMs::try_from(1_700_000_000_123.0),
            Ok(ms(1_700_000_000_123))
        );
        assert_eq!(ms(1_700_000_000_123).as_f64(), 1_700_000_000_123.0);
        assert_eq!(EpochMs::try_from(f64::NAN), Err(EpochMsError::NotFinite));
        assert_eq!(
            EpochMs::try_from(f64::INFINITY),
            Err(EpochMsError::NotFinite)
        );
        assert_eq!(
            EpochMs::try_from(f64::NEG_INFINITY),
            Err(EpochMsError::NotFinite)
        );
        assert_eq!(EpochMs::try_from(1.5), Err(EpochMsError::Fractional));
        assert_eq!(EpochMs::from_f64_floor(1.5), Ok(ms(1)));
        assert_eq!(EpochMs::from_f64_floor(-1.5), Ok(ms(-2)));
        assert_eq!(
            EpochMs::try_from(2f64.powi(53)),
            Err(EpochMsError::OutOfRange)
        );
        assert_eq!(
            EpochMs::new(MAX_EPOCH_MS + 1),
            Err(EpochMsError::OutOfRange)
        );
        assert_eq!(
            EpochMs::new(-MAX_EPOCH_MS).map(EpochMs::as_f64),
            Ok(-MAX_EPOCH_MS_F64)
        );
        for v in [0, 1, -1, 1_700_000_000_000, MAX_EPOCH_MS, -MAX_EPOCH_MS] {
            let t = ms(v);
            assert_eq!(EpochMs::try_from(t.as_f64()), Ok(t), "{v}");
            assert_eq!(i64::from(t), v);
        }
    }

    #[test]
    fn saturating_from_f64_floors_clamps_and_reads_nan_as_the_epoch() {
        let f = |v: f64| EpochMs::saturating_from_f64(v).as_millis();
        // Real clocks: identical to the `as i64` this replaces.
        for v in [0.0, 1.0, 1_700_000_000_123.0, 1_700_000_000_123.9] {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "the old behavior under test"
            )]
            let old = v as i64;
            assert_eq!(f(v), old, "{v}");
        }
        assert_eq!(f(-1.5), -2, "pre-1970 fractions floor");
        assert_eq!(f(f64::NAN), 0);
        assert_eq!(f(f64::INFINITY), MAX_EPOCH_MS);
        assert_eq!(f(f64::NEG_INFINITY), -MAX_EPOCH_MS);
        assert_eq!(f(1e300), MAX_EPOCH_MS);
    }

    #[test]
    fn saturating_from_i64_keeps_real_timestamps_and_clamps_the_rest() {
        let f = |v: i64| EpochMs::saturating_from_i64(v);
        for v in [0, 1, -1, 1_700_000_000_123, MAX_EPOCH_MS, -MAX_EPOCH_MS] {
            assert_eq!(f(v).as_millis(), v);
            assert_eq!(f(v).as_f64(), ms(v).as_f64());
        }
        assert_eq!(f(MAX_EPOCH_MS + 1).as_millis(), MAX_EPOCH_MS);
        assert_eq!(f(i64::MAX).as_millis(), MAX_EPOCH_MS);
        assert_eq!(f(i64::MIN).as_millis(), -MAX_EPOCH_MS);
        assert_eq!(f(i64::MAX).as_f64(), MAX_EPOCH_MS_F64);
    }

    #[test]
    fn now_reads_the_system_clock() {
        let before = i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap();
        let now = EpochMs::now().as_millis();
        assert!(now >= before && now - before < 60_000, "{before} {now}");
    }

    #[test]
    fn serde_writes_integers_and_reads_numbers() {
        assert_eq!(
            serde_json::to_string(&ms(1_700_000_000_000)).unwrap(),
            "1700000000000"
        );
        for (json, want) in [
            ("1700000000000", 1_700_000_000_000),
            ("1700000000000.0", 1_700_000_000_000),
            ("1700000000000.9", 1_700_000_000_000),
            ("-5", -5),
        ] {
            assert_eq!(
                serde_json::from_str::<EpochMs>(json).unwrap(),
                ms(want),
                "{json}"
            );
        }
        assert!(serde_json::from_str::<EpochMs>("\"1700000000000\"").is_err());
        assert!(serde_json::from_str::<EpochMs>("9007199254740992").is_err());
        assert!(serde_json::from_str::<EpochMs>("18446744073709551615").is_err());
    }

    #[test]
    fn legacy_strings_read_as_numbers_or_the_epoch() {
        #[derive(Deserialize)]
        struct Row {
            #[serde(deserialize_with = "deserialize_legacy")]
            at: EpochMs,
        }
        for (json, want) in [
            (r#"{"at":1700000000000}"#, 1_700_000_000_000),
            (r#"{"at":1700000000000.0}"#, 1_700_000_000_000),
            (r#"{"at":"1700000000000"}"#, 1_700_000_000_000),
            (r#"{"at":" 1700000000000.4 "}"#, 1_700_000_000_000),
            (r#"{"at":"now"}"#, 0),
            (r#"{"at":""}"#, 0),
            (r#"{"at":"NaN"}"#, 0),
        ] {
            let row: Row = serde_json::from_str(json).unwrap();
            assert_eq!(row.at, ms(want), "{json}");
        }
        assert!(serde_json::from_str::<Row>(r#"{"at":null}"#).is_err());
    }

    #[test]
    fn days_in_month_handles_leap_years() {
        // Desktop `date_picker_ui` cases.
        assert_eq!(days_in_month(2024, 2), Some(29));
        assert_eq!(days_in_month(2026, 2), Some(28));
        assert_eq!(days_in_month(2026, 12), Some(31));
        assert_eq!(days_in_month(1900, 2), Some(28));
        assert_eq!(days_in_month(2000, 2), Some(29));
        assert_eq!(days_in_month(2026, 4), Some(30));
        assert_eq!(days_in_month(2026, 0), None);
        assert_eq!(days_in_month(2026, 13), None);
    }

    #[test]
    fn compact_style_matches_web_sidebar() {
        // Web `format::relative_time` cases.
        let now = ms(1_700_000_000_000);
        let ago = |d: i64| ms(now.as_millis() - d);
        let f = |then| format_relative_ms(then, now, RelativeTimeStyle::Compact);
        assert_eq!(f(ago(20_000)), "now");
        assert_eq!(f(ago(5 * 60_000)), "5m");
        assert_eq!(f(ago(3 * 3_600_000)), "3h");
        assert_eq!(f(ago(2 * 86_400_000)), "2d");
        assert_eq!(f(ago(40 * 86_400_000)), "5w");
        // Desktop `sidebar_order::relative_age` cases.
        assert_eq!(f(ago(5_000)), "now");
        assert_eq!(f(ago(15 * 86_400_000)), "2w");
        assert_eq!(f(ms(now.as_millis() + 5_000)), "now");
        assert_eq!(f(ago(59_999)), "now");
        assert_eq!(f(ago(60_000)), "1m");
    }

    #[test]
    fn compact_days_style_matches_web_tasks_panel() {
        let now = ms(1_700_000_000_000);
        let ago = |d: i64| ms(now.as_millis() - d);
        let f = |then| format_relative_ms(then, now, RelativeTimeStyle::CompactDays);
        assert_eq!(f(ago(30_000)), "now");
        assert_eq!(f(ago(3 * 60_000)), "3m");
        assert_eq!(f(ago(2 * 3_600_000)), "2h");
        assert_eq!(f(ago(40 * 86_400_000)), "40d");
        assert_eq!(f(ms(now.as_millis() + 90_000)), "now");
    }

    #[test]
    fn ago_style_matches_web_google_calendar_settings() {
        let f = |then: i64, now: i64| format_relative_ms(ms(then), ms(now), RelativeTimeStyle::Ago);
        // Desktop `meetings_hub::format_relative_ms` cases.
        assert_eq!(f(0, 3_000), "just now");
        assert_eq!(f(0, 90_000), "1m ago");
        assert_eq!(f(0, 12_000), "12s ago");
        assert_eq!(f(0, 2 * 3_600_000), "2h ago");
        assert_eq!(f(0, 3 * 86_400_000), "3d ago");
        assert_eq!(f(10_000, 0), "just now");
    }
}
