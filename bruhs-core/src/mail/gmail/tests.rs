use super::*;
mod attachments;
mod compose_profile;
mod send;
mod sent;
use crate::{
    connections::Connections,
    mail::{CacheError, MailRuntime},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde_json::{json, Value};
use wiremock::{
    matchers::{header, method, path, query_param},
    Mock, MockServer, ResponseTemplate,
};

const API: &str = "/gmail/v1/users/me";
const TOKEN: &str = "synthetic-gmail-token";

struct Fixture {
    directory: tempfile::TempDir,
    runtime: MailRuntime,
    connections: Connections,
    client: GmailClient,
    server: MockServer,
}

impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let runtime = MailRuntime::open(directory.path().join("mail.sqlite3"))
            .await
            .unwrap();
        let connections = Connections::test_connected(ServiceProvider::Gmail, TOKEN).await;
        let server = MockServer::builder().start().await;
        let client =
            GmailClient::at(Url::parse(&format!("{}{API}/", server.uri())).unwrap()).unwrap();
        Self {
            directory,
            runtime,
            connections,
            client,
            server,
        }
    }

    async fn session(&self) -> MailSession {
        self.runtime
            .session(
                &self.connections,
                ServiceProvider::Gmail,
                "synthetic-connection",
            )
            .await
            .unwrap()
    }
}

