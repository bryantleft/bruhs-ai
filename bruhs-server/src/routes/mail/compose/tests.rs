use super::*;
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine as _,
};
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};
use std::time::Duration;
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, ResponseTemplate,
};

mod prequeue;

const TOKEN: &str = "synthetic-compose-http-token";
struct Fixture {
    _directory: tempfile::TempDir,
    runtime: bruhs_core::mail::MailRuntime,
    connections: bruhs_core::connections::Connections,
    mailbox: String,
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
    async fn new(provider: ServiceProvider) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let runtime = bruhs_core::mail::MailRuntime::open(directory.path().join("mail.sqlite3"))
            .await
            .unwrap();
        let connections =
            bruhs_core::connections::Connections::test_connected(provider, TOKEN).await;
        let session = runtime
            .session(&connections, provider, "synthetic-connection")
            .await
            .unwrap();
        let mailbox = session.authorization().mailbox().as_str().to_owned();
        drop(session); // The HTTP operation must obtain the exclusive current lease.
        let state = Arc::new(
            AppState::builder()
                .secret_store(bruhs_core::secrets::SecretStore::in_memory())
                .connections(connections.clone())
                .build(),
        );
        state.attach_mail(runtime.clone()).unwrap();
        let origin = MockServer::builder().start().await;
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
            GmailClient::for_test_origin(&origin.uri()).unwrap(),
        )))
        .layer(Extension(Arc::new(
            OutlookClient::for_test_origin(&origin.uri()).unwrap(),
        )));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = crate::desktop_server_client::ServerClient::new(&base, &base).unwrap();
        let profile = if provider == ServiceProvider::Gmail {
            "/gmail/v1/users/me/profile"
        } else {
            "/v1.0/me"
        };
        Mock::given(method("GET"))
            .and(path(profile))
            .and(header("authorization", format!("Bearer {TOKEN}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                if provider == ServiceProvider::Gmail {
                    json!({"emailAddress":"you@example.test","historyId":"10"})
                } else {
                    json!({"id":"stable-subject","mail":"you@example.test"})
                },
            ))
            .mount(&origin)
            .await;
        Self {
            _directory: directory,
            runtime,
            connections,
            mailbox,
            client,
            http: reqwest::Client::new(),
            base,
            provider: origin,
            server,
        }
    }
    fn host(&self) -> String {
        self.runtime.host().as_str().into()
    }
    fn route(&self, tail: &str) -> String {
        format!(
            "{}/api/mail/mailboxes/{}/drafts{}",
            self.base, self.mailbox, tail
        )
    }
    async fn create(&self, id: &str) -> MailComposeDraft {
        self.client
            .mail_draft_create(
                &self.mailbox,
                &MailDraftCreate {
                    host_id: self.host(),
                    client_id: id.into(),
                    reply: None,
                },
            )
            .await
            .unwrap()
    }
    async fn saved(&self, id: &str) -> MailComposeDraft {
        let draft = self.create(id).await;
        self.client
            .mail_draft_save(
                &self.mailbox,
                id,
                &MailDraftSave {
                    host_id: self.host(),
                    expected_revision: draft.revision,
                    content: MailComposeContent {
                        to: "Recipient <recipient@example.test>".into(),
                        cc: "copy@example.test".into(),
                        bcc: "hidden@example.test".into(),
                        subject: "HTTP résumé 東京".into(),
                        body: "Reviewed private body <script> 東京".into(),
                        ..Default::default()
                    },
                },
            )
            .await
            .unwrap()
    }
    async fn review(&self, id: &str, revision: i64) -> MailSendReview {
        self.client
            .mail_draft_review(
                &self.mailbox,
                id,
                &MailReviewRequest {
                    host_id: self.host(),
                    connection_id: "synthetic-connection".into(),
                    expected_revision: revision,
                },
            )
            .await
            .unwrap()
    }
    fn request(&self, review: &MailSendReview) -> MailSendRequest {
        MailSendRequest {
            host_id: self.host(),
            connection_id: review.connection_id.clone(),
            operation_id: "http-send-operation".into(),
            revision: review.revision,
            digest: review.digest.clone(),
        }
    }
    async fn posts(&self) -> Vec<wiremock::Request> {
        self.provider
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|request| request.method == "POST")
            .collect()
    }
}

