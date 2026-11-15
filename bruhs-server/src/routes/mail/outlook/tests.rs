use super::*;
use bruhs_core::{connections::Connections, mail::MailRuntime};
use bruhs_models::{mail::*, InboxFilter};
use serde_json::{json, Value};
use wiremock::{
    matchers::{header, method, path, query_param},
    Mock, MockServer, ResponseTemplate,
};

const API: &str = "/v1.0/me";
const TOKEN: &str = "synthetic-outlook-http-token";

struct Fixture {
    directory: tempfile::TempDir,
    runtime: MailRuntime,
    native: crate::desktop_server_client::ServerClient,
    http: reqwest::Client,
    base: String,
    provider: MockServer,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let runtime = MailRuntime::open(directory.path().join("mail.sqlite3"))
            .await
            .unwrap();
        let connections = Connections::test_connected(ServiceProvider::Outlook, TOKEN).await;
        let app = Arc::new(
            AppState::builder()
                .secret_store(bruhs_core::secrets::SecretStore::in_memory())
                .connections(connections)
                .build(),
        );
        app.attach_mail(runtime.clone()).unwrap();
        let provider = MockServer::builder().start().await;
        let config = crate::config::Config::from_settings(crate::config::Settings {
            port: Some("0"),
            web_dist: directory.path().join("missing").to_str(),
            ..Default::default()
        })
        .unwrap();
        let router = crate::router(crate::config::ServerState {
            app,
            config: Arc::new(config),
            updates: bruhs_core::updates::UpdateChecker::new(bruhs_core::updates::BuildInfo::new(
                "", "", "unused",
            )),
        })
        .layer(Extension(Arc::new(
            OutlookClient::for_test_origin(&provider.uri()).unwrap(),
        )));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let native = crate::desktop_server_client::ServerClient::new(&base, &base).unwrap();
        Self {
            directory,
            runtime,
            native,
            http: reqwest::Client::new(),
            base,
            provider,
            server,
        }
    }
    fn request(&self) -> SyncRequest {
        SyncRequest {
            host_id: self.runtime.host().as_str().into(),
            connection_id: "synthetic-connection".into(),
            mailbox_id: None,
            restart: false,
        }
    }
    fn link(&self, resource: &str, query: &str) -> String {
        format!("{}{API}/{resource}?{query}", self.provider.uri())
    }
    async fn reply(&self, resource: &str, query: &[(&str, &str)], value: Value) {
        let resource = if resource.is_empty() {
            API.into()
        } else {
            format!("{API}/{resource}")
        };
        let mut mock = Mock::given(method("GET"))
            .and(path(resource))
            .and(header("authorization", format!("Bearer {TOKEN}")));
        for (key, value) in query {
            mock = mock.and(query_param(*key, *value));
        }
        mock.respond_with(ResponseTemplate::new(200).set_body_json(value))
            .mount(&self.provider)
            .await;
    }
    async fn read_fixture(&self) {
        self.reply("",&[],json!({"id":"subject","mail":"you@example.test","userPrincipalName":"you@example.test"})).await;
        self.reply("mailFolders/inbox", &[], json!({"id":"folder-inbox"}))
            .await;
        self.reply("mailFolders/sentitems", &[], json!({"id":"folder-sent"}))
            .await;
        self.reply(
            "mailFolders",
            &[],
            json!({"value":[{"id":"folder-inbox","childFolderCount":0}]}),
        )
        .await;
        let resource = "mailFolders/folder-inbox/messages/delta";
        self.reply(
            resource,
            &[("$select", "id,conversationId")],
            json!({"value":[],"@odata.deltaLink":self.link(resource,"$deltatoken=anchor")}),
        )
        .await;
        self.reply(
            resource,
            &[("$deltatoken", "anchor")],
            json!({"value":[],"@odata.deltaLink":self.link(resource,"$deltatoken=done")}),
        )
        .await;
        self.reply(
            "messages",
            &[("$select", "id,conversationId")],
            json!({"value":[{"id":"message-a","conversationId":"thread-a"}]}),
        )
        .await;
        self.reply("messages",&[("$filter","conversationId eq 'thread-a'")],json!({"value":[{
            "id":"message-a","conversationId":"thread-a","parentFolderId":"folder-inbox","subject":"Outlook HTTP Café","body":{"contentType":"text","content":"PRIVATE-CONTROLLED-BODY full-history-outlook-needle"},
            "from":{"emailAddress":{"name":"Zoë","address":"zoe@example.test"}},"sender":{"emailAddress":{"name":"Zoë","address":"zoe@example.test"}},"toRecipients":[{"emailAddress":{"name":"You","address":"you@example.test"}}],"ccRecipients":[],"bccRecipients":[],"replyTo":[],"internetMessageId":"<message-a@example.test>","internetMessageHeaders":[],"receivedDateTime":"2026-11-14T00:00:00Z","sentDateTime":null,"createdDateTime":"2026-11-14T00:00:00Z","isRead":false,"isDraft":false,"importance":"normal","categories":[],
            "attachments":[{"@odata.type":"#microsoft.graph.fileAttachment","id":"file-a","name":"../設計.txt","contentType":"text/plain","size":4,"isInline":false},{"@odata.type":"#microsoft.graph.itemAttachment","id":"item-a","name":"attached-message.eml","contentType":null,"size":1000,"isInline":false}]
        }]})).await;
    }
    async fn sync(&self) -> MailSyncResult {
        let mut request = self.request();
        let mut last = None;
        for index in 0..4 {
            let page = self.native.sync_outlook(&request).await.unwrap();
            assert_eq!(page.caught_up, index == 3);
            request.mailbox_id = Some(page.mailbox_id.clone());
            last = Some(page);
        }
        last.unwrap()
    }
}