async fn reply(server: &MockServer, route: &str, value: Value) {
    Mock::given(method("GET"))
        .and(path(format!("{API}/{route}")))
        .and(header("authorization", format!("Bearer {TOKEN}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(value))
        .mount(server)
        .await;
}

fn plain(part_id: &str, text: &str) -> Value {
    json!({"partId":part_id,"mimeType":"text/plain","headers":[{"name":"Content-Type","value":"text/plain; charset=utf-8"}],"body":{"size":text.len(),"data":URL_SAFE_NO_PAD.encode(text)}})
}

fn message(id: &str, thread_id: &str, text: &str) -> Value {
    let mut payload = plain("", text);
    payload["headers"] = json!([
        {"name":"From","value":"=?UTF-8?Q?Zo=C3=AB?= <zoe@example.test>"},
        {"name":"To","value":"\"Ops, Team\" <ops@example.test>, other@example.test"},
        {"name":"Cc","value":"copy@example.test"},
        {"name":"Reply-To","value":"replies@example.test"},
        {"name":"Subject","value":"=?UTF-8?Q?Caf=C3=A9_planning?="},
        {"name":"Message-ID","value":format!("<{id}@example.test>")},
        {"name":"References","value":"<original@example.test> <second@example.test>"},
        {"name":"Content-Type","value":"text/plain; charset=utf-8"}
    ]);
    json!({"id":id,"threadId":thread_id,"internalDate":"1794652800000","labelIds":["INBOX","UNREAD"],"payload":payload})
}

fn thread(id: &str, text: &str) -> Value {
    json!({"id":id,"messages":[message(&format!("{id}-message"),id,text)]})
}

async fn seed(
    active: &MailSession,
    id: &str,
    text: &str,
    checkpoint: Option<Checkpoint>,
) -> String {
    let record = projection::thread(
        active.authorization().mailbox(),
        "you@example.test",
        serde_json::from_value(thread(id, text)).unwrap(),
    )
    .unwrap();
    let payload = serde_json::to_string(&record).unwrap();
    active
        .apply_sync(
            active.revision().await.unwrap(),
            SyncBatch {
                upserts: vec![ThreadWrite {
                    reference: active.authorization().mailbox().thread(id).unwrap(),
                    payload: payload.clone(),
                }],
                confirmed_deleted: vec![],
                cursor: checkpoint.map(|checkpoint| {
                    (
                        CursorScope::GmailHistory,
                        serde_json::to_string(&checkpoint).unwrap(),
                    )
                }),
            },
        )
        .await
        .unwrap();
    payload
}

fn history(active: &MailSession, baseline: &str) -> Checkpoint {
    Checkpoint::History {
        baseline: baseline.into(),
        page_token: None,
        account: "you@example.test".into(),
        generation: active.authorization().connection_id().into(),
    }
}

#[tokio::test]
async fn full_thread_preserves_recipients_encoded_headers_all_messages_and_attachments_without_external_loads(
) {
    let fixture = Fixture::new().await;
    let active = fixture.session().await;
    let mut incoming = message("incoming", "thread-a", "First — 林");
    let headers = incoming["payload"]["headers"].clone();
    incoming["payload"] = json!({"mimeType":"multipart/mixed","headers":headers,"parts":[plain("0","First — 林"),{"partId":"1","mimeType":"application/pdf","filename":"設計.pdf","body":{"size":42,"attachmentId":"attachment-a"}}]});
    let mut outgoing = message("outgoing", "thread-a", "Reply ✓");
    outgoing["labelIds"] = json!(["SENT"]);
    outgoing["internalDate"] = json!("1794652860000");
    reply(
        &fixture.server,
        "threads/thread-a",
        json!({"id":"thread-a","messages":[outgoing,incoming]}),
    )
    .await;
    let record = fixture
        .client
        .read_thread(&active, "thread-a", "you@example.test")
        .await
        .unwrap();
    assert_eq!(record.conversation.subject, "Café planning");
    assert_eq!(record.conversation.messages.len(), 2);
    assert_eq!(record.conversation.messages[0].body, "First — 林");
    assert_eq!(
        record.conversation.messages[1].direction,
        bruhs_models::MessageDirection::Outgoing
    );
    assert!(record.conversation.unread);
    assert!(!record.conversation.archived);
    let detail = &record.connected_mail.messages[0];
    assert_eq!(detail.envelope.from[0].name, "Zoë");
    assert_eq!(detail.envelope.to.len(), 2);
    assert_eq!(detail.envelope.to[0].name, "Ops, Team");
    assert_eq!(detail.envelope.reply_to[0].address, "replies@example.test");
    assert_eq!(
        detail.envelope.references,
        vec!["original@example.test", "second@example.test"]
    );
    assert_eq!(detail.attachments[0].file_name, "設計.pdf");
    assert_eq!(
        detail.attachments[0].external_id.as_deref(),
        Some("attachment-a")
    );
    assert_eq!(fixture.server.received_requests().await.unwrap().len(), 1);
    drop(active);
}

#[tokio::test]
async fn alternatives_prefer_plain_text_and_html_only_messages_drop_active_markup() {
    let fixture = Fixture::new().await;
    let active = fixture.session().await;
    let mut mail = message("message-a", "thread-a", "placeholder");
    let html = "<style>STYLE-CANARY{}</style><script>SCRIPT-CANARY</script><p>Hello &amp; 林</p><img src='https://external.invalid/tracker'>";
    let html_part = json!({"partId":"html","mimeType":"text/html","body":{"size":html.len(),"data":URL_SAFE_NO_PAD.encode(html)}});
    let headers = mail["payload"]["headers"].clone();
    mail["payload"] = json!({"mimeType":"multipart/alternative","headers":headers,"parts":[plain("plain","Plain preferred"),html_part.clone()]});
    let projected = projection::thread(
        active.authorization().mailbox(),
        "you@example.test",
        serde_json::from_value(json!({"id":"thread-a","messages":[mail.clone()]})).unwrap(),
    )
    .unwrap();
    assert_eq!(projected.conversation.messages[0].body, "Plain preferred");
    assert!(!projected.connected_mail.messages[0].body_from_html);
    mail["payload"]["parts"] = json!([html_part]);
    reply(
        &fixture.server,
        "threads/thread-a",
        json!({"id":"thread-a","messages":[mail]}),
    )
    .await;
    let record = fixture
        .client
        .read_thread(&active, "thread-a", "you@example.test")
        .await
        .unwrap();
    assert!(record.conversation.messages[0].body.contains("Hello & 林"));
    for hidden in ["SCRIPT-CANARY", "STYLE-CANARY", "external.invalid", "<img"] {
        assert!(!record.conversation.messages[0].body.contains(hidden));
    }
    assert!(record.connected_mail.messages[0].body_from_html);
    assert_eq!(fixture.server.received_requests().await.unwrap().len(), 1);
    drop(active);
}

#[tokio::test]
async fn separately_stored_text_is_loaded_completely_and_decoded_using_its_charset() {
    let fixture = Fixture::new().await;
    let active = fixture.session().await;
    let mut record = thread("thread-a", "placeholder");
    record["messages"][0]["payload"]["headers"]
        .as_array_mut()
        .unwrap()
        .retain(|header| header["name"] != "Content-Type");
    record["messages"][0]["payload"]["headers"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"Content-Type","value":"text/plain; charset=iso-8859-1"}));
    record["messages"][0]["payload"]["body"] = json!({"size":4,"attachmentId":"body-part"});
    reply(&fixture.server, "threads/thread-a", record).await;
    reply(
        &fixture.server,
        "messages/thread-a-message/attachments/body-part",
        json!({"size":4,"data":URL_SAFE_NO_PAD.encode(b"caf\xe9")}),
    )
    .await;
    let record = fixture
        .client
        .read_thread(&active, "thread-a", "you@example.test")
        .await
        .unwrap();
    assert_eq!(record.conversation.messages[0].body, "café");
    assert_eq!(fixture.server.received_requests().await.unwrap().len(), 2);
    drop(active);
}

#[tokio::test]
async fn full_scan_resumes_after_restart_and_only_final_completion_prunes_unseen_threads() {
    let Fixture {
        directory,
        runtime,
        connections,
        client,
        server,
    } = Fixture::new().await;
    let active = runtime
        .session(&connections, ServiceProvider::Gmail, "synthetic-connection")
        .await
        .unwrap();
    seed(&active, "stale", "Retained until full completion", None).await;
    let stale = active.authorization().mailbox().thread("stale").unwrap();
    let draft = stale
        .mailbox
        .draft("independent-reply", Some(&stale))
        .unwrap();
    active
        .save_draft(draft.clone(), 0, r#"{"body":"Preserved draft"}"#.into())
        .await
        .unwrap();
    reply(
        &server,
        "profile",
        json!({"emailAddress":"you@example.test","historyId":"100"}),
    )
    .await;
    Mock::given(method("GET"))
        .and(path(format!("{API}/threads")))
        .and(query_param("maxResults", "50"))
        .and(query_param("includeSpamTrash", "true"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"threads":[{"id":"first"}],"nextPageToken":"page&two"})),
        )
        .with_priority(2)
        .mount(&server)
        .await;
    reply(&server, "threads/first", thread("first", "First page")).await;
    let progress = client.sync_page(&active).await.unwrap();
    assert!(!progress.full_scan_complete);
    assert!(runtime
        .cached_thread(stale.clone())
        .await
        .unwrap()
        .is_some());
    let host = runtime.host().clone();
    drop(active);
    drop(runtime);
    let runtime = MailRuntime::open(directory.path().join("mail.sqlite3"))
        .await
        .unwrap();
    assert_eq!(runtime.host(), &host);
    let active = runtime
        .session(&connections, ServiceProvider::Gmail, "synthetic-connection")
        .await
        .unwrap();
    Mock::given(method("GET"))
        .and(path(format!("{API}/threads")))
        .and(query_param("pageToken", "page&two"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"threads":[{"id":"second"}]})),
        )
        .with_priority(1)
        .mount(&server)
        .await;
    reply(&server, "threads/second", thread("second", "Second page")).await;
    let progress = client.sync_page(&active).await.unwrap();
    assert!(progress.full_scan_complete);
    assert!(!progress.caught_up);
    assert!(runtime.cached_thread(stale).await.unwrap().is_none());
    for id in ["first", "second"] {
        assert!(runtime
            .cached_thread(active.authorization().mailbox().thread(id).unwrap())
            .await
            .unwrap()
            .is_some());
    }
    assert!(runtime
        .cached_draft(draft)
        .await
        .unwrap()
        .unwrap()
        .1
        .contains("Preserved draft"));
    reply(&server, "history", json!({"historyId":"105"})).await;
    assert!(client.sync_page(&active).await.unwrap().caught_up);
    let profile_reads = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| request.url.path().ends_with("/profile"))
        .count();
    assert_eq!(profile_reads, 1);
    drop(active);
}

