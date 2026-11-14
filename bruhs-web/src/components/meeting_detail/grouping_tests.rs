use super::group_by_speaker;
use crate::state::TranscriptSegment;

fn segment(id: usize, speaker: Option<&str>) -> TranscriptSegment {
    TranscriptSegment {
        id: format!("segment-{id}"),
        meeting_id: "meeting".into(),
        speaker_id: speaker.map(str::to_owned),
        start_ms: 0,
        end_ms: 1000,
        text: "A saved transcript segment with searchable text. ".repeat(5),
        confidence: 0.9,
        partial: false,
        speaker_confidence: Some(0.8),
    }
}

#[test]
fn grouping_preserves_order_boundaries_fields_and_owned_text_allocations() {
    let segments = [Some("me"), Some("me"), None, None, Some("them"), Some("me")]
        .into_iter()
        .enumerate()
        .map(|(i, speaker)| segment(i, speaker))
        .collect::<Vec<_>>();
    let expected = segments.clone();
    let pointers = segments
        .iter()
        .map(|segment| segment.text.as_ptr())
        .collect::<Vec<_>>();
    let groups = group_by_speaker(segments);
    assert_eq!(
        groups
            .iter()
            .map(|(speaker, rows)| (speaker.as_deref(), rows.len()))
            .collect::<Vec<_>>(),
        [
            (Some("me"), 2),
            (None, 2),
            (Some("them"), 1),
            (Some("me"), 1)
        ]
    );
    let flattened = groups
        .into_iter()
        .flat_map(|(_, segments)| segments)
        .collect::<Vec<_>>();
    assert_eq!(flattened, expected);
    assert_eq!(
        flattened
            .iter()
            .map(|segment| segment.text.as_ptr())
            .collect::<Vec<_>>(),
        pointers
    );
    assert!(group_by_speaker(Vec::<TranscriptSegment>::new()).is_empty());
}

#[test]
#[ignore = "diagnostic release timing; cargo test -p bruhs-web --release transcript_grouping_cost -- --ignored --nocapture"]
fn transcript_grouping_cost() {
    use std::{hint::black_box, time::Instant};
    let segments = (0..50_000)
        .map(|i| {
            segment(
                i,
                if i % 10 == 0 {
                    Some("me")
                } else {
                    Some("them")
                },
            )
        })
        .collect::<Vec<_>>();
    let measure = |legacy: bool| {
        let mut samples = (0..31)
            .map(|_| {
                let started = Instant::now();
                if legacy {
                    let memo = black_box(segments.clone());
                    let read = black_box(memo.clone());
                    black_box(group_by_speaker(read.iter().cloned()));
                    black_box(memo);
                } else {
                    black_box(group_by_speaker(segments.iter().cloned()));
                }
                started.elapsed()
            })
            .collect::<Vec<_>>();
        samples.sort_unstable();
        (
            samples[15].as_secs_f64() * 1000.0,
            samples[29].as_secs_f64() * 1000.0,
        )
    };
    let before = measure(true);
    let after = measure(false);
    println!("transcript grouping / 50,000 segments: legacy p50 {:.3} ms, p95 {:.3} ms; current p50 {:.3} ms, p95 {:.3} ms", before.0, before.1, after.0, after.1);
}