#[tokio::test]
async fn outlook_native_http_sync_search_reader_and_attachment_use_actual_router_and_private_cache()
{
    let f = Fixture::new().await;
    f.read_fixture().await;
    let result = f.sync().await;
    let snapshot = f.native.mailboxes().await.unwrap();
    assert_eq!(snapshot.mailboxes.len(), 1);
    assert_eq!(snapshot.mailboxes[0].provider, ServiceProvider::Outlook);
    let page = f
        .native
        .browse_mailbox(
            &result.mailbox_id,
            &MailBrowseQuery {
                host_id: snapshot.host_id.clone(),
                query: "full-history-outlook-needle".into(),
                filter: InboxFilter::All,
                page: 0,
                expected_revision: Some(result.revision),
            },
        )
        .await
        .unwrap();
    assert_eq!(page.total_matches, 1);
    assert_eq!(page.rows[0].card.sender, "Zoë");
    let record = f
        .native
        .mail_thread(&snapshot.host_id, &result.mailbox_id, "thread-a")
        .await
        .unwrap();
    assert!(record.conversation.messages[0]
        .body
        .contains("full-history-outlook-needle"));
    assert_eq!(record.connected_mail.provider, ServiceProvider::Outlook);
    f.reply("messages/message-a/attachments/file-a",&[],json!({"@odata.type":"#microsoft.graph.fileAttachment","id":"file-a","name":"../設計.txt","contentType":"text/plain","size":4,"isInline":false})).await;
    Mock::given(method("GET"))
        .and(path(format!(
            "{API}/messages/message-a/attachments/file-a/$value"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"\0abc".to_vec()))
        .mount(&f.provider)
        .await;
    let destination = f.directory.path().join("chosen.txt");
    std::fs::write(&destination, b"old file").unwrap();
    f.native
        .save_provider_mail_attachment(
            ServiceProvider::Outlook,
            &snapshot.host_id,
            &result.mailbox_id,
            "thread-a",
            &record.connected_mail.messages[0].attachments[0],
            &destination,
        )
        .await
        .unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), b"\0abc");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            std::fs::metadata(&destination)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let response = f
        .http
        .get(format!("{}/api/mail", f.base))
        .send()
        .await
        .unwrap();
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    let json = serde_json::to_string(&snapshot).unwrap();
    assert!(!json.contains(TOKEN));
    assert!(!json.contains("stable-subject"));
}

