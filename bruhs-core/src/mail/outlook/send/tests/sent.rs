use super::*;
use serde_json::Value;

const FOLDER: &str = "sent-folder";
const SENT_ID: &str = "sent-a";
const TIMEOUT: Duration = Duration::from_secs(5);

async fn uncertain() -> (Fixture, MailSendRequest, Vec<u8>, String) {
    let fixture = Fixture::new().await;
    fixture.respond(ResponseTemplate::new(500)).await;
    let (session, _, request) = fixture
        .reviewed(
            Some(vec![0, 0xff, 13, 10, 42]),
            fixture.client.mime_limit_bytes(),
        )
        .await;
    let result = fixture
        .runtime
        .send_reviewed_compose(
            session,
            Arc::new(fixture.client.clone()),
            CLIENT.into(),
            request.clone(),
        )
        .wait()
        .await;
    assert!(matches!(
        result,
        Err(ComposeError::Provider(MailSendProblem {
            kind: MailSendProblemKind::Unconfirmed,
            ..
        }))
    ));
    let posts = fixture.posts().await;
    assert_eq!(posts.len(), 1);
    let raw = STANDARD.decode(&posts[0].body).unwrap();
    let message_id = mail_parser::MessageParser::default()
        .parse(&raw)
        .unwrap()
        .message_id()
        .unwrap()
        .to_owned();
    assert_eq!(
        fixture.status(&request).await.unwrap().state,
        MailDispatchState::Uncertain
    );
    (fixture, request, raw, message_id)
}

fn metadata(message_id: &str) -> Value {
    json!({
        "id":SENT_ID,
        "conversationId":"conversation-a",
        "parentFolderId":FOLDER,
        "internetMessageId":format!("<{message_id}>"),
        "isDraft":false,
    })
}

