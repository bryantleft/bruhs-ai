use super::*;
use bruhs_models::mail::{MailDispatchState, MailSendRequest};

async fn uncertain() -> (Fixture, MailSendRequest, Vec<u8>, String) {
    let fixture = Fixture::new().await;
    let (request, _) = send::reviewed(&fixture, false).await;
    send::respond(&fixture, ResponseTemplate::new(500)).await;
    assert!(send::send(&fixture, request.clone()).await.is_err());
    let requests = fixture.server.received_requests().await.unwrap();
    let post = requests
        .iter()
        .find(|request| request.method.as_str() == "POST")
        .unwrap();
    let body: Value = serde_json::from_slice(&post.body).unwrap();
    let raw = URL_SAFE_NO_PAD
        .decode(body["raw"].as_str().unwrap())
        .unwrap();
    let id = mail_parser::MessageParser::default()
        .parse(&raw)
        .unwrap()
        .message_id()
        .unwrap()
        .to_owned();
    (fixture, request, raw, id)
}

async fn candidates(fixture: &Fixture, message_id: &str, value: Value) {
    Mock::given(method("GET"))
        .and(path(format!("{API}/messages")))
        .and(header("authorization", format!("Bearer {TOKEN}")))
        .and(query_param("q", format!("rfc822msgid:<{message_id}>")))
        .and(query_param("labelIds", "SENT"))
        .and(query_param("maxResults", "2"))
        .and(query_param("includeSpamTrash", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(value))
        .expect(1)
        .mount(&fixture.server)
        .await;
}

async fn sent_copy(fixture: &Fixture, value: Value) {
    Mock::given(method("GET"))
        .and(path(format!("{API}/messages/sent-a")))
        .and(header("authorization", format!("Bearer {TOKEN}")))
        .and(query_param("format", "raw"))
        .respond_with(ResponseTemplate::new(200).set_body_json(value))
        .expect(1)
        .mount(&fixture.server)
        .await;
}

async fn reconcile(
    fixture: &Fixture,
    request: &MailSendRequest,
) -> bruhs_models::mail::MailSendStatus {
    let session = fixture.session().await;
    let result = session
        .reconcile_uncertain_send(
            request.host_id.clone(),
            request.operation_id.clone(),
            &fixture.client,
        )
        .await;
    drop(session);
    result.unwrap()
}

async fn assert_one_dispatch(fixture: &Fixture) {
    assert_eq!(
        fixture
            .server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.method.as_str() == "POST")
            .count(),
        1
    );
}

#[tokio::test]
async fn unique_sent_copy_of_frozen_multipart_mime_confirms_without_dispatching_again() {
    let (fixture, request, raw, id) = uncertain().await;
    candidates(
        &fixture,
        &id,
        json!({"messages":[{"id":"sent-a","threadId":"thread-a"}]}),
    )
    .await;
    let mut copy = b"Received: by fixture.example.test\r\n".to_vec();
    copy.extend_from_slice(&raw);
    sent_copy(
        &fixture,
        json!({"id":"sent-a","threadId":"thread-a","labelIds":["SENT"],"raw":URL_SAFE_NO_PAD.encode(copy)}),
    )
    .await;
    let status = reconcile(&fixture, &request).await;
    assert_eq!(status.state, MailDispatchState::Accepted);
    assert_eq!(status.provider_message_id.as_deref(), Some("sent-a"));
    assert_eq!(status.provider_thread_id.as_deref(), Some("thread-a"));
    assert_eq!(
        reconcile(&fixture, &request).await.state,
        MailDispatchState::Accepted
    );
    assert_eq!(
        send::send(&fixture, request).await.unwrap().state,
        MailDispatchState::Accepted
    );
    assert_one_dispatch(&fixture).await;
}

#[tokio::test]
async fn missing_ambiguous_or_incomplete_sent_search_never_confirms_or_fetches_a_copy() {
    for result in [
        json!({}),
        json!({"messages":[{"id":"sent-a"},{"id":"sent-b"}]}),
        json!({"messages":[{"id":"sent-a"}],"nextPageToken":"unread-page"}),
    ] {
        let (fixture, request, _, id) = uncertain().await;
        candidates(&fixture, &id, result).await;
        assert_eq!(
            reconcile(&fixture, &request).await.state,
            MailDispatchState::Uncertain
        );
        assert_one_dispatch(&fixture).await;
        let requests = fixture.server.received_requests().await.unwrap();
        assert!(!requests.iter().any(|request| request
            .url
            .path()
            .starts_with(&format!("{API}/messages/"))
            && request.method.as_str() == "GET"));
    }
}

#[tokio::test]
async fn sent_metadata_and_frozen_content_must_both_match() {
    for case in [
        "not-sent",
        "draft",
        "id",
        "thread",
        "content",
        "message-id",
        "bcc",
    ] {
        let (fixture, request, raw, id) = uncertain().await;
        candidates(
            &fixture,
            &id,
            json!({"messages":[{"id":"sent-a","threadId":"thread-a"}]}),
        )
        .await;
        let mut value = json!({"id":"sent-a","threadId":"thread-a","labelIds":["SENT"],"raw":URL_SAFE_NO_PAD.encode(&raw)});
        match case {
            "not-sent" => value["labelIds"] = json!(["INBOX"]),
            "draft" => value["labelIds"] = json!(["SENT", "DRAFT"]),
            "id" => value["id"] = json!("foreign-message"),
            "thread" => value["threadId"] = json!("foreign-thread"),
            "content" => {
                let mut changed = raw;
                changed.push(b'x');
                value["raw"] = json!(URL_SAFE_NO_PAD.encode(changed));
            }
            "message-id" => {
                let changed = String::from_utf8(raw)
                    .unwrap()
                    .replace(&id, "different@example.test");
                value["raw"] = json!(URL_SAFE_NO_PAD.encode(changed));
            }
            "bcc" => {
                let changed = String::from_utf8(raw)
                    .unwrap()
                    .replace("hidden@example.test", "other@example.test");
                value["raw"] = json!(URL_SAFE_NO_PAD.encode(changed));
            }
            _ => unreachable!(),
        }
        sent_copy(&fixture, value).await;
        assert_eq!(
            reconcile(&fixture, &request).await.state,
            MailDispatchState::Uncertain,
            "{case}"
        );
        assert_one_dispatch(&fixture).await;
    }
}