#[tokio::test]
async fn history_deduplicates_changes_fetches_complete_threads_and_preserves_drafts_after_deletion()
{
    let fixture = Fixture::new().await;
    let active = fixture.session().await;
    seed(
        &active,
        "thread-a",
        "Old body",
        Some(history(&active, "100")),
    )
    .await;
    seed(&active, "thread-b", "Deleted remotely", None).await;
    let removed = active.authorization().mailbox().thread("thread-b").unwrap();
    let draft = removed.mailbox.draft("reply", Some(&removed)).unwrap();
    active
        .save_draft(draft.clone(), 0, r#"{"body":"Keep reply"}"#.into())
        .await
        .unwrap();
    reply(&fixture.server,"history",json!({"historyId":"120","history":[{"messages":[{"threadId":"thread-a"}],"messagesAdded":[{"message":{"threadId":"thread-a"}}],"labelsRemoved":[{"message":{"threadId":"thread-a"}}],"messagesDeleted":[{"message":{"threadId":"thread-b"}}]}]})).await;
    let mut updated = thread("thread-a", "Updated complete message");
    updated["messages"][0]["labelIds"] = json!([]);
    reply(&fixture.server, "threads/thread-a", updated).await;
    Mock::given(method("GET"))
        .and(path(format!("{API}/threads/thread-b")))
        .respond_with(ResponseTemplate::new(404))
        .mount(&fixture.server)
        .await;
    let progress = fixture.client.sync_page(&active).await.unwrap();
    assert!(progress.caught_up);
    assert_eq!(progress.threads_updated, 1);
    let saved = fixture
        .runtime
        .cached_thread(active.authorization().mailbox().thread("thread-a").unwrap())
        .await
        .unwrap()
        .unwrap();
    let saved: ConnectedMailThread = serde_json::from_str(&saved).unwrap();
    assert!(saved.conversation.archived);
    assert!(!saved.conversation.unread);
    assert_eq!(
        saved.conversation.messages[0].body,
        "Updated complete message"
    );
    assert!(fixture
        .runtime
        .cached_thread(removed)
        .await
        .unwrap()
        .is_none());
    assert!(fixture.runtime.cached_draft(draft).await.unwrap().is_some());
    assert_eq!(
        fixture
            .server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.url.path().ends_with("/threads/thread-a"))
            .count(),
        1
    );
    drop(active);
}

