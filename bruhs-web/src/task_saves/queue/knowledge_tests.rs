use super::*;
use bruhs_models::{KnowledgeAttachment, KnowledgeDocument};
use std::sync::Arc;

fn upload(queue: &mut Queue) -> (KnowledgeDocument, Arc<KnowledgeAttachment>) {
    let mut original = KnowledgeDocument::new("Original", "Body", 1);
    original.id = "document".into();
    let attachment = Arc::new(KnowledgeAttachment {
        id: "attachment".into(),
        document_id: original.id.clone(),
        file_name: "fixture.bin".into(),
        mime_type: "application/octet-stream".into(),
        bytes: vec![0, 128, 255],
        created_at_ms: 2,
    });
    let mut draft = original.clone();
    draft.attachments.push(attachment.metadata());
    queue.edit_record(
        Kind::Document,
        draft.id.clone(),
        Some(Record::Document(original)),
        Some(Record::Document(draft.clone())),
    );
    queue.edit_record(
        Kind::Attachment,
        attachment.id.clone(),
        None,
        Some(Record::Attachment(attachment.clone())),
    );
    queue.group_records(&[
        (Kind::Document, draft.id.clone()),
        (Kind::Attachment, attachment.id.clone()),
    ]);
    (draft, attachment)
}

#[test]
fn upload_failure_retains_latest_text_and_shared_bytes_for_retry() {
    let mut queue = Queue::default();
    let (draft, bytes) = upload(&mut queue);
    let in_flight = queue.next_batch().unwrap();
    let mut latest = draft.clone();
    latest.revise("Latest", "Latest body", vec![], 3);
    queue.edit_record(
        Kind::Document,
        latest.id.clone(),
        Some(Record::Document(draft)),
        Some(Record::Document(latest.clone())),
    );
    queue.failed_record(Kind::Attachment, "attachment", "aborted".into(), false);
    assert!(queue.next_batch().is_none());
    assert!(Arc::ptr_eq(
        &bytes,
        &queue.attachment("attachment").unwrap()
    ));
    let Some(Record::Attachment(in_flight_bytes)) = &in_flight[1].draft else {
        panic!("attachment");
    };
    assert!(Arc::ptr_eq(&bytes, in_flight_bytes));
    queue.retry();
    let retried = queue.next_batch().unwrap();
    assert_eq!(retried[0].draft, Some(Record::Document(latest)));
    assert_eq!(retried[1].draft, Some(Record::Attachment(bytes)));
    for pending in retried {
        assert!(queue.saved(pending));
    }
    assert!(queue.is_empty());
}

#[test]
fn delete_during_upload_waits_for_ack_then_deletes_parent_and_bytes() {
    let mut queue = Queue::default();
    let (draft, attachment) = upload(&mut queue);
    let in_flight = queue.next_batch().unwrap();
    queue.delete_document_attachments(&draft.id);
    queue.edit_record(
        Kind::Document,
        draft.id.clone(),
        Some(Record::Document(draft.clone())),
        None,
    );
    for pending in in_flight {
        assert!(!queue.saved(pending));
    }
    let deletion = queue.next_batch().unwrap();
    assert_eq!(deletion.len(), 2);
    assert_eq!(deletion[0].expected, Some(Record::Document(draft)));
    assert_eq!(deletion[1].expected, Some(Record::Attachment(attachment)));
    assert!(deletion.iter().all(|pending| pending.draft.is_none()));
    assert!(queue.attachment("attachment").is_none());
    for pending in deletion {
        assert!(queue.saved(pending));
    }
    assert!(queue.is_empty());
}

#[test]
fn delete_after_failed_upload_never_retries_an_orphan() {
    let mut queue = Queue::default();
    let (draft, _) = upload(&mut queue);
    queue.failed_record(Kind::Attachment, "attachment", "aborted".into(), false);
    queue.delete_document_attachments(&draft.id);
    queue.edit_record(
        Kind::Document,
        draft.id.clone(),
        Some(Record::Document(draft)),
        None,
    );
    assert!(queue.next_batch().is_none());
    queue.retry();
    let deletion = queue.next_batch().unwrap();
    assert!(deletion[1].expected.is_none());
    assert!(deletion.iter().all(|pending| pending.draft.is_none()));
}

