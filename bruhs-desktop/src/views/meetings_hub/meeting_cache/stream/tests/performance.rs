use super::*;
use std::{hint::black_box, time::Instant};

// The former native stream reducer scanned all text sizes and IDs per event.
// Valid fixture inputs exercise its update path, without pretending it saved data.
fn previous_update(records: &mut Vec<TranscriptSegment>, segment: TranscriptSegment) {
    let bytes: usize = records.iter().map(|record| record.text.len()).sum();
    let replaced = records
        .iter()
        .find(|record| record.id == segment.id)
        .map_or(0, |record| record.text.len());
    assert!(
        bytes
            .saturating_sub(replaced)
            .saturating_add(segment.text.len())
            <= 64 * 1024 * 1024
    );
    if let Some(current) = records.iter_mut().find(|current| current.id == segment.id) {
        assert!(current.partial);
        *current = segment;
    } else {
        records.push(segment);
    }
}

fn micros(samples: &mut [f64]) -> (f64, f64, f64) {
    samples.sort_by(f64::total_cmp);
    (
        samples[samples.len() / 2],
        samples[samples.len() * 95 / 100],
        samples[samples.len() * 99 / 100],
    )
}

#[gpui::test]
fn native_stream_large_history_release_measurement(cx: &mut TestAppContext) {
    let hub = fixture(cx);
    let mut previous = hub.update(cx, |hub, cx| {
        hub.replace_meeting(meeting(), cx);
        for index in 0..90_000 {
            hub.apply_stream_segment(
                "cached-meeting",
                TranscriptSegment {
                    id: format!("segment-{index}"),
                    text: "x".repeat(256),
                    ..segment()
                },
            )
            .unwrap();
        }
        hub.apply_stream_segment(
            "cached-meeting",
            TranscriptSegment {
                id: "partial-tail".into(),
                partial: true,
                ..segment()
            },
        )
        .unwrap();
        hub.segments["cached-meeting"].clone()
    });
    let mut old_samples = Vec::new();
    let mut new_samples = Vec::new();
    for index in 0..500 {
        let record = TranscriptSegment {
            id: "partial-tail".into(),
            text: format!("Received live words {index}"),
            partial: true,
            ..segment()
        };
        let old_record = record.clone();
        let started = Instant::now();
        previous_update(black_box(&mut previous), black_box(old_record));
        old_samples.push(started.elapsed().as_secs_f64() * 1_000_000.0);
        let started = Instant::now();
        hub.update(cx, |hub, cx| {
            let held_snapshot = hub.meeting_cache();
            hub.apply_meeting_event(
                MeetingEvent::Segment {
                    meeting_id: "cached-meeting".into(),
                    segment: record,
                },
                cx,
            );
            black_box(hub.meeting_cache());
            black_box(held_snapshot);
        });
        new_samples.push(started.elapsed().as_secs_f64() * 1_000_000.0);
    }
    let old = micros(&mut old_samples);
    let new = micros(&mut new_samples);
    eprintln!("native_stream_release records=90001 samples=500 baseline_reducer_us p50={:.2} p95={:.2} p99={:.2}; indexed_reducer_plus_shared_snapshot_us p50={:.2} p95={:.2} p99={:.2}", old.0, old.1, old.2, new.0, new.1, new.2);
    assert!(
        new.1 < 16_000.0,
        "Reducer/snapshot budget only; excludes rendering and disk I/O"
    );
    hub.update(cx, |hub, _| {
        assert_eq!(hub.segments["cached-meeting"], previous)
    });
}
