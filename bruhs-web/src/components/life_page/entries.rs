//! What the Life page's health forms turn into: each "Log" reads the typed
//! text and builds a record when it holds one. Pure, so the parsing is
//! tested on the host. The finance forms' rules are shared with desktop in
//! `bruhs_models::finance_entry`.

use bruhs_models::num::ToIntSaturating;
use bruhs_models::{Habit, RecoveryLog, SleepLog, WorkoutLog};

/// A workout needs an activity and whole minutes (0–65535).
pub fn workout(activity: String, minutes: &str, now_ms: i64) -> Option<WorkoutLog> {
    let minutes = minutes.parse::<u16>().ok()?;
    (!activity.trim().is_empty()).then(|| WorkoutLog::manual(activity, minutes, now_ms))
}

/// Sleep needs positive hours and a quality percent (0–255, as typed).
pub fn sleep(hours: &str, quality: &str, now_ms: i64) -> Option<SleepLog> {
    let hours = hours.parse::<f64>().ok()?;
    let quality = quality.parse::<u8>().ok()?;
    (hours.is_finite() && hours > 0.0)
        .then(|| SleepLog::manual((hours * 60.0).round().to_u16_saturating(), quality, now_ms))
}

pub fn recovery(score: &str, now_ms: i64) -> Option<RecoveryLog> {
    Some(RecoveryLog::manual(score.parse::<u8>().ok()?, now_ms))
}

/// A habit with a weekly target of five days.
pub fn habit(title: String) -> Option<Habit> {
    (!title.trim().is_empty()).then(|| Habit::weekly(title, 5))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_entries_need_whole_numbers_and_a_name() {
        assert_eq!(workout("Run".into(), "30", 0).unwrap().duration_minutes, 30);
        assert!(workout("Run".into(), "30.5", 0).is_none());
        assert!(workout(" ".into(), "30", 0).is_none());
        let night = sleep("7.5", "80", 0).unwrap();
        assert_eq!((night.duration_minutes, night.quality_percent), (450, 80));
        assert!(sleep("0", "80", 0).is_none());
        assert!(sleep("inf", "80", 0).is_none());
        assert!(sleep("7", "300", 0).is_none());
        assert_eq!(recovery("64", 0).unwrap().score_percent, 64);
        assert!(recovery("-1", 0).is_none());
        assert!(habit("Walk".into()).is_some());
        assert!(habit("  ".into()).is_none());
    }
}
