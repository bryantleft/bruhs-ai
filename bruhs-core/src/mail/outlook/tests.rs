use super::*;
use crate::{
    connections::Connections,
    mail::{CacheError, CursorScope, MailRuntime, SyncBatch, ThreadWrite},
};
use bruhs_models::mail::ConnectedMailThread;
use serde_json::{json, Value};
use wiremock::{
    matchers::{header, headers, method, path, query_param},
    Mock, MockServer, ResponseTemplate,
};

const API: &str = "/v1.0/me";
const TOKEN: &str = "synthetic-outlook-token";

struct Fixture {
    _directory: tempfile::TempDir,
    runtime: MailRuntime,
    connections: Connections,
    client: OutlookClient,
    server: MockServer,
}

impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let runtime = MailRuntime::open(directory.path().join("mail.sqlite3"))
            .await
            .unwrap();
        let connections = Connections::test_connected(ServiceProvider::Outlook, TOKEN).await;
        let server = MockServer::builder().start().await;
        let client = OutlookClient::for_test_origin(&server.uri()).unwrap();
        Self {
            _directory: directory,
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
                ServiceProvider::Outlook,
                "synthetic-connection",
            )
            .await
            .unwrap()
    }

    fn link(&self, resource: &str, query: &str) -> String {
        format!("{}{API}/{resource}?{query}", self.server.uri())
    }

    async fn reply(&self, resource: &str, query: &[(&str, &str)], value: Value) {
        self.response(
            resource,
            query,
            ResponseTemplate::new(200).set_body_json(value),
        )
        .await;
    }

    async fn response(&self, resource: &str, query: &[(&str, &str)], response: ResponseTemplate) {
        let route = if resource.is_empty() {
            API.into()
        } else {
            format!("{API}/{resource}")
        };
        let mut mock = Mock::given(method("GET"))
            .and(path(route))
            .and(header("authorization", format!("Bearer {TOKEN}")))
            .and(headers(
                "prefer",
                PREFER.split(',').map(str::trim).collect(),
            ));
        for (key, value) in query {
            mock = mock.and(query_param(*key, *value));
        }
        mock.respond_with(response).mount(&self.server).await;
    }

    async fn profile(&self) {
        self.reply("", &[], json!({"id":"subject","mail":"you@example.test","userPrincipalName":"you@example.test"})).await;
        self.reply("mailFolders/inbox", &[], json!({"id":"folder-inbox"}))
            .await;
        self.reply("mailFolders/sentitems", &[], json!({"id":"folder-sent"}))
            .await;
    }

    async fn thread(&self, id: &str, messages: Vec<Value>) {
        self.reply(
            "messages",
            &[(
                "$filter",
                &format!("conversationId eq '{}'", id.replace('\'', "''")),
            )],
            json!({"value":messages}),
        )
        .await;
    }

    async fn seed(
        &self,
        session: &MailSession,
        id: &str,
        checkpoint: Option<Value>,
    ) -> ConnectedMailThread {
        let record = projection::thread(
            session.authorization().mailbox(),
            &context(),
            id,
            vec![serde_json::from_value(message(
                "message-a",
                id,
                "folder-inbox",
                "Saved before failure",
            ))
            .unwrap()],
        )
        .unwrap();
        session
            .apply_sync(
                session.revision().await.unwrap(),
                SyncBatch {
                    upserts: vec![ThreadWrite {
                        reference: session.authorization().mailbox().thread(id).unwrap(),
                        payload: serde_json::to_string(&record).unwrap(),
                    }],
                    confirmed_deleted: vec![],
                    cursor: checkpoint
                        .map(|checkpoint| (CursorScope::OutlookMailbox, checkpoint.to_string())),
                },
            )
            .await
            .unwrap();
        record
    }

    async fn saved(&self, session: &MailSession, id: &str) -> Option<ConnectedMailThread> {
        self.runtime
            .connected_thread(
                self.runtime.host().as_str().into(),
                session.authorization().mailbox().as_str().into(),
                id.into(),
            )
            .await
            .unwrap()
    }

    async fn cursor(&self, session: &MailSession) -> String {
        self.runtime
            .cached_cursor(
                session.authorization().mailbox().clone(),
                CursorScope::OutlookMailbox,
            )
            .await
            .unwrap()
            .unwrap()
    }

    fn delta(&self, session: &MailSession, token: &str) -> Value {
        json!({"generation":session.authorization().connection_id(),"context":{"account":"you@example.test","inbox":"folder-inbox","sent":"folder-sent"},"folders":[{"id":"folder-inbox","cursor":self.link("mailFolders/folder-inbox/messages/delta", &format!("$deltatoken={token}"))}],"state":{"phase":"delta","index":0,"seen":[]}})
    }
}

fn context() -> MailboxContext {
    MailboxContext {
        account: "you@example.test".into(),
        inbox: "folder-inbox".into(),
        sent: "folder-sent".into(),
    }
}

