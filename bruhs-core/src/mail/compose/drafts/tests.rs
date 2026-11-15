use super::*;
use crate::{connections::Connections, mail::CacheError};
use bruhs_models::connections::ServiceProvider;

struct Fixture {
    folder: tempfile::TempDir,
    runtime: MailRuntime,
    connections: Connections,
    session: MailSession,
    sender: VerifiedMailSender,
    mailbox: String,
    host: String,
}

struct OfflineFixture {
    folder: tempfile::TempDir,
    runtime: MailRuntime,
    connections: Connections,
    mailbox: String,
    host: String,
}

impl Fixture {
    fn release_lease(self) -> OfflineFixture {
        drop(self.sender);
        drop(self.session);
        OfflineFixture {
            folder: self.folder,
            runtime: self.runtime,
            connections: self.connections,
            mailbox: self.mailbox,
            host: self.host,
        }
    }
    async fn new() -> Self {
        let folder = tempfile::tempdir().unwrap();
        let runtime = MailRuntime::open(folder.path().join("mail.sqlite3"))
            .await
            .unwrap();
        let connections =
            Connections::test_connected(ServiceProvider::Gmail, "synthetic-token").await;
        let session = runtime
            .session(&connections, ServiceProvider::Gmail, "synthetic-connection")
            .await
            .unwrap();
        let sender = VerifiedMailSender::from_profile(
            &session,
            MailAddress {
                name: String::new(),
                address: "sender@example.test".into(),
            },
        )
        .unwrap();
        let mailbox = session.authorization().mailbox().as_str().to_owned();
        let host = runtime.host().as_str().to_owned();
        Self {
            folder,
            runtime,
            connections,
            session,
            sender,
            mailbox,
            host,
        }
    }

    async fn create(&self, id: &str) -> MailComposeDraft {
        self.runtime
            .compose_create(
                self.mailbox.clone(),
                MailDraftCreate {
                    host_id: self.host.clone(),
                    client_id: id.into(),
                    reply: None,
                },
            )
            .await
            .unwrap()
    }

    async fn save(
        &self,
        draft: &MailComposeDraft,
        content: MailComposeContent,
    ) -> MailComposeDraft {
        self.runtime
            .compose_save(
                self.mailbox.clone(),
                draft.client_id.clone(),
                MailDraftSave {
                    host_id: self.host.clone(),
                    expected_revision: draft.revision,
                    content,
                },
            )
            .await
            .unwrap()
    }

    async fn review(&self, id: &str, revision: i64) -> Result<MailSendReview, ComposeError> {
        self.session
            .review_compose(
                &self.sender,
                id.into(),
                MailReviewRequest {
                    host_id: self.host.clone(),
                    connection_id: self.session.authorization().connection_id().into(),
                    expected_revision: revision,
                },
            )
            .await
    }

    async fn frozen_bytes(&self, client: &str) -> Vec<u8> {
        let mailbox = self.mailbox.clone();
        let client = client.to_owned();
        self.runtime
            .compose_call(false, move |cache| {
                let snapshot = cache.compose_snapshot(&mailbox, &client)?;
                Ok(cache
                    .compose_prepared(
                        &snapshot.reference,
                        snapshot.payload.prepared.as_ref().unwrap(),
                    )?
                    .bytes()
                    .to_vec())
            })
            .await
            .unwrap()
    }
}

