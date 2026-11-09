//! Thread-list sections shared by web and desktop.
//!
//! The thread sidebar's rules: pinned, active, a Working shelf, snooze, and
//! settled. Ranks are local. A missing rank means the
//! thread has not been dragged into a spot.

/// Idle threads older than this fold into Settled. T3's default is three
/// days and is configurable; bruhs keeps one day.
pub const SETTLED_AFTER_MS: i64 = 24 * 60 * 60 * 1000;

/// How long an Undo notification stays up.
pub const UNDO_MS: i64 = 5_000;

/// Sidebar shelves, top to bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarSection {
    Pinned,
    Active,
    Working,
    Snoozed,
    Settled,
}

/// What a drop does. `index` is the slot in the destination list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragEffect {
    None,
    PinAt(usize),
    UnpinAt(usize),
    ReorderPinned(usize),
    ReorderActive(usize),
    Settle,
    UnsettleAt(usize),
    WakeAt(usize),
}

/// One thread, borrowed, with the flags the list needs.
#[derive(Clone, Copy, Debug)]
pub struct SidebarFacts<'a> {
    pub id: &'a str,
    pub updated_at_ms: i64,
    pub pinned: bool,
    pub settled: bool,
    pub pin_rank: Option<i64>,
    pub active_rank: Option<i64>,
    pub snoozed_until_ms: Option<i64>,
    pub working: bool,
    pub needs_you: bool,
    pub active: bool,
    pub unread: bool,
}

/// Ids in shelf order. Each vec is top to bottom.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SidebarGroups {
    pub pinned: Vec<String>,
    pub active: Vec<String>,
    pub working: Vec<String>,
    pub snoozed: Vec<String>,
    pub settled: Vec<String>,
}

impl SidebarGroups {
    /// The order keyboard navigation and ⌘1–9 walk, skipping closed shelves.
    pub fn flattened(&self, folds_open: FoldOpen) -> Vec<String> {
        let mut ids = Vec::new();
        ids.extend(self.pinned.iter().cloned());
        ids.extend(self.active.iter().cloned());
        if folds_open.working {
            ids.extend(self.working.iter().cloned());
        }
        if folds_open.snoozed {
            ids.extend(self.snoozed.iter().cloned());
        }
        if folds_open.settled {
            ids.extend(self.settled.iter().cloned());
        }
        ids
    }
}

/// Which folded shelves are expanded. Closed shelves stay in the groups
/// so their headers can show a count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoldOpen {
    pub working: bool,
    pub snoozed: bool,
    pub settled: bool,
}

/// True while `now_ms` is still inside the five-second undo window.
pub fn undo_open(started_ms: i64, now_ms: i64) -> bool {
    now_ms.saturating_sub(started_ms) < UNDO_MS && now_ms >= started_ms
}

/// A manually settled thread stays settled. Otherwise an idle, read,
/// unselected thread settles after [`SETTLED_AFTER_MS`], pin or not.
/// Working and a thread that needs an answer stay up.
pub fn is_settled(facts: &SidebarFacts<'_>, now_ms: i64) -> bool {
    if facts.working || facts.needs_you {
        return false;
    }
    if facts.settled {
        return true;
    }
    if facts.active || facts.unread {
        return false;
    }
    now_ms.saturating_sub(facts.updated_at_ms) >= SETTLED_AFTER_MS
}

fn snoozed(facts: &SidebarFacts<'_>, now_ms: i64) -> bool {
    facts.snoozed_until_ms.is_some_and(|until| until > now_ms) && !facts.needs_you
}

pub fn section(facts: &SidebarFacts<'_>, now_ms: i64, working_section: bool) -> SidebarSection {
    if snoozed(facts, now_ms) {
        return SidebarSection::Snoozed;
    }
    if is_settled(facts, now_ms) {
        return SidebarSection::Settled;
    }
    if facts.pinned {
        return SidebarSection::Pinned;
    }
    // An approval or a question comes back to the active list, ahead of the
    // Working shelf.
    if facts.needs_you {
        return SidebarSection::Active;
    }
    if facts.working && working_section {
        return SidebarSection::Working;
    }
    SidebarSection::Active
}