#[tokio::test]
async fn outlook_origin_host_connection_and_mailbox_fences_precede_provider_reads() {
    let f = Fixture::new().await;
    let endpoint = format!("{}/api/mail/outlook/sync", f.base);
    let mut request = f.request();
    assert_eq!(
        f.http
            .post(&endpoint)
            .header("origin", "https://untrusted.invalid")
            .json(&request)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    request.host_id = "another-host".into();
    assert_eq!(
        f.http
            .post(&endpoint)
            .json(&request)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    request = f.request();
    request.connection_id = "old-connection".into();
    assert_eq!(
        f.http
            .post(&endpoint)
            .json(&request)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    request = f.request();
    request.mailbox_id = Some("foreign-mailbox".into());
    assert_eq!(
        f.http
            .post(&endpoint)
            .json(&request)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert!(f.provider.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn outlook_incomplete_download_cannot_replace_the_selected_file() {
    let f = Fixture::new().await;
    f.read_fixture().await;
    let synced = f.sync().await;
    let host = f.runtime.host().as_str();
    let record = f
        .native
        .mail_thread(host, &synced.mailbox_id, "thread-a")
        .await
        .unwrap();
    let attachment = &record.connected_mail.messages[0].attachments[0];
    f.reply("messages/message-a/attachments/file-a",&[],json!({"@odata.type":"#microsoft.graph.fileAttachment","id":"file-a","name":"../設計.txt","contentType":"text/plain","size":4,"isInline":false})).await;
    Mock::given(method("GET"))
        .and(path(format!(
            "{API}/messages/message-a/attachments/file-a/$value"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"bad".to_vec()))
        .mount(&f.provider)
        .await;
    let destination = f.directory.path().join("chosen.txt");
    std::fs::write(&destination, b"preserve original").unwrap();
    assert!(f
        .native
        .save_provider_mail_attachment(
            ServiceProvider::Outlook,
            host,
            &synced.mailbox_id,
            "thread-a",
            attachment,
            &destination
        )
        .await
        .is_err());
    assert_eq!(std::fs::read(&destination).unwrap(), b"preserve original");
}

#[tokio::test]
async fn outlook_throttling_keeps_retry_header_and_redacts_provider_response() {
    let f = Fixture::new().await;
    f.read_fixture().await;
    Mock::given(method("GET"))
        .and(path(format!("{API}/mailFolders")))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "23")
                .set_body_string("PRIVATE-TOKEN-CANARY"),
        )
        .with_priority(1)
        .mount(&f.provider)
        .await;
    let response = f
        .http
        .post(format!("{}/api/mail/outlook/sync", f.base))
        .json(&f.request())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers().get("retry-after").unwrap(), "23");
    let text = response.text().await.unwrap();
    assert!(!text.contains("PRIVATE-TOKEN-CANARY"));
    assert!(!text.contains(TOKEN));
}

#[tokio::test]
async fn outlook_item_exports_use_explicit_bounded_size_receipt_through_native_save() {
    let f = Fixture::new().await;
    f.read_fixture().await;
    let synced = f.sync().await;
    let host = f.runtime.host().as_str();
    let record = f
        .native
        .mail_thread(host, &synced.mailbox_id, "thread-a")
        .await
        .unwrap();
    let attachment = &record.connected_mail.messages[0].attachments[1];
    f.reply("messages/message-a/attachments/item-a",&[],json!({"@odata.type":"#microsoft.graph.itemAttachment","id":"item-a","name":"attached-message.eml","contentType":null,"size":1000,"isInline":false})).await;
    Mock::given(method("GET"))
        .and(path(format!(
            "{API}/messages/message-a/attachments/item-a/$value"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"MIME".to_vec()))
        .mount(&f.provider)
        .await;
    let destination = f.directory.path().join("attached-message.eml");
    f.native
        .save_provider_mail_attachment(
            ServiceProvider::Outlook,
            host,
            &synced.mailbox_id,
            "thread-a",
            attachment,
            &destination,
        )
        .await
        .unwrap();
    assert_eq!(std::fs::read(destination).unwrap(), b"MIME");
    let response = f
        .http
        .get(format!(
            "{}/api/mail/outlook/mailboxes/{}/threads/thread-a/attachments/{}",
            f.base, synced.mailbox_id, attachment.id
        ))
        .query(&[("host_id", host), ("connection_id", "synthetic-connection")])
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.headers().get("x-bruhs-mail-export-size").unwrap(),
        "4"
    );
    let exposed = response
        .headers()
        .get("access-control-expose-headers")
        .unwrap()
        .to_str()
        .unwrap();
    for name in [
        "x-bruhs-transcript-source",
        "x-bruhs-transcript-refresh",
        "x-bruhs-mail-export-size",
    ] {
        assert!(exposed.split(',').any(|value| value.trim() == name));
    }
    assert_eq!(response.bytes().await.unwrap().as_ref(), b"MIME");
}
