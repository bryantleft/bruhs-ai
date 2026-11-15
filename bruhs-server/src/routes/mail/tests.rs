use super::*;
mod attachments;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde_json::{json, Value};
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, ResponseTemplate,
};

const TOKEN: &str = "synthetic-mail-api-token";

struct Fixture {
    _directory: tempfile::TempDir,
    runtime: MailRuntime,
    client: crate::desktop_server_client::ServerClient,
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
        let connections =
            bruhs_core::connections::Connections::test_connected(ServiceProvider::Gmail, TOKEN)
                .await;
        let state = Arc::new(
            AppState::builder()
                .secret_store(bruhs_core::secrets::SecretStore::in_memory())
                .connections(connections)
                .build(),
        );
        state.attach_mail(runtime.clone()).unwrap();
        let provider = MockServer::builder().start().await;
        let config = crate::config::Config::from_settings(crate::config::Settings {
            port: Some("0"),
            web_dist: directory.path().join("missing").to_str(),
            ..Default::default()
        })
        .unwrap();
        let app = crate::router(crate::config::ServerState {
            app: state,
            config: Arc::new(config),
            updates: bruhs_core::updates::UpdateChecker::new(bruhs_core::updates::BuildInfo::new(
                "", "", "unused",
            )),
        })
        .layer(Extension(Arc::new(
            GmailClient::for_test_origin(&provider.uri()).unwrap(),
        )));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = crate::desktop_server_client::ServerClient::new(&base, &base).unwrap();
        Self {
            _directory: directory,
            runtime,
            client,
            http: reqwest::Client::new(),
            base,
            provider,
            server,
        }
    }

    fn request(&self) -> GmailSyncRequest {
        GmailSyncRequest {
            host_id: self.runtime.host().as_str().into(),
            connection_id: "synthetic-connection".into(),
            mailbox_id: None,
            restart: false,
        }
    }
}

async fn reply(fixture: &Fixture, route: &str, value: Value) {
    Mock::given(method("GET"))
        .and(path(format!("/gmail/v1/users/me/{route}")))
        .and(header("authorization", format!("Bearer {TOKEN}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(value))
        .mount(&fixture.provider)
        .await;
}

#[tokio::test]
async fn native_http_mail_sync_search_and_full_thread_use_the_real_router_cache_and_provider_transport(
) {
    let fixture = Fixture::new().await;
    assert!(fixture
        .client
        .mailboxes()
        .await
        .unwrap()
        .mailboxes
        .is_empty());
    let body = format!("{} full-history-needle", "PRIVATE-FULL-BODY ".repeat(3000));
    reply(
        &fixture,
        "profile",
        json!({"emailAddress":"you@example.test","historyId":"100"}),
    )
    .await;
    reply(&fixture, "threads", json!({"threads":[{"id":"thread-a"}]})).await;
    reply(&fixture,"threads/thread-a",json!({"id":"thread-a","messages":[{"id":"message-a","threadId":"thread-a","internalDate":"1794652800000","labelIds":["INBOX","UNREAD"],"payload":{"mimeType":"text/plain","headers":[{"name":"From","value":"Zoë <zoe@example.test>"},{"name":"To","value":"you@example.test"},{"name":"Subject","value":"HTTP mail fixture"}],"body":{"size":body.len(),"data":URL_SAFE_NO_PAD.encode(&body)}}}]})).await;
    let synced = fixture.client.sync_gmail(&fixture.request()).await.unwrap();
    assert!(synced.full_scan_complete);
    assert!(!synced.caught_up);
    let snapshot = fixture.client.mailboxes().await.unwrap();
    assert_eq!(snapshot.mailboxes.len(), 1);
    assert_eq!(snapshot.mailboxes[0].threads, 1);
    assert_eq!(snapshot.mailboxes[0].account_label, "you@example.test");
    let query = MailBrowseQuery {
        host_id: snapshot.host_id.clone(),
        query: "full-history-needle".into(),
        filter: bruhs_models::InboxFilter::All,
        page: 0,
        expected_revision: Some(synced.revision),
    };
    let page = fixture
        .client
        .browse_mailbox(&synced.mailbox_id, &query)
        .await
        .unwrap();
    assert_eq!(page.total_matches, 1);
    assert_eq!(page.rows[0].card.sender, "Zoë");
    assert!(serde_json::to_string(&page).unwrap().len() < 4000);
    let record = fixture
        .client
        .mail_thread(&snapshot.host_id, &synced.mailbox_id, "thread-a")
        .await
        .unwrap();
    assert_eq!(record.conversation.messages[0].body, body);
    assert_eq!(
        record.connected_mail.messages[0].envelope.to[0].address,
        "you@example.test"
    );
    let json = serde_json::to_string(&snapshot).unwrap();
    assert!(!json.contains(TOKEN));
    assert!(!json.contains("stable-subject"));
    let response = fixture
        .http
        .get(format!("{}/api/mail", fixture.base))
        .send()
        .await
        .unwrap();
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    reply(&fixture, "history", json!({"historyId":"110"})).await;
    let mut request = fixture.request();
    request.mailbox_id = Some(synced.mailbox_id);
    assert!(fixture.client.sync_gmail(&request).await.unwrap().caught_up);
}

#[tokio::test]
async fn origin_host_generation_and_account_guards_run_before_provider_reads() {
    let fixture = Fixture::new().await;
    let request = fixture.request();
    let response = fixture
        .http
        .post(format!("{}/api/mail/gmail/sync", fixture.base))
        .header("origin", "https://untrusted.invalid")
        .json(&request)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let mut wrong_host = request.clone();
    wrong_host.host_id = "another-host".into();
    let response = fixture
        .http
        .post(format!("{}/api/mail/gmail/sync", fixture.base))
        .json(&wrong_host)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let mut wrong_generation = request.clone();
    wrong_generation.connection_id = "stale-connection".into();
    let response = fixture
        .http
        .post(format!("{}/api/mail/gmail/sync", fixture.base))
        .json(&wrong_generation)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let mut wrong_account = request;
    wrong_account.mailbox_id = Some("cached-mailbox-from-another-account".into());
    let response = fixture
        .http
        .post(format!("{}/api/mail/gmail/sync", fixture.base))
        .json(&wrong_account)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(fixture
        .provider
        .received_requests()
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn provider_quota_is_a_redacted_http_429_with_its_retry_hint() {
    let fixture = Fixture::new().await;
    Mock::given(method("GET")).and(path("/gmail/v1/users/me/profile")).respond_with(ResponseTemplate::new(403).insert_header("Retry-After","120").set_body_json(json!({"error":{"errors":[{"reason":"userRateLimitExceeded","message":"PRIVATE-PROVIDER-DETAIL"}]}}))).mount(&fixture.provider).await;
    let response = fixture
        .http
        .post(format!("{}/api/mail/gmail/sync", fixture.base))
        .json(&fixture.request())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers().get("retry-after").unwrap(), "120");
    let body = response.text().await.unwrap();
    assert!(!body.contains(TOKEN));
    assert!(!body.contains("PRIVATE-PROVIDER-DETAIL"));
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["retry_after_seconds"],
        120
    );
    assert_eq!(
        fixture.client.mailboxes().await.unwrap().mailboxes[0].threads,
        0
    );
}
