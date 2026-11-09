use super::*;
use bruhs_models::PersonProfile;

impl TaskSaves {
    pub fn contacts_saving(self) -> bool {
        self.queue.with(|queue| queue.saving_kind(Kind::Person))
    }

    /// The contact, capture receipt and pending-sync identity commit together.
    /// People hydration already merges profiles by identity/version; only the
    /// receipt needs this writer's initial-snapshot journal.
    pub fn capture_person(self, capture: CaptureRecord, person: PersonProfile) {
        self.initial_captures
            .update_value(|edits| edits.edit(&capture.id, Some(&capture)));
        self.queue.update(|queue| {
            let ids = [
                (Kind::Person, person.id.clone()),
                (Kind::Capture, capture.id.clone()),
            ];
            queue.edit_record(
                Kind::Person,
                person.id.clone(),
                None,
                Some(Record::Person(person)),
            );
            queue.edit_capture(capture.id.clone(), None, Some(Record::Capture(capture)));
            queue.group_records(&ids);
        });
        self.refresh_status();
        self.save();
    }
}
