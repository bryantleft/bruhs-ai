//! Pure formatting helpers shared by the chat surface. Host-testable.

use bruhs_models::num::ToIntSaturating;
use bruhs_models::time::{format_relative_ms, RelativeTimeStyle};
use bruhs_models::EpochMs;

/// Compact token count: `950`, `1.2k`, `84k`, `1.3m` (shared with desktop).
pub use bruhs_models::logic::context_meter::format_tokens;

/// Human duration: `0.9s`, `12s`, `1m 12s`, `1h 2m`.
pub fn format_duration_ms(ms: f64) -> String {
    let secs = (ms / 1000.0).max(0.0);
    if secs < 10.0 {
        format!("{secs:.1}s")
    } else if secs < 60.0 {
        format!("{}s", secs.round().to_u64_saturating())
    } else if secs < 3600.0 {
        let m = (secs / 60.0).floor().to_u64_saturating();
        let s = (secs % 60.0).round().to_u64_saturating();
        format!("{m}m {s}s")
    } else {
        let h = (secs / 3600.0).floor().to_u64_saturating();
        let m = ((secs % 3600.0) / 60.0).floor().to_u64_saturating();
        format!("{h}h {m}m")
    }
}

/// Sidebar-style relative time: `now`, `5m`, `3h`, `2d`, `5w`, for browser
/// clock readings ([`format_relative_ms`] in the `Compact` style).
pub fn relative_time(then_ms: f64, now_ms: f64) -> String {
    let at = |ms: f64| EpochMs::from_f64_floor(ms).unwrap_or(EpochMs::UNIX_EPOCH);
    format_relative_ms(at(then_ms), at(now_ms), RelativeTimeStyle::Compact)
}

/// Truncate to `max` characters (not bytes) and append an ellipsis.
pub fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_use_k_suffix_above_thousand() {
        assert_eq!(format_tokens(950), "950");
        assert_eq!(format_tokens(1_200), "1.2k");
        assert_eq!(format_tokens(84_000), "84k");
        assert_eq!(format_tokens(1_260_000), "1.3m");
    }

    #[test]
    fn duration_switches_units() {
        assert_eq!(format_duration_ms(870.0), "0.9s");
        assert_eq!(format_duration_ms(12_400.0), "12s");
        assert_eq!(format_duration_ms(72_000.0), "1m 12s");
        assert_eq!(format_duration_ms(3_725_000.0), "1h 2m");
    }

    #[test]
    fn relative_time_buckets() {
        let now = 1_700_000_000_000.0;
        assert_eq!(relative_time(now - 20_000.0, now), "now");
        assert_eq!(relative_time(now - 5.0 * 60_000.0, now), "5m");
        assert_eq!(relative_time(now - 3.0 * 3_600_000.0, now), "3h");
        assert_eq!(relative_time(now - 2.0 * 86_400_000.0, now), "2d");
        assert_eq!(relative_time(now - 40.0 * 86_400_000.0, now), "5w");
    }

    #[test]
    fn truncate_is_char_safe() {
        assert_eq!(truncate_chars("héllo wörld", 5), "héllo…");
        assert_eq!(truncate_chars("short", 10), "short");
    }
}
