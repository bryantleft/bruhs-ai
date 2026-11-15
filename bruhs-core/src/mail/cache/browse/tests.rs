use super::*;
use crate::mail::cache::{
    tests::{authorization, batch, cache, thread},
    MailCache, SyncBatch, ThreadWrite,
};
use bruhs_models::{mail::*, InboxConversation};
use std::collections::HashSet;

fn query(cache: &MailCache) -> MailBrowseQuery {
    MailBrowseQuery {
        host_id: cache.host().as_str().into(),
        query: String::new(),
        filter: InboxFilter::All,
        page: 0,
        expected_revision: None,
    }
}

#[test]
fn pages_are_bounded_and_search_reaches_complete_older_message_bodies() {
    let (_directory, mut cache) = cache();
    let auth = authorization("account", "auth-a");
    cache.bind(&auth).unwrap();
    let mut upserts = Vec::new();
    for index in 0..105 {
        let mut write = thread(&auth, &format!("thread-{index}"), &format!("Body {index}"));
        let mut record: InboxConversation = serde_json::from_str(&write.payload).unwrap();
        record.messages[0].sent_at_ms += i64::from(index) * 1000;
        if index == 0 {
            record.messages[0].body = format!("{} full-history-needle", "long body ".repeat(1000));
        }
        write.payload = serde_json::to_string(&record).unwrap();
        upserts.push(write);
    }
    cache
        .apply_sync(
            &auth,
            0,
            &SyncBatch {
                upserts,
                confirmed_deleted: vec![],
                cursor: None,
            },
        )
        .unwrap();
    let mut request = query(&cache);
    let first = cache.browse(auth.mailbox().as_str(), &request).unwrap();
    assert_eq!(first.rows.len(), 100);
    assert_eq!(first.total_matches, 105);
    request.page = 1;
    request.expected_revision = Some(first.revision);
    let second = cache.browse(auth.mailbox().as_str(), &request).unwrap();
    assert_eq!(second.rows.len(), 5);
    let ids = first
        .rows
        .iter()
        .chain(&second.rows)
        .map(|row| row.card.id.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(ids.len(), 105);
    request.page = 50;
    let clamped = cache.browse(auth.mailbox().as_str(), &request).unwrap();
    assert_eq!(
        clamped.page, 1,
        "A stale page remains reachable after sync removes threads"
    );
    assert_eq!(clamped.rows.len(), 5);
    request.page = 0;
    request.query = "full-history-needle".into();
    let found = cache.browse(auth.mailbox().as_str(), &request).unwrap();
    assert_eq!(found.total_matches, 1);
    assert_eq!(found.rows[0].external_id, "thread-0");
    assert!(
        serde_json::to_string(&found).unwrap().len() < 2000,
        "Read page must not include full message bodies"
    );
}

#[test]
fn search_matches_unicode_attachment_names_and_cards_use_the_real_sender() {
    let (_directory, mut cache) = cache();
    let auth = authorization("account", "auth-a");
    cache.bind(&auth).unwrap();
    let write = thread(&auth, "thread-a", "Body");
    let mut conversation: InboxConversation = serde_json::from_str(&write.payload).unwrap();
    let message = auth.mailbox().message("message-a").unwrap();
    conversation.messages[0].id = message.canonical_id.clone();
    conversation.messages[0].external_id = Some("message-a".into());
    let detail = MailMessageDetail {
        id: message.canonical_id,
        external_id: "message-a".into(),
        label_ids: vec![],
        envelope: MailEnvelope {
            from: vec![MailAddress {
                name: "Zoë".into(),
                address: "zoe@example.test".into(),
            }],
            ..Default::default()
        },
        attachments: vec![MailAttachment {
            id: auth.mailbox().attachment("message-a", "1").unwrap(),
            part_id: "1".into(),
            external_id: Some("provider-part".into()),
            file_name: "設計-Café.pdf".into(),
            content_type: "application/pdf".into(),
            size_bytes: 100,
            content_id: None,
            inline: false,
        }],
        body_from_html: false,
    };
    let record = ConnectedMailThread {
        conversation,
        connected_mail: ConnectedMailMetadata {
            provider: ServiceProvider::Gmail,
            mailbox_id: auth.mailbox().as_str().into(),
            external_thread_id: "thread-a".into(),
            messages: vec![detail],
        },
    };
    cache
        .apply_sync(
            &auth,
            0,
            &SyncBatch {
                upserts: vec![ThreadWrite {
                    reference: write.reference,
                    payload: serde_json::to_string(&record).unwrap(),
                }],
                confirmed_deleted: vec![],
                cursor: None,
            },
        )
        .unwrap();
    let mut request = query(&cache);
    request.query = "CAFÉ.PDF".into();
    let found = cache.browse(auth.mailbox().as_str(), &request).unwrap();
    assert_eq!(found.total_matches, 1);
    assert_eq!(found.rows[0].card.sender, "Zoë");
    request.query = "%' OR 1=1 --".into();
    assert_eq!(
        cache
            .browse(auth.mailbox().as_str(), &request)
            .unwrap()
            .total_matches,
        0
    );
}

#[test]
fn browse_filters_match_the_shared_inbox_membership_rules() {
    let (_directory, mut cache) = cache();
    let auth = authorization("account", "auth-a");
    cache.bind(&auth).unwrap();
    let mut upserts = Vec::new();
    for index in 0..4 {
        let mut write = thread(&auth, &format!("thread-{index}"), "Body");
        let mut record: InboxConversation = serde_json::from_str(&write.payload).unwrap();
        record.unread = index < 2;
        record.archived = index == 2;
        if index == 3 {
            record.snoozed_until_ms = Some(EpochMs::now().as_millis() + 86_400_000);
        }
        write.payload = serde_json::to_string(&record).unwrap();
        upserts.push(write);
    }
    cache
        .apply_sync(
            &auth,
            0,
            &SyncBatch {
                upserts,
                confirmed_deleted: vec![],
                cursor: None,
            },
        )
        .unwrap();
    for (filter, total) in [
        (InboxFilter::Inbox, 2),
        (InboxFilter::Unread, 2),
        (InboxFilter::Archived, 1),
        (InboxFilter::Snoozed, 1),
        (InboxFilter::All, 4),
    ] {
        let mut request = query(&cache);
        request.filter = filter;
        assert_eq!(
            cache
                .browse(auth.mailbox().as_str(), &request)
                .unwrap()
                .total_matches,
            total
        );
    }
}

#[test]
fn another_host_stale_revision_or_overflowing_page_cannot_select_a_different_cache_snapshot() {
    let (_directory, mut cache) = cache();
    let auth = authorization("account", "auth-a");
    cache.bind(&auth).unwrap();
    cache
        .apply_sync(&auth, 0, &batch(&auth, "thread", "Body", "cursor"))
        .unwrap();
    let mut request = query(&cache);
    request.host_id = "other-host".into();
    assert!(matches!(
        cache.browse(auth.mailbox().as_str(), &request),
        Err(CacheError::HostChanged)
    ));
    request = query(&cache);
    request.expected_revision = Some(0);
    assert!(matches!(
        cache.browse(auth.mailbox().as_str(), &request),
        Err(CacheError::RevisionConflict)
    ));
    request = query(&cache);
    request.page = u64::MAX;
    assert!(matches!(
        cache.browse(auth.mailbox().as_str(), &request),
        Err(CacheError::InvalidInput)
    ));
    assert!(matches!(
        cache.connected_thread("other-host", auth.mailbox().as_str(), "thread"),
        Err(CacheError::HostChanged)
    ));
    let summaries = cache.mailboxes().unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(
        summaries[0].threads, 1,
        "Opaque non-JSON provider cursors must remain readable"
    );
}

#[test]
fn a_failed_upsert_cannot_publish_a_new_search_or_card_projection() {
    let (_directory, mut cache) = cache();
    let auth = authorization("account", "auth-a");
    cache.bind(&auth).unwrap();
    cache
        .apply_sync(
            &auth,
            0,
            &batch(&auth, "thread", "Preserved body", "cursor-1"),
        )
        .unwrap();
    cache.connection.execute_batch("CREATE TRIGGER fail_revision BEFORE UPDATE OF revision ON mailboxes BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(matches!(
        cache.apply_sync(
            &auth,
            1,
            &batch(&auth, "thread", "New searchable body", "cursor-2")
        ),
        Err(CacheError::Storage(_))
    ));
    let mut request = query(&cache);
    request.query = "New searchable".into();
    assert_eq!(
        cache
            .browse(auth.mailbox().as_str(), &request)
            .unwrap()
            .total_matches,
        0
    );
    request.query = "Preserved body".into();
    assert_eq!(
        cache
            .browse(auth.mailbox().as_str(), &request)
            .unwrap()
            .total_matches,
        1
    );
}

