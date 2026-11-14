//! Deterministic, local action timings. Run with `cargo bench -p bruhs-models --bench actions`.

use std::hint::black_box;
use std::time::{Duration, Instant};

use bruhs_models::logic::{diff::lcs_diff, find::search_texts};
use bruhs_models::{SearchDocument, SearchIndex, SearchKind};

fn measure<T>(label: &str, mut action: impl FnMut() -> T) {
    black_box(action());
    let mut samples = (0..101)
        .map(|_| {
            let start = Instant::now();
            black_box(action());
            start.elapsed()
        })
        .collect::<Vec<Duration>>();
    samples.sort_unstable();
    println!(
        "{label}: p50 {:.3} ms, p95 {:.3} ms",
        samples[50].as_secs_f64() * 1000.0,
        samples[95].as_secs_f64() * 1000.0,
    );
}

fn main() {
    let index = SearchIndex::new((0..10_000).map(|i| {
        SearchDocument::new(
            format!("record-{i}"),
            SearchKind::Task,
            format!("Launch item {i}"),
            "Project",
            "Review research for launch",
            (i * 7919) % 10_000,
        )
    }));
    measure("search 10,000 matches / 50 results", || {
        index.search(black_box("launch"), 50)
    });

    let text = "Review the code and write the release notes. ".repeat(48);
    let sources = (0..500)
        .map(|_| ("message", text.as_str()))
        .collect::<Vec<_>>();
    measure("find absent text / 500 messages", || {
        search_texts(black_box("MISSING"), black_box(&sources))
    });
    measure("find repeated text / 500 messages", || {
        search_texts(black_box("RELEASE"), black_box(&sources))
    });

    let old = (0..1500)
        .map(|i| format!("let value_{i} = {i};\n"))
        .collect::<String>();
    let new = format!("{old}let final_value = 1500;\n");
    measure("diff append / 1,500 lines", || {
        lcs_diff(black_box(&old), black_box(&new))
    });
    measure("diff unchanged / 1,500 lines", || {
        lcs_diff(black_box(&old), black_box(&old))
    });
}
