use std::hint::black_box;
use std::time::{Duration, Instant};

use bruhs_models::{backlinks_for, KnowledgeDocument, KnowledgeLink, KnowledgeLinkKind};

const RECORD_COUNT: usize = 10_000;
const SAMPLE_COUNT: usize = 101;

/// Bench indices stay below the record count, far inside `i64`.
fn int(index: usize) -> i64 {
    i64::try_from(index).expect("bench index fits in i64")
}

fn fixture_documents() -> Vec<KnowledgeDocument> {
    (0..RECORD_COUNT)
        .map(|index| {
            let mut document = KnowledgeDocument::new(
                format!("Research note {index}"),
                format!("Decision evidence and working notes for initiative {index}."),
                int(index),
            );
            document.id = format!("doc-{index}");
            if index % 7 == 0 {
                document.links.push(KnowledgeLink {
                    kind: KnowledgeLinkKind::Document,
                    target_id: "doc-9999".to_string(),
                    label: "North star".to_string(),
                });
            }
            document
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
    let documents = fixture_documents();
    let _ = black_box(backlinks_for("doc-9999", &documents));

    let lookup_samples = (0..SAMPLE_COUNT)
        .map(|_| {
            let started = Instant::now();
            black_box(backlinks_for(black_box("doc-9999"), black_box(&documents)));
            started.elapsed()
        })
        .collect::<Vec<_>>();
    let lookup_p95 = percentile_95(lookup_samples);

    let mutation_samples = (0..SAMPLE_COUNT)
        .map(|iteration| {
            let mut document = documents[iteration].clone();
            let started = Instant::now();
            black_box(document.revise(
                "Updated knowledge",
                "Updated local content",
                vec!["research".to_string(), "decision".to_string()],
                int(iteration) + int(RECORD_COUNT),
            ));
            started.elapsed()
        })
        .collect::<Vec<_>>();
    let mutation_p95 = percentile_95(mutation_samples);

    println!(
        "knowledge backlinks: {RECORD_COUNT} documents, p95 {:.3} ms; optimistic mutation p95 {:.3} ms",
        lookup_p95.as_secs_f64() * 1_000.0,
        mutation_p95.as_secs_f64() * 1_000.0,
    );

    if let Some(budget) = asserted_budget() {
        assert!(
            lookup_p95 <= budget,
            "knowledge lookup p95 {:.3} ms exceeded {:.3} ms budget",
            lookup_p95.as_secs_f64() * 1_000.0,
            budget.as_secs_f64() * 1_000.0,
        );
        assert!(
            mutation_p95 <= Duration::from_millis(50),
            "knowledge mutation p95 {:.3} ms exceeded 50 ms budget",
            mutation_p95.as_secs_f64() * 1_000.0,
        );
    }
}
