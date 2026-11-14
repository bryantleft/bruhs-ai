use super::*;

#[test]
fn dense_lanes_keep_stable_ties_lowest_reuse_and_extreme_endpoints() {
    for count in [0, 1, 63, 64, 65, 66, 128, 1_000] {
        let mut events = vec![(0, 100); count];
        events.extend([(100, 100), (100, 200), (100, 50), (201, 300)]);
        assert_eq!(assign_lanes(&events), legacy_assign_lanes(&events));
    }
    let extremes = [
        (i64::MIN, i64::MAX),
        (i64::MIN, i64::MIN),
        (0, 0),
        (i64::MAX, i64::MAX),
    ];
    assert_eq!(assign_lanes(&extremes), legacy_assign_lanes(&extremes));
}

#[test]
fn mixed_intervals_match_the_first_available_lane_policy() {
    let mut seed = 0x0123_4567_89ab_cdef_u64;
    for count in 0..2_000 {
        let events = (0..count % 130)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let start = i64::try_from(seed % 80).unwrap() - 40;
                let end = start + i64::try_from((seed >> 8) % 90).unwrap() - 5;
                (start, end)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            assign_lanes(&events),
            legacy_assign_lanes(&events),
            "{events:?}"
        );
    }
}

#[test]
#[ignore = "diagnostic optimized timings depend on machine load"]
fn dense_calendar_lane_cost() {
    use std::{hint::black_box, time::Instant};
    for count in [100, 1_000, 10_000] {
        for density in [1, 8, 64, count] {
            let events = (0..count)
                .map(|index| (index, index + density))
                .collect::<Vec<_>>();
            assert_eq!(assign_lanes(&events), legacy_assign_lanes(&events));
            for (name, assign) in [
                (
                    "legacy",
                    legacy_assign_lanes as fn(&[(i64, i64)]) -> LaneLayout,
                ),
                ("current", assign_lanes),
            ] {
                let mut samples = (0..31)
                    .map(|_| {
                        let start = Instant::now();
                        black_box(assign(black_box(&events)));
                        start.elapsed()
                    })
                    .collect::<Vec<_>>();
                samples.sort_unstable();
                println!("calendar lanes / {count} events / {density} overlap / {name}: p50 {:.3} ms, p95 {:.3} ms", samples[15].as_secs_f64() * 1_000.0, samples[29].as_secs_f64() * 1_000.0);
            }
        }
    }
}

fn legacy_assign_lanes(events: &[(i64, i64)]) -> LaneLayout {
    let mut sorted: Vec<(usize, i64, i64)> = events
        .iter()
        .enumerate()
        .map(|(i, (start, end))| (i, *start, *end))
        .collect();
    sorted.sort_by_key(|(_, start, _)| *start);

    let mut lane_ends: Vec<i64> = Vec::new();
    let mut placements: Vec<LanePlacement> = Vec::with_capacity(sorted.len());

    for (index, start, end) in sorted {
        let lane = match lane_ends
            .iter_mut()
            .enumerate()
            .find_map(|(lane, lane_end)| {
                if start >= *lane_end {
                    *lane_end = end;
                    Some(lane)
                } else {
                    None
                }
            }) {
            Some(lane) => lane,
            None => {
                lane_ends.push(end);
                lane_ends.len() - 1
            }
        };
        placements.push(LanePlacement { index, lane });
    }

    LaneLayout {
        lanes: lane_ends.len().max(1),
        placements,
    }
}