fn message(id: &str, thread: &str, folder: &str, text: &str) -> Value {
    json!({"id":id,"conversationId":thread,"parentFolderId":folder,"subject":"Café — 林","body":{"contentType":"text","content":text},"from":{"emailAddress":{"name":null,"address":"sender@example.test"}},"sender":{"emailAddress":{"name":"Sender","address":"sender@example.test"}},"toRecipients":[{"emailAddress":{"name":"You","address":"you@example.test"}}],"ccRecipients":[{"emailAddress":{"name":"Copy","address":"copy@example.test"}}],"bccRecipients":[],"replyTo":[{"emailAddress":{"name":"Reply","address":"reply@example.test"}}],"internetMessageId":format!("<{id}@example.test>"),"internetMessageHeaders":[{"name":"References","value":"<original@example.test> <second@example.test>"}],"receivedDateTime":"2026-11-14T00:00:00Z","sentDateTime":"2026-11-14T00:00:01Z","createdDateTime":"2026-11-14T00:00:00Z","isRead":false,"isDraft":false,"importance":"high","categories":["Work"],"attachments":[]})
}

fn attachment(id: &str, kind: &str, size: usize) -> Value {
    json!({"id":id,"@odata.type":format!("#microsoft.graph.{kind}"),"name":"../../設計.txt","contentType":"text/plain","size":size,"isInline":true,"contentId":"inline-a"})
}

#[tokio::test]
async fn complete_cross_folder_thread_keeps_recipients_unicode_inline_metadata_and_safe_html() {
    let f = Fixture::new().await;
    let session = f.session().await;
    f.profile().await;
    let mut incoming = message("incoming", "thread-a", "folder-inbox", "First — 林");
    incoming["attachments"] = json!([attachment("inline-a", "fileAttachment", 3)]);
    let mut outgoing = message("outgoing", "thread-a", "folder-sent", "unused");
    outgoing["body"] = json!({"contentType":"html","content":"<style>STYLE-CANARY</style><script>SCRIPT-CANARY</script><p>Hello &amp; 林</p><img src='https://external.invalid/tracker'>"});
    outgoing["isRead"] = json!(true);
    f.thread("thread-a", vec![outgoing, incoming]).await;
    let record = f.client.read_thread(&session, "thread-a").await.unwrap();
    assert_eq!(record.connected_mail.provider, ServiceProvider::Outlook);
    assert_eq!(record.conversation.subject, "Café — 林");
    assert!(record.conversation.unread);
    assert!(!record.conversation.archived);
    assert_eq!(record.conversation.messages.len(), 2);
    assert_eq!(record.conversation.messages[0].body, "First — 林");
    assert_eq!(
        record.conversation.messages[1].direction,
        bruhs_models::MessageDirection::Outgoing
    );
    for text in ["SCRIPT-CANARY", "STYLE-CANARY", "external.invalid", "<img"] {
        assert!(!record.conversation.messages[1].body.contains(text));
    }
    let detail = &record.connected_mail.messages[0];
    assert_eq!(detail.envelope.from[0].name, "");
    assert_eq!(detail.envelope.cc[0].address, "copy@example.test");
    assert_eq!(detail.envelope.reply_to[0].address, "reply@example.test");
    assert_eq!(
        detail.envelope.references,
        vec!["original@example.test", "second@example.test"]
    );
    assert_eq!(
        detail.attachments[0].content_id.as_deref(),
        Some("inline-a")
    );
    let requests = f.server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 4);
    assert!(requests
        .iter()
        .find(|request| request.url.path() == format!("{API}/messages"))
        .unwrap()
        .url
        .query_pairs()
        .any(|(key, value)| key == "$expand" && value.contains("attachments($select=")));
    drop(session);
}

#[tokio::test]
async fn full_sync_anchors_nested_folders_before_scan_and_only_final_account_page_reconciles() {
    let f = Fixture::new().await;
    let session = f.session().await;
    f.profile().await;
    f.seed(&session, "obsolete", None).await;
    let draft = session
        .authorization()
        .mailbox()
        .draft("keep-draft", None)
        .unwrap();
    session
        .save_draft(
            draft.clone(),
            0,
            json!({"body":"reviewed draft survives"}).to_string(),
        )
        .await
        .unwrap();
    f.reply("mailFolders", &[], json!({"value":[{"id":"folder-inbox","childFolderCount":1},{"id":"folder-sent","childFolderCount":0}]})).await;
    f.reply(
        "mailFolders/folder-inbox/childFolders",
        &[],
        json!({"value":[{"id":"folder-nested","childFolderCount":0}]}),
    )
    .await;
    for folder in ["folder-inbox", "folder-nested", "folder-sent"] {
        let resource = format!("mailFolders/{folder}/messages/delta");
        f.reply(
            &resource,
            &[("$select", "id,conversationId")],
            json!({"value":[],"@odata.deltaLink":f.link(&resource,"$deltatoken=anchored")}),
        )
        .await;
        f.reply(
            &resource,
            &[("$deltatoken", "anchored")],
            json!({"value":[],"@odata.deltaLink":f.link(&resource,"$deltatoken=done")}),
        )
        .await;
    }
    f.reply("messages", &[("$select","id,conversationId")], json!({"value":[{"id":"in-a","conversationId":"thread-a"}],"@odata.nextLink":f.link("messages","$skiptoken=final")})).await;
    f.reply("messages", &[("$skiptoken", "final")], json!({"value":[]}))
        .await;
    f.thread(
        "thread-a",
        vec![
            message("in-a", "thread-a", "folder-inbox", "Incoming"),
            message("out-a", "thread-a", "folder-sent", "Reply"),
        ],
    )
    .await;
    // Two discovery pages and three folder baselines precede the first scan page.
    for _ in 0..5 {
        let progress = f.client.sync_page(&session).await.unwrap();
        assert!(!progress.full_scan_complete);
        assert!(f.saved(&session, "obsolete").await.is_some());
    }
    assert!(!f
        .server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .any(|request| request.url.path() == format!("{API}/messages")));
    assert!(
        !f.client
            .sync_page(&session)
            .await
            .unwrap()
            .full_scan_complete
    );
    assert!(f.saved(&session, "obsolete").await.is_some());
    assert_eq!(
        f.saved(&session, "thread-a")
            .await
            .unwrap()
            .conversation
            .messages
            .len(),
        2
    );
    assert!(
        f.client
            .sync_page(&session)
            .await
            .unwrap()
            .full_scan_complete
    );
    assert!(f.saved(&session, "obsolete").await.is_none());
    for index in 0..3 {
        assert_eq!(
            f.client.sync_page(&session).await.unwrap().caught_up,
            index == 2
        );
    }
    assert_eq!(
        f.runtime.cached_draft(draft).await.unwrap().unwrap().1,
        json!({"body":"reviewed draft survives"}).to_string()
    );
    drop(session);
}

