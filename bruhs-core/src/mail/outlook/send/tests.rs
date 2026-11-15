use super::*;
use crate::{
    connections::Connections,
    mail::{compose::ComposeError, DraftAttachmentUpload, MailRuntime, MailboxId},
};
use bruhs_models::mail::{
    MailComposeContent, MailComposeDraft, MailDispatchState, MailDraftCreate, MailDraftSave,
    MailReviewRequest, MailSendRequest, MailSendStatus, MAIL_PREPARED_MIME_MAX_BYTES,
};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use std::{sync::Arc, time::Duration};
use wiremock::{
    matchers::{header, headers, method, path, query_param},
    Mock, MockServer, ResponseTemplate,
};

const TOKEN: &str = "synthetic-outlook-send-token";
const CLIENT: &str = "draft-a";
const REQUEST_ID: &str = "8c8ac9de-5937-49fd-bac9-849c84ef5e3a";

struct Fixture {
    _directory: tempfile::TempDir,
    runtime: MailRuntime,
    connections: Connections,
    mailbox: MailboxId,
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
        let session = runtime
            .session(
                &connections,
                ServiceProvider::Outlook,
                "synthetic-connection",
            )
            .await
            .unwrap();
        let mailbox = session.authorization().mailbox().clone();
        drop(session);
        let server = MockServer::builder().start().await;
        let client = OutlookClient::for_test_origin(&server.uri()).unwrap();
        let fixture = Self {
            _directory: directory,
            runtime,
            connections,
            mailbox,
            client,
            server,
        };
        fixture
            .profile(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"id":"sender-subject","mail":"verified@example.test"})),
            )
            .await;
        fixture
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

    async fn profile(&self, response: ResponseTemplate) {
        Mock::given(method("GET"))
            .and(path("/v1.0/me"))
            .and(query_param("$select", "id,mail"))
            .and(header("authorization", format!("Bearer {TOKEN}")))
            .and(headers(
                "prefer",
                super::super::PREFER.split(',').map(str::trim).collect(),
            ))
            .respond_with(response)
            .mount(&self.server)
            .await;
    }

    async fn respond(&self, response: ResponseTemplate) {
        Mock::given(method("POST"))
            .and(path("/v1.0/me/sendMail"))
            .and(header("authorization", format!("Bearer {TOKEN}")))
            .and(header("content-type", "text/plain"))
            .and(header("prefer", "IdType=\"ImmutableId\""))
            .respond_with(response)
            .mount(&self.server)
            .await;
    }

    async fn draft(
        &self,
        attachment: Option<Vec<u8>>,
    ) -> (MailSession, VerifiedMailSender, MailComposeDraft) {
        let session = self.session().await;
        let sender = self.client.verified_sender(&session).await.unwrap();
        let mut draft = self
            .runtime
            .compose_create(
                self.mailbox.as_str().into(),
                MailDraftCreate {
                    host_id: self.runtime.host().as_str().into(),
                    client_id: CLIENT.into(),
                    reply: None,
                },
            )
            .await
            .unwrap();
        let mut content = MailComposeContent {
            to: "Recipient 林 <to@example.test>".into(),
            cc: "copy@example.test".into(),
            bcc: "hidden@example.test".into(),
            subject: "Frozen MIME — 東京".into(),
            body: "Exact reviewed café body\nNo provider reconstruction.".into(),
            ..Default::default()
        };
        if let Some(bytes) = attachment {
            let uploaded = self
                .runtime
                .compose_upload(
                    self.runtime.host().as_str().into(),
                    self.mailbox.as_str().into(),
                    CLIENT.into(),
                    DraftAttachmentUpload {
                        client_id: "synthetic-upload".into(),
                        file_name: "bytes-東京.bin".into(),
                        bytes,
                    },
                )
                .await
                .unwrap();
            content.attachments.push(uploaded.id);
            draft = self
                .runtime
                .compose_draft(
                    self.runtime.host().as_str().into(),
                    self.mailbox.as_str().into(),
                    CLIENT.into(),
                )
                .await
                .unwrap()
                .unwrap();
        }
        let draft = self
            .runtime
            .compose_save(
                self.mailbox.as_str().into(),
                CLIENT.into(),
                MailDraftSave {
                    host_id: self.runtime.host().as_str().into(),
                    expected_revision: draft.revision,
                    content,
                },
            )
            .await
            .unwrap();
        (session, sender, draft)
    }

    async fn reviewed(
        &self,
        attachment: Option<Vec<u8>>,
        limit: usize,
    ) -> (MailSession, VerifiedMailSender, MailSendRequest) {
        let (session, sender, draft) = self.draft(attachment).await;
        let review = session
            .review_compose_with_limit(
                &sender,
                CLIENT.into(),
                MailReviewRequest {
                    host_id: self.runtime.host().as_str().into(),
                    connection_id: "synthetic-connection".into(),
                    expected_revision: draft.revision,
                },
                limit,
            )
            .await
            .unwrap();
        let request = MailSendRequest {
            host_id: review.host_id,
            connection_id: review.connection_id,
            operation_id: "operation-a".into(),
            revision: review.revision,
            digest: review.digest,
        };
        (session, sender, request)
    }

    async fn dispatch(
        &self,
        attachment: Option<Vec<u8>>,
        limit: usize,
    ) -> (MailSession, MailDispatch) {
        let (session, sender, request) = self.reviewed(attachment, limit).await;
        session
            .queue_reviewed_compose(&sender, CLIENT.into(), request.clone(), limit)
            .await
            .unwrap();
        let dispatch = session
            .claim_reviewed_compose(&sender, CLIENT.into(), request, limit)
            .await
            .unwrap()
            .unwrap();
        (session, dispatch)
    }

    async fn posts(&self) -> Vec<wiremock::Request> {
        self.server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|request| request.method.as_str() == "POST")
            .collect()
    }

    async fn status(&self, request: &MailSendRequest) -> Option<MailSendStatus> {
        self.runtime
            .compose_send_status(
                self.runtime.host().as_str().into(),
                self.mailbox.as_str().into(),
                request.operation_id.clone(),
            )
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn accepted_post_is_exact_frozen_mime_with_optional_graph_ids() {
    let f = Fixture::new().await;
    f.respond(
        ResponseTemplate::new(202)
            .insert_header("request-id", REQUEST_ID)
            .set_body_string("PRIVATE-PROVIDER-RESPONSE"),
    )
    .await;
    let (session, dispatch) = f
        .dispatch(Some(vec![0xa5; 4096]), f.client.mime_limit_bytes())
        .await;
    let expected = dispatch.mime().bytes().to_vec();
    let digest = dispatch.mime().sha256().to_owned();
    let outcome = f.client.send(&session, dispatch).await;
    let ProviderSendOutcome::Accepted(receipt) = outcome else {
        panic!("expected positive acknowledgement, got {outcome:?}");
    };
    assert!(receipt.message_id.is_none());
    assert!(receipt.thread_id.is_none());
    assert_eq!(receipt.request_id.as_deref(), Some(REQUEST_ID));
    let posts = f.posts().await;
    assert_eq!(posts.len(), 1);
    let actual = STANDARD.decode(&posts[0].body).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(format!("{:x}", Sha256::digest(&actual)), digest);
    assert!(posts[0].url.query().is_none());
    assert!(posts[0].body.len() <= 4_000_000);
    drop(session);
}

#[tokio::test]
async fn known_rejections_are_redacted_and_never_retried() {
    for (status, kind) in [
        (400, MailSendProblemKind::InvalidMessage),
        (401, MailSendProblemKind::SignIn),
        (403, MailSendProblemKind::Permission),
        (404, MailSendProblemKind::Unavailable),
        (413, MailSendProblemKind::TooLarge),
        (429, MailSendProblemKind::RateLimited),
    ] {
        let f = Fixture::new().await;
        f.respond(
            ResponseTemplate::new(status)
                .insert_header("retry-after", "7")
                .set_body_string("PRIVATE-PROVIDER-RESPONSE recipient@example.test"),
        )
        .await;
        let (session, dispatch) = f.dispatch(None, f.client.mime_limit_bytes()).await;
        let outcome = f.client.send(&session, dispatch).await;
        let ProviderSendOutcome::Rejected(problem) = outcome else {
            panic!("expected explicit rejection, got {outcome:?}");
        };
        assert_eq!(problem.kind, kind);
        assert_eq!(problem.retry_after_seconds, (status == 429).then_some(7));
        assert!(!problem.to_string().contains("PRIVATE-PROVIDER"));
        assert!(!format!("{problem:?}").contains("recipient@example.test"));
        assert_eq!(f.posts().await.len(), 1);
        drop(session);
    }
}

#[tokio::test]
async fn unexpected_success_timeout_and_server_errors_remain_uncertain() {
    for status in [200, 201, 204, 408, 500, 503] {
        let f = Fixture::new().await;
        f.respond(ResponseTemplate::new(status).set_body_string("PRIVATE-SERVER-CANARY"))
            .await;
        let (session, dispatch) = f.dispatch(None, f.client.mime_limit_bytes()).await;
        assert!(matches!(
            f.client.send(&session, dispatch).await,
            ProviderSendOutcome::Uncertain(MailSendProblem {
                kind: MailSendProblemKind::Unconfirmed,
                ..
            })
        ));
        assert_eq!(f.posts().await.len(), 1);
        drop(session);
    }
}

#[tokio::test]
async fn redirect_is_not_followed_and_cannot_claim_rejection_or_acceptance() {
    let f = Fixture::new().await;
    let target = MockServer::start().await;
    f.respond(
        ResponseTemplate::new(307)
            .insert_header("location", format!("{}/unexpected-send", target.uri())),
    )
    .await;
    let (session, dispatch) = f.dispatch(None, f.client.mime_limit_bytes()).await;
    assert!(matches!(
        f.client.send(&session, dispatch).await,
        ProviderSendOutcome::Uncertain(_)
    ));
    assert_eq!(f.posts().await.len(), 1);
    assert!(target.received_requests().await.unwrap().is_empty());
    drop(session);
}

#[tokio::test]
async fn wrong_host_capability_is_rejected_before_any_network_read_or_send() {
    let f = Fixture::new().await;
    let (session, dispatch) = f.dispatch(None, f.client.mime_limit_bytes()).await;
    // Dispatch owns its binding and frozen data. Release the exclusive broker
    // lease before acquiring the same connection through a different host.
    drop(session);
    let directory = tempfile::tempdir().unwrap();
    let foreign = MailRuntime::open(directory.path().join("mail.sqlite3"))
        .await
        .unwrap();
    let other_session = tokio::time::timeout(
        Duration::from_secs(3),
        foreign.session(
            &f.connections,
            ServiceProvider::Outlook,
            "synthetic-connection",
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let before = f.server.received_requests().await.unwrap().len();
    assert!(matches!(
        f.client.send(&other_session, dispatch).await,
        ProviderSendOutcome::Rejected(MailSendProblem {
            kind: MailSendProblemKind::AccountChanged,
            ..
        })
    ));
    assert_eq!(f.server.received_requests().await.unwrap().len(), before);
    drop(other_session);
}

#[tokio::test]
async fn outlook_budget_rejects_before_freezing_and_again_at_transport() {
    let f = Fixture::new().await;
    let (session, sender, draft) = f.draft(Some(vec![0xa5; 2_300_000])).await;
    let request = MailReviewRequest {
        host_id: f.runtime.host().as_str().into(),
        connection_id: "synthetic-connection".into(),
        expected_revision: draft.revision,
    };
    assert!(matches!(
        session
            .review_compose_with_limit(
                &sender,
                CLIENT.into(),
                request.clone(),
                f.client.mime_limit_bytes()
            )
            .await,
        Err(ComposeError::Provider(MailSendProblem {
            kind: MailSendProblemKind::TooLarge,
            ..
        }))
    ));
    let after = f
        .runtime
        .compose_draft(
            f.runtime.host().as_str().into(),
            f.mailbox.as_str().into(),
            CLIENT.into(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.revision, draft.revision);
    assert_eq!(after.content, draft.content);
    let review = session
        .review_compose_with_limit(
            &sender,
            CLIENT.into(),
            request,
            MAIL_PREPARED_MIME_MAX_BYTES,
        )
        .await
        .unwrap();
    let send = MailSendRequest {
        host_id: review.host_id,
        connection_id: review.connection_id,
        operation_id: "operation-large".into(),
        revision: review.revision,
        digest: review.digest,
    };
    session
        .queue_reviewed_compose(
            &sender,
            CLIENT.into(),
            send.clone(),
            MAIL_PREPARED_MIME_MAX_BYTES,
        )
        .await
        .unwrap();
    let dispatch = session
        .claim_reviewed_compose(&sender, CLIENT.into(), send, MAIL_PREPARED_MIME_MAX_BYTES)
        .await
        .unwrap()
        .unwrap();
    assert!(dispatch.mime().bytes().len() > f.client.mime_limit_bytes());
    let before = f.server.received_requests().await.unwrap().len();
    assert!(matches!(
        f.client.send(&session, dispatch).await,
        ProviderSendOutcome::Rejected(MailSendProblem {
            kind: MailSendProblemKind::TooLarge,
            ..
        })
    ));
    assert_eq!(f.server.received_requests().await.unwrap().len(), before);
    drop(session);
}

#[tokio::test]
async fn dropped_waiter_keeps_real_provider_attempt_and_receipt_owned() {
    let f = Fixture::new().await;
    f.respond(ResponseTemplate::new(202).set_delay(Duration::from_millis(250)))
        .await;
    let (session, _, request) = f.reviewed(None, f.client.mime_limit_bytes()).await;
    let task = f.runtime.send_reviewed_compose(
        session,
        Arc::new(f.client.clone()),
        CLIENT.into(),
        request.clone(),
    );
    tokio::time::timeout(Duration::from_secs(3), async {
        while f.posts().await.is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    drop(task);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if f.status(&request)
                .await
                .is_some_and(|status| status.state == MailDispatchState::Accepted)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let repeated = f
        .runtime
        .send_reviewed_compose(
            f.session().await,
            Arc::new(f.client.clone()),
            CLIENT.into(),
            request,
        )
        .wait()
        .await
        .unwrap();
    assert_eq!(repeated.state, MailDispatchState::Accepted);
    assert!(repeated.provider_message_id.is_none());
    assert!(repeated.provider_thread_id.is_none());
    assert_eq!(f.posts().await.len(), 1);
}

#[tokio::test]
async fn lost_response_is_persisted_uncertain_and_same_operation_is_not_replayed() {
    let mut f = Fixture::new().await;
    f.client.http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .timeout(Duration::from_millis(500))
        .build()
        .unwrap();
    f.respond(ResponseTemplate::new(202).set_delay(Duration::from_secs(2)))
        .await;
    let (session, _, request) = f.reviewed(None, f.client.mime_limit_bytes()).await;
    let first = f
        .runtime
        .send_reviewed_compose(
            session,
            Arc::new(f.client.clone()),
            CLIENT.into(),
            request.clone(),
        )
        .wait()
        .await;
    assert!(matches!(
        first,
        Err(ComposeError::Provider(MailSendProblem {
            kind: MailSendProblemKind::Unconfirmed,
            ..
        }))
    ));
    let persisted = f.status(&request).await.unwrap();
    // Acquire the second lease only after the first worker has completed.
    let repeated = f
        .runtime
        .send_reviewed_compose(
            tokio::time::timeout(Duration::from_secs(3), f.session())
                .await
                .unwrap(),
            Arc::new(f.client.clone()),
            CLIENT.into(),
            request,
        )
        .wait()
        .await
        .unwrap();
    for status in [persisted, repeated] {
        assert_eq!(status.state, MailDispatchState::Uncertain);
        assert_eq!(
            status.problem.unwrap().kind,
            MailSendProblemKind::Unconfirmed
        );
    }
    assert_eq!(f.posts().await.len(), 1);
}

#[tokio::test]
async fn sender_preflight_preserves_redacted_recovery_problem_kinds() {
    for (status, kind) in [
        (401, MailSendProblemKind::SignIn),
        (403, MailSendProblemKind::Permission),
        (429, MailSendProblemKind::RateLimited),
    ] {
        let f = Fixture::new().await;
        f.server.reset().await;
        f.profile(
            ResponseTemplate::new(status)
                .insert_header("retry-after", "7")
                .set_body_string("PRIVATE-PROFILE-CANARY"),
        )
        .await;
        let session = f.session().await;
        let problem = f.client.verified_sender(&session).await.unwrap_err();
        assert_eq!(problem.kind, kind);
        assert_eq!(problem.retry_after_seconds, (status == 429).then_some(7));
        assert!(!format!("{problem:?}").contains("PRIVATE-PROFILE"));
        assert!(f.posts().await.is_empty());
        drop(session);
    }
}
