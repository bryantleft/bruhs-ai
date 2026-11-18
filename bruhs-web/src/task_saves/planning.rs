//! The app owns planning writes and their receipt across route changes.

use super::*;
use crate::state::{
    CalendarEventsSignal, SetCalendarEventsSignal, SetWorkItemsSignal, WorkItemsSignal,
};
use crate::storage;
use crate::storage::day_plans::{Commit, Receipt};
use bruhs_models::{DayPlanProposal, DayPlanWindow};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Default)]
pub struct DayPlanState {
    pub receipt: Option<Receipt>,
    pub busy: bool,
    pub loaded: bool,
    pub error: Option<String>,
    /// The banner (its id, then the message) that repeats a failed Apply's
    /// or Undo's `error` over any page. It goes with that error.
    pub(super) error_banner: Option<(String, String)>,
}

/// The banner to take down: it repeats an error that is no longer the day
/// plan's, because it was dismissed, replaced, or resolved by a later Apply
/// or Undo, as desktop's single error goes.
pub(super) fn stale_error_banner(state: &DayPlanState) -> Option<&str> {
    let (id, message) = state.error_banner.as_ref()?;
    (state.error.as_deref() != Some(message.as_str())).then_some(id.as_str())
}

/// Commute only this transaction's link changes with a newer local edit.
pub(super) fn patch_links(current: &mut WorkItem, before: &WorkItem, after: &WorkItem) {
    current.planning.time_block_event_ids.retain(|id| {
        !before.planning.time_block_event_ids.contains(id)
            || after.planning.time_block_event_ids.contains(id)
    });
    for id in &after.planning.time_block_event_ids {
        if !before.planning.time_block_event_ids.contains(id)
            && !current.planning.time_block_event_ids.contains(id)
        {
            current.planning.time_block_event_ids.push(id.clone());
        }
    }
    current.updated_at = current.updated_at.max(after.updated_at);
}

impl TaskSaves {
    pub fn refresh_day_plan_sources(self) {
        if !self.begin_day_plan() {
            return;
        }
        let WorkItemsSignal(tasks) = expect_context();
        let CalendarEventsSignal(events) = expect_context();
        let SetWorkItemsSignal(set_tasks) = expect_context();
        let SetCalendarEventsSignal(set_events) = expect_context();
        let baseline_tasks = tasks.get_untracked();
        let baseline_events = events.get_untracked();
        wasm_bindgen_futures::spawn_local(async move {
            match storage::day_plans::snapshot().await {
                Ok(mut snapshot) => {
                    let current_tasks = tasks.get_untracked();
                    let current_events = events.get_untracked();
                    preserve_newer(
                        &mut snapshot.tasks,
                        &baseline_tasks,
                        &current_tasks,
                        |task| &task.id,
                    );
                    preserve_newer(
                        &mut snapshot.events,
                        &baseline_events,
                        &current_events,
                        |event| &event.id,
                    );
                    // Live provider records may be projected outside the local
                    // store. Refreshing local records must not hide those blocks.
                    let mut event_ids: HashSet<String> = snapshot
                        .events
                        .iter()
                        .map(|event| event.id.clone())
                        .collect();
                    for event in current_events
                        .iter()
                        .filter(|event| event.source != bruhs_models::EventSource::Local)
                    {
                        if event_ids.insert(event.id.clone()) {
                            snapshot.events.push(event.clone());
                        }
                    }
                    self.queue.with_untracked(|queue| {
                        queue.overlay_plan_sources(&mut snapshot.tasks, &mut snapshot.events)
                    });
                    self.initial_edits.update_value(|journal| {
                        let indexed: HashMap<_, _> = snapshot
                            .tasks
                            .iter()
                            .map(|task| (task.id.as_str(), task))
                            .collect();
                        for task in baseline_tasks.iter().chain(snapshot.tasks.iter()) {
                            journal.edit(&task.id, indexed.get(task.id.as_str()).copied());
                        }
                    });
                    self.initial_events.update_value(|journal| {
                        let indexed: HashMap<_, _> = snapshot
                            .events
                            .iter()
                            .map(|event| (event.id.as_str(), event))
                            .collect();
                        for event in baseline_events.iter().chain(snapshot.events.iter()) {
                            journal.edit(&event.id, indexed.get(event.id.as_str()).copied());
                        }
                    });
                    set_tasks.set(snapshot.tasks);
                    set_events.set(snapshot.events);
                    self.day_plan.update(|state| {
                        state.receipt = snapshot.receipt;
                        state.busy = false;
                        state.error = None;
                    });
                    self.refresh_status();
                    self.save();
                }
                Err(error) => self.finish_day_plan_error(error.to_string()),
            }
        });
    }

    pub fn load_day_plan(self) {
        if self
            .day_plan
            .with_untracked(|state| state.busy || state.loaded)
        {
            return;
        }
        self.day_plan.update(|state| state.busy = true);
        self.refresh_status();
        wasm_bindgen_futures::spawn_local(async move {
            let result = storage::day_plans::load().await;
            let _ = self.day_plan.try_update(|state| {
                state.busy = false;
                match result {
                    Ok(receipt) => {
                        state.receipt = receipt;
                        state.loaded = true;
                        state.error = None;
                    }
                    Err(error) => state.error = Some(error.to_string()),
                }
            });
            self.save();
        });
    }

