use bruhs_models::EpochMs;
use std::hint::black_box;
use std::time::{Duration, Instant};

use bruhs_models::{CalendarEvent, EventSource, SchedulingPreferences};

const RECORD_COUNT: usize = 10_000;
const SAMPLE_COUNT: usize = 101;
const MINUTE_MS: i64 = 60_000;
const DAY_MS: i64 = 86_400_000;

/// Bench indices stay below the record count, far inside `i64`.
fn int(index: usize) -> i64 {
    i64::try_from(index).expect("bench index fits in i64")
}

fn fixture_events(from_ms: i64) -> Vec<CalendarEvent> {
    (0..RECORD_COUNT)
        .map(|index| {
            let minute = (int(index) * 3_791) % (30 * 24 * 60);
            let start = from_ms + (minute / 15) * 15 * MINUTE_MS;
            CalendarEvent {
                id: format!("event-{index}"),
                title: format!("Busy {index}"),
                start_at_ms: EpochMs::saturating_from_i64(start).as_f64(),
                end_at_ms: EpochMs::saturating_from_i64(start + 30 * MINUTE_MS).as_f64(),
                all_day: false,
                color: None,
                notes: None,
                created_at: int(index),
                updated_at: int(index),
                source: EventSource::Local,
                google_event_id: None,
                calendar_id: None,
                attendees: Vec::new(),
                organizer: None,
                location: None,
                conferencing: None,
                recurring_event_id: None,
                html_link: None,
            }
        })
        .collect()
}

fn percentile_95(mut samples: Vec<Duration>) -> Duration {
    samples.sort_unstable();
    samples[(samples.len() * 95 / 100).min(samples.len() - 1)]
}

fn asserted_budget() -> Option<Duration> {
    let args = std::env::args().collect::<Vec<_>>();
    let position = args.iter().position(|arg| arg == "--assert-ms")?;
    Some(Duration::from_millis(args.get(position + 1)?.parse().ok()?))
}

fn main() {
    // Unix day 4 is a Monday, keeping the benchmark deterministic.
    let from_ms = 4 * DAY_MS + 8 * 60 * MINUTE_MS;
    let through_ms = from_ms + 30 * DAY_MS;
    let events = fixture_events(from_ms);
    let preferences = SchedulingPreferences::default();
    black_box(preferences.first_available_slot(&events, from_ms, through_ms, 45, 0));

    let samples = (0..SAMPLE_COUNT)
        .map(|_| {
            let started = Instant::now();
            black_box(preferences.first_available_slot(
                black_box(&events),
                black_box(from_ms),
                black_box(through_ms),
                black_box(45),
                black_box(0),
            ));
            started.elapsed()
        })
        .collect::<Vec<_>>();
    let p95 = percentile_95(samples);

    println!(
        "smart scheduling: {RECORD_COUNT} events, p95 {:.3} ms",
        p95.as_secs_f64() * 1_000.0,
    );
    if let Some(budget) = asserted_budget() {
        assert!(
            p95 <= budget,
            "scheduling p95 {:.3} ms exceeded {:.3} ms budget",
            p95.as_secs_f64() * 1_000.0,
            budget.as_secs_f64() * 1_000.0,
        );
    }
}
