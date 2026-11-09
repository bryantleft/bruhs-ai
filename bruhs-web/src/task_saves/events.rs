use super::*;

impl TaskSaves {
    pub fn finish_loading_events(self, loaded: Vec<CalendarEvent>) -> Vec<CalendarEvent> {
        let mut merged = Vec::new();
        self.initial_events
            .update_value(|edits| merged = edits.merge_by(loaded, |record| &record.id));
        merged
    }

    pub fn edit_event(
        self,
        id: String,
        expected: Option<CalendarEvent>,
        draft: Option<CalendarEvent>,
    ) {
        self.edit_events(vec![(id, expected, draft)]);
    }

    /// Event batches (including deletions and conflict copies) are one operation,
    /// not independent writes that may leave a partially applied calendar.
    pub fn edit_events(self, changes: Vec<(String, Option<CalendarEvent>, Option<CalendarEvent>)>) {
        self.initial_events.update_value(|edits| {
            for (id, _, draft) in &changes {
                edits.edit(id, draft.as_ref());
            }
        });
        self.queue.update(|queue| {
            let ids = changes
                .iter()
                .map(|(id, _, _)| (Kind::Event, id.clone()))
                .collect::<Vec<_>>();
            for (id, expected, draft) in changes {
                queue.edit_record(
                    Kind::Event,
                    id,
                    expected.map(Record::Event),
                    draft.map(Record::Event),
                );
            }
            queue.group_records(&ids);
        });
        self.refresh_status();
        self.save();
    }

    pub fn capture_event(self, capture: CaptureRecord, event: CalendarEvent) {
        self.initial_events
            .update_value(|edits| edits.edit(&event.id, Some(&event)));
        self.initial_captures
            .update_value(|edits| edits.edit(&capture.id, Some(&capture)));
        self.queue.update(|queue| {
            let ids = [
                (Kind::Event, event.id.clone()),
                (Kind::Capture, capture.id.clone()),
            ];
            queue.edit_record(
                Kind::Event,
                event.id.clone(),
                None,
                Some(Record::Event(event)),
            );
            queue.edit_capture(capture.id.clone(), None, Some(Record::Capture(capture)));
            queue.group_records(&ids);
        });
        self.refresh_status();
        self.save();
    }

    pub fn schedule_task(self, original: WorkItem, task: WorkItem, event: CalendarEvent) {
        self.initial_edits
            .update_value(|edits| edits.edit(&task.id, Some(&task)));
        self.initial_events
            .update_value(|edits| edits.edit(&event.id, Some(&event)));
        self.queue.update(|queue| {
            let ids = [
                (Kind::Task, task.id.clone()),
                (Kind::Event, event.id.clone()),
            ];
            queue.edit(
                task.id.clone(),
                Some(Record::Task(original)),
                Some(Record::Task(task)),
            );
            queue.edit_record(
                Kind::Event,
                event.id.clone(),
                None,
                Some(Record::Event(event)),
            );
            queue.group_records(&ids);
        });
        self.refresh_status();
        self.save();
    }
}
