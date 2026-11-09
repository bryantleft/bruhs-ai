use super::*;
use std::sync::Arc;

impl TaskSaves {
    pub fn pending_attachment(self, id: &str) -> Option<Arc<KnowledgeAttachment>> {
        self.queue.with_untracked(|queue| queue.attachment(id))
    }
    pub fn finish_loading_documents(
        self,
        loaded: Vec<KnowledgeDocument>,
    ) -> Vec<KnowledgeDocument> {
        let mut merged = Vec::new();
        self.initial_documents
            .update_value(|edits| merged = edits.merge_by(loaded, |record| &record.id));
        merged
    }

    pub fn edit_document(
        self,
        id: String,
        expected: Option<KnowledgeDocument>,
        draft: Option<KnowledgeDocument>,
    ) {
        self.initial_documents
            .update_value(|edits| edits.edit(&id, draft.as_ref()));
        self.queue.update(|queue| {
            if draft.is_none() {
                queue.delete_document_attachments(&id);
            }
            let linked_sources = draft
                .as_ref()
                .map(|document| {
                    document
                        .citations
                        .iter()
                        .map(|citation| (Kind::SourceClip, citation.clip_id.clone()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let key = (Kind::Document, id.clone());
            queue.edit_record(
                Kind::Document,
                id,
                expected.map(Record::Document),
                draft.map(Record::Document),
            );
            let mut ids = linked_sources;
            ids.push(key);
            queue.group_records(&ids);
        });
        self.refresh_status();
        self.save();
    }

    pub fn capture_document(self, capture: CaptureRecord, document: KnowledgeDocument) {
        self.initial_documents
            .update_value(|edits| edits.edit(&document.id, Some(&document)));
        self.initial_captures
            .update_value(|edits| edits.edit(&capture.id, Some(&capture)));
        self.queue.update(|queue| {
            let ids = [
                (Kind::Document, document.id.clone()),
                (Kind::Capture, capture.id.clone()),
            ];
            queue.edit_record(
                Kind::Document,
                document.id.clone(),
                None,
                Some(Record::Document(document)),
            );
            queue.edit_capture(capture.id.clone(), None, Some(Record::Capture(capture)));
            queue.group_records(&ids);
        });
        self.refresh_status();
        self.save();
    }

    pub fn attach_document(
        self,
        original: KnowledgeDocument,
        document: KnowledgeDocument,
        attachment: KnowledgeAttachment,
    ) {
        self.initial_documents
            .update_value(|edits| edits.edit(&document.id, Some(&document)));
        self.queue.update(|queue| {
            let ids = [
                (Kind::Document, document.id.clone()),
                (Kind::Attachment, attachment.id.clone()),
            ];
            queue.edit_record(
                Kind::Document,
                document.id.clone(),
                Some(Record::Document(original)),
                Some(Record::Document(document)),
            );
            queue.edit_record(
                Kind::Attachment,
                attachment.id.clone(),
                None,
                Some(Record::Attachment(Arc::new(attachment))),
            );
            queue.group_records(&ids);
        });
        self.refresh_status();
        self.save();
    }
}
