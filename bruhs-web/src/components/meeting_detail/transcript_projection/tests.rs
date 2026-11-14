use super::*;

fn segment(index: usize, speaker: Option<&str>) -> TranscriptSegment {
    TranscriptSegment {
        id: format!("segment-{index}"),
        meeting_id: "meeting".into(),
        speaker_id: speaker.map(str::to_owned),
        start_ms: 10,
        end_ms: 1_000,
        text: format!(
            "Segment {index}: {}",
            "Saved searchable transcript text. ".repeat(7)
        ),
        confidence: 0.9,
        partial: false,
        speaker_confidence: Some(0.8),
    }
}

fn flattened(projection: &TranscriptProjection) -> Vec<TranscriptSegment> {
    projection
        .groups()
        .iter()
        .flat_map(|group| group.rows.get_untracked())
        .map(|row| row.get_untracked())
        .collect()
}

#[test]
fn partial_updates_reuse_other_text_allocations_and_all_row_identities() {
    let mut source = vec![segment(0, Some("me")), segment(1, Some("me"))];
    source[1].partial = true;
    let mut projection = TranscriptProjection::default();
    assert!(projection.sync(&source));
    let group = projection.groups()[0].clone();
    let rows = group.rows.get_untracked();
    let first_text = rows[0].with_untracked(|row| row.text.as_ptr());
    source[1].text.push_str(" final text");
    source[1].partial = false;
    source[1].speaker_confidence = Some(0.95);
    assert!(!projection.sync(&source));
    assert_eq!(projection.groups(), &[group]);
    assert_eq!(projection.groups()[0].rows.get_untracked(), rows);
    assert_eq!(rows[0].with_untracked(|row| row.text.as_ptr()), first_text);
    assert_eq!(flattened(&projection), source);
    assert!(!projection.sync(&source));
}

#[test]
fn appending_a_segment_preserves_existing_group_and_rows() {
    let mut source = vec![segment(0, Some("me")), segment(1, Some("them"))];
    let mut projection = TranscriptProjection::default();
    projection.sync(&source);
    let groups = projection.groups().to_vec();
    let row = groups[1].rows.get_untracked()[0].clone();
    source.push(segment(2, Some("them")));
    assert!(!projection.sync(&source));
    assert_eq!(projection.groups(), groups);
    assert_eq!(projection.groups()[1].rows.get_untracked()[0], row);
    assert_eq!(flattened(&projection), source);
    source.push(segment(3, None));
    assert!(projection.sync(&source));
    assert_eq!(&projection.groups()[..2], groups);
    assert_eq!(flattened(&projection), source);
}

#[test]
fn segment_updates_invalidate_only_the_changed_row_and_leave_group_membership_cached() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let owner = Owner::new();
    owner.with(|| {
        let mut source = vec![segment(0, Some("me")), segment(1, Some("me"))];
        let mut projection = TranscriptProjection::default();
        projection.sync(&source);
        let rows = projection.groups()[0].rows.get_untracked();
        let builds = Arc::new(AtomicUsize::new(0));
        let first_builds = Arc::clone(&builds);
        let first = rows[0].clone();
        let text = Memo::new(move |_| {
            first_builds.fetch_add(1, Ordering::Relaxed);
            first.with(|segment| segment.text.clone())
        });
        let group_builds = Arc::new(AtomicUsize::new(0));
        let count_builds = Arc::clone(&group_builds);
        let group = projection.groups()[0].rows.clone();
        let count = Memo::new(move |_| {
            count_builds.fetch_add(1, Ordering::Relaxed);
            group.with(Vec::len)
        });
        assert_eq!(text.get_untracked(), source[0].text);
        assert_eq!(count.get_untracked(), 2);
        source[1].text.push_str(" partial completion");
        source[1].partial = true;
        projection.sync(&source);
        assert_eq!(text.get_untracked(), source[0].text);
        assert_eq!(count.get_untracked(), 2);
        assert_eq!(builds.load(Ordering::Relaxed), 1);
        assert_eq!(group_builds.load(Ordering::Relaxed), 1);
        source.push(segment(2, Some("me")));
        projection.sync(&source);
        assert_eq!(count.get_untracked(), 3);
        assert_eq!(group_builds.load(Ordering::Relaxed), 2);
        assert_eq!(text.get_untracked(), source[0].text);
        assert_eq!(builds.load(Ordering::Relaxed), 1);
        source[0].text.push_str(" corrected");
        projection.sync(&source);
        assert_eq!(text.get_untracked(), source[0].text);
        assert_eq!(builds.load(Ordering::Relaxed), 2);
    });
}

