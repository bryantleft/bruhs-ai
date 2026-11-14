use super::*;
use crate::state::{CalendarEvent, WorkItem};

impl Queue {
    pub fn overlay_plan_sources(&self, tasks: &mut Vec<WorkItem>, events: &mut Vec<CalendarEvent>) {
        let mut pending_tasks: std::collections::HashMap<_, _> = self
            .0
            .values()
            .filter_map(|pending| match (&pending.kind, &pending.draft) {
                (Kind::Task, Some(Record::Task(draft))) => Some((pending.id.as_str(), Some(draft))),
                (Kind::Task, None) => Some((pending.id.as_str(), None)),
                _ => None,
            })
            .collect();
        tasks.retain_mut(|task| match pending_tasks.remove(task.id.as_str()) {
            Some(Some(draft)) => {
                *task = draft.clone();
                true
            }
            Some(None) => false,
            None => true,
        });
        tasks.extend(pending_tasks.into_values().flatten().cloned());
        let mut pending_events: std::collections::HashMap<_, _> = self
            .0
            .values()
            .filter_map(|pending| match (&pending.kind, &pending.draft) {
                (Kind::Event, Some(Record::Event(draft))) => {
                    Some((pending.id.as_str(), Some(draft)))
                }
                (Kind::Event, None) => Some((pending.id.as_str(), None)),
                _ => None,
            })
            .collect();
        events.retain_mut(|event| match pending_events.remove(event.id.as_str()) {
            Some(Some(draft)) => {
                *event = draft.clone();
                true
            }
            Some(None) => false,
            None => true,
        });
        events.extend(pending_events.into_values().flatten().cloned());
    }

    /// Rebase only the plan's relationship delta into edits queued during its
    /// exclusive transaction. A different durable baseline stays a conflict.
    pub fn adopt_plan_task(
        &mut self,
        before: &WorkItem,
        after: &WorkItem,
    ) -> Option<Option<WorkItem>> {
        let pending = self.0.get_mut(&(Kind::Task, after.id.clone()))?;
        if matches!(&pending.expected, Some(Record::Task(original)) if original == before) {
            pending.expected = Some(Record::Task(after.clone()));
            if let Some(Record::Task(draft)) = &mut pending.draft {
                super::super::planning::patch_links(draft, before, after);
            }
        } else {
            pending.failure = Some(("This task changed while the day plan was saving. Your unsaved edit is retained; copy recovery before reloading.".into(), true));
        }
        Some(match &pending.draft {
            Some(Record::Task(draft)) => Some(draft.clone()),
            _ => None,
        })
    }

    pub fn adopt_plan_event(
        &mut self,
        before: Option<&CalendarEvent>,
        after: Option<&CalendarEvent>,
    ) -> Option<Option<CalendarEvent>> {
        let id = before.or(after)?.id.clone();
        let pending = self.0.get_mut(&(Kind::Event, id))?;
        // A queued deletion of the same event is already satisfied by Undo.
        if after.is_none()
            && pending.draft.is_none()
            && matches!((&pending.expected, before), (Some(Record::Event(original)), Some(before)) if original == before)
        {
            pending.expected = None;
        } else {
            pending.failure = Some(("This event changed while the day plan was saving. Your unsaved edit is retained; copy recovery before reloading.".into(), true));
        }
        Some(match &pending.draft {
            Some(Record::Event(draft)) => Some(draft.clone()),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planning_commit_preserves_a_newer_queued_title_and_planning_edit() {
        let original = WorkItem::new_at("Original", 1);
        let mut planned = original.clone();
        planned
            .planning
            .time_block_event_ids
            .push("plan-owned".into());
        planned.updated_at = 2;
        let mut edited = original.clone();
        edited.title = "Newer local edit".into();
        edited.planning.estimate_minutes = Some(90);
        edited.planning.time_block_event_ids.push("manual".into());
        let mut queue = Queue::default();
        queue.edit(
            original.id.clone(),
            Some(Record::Task(original.clone())),
            Some(Record::Task(edited.clone())),
        );
        let merged = queue.adopt_plan_task(&original, &planned).unwrap().unwrap();
        assert_eq!(merged.title, edited.title);
        assert_eq!(merged.planning.estimate_minutes, Some(90));
        assert_eq!(
            merged.planning.time_block_event_ids,
            ["manual", "plan-owned"]
        );
        assert_eq!(queue.next().unwrap().expected, Some(Record::Task(planned)));
    }

    #[test]
    fn a_different_durable_baseline_preserves_the_unsaved_edit_as_a_conflict() {
        let original = WorkItem::new_at("Original", 1);
        let mut durable = original.clone();
        durable.title = "Other tab".into();
        let mut after = durable.clone();
        after.planning.time_block_event_ids.push("plan".into());
        let mut draft = original.clone();
        draft.description = Some("Retain this".into());
        let mut queue = Queue::default();
        queue.edit(
            original.id.clone(),
            Some(Record::Task(original)),
            Some(Record::Task(draft.clone())),
        );
        assert_eq!(queue.adopt_plan_task(&durable, &after), Some(Some(draft)));
        assert!(queue.failure().unwrap().1);
        assert!(queue.next_batch().is_none());
    }
}
