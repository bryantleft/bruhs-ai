//! Shared calendar primitives — constants and the lane-assignment
//! algorithm used by every timegrid renderer (web, desktop).
//!
//! Time units stay per-surface: bruhs-web works in `f64` ms (JS Date
//! native), bruhs-desktop works in `chrono::NaiveDateTime`. The greedy
//! algorithm below is generic over any monotonic `i64` so each side can
//! convert into a common scratch unit (typically seconds) at the call site.

use crate::num::{ToFloatLossy, ToIntSaturating};

/// Default pixels per hour row in the timegrid. Both surfaces start here;
/// either may override locally if a denser/sparser grid is wanted.
pub const HOUR_PX_DEFAULT: f32 = 40.0;

/// Drag-snap interval. 15 minutes feels "sticky" enough for real
/// scheduling but coarse enough that you never land on a noisy 17:43.
pub const SNAP_MINUTES: i64 = 15;

/// Minimum event duration enforced during resize.
pub const MIN_DURATION_MINUTES: i64 = 15;

/// Snap a vertical pixel offset to the nearest grid line. Returns minutes
/// from day start, never negative.
pub fn snap_minutes_from_offset(offset_px: f32, hour_px: f32, snap_minutes: i64) -> i64 {
    if hour_px <= 0.0 || snap_minutes <= 0 {
        return 0;
    }
    let raw_minutes = (offset_px / hour_px) * 60.0;
    let snap = snap_minutes.to_f32_lossy();
    let snapped = (raw_minutes / snap).round() * snap;
    snapped.max(0.0).to_i64_saturating()
}

/// Result of placing a single event into the lane layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanePlacement {
    /// Index back into the input slice the placement corresponds to.
    pub index: usize,
    /// Which lane (column-within-day) this event goes in. 0-indexed.
    pub lane: usize,
}

/// Result of laying out a day's events into lanes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneLayout {
    /// Per-event lane assignment, in input order.
    pub placements: Vec<LanePlacement>,
    /// Number of lanes the day uses (max concurrent overlap). All events
    /// on the day share this width factor (`1.0 / lanes`) so columns
    /// align cleanly.
    pub lanes: usize,
}

/// Greedy left-to-right lane assignment. Sorts events by start time, then
/// for each one picks the lowest lane whose previous occupant has already
/// ended. The returned `lanes` count is the day's maximum concurrent
/// usage, so all events render at the same width.
///
/// Inputs must already be filtered down to the events that overlap the day
/// being laid out — the algorithm doesn't know about days.
pub fn assign_lanes(events: &[(i64, i64)]) -> LaneLayout {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_overlap_each_in_lane_zero() {
        let events = [(0, 60), (60, 120), (120, 180)];
        let layout = assign_lanes(&events);
        assert_eq!(layout.lanes, 1);
        assert!(layout.placements.iter().all(|p| p.lane == 0));
    }

    #[test]
    fn full_overlap_grows_lanes() {
        let events = [(0, 100), (10, 90), (20, 80)];
        let layout = assign_lanes(&events);
        assert_eq!(layout.lanes, 3);
    }

    #[test]
    fn day_lanes_reflect_max_concurrency() {
        // First two events overlap (lanes=2), third is isolated. The whole
        // day uses 2 lanes so columns align across the day.
        let events = [(0, 60), (30, 90), (200, 260)];
        let layout = assign_lanes(&events);
        assert_eq!(layout.lanes, 2);
    }

    #[test]
    fn snap_rounds_to_nearest_grid_line() {
        // 27 minutes from start -> 30
        assert_eq!(snap_minutes_from_offset(18.0, 40.0, 15), 30);
        // 7 minutes -> 0 (rounds down)
        assert_eq!(snap_minutes_from_offset(4.7, 40.0, 15), 0);
        // 8 minutes -> 15 (rounds up)
        assert_eq!(snap_minutes_from_offset(5.4, 40.0, 15), 15);
    }

    #[test]
    fn snap_clamps_negative_to_zero() {
        assert_eq!(snap_minutes_from_offset(-100.0, 40.0, 15), 0);
    }
}
