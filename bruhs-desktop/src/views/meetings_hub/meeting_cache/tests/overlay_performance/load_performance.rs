//! Continuous validated decode -> restored native hub, with correlated stage samples.
use super::*;

mod reference;

const SHAPES: [&str; 4] = ["snapshot", "mixed", "fragments-only", "all-overlap"];
const MODES: [&str; 2] = ["empty-hub", "replace-hub"];

fn shaped_cache(records: usize, shape: &str) -> MeetingCache {
    let mut cache = cache(records);
    match shape {
        "snapshot" => cache.fragments.clear(),
        "mixed" => {}
        "fragments-only" | "all-overlap" => {
            if shape == "fragments-only" {
                cache.transcripts.clear();
            } else {
                cache.transcripts.insert(
                    "cached-meeting".into(),
                    Arc::new((0..records).map(|i| row(i, "Base", true)).collect()),
                );
            }
            let mut fragments = StreamFragments::default();
            for i in 0..records {
                fragments
                    .update(None, row(i, "Replaced", true))
                    .expect("valid fragment");
            }
            cache.fragments.insert("cached-meeting".into(), fragments);
        }
        _ => unreachable!(),
    }
    cache
}

fn expected(cache: &MeetingCache) -> Vec<TranscriptSegment> {
    let base = cache
        .transcripts
        .get("cached-meeting")
        .map_or_else(Vec::new, |records| records.as_ref().clone());
    match cache.fragments.get("cached-meeting") {
        Some(fragments) => fragments.merge(base),
        None => base,
    }
}

fn assert_restored(hub: &MeetingsHub, cache: &MeetingCache, expected: &[TranscriptSegment]) {
    assert_eq!(
        hub.meetings,
        [cache.records["cached-meeting"].as_ref().clone()]
    );
    assert_eq!(hub.segments.get("cached-meeting").unwrap(), expected);
    assert_eq!(hub.meeting_cache(), *cache);
    assert_eq!(
        hub.search_cached.contains("cached-meeting"),
        cache.transcripts.contains_key("cached-meeting")
            && !cache.fragments.contains_key("cached-meeting")
    );
    assert!(hub.live_meeting_id.is_none());
    assert!(hub.capture_status.recording().is_none());
}

#[gpui::test]
fn complete_load_fixture_preserves_all_cache_shapes(cx: &mut TestAppContext) {
    let hub = fixture(cx);
    for records in [0, 1, 129] {
        for shape in SHAPES {
            let cache = shaped_cache(records, shape);
            let expected = expected(&cache);
            let encoded = serde_json::to_vec(&cache).unwrap();
            for mode in MODES {
                if mode == "empty-hub" {
                    hub.update(cx, |hub, _| hub.restore_meetings(MeetingCache::default()));
                }
                let decoded: MeetingCache = serde_json::from_slice(&encoded).unwrap();
                hub.update(cx, |hub, _| {
                    hub.restore_meetings(decoded);
                    assert_restored(hub, &cache, &expected);
                });
            }
        }
    }
}

// Both restoration variants call this exact decoder entry point.
#[inline(never)]
fn decode_cache(encoded: &[u8]) -> MeetingCache {
    serde_json::from_slice(encoded).expect("validated cache decode")
}

#[inline(never)]
fn restore_current(hub: &mut MeetingsHub, cache: MeetingCache) {
    hub.restore_meetings(cache);
}

#[gpui::test]
#[ignore = "manual optimized continuous native cache load; no disk/HTTP/paint/providers"]
fn native_complete_cache_load_cost(cx: &mut TestAppContext) {
    let variant = std::env::var("BRUHS_PERF_LOAD_RESTORE").unwrap_or_else(|_| "revised".into());
    let restore_fn: fn(&mut MeetingsHub, MeetingCache) = match variant.as_str() {
        "original" => MeetingsHub::restore_original,
        "revised" => restore_current,
        _ => panic!("unknown restoration variant"),
    };
    let selected_case = std::env::var("BRUHS_PERF_LOAD_CASE").ok();
    let mut cases = 0;
    let hub = fixture(cx);
    for records in [128, 1024, 10000, 90000] {
        for shape in SHAPES {
            if selected_case
                .as_ref()
                .is_some_and(|case| !case.starts_with(&format!("{records}/{shape}/")))
            {
                continue;
            }
            let cache = shaped_cache(records, shape);
            let expected = expected(&cache);
            let encoded = serde_json::to_vec(&cache).expect("cache JSON");
            let fingerprint = format!("{:x}", Sha256::digest(&encoded));
            for mode in MODES {
                if selected_case
                    .as_ref()
                    .is_some_and(|case| case != &format!("{records}/{shape}/{mode}"))
                {
                    continue;
                }
                cases += 1;
                hub.update(cx, |hub, _| restore_fn(hub, cache.clone()));
                let mut total = Vec::new();
                let mut decode = Vec::new();
                let mut restore = Vec::new();
                for iteration in 0..35 {
                    if mode == "empty-hub" {
                        hub.update(cx, |hub, _| restore_fn(hub, MeetingCache::default()));
                    }
                    let start = Instant::now();
                    let decoded = decode_cache(&encoded);
                    let decoded_at = Instant::now();
                    hub.update(cx, |hub, _| restore_fn(hub, decoded));
                    let finished = Instant::now();
                    hub.update(cx, |hub, _| assert_restored(hub, &cache, &expected));
                    if iteration >= 5 {
                        total.push(finished.duration_since(start).as_secs_f64() * 1000.0);
                        decode.push(decoded_at.duration_since(start).as_secs_f64() * 1000.0);
                        restore.push(finished.duration_since(decoded_at).as_secs_f64() * 1000.0);
                    }
                }
                eprintln!(
                    "CACHE_LOAD_COST {}",
                    serde_json::json!({"restore_variant":variant,"records":records,"shape":shape,"mode":mode,"base_records":cache.transcripts.get("cached-meeting").map_or(0,|r|r.len()),"fragment_records":cache.fragments.get("cached-meeting").map_or(0,|f|f.iter().count()),"merged_records":expected.len(),"serialized_cache_bytes":encoded.len(),"fixture_sha256":fingerprint,"warmups":5,"total_samples_ms":total,"decode_samples_ms":decode,"restore_samples_ms":restore,"scope":"One continuous validated MeetingCache decode through completed GPUI restore; original and revised restoration share the same out-of-line decoder in one optimized executable. Correlated decode and restore stage timestamps; input construction and complete state assertions excluded. Empty-hub clears old state outside the timer; replace-hub includes old state destruction/replacement. Encoded bytes and fixture are warm; no cold OS/cache, disk, HTTP, input/paint, capture device or provider measurement."})
                );
            }
        }
    }
    assert!(cases > 0, "no matching cache-load case");
}
