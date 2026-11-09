use std::collections::{btree_map::Entry, BTreeMap, BTreeSet};

use super::attachment_recovery::{RecoveryAttachment, RecoveryRecord};
use super::{Kind, Record};

#[derive(Clone)]
pub(super) struct Pending {
    pub id: String,
    pub kind: Kind,
    pub expected: Option<Record>,
    pub draft: Option<Record>,
    pub failure: Option<(String, bool)>,
    group: String,
}

/// One original and latest draft per typed identity, including deletion tombstones.
#[derive(Default)]
pub(super) struct Queue(BTreeMap<(Kind, String), Pending>);

impl Queue {
    pub fn conversation(&self, id: &str) -> Option<Option<crate::storage::MessagesRecord>> {
        let pending = self.0.get(&(Kind::Conversation, id.to_owned()))?;
        Some(match &pending.draft {
            Some(Record::Conversation(record)) => Some(record.clone()),
            _ => None,
        })
    }

    pub fn saving_kind(&self, kind: Kind) -> bool {
        let blocked = self
            .0
            .values()
            .filter(|pending| pending.failure.is_some())
            .map(|pending| &pending.group)
            .collect::<BTreeSet<_>>();
        self.0
            .values()
            .any(|pending| pending.kind == kind && !blocked.contains(&pending.group))
    }

    pub fn edit(&mut self, id: String, expected: Option<Record>, draft: Option<Record>) {
        self.edit_record(Kind::Task, id, expected, draft);
    }

    pub fn edit_capture(&mut self, id: String, expected: Option<Record>, draft: Option<Record>) {
        self.edit_record(Kind::Capture, id, expected, draft);
    }

    pub fn edit_record(
        &mut self,
        kind: Kind,
        id: String,
        expected: Option<Record>,
        draft: Option<Record>,
    ) {
        // A task referring to an unsaved project must not outlive a failed
        // parent creation. This applies to every task entry point and keeps
        // capture/recurrence/calendar groups joined transitively.
        let project = match &draft {
            Some(Record::Task(task)) => task.project_id.clone(),
            _ => None,
        };
        match self.0.entry((kind, id.clone())) {
            Entry::Occupied(mut entry) => entry.get_mut().draft = draft,
            Entry::Vacant(entry) => {
                entry.insert(Pending {
                    group: uuid::Uuid::new_v4().to_string(),
                    id: id.clone(),
                    kind,
                    expected,
                    draft,
                    failure: None,
                });
            }
        }
        if let Some(project) = project {
            self.group_records(&[(kind, id), (Kind::Project, project)]);
        }
    }

    pub fn next(&self) -> Option<Pending> {
        let blocked = self
            .0
            .values()
            .filter(|pending| pending.failure.is_some())
            .map(|pending| &pending.group)
            .collect::<BTreeSet<_>>();
        self.0
            .values()
            .find(|pending| !blocked.contains(&pending.group))
            .cloned()
    }

    /// A queued upload must not resurrect bytes after its parent is deleted.
    /// Keep its original until any in-flight commit advances the baseline.
    pub fn delete_document_attachments(&mut self, document_id: &str) {
        for pending in self.0.values_mut() {
            if matches!(&pending.draft, Some(Record::Attachment(attachment)) if attachment.document_id == document_id)
            {
                pending.draft = None;
            }
        }
    }

    pub fn attachment(
        &self,
        id: &str,
    ) -> Option<std::sync::Arc<bruhs_models::KnowledgeAttachment>> {
        match &self.0.get(&(Kind::Attachment, id.into()))?.draft {
            Some(Record::Attachment(attachment)) => Some(attachment.clone()),
            _ => None,
        }
    }

    /// Join overlapping logical operations, including any earlier unsaved
    /// drafts. No member can commit separately until this group is saved.
    pub fn group(&mut self, ids: &[String]) {
        self.group_records(
            &ids.iter()
                .map(|id| (Kind::Task, id.clone()))
                .collect::<Vec<_>>(),
        );
    }

    pub fn group_records(&mut self, ids: &[(Kind, String)]) {
        let groups = ids
            .iter()
            .filter_map(|id| self.0.get(id).map(|pending| pending.group.clone()))
            .collect::<BTreeSet<_>>();
        if groups.len() < 2 {
            return;
        }
        let group = uuid::Uuid::new_v4().to_string();
        for pending in self.0.values_mut() {
            if groups.contains(&pending.group) {
                pending.group.clone_from(&group);
            }
        }
    }