#[tokio::test]
async fn paged_history_keeps_the_start_baseline_until_the_last_page() {
    let fixture = Fixture::new().await;
    let active = fixture.session().await;
    seed(
        &active,
        "thread-a",
        "Original",
        Some(history(&active, "100")),
    )
    .await;
    Mock::given(method("GET"))
        .and(path(format!("{API}/history")))
        .and(query_param("startHistoryId", "100"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"historyId":"110","nextPageToken":"page-2"})),
        )
        .with_priority(2)
        .mount(&fixture.server)
        .await;
    assert!(!fixture.client.sync_page(&active).await.unwrap().caught_up);
    let (_, cursor) = active
        .sync_checkpoint(CursorScope::GmailHistory)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&cursor.unwrap()).unwrap()["baseline"],
        "100"
    );
    Mock::given(method("GET"))
        .and(path(format!("{API}/history")))
        .and(query_param("pageToken", "page-2"))
        .and(query_param("startHistoryId", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"historyId":"120"})))
        .with_priority(1)
        .mount(&fixture.server)
        .await;
    assert!(fixture.client.sync_page(&active).await.unwrap().caught_up);
    let (_, cursor) = active
        .sync_checkpoint(CursorScope::GmailHistory)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&cursor.unwrap()).unwrap()["baseline"],
        "120"
    );
    drop(active);
}

