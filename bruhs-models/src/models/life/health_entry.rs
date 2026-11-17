//! What the Life page's health forms turn into, moved from the web Life
//! page's `entries.rs` so both apps read the same fields, check them in the
//! same order and say the same thing: each Log or Add builds a record or
//! says what is wrong, and the typed text stays for correction. A value the
//! record would quietly change (0 minutes, a 101% score) is refused rather
//! than saved as something else. Each form's words are in [`health_copy`].

use super::{Habit, RecoveryLog, SleepLog, WorkoutLog};
use crate::num::ToIntSaturating;

/// Why a health form could not be saved; the text is shown as the notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HealthEntryError {
    /// Workout minutes are not a whole number from 1 to 1440.
    #[error("Enter workout minutes from 1 to 1440.")]
    WorkoutMinutes,
    /// The workout's activity is blank.
    #[error("Name the workout first.")]
    UnnamedWorkout,
    /// Sleep hours are not a number from one minute to a day.
    #[error("Enter sleep hours from one minute to 24 hours.")]
    SleepHours,
    /// Sleep quality is not a whole percent.
    #[error("Enter sleep quality from 0 to 100.")]
    SleepQuality,
    /// The recovery score is not a whole percent.
    #[error("Enter a recovery score from 0 to 100.")]
    RecoveryScore,
    /// The habit's title is blank.
    #[error("Name the habit first.")]
    UnnamedHabit,
}

/// The longest workout or night a form takes: a day, in minutes, as the
/// logs store at most.
const DAY_MINUTES: u16 = 1_440;
/// A new habit's weekly target.
const HABIT_DAYS_PER_WEEK: u8 = 5;

/// A whole percent, 0 to 100.
fn percent(value: &str) -> Option<u8> {
    value
        .trim()
        .parse::<u8>()
        .ok()
        .filter(|percent| *percent <= 100)
}

/// The Workout form: minutes are checked before the activity, as the form
/// always did.
pub fn workout(activity: &str, minutes: &str, now_ms: i64) -> Result<WorkoutLog, HealthEntryError> {
    let minutes = minutes
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|minutes| (1..=DAY_MINUTES).contains(minutes))
        .ok_or(HealthEntryError::WorkoutMinutes)?;
    if activity.trim().is_empty() {
        return Err(HealthEntryError::UnnamedWorkout);
    }
    Ok(WorkoutLog::manual(activity, minutes, now_ms))
}

/// The Sleep & recovery form's Sleep row: hours (decimals allowed), then
/// the quality percent. The night is stored in whole minutes.
pub fn sleep(hours: &str, quality: &str, now_ms: i64) -> Result<SleepLog, HealthEntryError> {
    let hours = hours
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|hours| hours.is_finite() && (1.0 / 60.0..=24.0).contains(hours))
        .ok_or(HealthEntryError::SleepHours)?;
    let quality = percent(quality).ok_or(HealthEntryError::SleepQuality)?;
    let minutes = (hours * 60.0).round().to_u16_saturating();
    Ok(SleepLog::manual(minutes, quality, now_ms))
}

/// The Sleep & recovery form's Recovery row.
pub fn recovery(score: &str, now_ms: i64) -> Result<RecoveryLog, HealthEntryError> {
    let score = percent(score).ok_or(HealthEntryError::RecoveryScore)?;
    Ok(RecoveryLog::manual(score, now_ms))
}

/// The Habits form: a habit with a weekly target of five days.
pub fn habit(title: &str) -> Result<Habit, HealthEntryError> {
    if title.trim().is_empty() {
        return Err(HealthEntryError::UnnamedHabit);
    }
    Ok(Habit::weekly(title, HABIT_DAYS_PER_WEEK))
}

/// A health form field's words: its accessible name and its placeholder.
/// The health fields have no visible caption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthField {
    pub label: &'static str,
    pub placeholder: &'static str,
}

/// App-owned Health copy both surfaces render, so web and desktop match word
/// for word. Follows `docs/ui-standards.md`: short sentences, no dashes.
pub mod health_copy {
    use super::HealthField;

    /// The Health privacy card's line.
    pub const PRIVACY_NOTE: &str =
        "Local logs are excluded from agent context by default. This surface never diagnoses.";

    pub const WORKOUT_ACTIVITY: HealthField = HealthField {
        label: "Workout activity",
        placeholder: "Run",
    };
    pub const WORKOUT_MINUTES: HealthField = HealthField {
        label: "Workout duration minutes",
        placeholder: "Minutes",
    };
    pub const LOG_WORKOUT: &str = "Log";

