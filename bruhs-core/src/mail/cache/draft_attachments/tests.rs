use super::*;
use crate::mail::cache::tests::{authorization, batch, cache};
use crate::mail::{DispatchClaim, MailCacheHostId, ProviderAcceptance};
use bruhs_models::EpochMs;

fn draft(cache: &mut MailCache, client: &str) -> MailDraftRef {
    let auth = authorization("account", "generation");
    cache.bind(&auth).unwrap();
    let reference = auth.mailbox().draft(client, None).unwrap();
    cache
        .save_mail_draft(&reference, 0, r#"{"body":"private unsent draft"}"#)
        .unwrap();
    reference
}

fn upload(client: &str, bytes: &[u8]) -> DraftAttachmentUpload {
    DraftAttachmentUpload {
        client_id: client.into(),
        file_name: "../folder/résumé-東京.bin".into(),
        bytes: bytes.into(),
    }
}

#[test]
fn immutable_uploads_and_empty_files_survive_restart_without_advancing_revisions() {
    let (folder, mut cache) = cache();
    let reference = draft(&mut cache, "draft-a");
    let original = cache.mail_draft(&reference).unwrap();
    let data = b"private\0binary\xff";
    let metadata = cache
        .store_draft_attachment(&reference, upload("upload-a", data))
        .unwrap();
    assert_eq!(metadata.file_name, "résumé-東京.bin");
    assert_eq!(
        cache
            .store_draft_attachment(&reference, upload("upload-a", data))
            .unwrap(),
        metadata
    );
    let empty = cache
        .store_draft_attachment(&reference, upload("upload-empty", b""))
        .unwrap();
    assert_eq!(empty.size_bytes, 0);
    assert_eq!(cache.mail_draft(&reference).unwrap(), original);
    assert_eq!(
        cache
            .connection
            .query_row(
                "SELECT revision FROM mailboxes WHERE id=?1",
                [reference.mailbox.as_str()],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    drop(cache);
    let cache = MailCache::open(
        &folder.path().join("mail.sqlite3"),
        &MailCacheHostId::from_host_record("new-proposal").unwrap(),
    )
    .unwrap();
    assert_eq!(cache.host().as_str(), "host-a");
    assert_eq!(
        cache
            .draft_attachment(&reference, &metadata.id)
            .unwrap()
            .unwrap()
            .bytes,
        data
    );
    assert_eq!(
        cache
            .draft_attachment(&reference, &empty.id)
            .unwrap()
            .unwrap()
            .bytes,
        b""
    );
    assert_eq!(cache.draft_attachments(&reference).unwrap().len(), 2);
}

#[test]
fn same_upload_id_cannot_change_bytes_or_name_and_foreign_drafts_cannot_read_it() {
    let (_folder, mut cache) = cache();
    let reference = draft(&mut cache, "draft-a");
    let metadata = cache
        .store_draft_attachment(&reference, upload("upload-a", b"original"))
        .unwrap();
    assert!(matches!(
        cache.store_draft_attachment(&reference, upload("upload-a", b"changed")),
        Err(DraftAttachmentError::UploadConflict)
    ));
    let mut renamed = upload("upload-a", b"original");
    renamed.file_name = "another.bin".into();
    assert!(matches!(
        cache.store_draft_attachment(&reference, renamed),
        Err(DraftAttachmentError::UploadConflict)
    ));
    let other = draft(&mut cache, "draft-b");
    assert!(cache
        .draft_attachment(&other, &metadata.id)
        .unwrap()
        .is_none());
    let auth = authorization("another-account", "generation");
    cache.bind(&auth).unwrap();
    let another = auth.mailbox().draft("draft-a", None).unwrap();
    cache
        .save_mail_draft(&another, 0, r#"{"body":"another draft"}"#)
        .unwrap();
    assert!(cache
        .draft_attachment(&another, &metadata.id)
        .unwrap()
        .is_none());
    assert_eq!(
        cache
            .draft_attachment(&reference, &metadata.id)
            .unwrap()
            .unwrap()
            .bytes,
        b"original"
    );
}

#[test]
fn aggregate_size_and_file_count_are_bounded_and_duplicate_replays_do_not_consume_quota() {
    let (_folder, mut cache) = cache();
    let reference = draft(&mut cache, "large-draft");
    let full = vec![7; MAX_DRAFT_ATTACHMENT_BYTES];
    let metadata = cache
        .store_draft_attachment(&reference, upload("full", &full))
        .unwrap();
    assert!(matches!(
        cache.store_draft_attachment(&reference, upload("extra", b"x")),
        Err(DraftAttachmentError::TooLarge)
    ));
    assert_eq!(
        cache
            .store_draft_attachment(&reference, upload("full", &full))
            .unwrap(),
        metadata
    );
    assert!(cache
        .remove_draft_attachment(&reference, &metadata.id)
        .unwrap());
    for index in 0..MAX_DRAFT_ATTACHMENTS {
        cache
            .store_draft_attachment(&reference, upload(&format!("file-{index}"), b""))
            .unwrap();
    }
    assert!(matches!(
        cache.store_draft_attachment(&reference, upload("too-many", b"")),
        Err(DraftAttachmentError::TooMany)
    ));
    assert!(matches!(
        cache.store_draft_attachment(
            &reference,
            upload("oversize", &vec![0; MAX_DRAFT_ATTACHMENT_BYTES + 1])
        ),
        Err(DraftAttachmentError::TooLarge)
    ));
    assert_eq!(cache.draft_attachments(&reference).unwrap().len(), 20);
}

#[test]
fn removing_stored_bytes_requires_unlinking_the_file_in_a_new_draft_revision() {
    let (_folder, mut cache) = cache();
    let reference = draft(&mut cache, "draft-a");
    let metadata = cache
        .store_draft_attachment(&reference, upload("file-a", b"body"))
        .unwrap();
    let linked =
        serde_json::json!({"body":"Saved draft","attachments":[{"id":metadata.id}]}).to_string();
    assert_eq!(cache.save_mail_draft(&reference, 1, &linked).unwrap(), 2);
    assert!(matches!(
        cache.remove_draft_attachment(&reference, &metadata.id),
        Err(DraftAttachmentError::StillAttached)
    ));
    assert_eq!(
        cache
            .save_mail_draft(&reference, 2, r#"{"body":"Saved draft","attachments":[]}"#)
            .unwrap(),
        3
    );
    assert!(cache
        .remove_draft_attachment(&reference, &metadata.id)
        .unwrap());
    assert_eq!(cache.mail_draft(&reference).unwrap().unwrap().0, 3);
}

#[test]
fn failed_insert_rolls_back_and_a_retry_keeps_the_same_immutable_identity() {
    let (_folder, mut cache) = cache();
    let reference = draft(&mut cache, "draft-a");
    cache.connection.execute_batch("CREATE TRIGGER qa_fail_upload BEFORE INSERT ON draft_attachments BEGIN SELECT RAISE(ABORT,'private fixture'); END;").unwrap();
    assert!(matches!(
        cache.store_draft_attachment(&reference, upload("file-a", b"body")),
        Err(DraftAttachmentError::Cache(CacheError::Storage(_)))
    ));
    assert!(cache.draft_attachments(&reference).unwrap().is_empty());
    assert_eq!(cache.mail_draft(&reference).unwrap().unwrap().0, 1);
    cache
        .connection
        .execute_batch("DROP TRIGGER qa_fail_upload;")
        .unwrap();
    let metadata = cache
        .store_draft_attachment(&reference, upload("file-a", b"body"))
        .unwrap();
    assert_eq!(metadata.id, reference.attachment("file-a").unwrap());
}

#[test]
fn queued_uncertain_and_consumed_sends_retain_their_attachment_inputs() {
    let (_folder, mut cache) = cache();
    let reference = draft(&mut cache, "draft-a");
    let metadata = cache
        .store_draft_attachment(&reference, upload("file-a", b"body"))
        .unwrap();
    let auth = authorization("account", "generation");
    let review = cache.review_send(&auth, &reference, 1).unwrap();
    cache
        .queue_send("operation-a", &review, &auth, EpochMs::now())
        .unwrap();
    assert!(matches!(
        cache.remove_draft_attachment(&reference, &metadata.id),
        Err(DraftAttachmentError::SendPending)
    ));
    assert!(matches!(
        cache
            .claim_send("operation-a", &auth, EpochMs::now())
            .unwrap(),
        DispatchClaim::Start(_)
    ));
    cache
        .recover_interrupted_dispatches(EpochMs::now())
        .unwrap();
    assert!(matches!(
        cache.remove_draft_attachment(&reference, &metadata.id),
        Err(DraftAttachmentError::SendPending)
    ));
    cache
        .record_send_acceptance(
            "operation-a",
            &auth,
            &ProviderAcceptance {
                message_id: Some("message-a".into()),
                thread_id: Some("thread-a".into()),
                request_id: None,
            },
            EpochMs::now(),
        )
        .unwrap();
    assert!(matches!(
        cache.store_draft_attachment(&reference, upload("new-file", b"new")),
        Err(DraftAttachmentError::Cache(CacheError::DraftConsumed))
    ));
    assert!(matches!(
        cache.remove_draft_attachment(&reference, &metadata.id),
        Err(DraftAttachmentError::Cache(CacheError::DraftConsumed))
    ));
    assert_eq!(
        cache
            .draft_attachment(&reference, &metadata.id)
            .unwrap()
            .unwrap()
            .bytes,
        b"body"
    );
}

#[test]
fn changed_reply_scope_and_corrupt_saved_bytes_are_rejected_without_content_in_diagnostics() {
    let (_folder, mut cache) = cache();
    let reference = draft(&mut cache, "draft-a");
    let metadata = cache
        .store_draft_attachment(&reference, upload("file-a", b"private-body"))
        .unwrap();
    let mut reply = reference.clone();
    reply.reply_to = Some(reply.mailbox.thread("thread-a").unwrap());
    assert!(matches!(
        cache.draft_attachment(&reply, &metadata.id),
        Err(DraftAttachmentError::Cache(CacheError::RevisionConflict))
    ));
    cache
        .connection
        .execute(
            "UPDATE draft_attachments SET data=?1 WHERE id=?2",
            params![b"private-fail".as_slice(), metadata.id],
        )
        .unwrap();
    let error = cache
        .draft_attachment(&reference, &metadata.id)
        .unwrap_err();
    assert!(matches!(error, DraftAttachmentError::CorruptRecord));
    assert!(!format!("{error:?}").contains("private-fail"));
    assert!(!format!("{:?}", upload("file-a", b"private-body")).contains("private-body"));
    assert!(!format!("{metadata:?}").contains("résumé"));
}

#[test]
fn migration_from_schema_three_retains_threads_drafts_and_host() {
    let (folder, mut cache) = cache();
    let reference = draft(&mut cache, "draft-a");
    let original = cache.mail_draft(&reference).unwrap();
    let auth = authorization("account", "generation");
    let saved = batch(
        &auth,
        "preserved-thread",
        "Preserved full body",
        "preserved-cursor",
    );
    cache.apply_sync(&auth, 0, &saved).unwrap();
    cache
        .connection
        .execute_batch("DROP TABLE prepared_messages; DROP TRIGGER compose_projection_insert; DROP TRIGGER compose_projection_update; DROP INDEX compose_drafts_browse; ALTER TABLE drafts DROP COLUMN compose_subject; ALTER TABLE drafts DROP COLUMN compose_recipients; ALTER TABLE drafts DROP COLUMN compose_updated_at_ms; DROP TRIGGER compose_draft_insert; DROP TRIGGER compose_draft_update; DROP TRIGGER compose_draft_delete; ALTER TABLE mailboxes DROP COLUMN drafts_revision; DROP TABLE draft_attachments; PRAGMA user_version=3;")
        .unwrap();
    drop(cache);
    let mut cache = MailCache::open(
        &folder.path().join("mail.sqlite3"),
        &MailCacheHostId::from_host_record("other").unwrap(),
    )
    .unwrap();
    assert_eq!(cache.host().as_str(), "host-a");
    assert_eq!(cache.mail_draft(&reference).unwrap(), original);
    assert_eq!(
        cache
            .thread(&saved.upserts[0].reference)
            .unwrap()
            .as_deref(),
        Some(saved.upserts[0].payload.as_str())
    );
    assert_eq!(
        cache
            .cursor(&reference.mailbox, &super::super::CursorScope::GmailHistory)
            .unwrap()
            .as_deref(),
        Some("preserved-cursor")
    );
    cache
        .store_draft_attachment(&reference, upload("file-a", b"new"))
        .unwrap();
    assert_eq!(
        cache
            .connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        5
    );
}