#[tokio::test]
async fn expired_history_restarts_a_full_scan_instead_of_skipping_the_gap() {
    let fixture = Fixture::new().await;
    let active = fixture.session().await;
    seed(
        &active,
        "stale",
        "Old retained mail",
        Some(history(&active, "100")),
    )
    .await;
    Mock::given(method("GET"))
        .and(path(format!("{API}/history")))
        .and(query_param("startHistoryId", "100"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&fixture.server)
        .await;
    reply(
        &fixture.server,
        "profile",
        json!({"emailAddress":"you@example.test","historyId":"200"}),
    )
    .await;
    reply(
        &fixture.server,
        "threads",
        json!({"threads":[{"id":"current"}]}),
    )
    .await;
    reply(
        &fixture.server,
        "threads/current",
        thread("current", "Recovered from full scan"),
    )
    .await;
    let progress = fixture.client.sync_page(&active).await.unwrap();
    assert!(progress.full_scan_complete);
    assert!(!progress.caught_up);
    assert!(fixture
        .runtime
        .cached_thread(active.authorization().mailbox().thread("stale").unwrap())
        .await
        .unwrap()
        .is_none());
    let (_, cursor) = active
        .sync_checkpoint(CursorScope::GmailHistory)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&cursor.unwrap()).unwrap()["baseline"],
        "200"
    );
    drop(active);
}

#[tokio::test]
async fn a_missing_text_part_does_not_delete_its_thread_or_advance_the_cursor() {
    let fixture = Fixture::new().await;
    let active = fixture.session().await;
    let original = seed(
        &active,
        "thread-a",
        "Preserved full body",
        Some(history(&active, "100")),
    )
    .await;
    let before = active
        .sync_checkpoint(CursorScope::GmailHistory)
        .await
        .unwrap();
    reply(&fixture.server,"history",json!({"historyId":"120","history":[{"messagesAdded":[{"message":{"threadId":"thread-a"}}]}]})).await;
    let mut updated = thread("thread-a", "placeholder");
    updated["messages"][0]["payload"]["body"] = json!({"size":40,"attachmentId":"missing-body"});
    reply(&fixture.server, "threads/thread-a", updated).await;
    Mock::given(method("GET"))
        .and(path(format!(
            "{API}/messages/thread-a-message/attachments/missing-body"
        )))
        .respond_with(ResponseTemplate::new(404))
        .mount(&fixture.server)
        .await;
    assert!(matches!(
        fixture.client.sync_page(&active).await,
        Err(GmailError::InvalidResponse)
    ));
    assert_eq!(
        active
            .sync_checkpoint(CursorScope::GmailHistory)
            .await
            .unwrap(),
        before
    );
    assert_eq!(
        fixture
            .runtime
            .cached_thread(active.authorization().mailbox().thread("thread-a").unwrap())
            .await
            .unwrap()
            .unwrap(),
        original
    );
    drop(active);
}

#[tokio::test]
async fn provider_errors_retain_saved_mail_and_cursor_and_redact_response_content() {
    for status in [401, 403, 429, 500] {
        let fixture = Fixture::new().await;
        let active = fixture.session().await;
        let original = seed(
            &active,
            "thread-a",
            "Preserved",
            Some(history(&active, "100")),
        )
        .await;
        let before = active
            .sync_checkpoint(CursorScope::GmailHistory)
            .await
            .unwrap();
        Mock::given(method("GET"))
            .and(path(format!("{API}/history")))
            .respond_with(ResponseTemplate::new(status).set_body_string("PRIVATE-PROVIDER-DETAIL"))
            .mount(&fixture.server)
            .await;
        let error = fixture.client.sync_page(&active).await.unwrap_err();
        for private in [TOKEN, "PRIVATE-PROVIDER-DETAIL"] {
            assert!(!format!("{error:?} {error}").contains(private));
        }
        assert_eq!(
            active
                .sync_checkpoint(CursorScope::GmailHistory)
                .await
                .unwrap(),
            before
        );
        assert_eq!(
            fixture
                .runtime
                .cached_thread(active.authorization().mailbox().thread("thread-a").unwrap())
                .await
                .unwrap()
                .unwrap(),
            original
        );
        drop(active);
    }
}