#[tokio::test]
async fn native_http_drafts_keep_incomplete_text_after_disconnect_and_reject_stale_writes() {
    let f = Fixture::new(ServiceProvider::Gmail).await;
    let draft = f.create("editable").await;
    f.connections
        .disconnect(ServiceProvider::Gmail)
        .await
        .unwrap();
    let content = MailComposeContent {
        to: "Unfinished <".into(),
        subject: "unsent".into(),
        body: "private durable body".into(),
        ..Default::default()
    };
    let saved = f
        .client
        .mail_draft_save(
            &f.mailbox,
            "editable",
            &MailDraftSave {
                host_id: f.host(),
                expected_revision: draft.revision,
                content: content.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(saved.content, content);
    assert!(f
        .client
        .mail_draft_save(
            &f.mailbox,
            "editable",
            &MailDraftSave {
                host_id: f.host(),
                expected_revision: draft.revision,
                content: MailComposeContent::default()
            }
        )
        .await
        .is_err());
    assert_eq!(
        f.client
            .mail_draft(&f.host(), &f.mailbox, "editable")
            .await
            .unwrap()
            .content,
        content
    );
    let page = f
        .client
        .mail_drafts(&f.host(), &f.mailbox, 0, None)
        .await
        .unwrap();
    assert_eq!(page.total_matches, 1);
    assert_eq!(page.rows[0].subject, "unsent");
    let thin = serde_json::to_string(&page).unwrap();
    assert!(!thin.contains("private durable body"));
    assert!(!thin.contains(TOKEN));
    assert!(f
        .client
        .mail_draft_review(
            &f.mailbox,
            "editable",
            &MailReviewRequest {
                host_id: f.host(),
                connection_id: "synthetic-connection".into(),
                expected_revision: saved.revision
            }
        )
        .await
        .is_err());
    assert!(f.provider.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn native_http_upload_review_and_gmail_send_use_one_frozen_message() {
    let f = Fixture::new(ServiceProvider::Gmail).await;
    let mut draft = f.saved("frozen").await;
    let bytes = b"private\0file\xff".to_vec();
    let upload = f
        .client
        .mail_draft_upload(
            &f.host(),
            &f.mailbox,
            "frozen",
            "upload-a",
            "résumé-東京.bin",
            bytes.clone(),
        )
        .await
        .unwrap();
    assert_eq!(upload.sha256, format!("{:x}", Sha256::digest(&bytes)));
    assert_eq!(
        f.client
            .mail_draft_upload(
                &f.host(),
                &f.mailbox,
                "frozen",
                "upload-a",
                "résumé-東京.bin",
                bytes
            )
            .await
            .unwrap()
            .id,
        upload.id
    );
    assert!(f
        .client
        .mail_draft_upload(
            &f.host(),
            &f.mailbox,
            "frozen",
            "upload-a",
            "changed.bin",
            vec![1]
        )
        .await
        .is_err());
    draft.content.attachments.push(upload.id.clone());
    let saved = f
        .client
        .mail_draft_save(
            &f.mailbox,
            "frozen",
            &MailDraftSave {
                host_id: f.host(),
                expected_revision: draft.revision,
                content: draft.content,
            },
        )
        .await
        .unwrap();
    let review = f.review("frozen", saved.revision).await;
    assert_eq!(review.from.address, "you@example.test");
    assert_eq!(review.recipients.bcc[0].address, "hidden@example.test");
    assert_eq!(review.files[0].id, upload.id);
    Mock::given(method("POST"))
        .and(path("/gmail/v1/users/me/messages/send"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id":"accepted-id","threadId":"accepted-thread"})),
        )
        .expect(1)
        .mount(&f.provider)
        .await;
    let request = f.request(&review);
    let status = f
        .client
        .mail_draft_send(&f.mailbox, "frozen", &request)
        .await
        .unwrap();
    assert_eq!(status.state, MailDispatchState::Accepted);
    assert_eq!(
        f.client
            .mail_draft_send(&f.mailbox, "frozen", &request)
            .await
            .unwrap()
            .state,
        MailDispatchState::Accepted
    );
    assert_eq!(
        f.client
            .mail_send_status(&f.host(), &f.mailbox, &request.operation_id)
            .await
            .unwrap()
            .provider_message_id
            .as_deref(),
        Some("accepted-id")
    );
    let posts = f.posts().await;
    assert_eq!(posts.len(), 1);
    let body: Value = serde_json::from_slice(&posts[0].body).unwrap();
    let raw = URL_SAFE_NO_PAD
        .decode(body["raw"].as_str().unwrap())
        .unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&raw)), review.mime_sha256);
    assert_eq!(
        f.client
            .mail_drafts(&f.host(), &f.mailbox, 0, None)
            .await
            .unwrap()
            .total_matches,
        0
    );
}

#[tokio::test]
async fn actual_router_outlook_mime_202_has_optional_ids_and_no_replay() {
    let f = Fixture::new(ServiceProvider::Outlook).await;
    let saved = f.saved("outlook").await;
    let review = f.review("outlook", saved.revision).await;
    Mock::given(method("POST"))
        .and(path("/v1.0/me/sendMail"))
        .and(header("content-type", "text/plain"))
        .respond_with(ResponseTemplate::new(202))
        .expect(1)
        .mount(&f.provider)
        .await;
    let request = f.request(&review);
    let status = f
        .client
        .mail_draft_send(&f.mailbox, "outlook", &request)
        .await
        .unwrap();
    assert_eq!(status.state, MailDispatchState::Accepted);
    assert!(status.provider_message_id.is_none());
    assert!(status.provider_thread_id.is_none());
    assert_eq!(
        f.client
            .mail_draft_send(&f.mailbox, "outlook", &request)
            .await
            .unwrap()
            .state,
        MailDispatchState::Accepted
    );
    let posts = f.posts().await;
    assert_eq!(posts.len(), 1);
    let raw = STANDARD.decode(&posts[0].body).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&raw)), review.mime_sha256);
}

