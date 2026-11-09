//! Sidebar ordering shared with bruhs-web. The shelf rules live in
//! `bruhs_models::logic::sidebar`. This module adapts desktop thread facts
//! onto that API.

use bruhs_models::logic::sidebar::{group_threads, FoldOpen, SidebarFacts, SidebarGroups};
use bruhs_models::time::{format_relative_ms, RelativeTimeStyle};
use bruhs_models::EpochMs;

/// Idle threads older than this fold into "Settled".
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "sidebar unit tests read the one-day settle window"
    )
)]
pub const SETTLED_AFTER_MS: i64 = bruhs_models::logic::sidebar::SETTLED_AFTER_MS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThreadMeta<'a> {
    pub id: &'a str,
    pub updated_at_ms: EpochMs,
    pub pinned: bool,
    pub settled: bool,
    pub pin_rank: Option<i64>,
    pub active_rank: Option<i64>,
    pub snoozed_until_ms: Option<i64>,
}

/// Same rules as the web. A pin does not keep an idle thread out of Settled.
/// Working and a thread that needs an answer never settle. A manual
/// `settled` flag does, including when the thread is also pinned.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "sidebar unit tests check the settle rule directly"
    )
)]
pub fn is_settled(
    thread: &ThreadMeta<'_>,
    now: EpochMs,
    active: Option<&str>,
    working: bool,
    unread: bool,
) -> bool {
    bruhs_models::logic::sidebar::is_settled(
        &facts(thread, active == Some(thread.id), working, false, unread),
        now.as_millis(),
    )
}

fn facts<'a>(
    thread: &'a ThreadMeta<'a>,
    active: bool,
    working: bool,
    needs_you: bool,
    unread: bool,
) -> SidebarFacts<'a> {
    SidebarFacts {
        id: thread.id,
        updated_at_ms: thread.updated_at_ms.as_millis(),
        pinned: thread.pinned,
        settled: thread.settled,
        pin_rank: thread.pin_rank,
        active_rank: thread.active_rank,
        snoozed_until_ms: thread.snoozed_until_ms,
        working,
        needs_you,
        active,
        unread,
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SidebarOrder<'a> {
    pub pinned: Vec<&'a str>,
    pub active: Vec<&'a str>,
    pub working: Vec<&'a str>,
    pub snoozed: Vec<&'a str>,
    pub settled: Vec<&'a str>,
}

impl<'a> SidebarOrder<'a> {
    /// Pinned, active, then the open folds. Closed shelves are skipped.
    pub fn flattened(&self, folds: FoldOpen) -> Vec<&'a str> {
        let mut ids = Vec::new();
        ids.extend(self.pinned.iter().copied());
        ids.extend(self.active.iter().copied());
        if folds.working {
            ids.extend(self.working.iter().copied());
        }
        if folds.snoozed {
            ids.extend(self.snoozed.iter().copied());
        }
        if folds.settled {
            ids.extend(self.settled.iter().copied());
        }
        ids
    }

    fn from_groups(groups: &SidebarGroups, threads: &'a [ThreadMeta<'a>]) -> Self {
        let resolve = |ids: &[String]| {
            ids.iter()
                .filter_map(|id| {
                    threads
                        .iter()
                        .find(|thread| thread.id == id)
                        .map(|thread| thread.id)
                })
                .collect()
        };
        Self {
            pinned: resolve(&groups.pinned),
            active: resolve(&groups.active),
            working: resolve(&groups.working),
            snoozed: resolve(&groups.snoozed),
            settled: resolve(&groups.settled),
        }
    }
}

/// Bucket `threads` with the shared shelf rules.
pub fn sidebar_order<'a>(
    threads: &'a [ThreadMeta<'a>],
    now: EpochMs,
    active: Option<&str>,
    working: &dyn Fn(&str) -> bool,
    needs_you: &dyn Fn(&str) -> bool,
    unread: &dyn Fn(&str) -> bool,
    working_section: bool,
) -> SidebarOrder<'a> {
    let rows: Vec<SidebarFacts<'_>> = threads
        .iter()
        .map(|thread| {
            facts(
                thread,
                active == Some(thread.id),
                working(thread.id),
                needs_you(thread.id),
                unread(thread.id),
            )
        })
        .collect();
    let groups = group_threads(&rows, now.as_millis(), working_section);
    SidebarOrder::from_groups(&groups, threads)
}