#[tokio::test]
async fn malformed_content_and_forged_connected_metadata_cannot_replace_saved_threads() {
    let fixture = Fixture::new().await;
    let active = fixture.session().await;
    let original = seed(&active, "thread-a", "Preserved", None).await;
    for kind in [
        "duplicate",
        "timestamp",
        "base64",
        "truncated",
        "header-injection",
        "charset",
    ] {
        let mut payload = thread("thread-a", "Valid body");
        match kind {
            "duplicate" => {
                let duplicate = payload["messages"][0].clone();
                payload["messages"].as_array_mut().unwrap().push(duplicate);
            }
            "timestamp" => payload["messages"][0]["internalDate"] = json!("NaN"),
            "base64" => payload["messages"][0]["payload"]["body"]["data"] = json!("!!!"),
            "truncated" => payload["messages"][0]["payload"]["body"]["size"] = json!(999),
            "header-injection" => {
                payload["messages"][0]["payload"]["headers"][0]["value"] =
                    json!("original@example.test\r\nFrom: forged@example.test")
            }
            "charset" => payload["messages"][0]["payload"]["headers"]
                .as_array_mut()
                .unwrap()
                .push(
                    json!({"name":"Content-Type","value":"text/plain; charset=unknown-encoding"}),
                ),
            _ => unreachable!(),
        }
        // Replace the original Content-Type rather than introducing an ambiguous duplicate.
        if kind == "charset" {
            payload["messages"][0]["payload"]["headers"]
                .as_array_mut()
                .unwrap()
                .retain(|header| {
                    header["name"] != "Content-Type"
                        || header["value"] == "text/plain; charset=unknown-encoding"
                });
        }
        assert!(
            projection::thread(
                active.authorization().mailbox(),
                "you@example.test",
                serde_json::from_value(payload).unwrap()
            )
            .is_err(),
            "{kind}"
        );
    }
    let mut forged: Value = serde_json::from_str(&original).unwrap();
    forged["connected_mail"]["mailbox_id"] = json!("mailbox-from-another-account");
    assert!(matches!(
        active
            .apply_sync(
                active.revision().await.unwrap(),
                SyncBatch {
                    upserts: vec![ThreadWrite {
                        reference: active.authorization().mailbox().thread("thread-a").unwrap(),
                        payload: forged.to_string()
                    }],
                    confirmed_deleted: vec![],
                    cursor: None
                }
            )
            .await,
        Err(MailError::Cache(CacheError::InvalidInput))
    ));
    assert_eq!(
        fixture
            .runtime
            .cached_thread(active.authorization().mailbox().thread("thread-a").unwrap())
            .await
            .unwrap()
            .unwrap(),
        original
    );
    drop(active);
}

#[tokio::test]
async fn quota_and_domain_policy_errors_are_distinct_from_missing_mail_grants() {
    for (reason, rate_limited) in [
        ("dailyLimitExceeded", true),
        ("rateLimitExceeded", true),
        ("userRateLimitExceeded", true),
        ("domainPolicy", false),
    ] {
        let fixture = Fixture::new().await;
        let active = fixture.session().await;
        seed(
            &active,
            "thread-a",
            "Preserved",
            Some(history(&active, "100")),
        )
        .await;
        let before = active
            .sync_checkpoint(CursorScope::GmailHistory)
            .await
            .unwrap();
        Mock::given(method("GET")).and(path(format!("{API}/history"))).respond_with(ResponseTemplate::new(403).insert_header("Retry-After","120").set_body_json(json!({"error":{"errors":[{"reason":reason,"message":"PRIVATE-PROVIDER-DETAIL"}]}}))).mount(&fixture.server).await;
        let error = fixture.client.sync_page(&active).await.unwrap_err();
        if rate_limited {
            assert!(matches!(
                error,
                GmailError::RateLimited {
                    retry_after_seconds: Some(120)
                }
            ));
        } else {
            assert!(matches!(error, GmailError::DomainPolicy));
        }
        assert!(!format!("{error:?}").contains("PRIVATE-PROVIDER-DETAIL"));
        assert_eq!(
            active
                .sync_checkpoint(CursorScope::GmailHistory)
                .await
                .unwrap(),
            before
        );
        drop(active);
    }
}

