use super::*;
use wiremock::matchers::query_param;

async fn uncertain() -> (Fixture, MailSendRequest, MailSendReview, Vec<u8>) {
    let f = Fixture::new(ServiceProvider::Gmail).await;
    let draft = f.saved("sent-proof").await;
    let review = f.review("sent-proof", draft.revision).await;
    Mock::given(method("POST"))
        .and(path("/gmail/v1/users/me/messages/send"))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&f.provider)
        .await;
    let request = f.request(&review);
    assert!(f
        .client
        .mail_draft_send(&f.mailbox, "sent-proof", &request)
        .await
        .is_err());
    let posts = f.posts().await;
    assert_eq!(posts.len(), 1);
    let body: Value = serde_json::from_slice(&posts[0].body).unwrap();
    let raw = URL_SAFE_NO_PAD
        .decode(body["raw"].as_str().unwrap())
        .unwrap();
    (f, request, review, raw)
}

fn endpoint(f: &Fixture, operation: &str) -> String {
    format!(
        "{}/api/mail/mailboxes/{}/sends/{operation}/reconcile",
        f.base, f.mailbox
    )
}

fn request(f: &Fixture) -> Value {
    json!({"host_id":f.host(),"connection_id":"synthetic-connection"})
}

async fn stored(f: &Fixture, operation: &str) -> Value {
    serde_json::to_value(
        f.client
            .mail_send_status(&f.host(), &f.mailbox, operation)
            .await
            .unwrap(),
    )
    .unwrap()
}

async fn candidates(f: &Fixture, id: &str, body: Value) {
    Mock::given(method("GET"))
        .and(path("/gmail/v1/users/me/messages"))
        .and(header("authorization", format!("Bearer {TOKEN}")))
        .and(query_param("q", format!("rfc822msgid:<{id}>")))
        .and(query_param("labelIds", "SENT"))
        .and(query_param("maxResults", "2"))
        .and(query_param("includeSpamTrash", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .expect(1)
        .mount(&f.provider)
        .await;
}

#[tokio::test]
async fn only_explicit_post_checks_sent_and_matching_copy_never_resends() {
    let (f, send, review, raw) = uncertain().await;
    let before = f.provider.received_requests().await.unwrap().len();
    assert_eq!(stored(&f, &send.operation_id).await["state"], "uncertain");
    assert_eq!(
        f.provider.received_requests().await.unwrap().len(),
        before,
        "ordinary status GET must remain pure"
    );
    candidates(
        &f,
        &review.message_id,
        json!({"messages":[{"id":"sent-copy","threadId":"sent-thread"}]}),
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/gmail/v1/users/me/messages/sent-copy"))
        .and(query_param("format", "raw"))
        .and(header("authorization", format!("Bearer {TOKEN}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"sent-copy","threadId":"sent-thread","labelIds":["SENT"],"raw":URL_SAFE_NO_PAD.encode(raw)})))
        .expect(1)
        .mount(&f.provider)
        .await;
    for _ in 0..2 {
        let response = f
            .http
            .post(endpoint(&f, &send.operation_id))
            .json(&request(&f))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let status: MailSendStatus = response.json().await.unwrap();
        assert_eq!(status.state, MailDispatchState::Accepted);
        assert_eq!(status.provider_message_id.as_deref(), Some("sent-copy"));
    }
    assert_eq!(f.posts().await.len(), 1);
    assert_eq!(
        f.provider.received_requests().await.unwrap().len(),
        before + 2
    );
    assert_eq!(stored(&f, &send.operation_id).await["state"], "accepted");
}

#[tokio::test]
async fn unconfirmed_sent_results_preserve_original_uncertainty_and_receipt() {
    for results in [
        json!({}),
        json!({"messages":[{"id":"one"},{"id":"two"}]}),
        json!({"messages":[{"id":"one"}],"nextPageToken":"pending"}),
    ] {
        let (f, send, review, _) = uncertain().await;
        let before = stored(&f, &send.operation_id).await;
        candidates(&f, &review.message_id, results).await;
        let response = f
            .http
            .post(endpoint(&f, &send.operation_id))
            .json(&request(&f))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.json::<Value>().await.unwrap(), before);
        assert_eq!(stored(&f, &send.operation_id).await, before);
        assert_eq!(f.posts().await.len(), 1);
    }
}

#[tokio::test]
async fn reconcile_rejects_foreign_host_generation_and_browser_supplied_proof_without_io() {
    let (f, send, _, _) = uncertain().await;
    let before_status = stored(&f, &send.operation_id).await;
    let before_requests = f.provider.received_requests().await.unwrap().len();
    for (field, value, expected) in [
        ("host_id", json!("foreign-host"), StatusCode::CONFLICT),
        (
            "connection_id",
            json!("stale-connection"),
            StatusCode::FORBIDDEN,
        ),
        ("proof", json!({"accepted":true}), StatusCode::BAD_REQUEST),
        (
            "message_id",
            json!("browser-chosen"),
            StatusCode::BAD_REQUEST,
        ),
        ("mime", json!("browser-chosen"), StatusCode::BAD_REQUEST),
    ] {
        let mut body = request(&f);
        body[field] = value;
        let response = f
            .http
            .post(endpoint(&f, &send.operation_id))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), expected, "{field}");
        assert_eq!(stored(&f, &send.operation_id).await, before_status);
        assert_eq!(
            f.provider.received_requests().await.unwrap().len(),
            before_requests
        );
    }
}

#[tokio::test]
async fn lost_consent_on_check_sent_does_not_become_a_prequeue_rejection() {
    let (f, send, _, _) = uncertain().await;
    let before_status = stored(&f, &send.operation_id).await;
    let before_requests = f.provider.received_requests().await.unwrap().len();
    f.connections
        .disconnect(ServiceProvider::Gmail)
        .await
        .unwrap();
    let response = f
        .http
        .post(endpoint(&f, &send.operation_id))
        .json(&request(&f))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(stored(&f, &send.operation_id).await, before_status);
    assert_eq!(
        f.provider.received_requests().await.unwrap().len(),
        before_requests
    );
    assert_eq!(f.posts().await.len(), 1);
}

#[tokio::test]
async fn missing_operation_never_queries_sent_or_creates_a_journal_row() {
    let f = Fixture::new(ServiceProvider::Gmail).await;
    let response = f
        .http
        .post(endpoint(&f, "missing-operation"))
        .json(&request(&f))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(f.provider.received_requests().await.unwrap().is_empty());
    assert!(f
        .runtime
        .compose_send_status(f.host(), f.mailbox.clone(), "missing-operation".into())
        .await
        .unwrap()
        .is_none());
}