#[tokio::test]
async fn move_tombstone_advances_cursor_and_preserves_complete_conversation_after_rescan() {
    let f = Fixture::new().await;
    let session = f.session().await;
    let original = f
        .seed(&session, "thread-a", Some(f.delta(&session, "before")))
        .await;
    f.reply("mailFolders/folder-inbox/messages/delta", &[("$deltatoken","before")], json!({"value":[{"id":"message-a","@removed":{"reason":"deleted"}}],"@odata.deltaLink":f.link("mailFolders/folder-inbox/messages/delta","$deltatoken=after")})).await;
    assert!(
        !f.client
            .sync_page(&session)
            .await
            .unwrap()
            .full_scan_complete
    );
    assert_eq!(f.saved(&session, "thread-a").await, Some(original));
    let saved: Value = serde_json::from_str(&f.cursor(&session).await).unwrap();
    assert_eq!(saved["state"]["phase"], "full");
    assert!(saved["folders"][0]["cursor"]
        .as_str()
        .unwrap()
        .contains("after"));
    f.reply(
        "messages",
        &[("$select", "id,conversationId")],
        json!({"value":[{"id":"message-a","conversationId":"thread-a"}]}),
    )
    .await;
    f.thread(
        "thread-a",
        vec![message(
            "message-a",
            "thread-a",
            "folder-sent",
            "Moved, not deleted",
        )],
    )
    .await;
    f.client.sync_page(&session).await.unwrap();
    assert_eq!(
        f.saved(&session, "thread-a")
            .await
            .unwrap()
            .conversation
            .messages[0]
            .body,
        "Moved, not deleted"
    );
    f.reply("mailFolders/folder-inbox/messages/delta", &[("$deltatoken","after")],json!({"value":[],"@odata.deltaLink":f.link("mailFolders/folder-inbox/messages/delta","$deltatoken=after")})).await;
    assert!(f.client.sync_page(&session).await.unwrap().caught_up);
    assert_eq!(
        f.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request
                .url
                .query_pairs()
                .any(|(key, value)| key == "$deltatoken" && value == "before"))
            .count(),
        1
    );
    drop(session);
}

#[tokio::test]
async fn genuine_deletion_reconciles_only_after_successful_final_scan_and_keeps_draft() {
    let f = Fixture::new().await;
    let session = f.session().await;
    f.seed(&session, "thread-a", Some(f.delta(&session, "before")))
        .await;
    let draft = session
        .authorization()
        .mailbox()
        .draft("independent", None)
        .unwrap();
    session
        .save_draft(draft.clone(), 0, json!({"body":"newer draft"}).to_string())
        .await
        .unwrap();
    f.reply("mailFolders/folder-inbox/messages/delta",&[],json!({"value":[{"id":"message-a","@removed":{}}],"@odata.deltaLink":f.link("mailFolders/folder-inbox/messages/delta","$deltatoken=next")})).await;
    f.client.sync_page(&session).await.unwrap();
    let cursor = f.cursor(&session).await;
    f.response(
        "messages",
        &[],
        ResponseTemplate::new(503).set_body_string("PRIVATE-SERVER-CANARY"),
    )
    .await;
    assert!(matches!(
        f.client.sync_page(&session).await,
        Err(OutlookError::Network)
    ));
    assert_eq!(f.cursor(&session).await, cursor);
    assert!(f.saved(&session, "thread-a").await.is_some());
    f.server.reset().await;
    f.reply("messages", &[], json!({"value":[]})).await;
    assert!(
        f.client
            .sync_page(&session)
            .await
            .unwrap()
            .full_scan_complete
    );
    assert!(f.saved(&session, "thread-a").await.is_none());
    assert_eq!(
        f.runtime.cached_draft(draft).await.unwrap().unwrap().1,
        json!({"body":"newer draft"}).to_string()
    );
    drop(session);
}

