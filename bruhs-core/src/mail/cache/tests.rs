use super::*;
use bruhs_models::InboxMessage;

pub(super) fn authorization(subject: &str, generation: &str) -> MailAuthorizationRef {
    let mailbox =
        MailboxId::from_broker_identity(ServiceProvider::Gmail, &"a".repeat(64), subject).unwrap();
    MailAuthorizationRef::from_broker_access(mailbox, generation).unwrap()
}

pub(super) fn cache() -> (tempfile::TempDir, MailCache) {
    let folder = tempfile::tempdir().unwrap();
    let cache = MailCache::open(
        &folder.path().join("mail.sqlite3"),
        &MailCacheHostId::from_host_record("host-a").unwrap(),
    )
    .unwrap();
    (folder, cache)
}

pub(super) fn thread(auth: &MailAuthorizationRef, external: &str, body: &str) -> ThreadWrite {
    let reference = auth.mailbox().thread(external).unwrap();
    let mut payload = InboxConversation::new(
        InboxSource::Email,
        "Synthetic mail",
        vec![],
        1_700_000_000_000,
    );
    payload.id = reference.canonical_id.clone();
    payload.external_id = Some(reference.external_id.clone());
    payload.account_label = "synthetic@example.test".into();
    payload.receive(InboxMessage::incoming("Sender", body, 1_700_000_001_000));
    ThreadWrite {
        reference,
        payload: serde_json::to_string(&payload).unwrap(),
    }
}

pub(super) fn batch(
    auth: &MailAuthorizationRef,
    external: &str,
    body: &str,
    cursor: &str,
) -> SyncBatch {
    SyncBatch {
        upserts: vec![thread(auth, external, body)],
        confirmed_deleted: vec![],
        cursor: Some((CursorScope::GmailHistory, cursor.into())),
    }
}