    pub const SLEEP_HOURS: HealthField = HealthField {
        label: "Sleep hours",
        placeholder: "Hours",
    };
    pub const SLEEP_QUALITY: HealthField = HealthField {
        label: "Sleep quality percent",
        placeholder: "Quality %",
    };
    pub const LOG_SLEEP: &str = "Sleep";
    pub const RECOVERY_SCORE: HealthField = HealthField {
        label: "Recovery score percent",
        placeholder: "Recovery score %",
    };
    pub const LOG_RECOVERY: &str = "Recovery";

    pub const HABIT_TITLE: HealthField = HealthField {
        label: "New habit title",
        placeholder: "Walk after lunch",
    };
    pub const ADD_HABIT: &str = "Add";
}

#[cfg(test)]
mod tests {
    use super::{habit, recovery, sleep, workout, HealthEntryError};

    #[test]
    fn a_workout_needs_whole_minutes_in_a_day_then_an_activity() {
        let run = workout(" Run ", " 30 ", 7).unwrap();
        assert_eq!(
            (
                run.activity.as_str(),
                run.duration_minutes,
                run.occurred_at_ms
            ),
            ("Run", 30, 7)
        );
        assert_eq!(workout("Walk", "1", 0).unwrap().duration_minutes, 1);
        assert_eq!(workout("Walk", "1440", 0).unwrap().duration_minutes, 1_440);
        for minutes in ["0", "1441", "30.5", "-5", "abc", ""] {
            assert_eq!(
                workout("Run", minutes, 0).unwrap_err(),
                HealthEntryError::WorkoutMinutes,
                "{minutes:?}"
            );
        }
        // The minutes are checked first, then the activity.
        assert_eq!(
            workout(" ", "x", 0).unwrap_err(),
            HealthEntryError::WorkoutMinutes
        );
        assert_eq!(
            workout(" ", "30", 0).unwrap_err().to_string(),
            "Name the workout first."
        );
    }

    #[test]
    fn sleep_needs_hours_in_a_day_then_a_quality_percent() {
        let night = sleep("7.5", "80", 9).unwrap();
        assert_eq!(
            (
                night.duration_minutes,
                night.quality_percent,
                night.ended_at_ms
            ),
            (450, 80, 9)
        );
        assert_eq!(sleep(" 24 ", "100", 0).unwrap().duration_minutes, 1_440);
        assert_eq!(sleep("8.5", "0", 0).unwrap().duration_minutes, 510);
        for hours in ["0", "0.001", "24.1", "-1", "NaN", "inf", "abc", ""] {
            assert_eq!(
                sleep(hours, "80", 0).unwrap_err(),
                HealthEntryError::SleepHours,
                "{hours:?}"
            );
        }
        for quality in ["101", "300", "-1", "80.5", "Quality", ""] {
            assert_eq!(
                sleep("8", quality, 0).unwrap_err(),
                HealthEntryError::SleepQuality,
                "{quality:?}"
            );
        }
        // The hours are checked first.
        assert_eq!(
            sleep("x", "x", 0).unwrap_err(),
            HealthEntryError::SleepHours
        );
    }

    #[test]
    fn recovery_is_a_whole_percent() {
        assert_eq!(recovery("64", 3).unwrap().score_percent, 64);
        assert_eq!(recovery(" 0 ", 0).unwrap().score_percent, 0);
        assert_eq!(recovery("100", 0).unwrap().score_percent, 100);
        for score in ["101", "-1", "abc", ""] {
            assert_eq!(
                recovery(score, 0).unwrap_err(),
                HealthEntryError::RecoveryScore,
                "{score:?}"
            );
        }
    }

    #[test]
    fn a_habit_needs_a_title_and_aims_for_five_days() {
        let walk = habit(" Walk after lunch ").unwrap();
        assert_eq!(walk.title, "Walk after lunch");
        assert_eq!(walk.target_days_per_week, 5);
        assert!(walk.completion_days.is_empty() && !walk.archived);
        assert_eq!(habit("  ").unwrap_err(), HealthEntryError::UnnamedHabit);
    }

    /// Each refusal's text is the notice both apps show.
    #[test]
    fn each_refusal_says_what_to_enter() {
        let messages = [
            HealthEntryError::WorkoutMinutes,
            HealthEntryError::UnnamedWorkout,
            HealthEntryError::SleepHours,
            HealthEntryError::SleepQuality,
            HealthEntryError::RecoveryScore,
            HealthEntryError::UnnamedHabit,
        ]
        .map(|error| error.to_string());
        assert_eq!(
            messages,
            [
                "Enter workout minutes from 1 to 1440.",
                "Name the workout first.",
                "Enter sleep hours from one minute to 24 hours.",
                "Enter sleep quality from 0 to 100.",
                "Enter a recovery score from 0 to 100.",
                "Name the habit first.",
            ]
        );
        for message in messages {
            assert!(
                !message.contains(['—', '–', ';']),
                "ui-standards: no dashes in {message:?}"
            );
        }
    }
}