#[tokio::test]
async fn actual_router_rejects_injected_fields_wrong_host_and_stale_review_before_provider_io() {
    let f = Fixture::new(ServiceProvider::Gmail).await;
    let status=f.http.post(f.route("")).json(&json!({"host_id":f.host(),"client_id":"injected","reply":null,"from":"attacker@example.test"})).send().await.unwrap().status();
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let status = f
        .http
        .post(f.route(""))
        .json(&json!({"host_id":"other-host","client_id":"foreign","reply":null}))
        .send()
        .await
        .unwrap()
        .status();
    assert!(!status.is_success());
    let saved = f.saved("stale").await;
    let wrong=f.http.post(f.route("/stale/review")).json(&json!({"host_id":"foreign","connection_id":"synthetic-connection","expected_revision":saved.revision})).send().await.unwrap();
    assert!(!wrong.status().is_success());
    assert!(f.provider.received_requests().await.unwrap().is_empty());
    let mut content = saved.content.clone();
    content.body = "newer body".into();
    let newer = f
        .client
        .mail_draft_save(
            &f.mailbox,
            "stale",
            &MailDraftSave {
                host_id: f.host(),
                expected_revision: saved.revision,
                content,
            },
        )
        .await
        .unwrap();
    assert!(f
        .client
        .mail_draft_review(
            &f.mailbox,
            "stale",
            &MailReviewRequest {
                host_id: f.host(),
                connection_id: "synthetic-connection".into(),
                expected_revision: saved.revision
            }
        )
        .await
        .is_err());
    assert_eq!(
        f.client
            .mail_draft(&f.host(), &f.mailbox, "stale")
            .await
            .unwrap()
            .revision,
        newer.revision
    );
    assert!(f.posts().await.is_empty());
}

#[tokio::test]
async fn aborting_http_waiter_retains_provider_ack_and_status_without_a_second_post() {
    let f = Fixture::new(ServiceProvider::Gmail).await;
    let saved = f.saved("abandoned").await;
    let review = f.review("abandoned", saved.revision).await;
    let request = f.request(&review);
    Mock::given(method("POST"))
        .and(path("/gmail/v1/users/me/messages/send"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(200))
                .set_body_json(json!({"id":"accepted-after-close"})),
        )
        .expect(1)
        .mount(&f.provider)
        .await;
    let client = f.client.clone();
    let mailbox = f.mailbox.clone();
    let send_request = request.clone();
    let caller = tokio::spawn(async move {
        client
            .mail_draft_send(&mailbox, "abandoned", &send_request)
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while f.posts().await.is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    caller.abort();
    let _ = caller.await;
    let status = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let status = f
                .client
                .mail_send_status(&f.host(), &f.mailbox, &request.operation_id)
                .await
                .unwrap();
            if status.state == MailDispatchState::Accepted {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        status.provider_message_id.as_deref(),
        Some("accepted-after-close")
    );
    assert_eq!(
        f.client
            .mail_draft_send(&f.mailbox, "abandoned", &request)
            .await
            .unwrap()
            .state,
        MailDispatchState::Accepted
    );
    assert_eq!(f.posts().await.len(), 1);
}
