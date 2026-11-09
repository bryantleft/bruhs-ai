use std::hint::black_box;
use std::time::{Duration, Instant};

use bruhs_models::{SearchDocument, SearchIndex, SearchKind};

const RECORD_COUNT: usize = 10_000;
const SAMPLE_COUNT: usize = 101;

/// Bench indices stay below the record count, far inside `i64`.
fn int(index: usize) -> i64 {
    i64::try_from(index).expect("bench index fits in i64")
}

fn fixture_index() -> SearchIndex {
    SearchIndex::new((0..RECORD_COUNT).map(|index| {
        SearchDocument::new(
            format!("task-{index}"),
            SearchKind::Task,
            format!("Launch follow-up {index}"),
            format!("Project {}", index % 50),
            format!("Review research and prepare the decision memo for account {index}"),
            int(index),
        )
    }))
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
    let index = fixture_index();
    let _ = black_box(index.search("launch 9999", 50));

    let samples = (0..SAMPLE_COUNT)
        .map(|iteration| {
            let query = format!("launch {}", (iteration * 97) % RECORD_COUNT);
            let started = Instant::now();
            let hits = black_box(index.search(black_box(&query), 50));
            black_box(hits);
            started.elapsed()
        })
        .collect::<Vec<_>>();
    let p95 = percentile_95(samples);

    println!(
        "universal search: {RECORD_COUNT} records, p95 {:.3} ms",
        p95.as_secs_f64() * 1_000.0
    );

    if let Some(budget) = asserted_budget() {
        assert!(
            p95 <= budget,
            "search p95 {:.3} ms exceeded {:.3} ms budget",
            p95.as_secs_f64() * 1_000.0,
            budget.as_secs_f64() * 1_000.0,
        );
    }
}