#[test]
fn thread_cursor_draft_and_host_identity_survive_restart() {
    let (folder, mut cache) = cache();
    let auth = authorization("account", "auth-a");
    assert_eq!(cache.bind(&auth).unwrap(), 0);
    let batch = batch(&auth, "t1", "Full Café 世界 body", "cursor-1");
    let expected = batch.upserts[0].payload.clone();
    let reference = batch.upserts[0].reference.clone();
    assert_eq!(cache.apply_sync(&auth, 0, &batch).unwrap(), 1);
    cache
        .save_draft(&reference, 0, r#"{"body":"unsent local draft"}"#)
        .unwrap();
    drop(cache);
    let cache = MailCache::open(
        &folder.path().join("mail.sqlite3"),
        &MailCacheHostId::from_host_record("new-proposed-id").unwrap(),
    )
    .unwrap();
    assert_eq!(cache.host().as_str(), "host-a");
    assert_eq!(cache.thread(&reference).unwrap(), Some(expected));
    assert_eq!(
        cache
            .cursor(auth.mailbox(), &CursorScope::GmailHistory)
            .unwrap()
            .as_deref(),
        Some("cursor-1")
    );
    assert_eq!(
        cache.draft(&reference).unwrap(),
        Some((1, r#"{"body":"unsent local draft"}"#.into()))
    );
}

#[test]
fn failure_after_payload_and_cursor_writes_rolls_back_the_whole_batch() {
    let (_folder, mut cache) = cache();
    let auth = authorization("account", "auth-a");
    cache.bind(&auth).unwrap();
    let initial = batch(&auth, "t1", "Original body", "cursor-old");
    cache.apply_sync(&auth, 0, &initial).unwrap();
    cache.connection.execute_batch("CREATE TRIGGER fail_revision BEFORE UPDATE OF revision ON mailboxes BEGIN SELECT RAISE(ABORT,'synthetic revision write failure'); END;").unwrap();
    let changed = batch(&auth, "t1", "Must not replace the old body", "cursor-new");
    assert!(matches!(
        cache.apply_sync(&auth, 1, &changed),
        Err(CacheError::Storage(_))
    ));
    assert_eq!(
        cache
            .thread(&initial.upserts[0].reference)
            .unwrap()
            .as_deref(),
        Some(initial.upserts[0].payload.as_str())
    );
    assert_eq!(
        cache
            .cursor(auth.mailbox(), &CursorScope::GmailHistory)
            .unwrap()
            .as_deref(),
        Some("cursor-old")
    );
    assert_eq!(cache.bind(&auth).unwrap(), 1);
}

#[test]
fn concurrent_connection_cannot_commit_a_stale_sync_snapshot() {
    let (folder, mut first) = cache();
    let auth = authorization("account", "auth-a");
    first.bind(&auth).unwrap();
    let mut second = MailCache::open(&folder.path().join("mail.sqlite3"), first.host()).unwrap();
    let newer = batch(&auth, "t1", "Newer body", "newer-cursor");
    first.apply_sync(&auth, 0, &newer).unwrap();
    assert!(matches!(
        second.apply_sync(&auth, 0, &batch(&auth, "t1", "Stale body", "stale-cursor")),
        Err(CacheError::RevisionConflict)
    ));
    assert_eq!(
        second
            .thread(&newer.upserts[0].reference)
            .unwrap()
            .as_deref(),
        Some(newer.upserts[0].payload.as_str())
    );
    assert_eq!(
        second
            .cursor(auth.mailbox(), &CursorScope::GmailHistory)
            .unwrap()
            .as_deref(),
        Some("newer-cursor")
    );
}

#[test]
fn reconnect_preserves_cache_and_draft_but_refuses_old_authorization() {
    let (_folder, mut cache) = cache();
    let previous = authorization("account", "auth-old");
    cache.bind(&previous).unwrap();
    let initial = batch(&previous, "t1", "Retained body", "history");
    cache.apply_sync(&previous, 0, &initial).unwrap();
    let reference = &initial.upserts[0].reference;
    cache
        .save_draft(reference, 0, r#"{"body":"keep this draft"}"#)
        .unwrap();
    let current = authorization("account", "auth-new");
    assert_eq!(cache.bind(&current).unwrap(), 2);
    assert!(matches!(
        cache.apply_sync(&previous, 2, &initial),
        Err(CacheError::AuthorizationChanged)
    ));
    assert!(cache.thread(reference).unwrap().is_some());
    assert_eq!(
        cache.draft(reference).unwrap().unwrap().1,
        r#"{"body":"keep this draft"}"#
    );
}

#[test]
fn account_switch_and_equal_provider_ids_remain_isolated() {
    let (_folder, mut cache) = cache();
    let first = authorization("account-a", "auth-a");
    let other = authorization("account-b", "auth-b");
    cache.bind(&first).unwrap();
    cache.bind(&other).unwrap();
    let a = batch(&first, "same-id", "A's body", "cursor-a");
    let b = batch(&other, "same-id", "B's body", "cursor-b");
    cache.apply_sync(&first, 0, &a).unwrap();
    cache.apply_sync(&other, 0, &b).unwrap();
    assert_ne!(
        cache.thread(&a.upserts[0].reference).unwrap(),
        cache.thread(&b.upserts[0].reference).unwrap()
    );
    assert!(matches!(
        cache.apply_sync(&other, 1, &a),
        Err(CacheError::InvalidInput)
    ));
}

#[test]
fn stale_draft_revision_cannot_overwrite_a_newer_edit() {
    let (_folder, mut cache) = cache();
    let auth = authorization("account", "auth-a");
    cache.bind(&auth).unwrap();
    let reference = auth.mailbox().thread("t1").unwrap();
    assert_eq!(
        cache
            .save_draft(&reference, 0, r#"{"body":"newer owner edit"}"#)
            .unwrap(),
        1
    );
    assert!(matches!(
        cache.save_draft(&reference, 0, r#"{"body":"stale client edit"}"#),
        Err(CacheError::RevisionConflict)
    ));
    assert_eq!(
        cache.draft(&reference).unwrap().unwrap().1,
        r#"{"body":"newer owner edit"}"#
    );
    assert_eq!(
        cache
            .save_draft(&reference, 1, r#"{"body":"reviewed next revision"}"#)
            .unwrap(),
        2
    );
}

#[test]
fn confirmed_remote_deletion_preserves_the_independent_local_draft() {
    let (_folder, mut cache) = cache();
    let auth = authorization("account", "auth-a");
    cache.bind(&auth).unwrap();
    let initial = batch(&auth, "t1", "Remote body", "history-1");
    cache.apply_sync(&auth, 0, &initial).unwrap();
    let reference = initial.upserts[0].reference.clone();
    cache
        .save_draft(&reference, 0, r#"{"body":"unsent draft"}"#)
        .unwrap();
    let removed = SyncBatch {
        upserts: vec![],
        confirmed_deleted: vec![reference.clone()],
        cursor: Some((CursorScope::GmailHistory, "history-2".into())),
    };
    cache.apply_sync(&auth, 1, &removed).unwrap();
    assert!(cache.thread(&reference).unwrap().is_none());
    assert!(cache.draft(&reference).unwrap().is_some());
}

#[test]
fn replay_upserts_do_not_duplicate_threads() {
    let (_folder, mut cache) = cache();
    let auth = authorization("account", "auth-a");
    cache.bind(&auth).unwrap();
    let initial = batch(&auth, "t1", "One body", "history");
    cache.apply_sync(&auth, 0, &initial).unwrap();
    assert!(matches!(
        cache.apply_sync(&auth, 0, &initial),
        Err(CacheError::RevisionConflict)
    ));
    cache.apply_sync(&auth, 1, &initial).unwrap();
    let count: i64 = cache
        .connection
        .query_row("SELECT count(*) FROM threads", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn outlook_folder_cursors_are_separate_and_case_sensitive() {
    let (_folder, mut cache) = cache();
    let mailbox =
        MailboxId::from_broker_identity(ServiceProvider::Outlook, &"a".repeat(64), "account")
            .unwrap();
    let auth = MailAuthorizationRef::from_broker_access(mailbox, "auth-outlook").unwrap();
    cache.bind(&auth).unwrap();
    for (revision, folder, value) in [(0, "FolderA", "token-a"), (1, "Foldera", "token-b")] {
        let batch = SyncBatch {
            upserts: vec![],
            confirmed_deleted: vec![],
            cursor: Some((CursorScope::OutlookFolder(folder.into()), value.into())),
        };
        cache.apply_sync(&auth, revision, &batch).unwrap();
    }
    assert_eq!(
        cache
            .cursor(
                auth.mailbox(),
                &CursorScope::OutlookFolder("FolderA".into())
            )
            .unwrap()
            .as_deref(),
        Some("token-a")
    );
    assert_eq!(
        cache
            .cursor(
                auth.mailbox(),
                &CursorScope::OutlookFolder("Foldera".into())
            )
            .unwrap()
            .as_deref(),
        Some("token-b")
    );
    assert!(matches!(
        cache.cursor(auth.mailbox(), &CursorScope::GmailHistory),
        Err(CacheError::InvalidInput)
    ));
}

#[test]
fn future_or_unrelated_databases_are_not_reset() {
    for version in [0, 99] {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("existing.sqlite3");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE sentinel(value TEXT); INSERT INTO sentinel VALUES('preserve');",
            )
            .unwrap();
        connection
            .pragma_update(None, "user_version", version)
            .unwrap();
        drop(connection);
        assert!(matches!(
            MailCache::open(&path, &MailCacheHostId::from_host_record("host").unwrap()),
            Err(CacheError::UnsupportedSchema)
        ));
        let connection = Connection::open(path).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT value FROM sentinel", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "preserve"
        );
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            version
        );
    }
}

#[test]
fn a_forged_canonical_key_cannot_replace_or_delete_another_thread() {
    let (_folder, mut cache) = cache();
    let auth = authorization("account", "auth-a");
    cache.bind(&auth).unwrap();
    let initial = batch(&auth, "real-thread", "Preserved body", "cursor-1");
    cache.apply_sync(&auth, 0, &initial).unwrap();
    let mut forged = auth.mailbox().thread("another-provider-id").unwrap();
    forged.canonical_id = initial.upserts[0].reference.canonical_id.clone();
    let removal = SyncBatch {
        upserts: vec![],
        confirmed_deleted: vec![forged.clone()],
        cursor: None,
    };
    assert!(matches!(
        cache.apply_sync(&auth, 1, &removal),
        Err(CacheError::InvalidInput)
    ));
    assert!(matches!(
        cache.save_draft(&forged, 0, r#"{"body":"wrong reference"}"#),
        Err(CacheError::InvalidInput)
    ));
    assert!(cache
        .thread(&initial.upserts[0].reference)
        .unwrap()
        .is_some());
    assert_eq!(cache.bind(&auth).unwrap(), 1);
}

#[test]
fn independent_new_email_drafts_survive_restart_without_remote_threads() {
    let (folder, mut cache) = cache();
    let auth = authorization("account", "auth-a");
    cache.bind(&auth).unwrap();
    let first = auth.mailbox().draft("new-draft-a", None).unwrap();
    let second = auth.mailbox().draft("new-draft-b", None).unwrap();
    cache
        .save_mail_draft(
            &first,
            0,
            r#"{"to":["one@example.test"],"subject":"First","body":"Unsent one"}"#,
        )
        .unwrap();
    cache
        .save_mail_draft(
            &second,
            0,
            r#"{"to":["two@example.test"],"subject":"Second","body":"Unsent two"}"#,
        )
        .unwrap();
    let remote_count: i64 = cache
        .connection
        .query_row("SELECT count(*) FROM threads", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        remote_count, 0,
        "compose drafts must not fabricate provider threads"
    );
    drop(cache);
    let cache = MailCache::open(
        &folder.path().join("mail.sqlite3"),
        &MailCacheHostId::from_host_record("unused-new-host").unwrap(),
    )
    .unwrap();
    let restored = cache.drafts(auth.mailbox()).unwrap();
    assert_eq!(restored.len(), 2);
    assert!(restored
        .iter()
        .all(|item| item.reference.reply_to.is_none() && item.revision == 1));
    assert!(cache
        .mail_draft(&first)
        .unwrap()
        .unwrap()
        .1
        .contains("Unsent one"));
    assert!(cache
        .mail_draft(&second)
        .unwrap()
        .unwrap()
        .1
        .contains("Unsent two"));
}

#[test]
fn a_stale_new_compose_edit_preserves_the_newer_draft() {
    let (_folder, mut cache) = cache();
    let auth = authorization("account", "auth-a");
    cache.bind(&auth).unwrap();
    let draft = auth.mailbox().draft("new-draft", None).unwrap();
    cache
        .save_mail_draft(&draft, 0, r#"{"subject":"Owner revision","body":"newer"}"#)
        .unwrap();
    assert!(matches!(
        cache.save_mail_draft(&draft, 0, r#"{"subject":"Stale revision","body":"old"}"#),
        Err(CacheError::RevisionConflict)
    ));
    assert_eq!(
        cache.mail_draft(&draft).unwrap().unwrap(),
        (1, r#"{"subject":"Owner revision","body":"newer"}"#.into())
    );
}

#[test]
fn an_existing_draft_cannot_be_silently_retargeted_as_a_reply() {
    let (_folder, mut cache) = cache();
    let auth = authorization("account", "auth-a");
    cache.bind(&auth).unwrap();
    let original = auth.mailbox().draft("stable-local-draft", None).unwrap();
    cache
        .save_mail_draft(&original, 0, r#"{"body":"Original new email"}"#)
        .unwrap();
    let thread = auth.mailbox().thread("provider-thread").unwrap();
    let changed = auth
        .mailbox()
        .draft("stable-local-draft", Some(&thread))
        .unwrap();
    assert_eq!(original.canonical_id, changed.canonical_id);
    assert!(matches!(
        cache.save_mail_draft(&changed, 1, r#"{"body":"Silently changed reply target"}"#),
        Err(CacheError::RevisionConflict)
    ));
    assert_eq!(cache.drafts(auth.mailbox()).unwrap().len(), 1);
    assert!(cache
        .mail_draft(&original)
        .unwrap()
        .unwrap()
        .1
        .contains("Original new email"));
}

#[test]
fn reply_discovery_retains_external_identity_after_remote_deletion() {
    let (_folder, mut cache) = cache();
    let auth = authorization("account", "auth-a");
    cache.bind(&auth).unwrap();
    let initial = batch(
        &auth,
        "retained-external-thread",
        "Remote message",
        "cursor-1",
    );
    cache.apply_sync(&auth, 0, &initial).unwrap();
    let reference = initial.upserts[0].reference.clone();
    let draft = auth
        .mailbox()
        .draft("local-reply", Some(&reference))
        .unwrap();
    cache
        .save_mail_draft(&draft, 0, r#"{"body":"Keep my unsent reply"}"#)
        .unwrap();
    cache
        .apply_sync(
            &auth,
            1,
            &SyncBatch {
                upserts: vec![],
                confirmed_deleted: vec![reference.clone()],
                cursor: None,
            },
        )
        .unwrap();
    let restored = cache.drafts(auth.mailbox()).unwrap();
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].reference.reply_to, Some(reference));
    assert_eq!(restored[0].payload, r#"{"body":"Keep my unsent reply"}"#);
}

#[test]
fn invalid_revision_values_are_rejected_even_for_identical_content() {
    let (_folder, mut cache) = cache();
    let auth = authorization("account", "auth-a");
    cache.bind(&auth).unwrap();
    let draft = auth.mailbox().draft("compose", None).unwrap();
    let body = r#"{"body":"Retain this draft"}"#;
    assert_eq!(cache.save_mail_draft(&draft, 0, body).unwrap(), 1);
    assert!(matches!(
        cache.save_mail_draft(&draft, -1, body),
        Err(CacheError::InvalidInput)
    ));
    assert!(matches!(
        cache.save_mail_draft(&draft, 2, body),
        Err(CacheError::RevisionConflict)
    ));
    assert_eq!(cache.save_mail_draft(&draft, 0, body).unwrap(), 1);
    assert_eq!(
        cache.mail_draft(&draft).unwrap(),
        Some((1, body.to_owned()))
    );
}

#[test]
fn full_scan_reconciles_missing_threads_only_after_the_final_page_and_retains_drafts() {
    let (_folder, mut cache) = cache();
    let auth = authorization("account", "auth-a");
    cache.bind(&auth).unwrap();
    let initial = SyncBatch {
        upserts: vec![
            thread(&auth, "stale", "Stale"),
            thread(&auth, "first", "First"),
        ],
        confirmed_deleted: vec![],
        cursor: Some((CursorScope::GmailHistory, "old-position".into())),
    };
    cache.apply_sync(&auth, 0, &initial).unwrap();
    let stale = auth.mailbox().thread("stale").unwrap();
    cache
        .save_draft(&stale, 0, r#"{"body":"Preserved reply"}"#)
        .unwrap();
    let page1 = batch(&auth, "first", "First updated", "page-2");
    cache
        .apply_full_sync_page(&auth, 1, &page1, "run-1", false)
        .unwrap();
    assert!(cache.thread(&stale).unwrap().is_some());
    let page2 = batch(&auth, "second", "Second", "history-baseline");
    cache
        .apply_full_sync_page(&auth, 2, &page2, "run-1", true)
        .unwrap();
    assert!(cache.thread(&stale).unwrap().is_none());
    assert!(cache
        .thread(&auth.mailbox().thread("first").unwrap())
        .unwrap()
        .is_some());
    assert!(cache
        .thread(&auth.mailbox().thread("second").unwrap())
        .unwrap()
        .is_some());
    assert_eq!(
        cache.draft(&stale).unwrap().unwrap().1,
        r#"{"body":"Preserved reply"}"#
    );
    assert_eq!(
        cache
            .cursor(auth.mailbox(), &CursorScope::GmailHistory)
            .unwrap()
            .as_deref(),
        Some("history-baseline")
    );
}

#[test]
fn failed_full_scan_completion_rolls_back_seen_rows_pruning_and_the_cursor() {
    let (_folder, mut cache) = cache();
    let auth = authorization("account", "auth-a");
    cache.bind(&auth).unwrap();
    cache
        .apply_sync(&auth, 0, &batch(&auth, "stale", "Stale", "old-position"))
        .unwrap();
    let page = batch(&auth, "first", "First", "final-position");
    cache.connection.execute_batch("CREATE TRIGGER reject_full_finish BEFORE UPDATE OF revision ON mailboxes BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(matches!(
        cache.apply_full_sync_page(&auth, 1, &page, "run-1", true),
        Err(CacheError::Storage(_))
    ));
    assert!(cache
        .thread(&auth.mailbox().thread("stale").unwrap())
        .unwrap()
        .is_some());
    assert!(cache
        .thread(&auth.mailbox().thread("first").unwrap())
        .unwrap()
        .is_none());
    assert_eq!(
        cache
            .cursor(auth.mailbox(), &CursorScope::GmailHistory)
            .unwrap()
            .as_deref(),
        Some("old-position")
    );
    let seen: i64 = cache
        .connection
        .query_row("SELECT count(*) FROM full_sync_seen", [], |row| row.get(0))
        .unwrap();
    assert_eq!(seen, 0);
}

#[test]
fn schema_one_migrates_without_losing_host_threads_drafts_or_cursors() {
    let (folder, mut cache) = cache();
    let auth = authorization("account", "auth-a");
    cache.bind(&auth).unwrap();
    let original = batch(&auth, "thread", "Preserved body", "preserved-cursor");
    cache.apply_sync(&auth, 0, &original).unwrap();
    cache
        .save_draft(
            &original.upserts[0].reference,
            0,
            r#"{"body":"Preserved draft"}"#,
        )
        .unwrap();
    cache
        .connection
        .execute_batch("DROP TABLE prepared_messages; DROP TRIGGER compose_projection_insert; DROP TRIGGER compose_projection_update; DROP INDEX compose_drafts_browse; ALTER TABLE drafts DROP COLUMN compose_subject; ALTER TABLE drafts DROP COLUMN compose_recipients; ALTER TABLE drafts DROP COLUMN compose_updated_at_ms; DROP TRIGGER compose_draft_insert; DROP TRIGGER compose_draft_update; DROP TRIGGER compose_draft_delete; ALTER TABLE mailboxes DROP COLUMN drafts_revision; DROP TABLE draft_attachments; DROP TABLE full_sync_seen; DROP INDEX mail_threads_browse; ALTER TABLE threads DROP COLUMN card; ALTER TABLE threads DROP COLUMN search_fields; ALTER TABLE threads DROP COLUMN message_at_ms; ALTER TABLE threads DROP COLUMN unread; ALTER TABLE threads DROP COLUMN archived; ALTER TABLE threads DROP COLUMN snoozed_until_ms; PRAGMA user_version=1;")
        .unwrap();
    drop(cache);
    let migrated = MailCache::open(
        &folder.path().join("mail.sqlite3"),
        &MailCacheHostId::from_host_record("ignored-new-host").unwrap(),
    )
    .unwrap();
    assert_eq!(migrated.host().as_str(), "host-a");
    assert!(migrated
        .thread(&original.upserts[0].reference)
        .unwrap()
        .unwrap()
        .contains("Preserved body"));
    assert!(migrated
        .draft(&original.upserts[0].reference)
        .unwrap()
        .unwrap()
        .1
        .contains("Preserved draft"));
    assert_eq!(
        migrated
            .cursor(auth.mailbox(), &CursorScope::GmailHistory)
            .unwrap()
            .as_deref(),
        Some("preserved-cursor")
    );
    assert_eq!(
        migrated
            .connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        5
    );
}