#[test]
fn note_capture_and_upload_share_conflict_and_recovery() {
    let mut queue = Queue::default();
    let (document, _) = upload(&mut queue);
    let mut capture =
        crate::state::CaptureRecord::new(bruhs_models::CaptureKind::Note, "Note", 1).unwrap();
    capture.id = "capture".into();
    capture.target_id = Some(document.id.clone());
    queue.edit_capture(capture.id.clone(), None, Some(Record::Capture(capture)));
    queue.group_records(&[
        (Kind::Document, document.id),
        (Kind::Capture, "capture".into()),
    ]);
    queue.failed_capture("capture", "conflict".into(), true);
    queue.retry();
    assert!(queue.next_batch().is_none());
    let recovery: serde_json::Value = serde_json::from_str(&queue.recovery_json()).unwrap();
    assert_eq!(recovery.as_array().unwrap().len(), 3);
    assert!(recovery
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item["atomic_group"] == recovery[0]["atomic_group"]));
    assert_eq!(
        recovery[2]["unsaved"]["bytes"],
        serde_json::json!([0, 128, 255])
    );
}

#[test]
fn attachment_json_keeps_legacy_byte_array_and_document_wire_shape() {
    let mut queue = Queue::default();
    let (document, attachment) = upload(&mut queue);
    let json = serde_json::to_value(Record::Attachment(attachment.clone())).unwrap();
    assert_eq!(json["bytes"], serde_json::json!([0, 128, 255]));
    assert_eq!(
        serde_json::from_value::<KnowledgeAttachment>(json).unwrap(),
        *attachment
    );
    let json = serde_json::to_value(Record::Document(document.clone())).unwrap();
    assert_eq!(
        serde_json::from_value::<KnowledgeDocument>(json).unwrap(),
        document
    );
}

#[test]
fn large_recovery_is_compact_and_each_reference_retains_exact_shared_bytes() {
    let mut queue = Queue::default();
    let (_, original) = upload(&mut queue);
    let mut large = original.as_ref().clone();
    large.bytes = vec![193; 10 * 1024 * 1024];
    let large = Arc::new(large);
    queue.edit_record(
        Kind::Attachment,
        large.id.clone(),
        None,
        Some(Record::Attachment(large.clone())),
    );
    queue.failed_record(Kind::Attachment, "attachment", "aborted".into(), false);
    let text = queue.recovery_json();
    assert!(text.len() < 4096);
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    let attachment = json
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["kind"] == "attachment")
        .unwrap();
    assert!(attachment["unsaved"].get("bytes").is_none());
    assert_eq!(attachment["unsaved"]["byte_length"], 10 * 1024 * 1024);
    let files = queue.recovery_attachments();
    assert_eq!(files.len(), 1);
    assert_eq!(attachment["unsaved"]["bytes_file"], files[0].download_name);
    assert!(Arc::ptr_eq(&files[0].attachment, &large));
    assert_eq!(files[0].attachment.bytes[0], 193);
}

#[test]
fn failed_large_deletion_exposes_original_file_with_distinct_role() {
    let mut queue = Queue::default();
    let (_, original) = upload(&mut queue);
    let mut large = original.as_ref().clone();
    large.bytes = vec![17; 65537];
    let large = Arc::new(large);
    // Acknowledge the upload before requesting deletion, so recovery has a
    // durable original, not a still-uncommitted new-file tombstone.
    queue.edit_record(
        Kind::Attachment,
        large.id.clone(),
        None,
        Some(Record::Attachment(large.clone())),
    );
    for pending in queue.next_batch().unwrap() {
        assert!(queue.saved(pending));
    }
    queue.edit_record(
        Kind::Attachment,
        large.id.clone(),
        Some(Record::Attachment(large.clone())),
        None,
    );
    queue.failed_record(Kind::Attachment, &large.id, "aborted".into(), false);
    let files = queue.recovery_attachments();
    assert_eq!(files.len(), 1);
    assert!(files[0].download_name.starts_with("original-"));
    assert!(Arc::ptr_eq(&files[0].attachment, &large));
    let json: serde_json::Value = serde_json::from_str(&queue.recovery_json()).unwrap();
    assert_eq!(json[0]["original"]["bytes_file"], files[0].download_name);
    assert!(json[0]["unsaved"].is_null());
}
