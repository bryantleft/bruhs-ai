use super::*;
use bruhs_models::PersonProfile;

impl TaskSaves {
    pub fn contacts_saving(self) -> bool {
        self.queue.with(|queue| queue.saving_kind(Kind::Person))
    }

    /// Inbox or knowledge edits are still being written. People deletes and
    /// undo rewrite those records, so they wait.
    pub fn saving_links(self) -> bool {
        self.queue
            .with_untracked(|queue| queue.saving_kind(Kind::Inbox) || queue.saving_kind(Kind::Document))
    }

    /// A contact created on the People page. The contact and its pending-sync
    /// identity commit together; sync starts once it is saved.
    pub fn create_person(self, person: PersonProfile) {
        self.queue.update(|queue| {
            queue.edit_record(
                Kind::Person,
                person.id.clone(),
                None,
                Some(Record::Person(person)),
            );
        });
        self.refresh_status();
        self.save();
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
