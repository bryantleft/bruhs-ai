//! Synthetic cache validation/decode/restore/event boundaries, without providers.
use super::*;
use sha2::{Digest, Sha256};
use std::time::Instant;

fn row(index: usize, label: &str, partial: bool) -> TranscriptSegment {
    let start = u32::try_from(index).expect("fixture timestamp") * 2;
    TranscriptSegment {
        id: format!("segment-{index}"),
        text: format!(
            "{label} {index}: ΟΣ Café 👩🏽‍💻 \"quoted\"\n{}",
            "x".repeat(256)
        ),
        start_ms: start,
        end_ms: start + 1,
        partial,
        ..segment()
    }
}

fn cache(records: usize) -> MeetingCache {
    let overlap = records / 10;
    let appended = records / 10;
    let base = (0..records)
        .map(|i| row(i, "Base", i < overlap))
        .collect::<Vec<_>>();
    let mut fragments = StreamFragments::default();
    for i in 0..overlap {
        fragments
            .update(None, row(i, "Updated", true))
            .expect("valid fragment");
    }
    for i in records..records + appended {
        fragments
            .update(None, row(i, "Appended", true))
            .expect("valid fragment");
    }
    let mut meeting = meeting();
    meeting.created_at = 0;
    MeetingCache {
        records: [("cached-meeting".into(), Arc::new(meeting))]
            .into_iter()
            .collect(),
        transcripts: [("cached-meeting".into(), Arc::new(base))]
            .into_iter()
            .collect(),
        fragments: [("cached-meeting".into(), fragments)].into_iter().collect(),
    }
}

#[gpui::test]
fn overlay_fixture_retains_complete_snapshot_and_fragment_state(cx: &mut TestAppContext) {
    let cache = cache(128);
    let encoded = serde_json::to_vec(&cache).expect("cache JSON");
    let restored: MeetingCache = serde_json::from_slice(&encoded).expect("validated cache");
    assert_eq!(cache, restored);
    let expected = cache.fragments["cached-meeting"]
        .merge(cache.transcripts["cached-meeting"].as_ref().clone());
    let hub = fixture(cx);
    hub.update(cx, |hub, _| {
        hub.restore_meetings(restored);
        assert_eq!(hub.segments["cached-meeting"], expected);
        assert!(hub.live_meeting_id.is_none());
        assert!(!hub.search_cached.contains("cached-meeting"));
    });
}

#[gpui::test]
#[ignore = "manual optimized native overlay validation/decode/restore/event timing; no I/O/providers"]
fn native_overlay_action_cost(cx: &mut TestAppContext) {
    let hub = fixture(cx);
    for records in [128, 1024, 10000, 90000] {
        let cache = cache(records);
        let encoded = serde_json::to_vec(&cache).expect("cache JSON");
        let expected = cache.fragments["cached-meeting"]
            .merge(cache.transcripts["cached-meeting"].as_ref().clone());
        let fingerprint = format!("{:x}", Sha256::digest(&encoded));
        for action in [
            "validate-fragments",
            "decode-cache",
            "restore-cache",
            "stream-event",
        ] {
            if action == "stream-event" {
                hub.update(cx, |hub, _| hub.restore_meetings(cache.clone()));
            }
            let mut samples = Vec::new();
            for iteration in 0..35 {
                let elapsed = match action {
                    "validate-fragments" => {
                        let start = Instant::now();
                        let result = cache.fragments["cached-meeting"]
                            .validate("cached-meeting", &cache.transcripts["cached-meeting"]);
                        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                        result.expect("valid overlay");
                        elapsed
                    }
                    "decode-cache" => {
                        let start = Instant::now();
                        let decoded: MeetingCache =
                            serde_json::from_slice(&encoded).expect("validated decode");
                        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                        assert_eq!(decoded, cache);
                        elapsed
                    }
                    "restore-cache" => {
                        let incoming = cache.clone();
                        let start = Instant::now();
                        hub.update(cx, |hub, _| hub.restore_meetings(incoming));
                        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                        hub.update(cx, |hub, _| {
                            assert_eq!(hub.segments["cached-meeting"], expected)
                        });
                        elapsed
                    }
                    "stream-event" => {
                        let id = records + records / 10 - 1;
                        let mut record = row(id, &format!("Live {iteration}"), true);
                        record.text.push_str(" received");
                        let saved = hub.update(cx, |hub, _| hub.meeting_cache());
                        let start = Instant::now();
                        hub.update(cx, |hub, cx| {
                            hub.apply_meeting_event(
                                MeetingEvent::Segment {
                                    meeting_id: "cached-meeting".into(),
                                    segment: record.clone(),
                                },
                                cx,
                            )
                        });
                        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                        hub.update(cx, |hub, _| {
                            assert_eq!(hub.segments["cached-meeting"].len(), expected.len());
                            assert_eq!(hub.segments["cached-meeting"].last(), Some(&record));
                        });
                        assert_eq!(
                            saved.transcripts["cached-meeting"],
                            cache.transcripts["cached-meeting"]
                        );
                        elapsed
                    }
                    _ => unreachable!(),
                };
                if iteration >= 5 {
                    samples.push(elapsed);
                }
            }
            eprintln!(
                "OVERLAY_ACTION_COST {}",
                serde_json::json!({"records":records,"overlap":records/10,"appended":records/10,"merged_records":expected.len(),"action":action,"samples_ms":samples,"warmups":5,"serialized_cache_bytes":encoded.len(),"fixture_sha256":fingerprint,"scope":"Native component boundaries; validation only, JSON decode through validated MeetingCache, restore through completed hub update, or captured event through hub update. Construction/assertions/decoded-cache destruction outside timers. Restore and event include GPUI dispatch and old-state replacement; no physical paint, disk, HTTP, capture device or provider."})
            );
        }
    }
}

mod load_performance;