#[test]
fn deleting_reordering_and_speaker_reassignment_preserve_complete_records() {
    let mut source = [Some("me"), Some("me"), None, Some("them"), Some("me")]
        .into_iter()
        .enumerate()
        .map(|(index, speaker)| segment(index, speaker))
        .collect::<Vec<_>>();
    let mut projection = TranscriptProjection::default();
    projection.sync(&source);
    let retained = projection.groups()[2].clone();
    source.remove(0);
    projection.sync(&source);
    assert_eq!(projection.groups()[2], retained);
    assert_eq!(projection.segments.len(), source.len());
    assert_eq!(flattened(&projection), source);
    source.swap(0, 3);
    source[1].speaker_id = Some("them".into());
    source[1].start_ms = 333;
    source[1].end_ms = 777;
    source[1].confidence = 0.7;
    projection.sync(&source);
    assert_eq!(flattened(&projection), source);
    assert_eq!(projection.groups().len(), 3);
    projection.sync(&[]);
    assert!(projection.groups().is_empty());
    assert!(projection.groups.is_empty());
    assert!(projection.segments.is_empty());
    projection.sync(&source);
    assert_eq!(flattened(&projection), source);
    assert_ne!(projection.groups()[1], retained);
}

#[test]
#[ignore = "diagnostic release timings depend on machine load"]
fn transcript_update_cost() {
    use crate::components::meeting_detail::group_by_speaker;
    use std::{hint::black_box, time::Instant};

    let summarize = |mut samples: Vec<std::time::Duration>| {
        samples.sort_unstable();
        (
            samples[15].as_secs_f64() * 1_000.0,
            samples[29].as_secs_f64() * 1_000.0,
        )
    };
    for count in [100, 1_000, 10_000, 50_000] {
        let source = (0..count)
            .map(|index| segment(index, Some(if index / 10 % 2 == 0 { "me" } else { "them" })))
            .collect::<Vec<_>>();
        let before_cold = summarize(
            (0..31)
                .map(|_| {
                    let start = Instant::now();
                    let memo = group_by_speaker(black_box(&source).iter().cloned());
                    let view = memo.clone();
                    let elapsed = start.elapsed();
                    black_box((memo, view));
                    elapsed
                })
                .collect(),
        );
        let mut before_source = source.clone();
        let mut before_memo = group_by_speaker(before_source.iter().cloned());
        let mut before_view = before_memo.clone();
        let before_update = summarize(
            (0..31)
                .map(|_| {
                    before_source
                        .last_mut()
                        .expect("nonempty fixture")
                        .text
                        .push_str(" token");
                    let start = Instant::now();
                    let next = group_by_speaker(black_box(&before_source).iter().cloned());
                    if next != before_memo {
                        before_memo = next;
                        before_view = before_memo.clone();
                    }
                    black_box(&before_view);
                    start.elapsed()
                })
                .collect(),
        );
        drop((before_source, before_memo, before_view));
        let cold = summarize(
            (0..31)
                .map(|_| {
                    let start = Instant::now();
                    let mut projection = TranscriptProjection::default();
                    projection.sync(black_box(&source));
                    let groups = projection.groups().to_vec();
                    let elapsed = start.elapsed();
                    black_box((projection, groups));
                    elapsed
                })
                .collect(),
        );
        let mut source = source;
        let mut projection = TranscriptProjection::default();
        projection.sync(&source);
        let update = summarize(
            (0..31)
                .map(|_| {
                    source
                        .last_mut()
                        .expect("nonempty fixture")
                        .text
                        .push_str(" token");
                    let start = Instant::now();
                    black_box(projection.sync(black_box(&source)));
                    start.elapsed()
                })
                .collect(),
        );
        println!("transcript / {count} segments: former cold p50 {:.3}, p95 {:.3} ms; stable projection cold p50 {:.3}, p95 {:.3} ms; former one-row update p50 {:.3}, p95 {:.3} ms; stable one-row update p50 {:.3}, p95 {:.3} ms", before_cold.0, before_cold.1, cold.0, cold.1, before_update.0, before_update.1, update.0, update.1);
    }
}
