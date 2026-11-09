use std::hint::black_box;
use std::time::{Duration, Instant};

use bruhs_models::{FinanceBudget, FinanceTransaction, Habit, LifeContext, SleepLog, WorkoutLog};

const RECORD_COUNT: usize = 10_000;
const SAMPLE_COUNT: usize = 101;
const DAY_MS: i64 = 86_400_000;

/// Bench indices stay below the record count, far inside `i64`.
fn int(index: usize) -> i64 {
    i64::try_from(index).expect("bench index fits in i64")
}

fn fixture() -> LifeContext {
    let mut context = LifeContext::default();
    context.finance.budgets = (0..100)
        .map(|index| FinanceBudget::monthly(format!("Category {index}"), 100_000))
        .collect();
    context.finance.transactions = (0..RECORD_COUNT)
        .map(|index| {
            FinanceTransaction::manual(
                format!("Merchant {index}"),
                format!("Category {}", index % 100),
                -1_500,
                40 * DAY_MS - int(index % 30) * DAY_MS,
            )
        })
        .collect();
    context.health.workouts = (0..RECORD_COUNT)
        .map(|index| WorkoutLog::manual("Walk", 30, int(index) * DAY_MS))
        .collect();
    context.health.sleep = (0..RECORD_COUNT)
        .map(|index| SleepLog::manual(450, 80, int(index) * DAY_MS))
        .collect();
    context.health.habits = (0..100)
        .map(|index| Habit::weekly(format!("Habit {index}"), 5))
        .collect();
    context
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
    let context = fixture();
    let now = 40 * DAY_MS;
    black_box(context.insights(&[], now));
    let samples = (0..SAMPLE_COUNT)
        .map(|_| {
            let started = Instant::now();
            black_box(context.insights(black_box(&[]), black_box(now)));
            started.elapsed()
        })
        .collect::<Vec<_>>();
    let insight_p95 = percentile_95(samples);

    let mutation_samples = (0..SAMPLE_COUNT)
        .map(|index| {
            let mut habit = context.health.habits[index % context.health.habits.len()].clone();
            let started = Instant::now();
            habit.toggle_day(now);
            black_box(habit);
            started.elapsed()
        })
        .collect::<Vec<_>>();
    let mutation_p95 = percentile_95(mutation_samples);

    println!(
        "life context: {RECORD_COUNT} transactions + workouts + sleep logs, insight p95 {:.3} ms; optimistic habit mutation p95 {:.3} ms",
        insight_p95.as_secs_f64() * 1_000.0,
        mutation_p95.as_secs_f64() * 1_000.0,
    );
    if let Some(budget) = asserted_budget() {
        assert!(
            insight_p95 <= budget,
            "life insight p95 exceeded frame budget"
        );
        assert!(
            mutation_p95 <= Duration::from_millis(50),
            "life mutation p95 exceeded 50 ms"
        );
    }
}
