//! Exact local reminder receipts. Undo restores relationship context, never a person.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::{
    RelationshipItem, RelationshipItemKind, RelationshipProfile, WorkflowOperation, WorkflowStep,
    WorkflowStepStatus,
};

/// Why a person reminder could not be recorded or undone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{}", self.message())]
pub enum PersonReminderError {
    NotAReminder,
    InvalidReminder,
    MissingReceipt,
    MissingTarget,
    InvalidReceipt,
    ReceiptMismatch,
    DuplicateProfiles,
    RelationshipChanged,
}

impl PersonReminderError {
    /// The user-facing sentence.
    pub const fn message(self) -> &'static str {
        match self {
            Self::NotAReminder => "This is not a person reminder.",
            Self::InvalidReminder => "The reviewed person reminder is invalid. No records were changed.",
            Self::MissingReceipt => "This reminder has no undo receipt. No records were changed.",
            Self::MissingTarget => "This reminder has no target ID. No records were changed.",
            Self::InvalidReceipt => "This reminder's undo receipt is invalid. No records were changed.",
            Self::ReceiptMismatch => "This reminder's undo receipt does not match the reviewed change. No records were changed.",
            Self::DuplicateProfiles => "Duplicate relationship profiles prevent safe undo. No records were changed.",
            Self::RelationshipChanged => "A relationship was edited, removed, or is unavailable. No records were changed.",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonReminderReceipt {
    before: Option<RelationshipProfile>,
    after: RelationshipProfile,
}

impl PersonReminderReceipt {
    /// The caller must resolve the exact person ID and reviewed name first.
    pub fn prepare(
        operation: &WorkflowOperation,
        before: Option<&RelationshipProfile>,
        now_ms: i64,
    ) -> Result<Self, PersonReminderError> {
        let WorkflowOperation::CreatePersonReminder {
            person_id,
            reminder,
            ..
        } = operation
        else {
            return Err(PersonReminderError::NotAReminder);
        };
        if !operation.is_valid() || before.is_some_and(|profile| profile.person_id != *person_id) {
            return Err(PersonReminderError::InvalidReminder);
        }
        let mut after = before
            .cloned()
            .unwrap_or_else(|| RelationshipProfile::new(person_id, now_ms));
        after.add_item(
            RelationshipItem::new(RelationshipItemKind::Reminder, reminder, now_ms),
            now_ms,
        );
        Ok(Self {
            before: before.cloned(),
            after,
        })
    }

    pub fn target_id(&self) -> &str {
        // Receipts are constructed with one new item, but decoded receipts can be malformed.
        self.after.items.first().map_or("", |item| item.id.as_str())
    }

    pub fn after(&self) -> &RelationshipProfile {
        &self.after
    }

    pub fn into_after(self) -> RelationshipProfile {
        self.after
    }

    fn matches_operation(&self, operation: &WorkflowOperation, target_id: &str) -> bool {
        let WorkflowOperation::CreatePersonReminder {
            person_id,
            reminder,
            ..
        } = operation
        else {
            return false;
        };
        let Some(item) = self.after.items.first() else {
            return false;
        };
        if !operation.is_valid()
            || target_id.is_empty()
            || item.id != target_id
            || self.after.person_id != *person_id
            || self.before.as_ref().is_some_and(|before| {
                before.person_id != *person_id || before.items.iter().any(|old| old.id == item.id)
            })
            || item.kind != RelationshipItemKind::Reminder
            || item.text != reminder.trim()
            || item.due_at_ms.is_some()
            || item.completed_at_ms.is_some()
            || item.source_record_id.is_some()
        {
            return false;
        }
        let mut expected = self
            .before
            .clone()
            .unwrap_or_else(|| RelationshipProfile::new(person_id, item.created_at_ms));
        expected.add_item(item.clone(), item.created_at_ms);
        expected == self.after
    }
}

#[derive(Debug, Default)]
pub struct PersonReminderRollback {
    snapshots: HashMap<String, String>,
    restored: HashMap<String, Option<RelationshipProfile>>,
}

impl PersonReminderRollback {
    /// Simulate all People steps in reverse before any domain mutates. This also
    /// supports several reminders for one person in the same workflow.
    pub fn prepare(
        steps: &[WorkflowStep],
        profiles: &[RelationshipProfile],
    ) -> Result<Self, PersonReminderError> {
        let mut plan = Self::default();
        for step in steps
            .iter()
            .rev()
            .filter(|step| step.status == WorkflowStepStatus::Completed)
        {
            let WorkflowOperation::CreatePersonReminder { person_id, .. } = &step.operation else {
                continue;
            };
            let snapshot = step
                .target_snapshot
                .as_ref()
                .ok_or(PersonReminderError::MissingReceipt)?;
            let target_id = step
                .target_id
                .as_deref()
                .ok_or(PersonReminderError::MissingTarget)?;
            let receipt: PersonReminderReceipt =
                serde_json::from_str(snapshot).map_err(|_| PersonReminderError::InvalidReceipt)?;
            if !receipt.matches_operation(&step.operation, target_id)
                || plan.snapshots.contains_key(target_id)
            {
                return Err(PersonReminderError::ReceiptMismatch);
            }
            let current = if let Some(restored) = plan.restored.get(person_id) {
                restored.as_ref()
            } else {
                let mut matching = profiles
                    .iter()
                    .filter(|profile| profile.person_id == *person_id);
                let current = matching.next();
                if matching.next().is_some() {
                    return Err(PersonReminderError::DuplicateProfiles);
                }
                current
            };
            if current != Some(receipt.after()) {
                return Err(PersonReminderError::RelationshipChanged);
            }
            plan.snapshots.insert(target_id.into(), snapshot.clone());
            plan.restored.insert(person_id.clone(), receipt.before);
        }
        Ok(plan)
    }

    pub fn snapshot(&self, target_id: &str) -> Option<String> {
        self.snapshots.get(target_id).cloned()
    }

    /// Exact replacements for a transactional store; `None` removes only the
    /// relationship profile introduced by this workflow, never the contact.
    pub fn restorations(&self) -> impl Iterator<Item = (&str, Option<&RelationshipProfile>)> {
        self.restored
            .iter()
            .map(|(id, profile)| (id.as_str(), profile.as_ref()))
    }

    /// Apply only after the caller's full-workflow lifecycle, grant and snapshot
    /// preflight, in the same synchronous state update / storage transaction.
    pub fn apply(mut self, profiles: &mut Vec<RelationshipProfile>) {
        if self.restored.is_empty() {
            return;
        }
        profiles.retain_mut(|profile| match self.restored.remove(&profile.person_id) {
            Some(Some(before)) => {
                *profile = before;
                true
            }
            Some(None) => false,
            None => true,
        });
    }
}
