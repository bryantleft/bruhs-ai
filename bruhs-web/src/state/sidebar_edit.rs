//! Mutations for the thread sidebar. Ordering lives in
//! `bruhs_models::logic::sidebar`; this module applies a drop or a menu
//! action to the web [`Thread`] list.

use std::collections::HashSet;

use bruhs_models::logic::sidebar::{
    apply_title_instructions, drag_effect, group_threads, place_id, ranks_for, section, DragEffect,
    SidebarFacts, SidebarGroups, SidebarSection,
};

use super::Thread;

/// Borrowed facts for one thread. A legacy row with no timestamp does not
/// age into Settled: its clock reads as `now_ms`.
pub fn thread_facts<'a>(
    thread: &'a Thread,
    now_ms: i64,
    active: bool,
    working: bool,
    needs_you: bool,
    unread: bool,
) -> SidebarFacts<'a> {
    SidebarFacts {
        id: &thread.id,
        updated_at_ms: thread.last_active().map_or(now_ms, |at| at.as_millis()),
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

/// Clock and membership the shelf rules need for one paint.
pub struct ShelfClock<'a> {
    pub now_ms: i64,
    pub active: Option<&'a str>,
    pub working: Option<&'a str>,
    pub needs_you: Option<&'a str>,
    pub unread: &'a HashSet<String>,
    pub working_section: bool,
}

fn facts_for<'a>(threads: &'a [Thread], clock: &ShelfClock<'_>) -> Vec<SidebarFacts<'a>> {
    threads
        .iter()
        .map(|thread| {
            thread_facts(
                thread,
                clock.now_ms,
                clock.active == Some(thread.id.as_str()),
                clock.working == Some(thread.id.as_str()),
                clock.needs_you == Some(thread.id.as_str()),
                clock.unread.contains(&thread.id),
            )
        })
        .collect()
}

pub fn groups_for(threads: &[Thread], clock: &ShelfClock<'_>) -> SidebarGroups {
    let facts = facts_for(threads, clock);
    group_threads(&facts, clock.now_ms, clock.working_section)
}

/// Apply a drop. Threads that move out of Snooze or Settled are cleared so
/// the section rule does not put them straight back.
pub fn apply_drag(threads: &mut [Thread], id: &str, effect: DragEffect, groups: &SidebarGroups) {
    match effect {
        DragEffect::None => {}
        DragEffect::Settle => settle_in_place(threads, id, true),
        DragEffect::PinAt(index) => {
            let order = place_id(&groups.pinned, id, index);
            assign(threads, &order, RankList::Pinned);
        }
        DragEffect::ReorderPinned(index) => {
            let order = place_id(&groups.pinned, id, index);
            assign(threads, &order, RankList::Pinned);
        }
        DragEffect::UnpinAt(index) | DragEffect::WakeAt(index) | DragEffect::UnsettleAt(index) => {
            let order = place_id(&groups.active, id, index);
            assign(threads, &order, RankList::Active);
        }
        DragEffect::ReorderActive(index) => {
            let order = place_id(&groups.active, id, index);
            assign(threads, &order, RankList::Active);
        }
    }
}

enum RankList {
    Pinned,
    Active,
}

fn assign(threads: &mut [Thread], order: &[String], list: RankList) {
    for (id, rank) in ranks_for(order) {
        if let Some(thread) = threads.iter_mut().find(|thread| thread.id == id) {
            match list {
                RankList::Pinned => {
                    thread.pinned = true;
                    thread.pin_rank = Some(rank);
                    thread.settled = false;
                    thread.snoozed_until_ms = None;
                }
                RankList::Active => {
                    thread.pinned = false;
                    thread.pin_rank = None;
                    thread.active_rank = Some(rank);
                    thread.settled = false;
                    thread.snoozed_until_ms = None;
                }
            }
        }
    }
}

/// Pin keeps the active rank. A new pin has no pin rank, so it sits above
/// pins that were dragged into a spot. Unpinning keeps the active rank.
pub fn set_pinned(thread: &mut Thread, pinned: bool) {
    thread.pinned = pinned;
    if pinned {
        thread.pin_rank = None;
        thread.settled = false;
        thread.snoozed_until_ms = None;
    } else {
        thread.pin_rank = None;
    }
}

/// Settling clears the pin and the active position. Un-settling clears the
/// active rank so the thread returns to the top of the active list.
pub fn set_settled(thread: &mut Thread, settled: bool) {
    if settled {
        thread.settled = true;
        thread.pinned = false;
        thread.pin_rank = None;
        thread.active_rank = None;
        thread.snoozed_until_ms = None;
    } else {
        thread.settled = false;
        thread.active_rank = None;
    }
}

fn settle_in_place(threads: &mut [Thread], id: &str, settled: bool) {
    if let Some(thread) = threads.iter_mut().find(|thread| thread.id == id) {
        set_settled(thread, settled);
    }
}

pub fn snooze_until(thread: &mut Thread, until_ms: i64) {
    thread.snoozed_until_ms = Some(until_ms);
}

/// A user message ends a snooze. Saved order stays as it was.
pub fn clear_snooze(thread: &mut Thread) {
    thread.snoozed_until_ms = None;
}