/// Bucket and sort. `working_section` folds streaming threads under Working
/// and orders Active by recency. With it off, saved ranks apply and
/// unranked threads sit above arranged ones.
pub fn group_threads(
    threads: &[SidebarFacts<'_>],
    now_ms: i64,
    working_section: bool,
) -> SidebarGroups {
    let mut pinned = Vec::new();
    let mut active = Vec::new();
    let mut working = Vec::new();
    let mut snoozed_rows = Vec::new();
    let mut settled = Vec::new();
    for facts in threads {
        match section(facts, now_ms, working_section) {
            SidebarSection::Pinned => pinned.push(*facts),
            SidebarSection::Active => active.push(*facts),
            SidebarSection::Working => working.push(*facts),
            SidebarSection::Snoozed => snoozed_rows.push(*facts),
            SidebarSection::Settled => settled.push(*facts),
        }
    }
    sort_ranked(&mut pinned, |facts| facts.pin_rank);
    if working_section {
        active.sort_by(|a, b| {
            b.needs_you
                .cmp(&a.needs_you)
                .then(b.updated_at_ms.cmp(&a.updated_at_ms))
                .then(a.id.cmp(b.id))
        });
    } else {
        sort_ranked(&mut active, |facts| facts.active_rank);
    }
    working.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms).then(a.id.cmp(b.id)));
    snoozed_rows.sort_by(|a, b| {
        a.snoozed_until_ms
            .cmp(&b.snoozed_until_ms)
            .then(a.id.cmp(b.id))
    });
    settled.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms).then(a.id.cmp(b.id)));
    SidebarGroups {
        pinned: ids(&pinned),
        active: ids(&active),
        working: ids(&working),
        snoozed: ids(&snoozed_rows),
        settled: ids(&settled),
    }
}

/// Unranked rows first, newest activity first. Ranked rows follow in rank
/// order. Equal keys fall back to id so the order is stable.
fn sort_ranked(rows: &mut [SidebarFacts<'_>], rank: impl Fn(&SidebarFacts<'_>) -> Option<i64>) {
    rows.sort_by(|a, b| match (rank(a), rank(b)) {
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (None, None) => b.updated_at_ms.cmp(&a.updated_at_ms).then(a.id.cmp(b.id)),
        (Some(left), Some(right)) => left.cmp(&right).then(a.id.cmp(b.id)),
    });
}

