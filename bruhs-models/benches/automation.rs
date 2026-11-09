use std::hint::black_box;
use std::time::{Duration, Instant};

use bruhs_models::{AgentAccess, AgentDomain, AutomationState, WorkflowOperation};

const WORKFLOW_COUNT: usize = 10_000;
const SAMPLE_COUNT: usize = 101;

/// Bench indices stay below the record count, far inside `i64`.
fn int(index: usize) -> i64 {
    i64::try_from(index).expect("bench index fits in i64")
}

fn percentile_95(mut samples: Vec<Duration>) -> Duration {
    samples.sort_unstable();
    samples[(samples.len() * 95 / 100).min(samples.len() - 1)]
}

fn main() {
    let mut state = AutomationState::default();
    for index in 0..WORKFLOW_COUNT {
        state.create_workflow(
            format!("Archived workflow {index}"),
            vec![WorkflowOperation::AddCapture {
                content: format!("Archived receipt {index}"),
            }],
            int(index),
        );
    }
    for domain in [
        AgentDomain::Tasks,
        AgentDomain::Calendar,
        AgentDomain::Capture,
    ] {
        state.permissions.set(domain, AgentAccess::Write, true);
    }
    let current = state.create_workflow(
        "Protect focus",
        vec![
            WorkflowOperation::CreateTask {
                title: "Highest-impact task".into(),
            },
            WorkflowOperation::CreateCalendarEvent {
                title: "Protected focus".into(),
                start_at_ms: 1_000.0,
                end_at_ms: 2_000.0,
            },
            WorkflowOperation::AddCapture {
                content: "Focus plan".into(),
            },
        ],
        int(WORKFLOW_COUNT),
    );

    black_box(state.preview(&current, 20_000));
    let samples = (0..SAMPLE_COUNT)
        .map(|index| {
            let started = Instant::now();
            black_box(state.preview(black_box(&current), 20_001 + int(index)));
            started.elapsed()
        })
        .collect::<Vec<_>>();
    let p95 = percentile_95(samples);
    println!(
        "automation: preview and capability check across {WORKFLOW_COUNT} prior workflows, p95 {:.3} ms",
        p95.as_secs_f64() * 1_000.0
    );
    assert!(
        p95 <= Duration::from_millis(16),
        "automation preview exceeded one frame"
    );
}
