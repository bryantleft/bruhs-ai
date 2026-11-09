//! Pure formatting helpers shared by the chat surface. Host-testable.

use bruhs_models::num::{ToFloatLossy, ToIntSaturating};
use bruhs_models::time::{format_relative_ms, RelativeTimeStyle};
use bruhs_models::EpochMs;

/// Compact token count: `950`, `1.2k`, `84k`, `1.3m`.
pub fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}m", n.to_f64_lossy() / 1_000_000.0)
    } else if n >= 10_000 {
        format!("{}k", n / 1000)
    } else if n >= 1_000 {
        format!("{:.1}k", n.to_f64_lossy() / 1000.0)
    } else {
        n.to_string()
    }
}

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

/// Percentage of the context window used, clamped to 0..=100.
pub fn context_percentage(used: u64, window: u64) -> f64 {
    if window == 0 {
        return 0.0;
    }
    ((used.to_f64_lossy() / window.to_f64_lossy()) * 100.0).clamp(0.0, 100.0)
}

/// Best-known context window per provider family. Claude models advertise
/// 200k; GPT-5 models 400k; the installed GPT-6.1 Sol catalog advertises 500k.
pub fn context_window_for(model: &str) -> u64 {
    if model == "gpt-6.1-sol" {
        500_000
    } else if model.starts_with("gpt-") {
        400_000
    } else {
        200_000
    }
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
    fn context_percentage_clamps() {
        assert_eq!(context_percentage(50_000, 200_000), 25.0);
        assert_eq!(context_percentage(300_000, 200_000), 100.0);
        assert_eq!(context_percentage(1, 0), 0.0);
    }

    #[test]
    fn context_window_by_provider() {
        assert_eq!(context_window_for("claude-sonnet-4-6"), 200_000);
        assert_eq!(context_window_for("gpt-5.5"), 400_000);
        assert_eq!(context_window_for("gpt-6.1-sol"), 500_000);
    }

    #[test]
    fn truncate_is_char_safe() {
        assert_eq!(truncate_chars("héllo wörld", 5), "héllo…");
        assert_eq!(truncate_chars("short", 10), "short");
    }
}
