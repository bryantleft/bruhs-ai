//! People page actions that need the server: create goes through
//! [`MeetingsHub::capture_person`]; this file holds aliases, delete, restore
//! and unmerge. The workspace owns the private side (relationships and links)
//! and reacts to the events emitted here.

use super::*;
use bruhs_models::{PersonDeletion, PersonUnmerge};

const DELETE_SYNCING: &str =
    "This contact is still syncing. Try again in a moment. Nothing was deleted.";

impl MeetingsHub {
    /// The cached profile with this id.
    pub fn person(&self, id: &str) -> Option<&PersonProfile> {
        self.people.iter().find(|p| p.id == id).map(Arc::as_ref)
    }

    /// Merges waiting for local recovery, and archived originals kept after a
    /// conflicting recovery.
    pub fn person_merge_recovery_counts(&self) -> (usize, usize) {
        (
            self.person_merges_waiting.len(),
            self.person_merge_recoveries.values().map(Vec::len).sum(),
        )
    }

    /// Replace one cached profile with the server's newer copy.
    fn replace_person(&mut self, person: PersonProfile) {
        if let Some(current) = self.people.iter_mut().find(|p| p.id == person.id) {
            if current.updated_at <= person.updated_at {
                *current = Arc::new(person);
            }
        } else {
            self.people.push(Arc::new(person));
        }
    }

    /// Drop a contact from every cache. Used for this client's deletes and
    /// for `PersonDeleted` from the server.
    pub fn forget_person(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        let before = self.people.len();
        self.people.retain(|p| p.id != id);
        let was_pending = self.pending_person_creates.remove(id);
        self.person_creates_attempted.remove(id);
        self.deleted_person_ids.insert(id.to_string());
        if self.people.len() != before || was_pending {
            self.people_changed(cx);
        }
    }

    /// Save a profile's complete alias list. The caller reviews new aliases
    /// with `bruhs_models::review_alias` first; the server reviews again.
    pub fn set_person_aliases(
        &mut self,
        person_id: String,
        aliases: Vec<String>,
        _cx: &mut Context<'_, Self>,
    ) {
        let client = self.client.clone();
        self.run(
            async move { client.set_person_aliases(&person_id, aliases).await },
            |hub, result, cx| {
                match result {
                    Ok(updated) => {
                        hub.replace_person(updated);
                        hub.people_changed(cx);
                    }
                    Err(error) => {
                        let message = error.conflict_message().unwrap_or_else(|| {
                            format!("The aliases were not saved. Try again. Details: {error}")
                        });
                        hub.notify_notice(message, true, cx);
                    }
                }
                cx.notify();
            },
        );
    }

    /// Delete a contact on the server, then from this cache. A contact the
    /// server does not know (a create that never reached it) is deleted
    /// locally only. Emits [`HubEvent::PersonDeleted`] once it is gone.
    pub fn delete_person(&mut self, person_id: String, cx: &mut Context<'_, Self>) {
        if self.person_creates_in_flight.contains(&person_id) {
            self.notify_notice(DELETE_SYNCING, true, cx);
            return;
        }
        let Some(person) = self.person(&person_id).cloned() else {
            return;
        };
        let client = self.client.clone();
        self.run(
            async move { client.delete_person(&person_id).await },
            move |hub, result: Result<PersonDeletion, ServerError>, cx| {
                let local_only = match result {
                    Ok(_) => false,
                    Err(error) if error.is_not_found() => true,
                    Err(error) => {
                        hub.note_request_failure(&error);
                        hub.notify_notice(
                            format!("The profile was not deleted. Try again. Details: {error}"),
                            true,
                            cx,
                        );
                        return;
                    }
                };
                hub.forget_person(&person.id, cx);
                cx.emit(HubEvent::PersonDeleted {
                    person: Box::new(person),
                    local_only,
                });
                cx.notify();
            },
        );
    }

    /// Undo of a delete: restore on the server, or re-add a local-only
    /// contact as a pending create. Emits [`HubEvent::PersonRestored`].
    pub fn restore_person(
        &mut self,
        person: PersonProfile,
        local_only: bool,
        cx: &mut Context<'_, Self>,
    ) {
        self.deleted_person_ids.remove(&person.id);
        if local_only {
            let id = person.id.clone();
            self.capture_person(person, cx);
            cx.emit(HubEvent::PersonRestored { person_id: id });
            return;
        }
        let client = self.client.clone();
        let id = person.id.clone();
        self.run(
            async move { client.restore_person(&id).await },
            move |hub, result, cx| {
                match result {
                    Ok(restored) if restored.id == person.id => {
                        let person_id = restored.id.clone();
                        hub.replace_person(restored);
                        hub.people_changed(cx);
                        cx.emit(HubEvent::PersonRestored { person_id });
                    }
                    Ok(_) => {
                        hub.deleted_person_ids.insert(person.id);
                        cx.emit(HubEvent::PeopleUndoFailed(
                            "Undo returned a different contact. Nothing else was changed. Retry contact sync to confirm.".into(),
                        ));
                    }
                    Err(error) => {
                        hub.deleted_person_ids.insert(person.id);
                        cx.emit(HubEvent::PeopleUndoFailed(format!(
                            "Undo did not complete. The profile is still deleted. Details: {error}"
                        )));
                    }
                }
                cx.notify();
            },
        );
    }

    /// Undo of a merge: the server restores both contacts and the meeting
    /// links the merge moved; this cache forgets the merge archive so the
    /// source identity is live again. Emits [`HubEvent::PersonUnmerged`].
    pub fn unmerge_person(&mut self, source_id: String, _cx: &mut Context<'_, Self>) {
        let client = self.client.clone();
        let id = source_id.clone();
        self.run(
            async move { client.unmerge_person(&id).await },
            move |hub, result: Result<PersonUnmerge, ServerError>, cx| {
                match result {
                    Ok(reply) if reply.source.id == source_id && reply.target.id != source_id => {
                        hub.person_merge_archives.remove(&source_id);
                        hub.person_merge_recoveries.remove(&source_id);
                        hub.person_merges_waiting.remove(&source_id);
                        hub.replace_person(reply.target);
                        hub.replace_person(reply.source);
                        hub.people_changed(cx);
                        cx.emit(HubEvent::PersonUnmerged { source_id });
                    }
                    Ok(_) => cx.emit(HubEvent::PeopleUndoFailed(
                        "Undo returned different contacts. Nothing else was changed. Retry contact sync to confirm.".into(),
                    )),
                    Err(error) => cx.emit(HubEvent::PeopleUndoFailed(format!(
                        "Undo did not complete. The profiles are still merged. Details: {error}"
                    ))),
                }
                cx.notify();
            },
        );
    }
}