    fn begin_day_plan(self) -> bool {
        if self.day_plan.with_untracked(|state| state.busy) {
            return false;
        }
        let error = if !self.day_plan.with_untracked(|state| state.loaded) {
            Some("Day-plan recovery has not loaded. Retry loading before planning.")
        } else if !expect_context::<crate::workflow_executor::WorkflowExecutor>()
            .ready
            .get_untracked()
        {
            Some("Local tasks and calendar have not loaded. Retry loading before planning.")
        } else if self.saving.get_untracked()
            || self.queue.with_untracked(|queue| !queue.is_empty())
        {
            Some("Finish saving current changes before applying or undoing a day plan. Your review is retained.")
        } else {
            None
        };
        self.day_plan.update(|state| {
            state.error = error.map(str::to_owned);
            state.busy = error.is_none();
        });
        self.refresh_status();
        error.is_none()
    }

    pub fn apply_day_plan(self, proposal: DayPlanProposal, window: DayPlanWindow) -> bool {
        if !self.begin_day_plan() {
            return false;
        }
        let WorkItemsSignal(tasks) = expect_context();
        let CalendarEventsSignal(events) = expect_context();
        let SetWorkItemsSignal(set_tasks) = expect_context();
        let SetCalendarEventsSignal(set_events) = expect_context();
        let baseline_tasks = tasks.get_untracked();
        let baseline_events = events.get_untracked();
        if let Err(error) = bruhs_models::prepare_day_plan(
            &proposal,
            &baseline_tasks,
            &baseline_events,
            window,
            crate::clock::now_ms(),
        ) {
            self.finish_day_plan_error(error.to_string());
            return false;
        }
        wasm_bindgen_futures::spawn_local(async move {
            match storage::day_plans::apply(&proposal, window).await {
                Ok(commit) => self.publish_day_plan(
                    commit,
                    &baseline_tasks,
                    &baseline_events,
                    set_tasks,
                    set_events,
                ),
                Err(error) => self.finish_day_plan_error(error.to_string()),
            }
        });
        true
    }

    pub fn undo_day_plan(self) {
        let Some(receipt) = self.day_plan.with_untracked(|state| state.receipt.clone()) else {
            return;
        };
        if !self.begin_day_plan() {
            return;
        }
        let WorkItemsSignal(tasks) = expect_context();
        let CalendarEventsSignal(events) = expect_context();
        let SetWorkItemsSignal(set_tasks) = expect_context();
        let SetCalendarEventsSignal(set_events) = expect_context();
        let baseline_tasks = tasks.get_untracked();
        let baseline_events = events.get_untracked();
        wasm_bindgen_futures::spawn_local(async move {
            match storage::day_plans::undo(&receipt).await {
                Ok(commit) => self.publish_day_plan(
                    commit,
                    &baseline_tasks,
                    &baseline_events,
                    set_tasks,
                    set_events,
                ),
                Err(error) => self.finish_day_plan_error(error.to_string()),
            }
        });
    }

    fn finish_day_plan_error(self, message: String) {
        let banner = self.banners.push_keyed(
            crate::components::banner::BannerKind::Error,
            message.clone(),
            None,
        );
        let replaced = self
            .day_plan
            .try_update(|state| {
                state.busy = false;
                state.error = Some(message.clone());
                state.error_banner.replace((banner, message))
            })
            .flatten();
        if let Some((old, _)) = replaced {
            self.banners.dismiss(&old);
        }
        self.refresh_status();
        self.save();
    }

