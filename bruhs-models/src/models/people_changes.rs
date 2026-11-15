//! Wire formats for People deletes, restores, unmerges and alias edits, and
//! the undo receipts both surfaces keep for supported People changes.
//!
//! Undo follows the workflow reminder rule: it reverts a change only while
//! the records it touched are exactly as the change left them. Otherwise it
//! refuses and changes nothing.

use serde::{Deserialize, Serialize};

use super::{PersonLinkSet, PersonProfile, RelationshipProfile};

/// A meeting speaker that was linked to a deleted contact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeetingSpeakerRef {
    pub meeting_id: String,
    pub speaker_id: String,
}

/// A summary action item that was assigned to a deleted contact. The text is
/// kept so a restore never reassigns an item that was rewritten since.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionItemRef {
    pub meeting_id: String,
    pub index: usize,
    pub text: String,
}

/// Reply to `DELETE /api/people/{id}`: the contact as it was, with every
/// meeting link the delete removed. The server keeps this record so
/// `POST /api/people/{id}/restore` can put the contact and those links back.
///
/// Cascade: the contact's observations and aliases go with it. Speakers and
/// action items stay in their meetings, unlinked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonDeletion {
    pub person: PersonProfile,
    #[serde(default)]
    pub speakers: Vec<MeetingSpeakerRef>,
    #[serde(default)]
    pub action_items: Vec<ActionItemRef>,
    pub deleted_at: i64,
}

/// Reply to `POST /api/people/{source_id}/unmerge`: both contacts as they are
/// after the merge was undone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonUnmerge {
    pub source: PersonProfile,
    pub target: PersonProfile,
}

/// Body of `PUT /api/people/{id}/aliases`: the complete alias list to save.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonAliasesBody {
    pub aliases: Vec<String>,
}

/// Why an undo was refused. Every refusal changed nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{}", self.message())]
pub enum PeopleUndoError {
    /// The profile was edited, removed or replaced after the change.
    ProfileChanged,
    /// The relationship details or reminders changed after the change.
    RelationshipChanged,
    /// The contact already exists again, so there is nothing to restore.
    AlreadyRestored,
    /// Contacts are still loading, syncing or recovering a merge.
    Busy,
}

impl PeopleUndoError {
    /// The user-facing sentence.
    pub const fn message(self) -> &'static str {
        match self {
            Self::ProfileChanged => {
                "The profile changed after this action, so it cannot be undone. No records were changed."
            }
            Self::RelationshipChanged => {
                "The relationship details changed after this action, so it cannot be undone. No records were changed."
            }
            Self::AlreadyRestored => {
                "This contact already exists again. No records were changed."
            }
            Self::Busy => {
                "Contacts are still loading or syncing. Try undo again in a moment. No records were changed."
            }
        }
    }
}

/// The latest People change a surface can still undo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[expect(
    clippy::large_enum_variant,
    reason = "each surface holds at most one receipt at a time, so boxing buys nothing"
)]
pub enum PeopleUndo {
    Rename {
        person_id: String,
        before: String,
        after: String,
    },
    Tracked {
        person_id: String,
        name: String,
        before: bool,
        after: bool,
    },
    Merge {
        source_id: String,
        source_name: String,
        target_id: String,
        target_name: String,
        before_source: Option<RelationshipProfile>,
        before_target: Option<RelationshipProfile>,
        after_relationship: Option<RelationshipProfile>,
        /// Private links the merge moved from the source to the target.
        links: PersonLinkSet,
    },
    Delete {
        person: PersonProfile,
        relationship: Option<RelationshipProfile>,
        /// Private links the delete removed.
        links: PersonLinkSet,
        /// The contact never reached the server, so undo is local only.
        local_only: bool,
    },
}

impl PeopleUndo {
    /// What happened, in one sentence.
    pub fn summary(&self) -> String {
        match self {
            Self::Rename { before, after, .. } => format!("Renamed {before} to {after}."),
            Self::Tracked {
                name, after: false, ..
            } => format!("Stopped tracking {name}."),
            Self::Tracked { name, .. } => format!("Started tracking {name}."),
            Self::Merge {
                source_name,
                target_name,
                ..
            } => format!("Merged {source_name} into {target_name}."),
            Self::Delete { person, .. } => format!("Deleted {}.", person.display_name),
        }
    }

    /// The undo button's visible label.
    pub const fn action_label(&self) -> &'static str {
        match self {
            Self::Rename { .. } => "Undo rename",
            Self::Tracked { .. } => "Undo tracking change",
            Self::Merge { .. } => "Undo merge",
            Self::Delete { .. } => "Undo delete",
        }
    }

    /// The profile ids this undo would change.
    pub fn person_ids(&self) -> Vec<&str> {
        match self {
            Self::Rename { person_id, .. } | Self::Tracked { person_id, .. } => {
                vec![person_id.as_str()]
            }
            Self::Merge {
                source_id,
                target_id,
                ..
            } => vec![source_id.as_str(), target_id.as_str()],
            Self::Delete { person, .. } => vec![person.id.as_str()],
        }
    }

    /// Whether undo still applies to the current records. `find_person` and
    /// `find_relationship` look up the current profile and relationship by id;
    /// the caller resolves duplicates before calling.
    pub fn check<'p, 'r>(
        &self,
        find_person: impl Fn(&str) -> Option<&'p PersonProfile>,
        find_relationship: impl Fn(&str) -> Option<&'r RelationshipProfile>,
    ) -> Result<(), PeopleUndoError> {
        match self {
            Self::Rename {
                person_id, after, ..
            } => match find_person(person_id) {
                Some(current) if current.display_name == *after => Ok(()),
                _ => Err(PeopleUndoError::ProfileChanged),
            },
            Self::Tracked {
                person_id, after, ..
            } => match find_person(person_id) {
                Some(current) if current.tracked == *after => Ok(()),
                _ => Err(PeopleUndoError::ProfileChanged),
            },
            Self::Merge {
                source_id,
                target_id,
                after_relationship,
                ..
            } => {
                if find_person(source_id).is_some() {
                    return Err(PeopleUndoError::AlreadyRestored);
                }
                if find_person(target_id).is_none() {
                    return Err(PeopleUndoError::ProfileChanged);
                }
                if find_relationship(source_id).is_some()
                    || find_relationship(target_id) != after_relationship.as_ref()
                {
                    return Err(PeopleUndoError::RelationshipChanged);
                }
                Ok(())
            }
            Self::Delete { person, .. } => {
                if find_person(&person.id).is_some() {
                    return Err(PeopleUndoError::AlreadyRestored);
                }
                if find_relationship(&person.id).is_some() {
                    return Err(PeopleUndoError::RelationshipChanged);
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests;