    pub fn next_batch(&self) -> Option<Vec<Pending>> {
        let first = self.next()?;
        Some(
            self.0
                .values()
                .filter(|pending| pending.group == first.group)
                .cloned()
                .collect(),
        )
    }

    pub fn saved(&mut self, saved: Pending) -> bool {
        let key = (saved.kind, saved.id);
        let Some(pending) = self.0.get_mut(&key) else {
            return false;
        };
        if pending.draft == saved.draft && pending.group == saved.group {
            self.0.remove(&key);
            true
        } else {
            pending.expected = saved.draft;
            false
        }
    }

    pub fn failed(&mut self, id: &str, message: String, conflict: bool) {
        self.failed_record(Kind::Task, id, message, conflict);
    }

    pub fn failed_capture(&mut self, id: &str, message: String, conflict: bool) {
        self.failed_record(Kind::Capture, id, message, conflict);
    }

    pub fn failed_record(&mut self, kind: Kind, id: &str, message: String, conflict: bool) {
        let Some(group) = self
            .0
            .get(&(kind, id.into()))
            .map(|pending| pending.group.clone())
        else {
            return;
        };
        for pending in self.0.values_mut() {
            if pending.group == group {
                pending.failure = Some((message.clone(), conflict));
            }
        }
    }

    pub fn retry(&mut self) {
        for pending in self.0.values_mut() {
            if pending
                .failure
                .as_ref()
                .is_some_and(|(_, conflict)| !conflict)
            {
                pending.failure = None;
            }
        }
    }

    pub fn failure(&self) -> Option<&(String, bool)> {
        self.0
            .values()
            .filter_map(|pending| pending.failure.as_ref())
            .find(|(_, conflict)| !conflict)
            .or_else(|| self.0.values().find_map(|pending| pending.failure.as_ref()))
    }

    pub fn status(&self, kind: Kind, id: &str) -> crate::durable_metadata::SaveState {
        use crate::durable_metadata::SaveState;
        let Some(pending) = self.0.get(&(kind, id.to_owned())) else {
            return SaveState::Saved;
        };
        if let Some((message, conflict)) = self
            .0
            .values()
            .filter(|member| member.group == pending.group)
            .find_map(|member| member.failure.as_ref())
        {
            SaveState::Error {
                message: message.clone(),
                conflict: *conflict,
            }
        } else {
            SaveState::Saving
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn recovery_json(&self) -> String {
        #[derive(serde::Serialize)]
        struct Recovery<'a> {
            id: &'a str,
            original: Option<RecoveryRecord<'a>>,
            unsaved: Option<RecoveryRecord<'a>>,
            kind: Kind,
            atomic_group: &'a str,
            operation: &'static str,
        }
        // Include the original for failed deletions, not just a tombstone.
        // Serialize references directly; an intermediate JSON Value tree can
        // multiply memory usage for even ordinary inline attachment bytes.
        let records = self
            .0
            .values()
            .map(|pending| Recovery {
                id: &pending.id,
                original: pending.expected.as_ref().map(|record| RecoveryRecord {
                    record,
                    role: "original",
                }),
                unsaved: pending.draft.as_ref().map(|record| RecoveryRecord {
                    record,
                    role: "unsaved",
                }),
                kind: pending.kind,
                atomic_group: &pending.group,
                operation: if pending.draft.is_some() {
                    "save"
                } else {
                    "delete"
                },
            })
            .collect::<Vec<_>>();
        serde_json::to_string_pretty(&records)
            .unwrap_or_else(|_| "Recovery data could not be serialized. Keep this tab open.".into())
    }

    pub fn recovery_attachments(&self) -> Vec<RecoveryAttachment> {
        self.0
            .values()
            .flat_map(|pending| {
                [
                    ("original", pending.expected.as_ref()),
                    ("unsaved", pending.draft.as_ref()),
                ]
                .into_iter()
                .filter_map(|(role, record)| RecoveryAttachment::from_record(record?, role))
            })
            .collect()
    }
}

#[cfg(test)]
mod capture_tests;
#[cfg(test)]
mod event_tests;
#[cfg(test)]
mod group_tests;
#[cfg(test)]
mod inbox_tests;
#[cfg(test)]
mod knowledge_tests;
#[cfg(test)]
mod person_tests;
#[cfg(test)]
mod project_tests;
#[cfg(test)]
mod source_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod thread_tests;
