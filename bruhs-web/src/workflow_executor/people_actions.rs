//! People page actions that change a contact on the server and in this
//! browser: rename, tracking, aliases, delete, and undo for each supported
//! change. Undo applies only while the touched records are unchanged.

use bruhs_models::{
    relink_person, restore_person_links, retarget_person_links, unlink_person, PeopleUndo,
    PeopleUndoError, PersonProfile,
};

use super::*;
use crate::api::ApiError;
use crate::storage::people::{changes, merge};

impl WorkflowExecutor {
    /// Contacts are loading, syncing, merging, or another People action runs.
    pub fn people_busy(self) -> bool {
        !self.people_hydrated.get()
            || self.people_loading.get()
            || self.contact_syncing.get()
            || self.contact_merging.get()
            || self.people_action_busy.get()
    }

    fn people_busy_untracked(self) -> bool {
        !self.people_hydrated.get_untracked()
            || self.people_loading.get_untracked()
            || self.contact_syncing.get_untracked()
            || self.contact_merging.get_untracked()
            || self.people_action_busy.get_untracked()
    }

    fn person_untracked(self, id: &str) -> Option<PersonProfile> {
        self.people
            .0
            .with_untracked(|people| people.iter().find(|person| person.id == id).cloned())
    }

    fn people_failed(self, message: String) {
        self.people_notice.set(Some(message));
    }

