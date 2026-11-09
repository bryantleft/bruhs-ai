use std::hint::black_box;
use std::time::{Duration, Instant};

use bruhs_models::{propose_day_plan, Priority, WorkItem, WorkItemStatus};

const RECORD_COUNT: usize = 10_000;
const SAMPLE_COUNT: usize = 101;

/// Bench indices stay below the record count, far inside `i64`.
fn int(index: usize) -> i64 {
    i64::try_from(index).expect("bench index fits in i64")
}

fn fixture_tasks() -> Vec<WorkItem> {
    (0..RECORD_COUNT)
        .map(|index| {
            let mut item = WorkItem::new(format!("Plan task {index}"));
            item.id = format!("task-{index}");
            item.status = if index % 5 == 0 {
                WorkItemStatus::InProgress
            } else {
                WorkItemStatus::Todo
            };
            item.priority = match index % 5 {
                0 => Priority::Urgent,
                1 => Priority::High,
                2 => Priority::Medium,
                3 => Priority::Low,
                _ => Priority::None,
            };
            item.planning.estimate_minutes =
                Some(30 + u16::try_from(index % 4).expect("0..4 fits in u16") * 15);
            item.updated_at = int(index);
            item
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
    let milliseconds = args.get(position + 1)?.parse::<u64>().ok()?;
    Some(Duration::from_millis(milliseconds))
}

fn main() {
    let tasks = fixture_tasks();
    let day_start_ms = 1_800_000_000_000.0;
    let now_ms = day_start_ms + 9.0 * 60.0 * 60_000.0;
    let _ = black_box(propose_day_plan(now_ms, day_start_ms, &tasks, &[]));

    let samples = (0..SAMPLE_COUNT)
        .map(|_| {
            let started = Instant::now();
            let proposal = propose_day_plan(
                black_box(now_ms),
                black_box(day_start_ms),
                black_box(&tasks),
                black_box(&[]),
            );
            black_box(proposal);
            started.elapsed()
        })
        .collect::<Vec<_>>();
    let p95 = percentile_95(samples);

    println!(
        "day planning: {RECORD_COUNT} tasks, p95 {:.3} ms",
        p95.as_secs_f64() * 1_000.0
    );

    if let Some(budget) = asserted_budget() {
        assert!(
            p95 <= budget,
            "planning p95 {:.3} ms exceeded {:.3} ms budget",
            p95.as_secs_f64() * 1_000.0,
            budget.as_secs_f64() * 1_000.0,
        );
    }
}
