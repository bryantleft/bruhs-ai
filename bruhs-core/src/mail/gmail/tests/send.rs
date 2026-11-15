use super::*;
use crate::mail::compose::MailSendTransport;
use mail_parser::MimeHeaders;
use sha2::{Digest as _, Sha256};
use std::sync::Arc;

pub(super) async fn reviewed(
    fixture: &Fixture,
    reply_to: bool,
) -> (bruhs_models::mail::MailSendRequest, String) {
    use bruhs_models::mail::*;
    reply(
        &fixture.server,
        "profile",
        json!({"emailAddress":"you@example.test","historyId":"123"}),
    )
    .await;
    let active = fixture.session().await;
    let target = if reply_to {
        seed(&active, "source-thread", "Saved source", None).await;
        Some(MailReplyTarget {
            external_thread_id: "source-thread".into(),
            message_id: active
                .authorization()
                .mailbox()
                .message("source-thread-message")
                .unwrap()
                .canonical_id,
        })
    } else {
        None
    };
    let mut draft = fixture
        .runtime
        .compose_create(
            active.authorization().mailbox().as_str().into(),
            MailDraftCreate {
                host_id: fixture.runtime.host().as_str().into(),
                client_id: "send-a".into(),
                reply: target,
            },
        )
        .await
        .unwrap();
    if !reply_to {
        draft.content.to = "Recipient <recipient@example.test>".into();
        draft.content.subject = "Résumé 東京".into();
    }
    draft.content.cc = "copy@example.test".into();
    draft.content.bcc = "hidden@example.test".into();
    draft.content.body = "Private reviewed body 東京 <script>".into();
    let upload = fixture
        .runtime
        .compose_upload(
            fixture.runtime.host().as_str().into(),
            active.authorization().mailbox().as_str().into(),
            "send-a".into(),
            crate::mail::DraftAttachmentUpload {
                client_id: "file-a".into(),
                file_name: "résumé-東京.bin".into(),
                bytes: b"private\0file\xff".to_vec(),
            },
        )
        .await
        .unwrap();
    draft.content.attachments = vec![upload.id];
    let draft = fixture
        .runtime
        .compose_save(
            active.authorization().mailbox().as_str().into(),
            "send-a".into(),
            MailDraftSave {
                host_id: fixture.runtime.host().as_str().into(),
                expected_revision: draft.revision,
                content: draft.content,
            },
        )
        .await
        .unwrap();
    let sender = fixture.client.sender(&active).await.unwrap();
    let review = active
        .review_compose_with_limit(
            &sender,
            "send-a".into(),
            MailReviewRequest {
                host_id: fixture.runtime.host().as_str().into(),
                connection_id: "synthetic-connection".into(),
                expected_revision: draft.revision,
            },
            fixture.client.mime_limit_bytes(),
        )
        .await
        .unwrap();
    drop(active);
    (
        MailSendRequest {
            host_id: review.host_id,
            connection_id: review.connection_id,
            operation_id: "send-operation".into(),
            revision: review.revision,
            digest: review.digest,
        },
        review.mime_sha256,
    )
}

pub(super) async fn send(
    fixture: &Fixture,
    request: bruhs_models::mail::MailSendRequest,
) -> Result<bruhs_models::mail::MailSendStatus, crate::mail::compose::ComposeError> {
    fixture
        .runtime
        .send_reviewed_compose(
            fixture.session().await,
            Arc::new(fixture.client.clone()),
            "send-a".into(),
            request,
        )
        .wait()
        .await
}

pub(super) async fn respond(fixture: &Fixture, response: ResponseTemplate) {
    Mock::given(method("POST"))
        .and(path(format!("{API}/messages/send")))
        .and(header("authorization", format!("Bearer {TOKEN}")))
        .respond_with(response)
        .expect(1)
        .mount(&fixture.server)
        .await;
}

#[tokio::test]
async fn send_posts_exact_frozen_mime_with_checked_recipients_binary_file_and_no_duplicate() {
    use bruhs_models::mail::*;
    let fixture = Fixture::new().await;
    let (request, digest) = reviewed(&fixture, false).await;
    respond(
        &fixture,
        ResponseTemplate::new(200)
            .set_body_json(json!({"id":"accepted-message","threadId":"accepted-thread"}))
            .insert_header("x-request-id", "request-a"),
    )
    .await;
    let result = send(&fixture, request.clone()).await.unwrap();
    assert_eq!(result.state, MailDispatchState::Accepted);
    assert_eq!(
        result.provider_message_id.as_deref(),
        Some("accepted-message")
    );
    assert_eq!(
        send(&fixture, request).await.unwrap().state,
        MailDispatchState::Accepted
    );
    let posts = fixture
        .server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|request| request.method.as_str() == "POST")
        .collect::<Vec<_>>();
    assert_eq!(posts.len(), 1);
    let body: Value = serde_json::from_slice(&posts[0].body).unwrap();
    assert!(body.get("threadId").is_none());
    let bytes = URL_SAFE_NO_PAD
        .decode(body["raw"].as_str().unwrap())
        .unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), digest);
    let message = mail_parser::MessageParser::default().parse(&bytes).unwrap();
    assert_eq!(message.subject(), Some("Résumé 東京"));
    assert_eq!(
        message.from().unwrap().first().unwrap().address(),
        Some("you@example.test")
    );
    assert_eq!(
        message.to().unwrap().first().unwrap().address(),
        Some("recipient@example.test")
    );
    assert_eq!(
        message.bcc().unwrap().first().unwrap().address(),
        Some("hidden@example.test")
    );
    let file = message.attachments().next().unwrap();
    assert_eq!(file.attachment_name(), Some("résumé-東京.bin"));
    assert_eq!(file.contents(), b"private\0file\xff");
}