/// Compact relative age for sidebar cards: `now`, `5m`, `3h`, `2d`, `1w`.
pub fn relative_age(then: EpochMs, now: EpochMs) -> String {
    format_relative_ms(then, now, RelativeTimeStyle::Compact)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: EpochMs = match EpochMs::new(10 * SETTLED_AFTER_MS) {
        Ok(now) => now,
        Err(_) => panic!("in range"),
    };

    fn ago(age_ms: i64) -> EpochMs {
        EpochMs::new(NOW.as_millis() - age_ms).expect("in range")
    }

    fn meta(id: &str, age_ms: i64, pinned: bool, settled: bool) -> ThreadMeta<'_> {
        ThreadMeta {
            id,
            updated_at_ms: ago(age_ms),
            pinned,
            settled,
            pin_rank: None,
            active_rank: None,
            snoozed_until_ms: None,
        }
    }

    fn never(_: &str) -> bool {
        false
    }

    fn open_folds() -> FoldOpen {
        FoldOpen {
            working: true,
            snoozed: true,
            settled: true,
        }
    }

    #[test]
    fn pinned_first_then_fresh_then_settled() {
        let threads = [
            meta("old", 3 * SETTLED_AFTER_MS, false, false),
            meta("pin", 1_000, true, false),
            meta("new", 2_000, false, false),
            meta("newer", 10, false, false),
        ];
        let order = sidebar_order(&threads, NOW, None, &never, &never, &never, true);
        assert_eq!(order.pinned, vec!["pin"]);
        assert_eq!(order.active, vec!["newer", "new"]);
        assert_eq!(order.settled, vec!["old"]);
        assert_eq!(
            order.flattened(open_folds()),
            vec!["pin", "newer", "new", "old"]
        );
        assert!(is_settled(
            &meta("oldpin", 5 * SETTLED_AFTER_MS, true, false),
            NOW,
            None,
            false,
            false
        ));
    }

    #[test]
    fn working_thread_never_settles() {
        let t = meta("w", 3 * SETTLED_AFTER_MS, false, true);
        assert!(!is_settled(&t, NOW, None, true, false));
        let threads = [t];
        let order = sidebar_order(&threads, NOW, None, &|id| id == "w", &never, &never, true);
        assert_eq!(order.working, vec!["w"]);
        assert!(order.settled.is_empty());
    }

    #[test]
    fn unread_or_active_stay_fresh() {
        let old = meta("old", 3 * SETTLED_AFTER_MS, false, false);
        assert!(!is_settled(&old, NOW, Some("old"), false, false));
        assert!(!is_settled(&old, NOW, None, false, true));
        assert!(is_settled(&old, NOW, None, false, false));
    }

    #[test]
    fn manual_settled_flag_wins() {
        let fresh_but_settled = meta("s", 10, false, true);
        assert!(is_settled(&fresh_but_settled, NOW, None, false, false));
        assert!(is_settled(&fresh_but_settled, NOW, Some("s"), false, false));
        let pinned = meta("p", 10, true, true);
        assert!(is_settled(&pinned, NOW, None, false, false));
    }

    #[test]
    fn age_rule_uses_24h() {
        let just_under = meta("a", SETTLED_AFTER_MS - 1, false, false);
        let exactly = meta("b", SETTLED_AFTER_MS, false, false);
        assert!(!is_settled(&just_under, NOW, None, false, false));
        assert!(is_settled(&exactly, NOW, None, false, false));
    }

    #[test]
    fn relative_age_buckets() {
        assert_eq!(relative_age(ago(5_000), NOW), "now");
        assert_eq!(relative_age(ago(5 * 60_000), NOW), "5m");
        assert_eq!(relative_age(ago(3 * 3_600_000), NOW), "3h");
        assert_eq!(relative_age(ago(2 * 86_400_000), NOW), "2d");
        assert_eq!(relative_age(ago(15 * 86_400_000), NOW), "2w");
        assert_eq!(relative_age(ago(-5_000), NOW), "now");
    }
}
