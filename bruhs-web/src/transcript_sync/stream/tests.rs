use super::*;

fn segment(partial: bool) -> TranscriptSegment {
    TranscriptSegment {
        id: "segment".into(),
        meeting_id: "meeting".into(),
        text: "Saved speech 🗒".into(),
        start_ms: 0,
        end_ms: 20,
        speaker_id: None,
        confidence: 0.9,
        partial,
        speaker_confidence: None,
    }
}

#[test]
fn forgotten_chunk_is_not_restored_by_a_later_replay() {
    let mut buffer = StreamBuffer::default();
    let mut records = Vec::new();
    assert!(buffer
        .receive("meeting", segment(true), &mut records)
        .unwrap());
    assert_eq!(buffer.batch("meeting").len(), 1);
    let receipt = buffer.receipt("meeting");
    let (changed, removed) = buffer.forget("meeting", "segment", &mut records);
    assert!(changed);
    assert_eq!(removed.unwrap().text, "Saved speech 🗒");
    assert!(records.is_empty());
    assert!(buffer.batch("meeting").is_empty());
    assert!(!buffer.current("meeting", &receipt));
    assert!(!buffer
        .receive("meeting", segment(false), &mut records)
        .unwrap());
    assert!(records.is_empty());
    assert!(buffer.pending.is_empty());
    let stable = buffer.receipt("meeting");
    let (again, none) = buffer.forget("meeting", "segment", &mut records);
    assert!(!again);
    assert!(none.is_none());
    assert!(buffer.current("meeting", &stable));
}

#[test]
fn repeated_partials_coalesce_without_duplicate_rows_or_pending_writes() {
    let mut buffer = StreamBuffer::default();
    let mut records = Vec::new();
    for index in 0..1000 {
        let mut row = segment(true);
        row.text = format!("Partial {index}");
        assert!(buffer.receive("meeting", row, &mut records).unwrap());
    }
    assert_eq!(records.len(), 1);
    assert_eq!(buffer.batch("meeting").len(), 1);
    assert_eq!(records[0].text, "Partial 999");
    assert_eq!(buffer.pending_bytes, records[0].text.len());
}

#[test]
fn old_acknowledgment_does_not_discard_newer_unsaved_speech() {
    let mut buffer = StreamBuffer::default();
    let mut records = Vec::new();
    buffer
        .receive("meeting", segment(true), &mut records)
        .unwrap();
    let old = buffer.batch("meeting");
    let mut final_row = segment(false);
    final_row.text = "Final words".into();
    buffer
        .receive("meeting", final_row.clone(), &mut records)
        .unwrap();
    buffer.acknowledge(&old);
    assert_eq!(buffer.batch("meeting")[0].as_ref(), &final_row);
    let current = buffer.batch("meeting");
    buffer.acknowledge(&current);
    assert!(buffer.pending.is_empty());
    assert_eq!(buffer.pending_bytes, 0);
}

#[test]
fn failed_batch_stays_available_for_explicit_retry() {
    let mut buffer = StreamBuffer::default();
    let mut records = Vec::new();
    buffer
        .receive("meeting", segment(false), &mut records)
        .unwrap();
    let failed = buffer.batch("meeting");
    // No terminal acknowledgment on error or abort.
    assert_eq!(buffer.batch("meeting"), failed);
    assert!(Arc::ptr_eq(&buffer.batch("meeting")[0], &failed[0]));
}

#[test]
fn finalized_speech_and_newer_speaker_annotations_survive_stream_replay() {
    let mut buffer = StreamBuffer::default();
    let mut canonical = segment(false);
    canonical.speaker_id = Some("corrected-speaker".into());
    let mut records = vec![canonical.clone()];
    assert!(!buffer
        .receive("meeting", segment(false), &mut records)
        .unwrap());
    assert!(!buffer
        .receive("meeting", segment(true), &mut records)
        .unwrap());
    let mut changed = segment(false);
    changed.text = "Rewritten finalized speech".into();
    assert!(buffer.receive("meeting", changed, &mut records).is_err());
    assert_eq!(records, [canonical]);
    assert!(buffer.pending.is_empty());
}

#[test]
fn new_speech_and_replaced_snapshots_invalidate_http_receipts() {
    let mut buffer = StreamBuffer::default();
    let old = buffer.receipt("meeting");
    let other = buffer.receipt("other");
    buffer
        .receive("meeting", segment(false), &mut Vec::new())
        .unwrap();
    assert!(!buffer.current("meeting", &old));
    let after_speech = buffer.receipt("meeting");
    buffer.replaced("meeting");
    assert!(!buffer.current("meeting", &after_speech));
    assert!(buffer.current("other", &other));
}

#[test]
fn foreign_or_invalid_stream_segment_does_not_change_cached_speech() {
    let mut buffer = StreamBuffer::default();
    let mut records = Vec::new();
    assert!(buffer
        .receive("foreign", segment(false), &mut records)
        .is_err());
    let mut invalid = segment(false);
    invalid.confidence = f32::NAN;
    assert!(buffer.receive("meeting", invalid, &mut records).is_err());
    assert!(records.is_empty());
    assert!(buffer.pending.is_empty());
    assert!(transcripts::stream::should_write(
        Some(&segment(false)),
        &TranscriptSegment {
            meeting_id: "foreign".into(),
            ..segment(false)
        }
    )
    .is_err());
}

#[test]
fn corrupt_saved_partial_cannot_be_silently_replaced() {
    let mut corrupt = segment(true);
    corrupt.start_ms = corrupt.end_ms + 1;
    assert!(transcripts::stream::should_write(Some(&corrupt), &segment(false)).is_err());
}

#[test]
fn snapshot_replacement_rebuilds_index_before_next_live_update() {
    let mut buffer = StreamBuffer::default();
    let mut records = Vec::new();
    buffer
        .receive("meeting", segment(true), &mut records)
        .unwrap();
    let first = buffer.batch("meeting");
    buffer.acknowledge(&first);
    let mut inserted = segment(false);
    inserted.id = "earlier".into();
    records.insert(0, inserted.clone());
    buffer.replaced("meeting");
    buffer
        .receive("meeting", segment(false), &mut records)
        .unwrap();
    assert_eq!(records, [inserted, segment(false)]);
}