    /// Rename on the server; the receipt is kept only after it confirms.
    pub fn rename_person(self, id: String, name: String) {
        let name = name.trim().to_string();
        if name.is_empty() {
            self.people_failed(bruhs_models::people_copy::RENAME_BLANK.into());
            return;
        }
        let Some(current) = self.person_untracked(&id) else {
            return;
        };
        if current.display_name == name {
            return;
        }
        self.people_notice.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            match crate::api::rename_person(&id, &name).await {
                Ok(updated) => {
                    let after = updated.display_name.clone();
                    self.cache_people(vec![updated]).await;
                    self.people_undo.set(Some(PeopleUndo::Rename {
                        person_id: id,
                        before: current.display_name,
                        after,
                    }));
                }
                Err(error) => self.people_failed(format!(
                    "The profile was not renamed. Try again. Details: {error}"
                )),
            }
        });
    }

    /// Change the opt-out on the server; a failure never looks applied.
    pub fn set_person_tracked(self, id: String, tracked: bool) {
        let Some(current) = self.person_untracked(&id) else {
            return;
        };
        self.people_notice.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            match crate::api::set_person_tracked(&id, tracked).await {
                Ok(updated) => {
                    let after = updated.tracked;
                    self.cache_people(vec![updated]).await;
                    if after != current.tracked {
                        self.people_undo.set(Some(PeopleUndo::Tracked {
                            person_id: id,
                            name: current.display_name,
                            before: current.tracked,
                            after,
                        }));
                    }
                }
                Err(error) => self.people_failed(format!(
                    "Observation tracking was not changed. Try again. Details: {error}"
                )),
            }
        });
    }

    /// Save a reviewed alias list. `on_saved` runs only after the server
    /// confirms, so a refused alias stays in its input.
    pub fn set_person_aliases(
        self,
        id: String,
        aliases: Vec<String>,
        on_saved: impl FnOnce() + 'static,
    ) {
        self.people_notice.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            match crate::api::set_person_aliases(&id, aliases).await {
                Ok(updated) => {
                    self.cache_people(vec![updated]).await;
                    on_saved();
                }
                Err(ApiError::Http {
                    status: 409,
                    detail: Some(detail),
                }) => self.people_failed(detail),
                Err(error) => self.people_failed(format!(
                    "The aliases were not saved. Try again. Details: {error}"
                )),
            }
        });
    }

    /// Delete a contact after the user confirmed. The server deletes first,
    /// then this browser removes the contact and its relationship and unlinks
    /// its private links in one transaction. `links_saving` is true while
    /// Inbox or knowledge edits are still being written.
    pub fn delete_person(self, id: String, links_saving: bool) {
        if self.people_busy_untracked() || links_saving {
            self.people_failed(PeopleUndoError::Busy.message().into());
            return;
        }
        let Some(person) = self.person_untracked(&id) else {
            return;
        };
        let guard = match self
            .relationships
            .begin_transaction(std::slice::from_ref(&id))
        {
            Ok(guard) => guard,
            Err(error) => {
                self.people_failed(error.to_string());
                return;
            }
        };
        let relationship = guard.expected().get(&id).cloned().flatten();
        self.people_action_busy.set(true);
        self.people_deleting.set(Some(id.clone()));
        self.people_notice.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            let local_only = match crate::api::delete_person(&id).await {
                Ok(_) => false,
                // The server never saw this contact: it was still waiting for sync.
                Err(ApiError::Http { status: 404, .. }) => true,
                Err(error) => {
                    drop(guard);
                    self.people_action_busy.set(false);
                    self.people_deleting.set(None);
                    self.people_failed(format!(
                        "The profile was not deleted. Try again. Details: {error}"
                    ));
                    return;
                }
            };
            match changes::delete(&person, relationship.as_ref()).await {
                Ok(deleted) => {
                    guard.replaced(vec![(id.clone(), None)]);
                    self.forget_person_in_memory(&id);
                    self.people_undo.set(Some(PeopleUndo::Delete {
                        person,
                        relationship,
                        links: deleted.links,
                        local_only: local_only || deleted.was_pending,
                    }));
                }
                Err(error) => {
                    drop(guard);
                    self.people_failed(format!(
                        "The server deleted {}, but this browser could not save the change. Reload People. Details: {error}",
                        person.display_name
                    ));
                }
            }
            self.people_action_busy.set(false);
            self.people_deleting.set(None);
        });
    }

    fn forget_person_in_memory(self, id: &str) {
        self.set_people
            .0
            .update(|people| people.retain(|person| person.id != id));
        self.inbox.0.update(|items| {
            unlink_person(id, items, &mut []);
        });
        self.knowledge.0.update(|documents| {
            unlink_person(id, &mut [], documents);
        });
    }

    /// Another client deleted a contact: apply the same cascade here. This
    /// browser keeps no undo for a change it did not make.
    pub fn remote_person_deleted(self, id: String) {
        if self
            .people_deleting
            .with_untracked(|deleting| deleting.as_deref() == Some(id.as_str()))
        {
            return;
        }
        let Some(person) = self.person_untracked(&id) else {
            return;
        };
        let Ok(guard) = self
            .relationships
            .begin_transaction(std::slice::from_ref(&id))
        else {
            // Edits are saving; drop the contact from view and let the next load settle it.
            self.set_people
                .0
                .update(|people| people.retain(|person| person.id != id));
            return;
        };
        let relationship = guard.expected().get(&id).cloned().flatten();
        wasm_bindgen_futures::spawn_local(async move {
            match changes::delete(&person, relationship.as_ref()).await {
                Ok(_) => {
                    guard.replaced(vec![(id.clone(), None)]);
                    self.forget_person_in_memory(&id);
                }
                Err(error) => {
                    drop(guard);
                    self.report_people_cache(format!(
                        "A contact deleted on another device could not be removed here. {error} Reload People."
                    ));
                }
            }
        });
    }

    /// Record a merge this browser completed, so it can be undone.
    pub(super) fn remember_merge(self, record: &merge::Record) {
        self.people_undo.set(Some(PeopleUndo::Merge {
            source_id: record.source.id.clone(),
            source_name: record.source.display_name.clone(),
            target_id: record.target.id.clone(),
            target_name: record.target.display_name.clone(),
            before_source: record.before_source.clone(),
            before_target: record.before_target.clone(),
            after_relationship: record.after_relationship.clone(),
            links: record.relinked.clone(),
        }));
    }

    /// Merge cascade in memory, matching what the merge commit wrote.
    pub(super) fn retarget_links_in_memory(self, from: &str, to: &str) {
        self.inbox.0.update(|items| {
            retarget_person_links(from, to, items, &mut []);
        });
        self.knowledge.0.update(|documents| {
            retarget_person_links(from, to, &mut [], documents);
        });
    }

    /// Undo the latest People change, if its records are unchanged.
    pub fn undo_people(self) {
        let Some(undo) = self.people_undo.get_untracked() else {
            return;
        };
        if self.people_busy_untracked() {
            self.people_failed(PeopleUndoError::Busy.message().into());
            return;
        }
        let ids = undo
            .person_ids()
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let guard = match self.relationships.begin_transaction(&ids) {
            Ok(guard) => guard,
            Err(error) => {
                self.people_failed(error.to_string());
                return;
            }
        };
        let checked = self.people.0.with_untracked(|people| {
            undo.check(
                |id| people.iter().find(|person| person.id == id),
                |id| guard.expected().get(id).and_then(Option::as_ref),
            )
        });
        if let Err(error) = checked {
            drop(guard);
            self.people_undo.set(None);
            self.people_failed(error.message().into());
            return;
        }
        self.people_action_busy.set(true);
        self.people_notice.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            let result = self.run_undo(&undo, guard).await;
            match result {
                Ok(()) => self.people_undo.set(None),
                Err(message) => self.people_failed(message),
            }
            self.people_action_busy.set(false);
        });
    }

    async fn run_undo(
        self,
        undo: &PeopleUndo,
        guard: crate::relationship_store::RelationshipTransaction,
    ) -> Result<(), String> {
        match undo {
            PeopleUndo::Rename {
                person_id, before, ..
            } => {
                drop(guard);
                let updated =
                    crate::api::rename_person(person_id, before)
                        .await
                        .map_err(|error| {
                            format!("The rename was not undone. Try again. Details: {error}")
                        })?;
                self.cache_people(vec![updated]).await;
                Ok(())
            }
            PeopleUndo::Tracked {
                person_id, before, ..
            } => {
                drop(guard);
                let updated = crate::api::set_person_tracked(person_id, *before)
                    .await
                    .map_err(|error| {
                        format!("The tracking change was not undone. Try again. Details: {error}")
                    })?;
                self.cache_people(vec![updated]).await;
                Ok(())
            }
            PeopleUndo::Delete {
                person,
                relationship,
                links,
                local_only,
            } => {
                let restored_person = if *local_only {
                    person.clone()
                } else {
                    crate::api::restore_person(&person.id)
                        .await
                        .map_err(|error| {
                            format!("The delete was not undone. Try again. Details: {error}")
                        })?
                };
                let restored = changes::restore(
                    &restored_person,
                    relationship.as_ref(),
                    links,
                    *local_only,
                )
                .await
                .map_err(|error| {
                    format!("The contact was restored on the server, but this browser could not save it. Reload People. Details: {error}")
                })?;
                guard.replaced(vec![(person.id.clone(), relationship.clone())]);
                self.set_people.0.update(|people| {
                    if !people
                        .iter()
                        .any(|current| current.id == restored_person.id)
                    {
                        people.push(restored_person.clone());
                    }
                });
                let id = person.id.clone();
                self.inbox.0.update(|items| {
                    relink_person(&id, &restored, items, &mut []);
                });
                self.knowledge.0.update(|documents| {
                    relink_person(&id, &restored, &mut [], documents);
                });
                if *local_only {
                    self.retry_contact_sync();
                }
                Ok(())
            }
            PeopleUndo::Merge {
                source_id,
                target_id,
                before_source,
                before_target,
                ..
            } => {
                let record = self
                    .merge_record
                    .get_untracked()
                    .filter(|record| {
                        matches!(record.phase, merge::Phase::Completed { .. })
                            && record.source.id == *source_id
                            && record.target.id == *target_id
                    })
                    .ok_or_else(|| PeopleUndoError::ProfileChanged.message().to_string())?;
                let reply = crate::api::unmerge_person(source_id)
                    .await
                    .map_err(|error| {
                        format!("The merge was not undone. Try again. Details: {error}")
                    })?;
                let restored = merge::unmerge(&record, &reply).await.map_err(|error| {
                    format!("The server undid the merge, but this browser could not save it. Reload People. Details: {error}")
                })?;
                guard.replaced(vec![
                    (source_id.clone(), before_source.clone()),
                    (target_id.clone(), before_target.clone()),
                ]);
                self.set_people.0.update(|people| {
                    people.retain(|person| person.id != reply.source.id);
                    if let Some(target) = people
                        .iter_mut()
                        .find(|person| person.id == reply.target.id)
                    {
                        *target = reply.target.clone();
                    } else {
                        people.push(reply.target.clone());
                    }
                    people.push(reply.source.clone());
                });
                let (from, to) = (source_id.clone(), target_id.clone());
                self.inbox.0.update(|items| {
                    restore_person_links(&from, &to, &restored, items, &mut []);
                });
                self.knowledge.0.update(|documents| {
                    restore_person_links(&from, &to, &restored, &mut [], documents);
                });
                self.publish_merge_record(None);
                Ok(())
            }
        }
    }
}
