//! Private, unsubmitted editor state. Keeping a draft never submits a write.
use bruhs_models::{RelationshipItemKind, RelationshipProfile};
use serde::{ser::SerializeMap, Serialize, Serializer};

pub(crate) type Fields = [String; 6];

pub(crate) fn profile_fields(profile: &RelationshipProfile) -> Fields {
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
pub(crate) struct EditorDraft {
    #[serde(serialize_with = "serialize_fields")]
    pub fields: Fields,
    #[serde(serialize_with = "serialize_fields")]
    pub baseline: Fields,
    pub item_kind: RelationshipItemKind,
    pub item_text: String,
    pub due_tomorrow: bool,
}

fn serialize_fields<S: Serializer>(fields: &Fields, serializer: S) -> Result<S::Ok, S::Error> {
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
    pub fn new(fields: Fields) -> Self {
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
    pub fn reconcile(&mut self, saved: Fields) {
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
    pub fn details_to_save(&self, mut current: Fields) -> Option<Fields> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(company: &str, notes: &str) -> Fields {
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
}
