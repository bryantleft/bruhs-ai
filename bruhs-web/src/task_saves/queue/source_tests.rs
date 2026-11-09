use super::*;
use bruhs_models::{KnowledgeCitation, KnowledgeDocument, SourceClip};

fn source() -> SourceClip {
    let mut source = SourceClip::new("Source", "https://example.invalid/source", "Excerpt", 1);
    source.id = "source".into();
    source
}

fn document(id: &str) -> KnowledgeDocument {
    let mut document = KnowledgeDocument::new("Citing document", "Body", 1);
    document.id = id.into();
    document.citations.push(KnowledgeCitation {
        clip_id: "source".into(),
        locator: String::new(),
        note: String::new(),
    });
    document
}

fn delete_group(queue: &mut Queue) {
    queue.edit_record(
        Kind::SourceClip,
        "source".into(),
        Some(Record::SourceClip(source())),
        None,
    );
    for id in ["first", "second"] {
        let original = document(id);
        let mut draft = original.clone();
        draft.update_citations(Vec::new(), 2);
        queue.edit_record(
            Kind::Document,
            id.into(),
            Some(Record::Document(original)),
            Some(Record::Document(draft)),
        );
    }
    queue.group_records(&[
        (Kind::SourceClip, "source".into()),
        (Kind::Document, "first".into()),
        (Kind::Document, "second".into()),
    ]);
}

#[test]
fn failed_source_deletion_retries_two_citation_removals_and_latest_text_together() {
    let mut queue = Queue::default();
    delete_group(&mut queue);
    queue.failed_record(Kind::SourceClip, "source", "aborted".into(), false);
    let mut latest = document("first");
    latest.update_citations(Vec::new(), 2);
    let original = latest.clone();
    latest.revise("Latest title", "Latest body", vec![], 3);
    queue.edit_record(
        Kind::Document,
        "first".into(),
        Some(Record::Document(original)),
        Some(Record::Document(latest.clone())),
    );
    assert!(queue.next_batch().is_none());
    queue.retry();
    let batch = queue.next_batch().unwrap();
    assert_eq!(batch.len(), 3);
    assert_eq!(batch[0].expected, Some(Record::Document(document("first"))));
    assert_eq!(batch[0].draft, Some(Record::Document(latest)));
    assert_eq!(batch[2].expected, Some(Record::SourceClip(source())));
    assert!(batch[2].draft.is_none());
    for item in batch {
        assert!(queue.saved(item));
    }
    assert!(queue.is_empty());
}

#[test]
fn conflict_in_any_citing_document_blocks_the_whole_source_deletion() {
    let mut queue = Queue::default();
    delete_group(&mut queue);
    queue.failed_record(Kind::Document, "second", "conflict".into(), true);
    queue.retry();
    assert!(queue.next_batch().is_none());
    let recovery: serde_json::Value = serde_json::from_str(&queue.recovery_json()).unwrap();
    assert_eq!(recovery.as_array().unwrap().len(), 3);
    assert!(recovery
        .as_array()
        .unwrap()
        .iter()
        .all(|record| record["atomic_group"] == recovery[0]["atomic_group"]));
    assert_eq!(recovery[2]["kind"], "source_clip");
    assert_eq!(
        recovery[2]["original"]["source_url"],
        "https://example.invalid/source"
    );
    assert!(recovery[2]["unsaved"].is_null());
}

#[test]
fn a_new_document_citing_failed_source_creation_joins_its_retry() {
    let mut queue = Queue::default();
    queue.edit_record(
        Kind::SourceClip,
        "source".into(),
        None,
        Some(Record::SourceClip(source())),
    );
    queue.failed_record(Kind::SourceClip, "source", "aborted".into(), false);
    queue.edit_record(
        Kind::Document,
        "first".into(),
        None,
        Some(Record::Document(document("first"))),
    );
    queue.group_records(&[
        (Kind::SourceClip, "source".into()),
        (Kind::Document, "first".into()),
    ]);
    assert!(queue.next_batch().is_none());
    queue.retry();
    let batch = queue.next_batch().unwrap();
    assert_eq!(batch.len(), 2);
    assert!(batch.iter().all(|record| record.expected.is_none()));
}

#[test]
fn source_wire_shape_remains_untagged_and_decodable() {
    let source = source();
    let json = serde_json::to_value(Record::SourceClip(source.clone())).unwrap();
    assert!(json.get("SourceClip").is_none());
    assert_eq!(serde_json::from_value::<SourceClip>(json).unwrap(), source);
}
