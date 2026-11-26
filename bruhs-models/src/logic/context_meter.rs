//! The context meter both composers show in their options panel: how full
//! the model's context window is after the latest reply. Moved from the web
//! crate's `format.rs` so the desktop meter reads the same numbers.

use crate::num::{ToFloatLossy, ToIntSaturating};

/// Past this share of the window, the meter's ring turns durian.
pub const CONTEXT_METER_CAUTION_PERCENT: f64 = 75.0;

/// Past this share of the window, the meter's ring turns lychee.
pub const CONTEXT_METER_WARN_PERCENT: f64 = 90.0;

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

/// One reading of the meter: the latest reply's context window use against
/// the window of the model that wrote it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContextReading {
    pub used: u64,
    pub window: u64,
}

impl ContextReading {
    /// `used` is the reply's window total (`logic::compact::window_tokens`).
    pub fn new(used: u64, model: &str) -> Self {
        Self {
            used,
            window: context_window_for(model),
        }
    }

    pub fn percent(self) -> f64 {
        context_percentage(self.used, self.window)
    }

    /// Whether the ring shows the caution color (75% to 90%).
    pub fn is_caution(self) -> bool {
        self.percent() >= CONTEXT_METER_CAUTION_PERCENT && !self.is_warning()
    }

    /// Whether the ring shows the warning color.
    pub fn is_warning(self) -> bool {
        self.percent() > CONTEXT_METER_WARN_PERCENT
    }

    /// The ring's accessible name and tooltip.
    pub fn label(self) -> String {
        format!(
            "Context window, {}%, {} of {}",
            self.percent().round().to_u64_saturating(),
            format_tokens(self.used),
            format_tokens(self.window)
        )
    }

    /// The text beside the ring in the options panel.
    pub fn used_label(self) -> String {
        format!("{:.0}% used", self.percent())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_read_compactly() {
        assert_eq!(format_tokens(950), "950");
        assert_eq!(format_tokens(1_200), "1.2k");
        assert_eq!(format_tokens(84_000), "84k");
        assert_eq!(format_tokens(1_300_000), "1.3m");
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
        assert_eq!(context_window_for(""), 200_000);
    }

    #[test]
    fn a_reading_names_its_share_and_warns_past_ninety_percent() {
        let reading = ContextReading::new(84_000, "claude-sonnet-4-6");
        assert_eq!(reading.label(), "Context window, 42%, 84k of 200k");
        assert_eq!(reading.used_label(), "42% used");
        assert!(!reading.is_caution());
        assert!(!reading.is_warning());
        let caution = ContextReading::new(160_000, "claude-sonnet-4-6");
        assert!(caution.is_caution());
        assert!(!caution.is_warning());
        let warning = ContextReading::new(190_000, "claude-sonnet-4-6");
        assert!(!warning.is_caution());
        assert!(warning.is_warning());
        assert!(!ContextReading::new(180_000, "claude-sonnet-4-6").is_warning());
        assert!(ContextReading::new(180_000, "claude-sonnet-4-6").is_caution());
    }
}