fn content() -> MailComposeContent {
    MailComposeContent {
        to: "recipient@example.test".into(),
        subject: "Private subject 東京".into(),
        body: "Private unsent body\nCafé".into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn typed_drafts_restore_after_disconnect_and_restart_without_provider_access() {
    let (draft, content, offline) = {
        let fixture = Fixture::new().await;
        let draft = fixture.create("persistent").await;
        let mut content = content();
        content.to = "Unfinished <recipient@".into();
        let draft = fixture.save(&draft, content.clone()).await;
        (draft, content, fixture.release_lease())
    };
    let OfflineFixture {
        folder,
        runtime,
        connections,
        mailbox,
        host,
    } = offline;
    connections
        .disconnect(ServiceProvider::Gmail)
        .await
        .unwrap();
    let restored = runtime
        .compose_draft(host.clone(), mailbox.clone(), "persistent".into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(restored.content, content);
    assert_eq!(restored.revision, draft.revision);
    drop(runtime);
    let reopened = MailRuntime::open(folder.path().join("mail.sqlite3"))
        .await
        .unwrap();
    let restored = reopened
        .compose_draft(host, mailbox, "persistent".into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(restored.content, content);
    assert_eq!(restored.revision, draft.revision);
}

#[tokio::test]
async fn lost_create_and_save_responses_do_not_reset_newer_content_or_clobber_conflicts() {
    let fixture = Fixture::new().await;
    let initial = fixture.create("lost-response").await;
    let saved = fixture.save(&initial, content()).await;
    assert_eq!(fixture.create("lost-response").await.content, saved.content);
    let request = MailDraftSave {
        host_id: fixture.host.clone(),
        expected_revision: initial.revision,
        content: saved.content.clone(),
    };
    let retried = fixture
        .runtime
        .compose_save(fixture.mailbox.clone(), initial.client_id.clone(), request)
        .await
        .unwrap();
    assert_eq!(retried.revision, saved.revision);
    let request = MailDraftSave {
        host_id: fixture.host.clone(),
        expected_revision: initial.revision,
        content: MailComposeContent {
            body: "stale overwrite".into(),
            ..content()
        },
    };
    assert!(matches!(
        fixture
            .runtime
            .compose_save(fixture.mailbox.clone(), initial.client_id, request)
            .await,
        Err(ComposeError::Mail(MailError::Cache(
            CacheError::RevisionConflict
        )))
    ));
    assert_eq!(
        fixture
            .runtime
            .compose_draft(
                fixture.host.clone(),
                fixture.mailbox.clone(),
                "lost-response".into()
            )
            .await
            .unwrap()
            .unwrap()
            .content,
        saved.content
    );
    drop(fixture);
}

#[tokio::test]
async fn review_retries_reuse_identical_durable_mime_then_any_edit_invalidates_it() {
    let fixture = Fixture::new().await;
    let draft = fixture.create("reviewed").await;
    let draft = fixture.save(&draft, content()).await;
    let review = fixture.review("reviewed", draft.revision).await.unwrap();
    assert_eq!(review.revision, draft.revision + 1);
    let bytes = fixture.frozen_bytes("reviewed").await;
    let retried = fixture.review("reviewed", draft.revision).await.unwrap();
    assert_eq!(retried.revision, review.revision);
    assert_eq!(retried.digest, review.digest);
    assert_eq!(retried.mime_sha256, review.mime_sha256);
    assert_eq!(fixture.frozen_bytes("reviewed").await, bytes);
    let restored = fixture
        .runtime
        .compose_draft(
            fixture.host.clone(),
            fixture.mailbox.clone(),
            "reviewed".into(),
        )
        .await
        .unwrap()
        .unwrap();
    let mut changed = restored.content.clone();
    changed.body.push_str("\nA later edit");
    let changed = fixture.save(&restored, changed).await;
    assert!(matches!(
        fixture.review("reviewed", review.revision).await,
        Err(ComposeError::StaleReview)
    ));
    let new_review = fixture.review("reviewed", changed.revision).await.unwrap();
    assert_ne!(new_review.digest, review.digest);
    assert_ne!(new_review.message_id, review.message_id);
    drop(fixture);
}

#[tokio::test]
async fn delayed_preparation_cannot_replace_a_newer_draft_edit() {
    let fixture = Fixture::new().await;
    let draft = fixture.create("delayed").await;
    let draft = fixture.save(&draft, content()).await;
    let mailbox = fixture.mailbox.clone();
    let source = fixture
        .runtime
        .compose_call(false, move |cache| {
            cache.compose_snapshot(&mailbox, "delayed")
        })
        .await
        .unwrap();
    let prepared = fixture
        .session
        .prepare_mime(
            &fixture.sender,
            MailMimeInput {
                reference: source.reference.clone(),
                source_revision: source.revision,
                content: source.payload.content.clone(),
                reply: None,
            },
        )
        .await
        .unwrap();
    let mut changed = draft.content.clone();
    changed.body = "newer body".into();
    let changed = fixture.save(&draft, changed).await;
    let authorization = fixture.session.authorization().clone();
    assert!(matches!(
        fixture
            .runtime
            .compose_call(true, move |cache| cache.compose_freeze(
                &authorization,
                source,
                prepared
            ))
            .await,
        Err(ComposeError::StaleReview)
    ));
    let restored = fixture
        .runtime
        .compose_draft(
            fixture.host.clone(),
            fixture.mailbox.clone(),
            "delayed".into(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(restored.revision, changed.revision);
    assert_eq!(restored.content.body, "newer body");
    let connection =
        rusqlite::Connection::open(fixture.folder.path().join("mail.sqlite3")).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM prepared_messages", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(fixture);
}

#[tokio::test]
async fn preparation_and_revision_commit_atomically_when_the_draft_write_fails() {
    let fixture = Fixture::new().await;
    let draft = fixture.create("rollback").await;
    let draft = fixture.save(&draft, content()).await;
    let connection =
        rusqlite::Connection::open(fixture.folder.path().join("mail.sqlite3")).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_preparation BEFORE UPDATE OF revision ON drafts BEGIN SELECT RAISE(ABORT,'synthetic write failure'); END;").unwrap();
    assert!(fixture.review("rollback", draft.revision).await.is_err());
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM prepared_messages", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        fixture
            .runtime
            .compose_draft(
                fixture.host.clone(),
                fixture.mailbox.clone(),
                "rollback".into()
            )
            .await
            .unwrap()
            .unwrap()
            .revision,
        draft.revision
    );
    connection
        .execute_batch("DROP TRIGGER reject_preparation")
        .unwrap();
    assert!(fixture.review("rollback", draft.revision).await.is_ok());
    drop(fixture);
}

#[tokio::test]
async fn frozen_bytes_and_metadata_are_both_bound_to_the_review_reference() {
    for metadata in [false, true] {
        let fixture = Fixture::new().await;
        let draft = fixture.create("corruption").await;
        let draft = fixture.save(&draft, content()).await;
        let review = fixture.review("corruption", draft.revision).await.unwrap();
        let connection =
            rusqlite::Connection::open(fixture.folder.path().join("mail.sqlite3")).unwrap();
        if metadata {
            connection.execute("UPDATE prepared_messages SET metadata=json_set(metadata,'$.from.address','another@example.test')",[]).unwrap();
        } else {
            let mut bytes: Vec<u8> = connection
                .query_row("SELECT data FROM prepared_messages", [], |row| row.get(0))
                .unwrap();
            bytes[0] ^= 1;
            connection
                .execute("UPDATE prepared_messages SET data=?1", [bytes])
                .unwrap();
        }
        assert!(matches!(
            fixture.review("corruption", review.revision).await,
            Err(ComposeError::PreparedCorrupt)
        ));
        drop(fixture);
    }
}

#[tokio::test]
async fn draft_pages_have_independent_revision_fences_and_bounded_metadata() {
    let fixture = Fixture::new().await;
    let sync_revision = fixture.session.revision().await.unwrap();
    for index in 0..53 {
        fixture.create(&format!("draft-{index:03}")).await;
    }
    let first = fixture
        .runtime
        .compose_drafts(fixture.host.clone(), fixture.mailbox.clone(), 0, None)
        .await
        .unwrap();
    assert_eq!(first.rows.len(), 50);
    assert_eq!(first.total_matches, 53);
    let last = fixture
        .runtime
        .compose_drafts(
            fixture.host.clone(),
            fixture.mailbox.clone(),
            u64::MAX,
            Some(first.revision),
        )
        .await
        .unwrap();
    assert_eq!(last.page, 1);
    assert_eq!(last.rows.len(), 3);
    fixture.create("newer").await;
    assert!(matches!(
        fixture
            .runtime
            .compose_drafts(
                fixture.host.clone(),
                fixture.mailbox.clone(),
                1,
                Some(first.revision)
            )
            .await,
        Err(ComposeError::Mail(MailError::Cache(
            CacheError::RevisionConflict
        )))
    ));
    assert_eq!(fixture.session.revision().await.unwrap(), sync_revision);
    drop(fixture);
}

#[tokio::test]
async fn cross_host_cross_mailbox_and_unowned_file_references_fail_without_changes() {
    let fixture = Fixture::new().await;
    let draft = fixture.create("owner").await;
    let mut request = MailDraftSave {
        host_id: "foreign-host".into(),
        expected_revision: draft.revision,
        content: content(),
    };
    assert!(fixture
        .runtime
        .compose_save(fixture.mailbox.clone(), "owner".into(), request.clone())
        .await
        .is_err());
    request.host_id = fixture.host.clone();
    request.content.attachments = vec!["mail-draft-attachment-v1-foreign".into()];
    assert!(matches!(
        fixture
            .runtime
            .compose_save(fixture.mailbox.clone(), "owner".into(), request)
            .await,
        Err(ComposeError::AttachmentChanged)
    ));
    assert!(fixture
        .runtime
        .compose_draft(
            fixture.host.clone(),
            "mailbox-v1-foreign".into(),
            "owner".into()
        )
        .await
        .is_err());
    assert_eq!(
        fixture
            .runtime
            .compose_draft(
                fixture.host.clone(),
                fixture.mailbox.clone(),
                "owner".into()
            )
            .await
            .unwrap()
            .unwrap()
            .revision,
        draft.revision
    );
    drop(fixture);
}

#[tokio::test]
async fn reply_creation_keeps_one_canonical_source_and_never_infers_received_bcc() {
    use crate::mail::{SyncBatch, ThreadWrite};
    use bruhs_models::{InboxConversation, InboxMessage, InboxSource};
    let fixture = Fixture::new().await;
    let remote = fixture
        .session
        .authorization()
        .mailbox()
        .thread("source-thread")
        .unwrap();
    let id = fixture
        .session
        .authorization()
        .mailbox()
        .message("source-message")
        .unwrap()
        .canonical_id;
    let mut message = InboxMessage::incoming("Source", "Source body", 1);
    message.id = id.clone();
    message.external_id = Some("source-message".into());
    let mut conversation =
        InboxConversation::new(InboxSource::Email, "Exact source subject", vec![], 1);
    conversation.id = remote.canonical_id.clone();
    conversation.external_id = Some(remote.external_id.clone());
    conversation.receive(message);
    let address = |value: &str| MailAddress {
        name: String::new(),
        address: value.into(),
    };
    let mut record = ConnectedMailThread {
        conversation,
        connected_mail: ConnectedMailMetadata {
            provider: ServiceProvider::Gmail,
            mailbox_id: fixture.mailbox.clone(),
            external_thread_id: remote.external_id.clone(),
            messages: vec![MailMessageDetail {
                id: id.clone(),
                external_id: "source-message".into(),
                label_ids: vec![],
                attachments: vec![],
                body_from_html: false,
                envelope: MailEnvelope {
                    from: vec![address("source@example.test")],
                    reply_to: vec![address("replies@example.test")],
                    to: vec![address("sender@example.test")],
                    cc: vec![address("copy@example.test")],
                    bcc: vec![address("hidden@example.test")],
                    subject: "Exact source subject".into(),
                    message_id: Some("source@example.test".into()),
                    ..Default::default()
                },
            }],
        },
    };
    fixture
        .session
        .apply_sync(
            fixture.session.revision().await.unwrap(),
            SyncBatch {
                upserts: vec![ThreadWrite {
                    reference: remote,
                    payload: serde_json::to_string(&record).unwrap(),
                }],
                confirmed_deleted: vec![],
                cursor: None,
            },
        )
        .await
        .unwrap();
    let request = MailDraftCreate {
        host_id: fixture.host.clone(),
        client_id: "reply-draft".into(),
        reply: Some(MailReplyTarget {
            external_thread_id: "source-thread".into(),
            message_id: id,
        }),
    };
    let draft = fixture
        .runtime
        .compose_create(fixture.mailbox.clone(), request.clone())
        .await
        .unwrap();
    assert_eq!(draft.content.subject, "Exact source subject");
    assert_eq!(draft.content.to, "replies@example.test");
    assert!(draft.content.cc.is_empty());
    assert!(draft.content.bcc.is_empty());
    assert!(draft.content.body.is_empty());
    assert_eq!(draft.reply, request.reply);
    let mut changed = request;
    changed.reply = None;
    assert!(matches!(
        fixture
            .runtime
            .compose_create(fixture.mailbox.clone(), changed)
            .await,
        Err(ComposeError::InvalidReply)
    ));
    let review = fixture.review("reply-draft", draft.revision).await.unwrap();
    assert_eq!(review.content.subject, "Exact source subject");
    let parsed_bytes = fixture.frozen_bytes("reply-draft").await;
    let parsed = mail_parser::MessageParser::default()
        .parse(&parsed_bytes)
        .unwrap();
    assert!(format!("{:?}", parsed.in_reply_to()).contains("source@example.test"));
    assert!(parsed.bcc().is_none());
    record.connected_mail.messages[0].envelope.message_id = Some("changed@example.test".into());
    fixture
        .session
        .apply_sync(
            fixture.session.revision().await.unwrap(),
            SyncBatch {
                upserts: vec![ThreadWrite {
                    reference: fixture
                        .session
                        .authorization()
                        .mailbox()
                        .thread("source-thread")
                        .unwrap(),
                    payload: serde_json::to_string(&record).unwrap(),
                }],
                confirmed_deleted: vec![],
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        fixture.review("reply-draft", review.revision).await,
        Err(ComposeError::InvalidReply)
    ));
    let unchanged = fixture
        .runtime
        .compose_draft(
            fixture.host.clone(),
            fixture.mailbox.clone(),
            "reply-draft".into(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.revision, review.revision);
    assert_eq!(unchanged.content.subject, "Exact source subject");
    drop(fixture);
}

#[tokio::test]
async fn uploaded_files_link_explicitly_and_survive_with_exact_frozen_mime_after_restart() {
    let (file, review, bytes, offline) = {
        let fixture = Fixture::new().await;
        let draft = fixture.create("with-files").await;
        let file = fixture
            .runtime
            .compose_upload(
                fixture.host.clone(),
                fixture.mailbox.clone(),
                "with-files".into(),
                crate::mail::DraftAttachmentUpload {
                    client_id: "file-a".into(),
                    file_name: "../résumé-東京.bin".into(),
                    bytes: b"binary\0private\xff".to_vec(),
                },
            )
            .await
            .unwrap();
        let restored = fixture
            .runtime
            .compose_draft(
                fixture.host.clone(),
                fixture.mailbox.clone(),
                "with-files".into(),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(restored.revision, draft.revision);
        assert!(restored.content.attachments.is_empty());
        assert_eq!(restored.uploads.len(), 1);
        assert_eq!(file.file_name, "résumé-東京.bin");
        let mut edited = content();
        edited.attachments = vec![file.id.clone()];
        let saved = fixture.save(&restored, edited).await;
        assert!(fixture
            .runtime
            .compose_remove_upload(
                fixture.host.clone(),
                fixture.mailbox.clone(),
                "with-files".into(),
                file.id.clone()
            )
            .await
            .is_err());
        let review = fixture.review("with-files", saved.revision).await.unwrap();
        let bytes = fixture.frozen_bytes("with-files").await;
        (file, review, bytes, fixture.release_lease())
    };
    let OfflineFixture {
        folder,
        runtime,
        connections,
        mailbox,
        host,
    } = offline;
    drop(runtime);
    let runtime = MailRuntime::open(folder.path().join("mail.sqlite3"))
        .await
        .unwrap();
    let session = runtime
        .session(&connections, ServiceProvider::Gmail, "synthetic-connection")
        .await
        .unwrap();
    let sender = VerifiedMailSender::from_profile(
        &session,
        MailAddress {
            name: String::new(),
            address: "sender@example.test".into(),
        },
    )
    .unwrap();
    let after = session
        .review_compose(
            &sender,
            "with-files".into(),
            MailReviewRequest {
                host_id: host.clone(),
                connection_id: "synthetic-connection".into(),
                expected_revision: review.revision,
            },
        )
        .await
        .unwrap();
    drop(session);
    assert_eq!(after.digest, review.digest);
    assert_eq!(after.mime_sha256, review.mime_sha256);
    let restored = runtime
        .compose_draft(host, mailbox.clone(), "with-files".into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(restored.uploads[0].sha256, file.sha256);
    let frozen = runtime
        .compose_call(false, move |cache| {
            let snapshot = cache.compose_snapshot(&mailbox, "with-files")?;
            Ok(cache
                .compose_prepared(
                    &snapshot.reference,
                    snapshot.payload.prepared.as_ref().unwrap(),
                )?
                .bytes()
                .to_vec())
        })
        .await
        .unwrap();
    assert_eq!(frozen, bytes);
    let parsed = mail_parser::MessageParser::default()
        .parse(&frozen)
        .unwrap();
    assert_eq!(
        parsed.attachments().next().unwrap().contents(),
        b"binary\0private\xff"
    );
}

#[tokio::test]
async fn schema_four_upgrade_preserves_host_draft_and_uploaded_file_bytes() {
    let (draft, file, offline) = {
        let fixture = Fixture::new().await;
        let draft = fixture.create("upgrade").await;
        let file = fixture
            .runtime
            .compose_upload(
                fixture.host.clone(),
                fixture.mailbox.clone(),
                "upgrade".into(),
                crate::mail::DraftAttachmentUpload {
                    client_id: "original".into(),
                    file_name: "original.bin".into(),
                    bytes: b"retained bytes".to_vec(),
                },
            )
            .await
            .unwrap();
        let mut content = content();
        content.attachments = vec![file.id.clone()];
        let draft = fixture.save(&draft, content).await;
        (draft, file, fixture.release_lease())
    };
    let OfflineFixture {
        folder,
        runtime,
        connections: _,
        mailbox,
        host,
    } = offline;
    drop(runtime);
    let connection = rusqlite::Connection::open(folder.path().join("mail.sqlite3")).unwrap();
    connection.execute_batch("DROP TABLE prepared_messages; DROP TRIGGER compose_projection_insert; DROP TRIGGER compose_projection_update; DROP INDEX compose_drafts_browse; ALTER TABLE drafts DROP COLUMN compose_subject; ALTER TABLE drafts DROP COLUMN compose_recipients; ALTER TABLE drafts DROP COLUMN compose_updated_at_ms; DROP TRIGGER compose_draft_insert; DROP TRIGGER compose_draft_update; DROP TRIGGER compose_draft_delete; ALTER TABLE mailboxes DROP COLUMN drafts_revision; PRAGMA user_version=4;").unwrap();
    drop(connection);
    let runtime = MailRuntime::open(folder.path().join("mail.sqlite3"))
        .await
        .unwrap();
    assert_eq!(runtime.host().as_str(), host);
    let restored = runtime
        .compose_draft(host, mailbox.clone(), "upgrade".into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(restored.content, draft.content);
    assert_eq!(restored.revision, draft.revision);
    assert_eq!(restored.uploads[0].sha256, file.sha256);
    let bytes = runtime
        .compose_call(false, move |cache| {
            let snapshot = cache.compose_snapshot(&mailbox, "upgrade")?;
            Ok(cache
                .draft_attachment(&snapshot.reference, &file.id)?
                .unwrap()
                .bytes)
        })
        .await
        .unwrap();
    assert_eq!(bytes, b"retained bytes");
}
