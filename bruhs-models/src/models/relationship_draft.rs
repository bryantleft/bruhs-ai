//! A contact's unsubmitted relationship editor state. Each app keeps these
//! while it runs (web: per tab; desktop: until it quits), so moving between
//! contacts never loses an edit. Keeping a draft never submits a write.
//! Moved from the web's `relationship_store/drafts.rs` so both apps keep,
//! reconcile and save drafts the same way.

use std::collections::BTreeMap;

use serde::{ser::SerializeMap, Serialize, Serializer};

use crate::{RelationshipItemKind, RelationshipProfile};

/// Company, role, email, phone, location and notes, in that order.
pub type RelationshipFields = [String; 6];

/// How many drafts the unsaved list names; recovery data includes all.
pub const DRAFT_LIST_LIMIT: usize = 24;

pub fn profile_fields(profile: &RelationshipProfile) -> RelationshipFields {
    [
        profile.company.clone(),
        profile.role.clone(),
        profile.email.clone(),
        profile.phone.clone(),
        profile.location.clone(),
        profile.notes.clone(),
    ]
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EditorDraft {
    #[serde(serialize_with = "serialize_fields")]
    pub fields: RelationshipFields,
    /// The saved values the fields were edited from.
    #[serde(serialize_with = "serialize_fields")]
    pub baseline: RelationshipFields,
    pub item_kind: RelationshipItemKind,
    pub item_text: String,
    pub due_tomorrow: bool,
}

fn serialize_fields<S: Serializer>(
    fields: &RelationshipFields,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let mut map = serializer.serialize_map(Some(6))?;
    for (name, field) in ["company", "role", "email", "phone", "location", "notes"]
        .into_iter()
        .zip(fields)
    {
        map.serialize_entry(name, field)?;
    }
    map.end()
}

impl EditorDraft {
    pub fn new(fields: RelationshipFields) -> Self {
        Self {
            baseline: fields.clone(),
            fields,
            item_kind: RelationshipItemKind::Interaction,
            item_text: String::new(),
            due_tomorrow: false,
        }
    }

    pub fn dirty(&self) -> bool {
        self.fields
            .iter()
            .zip(&self.baseline)
            .any(|(field, baseline)| field.trim() != baseline.trim())
            || !self.item_text.trim().is_empty()
    }

    /// Refresh untouched fields without replacing a conflicting private edit or
    /// its original comparison point. A matching commit acknowledges that field.
    pub fn reconcile(&mut self, saved: RelationshipFields) {
        for ((field, baseline), saved) in self.fields.iter_mut().zip(&mut self.baseline).zip(saved)
        {
            if field.trim() == baseline.trim() || field.trim() == saved.trim() {
                field.clone_from(&saved);
                *baseline = saved;
            }
        }
    }

    /// Only edited fields replace current data. Conflicting changes to the same
    /// field require review; unrelated newer fields are retained.
    pub fn details_to_save(&self, mut current: RelationshipFields) -> Option<RelationshipFields> {
        for ((field, baseline), current) in self.fields.iter().zip(&self.baseline).zip(&mut current)
        {
            if field.trim() == baseline.trim() {
                continue;
            }
            if current.trim() != baseline.trim() && current.trim() != field.trim() {
                return None;
            }
            *current = field.trim().to_string();
        }
        Some(current)
    }
}

/// Every contact's kept draft, by person id.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RelationshipDrafts(BTreeMap<String, EditorDraft>);

impl RelationshipDrafts {
    pub fn get(&self, id: &str) -> Option<&EditorDraft> {
        self.0.get(id)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.0.contains_key(id)
    }

    /// Whether [`Self::keep`] would leave the drafts as they are.
    pub fn unchanged_by(&self, id: &str, draft: &EditorDraft) -> bool {
        if draft.dirty() {
            self.0.get(id) == Some(draft)
        } else {
            !self.0.contains_key(id)
        }
    }

    /// Keep a draft with unsaved edits; a clean one drops the contact's
    /// draft. Returns whether anything changed.
    pub fn keep(&mut self, id: String, draft: EditorDraft) -> bool {
        if self.unchanged_by(&id, &draft) {
            return false;
        }
        if draft.dirty() {
            self.0.insert(id, draft);
        } else {
            self.0.remove(&id);
        }
        true
    }

    pub fn discard(&mut self, id: &str) -> bool {
        self.0.remove(id).is_some()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The contacts the unsaved list names: the first [`DRAFT_LIST_LIMIT`].
    pub fn listed_ids(&self) -> Vec<String> {
        self.0.keys().take(DRAFT_LIST_LIMIT).cloned().collect()
    }

    /// Every draft, with its original comparison fields, as private
    /// recovery data to copy before discarding.
    pub fn recovery_json(&self) -> String {
        serde_json::to_string_pretty(&self.0).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(company: &str, notes: &str) -> RelationshipFields {
        [
            company.into(),
            "".into(),
            "".into(),
            "".into(),
            "".into(),
            notes.into(),
        ]
    }

    #[test]
    fn clean_and_whitespace_only_edits_are_not_pending() {
        let mut draft = EditorDraft::new(fields("Original", "Notes"));
        assert!(!draft.dirty());
        draft.fields[0] = " Original ".into();
        draft.item_text = " \n ".into();
        assert!(!draft.dirty());
    }

    #[test]
    fn snapshot_keeps_exact_text_and_item_controls_without_submitting() {
        let mut draft = EditorDraft::new(fields("Original", "Notes"));
        draft.fields[5] = "  Private\ntext 💬  ".into();
        draft.item_text = " Unsubmitted reminder ".into();
        draft.item_kind = RelationshipItemKind::Reminder;
        draft.due_tomorrow = true;
        assert!(draft.dirty());
        let retained = draft.clone();
        assert_eq!(retained, draft);
        assert_eq!(retained.fields[5], "  Private\ntext 💬  ");
        assert_eq!(retained.baseline[5], "Notes");
    }

    #[test]
    fn unrelated_newer_fields_survive_an_explicit_details_save() {
        let mut draft = EditorDraft::new(fields("Original", "Notes"));
        draft.fields[5] = " My edit ".into();
        let current = fields("New company", "Notes");
        assert_eq!(
            draft.details_to_save(current.clone()),
            Some(fields("New company", "My edit"))
        );
        draft.reconcile(current);
        assert_eq!(draft.fields, fields("New company", " My edit "));
        assert!(draft.dirty());
    }

    #[test]
    fn same_field_conflict_retains_local_text_and_original_baseline() {
        let mut draft = EditorDraft::new(fields("Original", "Notes"));
        draft.fields[5] = "My edit".into();
        let current = fields("Original", "External edit");
        draft.reconcile(current.clone());
        assert_eq!(draft.fields[5], "My edit");
        assert_eq!(draft.baseline[5], "Notes");
        assert_eq!(draft.details_to_save(current), None);
        assert!(draft.dirty());
    }

    #[test]
    fn acknowledged_details_do_not_clear_an_unsubmitted_item() {
        let mut draft = EditorDraft::new(fields("Original", "Notes"));
        draft.fields[5] = "My edit".into();
        draft.item_text = "Keep this reminder".into();
        draft.reconcile(fields("Original", "My edit"));
        assert!(draft.dirty());
        assert_eq!(draft.item_text, "Keep this reminder");
        draft.item_text.clear();
        assert!(!draft.dirty());
    }

    #[test]
    fn private_recovery_names_fields_and_keeps_the_original_comparison() {
        let mut draft = EditorDraft::new(fields("Original", "Notes"));
        draft.fields[5] = "Private edit".into();
        let recovery = serde_json::to_value(draft).unwrap();
        assert_eq!(recovery["fields"]["notes"], "Private edit");
        assert_eq!(recovery["baseline"]["notes"], "Notes");
        assert_eq!(recovery["fields"]["company"], "Original");
    }

    #[test]
    fn drafts_keep_only_unsaved_edits_and_list_the_first_ones() {
        let mut drafts = RelationshipDrafts::default();
        let clean = EditorDraft::new(fields("Original", "Notes"));
        assert!(!drafts.keep("ada".into(), clean.clone()));
        let mut edited = clean.clone();
        edited.fields[5] = "Edit".into();
        assert!(drafts.keep("ada".into(), edited.clone()));
        assert!(!drafts.keep("ada".into(), edited));
        assert!(drafts.contains("ada"));
        assert!(drafts.keep("ada".into(), clean), "a clean draft drops it");
        assert!(drafts.is_empty());

        for index in 0..30 {
            let mut draft = EditorDraft::new(fields("", ""));
            draft.item_text = format!("Item {index}");
            drafts.keep(format!("p{index:02}"), draft);
        }
        assert_eq!(drafts.len(), 30);
        assert_eq!(drafts.listed_ids().len(), DRAFT_LIST_LIMIT);
        assert_eq!(drafts.listed_ids()[0], "p00");
        let recovery: serde_json::Value = serde_json::from_str(&drafts.recovery_json()).unwrap();
        assert_eq!(recovery.as_object().map(serde_json::Map::len), Some(30));
        assert!(drafts.discard("p00"));
        assert!(!drafts.discard("p00"));
    }
}