#[tokio::test]
async fn disappearing_partial_delta_message_requires_reconciliation_instead_of_silently_dropping_change(
) {
    let f = Fixture::new().await;
    let session = f.session().await;
    f.seed(&session, "thread-a", Some(f.delta(&session, "old")))
        .await;
    f.reply("mailFolders/folder-inbox/messages/delta",&[],json!({"value":[{"id":"message-a"}],"@odata.deltaLink":f.link("mailFolders/folder-inbox/messages/delta","$deltatoken=new")})).await;
    f.response("messages/message-a", &[], ResponseTemplate::new(404))
        .await;
    assert!(
        !f.client
            .sync_page(&session)
            .await
            .unwrap()
            .full_scan_complete
    );
    assert_eq!(
        serde_json::from_str::<Value>(&f.cursor(&session).await).unwrap()["state"]["phase"],
        "full"
    );
    assert!(f.saved(&session, "thread-a").await.is_some());
    drop(session);
}

#[tokio::test]
async fn expired_delta_restarts_discovery_without_removing_saved_mail() {
    let f = Fixture::new().await;
    let session = f.session().await;
    f.profile().await;
    f.seed(&session, "thread-a", Some(f.delta(&session, "expired")))
        .await;
    f.response(
        "mailFolders/folder-inbox/messages/delta",
        &[],
        ResponseTemplate::new(410),
    )
    .await;
    assert!(!f.client.sync_page(&session).await.unwrap().caught_up);
    assert_eq!(
        serde_json::from_str::<Value>(&f.cursor(&session).await).unwrap()["state"]["phase"],
        "discover"
    );
    assert!(f.saved(&session, "thread-a").await.is_some());
    drop(session);
}

#[tokio::test]
async fn non_410_expired_delta_restarts_discovery_and_keeps_drafts() {
    for status in [400, 404] {
        let f = Fixture::new().await;
        let session = f.session().await;
        f.profile().await;
        let saved = f
            .seed(&session, "thread-a", Some(f.delta(&session, "expired")))
            .await;
        let draft = session
            .authorization()
            .mailbox()
            .draft("keep-through-expiration", None)
            .unwrap();
        let payload = json!({"body":"Keep this draft through cursor recovery"}).to_string();
        session
            .save_draft(draft.clone(), 0, payload.clone())
            .await
            .unwrap();
        f.response(
            "mailFolders/folder-inbox/messages/delta",
            &[("$deltatoken", "expired")],
            ResponseTemplate::new(status)
                .set_body_json(json!({"error":{"code":"syncStateNotFound"}})),
        )
        .await;
        assert!(!f.client.sync_page(&session).await.unwrap().caught_up);
        assert_eq!(
            serde_json::from_str::<Value>(&f.cursor(&session).await).unwrap()["state"]["phase"],
            "discover"
        );
        assert_eq!(f.saved(&session, "thread-a").await, Some(saved.clone()));
        assert_eq!(
            f.runtime
                .cached_draft(draft.clone())
                .await
                .unwrap()
                .unwrap()
                .1,
            payload
        );
        f.reply("mailFolders", &[], json!({"value":[]})).await;
        f.client.sync_page(&session).await.unwrap();
        assert_eq!(
            f.server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|request| request
                    .url
                    .query_pairs()
                    .any(|(key, value)| key == "$deltatoken" && value == "expired"))
                .count(),
            1
        );
        assert_eq!(f.saved(&session, "thread-a").await, Some(saved));
        assert_eq!(
            f.runtime.cached_draft(draft).await.unwrap().unwrap().1,
            payload
        );
        drop(session);
    }
}

#[tokio::test]
async fn attachment_page_404_cannot_commit_a_partial_full_scan() {
    let f = Fixture::new().await;
    let session = f.session().await;
    let mut checkpoint = f.delta(&session, "anchor");
    checkpoint["state"] = json!({"phase":"full","run_id":"attachment-retry","next":null,"seen":[]});
    let cached = projection::thread(
        session.authorization().mailbox(),
        &context(),
        "thread-a",
        vec![
            serde_json::from_value(message("first", "thread-a", "folder-inbox", "Cached first"))
                .unwrap(),
            serde_json::from_value(message(
                "second",
                "thread-a",
                "folder-inbox",
                "Surviving second",
            ))
            .unwrap(),
        ],
    )
    .unwrap();
    session
        .apply_sync(
            session.revision().await.unwrap(),
            SyncBatch {
                upserts: vec![ThreadWrite {
                    reference: session
                        .authorization()
                        .mailbox()
                        .thread("thread-a")
                        .unwrap(),
                    payload: serde_json::to_string(&cached).unwrap(),
                }],
                confirmed_deleted: vec![],
                cursor: Some((CursorScope::OutlookMailbox, checkpoint.to_string())),
            },
        )
        .await
        .unwrap();
    let draft = session
        .authorization()
        .mailbox()
        .draft("keep-through-attachment-race", None)
        .unwrap();
    let payload = json!({"body":"Keep this independent draft"}).to_string();
    session
        .save_draft(draft.clone(), 0, payload.clone())
        .await
        .unwrap();
    f.reply(
        "messages",
        &[("$select", "id,conversationId")],
        json!({"value":[{"id":"second","conversationId":"thread-a"}]}),
    )
    .await;
    let mut disappearing = message("first", "thread-a", "folder-inbox", "Cached first");
    disappearing["attachments@odata.nextLink"] =
        json!(f.link("messages/first/attachments", "$skiptoken=gone-message"));
    f.thread(
        "thread-a",
        vec![
            disappearing,
            message("second", "thread-a", "folder-inbox", "Surviving second"),
        ],
    )
    .await;
    f.response(
        "messages/first/attachments",
        &[("$skiptoken", "gone-message")],
        ResponseTemplate::new(404),
    )
    .await;
    let before_cursor = f.cursor(&session).await;
    let before_revision = session.revision().await.unwrap();
    let result = f.client.sync_page(&session).await;
    assert_eq!(
        f.saved(&session, "thread-a").await,
        Some(cached),
        "Incomplete attachment hydration must not authorize conversation eviction"
    );
    assert!(matches!(result, Err(OutlookError::NotFound)));
    assert_eq!(f.cursor(&session).await, before_cursor);
    assert_eq!(session.revision().await.unwrap(), before_revision);
    assert_eq!(
        f.runtime
            .cached_draft(draft.clone())
            .await
            .unwrap()
            .unwrap()
            .1,
        payload
    );
    f.server.reset().await;
    f.reply(
        "messages",
        &[("$select", "id,conversationId")],
        json!({"value":[{"id":"second","conversationId":"thread-a"}]}),
    )
    .await;
    f.thread(
        "thread-a",
        vec![message(
            "second",
            "thread-a",
            "folder-inbox",
            "Surviving second",
        )],
    )
    .await;
    assert!(
        f.client
            .sync_page(&session)
            .await
            .unwrap()
            .full_scan_complete
    );
    let remaining = f.saved(&session, "thread-a").await.unwrap();
    assert_eq!(remaining.connected_mail.messages.len(), 1);
    assert_eq!(remaining.connected_mail.messages[0].external_id, "second");
    assert_eq!(
        f.runtime.cached_draft(draft).await.unwrap().unwrap().1,
        payload
    );
    drop(session);
}

