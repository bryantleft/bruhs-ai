use super::*;
use bruhs_models::KnowledgeDocument;

#[test]
fn source_deletion_locks_documents_even_when_ui_knows_no_citations() {
    let writes = [Write {
        kind: Kind::SourceClip,
        id: "source",
        expected: None,
        draft: None,
    }];
    let guard = SourceGuard::new(&writes);
    assert_eq!(guard.deleted, BTreeSet::from(["source".into()]));
    assert_eq!(
        guard.stores().collect::<Vec<_>>(),
        vec!["source_clips", "knowledge_documents"]
    );
}

#[test]
fn only_new_citations_require_source_validation_not_ordinary_text_edits() {
    let mut original = KnowledgeDocument::new("Title", "Body", 1);
    original.citations.push(KnowledgeCitation {
        clip_id: "existing".into(),
        locator: String::new(),
        note: String::new(),
    });
    let mut draft = original.clone();
    draft.revise("Latest", "Latest body", vec![], 2);
    let expected = Record::Document(original);
    let text_only = Record::Document(draft.clone());
    let write = |draft| Write {
        kind: Kind::Document,
        id: "document",
        expected: Some(&expected),
        draft: Some(draft),
    };
    let guard = SourceGuard::new(&[write(&text_only)]);
    assert!(guard.added.is_empty());
    assert_eq!(guard.stores().count(), 0);
    draft.citations.push(KnowledgeCitation {
        clip_id: "new".into(),
        locator: String::new(),
        note: String::new(),
    });
    let record = Record::Document(draft);
    let guard = SourceGuard::new(&[write(&record)]);
    assert_eq!(guard.added, BTreeSet::from(["new".into()]));
    assert_eq!(guard.stores().collect::<Vec<_>>(), vec!["source_clips"]);
}
