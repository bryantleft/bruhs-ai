use super::*;

fn restored_row(id: &str, index: usize, partial: bool) -> TranscriptSegment {
    let start = u32::try_from(index % 11).unwrap() * 10;
    TranscriptSegment {
        id: id.into(),
        start_ms: start,
        end_ms: start + 1,
        text: format!("Words ΟΣ Café 👩🏽‍💻 {index}"),
        partial,
        ..segment()
    }
}

#[test]
fn borrowed_merge_matches_owned_reference_for_unsorted_fragment_overlays() {
    for count in [0, 1, 129, 257] {
        let mut base = (0..count)
            .map(|i| restored_row(&format!("b{i}"), i, true))
            .collect::<Vec<_>>();
        base.reverse();
        for overlap in [0, count / 2, count] {
            let mut fragments = StreamFragments::default();
            for index in (0..overlap).rev() {
                let mut updated = restored_row(&format!("b{index}"), index + 2, true);
                updated.text.push_str(" updated");
                fragments.update(None, updated).unwrap();
            }
            for index in (0..count / 3).rev() {
                fragments
                    .update(None, restored_row(&format!("a{index}"), index, true))
                    .unwrap();
            }
            let before = base.clone();
            let saved = fragments.clone();
            assert_eq!(
                fragments.merge_borrowed(&base),
                fragments.merge(base.clone())
            );
            assert_eq!(base, before);
            assert!(fragments.same_snapshot(&saved));
            if count > 0 {
                // The original merge's last-fragment-wins rule is retained too.
                let mut final_fragment = restored_row("b0", 0, false);
                final_fragment.text = "Last fragment".into();
                fragments.update(None, final_fragment).unwrap();
                assert_eq!(
                    fragments.merge_borrowed(&base),
                    fragments.merge(base.clone())
                );
            }
        }
    }
}

#[gpui::test]
fn borrowed_restore_preserves_order_indexes_cache_and_receipts(cx: &mut TestAppContext) {
    let hub = fixture(cx);
    let mut cache = MeetingCache::default();
    let ids = [
        "plain",
        "overlay",
        "empty-fragments",
        "fragments-only",
        "no-history",
    ];
    for id in ids {
        let mut record = meeting();
        record.id = id.into();
        cache.records.insert(id.into(), Arc::new(record));
        if !matches!(id, "fragments-only" | "no-history") {
            let mut rows = (0..130)
                .map(|index| TranscriptSegment {
                    meeting_id: id.into(),
                    ..restored_row(&format!("b{index}"), index, true)
                })
                .collect::<Vec<_>>();
            rows.reverse();
            cache.transcripts.insert(id.into(), Arc::new(rows));
        }
        if matches!(id, "overlay" | "empty-fragments" | "fragments-only") {
            let mut fragments = StreamFragments::default();
            if id != "empty-fragments" {
                for (key, index) in [("b129", 129), ("appended", 130)] {
                    fragments
                        .update(
                            None,
                            TranscriptSegment {
                                meeting_id: id.into(),
                                ..restored_row(key, index, true)
                            },
                        )
                        .unwrap();
                }
            }
            cache.fragments.insert(id.into(), fragments);
        }
    }
    let cache: MeetingCache = serde_json::from_slice(&serde_json::to_vec(&cache).unwrap()).unwrap();
    hub.update(cx, |hub, cx| {
        hub.restore_meetings(cache.clone());
        assert_eq!(hub.meeting_cache(), cache);
        assert_eq!(hub.meetings.len(), 5);
        assert_eq!(hub.segments.len(), 4);
        assert_eq!(hub.search_cached.len(), 1);
        assert!(hub.search_cached.contains("plain"));
        assert!(hub.live_meeting_id.is_none());
        assert!(hub.capture_status.recording().is_none());
        for id in ids {
            let base = cache
                .transcripts
                .get(id)
                .map_or_else(Vec::new, |r| r.as_ref().clone());
            let expected = match cache.fragments.get(id) {
                Some(fragments) => fragments.merge(base),
                None => base,
            };
            if id == "no-history" {
                assert!(!hub.segments.contains_key(id));
                assert!(!hub.transcript_indexes.contains_key(id));
                continue;
            }
            assert_eq!(hub.segments[id], expected, "{id}");
            let index = &hub.transcript_indexes[id];
            assert_eq!(
                index.text_bytes,
                expected.iter().map(|r| r.text.len()).sum::<usize>()
            );
            assert_eq!(index.positions.len(), expected.len());
            for (position, row) in expected.iter().enumerate() {
                assert_eq!(index.positions[&row.id], position);
            }
            let fragment_positions = cache
                .fragments
                .get(id)
                .into_iter()
                .flat_map(StreamFragments::iter)
                .enumerate()
                .map(|(i, row)| (row.id.clone(), i))
                .collect::<HashMap<_, _>>();
            assert_eq!(index.fragment_positions, fragment_positions);
        }
        let receipt = hub.transcript_read_receipt("overlay");
        assert!(hub.transcript_receipt_current("overlay", &receipt));
        let next = TranscriptSegment {
            meeting_id: "overlay".into(),
            text: "Streamed after restore".into(),
            ..restored_row("b129", 129, true)
        };
        hub.apply_meeting_event(
            MeetingEvent::Segment {
                meeting_id: "overlay".into(),
                segment: next.clone(),
            },
            cx,
        );
        let position = hub.transcript_indexes["overlay"].positions["b129"];
        assert_eq!(hub.segments["overlay"][position], next);
        assert!(!hub.transcript_receipt_current("overlay", &receipt));
        assert!(Arc::ptr_eq(
            &hub.meeting_cache.transcripts["overlay"],
            &cache.transcripts["overlay"]
        ));
        assert_eq!(
            cache.fragments["overlay"]
                .iter()
                .find(|r| r.id == "b129")
                .unwrap()
                .text,
            restored_row("b129", 129, true).text
        );
    });
}