#[tokio::test]
async fn reauthorization_discards_prior_generation_cursor_and_preserves_saved_records() {
    let f = Fixture::new().await;
    let session = f.session().await;
    f.profile().await;
    let mut checkpoint = f.delta(&session, "do-not-replay");
    checkpoint["generation"] = json!("previous-generation");
    f.seed(&session, "thread-a", Some(checkpoint)).await;
    f.reply(
        "mailFolders",
        &[],
        json!({"value":[{"id":"folder-inbox","childFolderCount":0}]}),
    )
    .await;
    f.client.sync_page(&session).await.unwrap();
    assert!(f.saved(&session, "thread-a").await.is_some());
    assert!(!f
        .server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .any(|request| request.url.as_str().contains("do-not-replay")));
    assert_eq!(
        serde_json::from_str::<Value>(&f.cursor(&session).await).unwrap()["generation"],
        session.authorization().connection_id()
    );
    drop(session);
}

#[tokio::test]
async fn hostile_continuation_never_receives_bearer_token_and_never_advances_checkpoint() {
    let f = Fixture::new().await;
    let session = f.session().await;
    f.seed(&session, "thread-a", Some(f.delta(&session, "old")))
        .await;
    let before = f.cursor(&session).await;
    f.reply(
        "mailFolders/folder-inbox/messages/delta",
        &[],
        json!({"value":[],"@odata.nextLink":"https://external.invalid/collect?$skiptoken=secret"}),
    )
    .await;
    assert!(matches!(
        f.client.sync_page(&session).await,
        Err(OutlookError::InvalidResponse)
    ));
    assert_eq!(f.cursor(&session).await, before);
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    drop(session);
}

#[tokio::test]
async fn throttling_retains_checkpoint_and_returns_retry_after_without_provider_body() {
    let f = Fixture::new().await;
    let session = f.session().await;
    f.seed(&session, "thread-a", Some(f.delta(&session, "old")))
        .await;
    let before = f.cursor(&session).await;
    f.response(
        "mailFolders/folder-inbox/messages/delta",
        &[],
        ResponseTemplate::new(429)
            .insert_header("Retry-After", "17")
            .set_body_string("PRIVATE-TOKEN-CANARY"),
    )
    .await;
    let error = f.client.sync_page(&session).await.err().unwrap();
    assert!(matches!(
        error,
        OutlookError::RateLimited {
            retry_after_seconds: Some(17)
        }
    ));
    assert!(!error.to_string().contains("PRIVATE"));
    assert_eq!(f.cursor(&session).await, before);
    drop(session);
}

#[test]
fn continuations_preserve_opaque_queries_but_reject_other_origin_resource_credentials_and_fragment()
{
    let client = OutlookClient::new().unwrap();
    for url in [
        "https://evil.invalid/v1.0/me/messages?$skiptoken=x",
        "https://user:pass@graph.microsoft.com/v1.0/me/messages?$skiptoken=x",
        "https://graph.microsoft.com/v1.0/users/other/messages?$skiptoken=x",
        "https://graph.microsoft.com/v1.0/me/drive?$skiptoken=x",
        "https://graph.microsoft.com/v1.0/me/messages?$skiptoken=x#bad",
    ] {
        assert!(client.checked_link(url, &["messages"]).is_err());
    }
    let valid="https://graph.microsoft.com/v1.0/me/mailFolders('folder-A')/messages/delta?$skiptoken=A%2BB%2F%3D";
    assert_eq!(
        client
            .checked_link(valid, &["mailFolders", "folder-A", "messages", "delta"])
            .unwrap()
            .as_str(),
        valid
    );
    assert!(client
        .checked_link(
            &valid.replace("mailFolders", "mailfolders"),
            &["mailFolders", "folder-A", "messages", "delta"]
        )
        .is_ok());
    assert!(client
        .checked_link(valid, &["mailFolders", "folder-a", "messages", "delta"])
        .is_err());
}