#[test]
fn schema_two_migration_builds_bounded_cards_and_full_search_without_losing_payloads() {
    let (directory, mut cache) = cache();
    let auth = authorization("account", "auth-a");
    cache.bind(&auth).unwrap();
    let original = batch(&auth, "thread", "Complete preserved body", "opaque-cursor");
    cache.apply_sync(&auth, 0, &original).unwrap();
    cache.connection.execute_batch("DROP TABLE prepared_messages; DROP TRIGGER compose_projection_insert; DROP TRIGGER compose_projection_update; DROP INDEX compose_drafts_browse; ALTER TABLE drafts DROP COLUMN compose_subject; ALTER TABLE drafts DROP COLUMN compose_recipients; ALTER TABLE drafts DROP COLUMN compose_updated_at_ms; DROP TRIGGER compose_draft_insert; DROP TRIGGER compose_draft_update; DROP TRIGGER compose_draft_delete; ALTER TABLE mailboxes DROP COLUMN drafts_revision; DROP TABLE draft_attachments; DROP INDEX mail_threads_browse; ALTER TABLE threads DROP COLUMN card; ALTER TABLE threads DROP COLUMN search_fields; ALTER TABLE threads DROP COLUMN message_at_ms; ALTER TABLE threads DROP COLUMN unread; ALTER TABLE threads DROP COLUMN archived; ALTER TABLE threads DROP COLUMN snoozed_until_ms; PRAGMA user_version=2;").unwrap();
    drop(cache);
    let mut reopened = MailCache::open(
        &directory.path().join("mail.sqlite3"),
        &crate::mail::MailCacheHostId::from_host_record("ignored-host").unwrap(),
    )
    .unwrap();
    let mut request = query(&reopened);
    request.query = "preserved body".into();
    assert_eq!(
        reopened
            .browse(auth.mailbox().as_str(), &request)
            .unwrap()
            .total_matches,
        1
    );
    assert_eq!(
        reopened
            .thread(&original.upserts[0].reference)
            .unwrap()
            .as_deref(),
        Some(original.upserts[0].payload.as_str())
    );
}
