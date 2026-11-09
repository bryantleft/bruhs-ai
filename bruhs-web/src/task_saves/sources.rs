use super::*;

impl TaskSaves {
    pub fn finish_loading_sources(self, loaded: Vec<SourceClip>) -> Vec<SourceClip> {
        let mut merged = Vec::new();
        self.initial_sources
            .update_value(|edits| merged = edits.merge_by(loaded, |record| &record.id));
        merged
    }

    pub fn edit_source(self, expected: Option<SourceClip>, draft: SourceClip) {
        self.initial_sources
            .update_value(|edits| edits.edit(&draft.id, Some(&draft)));
        self.queue.update(|queue| {
            queue.edit_record(
                Kind::SourceClip,
                draft.id.clone(),
                expected.map(Record::SourceClip),
                Some(Record::SourceClip(draft)),
            )
        });
        self.refresh_status();
        self.save();
    }

    /// A source deletion and every known citation removal are one user action.
    /// Join pending document edits/uploads instead of committing partial links.
    pub fn delete_source(
        self,
        source: SourceClip,
        documents: Vec<(KnowledgeDocument, KnowledgeDocument)>,
    ) {
        self.initial_sources
            .update_value(|edits| edits.edit(&source.id, None));
        self.initial_documents.update_value(|edits| {
            for (_, draft) in &documents {
                edits.edit(&draft.id, Some(draft));
            }
        });
        self.queue.update(|queue| {
            let mut ids = vec![(Kind::SourceClip, source.id.clone())];
            queue.edit_record(
                Kind::SourceClip,
                source.id.clone(),
                Some(Record::SourceClip(source)),
                None,
            );
            for (original, draft) in documents {
                ids.push((Kind::Document, draft.id.clone()));
                queue.edit_record(
                    Kind::Document,
                    draft.id.clone(),
                    Some(Record::Document(original)),
                    Some(Record::Document(draft)),
                );
            }
            queue.group_records(&ids);
        });
        self.refresh_status();
        self.save();
    }
}