#[tokio::test]
async fn explicit_file_download_uses_cached_account_reference_and_never_opens_remote_content() {
    let f = Fixture::new().await;
    let session = f.session().await;
    let mut value = message("message-a", "thread-a", "folder-inbox", "Body");
    value["attachments"] = json!([
        attachment("file-a", "fileAttachment", 3),
        attachment("link-a", "referenceAttachment", 0)
    ]);
    let record = projection::thread(
        session.authorization().mailbox(),
        &context(),
        "thread-a",
        vec![serde_json::from_value(value).unwrap()],
    )
    .unwrap();
    session
        .apply_sync(
            session.revision().await.unwrap(),
            SyncBatch {
                upserts: vec![ThreadWrite {
                    reference: session
                        .authorization()
                        .mailbox()
                        .thread("thread-a")
                        .unwrap(),
                    payload: serde_json::to_string(&record).unwrap(),
                }],
                confirmed_deleted: vec![],
                cursor: None,
            },
        )
        .await
        .unwrap();
    f.reply(
        "messages/message-a/attachments/file-a",
        &[],
        attachment("file-a", "fileAttachment", 3),
    )
    .await;
    f.response(
        "messages/message-a/attachments/file-a/$value",
        &[],
        ResponseTemplate::new(200).set_body_bytes(b"abc"),
    )
    .await;
    let file = &record.connected_mail.messages[0].attachments[0];
    let result = f
        .client
        .attachment(&session, "thread-a", &file.id)
        .await
        .unwrap();
    assert_eq!(result.bytes, b"abc");
    assert_eq!(result.metadata.id, file.id);
    let linked = &record.connected_mail.messages[0].attachments[1];
    assert!(matches!(
        f.client.attachment(&session, "thread-a", &linked.id).await,
        Err(OutlookError::UnsupportedAttachment)
    ));
    let foreign = crate::mail::MailboxId::from_broker_identity(
        ServiceProvider::Outlook,
        &"2".repeat(64),
        "other-account",
    )
    .unwrap()
    .attachment("message-a", "file-a")
    .unwrap();
    assert!(matches!(
        f.client.attachment(&session, "thread-a", &foreign).await,
        Err(OutlookError::NotFound)
    ));
    assert_eq!(f.server.received_requests().await.unwrap().len(), 2);
    drop(session);
}

#[tokio::test]
async fn stale_mailbox_revision_cannot_commit_provider_page() {
    use axum::{routing::get, Json, Router};
    let f = Fixture::new().await;
    let session = f.session().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let link = format!("{origin}{API}/mailFolders/folder-inbox/messages/delta?$deltatoken=next");
    let mut checkpoint = f.delta(&session, "old");
    checkpoint["folders"][0]["cursor"] = json!(format!(
        "{origin}{API}/mailFolders/folder-inbox/messages/delta?$deltatoken=old"
    ));
    f.seed(&session, "thread-a", Some(checkpoint)).await;
    let runtime = f.runtime.clone();
    let auth = session.authorization().clone();
    let app = Router::new().route(
        &format!("{API}/mailFolders/folder-inbox/messages/delta"),
        get(move || {
            let runtime = runtime.clone();
            let auth = auth.clone();
            let link = link.clone();
            async move {
                runtime
                    .call(true, move |cache| {
                        let revision = cache.bind(&auth)?;
                        Ok(cache.apply_sync(
                            &auth,
                            revision,
                            &SyncBatch {
                                upserts: vec![],
                                confirmed_deleted: vec![],
                                cursor: None,
                            },
                        )?)
                    })
                    .await
                    .unwrap();
                Json(json!({"value":[],"@odata.deltaLink":link}))
            }
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let result = OutlookClient::for_test_origin(&origin)
        .unwrap()
        .sync_page(&session)
        .await;
    server.abort();
    assert!(matches!(
        result,
        Err(OutlookError::Mail(crate::mail::MailError::Cache(
            CacheError::RevisionConflict
        )))
    ));
    assert!(f.cursor(&session).await.contains("$deltatoken=old"));
    drop(session);
}

#[tokio::test]
async fn deletion_during_initial_scan_is_caught_from_earlier_delta_anchor() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let f = Fixture::new().await;
    let session = f.session().await;
    f.profile().await;
    f.reply(
        "mailFolders",
        &[],
        json!({"value":[{"id":"folder-inbox","childFolderCount":0}]}),
    )
    .await;
    let resource = "mailFolders/folder-inbox/messages/delta";
    f.reply(
        resource,
        &[("$select", "id,conversationId")],
        json!({"value":[],"@odata.deltaLink":f.link(resource,"$deltatoken=anchor")}),
    )
    .await;
    f.reply(resource,&[("$deltatoken","anchor")],json!({"value":[{"id":"message-a","@removed":{}}],"@odata.deltaLink":f.link(resource,"$deltatoken=after")})).await;
    f.reply(
        resource,
        &[("$deltatoken", "after")],
        json!({"value":[],"@odata.deltaLink":f.link(resource,"$deltatoken=after")}),
    )
    .await;
    let scans = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&scans);
    Mock::given(method("GET"))
        .and(path(format!("{API}/messages")))
        .and(query_param("$select", "id,conversationId"))
        .respond_with(move |_: &wiremock::Request| {
            ResponseTemplate::new(200).set_body_json(
                if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                    json!({"value":[{"id":"message-a","conversationId":"thread-a"}]})
                } else {
                    json!({"value":[]})
                },
            )
        })
        .mount(&f.server)
        .await;
    f.thread(
        "thread-a",
        vec![message(
            "message-a",
            "thread-a",
            "folder-inbox",
            "Deleted immediately after scanning",
        )],
    )
    .await;
    for _ in 0..3 {
        f.client.sync_page(&session).await.unwrap();
    }
    assert!(f.saved(&session, "thread-a").await.is_some());
    assert!(
        !f.client
            .sync_page(&session)
            .await
            .unwrap()
            .full_scan_complete
    );
    assert!(f.saved(&session, "thread-a").await.is_some());
    assert!(
        f.client
            .sync_page(&session)
            .await
            .unwrap()
            .full_scan_complete
    );
    assert!(f.saved(&session, "thread-a").await.is_none());
    assert!(f.client.sync_page(&session).await.unwrap().caught_up);
    assert_eq!(scans.load(Ordering::SeqCst), 2);
    drop(session);
}