/// Wake brings the thread back to the top of the active list.
pub fn wake(thread: &mut Thread) {
    thread.snoozed_until_ms = None;
    thread.settled = false;
    thread.active_rank = None;
}

/// Drop effect for `id`, or `None` when the thread is missing.
pub fn effect_for(
    threads: &[Thread],
    id: &str,
    onto: SidebarSection,
    index: usize,
    clock: &ShelfClock<'_>,
) -> Option<(DragEffect, SidebarGroups)> {
    let facts = facts_for(threads, clock);
    let from = facts.iter().find(|facts| facts.id == id)?;
    let from_section = section(from, clock.now_ms, clock.working_section);
    let groups = group_threads(&facts, clock.now_ms, clock.working_section);
    Some((
        drag_effect(from_section, onto, index, clock.working_section),
        groups,
    ))
}

/// Global instructions, then the project line so a project prefix wins.
pub fn instructions_text(
    global: &str,
    by_project: &std::collections::BTreeMap<String, String>,
    project: &str,
) -> String {
    let project_text = by_project
        .get(project)
        .map(|text| text.trim())
        .filter(|text| !text.is_empty());
    match (global.trim().is_empty(), project_text) {
        (true, None) => String::new(),
        (false, None) => global.to_string(),
        (true, Some(project_text)) => project_text.to_string(),
        (false, Some(project_text)) => format!("{}\n{project_text}", global.trim()),
    }
}

/// Title for a new prompt or a regenerate. `prompt` is the first user
/// message when this surface has it; otherwise the current title is reused.
/// This build does not call a model.
pub fn titled(current: &str, prompt: Option<&str>, instructions: &str) -> String {
    let from_prompt = prompt.and_then(|prompt| {
        let titled = super::auto_title(prompt);
        (!titled.is_empty()).then_some(titled)
    });
    let base = from_prompt.unwrap_or_else(|| {
        let trimmed = current.split_whitespace().collect::<Vec<_>>().join(" ");
        if trimmed.is_empty() {
            "New thread".to_string()
        } else {
            trimmed
        }
    });
    apply_title_instructions(&base, instructions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::EpochMs;

    fn test_clock(now_ms: i64, unread: &HashSet<String>, working_section: bool) -> ShelfClock<'_> {
        ShelfClock {
            now_ms,
            active: None,
            working: None,
            needs_you: None,
            unread,
            working_section,
        }
    }

    fn thread(id: &str, updated: i64) -> Thread {
        let mut thread = Thread::new(
            id.into(),
            String::new(),
            None,
            EpochMs::new(updated).unwrap(),
        );
        thread.id = id.into();
        thread
    }

    #[test]
    fn settle_clears_pin_and_active_rank() {
        let mut thread = thread("a", 10);
        thread.pinned = true;
        thread.pin_rank = Some(1);
        thread.active_rank = Some(4);
        set_settled(&mut thread, true);
        assert!(thread.settled);
        assert!(!thread.pinned);
        assert_eq!(thread.pin_rank, None);
        assert_eq!(thread.active_rank, None);
        set_settled(&mut thread, false);
        assert!(!thread.settled);
        assert_eq!(thread.active_rank, None);
    }

    #[test]
    fn unpin_drop_lands_in_the_active_list() {
        let mut pinned = thread("pin", 50);
        pinned.pinned = true;
        pinned.pin_rank = Some(0);
        let mut live = thread("live", 40);
        live.active_rank = Some(0);
        let mut threads = vec![pinned, live];
        let unread = HashSet::new();
        let clock = test_clock(100, &unread, false);
        let (effect, groups) =
            effect_for(&threads, "pin", SidebarSection::Active, 1, &clock).unwrap();
        assert_eq!(effect, DragEffect::UnpinAt(1));
        apply_drag(&mut threads, "pin", effect, &groups);
        let groups = groups_for(&threads, &clock);
        assert!(groups.pinned.is_empty());
        assert_eq!(groups.active, vec!["live".to_string(), "pin".to_string()]);
    }

    #[test]
    fn snooze_hides_a_pin_until_wake_or_a_user_need() {
        let mut thread = thread("later", 50);
        thread.pinned = true;
        snooze_until(&mut thread, 500);
        let unread = HashSet::new();
        let groups = groups_for(&[thread.clone()], &test_clock(100, &unread, true));
        assert_eq!(groups.snoozed, vec!["later".to_string()]);
        let mut awake = thread.clone();
        wake(&mut awake);
        let groups = groups_for(&[awake], &test_clock(100, &unread, true));
        assert_eq!(groups.pinned, vec!["later".to_string()]);
    }

    #[test]
    fn title_instructions_prefer_the_project_line() {
        let mut by_project = std::collections::BTreeMap::new();
        by_project.insert("bruhs".into(), "prefix: BR".into());
        let text = instructions_text("max 12", &by_project, "bruhs");
        assert_eq!(titled("Fix the login", None, &text), "BR Fix the l");
        assert_eq!(
            titled(
                "New thread",
                Some("  Rename the parser\nplease"),
                "prefix: Bug"
            ),
            "Bug Rename the parser"
        );
    }
}