fn ids(rows: &[SidebarFacts<'_>]) -> Vec<String> {
    rows.iter().map(|facts| facts.id.to_string()).collect()
}

/// Where a drag lands. Drops into Working or Snooze do nothing: those
/// shelves are not places a thread is put by hand. Reordering the active
/// list is off while the Working section is on.
pub fn drag_effect(
    from: SidebarSection,
    onto: SidebarSection,
    index: usize,
    working_section: bool,
) -> DragEffect {
    if matches!(onto, SidebarSection::Working | SidebarSection::Snoozed) {
        return DragEffect::None;
    }
    if onto == SidebarSection::Settled {
        return if from == SidebarSection::Settled {
            DragEffect::None
        } else {
            DragEffect::Settle
        };
    }
    match (from, onto) {
        (SidebarSection::Pinned, SidebarSection::Pinned) => DragEffect::ReorderPinned(index),
        (SidebarSection::Pinned, SidebarSection::Active) => DragEffect::UnpinAt(index),
        (SidebarSection::Active, SidebarSection::Pinned) => DragEffect::PinAt(index),
        (SidebarSection::Active, SidebarSection::Active) => {
            if working_section {
                DragEffect::None
            } else {
                DragEffect::ReorderActive(index)
            }
        }
        (SidebarSection::Working, SidebarSection::Pinned) => DragEffect::PinAt(index),
        (SidebarSection::Working, SidebarSection::Active) => DragEffect::None,
        (SidebarSection::Snoozed, SidebarSection::Pinned) => DragEffect::PinAt(index),
        (SidebarSection::Snoozed, SidebarSection::Active) => DragEffect::WakeAt(index),
        (SidebarSection::Settled, SidebarSection::Pinned) => DragEffect::PinAt(index),
        (SidebarSection::Settled, SidebarSection::Active) => DragEffect::UnsettleAt(index),
        _ => DragEffect::None,
    }
}

/// Move `id` to `index` in `order`. An index past the end appends.
pub fn place_id(order: &[impl AsRef<str>], id: &str, index: usize) -> Vec<String> {
    let mut ids: Vec<String> = order
        .iter()
        .map(|item| item.as_ref().to_string())
        .filter(|item| item != id)
        .collect();
    let index = index.min(ids.len());
    ids.insert(index, id.to_string());
    ids
}

/// Ranks for a top-to-bottom list. `0` is the top.
pub fn ranks_for(ids: &[impl AsRef<str>]) -> Vec<(String, i64)> {
    ids.iter()
        .enumerate()
        .map(|(index, id)| {
            (
                id.as_ref().to_string(),
                i64::try_from(index).unwrap_or(i64::MAX),
            )
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnoozeUnit {
    Minutes,
    Hours,
    Days,
}

/// A positive duration from `now_ms`. One day is 24 hours. `None` when the
/// amount is zero or the sum overflows.
pub fn snooze_after(now_ms: i64, amount: u64, unit: SnoozeUnit) -> Option<i64> {
    if amount == 0 {
        return None;
    }
    let unit_ms: u64 = match unit {
        SnoozeUnit::Minutes => 60_000,
        SnoozeUnit::Hours => 3_600_000,
        SnoozeUnit::Days => 86_400_000,
    };
    let delta = amount.checked_mul(unit_ms)?;
    now_ms.checked_add(i64::try_from(delta).ok()?)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnoozePreset {
    pub label: &'static str,
    pub until_ms: i64,
}

/// Later today (18:00 local, or three hours when that has passed), tomorrow
/// at 09:00, and a week from today at 09:00. `utc_offset_minutes` is minutes
/// east of UTC.
pub fn snooze_presets(now_ms: i64, utc_offset_minutes: i32) -> Vec<SnoozePreset> {
    let offset_ms = i64::from(utc_offset_minutes).saturating_mul(60_000);
    let local = now_ms.saturating_add(offset_ms);
    let day = 86_400_000;
    let day_start = local - local.rem_euclid(day);
    let local_minute = (local - day_start) / 60_000;
    let later = if local_minute < 18 * 60 {
        day_start + 18 * 3_600_000 - offset_ms
    } else {
        now_ms.saturating_add(3 * 3_600_000)
    };
    let at_local =
        |day_index: i64, minute: i64| day_start + day_index * day + minute * 60_000 - offset_ms;
    vec![
        SnoozePreset {
            label: "Later today",
            until_ms: later,
        },
        SnoozePreset {
            label: "Tomorrow",
            until_ms: at_local(1, 9 * 60),
        },
        SnoozePreset {
            label: "Next week",
            until_ms: at_local(7, 9 * 60),
        },
    ]
}

/// Apply thread title instructions to a candidate title.
///
/// A `prefix:` line is placed first. `max N` caps the length in characters.
/// Other lines stay on the instruction record for the person who wrote them;
/// this build does not send them to a model.
pub fn apply_title_instructions(title: &str, instructions: &str) -> String {
    let mut title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        title = "New thread".to_string();
    }
    let mut prefix: Option<String> = None;
    let mut max_chars: Option<usize> = None;
    for line in instructions.lines() {
        let line = line.trim();
        if let Some(rest) = strip_prefix_ignore_ascii_case(line, "prefix:") {
            let rest = rest.trim();
            if !rest.is_empty() {
                prefix = Some(rest.to_string());
            }
        } else if let Some(rest) = strip_prefix_ignore_ascii_case(line, "max ") {
            if let Some(n) = rest
                .split_whitespace()
                .next()
                .and_then(|token| token.parse().ok())
                .filter(|n: &usize| *n > 0)
            {
                max_chars = Some(n);
            }
        }
    }
    if let Some(prefix) = prefix {
        if !title.starts_with(&prefix) {
            title = format!("{prefix} {title}");
        }
    }
    if let Some(max_chars) = max_chars {
        if title.chars().count() > max_chars {
            title = title.chars().take(max_chars).collect::<String>();
            title = title.trim_end().to_string();
        }
    }
    title
}

fn strip_prefix_ignore_ascii_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let mut value_chars = value.chars();
    for expected in prefix.chars() {
        let next = value_chars.next()?;
        if !next.eq_ignore_ascii_case(&expected) {
            return None;
        }
    }
    Some(&value[prefix.len()..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(id: &str) -> SidebarFacts<'_> {
        SidebarFacts {
            id,
            updated_at_ms: 1_000,
            pinned: false,
            settled: false,
            pin_rank: None,
            active_rank: None,
            snoozed_until_ms: None,
            working: false,
            needs_you: false,
            active: false,
            unread: false,
        }
    }

    const NOW: i64 = 10 * SETTLED_AFTER_MS;

    #[test]
    fn age_settles_a_pin_and_a_manual_flag_wins() {
        let mut old = facts("old");
        old.updated_at_ms = NOW - SETTLED_AFTER_MS;
        old.pinned = true;
        assert!(is_settled(&old, NOW));
        let mut fresh = facts("fresh");
        fresh.updated_at_ms = NOW - 10;
        fresh.settled = true;
        assert!(is_settled(&fresh, NOW));
        fresh.working = true;
        assert!(!is_settled(&fresh, NOW));
        fresh.working = false;
        fresh.needs_you = true;
        assert!(!is_settled(&fresh, NOW));
    }

    #[test]
    fn shelves_follow_snooze_then_settled_then_pin_then_working() {
        let mut pinned = facts("pin");
        pinned.pinned = true;
        pinned.updated_at_ms = NOW;
        let mut active = facts("live");
        active.updated_at_ms = NOW;
        let mut working = facts("run");
        working.working = true;
        working.updated_at_ms = NOW - 5;
        let mut snoozed = facts("later");
        snoozed.snoozed_until_ms = Some(NOW + 1_000);
        snoozed.pinned = true;
        let mut settled = facts("done");
        settled.settled = true;
        let groups = group_threads(&[settled, snoozed, working, active, pinned], NOW, true);
        assert_eq!(groups.pinned, vec!["pin"]);
        assert_eq!(groups.active, vec!["live"]);
        assert_eq!(groups.working, vec!["run"]);
        assert_eq!(groups.snoozed, vec!["later"]);
        assert_eq!(groups.settled, vec!["done"]);
    }

    #[test]
    fn approval_leaves_the_working_and_snoozed_shelves() {
        let mut waiting = facts("ask");
        waiting.working = true;
        waiting.needs_you = true;
        waiting.snoozed_until_ms = Some(NOW + 5_000);
        let groups = group_threads(&[waiting], NOW, true);
        assert_eq!(groups.active, vec!["ask"]);
        assert!(groups.working.is_empty());
        assert!(groups.snoozed.is_empty());
    }

    #[test]
    fn unranked_threads_sit_above_arranged_ones() {
        let mut arranged = facts("old");
        arranged.active_rank = Some(0);
        arranged.updated_at_ms = NOW;
        let mut fresh = facts("new");
        fresh.updated_at_ms = NOW - 50;
        let groups = group_threads(&[arranged, fresh], NOW, false);
        assert_eq!(groups.active, vec!["new", "old"]);
    }

    #[test]
    fn working_section_orders_active_by_recency_and_blocks_reorder() {
        let mut older = facts("older");
        older.updated_at_ms = NOW - 100;
        older.active_rank = Some(0);
        let mut newer = facts("newer");
        newer.updated_at_ms = NOW;
        newer.active_rank = Some(5);
        let groups = group_threads(&[older, newer], NOW, true);
        assert_eq!(groups.active, vec!["newer", "older"]);
        assert_eq!(
            drag_effect(SidebarSection::Active, SidebarSection::Active, 0, true),
            DragEffect::None
        );
        assert_eq!(
            drag_effect(SidebarSection::Active, SidebarSection::Active, 1, false),
            DragEffect::ReorderActive(1)
        );
    }

    #[test]
    fn drops_match_the_section_they_land_on() {
        assert_eq!(
            drag_effect(SidebarSection::Active, SidebarSection::Pinned, 0, false),
            DragEffect::PinAt(0)
        );
        assert_eq!(
            drag_effect(SidebarSection::Pinned, SidebarSection::Active, 2, false),
            DragEffect::UnpinAt(2)
        );
        assert_eq!(
            drag_effect(SidebarSection::Active, SidebarSection::Settled, 0, false),
            DragEffect::Settle
        );
        assert_eq!(
            drag_effect(SidebarSection::Settled, SidebarSection::Active, 0, true),
            DragEffect::UnsettleAt(0)
        );
        assert_eq!(
            drag_effect(SidebarSection::Snoozed, SidebarSection::Active, 1, true),
            DragEffect::WakeAt(1)
        );
        assert_eq!(
            drag_effect(SidebarSection::Active, SidebarSection::Snoozed, 0, false),
            DragEffect::None
        );
        assert_eq!(
            drag_effect(SidebarSection::Active, SidebarSection::Working, 0, true),
            DragEffect::None
        );
    }

    #[test]
    fn place_id_moves_without_duplicating() {
        assert_eq!(place_id(&["a", "b", "c"], "a", 2), vec!["b", "c", "a"]);
        assert_eq!(place_id(&["a", "b"], "c", 0), vec!["c", "a", "b"]);
        assert_eq!(
            ranks_for(&["b", "a"]),
            vec![("b".into(), 0), ("a".into(), 1)]
        );
    }

    #[test]
    fn snooze_duration_and_presets() {
        assert_eq!(snooze_after(1_000, 0, SnoozeUnit::Days), None);
        assert_eq!(
            snooze_after(1_000, 1, SnoozeUnit::Days),
            Some(1_000 + 86_400_000)
        );
        // Noon on a UTC day. Later today is 18:00 that day.
        let day = 86_400_000_i64;
        let noon = day * 100 + 15 * 3_600_000;
        let presets = snooze_presets(noon, 0);
        assert_eq!(presets[0].label, "Later today");
        assert_eq!(presets[0].until_ms, day * 100 + 18 * 3_600_000);
        assert_eq!(presets[1].until_ms, day * 101 + 9 * 3_600_000);
        assert_eq!(presets[2].until_ms, day * 107 + 9 * 3_600_000);
        let evening = day * 100 + 19 * 3_600_000;
        assert_eq!(
            snooze_presets(evening, 0)[0].until_ms,
            evening + 3 * 3_600_000
        );
    }

    #[test]
    fn title_instructions_apply_prefix_and_cap() {
        assert_eq!(
            apply_title_instructions("Fix the parser", "prefix: bruhs\nmax 12"),
            "bruhs Fix th"
        );
        assert_eq!(apply_title_instructions("   ", ""), "New thread");
        assert_eq!(
            apply_title_instructions("bruhs notes", "prefix: bruhs"),
            "bruhs notes"
        );
    }

    #[test]
    fn undo_lasts_five_seconds() {
        assert!(undo_open(1_000, 1_000 + UNDO_MS - 1));
        assert!(!undo_open(1_000, 1_000 + UNDO_MS));
    }
}