async fn folder(fixture: &Fixture, token: &str) {
    Mock::given(method("GET"))
        .and(path("/v1.0/me/mailFolders/sentitems"))
        .and(query_param("$select", "id"))
        .and(header("authorization", format!("Bearer {token}")))
        .and(headers(
            "prefer",
            crate::mail::outlook::PREFER
                .split(',')
                .map(str::trim)
                .collect(),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":FOLDER})))
        .expect(1)
        .mount(&fixture.server)
        .await;
}

async fn candidates(fixture: &Fixture, token: &str, message_id: &str, response: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path(format!("/v1.0/me/mailFolders/{FOLDER}/messages")))
        .and(query_param(
            "$filter",
            format!("internetMessageId eq '<{message_id}>'"),
        ))
        .and(query_param("$top", "2"))
        .and(query_param(
            "$select",
            "id,conversationId,parentFolderId,internetMessageId,isDraft",
        ))
        .and(header("authorization", format!("Bearer {token}")))
        .respond_with(response)
        .expect(1)
        .mount(&fixture.server)
        .await;
}

async fn copy(fixture: &Fixture, token: &str, response: ResponseTemplate, expected: u64) {
    Mock::given(method("GET"))
        .and(path(format!(
            "/v1.0/me/mailFolders/{FOLDER}/messages/{SENT_ID}/$value"
        )))
        .and(header("authorization", format!("Bearer {token}")))
        .and(header("accept", "message/rfc822"))
        .respond_with(response)
        .expect(expected)
        .mount(&fixture.server)
        .await;
}

async fn reconcile(
    fixture: &Fixture,
    request: &MailSendRequest,
    client: &OutlookClient,
) -> Result<MailSendStatus, ComposeError> {
    let session = tokio::time::timeout(TIMEOUT, fixture.session())
        .await
        .unwrap();
    let result = tokio::time::timeout(
        TIMEOUT,
        session.reconcile_uncertain_send(
            request.host_id.clone(),
            request.operation_id.clone(),
            client,
        ),
    )
    .await
    .unwrap();
    drop(session);
    result
}

#[tokio::test]
async fn unique_sent_multipart_copy_confirms_once_without_another_send() {
    let (fixture, request, raw, id) = uncertain().await;
    folder(&fixture, TOKEN).await;
    candidates(
        &fixture,
        TOKEN,
        &id,
        ResponseTemplate::new(200).set_body_json(json!({"value":[metadata(&id)]})),
    )
    .await;
    let mut transported = b"Received: by outlook.fixture.example.test\r\n".to_vec();
    transported.extend_from_slice(&raw);
    copy(
        &fixture,
        TOKEN,
        ResponseTemplate::new(200).set_body_bytes(transported),
        1,
    )
    .await;
    let status = reconcile(&fixture, &request, &fixture.client)
        .await
        .unwrap();
    assert_eq!(status.state, MailDispatchState::Accepted);
    assert_eq!(status.provider_message_id.as_deref(), Some(SENT_ID));
    assert_eq!(status.provider_thread_id.as_deref(), Some("conversation-a"));
    let before = fixture.server.received_requests().await.unwrap().len();
    assert_eq!(
        reconcile(&fixture, &request, &fixture.client)
            .await
            .unwrap()
            .state,
        MailDispatchState::Accepted
    );
    assert_eq!(
        fixture.server.received_requests().await.unwrap().len(),
        before
    );
    let repeated = fixture
        .runtime
        .send_reviewed_compose(
            tokio::time::timeout(TIMEOUT, fixture.session())
                .await
                .unwrap(),
            Arc::new(fixture.client.clone()),
            CLIENT.into(),
            request,
        )
        .wait()
        .await
        .unwrap();
    assert_eq!(repeated.state, MailDispatchState::Accepted);
    assert_eq!(fixture.posts().await.len(), 1);
}

#[tokio::test]
async fn missing_ambiguous_and_partial_searches_never_fetch_mime_or_follow_links() {
    let tripwire = MockServer::start().await;
    for kind in [
        "missing",
        "ambiguous",
        "continuation",
        "delta",
        "http-partial",
        "redirect",
    ] {
        let (fixture, request, raw, id) = uncertain().await;
        folder(&fixture, TOKEN).await;
        let result = match kind {
            "missing" => json!({"value":[]}),
            "ambiguous" => json!({"value":[metadata(&id),metadata(&id)]}),
            "continuation" => {
                json!({"value":[metadata(&id)],"@odata.nextLink":format!("{}/credential-tripwire",tripwire.uri())})
            }
            "delta" => json!({"value":[metadata(&id)],"@odata.deltaLink":"unexpected-delta"}),
            "http-partial" | "redirect" => json!({"value":[metadata(&id)]}),
            _ => unreachable!(),
        };
        let response = match kind {
            "http-partial" => ResponseTemplate::new(206),
            "redirect" => ResponseTemplate::new(302).insert_header(
                "location",
                format!("{}/credential-tripwire", tripwire.uri()),
            ),
            _ => ResponseTemplate::new(200),
        };
        candidates(&fixture, TOKEN, &id, response.set_body_json(result)).await;
        copy(
            &fixture,
            TOKEN,
            ResponseTemplate::new(200).set_body_bytes(raw),
            0,
        )
        .await;
        assert_eq!(
            reconcile(&fixture, &request, &fixture.client)
                .await
                .unwrap()
                .state,
            MailDispatchState::Uncertain,
            "{kind}"
        );
        assert_eq!(fixture.posts().await.len(), 1);
    }
    assert!(tripwire.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn candidate_identity_folder_and_non_draft_metadata_are_required_before_mime() {
    for kind in [
        "folder",
        "draft",
        "missing-draft",
        "echo",
        "resource",
        "conversation",
    ] {
        let (fixture, request, raw, id) = uncertain().await;
        folder(&fixture, TOKEN).await;
        let mut candidate = metadata(&id);
        match kind {
            "folder" => candidate["parentFolderId"] = json!("foreign-folder"),
            "draft" => candidate["isDraft"] = json!(true),
            "missing-draft" => {
                candidate.as_object_mut().unwrap().remove("isDraft");
            }
            "echo" => candidate["internetMessageId"] = json!("<other@example.test>"),
            "resource" => candidate["id"] = json!(".."),
            "conversation" => candidate["conversationId"] = json!(""),
            _ => unreachable!(),
        }
        candidates(
            &fixture,
            TOKEN,
            &id,
            ResponseTemplate::new(200).set_body_json(json!({"value":[candidate]})),
        )
        .await;
        copy(
            &fixture,
            TOKEN,
            ResponseTemplate::new(200).set_body_bytes(raw),
            0,
        )
        .await;
        assert_eq!(
            reconcile(&fixture, &request, &fixture.client)
                .await
                .unwrap()
                .state,
            MailDispatchState::Uncertain,
            "{kind}"
        );
        assert_eq!(fixture.posts().await.len(), 1);
    }
}

#[tokio::test]
async fn changed_mime_or_an_incomplete_copy_cannot_confirm_an_uncertain_send() {
    for kind in [
        "body",
        "bcc",
        "message-id",
        "unsupported-header",
        "missing",
        "partial",
    ] {
        let (fixture, request, raw, id) = uncertain().await;
        folder(&fixture, TOKEN).await;
        candidates(
            &fixture,
            TOKEN,
            &id,
            ResponseTemplate::new(200).set_body_json(json!({"value":[metadata(&id)]})),
        )
        .await;
        let response = match kind {
            "body" => {
                let mut bytes = raw;
                bytes.push(b'x');
                ResponseTemplate::new(200).set_body_bytes(bytes)
            }
            "bcc" => ResponseTemplate::new(200).set_body_string(
                String::from_utf8(raw)
                    .unwrap()
                    .replace("hidden@example.test", "different@example.test"),
            ),
            "message-id" => ResponseTemplate::new(200).set_body_string(
                String::from_utf8(raw)
                    .unwrap()
                    .replace(&id, "different@example.test"),
            ),
            "unsupported-header" => {
                let mut bytes = b"X-MS-Exchange-Organization-AuthAs: Internal\r\n".to_vec();
                bytes.extend_from_slice(&raw);
                ResponseTemplate::new(200).set_body_bytes(bytes)
            }
            "missing" => ResponseTemplate::new(404),
            "partial" => ResponseTemplate::new(206).set_body_bytes(raw),
            _ => unreachable!(),
        };
        copy(&fixture, TOKEN, response, 1).await;
        assert_eq!(
            reconcile(&fixture, &request, &fixture.client)
                .await
                .unwrap()
                .state,
            MailDispatchState::Uncertain,
            "{kind}"
        );
        assert_eq!(fixture.posts().await.len(), 1);
    }
}

#[tokio::test]
async fn a_renewed_same_account_lease_can_lookup_without_rebinding_the_send() {
    let (fixture, request, raw, id) = uncertain().await;
    const RENEWED: &str = "synthetic-renewed-sent-token";
    let connections = Connections::test_connected_with_id(
        ServiceProvider::Outlook,
        RENEWED,
        "renewed-connection",
    )
    .await;
    folder(&fixture, RENEWED).await;
    candidates(
        &fixture,
        RENEWED,
        &id,
        ResponseTemplate::new(200).set_body_json(json!({"value":[metadata(&id)]})),
    )
    .await;
    copy(
        &fixture,
        RENEWED,
        ResponseTemplate::new(200).set_body_bytes(raw),
        1,
    )
    .await;
    let session = fixture
        .runtime
        .session(&connections, ServiceProvider::Outlook, "renewed-connection")
        .await
        .unwrap();
    assert_eq!(session.authorization().mailbox(), &fixture.mailbox);
    let status = session
        .reconcile_uncertain_send(request.host_id, request.operation_id, &fixture.client)
        .await
        .unwrap();
    drop(session);
    assert_eq!(status.state, MailDispatchState::Accepted);
    assert_eq!(fixture.posts().await.len(), 1);
}

#[tokio::test]
async fn a_foreign_host_target_is_rejected_before_any_network_read() {
    use crate::mail::compose::reconciliation::{
        MailSentLookup, SentLookupOutcome, SentLookupTarget,
    };
    struct Capture(std::sync::Mutex<Option<Arc<SentLookupTarget>>>);
    #[async_trait::async_trait]
    impl MailSentLookup for Capture {
        fn provider(&self) -> ServiceProvider {
            ServiceProvider::Outlook
        }
        async fn find_sent(
            &self,
            session: &MailSession,
            target: Arc<SentLookupTarget>,
        ) -> Result<SentLookupOutcome, MailSendProblem> {
            target.validate_session(session)?;
            *self.0.lock().unwrap() = Some(target);
            Ok(SentLookupOutcome::Unconfirmed)
        }
    }
    let (fixture, request, _, _) = uncertain().await;
    let session = fixture.session().await;
    let capture = Capture(std::sync::Mutex::new(None));
    let status = session
        .reconcile_uncertain_send(request.host_id, request.operation_id, &capture)
        .await
        .unwrap();
    drop(session);
    assert_eq!(status.state, MailDispatchState::Uncertain);
    let target = capture.0.lock().unwrap().take().unwrap();
    let foreign = MailRuntime::open(fixture._directory.path().join("foreign-mail.sqlite3"))
        .await
        .unwrap();
    let session = tokio::time::timeout(
        TIMEOUT,
        foreign.session(
            &fixture.connections,
            ServiceProvider::Outlook,
            "synthetic-connection",
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let before = fixture.server.received_requests().await.unwrap().len();
    let result = MailSentLookup::find_sent(&fixture.client, &session, target).await;
    drop(session);
    assert!(matches!(
        result,
        Err(MailSendProblem {
            kind: MailSendProblemKind::AccountChanged,
            ..
        })
    ));
    assert_eq!(
        fixture.server.received_requests().await.unwrap().len(),
        before
    );
    assert_eq!(fixture.posts().await.len(), 1);
}

#[tokio::test]
async fn lookup_permission_and_throttling_failures_are_redacted_and_preserve_uncertainty() {
    for (code, kind) in [
        (401, MailSendProblemKind::SignIn),
        (403, MailSendProblemKind::Permission),
        (429, MailSendProblemKind::RateLimited),
    ] {
        let (fixture, request, _, id) = uncertain().await;
        folder(&fixture, TOKEN).await;
        candidates(
            &fixture,
            TOKEN,
            &id,
            ResponseTemplate::new(code)
                .insert_header("retry-after", "7")
                .set_body_string("PRIVATE-LOOKUP-CANARY recipient@example.test"),
        )
        .await;
        let Err(ComposeError::Provider(problem)) =
            reconcile(&fixture, &request, &fixture.client).await
        else {
            panic!("expected a redacted lookup error");
        };
        assert_eq!(problem.kind, kind);
        assert_eq!(problem.retry_after_seconds, (code == 429).then_some(7));
        assert!(!format!("{problem:?}").contains("PRIVATE-LOOKUP"));
        assert_eq!(
            fixture.status(&request).await.unwrap().state,
            MailDispatchState::Uncertain
        );
        assert_eq!(fixture.posts().await.len(), 1);
    }
}

#[tokio::test]
async fn an_oversized_copy_is_bounded_before_it_can_be_verified() {
    let (fixture, request, raw, id) = uncertain().await;
    folder(&fixture, TOKEN).await;
    candidates(
        &fixture,
        TOKEN,
        &id,
        ResponseTemplate::new(200).set_body_json(json!({"value":[metadata(&id)]})),
    )
    .await;
    let oversized = vec![b'x'; raw.len() + (1 << 20) + 1];
    copy(
        &fixture,
        TOKEN,
        ResponseTemplate::new(200).set_body_bytes(oversized),
        1,
    )
    .await;
    assert!(matches!(
        reconcile(&fixture, &request, &fixture.client).await,
        Err(ComposeError::Provider(MailSendProblem {
            kind: MailSendProblemKind::TooLarge,
            ..
        }))
    ));
    assert_eq!(
        fixture.status(&request).await.unwrap().state,
        MailDispatchState::Uncertain
    );
    assert_eq!(fixture.posts().await.len(), 1);
}

struct StopOnDrop(tokio::task::AbortHandle);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[tokio::test]
async fn a_chunked_copy_without_content_length_obeys_the_same_stream_limit() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let (fixture, request, raw, id) = uncertain().await;
    let limit = raw.len() + (1 << 20);
    let responses = [
        json!({"id":FOLDER}).to_string(),
        json!({"value":[metadata(&id)]}).to_string(),
    ];
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for index in 0..3 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                let mut block = [0; 2048];
                let count = stream.read(&mut block).await.unwrap();
                assert!(count > 0 && request.len() < 16_384);
                request.extend_from_slice(&block[..count]);
            }
            requests.push(String::from_utf8(request).unwrap());
            if let Some(body) = responses.get(index) {
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            } else {
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: message/rfc822\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").await.unwrap();
                let block = [b'x'; 8192];
                let mut written = 0;
                while written <= limit {
                    let count = (limit + 1 - written).min(block.len());
                    if stream
                        .write_all(format!("{count:x}\r\n").as_bytes())
                        .await
                        .is_err()
                        || stream.write_all(&block[..count]).await.is_err()
                        || stream.write_all(b"\r\n").await.is_err()
                    {
                        break;
                    }
                    written += count;
                }
                let _ = stream.write_all(b"0\r\n\r\n").await;
            }
        }
        requests
    });
    let stop = StopOnDrop(server.abort_handle());
    let client = OutlookClient::for_test_origin(&format!("http://{address}")).unwrap();
    let result = reconcile(&fixture, &request, &client).await;
    let requests = tokio::time::timeout(TIMEOUT, server)
        .await
        .unwrap()
        .unwrap();
    drop(stop);
    assert!(matches!(
        result,
        Err(ComposeError::Provider(MailSendProblem {
            kind: MailSendProblemKind::TooLarge,
            ..
        }))
    ));
    assert_eq!(requests.len(), 3);
    for request in requests {
        assert!(request.starts_with("GET /v1.0/me/"));
        assert!(request
            .to_ascii_lowercase()
            .contains(&format!("authorization: bearer {TOKEN}")));
    }
    assert_eq!(
        fixture.status(&request).await.unwrap().state,
        MailDispatchState::Uncertain
    );
    assert_eq!(fixture.posts().await.len(), 1);
}