#[tokio::test]
async fn a_history_response_cannot_regress_the_saved_baseline() {
    let fixture = Fixture::new().await;
    let active = fixture.session().await;
    seed(
        &active,
        "thread-a",
        "Preserved",
        Some(history(&active, "100000000000000000000")),
    )
    .await;
    let before = active
        .sync_checkpoint(CursorScope::GmailHistory)
        .await
        .unwrap();
    reply(
        &fixture.server,
        "history",
        json!({"historyId":"99999999999999999999"}),
    )
    .await;
    assert!(matches!(
        fixture.client.sync_page(&active).await,
        Err(GmailError::InvalidResponse)
    ));
    assert_eq!(
        active
            .sync_checkpoint(CursorScope::GmailHistory)
            .await
            .unwrap(),
        before
    );
    drop(active);
}

#[tokio::test]
async fn explicit_full_restart_recovers_a_malformed_cursor_without_erasing_drafts() {
    let fixture = Fixture::new().await;
    let active = fixture.session().await;
    seed(&active, "thread-a", "Preserved", None).await;
    let draft = active
        .authorization()
        .mailbox()
        .draft("independent-compose", None)
        .unwrap();
    active
        .save_draft(draft.clone(), 0, r#"{"body":"Keep this draft"}"#.into())
        .await
        .unwrap();
    active
        .apply_sync(
            active.revision().await.unwrap(),
            SyncBatch {
                upserts: vec![],
                confirmed_deleted: vec![],
                cursor: Some((CursorScope::GmailHistory, "malformed-private-cursor".into())),
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        fixture.client.sync_page(&active).await,
        Err(GmailError::InvalidCheckpoint)
    ));
    reply(
        &fixture.server,
        "profile",
        json!({"emailAddress":"you@example.test","historyId":"200"}),
    )
    .await;
    reply(&fixture.server, "threads", json!({"threads":[]})).await;
    assert!(
        fixture
            .client
            .restart_sync_page(&active)
            .await
            .unwrap()
            .full_scan_complete
    );
    assert!(fixture
        .runtime
        .cached_draft(draft)
        .await
        .unwrap()
        .unwrap()
        .1
        .contains("Keep this draft"));
    drop(active);
}

#[test]
fn opaque_history_ids_compare_without_integer_overflow_or_lexical_order_errors() {
    assert!(history_not_older(
        "100000000000000000000",
        "99999999999999999999"
    ));
    assert!(history_not_older("00100", "100"));
    assert!(!history_not_older("99", "100"));
    assert!(!history_not_older("invalid", "100"));
}

#[test]
fn inbox_cards_use_the_actual_counterparty_ahead_of_cc_recipients() {
    let mailbox = crate::mail::MailboxId::from_broker_identity(
        ServiceProvider::Gmail,
        &"a".repeat(64),
        "synthetic-subject",
    )
    .unwrap();
    for outgoing in [false, true] {
        let mut payload = thread("thread-a", "Body");
        if outgoing {
            payload["messages"][0]["labelIds"] = json!(["SENT"]);
        }
        let projected = projection::thread(
            &mailbox,
            "you@example.test",
            serde_json::from_value(payload).unwrap(),
        )
        .unwrap();
        let card = bruhs_models::InboxCard::from(&projected.conversation);
        assert_eq!(card.sender, if outgoing { "Ops, Team" } else { "Zoë" });
    }
    let mut missing = thread("thread-a", "Body");
    missing["messages"][0]["payload"]["headers"]
        .as_array_mut()
        .unwrap()
        .retain(|header| header["name"] != "From");
    let projected = projection::thread(
        &mailbox,
        "you@example.test",
        serde_json::from_value(missing).unwrap(),
    )
    .unwrap();
    assert_eq!(
        bruhs_models::InboxCard::from(&projected.conversation).sender,
        "Unknown sender"
    );
}