#[tokio::test]
async fn replies_post_exact_cached_thread_and_checked_rfc_reply_headers() {
    let fixture = Fixture::new().await;
    let (request, digest) = reviewed(&fixture, true).await;
    respond(
        &fixture,
        ResponseTemplate::new(200)
            .set_body_json(json!({"id":"reply-message","threadId":"source-thread"})),
    )
    .await;
    send(&fixture, request).await.unwrap();
    let posts = fixture
        .server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|request| request.method.as_str() == "POST")
        .collect::<Vec<_>>();
    assert_eq!(posts.len(), 1);
    let body: Value = serde_json::from_slice(&posts[0].body).unwrap();
    assert_eq!(body["threadId"], "source-thread");
    let bytes = URL_SAFE_NO_PAD
        .decode(body["raw"].as_str().unwrap())
        .unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), digest);
    let message = mail_parser::MessageParser::default().parse(&bytes).unwrap();
    assert_eq!(message.subject(), Some("Café planning"));
    assert!(format!("{:?}", message.in_reply_to()).contains("source-thread-message@example.test"));
}

#[tokio::test]
async fn explicit_client_errors_are_rejected_and_server_errors_are_uncertain_without_retries() {
    use bruhs_models::mail::*;
    for (status, state, kind) in [
        (
            400,
            MailDispatchState::Rejected,
            MailSendProblemKind::InvalidMessage,
        ),
        (
            401,
            MailDispatchState::Rejected,
            MailSendProblemKind::SignIn,
        ),
        (
            403,
            MailDispatchState::Rejected,
            MailSendProblemKind::Permission,
        ),
        (
            413,
            MailDispatchState::Rejected,
            MailSendProblemKind::TooLarge,
        ),
        (
            429,
            MailDispatchState::Rejected,
            MailSendProblemKind::RateLimited,
        ),
        (
            500,
            MailDispatchState::Uncertain,
            MailSendProblemKind::Unconfirmed,
        ),
    ] {
        let fixture = Fixture::new().await;
        let (request, _) = reviewed(&fixture, false).await;
        respond(
            &fixture,
            ResponseTemplate::new(status)
                .set_body_json(json!({"error":{"message":"PRIVATE PROVIDER BODY"}}))
                .insert_header("retry-after", "7"),
        )
        .await;
        let Err(error) = send(&fixture, request.clone()).await else {
            panic!("Expected a provider rejection or uncertain outcome")
        };
        assert!(!format!("{error:?} {error}").contains("PRIVATE PROVIDER BODY"));
        let active = fixture.session().await;
        let mailbox = active.authorization().mailbox().as_str().to_owned();
        drop(active);
        let saved = fixture
            .runtime
            .compose_send_status(
                fixture.runtime.host().as_str().into(),
                mailbox,
                request.operation_id.clone(),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.state, state);
        assert_eq!(saved.problem.as_ref().unwrap().kind, kind);
        if status == 429 {
            assert_eq!(saved.problem.unwrap().retry_after_seconds, Some(7));
        }
        send(&fixture, request).await.unwrap();
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
}

#[tokio::test]
async fn missing_malformed_or_oversized_success_receipt_is_uncertain_and_never_replayed() {
    use bruhs_models::mail::*;
    for body in [
        "{}".to_string(),
        "not JSON".into(),
        json!({"id":"bad\nidentity"}).to_string(),
        "x".repeat(65537),
    ] {
        let fixture = Fixture::new().await;
        let (request, _) = reviewed(&fixture, false).await;
        respond(&fixture, ResponseTemplate::new(200).set_body_string(body)).await;
        assert!(send(&fixture, request.clone()).await.is_err());
        assert_eq!(
            send(&fixture, request).await.unwrap().state,
            MailDispatchState::Uncertain
        );
    }
}

#[tokio::test]
async fn send_redirect_never_receives_bearer_token_or_body_at_another_origin() {
    use bruhs_models::mail::*;
    let fixture = Fixture::new().await;
    let (request, _) = reviewed(&fixture, false).await;
    let other = MockServer::builder().start().await;
    respond(
        &fixture,
        ResponseTemplate::new(307).insert_header("location", format!("{}/steal", other.uri())),
    )
    .await;
    assert!(send(&fixture, request.clone()).await.is_err());
    assert_eq!(
        send(&fixture, request).await.unwrap().state,
        MailDispatchState::Uncertain
    );
    assert!(other.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn lost_send_response_is_uncertain_after_one_real_post_even_when_retried_by_caller() {
    use bruhs_models::mail::*;
    let mut fixture = Fixture::new().await;
    let (request, _) = reviewed(&fixture, false).await;
    fixture.client.http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .timeout(std::time::Duration::from_millis(500))
        .build()
        .unwrap();
    respond(
        &fixture,
        ResponseTemplate::new(200)
            .set_body_json(json!({"id":"late-acceptance"}))
            .set_delay(std::time::Duration::from_secs(2)),
    )
    .await;
    assert!(send(&fixture, request.clone()).await.is_err());
    assert_eq!(
        send(&fixture, request).await.unwrap().state,
        MailDispatchState::Uncertain
    );
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