#[tokio::test]
async fn conversation_and_inline_attachment_pagination_are_complete_before_projection() {
    let f = Fixture::new().await;
    let session = f.session().await;
    f.profile().await;
    let mut first = message("first", "thread-a", "folder-inbox", "First page");
    first["attachments@odata.nextLink"] =
        json!(f.link("messages/first/attachments", "$skiptoken=attachments-next"));
    f.reply(
        "messages",
        &[("$filter", "conversationId eq 'thread-a'")],
        json!({"value":[first],"@odata.nextLink":f.link("messages","$skiptoken=thread-next")}),
    )
    .await;
    f.reply(
        "messages",
        &[("$skiptoken", "thread-next")],
        json!({"value":[message("second","thread-a","folder-sent","Second page")]}),
    )
    .await;
    f.reply(
        "messages/first/attachments",
        &[("$skiptoken", "attachments-next")],
        json!({"value":[attachment("inline-page-two","fileAttachment",3)]}),
    )
    .await;
    let record = f.client.read_thread(&session, "thread-a").await.unwrap();
    assert_eq!(record.conversation.messages.len(), 2);
    assert_eq!(record.connected_mail.messages[0].attachments.len(), 1);
    assert_eq!(
        record.connected_mail.messages[0].attachments[0]
            .external_id
            .as_deref(),
        Some("inline-page-two")
    );
    assert_eq!(f.server.received_requests().await.unwrap().len(), 6);
    drop(session);
}

#[tokio::test]
async fn changed_folder_inventory_anchors_new_folders_before_global_reconciliation() {
    let f = Fixture::new().await;
    let session = f.session().await;
    let mut checkpoint = f.delta(&session, "old");
    checkpoint["state"] = json!({"phase":"discover","pending":[{"parent":null,"next":null,"seen":[]}],"discovered":[],"require_full":false});
    f.seed(&session, "saved", Some(checkpoint)).await;
    f.reply(
        "mailFolders",
        &[],
        json!({"value":[{"id":"new-folder","childFolderCount":0}]}),
    )
    .await;
    f.client.sync_page(&session).await.unwrap();
    assert!(f.saved(&session, "saved").await.is_some());
    let resource = "mailFolders/new-folder/messages/delta";
    f.reply(
        resource,
        &[],
        json!({"value":[],"@odata.deltaLink":f.link(resource,"$deltatoken=new-anchor")}),
    )
    .await;
    assert!(
        !f.client
            .sync_page(&session)
            .await
            .unwrap()
            .full_scan_complete
    );
    assert!(f.saved(&session, "saved").await.is_some());
    let saved: Value = serde_json::from_str(&f.cursor(&session).await).unwrap();
    assert_eq!(saved["state"]["phase"], "full");
    assert_eq!(saved["folders"][0]["id"], "new-folder");
    f.reply("messages", &[], json!({"value":[]})).await;
    assert!(
        f.client
            .sync_page(&session)
            .await
            .unwrap()
            .full_scan_complete
    );
    assert!(f.saved(&session, "saved").await.is_none());
    drop(session);
}

#[tokio::test]
async fn missing_collection_value_cannot_be_treated_as_empty_mailbox() {
    let f = Fixture::new().await;
    let session = f.session().await;
    let mut checkpoint = f.delta(&session, "old");
    checkpoint["state"] =
        json!({"phase":"full","run_id":"run-for-malformed-page","next":null,"seen":[]});
    f.seed(&session, "saved", Some(checkpoint)).await;
    let before = f.cursor(&session).await;
    f.reply(
        "messages",
        &[],
        json!({"@odata.context":"malformed-response"}),
    )
    .await;
    assert!(matches!(
        f.client.sync_page(&session).await,
        Err(OutlookError::InvalidResponse)
    ));
    assert_eq!(f.cursor(&session).await, before);
    assert!(f.saved(&session, "saved").await.is_some());
    drop(session);
}