    fn publish_day_plan(
        self,
        commit: Commit,
        baseline_tasks: &[WorkItem],
        baseline_events: &[CalendarEvent],
        set_tasks: WriteSignal<Vec<WorkItem>>,
        set_events: WriteSignal<Vec<CalendarEvent>>,
    ) {
        let mut task_overrides = HashMap::new();
        let mut event_overrides = HashMap::new();
        self.queue.update(|queue| {
            for (before, after) in &commit.events {
                if let Some(pending) = queue.adopt_plan_event(before.as_ref(), after.as_ref()) {
                    if let Some(event) = before.as_ref().or(after.as_ref()) {
                        event_overrides.insert(event.id.clone(), pending);
                    }
                }
            }
            for (before, after) in &commit.tasks {
                if let Some(pending) = queue.adopt_plan_task(before, after) {
                    // A task deletion queued after Apply also removes its newly
                    // created focus blocks, in the same ordered save group.
                    if pending.is_none() {
                        for (_, event) in &commit.events {
                            if let Some(event) = event.as_ref().filter(|event| {
                                after.planning.time_block_event_ids.contains(&event.id)
                            }) {
                                queue.edit_record(
                                    Kind::Event,
                                    event.id.clone(),
                                    Some(Record::Event(event.clone())),
                                    None,
                                );
                                queue.group_records(&[
                                    (Kind::Task, after.id.clone()),
                                    (Kind::Event, event.id.clone()),
                                ]);
                                event_overrides.insert(event.id.clone(), None);
                            }
                        }
                    }
                    task_overrides.insert(after.id.clone(), pending);
                }
            }
        });
        set_tasks.update(|items| {
            for (before, after) in &commit.tasks {
                match task_overrides.get(&after.id) {
                    Some(None) => items.retain(|task| task.id != after.id),
                    Some(Some(draft)) => replace(items, &draft.id, Some(draft), |task| &task.id),
                    None => {
                        if let Some(current) = items.iter_mut().find(|task| task.id == after.id) {
                            if current == before
                                || baseline_tasks.iter().any(|task| task == current)
                            {
                                *current = after.clone();
                            } else {
                                patch_links(current, before, after);
                            }
                        } else if !baseline_tasks.iter().any(|task| task.id == after.id) {
                            items.push(after.clone());
                        }
                    }
                }
            }
            self.initial_edits.update_value(|journal| {
                for (_, task) in &commit.tasks {
                    journal.edit(&task.id, items.iter().find(|current| current.id == task.id));
                }
            });
        });
        set_events.update(|items| {
            for (before, after) in &commit.events {
                let Some(event) = before.as_ref().or(after.as_ref()) else {
                    continue;
                };
                if let Some(pending) = event_overrides.get(&event.id) {
                    replace(items, &event.id, pending.as_ref(), |event| &event.id);
                } else if let Some(current) = items.iter().find(|current| current.id == event.id) {
                    if before.as_ref() == Some(current)
                        || baseline_events.iter().any(|event| event == current)
                    {
                        replace(items, &event.id, after.as_ref(), |event| &event.id);
                    }
                } else if before.is_none() {
                    replace(items, &event.id, after.as_ref(), |event| &event.id);
                }
                self.initial_events.update_value(|journal| {
                    journal.edit(
                        &event.id,
                        items.iter().find(|current| current.id == event.id),
                    )
                });
            }
        });
        self.day_plan.update(|state| {
            state.receipt = commit.receipt;
            state.busy = false;
            state.error = None;
        });
        self.refresh_status();
        self.save();
    }
}

pub(super) fn replace<T: Clone>(
    items: &mut Vec<T>,
    key: &str,
    value: Option<&T>,
    id: impl Fn(&T) -> &str,
) {
    if let Some(value) = value {
        if let Some(current) = items.iter_mut().find(|item| id(item) == key) {
            *current = value.clone();
        } else {
            items.push(value.clone());
        }
    } else {
        items.retain(|item| id(item) != key);
    }
}

fn preserve_newer<T: Clone + PartialEq>(
    loaded: &mut Vec<T>,
    baseline: &[T],
    current: &[T],
    id: impl Fn(&T) -> &str,
) {
    let original: HashMap<_, _> = baseline.iter().map(|item| (id(item), item)).collect();
    let latest: HashMap<_, _> = current.iter().map(|item| (id(item), item)).collect();
    loaded.retain(|item| !original.contains_key(id(item)) || latest.contains_key(id(item)));
    let mut included = HashSet::new();
    for item in loaded.iter_mut() {
        if let Some(current) = latest.get(id(item)) {
            if original
                .get(id(item))
                .is_none_or(|before| *before != *current)
            {
                *item = (*current).clone();
            }
        }
        included.insert(id(item).to_owned());
    }
    for current in current {
        if original
            .get(id(current))
            .is_none_or(|before| *before != current)
            && included.insert(id(current).to_owned())
        {
            loaded.push(current.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refreshing_keeps_newer_local_edits_and_deletions_without_reviving_stale_records() {
        let a = WorkItem::new_at("Original", 1);
        let b = WorkItem::new_at("Locally deleted", 1);
        let c = WorkItem::new_at("Deleted in the other tab", 1);
        let mut remote = a.clone();
        remote.priority = bruhs_models::Priority::Urgent;
        let mut local = a.clone();
        local.title = "Newer unsaved local title".into();
        let baseline = vec![a, b.clone(), c.clone()];
        let mut loaded = vec![remote, b];
        let current = vec![local.clone(), c];
        preserve_newer(&mut loaded, &baseline, &current, |task| &task.id);
        assert_eq!(loaded, [local]);
    }

    /// A refused Undo's banner said "A focus block changed after planning"
    /// long after the block was renamed back and Undo had worked.
    #[test]
    fn the_error_banner_goes_with_the_error_it_repeats() {
        let refused = "A focus block changed after planning. Review it before undoing the plan.";
        let mut state = DayPlanState {
            error: Some(refused.into()),
            error_banner: Some(("banner-1".into(), refused.into())),
            ..DayPlanState::default()
        };
        assert_eq!(stale_error_banner(&state), None, "still the error");
        state.error = None;
        assert_eq!(
            stale_error_banner(&state),
            Some("banner-1"),
            "dismissed or resolved"
        );
        state.error = Some("Your calendar changed. Review a new day plan.".into());
        assert_eq!(stale_error_banner(&state), Some("banner-1"), "replaced");
        state.error_banner = None;
        assert_eq!(stale_error_banner(&state), None, "nothing to take down");
    }
}
