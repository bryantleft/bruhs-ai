use std::collections::BTreeMap;

use crate::state::WorkItem;

/// Keep edits until the initial snapshot is published, including edits whose
/// writes have already completed. A save acknowledgment cannot retire this
/// journal: the in-flight load may still hold an older snapshot.
pub(crate) struct InitialEdits<T = WorkItem>(Option<BTreeMap<String, Option<T>>>);

impl<T> Default for InitialEdits<T> {
    fn default() -> Self {
        Self(Some(BTreeMap::new()))
    }
}

impl<T: Clone> InitialEdits<T> {
    pub fn edit(&mut self, id: &str, draft: Option<&T>) {
        if let Some(edits) = self.0.as_mut() {
            edits.insert(id.to_owned(), draft.cloned());
        }
    }

    pub fn merge_by(&mut self, loaded: Vec<T>, id: impl Fn(&T) -> &str) -> Vec<T> {
        let Some(mut edits) = self.0.take() else {
            return loaded;
        };
        let mut merged = loaded
            .into_iter()
            .filter_map(|item| edits.remove(id(&item)).unwrap_or(Some(item)))
            .collect::<Vec<_>>();
        merged.extend(edits.into_values().flatten());
        merged
    }
}

impl InitialEdits<WorkItem> {
    pub fn merge(&mut self, loaded: Vec<WorkItem>) -> Vec<WorkItem> {
        self.merge_by(loaded, |item| &item.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_journal_keeps_new_records_and_deletion_across_initial_load() {
        let original = crate::state::Project::new("Original");
        let created = crate::state::Project::new("Created during load");
        let mut edits = InitialEdits::default();
        edits.edit(&original.id, None);
        edits.edit(&created.id, Some(&created));
        assert_eq!(
            edits.merge_by(vec![original], |record| &record.id),
            vec![created]
        );
    }

    #[test]
    fn source_journal_keeps_latest_creation_and_deletion_across_loading() {
        let mut source =
            bruhs_models::SourceClip::new("Original", "https://example.invalid", "Excerpt", 1);
        let old = source.clone();
        source.title = "Latest".into();
        let deleted =
            bruhs_models::SourceClip::new("Deleted", "https://example.invalid/deleted", "", 1);
        let mut edits = InitialEdits::default();
        edits.edit(&source.id, Some(&source));
        edits.edit(&deleted.id, None);
        assert_eq!(
            edits.merge_by(vec![old, deleted], |record| &record.id),
            vec![source]
        );
    }

    #[test]
    fn document_journal_keeps_latest_attachment_metadata_and_deletion() {
        let original = bruhs_models::KnowledgeDocument::new("Original", "Body", 1);
        let mut latest = original.clone();
        latest.revise("Latest", "Latest body", vec![], 2);
        latest.attachments.push(bruhs_models::AttachmentMeta {
            id: "file".into(),
            file_name: "file.txt".into(),
            mime_type: "text/plain".into(),
            size_bytes: 3,
        });
        let deleted = bruhs_models::KnowledgeDocument::new("Deleted", "", 1);
        let mut edits = InitialEdits::default();
        edits.edit(&latest.id, Some(&latest));
        edits.edit(&deleted.id, None);
        assert_eq!(
            edits.merge_by(vec![original, deleted], |record| &record.id),
            vec![latest]
        );
    }

    #[test]
    fn event_journal_retains_reschedule_and_deletion_across_initial_loading() {
        let original: crate::state::CalendarEvent = serde_json::from_value(serde_json::json!({
            "id": "event", "title": "Original", "start_at_ms": 1000.0,
            "end_at_ms": 2000.0, "all_day": false, "created_at": 1, "updated_at": 2
        }))
        .unwrap();
        let mut latest = original.clone();
        latest.start_at_ms = 3000.0;
        latest.end_at_ms = 4000.0;
        let mut deleted = original.clone();
        deleted.id = "deleted".into();
        let mut edits = InitialEdits::default();
        edits.edit(&latest.id, Some(&latest));
        edits.edit(&deleted.id, None);
        assert_eq!(
            edits.merge_by(vec![original, deleted], |event| &event.id),
            vec![latest]
        );
        assert!(edits.0.is_none());
    }

    #[test]
    fn inbox_journal_overlays_latest_draft_and_deletion_on_initial_snapshot() {
        let mut edits = InitialEdits::default();
        let original = crate::state::InboxConversation::local_draft("Recipient", "Original", 10);
        let deleted = crate::state::InboxConversation::local_draft("Other", "Deleted", 10);
        let mut latest = original.clone();
        latest.upsert_draft("Latest", 20);
        edits.edit(&latest.id, Some(&latest));
        edits.edit(&deleted.id, None);
        assert_eq!(
            edits.merge_by(vec![original, deleted], |record| &record.id),
            vec![latest]
        );
        assert!(edits.0.is_none());
    }

    #[test]
    fn capture_journal_preserves_latest_edits_across_an_older_load_snapshot() {
        let mut edits = InitialEdits::default();
        let original =
            crate::state::CaptureRecord::new(bruhs_models::CaptureKind::Task, "Original", 10)
                .unwrap();
        let mut latest = original.clone();
        latest.title = "Latest".into();
        edits.edit(&latest.id, Some(&latest));
        assert_eq!(
            edits.merge_by(vec![original], |record| &record.id),
            vec![latest]
        );
        assert!(edits.0.is_none());
    }

    fn task(id: &str, title: &str) -> WorkItem {
        WorkItem {
            id: id.into(),
            title: title.into(),
            project_id: None,
            description: None,
            status: crate::state::WorkItemStatus::Todo,
            priority: crate::state::Priority::None,
            due_date: None,
            planning: Default::default(),
            created_at: 1,
            updated_at: 2,
        }
    }

    #[test]
    fn retains_created_records_missing_from_an_older_load_snapshot() {
        let mut edits = InitialEdits::default();
        let created = task("new", "created while loading");
        edits.edit(&created.id, Some(&created));
        assert_eq!(
            edits.merge(vec![task("old", "saved")]),
            vec![task("old", "saved"), created]
        );
    }

    #[test]
    fn latest_edit_wins_even_if_the_snapshot_already_contains_the_created_id() {
        let mut edits = InitialEdits::default();
        let created = task("new", "first");
        let latest = task("new", "latest");
        edits.edit(&created.id, Some(&created));
        edits.edit(&latest.id, Some(&latest));
        assert_eq!(edits.merge(vec![created]), vec![latest]);
    }

    #[test]
    fn deletion_tombstone_prevents_a_stale_load_from_resurrecting_a_task() {
        let mut edits = InitialEdits::default();
        let created = task("new", "removed while loading");
        edits.edit(&created.id, Some(&created));
        edits.edit(&created.id, None);
        assert!(edits.merge(vec![created]).is_empty());
    }

    #[test]
    fn journal_is_retired_only_after_a_successful_merge() {
        let mut edits = InitialEdits::default();
        let created = task("new", "kept through load failure");
        edits.edit(&created.id, Some(&created));
        // Failed loads never call merge; Retry still overlays the latest edit.
        assert!(edits.0.is_some());
        assert_eq!(edits.merge(Vec::new()), vec![created.clone()]);
        edits.edit(&created.id, Some(&task("new", "later ordinary edit")));
        assert!(edits.0.is_none());
    }
}