#[tokio::test]
async fn alias_sender_stays_outgoing_after_message_is_moved_out_of_sent_folder() {
    let f = Fixture::new().await;
    let session = f.session().await;
    let mut value = message("message-a", "thread-a", "archive-folder", "From my alias");
    value["from"]["emailAddress"]["address"] = json!("alias@example.test");
    value["sender"]["emailAddress"]["address"] = json!("you@example.test");
    let record = projection::thread(
        session.authorization().mailbox(),
        &context(),
        "thread-a",
        vec![serde_json::from_value(value).unwrap()],
    )
    .unwrap();
    assert_eq!(
        record.conversation.messages[0].direction,
        bruhs_models::MessageDirection::Outgoing
    );
    assert_eq!(
        record.connected_mail.messages[0].envelope.from[0].address,
        "alias@example.test"
    );
    drop(session);
}

#[tokio::test]
async fn item_export_receipt_reports_mime_size_without_weakening_file_size_checks() {
    let f = Fixture::new().await;
    let session = f.session().await;
    let mut value = message(
        "message-a",
        "thread-a",
        "folder-inbox",
        "With an attached Outlook item",
    );
    value["attachments"] = json!([attachment("item-a", "itemAttachment", 1000)]);
    let record = projection::thread(
        session.authorization().mailbox(),
        &context(),
        "thread-a",
        vec![serde_json::from_value(value).unwrap()],
    )
    .unwrap();
    session
        .apply_sync(
            session.revision().await.unwrap(),
            SyncBatch {
                upserts: vec![ThreadWrite {
                    reference: session
                        .authorization()
                        .mailbox()
                        .thread("thread-a")
                        .unwrap(),
                    payload: serde_json::to_string(&record).unwrap(),
                }],
                confirmed_deleted: vec![],
                cursor: None,
            },
        )
        .await
        .unwrap();
    f.reply(
        "messages/message-a/attachments/item-a",
        &[],
        attachment("item-a", "itemAttachment", 1000),
    )
    .await;
    f.response(
        "messages/message-a/attachments/item-a/$value",
        &[],
        ResponseTemplate::new(200).set_body_bytes(b"MIME".to_vec()),
    )
    .await;
    let result = f
        .client
        .attachment(
            &session,
            "thread-a",
            &record.connected_mail.messages[0].attachments[0].id,
        )
        .await
        .unwrap();
    assert_eq!(result.bytes, b"MIME");
    assert_eq!(result.item_export_size, Some(4));
    assert_eq!(result.metadata.size_bytes, 4);
    drop(session);
}

#[tokio::test]
async fn sender_uses_the_current_leased_profile_without_reading_mail() {
    let f = Fixture::new().await;
    let session = f.session().await;
    f.reply(
        "",
        &[("$select", "id,mail")],
        json!({"id":"current-subject","mail":"verified@example.test","userPrincipalName":"fallback@example.test"}),
    )
    .await;
    let sender = f.client.sender(&session).await.unwrap();
    assert_eq!(sender.address().address, "verified@example.test");
    assert!(sender.address().name.is_empty());
    assert!(!format!("{sender:?}").contains("verified@example.test"));
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    drop(session);
}

#[tokio::test]
async fn sender_never_promotes_a_sign_in_name_to_a_missing_smtp_address() {
    for primary in [Value::Null, json!(""), json!("   ")] {
        let f = Fixture::new().await;
        let session = f.session().await;
        f.reply(
            "",
            &[("$select", "id,mail")],
            json!({"id":"current-subject","mail":primary,"userPrincipalName":"fallback@example.test"}),
        )
        .await;
        assert!(matches!(
            f.client.sender(&session).await,
            Err(OutlookError::InvalidResponse)
        ));
        drop(session);
    }
}

#[tokio::test]
async fn sender_rejects_malformed_or_missing_verified_identity() {
    for profile in [
        json!({"id":"","mail":"verified@example.test"}),
        json!({"id":"current-subject"}),
        json!({"id":"current-subject","mail":"not-an-address","userPrincipalName":"fallback@example.test"}),
        json!({"id":"current-subject","mail":"a@example.test,b@example.test"}),
        json!({"id":"current-subject","mail":"verified@example.test\r\nBcc: other@example.test"}),
        json!({"id":"current-subject","mail":null,"userPrincipalName":"guest#EXT#@example.test"}),
    ] {
        let f = Fixture::new().await;
        let session = f.session().await;
        f.reply("", &[("$select", "id,mail")], profile).await;
        assert!(matches!(
            f.client.sender(&session).await,
            Err(OutlookError::InvalidResponse)
        ));
        drop(session);
    }
}

#[tokio::test]
async fn sender_preserves_authorization_errors_and_provider_boundary() {
    let f = Fixture::new().await;
    let session = f.session().await;
    f.response("", &[], ResponseTemplate::new(401)).await;
    assert!(matches!(
        f.client.sender(&session).await,
        Err(OutlookError::SignIn)
    ));
    drop(session);
    let connections =
        Connections::test_connected(ServiceProvider::Gmail, "synthetic-gmail-token").await;
    let gmail = f
        .runtime
        .session(&connections, ServiceProvider::Gmail, "synthetic-connection")
        .await
        .unwrap();
    assert!(matches!(
        f.client.sender(&gmail).await,
        Err(OutlookError::WrongProvider)
    ));
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    drop(gmail);
}
