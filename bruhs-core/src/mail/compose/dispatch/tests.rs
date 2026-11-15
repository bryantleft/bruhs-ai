use super::*;
use crate::{
    connections::Connections,
    mail::{MailboxId, ProviderAcceptance},
};
use bruhs_models::connections::ServiceProvider;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};
use tokio::sync::Notify;

mod prequeue;
mod reconciliation;

struct Fixture {
    runtime: MailRuntime,
    connections: Connections,
    mailbox: MailboxId,
    request: MailSendRequest,
    folder: tempfile::TempDir,
}

fn address() -> MailAddress {
    MailAddress {
        name: String::new(),
        address: "sender@example.test".into(),
    }
}

impl Fixture {
    async fn new() -> Self {
        let folder = tempfile::tempdir().unwrap();
        let runtime = MailRuntime::open(folder.path().join("mail.sqlite3"))
            .await
            .unwrap();
        let connections =
            Connections::test_connected(ServiceProvider::Gmail, "synthetic-token").await;
        let session = runtime
            .session(&connections, ServiceProvider::Gmail, "synthetic-connection")
            .await
            .unwrap();
        let mailbox = session.authorization().mailbox().clone();
        let sender = VerifiedMailSender::from_profile(&session, address()).unwrap();
        let draft = runtime
            .compose_create(
                mailbox.as_str().into(),
                MailDraftCreate {
                    host_id: runtime.host().as_str().into(),
                    client_id: "draft-a".into(),
                    reply: None,
                },
            )
            .await
            .unwrap();
        let draft = runtime
            .compose_save(
                mailbox.as_str().into(),
                "draft-a".into(),
                MailDraftSave {
                    host_id: runtime.host().as_str().into(),
                    expected_revision: draft.revision,
                    content: MailComposeContent {
                        to: "recipient@example.test".into(),
                        subject: "Synthetic reviewed email".into(),
                        body: "Exact reviewed body 東京".into(),
                        ..Default::default()
                    },
                },
            )
            .await
            .unwrap();
        let review = session
            .review_compose(
                &sender,
                "draft-a".into(),
                MailReviewRequest {
                    host_id: runtime.host().as_str().into(),
                    connection_id: "synthetic-connection".into(),
                    expected_revision: draft.revision,
                },
            )
            .await
            .unwrap();
        drop(session);
        let request = MailSendRequest {
            host_id: review.host_id,
            connection_id: review.connection_id,
            operation_id: "operation-a".into(),
            revision: review.revision,
            digest: review.digest,
        };
        Self {
            runtime,
            connections,
            mailbox,
            request,
            folder,
        }
    }

    async fn send(&self, transport: Arc<FakeTransport>, request: MailSendRequest) -> MailSendTask {
        self.runtime.send_reviewed_compose(
            self.runtime
                .session(
                    &self.connections,
                    ServiceProvider::Gmail,
                    "synthetic-connection",
                )
                .await
                .unwrap(),
            transport,
            "draft-a".into(),
            request,
        )
    }

    async fn status(&self) -> MailSendStatus {
        self.runtime
            .compose_send_status(
                self.runtime.host().as_str().into(),
                self.mailbox.as_str().into(),
                self.request.operation_id.clone(),
            )
            .await
            .unwrap()
            .unwrap()
    }
}

#[derive(Clone, Copy)]
enum Mode {
    Accepted,
    Rejected,
    Uncertain,
    Panic,
}

struct FakeTransport {
    mode: Mode,
    count: AtomicUsize,
    bytes: Mutex<Vec<Vec<u8>>>,
    entered: Notify,
    release: Option<Notify>,
}

impl FakeTransport {
    fn new(mode: Mode, waiting: bool) -> Arc<Self> {
        Arc::new(Self {
            mode,
            count: AtomicUsize::new(0),
            bytes: Mutex::new(Vec::new()),
            entered: Notify::new(),
            release: waiting.then(Notify::new),
        })
    }
}

#[async_trait::async_trait]
impl MailSendTransport for FakeTransport {
    fn provider(&self) -> ServiceProvider {
        ServiceProvider::Gmail
    }
    fn mime_limit_bytes(&self) -> usize {
        MAIL_PREPARED_MIME_MAX_BYTES
    }
    async fn verified_sender(
        &self,
        session: &MailSession,
    ) -> Result<VerifiedMailSender, MailSendProblem> {
        VerifiedMailSender::from_profile(session, address())
            .map_err(|_| MailSendProblem::new(MailSendProblemKind::AccountChanged))
    }
    async fn send(&self, session: &MailSession, dispatch: MailDispatch) -> ProviderSendOutcome {
        dispatch.validate_session(session).unwrap();
        self.count.fetch_add(1, Ordering::SeqCst);
        self.bytes
            .lock()
            .unwrap()
            .push(dispatch.mime().bytes().to_vec());
        self.entered.notify_one();
        if let Some(release) = &self.release {
            release.notified().await;
        }
        match self.mode {
            Mode::Accepted => ProviderSendOutcome::Accepted(ProviderAcceptance {
                message_id: None,
                thread_id: None,
                request_id: None,
            }),
            Mode::Rejected => ProviderSendOutcome::Rejected(MailSendProblem {
                kind: MailSendProblemKind::RateLimited,
                retry_after_seconds: Some(3),
            }),
            Mode::Uncertain => ProviderSendOutcome::Uncertain(MailSendProblem::new(
                MailSendProblemKind::Unconfirmed,
            )),
            Mode::Panic => panic!("synthetic adapter panic"),
        }
    }
}

#[tokio::test]
async fn accepted_without_provider_ids_is_durable_and_same_operation_never_sends_twice() {
    let fixture = Fixture::new().await;
    let transport = FakeTransport::new(Mode::Accepted, false);
    let status = fixture
        .send(transport.clone(), fixture.request.clone())
        .await
        .wait()
        .await
        .unwrap();
    assert_eq!(status.state, MailDispatchState::Accepted);
    assert!(status.provider_message_id.is_none());
    assert!(status.provider_thread_id.is_none());
    assert!(status.problem.is_none());
    let repeated = fixture
        .send(transport.clone(), fixture.request.clone())
        .await
        .wait()
        .await
        .unwrap();
    assert_eq!(repeated.state, MailDispatchState::Accepted);
    assert_eq!(transport.count.load(Ordering::SeqCst), 1);
    assert!(fixture
        .runtime
        .compose_draft(
            fixture.runtime.host().as_str().into(),
            fixture.mailbox.as_str().into(),
            "draft-a".into()
        )
        .await
        .unwrap()
        .is_none());
    let operation = fixture
        .runtime
        .send_operation(
            fixture.request.operation_id.clone(),
            fixture.runtime.host().clone(),
            fixture.mailbox.clone(),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(operation
        .receipt
        .as_deref()
        .unwrap()
        .contains("\"delivery\":\"unknown\""));
}

#[tokio::test]
async fn dropped_waiter_does_not_abort_send_and_newer_edits_survive_acceptance() {
    let fixture = Fixture::new().await;
    let transport = FakeTransport::new(Mode::Accepted, true);
    let task = fixture
        .send(transport.clone(), fixture.request.clone())
        .await;
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        transport.entered.notified(),
    )
    .await
    .unwrap();
    drop(task);
    let mut draft = fixture
        .runtime
        .compose_draft(
            fixture.runtime.host().as_str().into(),
            fixture.mailbox.as_str().into(),
            "draft-a".into(),
        )
        .await
        .unwrap()
        .unwrap();
    draft.content.body = "Newer unsent edits".into();
    fixture
        .runtime
        .compose_save(
            fixture.mailbox.as_str().into(),
            "draft-a".into(),
            MailDraftSave {
                host_id: fixture.runtime.host().as_str().into(),
                expected_revision: draft.revision,
                content: draft.content,
            },
        )
        .await
        .unwrap();
    let unused = fixture
        .runtime
        .compose_upload(
            fixture.runtime.host().as_str().into(),
            fixture.mailbox.as_str().into(),
            "draft-a".into(),
            crate::mail::DraftAttachmentUpload {
                client_id: "newer-unused-file".into(),
                file_name: "unused.bin".into(),
                bytes: b"retained while pending".to_vec(),
            },
        )
        .await
        .unwrap();
    assert!(fixture
        .runtime
        .compose_remove_upload(
            fixture.runtime.host().as_str().into(),
            fixture.mailbox.as_str().into(),
            "draft-a".into(),
            unused.id.clone()
        )
        .await
        .is_err());
    transport.release.as_ref().unwrap().notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if fixture.status().await.state == MailDispatchState::Accepted {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let newer = fixture
        .runtime
        .compose_draft(
            fixture.runtime.host().as_str().into(),
            fixture.mailbox.as_str().into(),
            "draft-a".into(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(newer.content.body, "Newer unsent edits");
    fixture
        .runtime
        .compose_remove_upload(
            fixture.runtime.host().as_str().into(),
            fixture.mailbox.as_str().into(),
            "draft-a".into(),
            unused.id,
        )
        .await
        .unwrap();
    let bytes = transport.bytes.lock().unwrap().clone();
    assert_eq!(bytes.len(), 1);
    let parsed = mail_parser::MessageParser::default()
        .parse(&bytes[0])
        .unwrap();
    assert!(parsed
        .body_text(0)
        .unwrap()
        .contains("Exact reviewed body 東京"));
    assert!(!parsed.body_text(0).unwrap().contains("Newer unsent edits"));
}

#[tokio::test]
async fn uncertain_or_panicked_provider_attempt_is_saved_and_never_automatically_replayed() {
    for mode in [Mode::Uncertain, Mode::Panic] {
        let fixture = Fixture::new().await;
        let transport = FakeTransport::new(mode, false);
        assert!(fixture
            .send(transport.clone(), fixture.request.clone())
            .await
            .wait()
            .await
            .is_err());
        let status = fixture.status().await;
        assert_eq!(status.state, MailDispatchState::Uncertain);
        assert_eq!(
            status.problem.unwrap().kind,
            MailSendProblemKind::Unconfirmed
        );
        let repeated = fixture
            .send(transport.clone(), fixture.request.clone())
            .await
            .wait()
            .await
            .unwrap();
        assert_eq!(repeated.state, MailDispatchState::Uncertain);
        assert_eq!(transport.count.load(Ordering::SeqCst), 1);
        let session = fixture
            .runtime
            .session(
                &fixture.connections,
                ServiceProvider::Gmail,
                "synthetic-connection",
            )
            .await
            .unwrap();
        let sender = VerifiedMailSender::from_profile(&session, address()).unwrap();
        assert!(matches!(
            session
                .review_compose(
                    &sender,
                    "draft-a".into(),
                    MailReviewRequest {
                        host_id: fixture.runtime.host().as_str().into(),
                        connection_id: "synthetic-connection".into(),
                        expected_revision: fixture.request.revision
                    }
                )
                .await,
            Err(ComposeError::SendPending)
        ));
        drop(session);
    }
}

#[tokio::test]
async fn explicit_rejection_retains_problem_and_requires_another_reviewed_operation_to_retry() {
    let fixture = Fixture::new().await;
    let transport = FakeTransport::new(Mode::Rejected, false);
    assert!(fixture
        .send(transport.clone(), fixture.request.clone())
        .await
        .wait()
        .await
        .is_err());
    let status = fixture.status().await;
    assert_eq!(status.state, MailDispatchState::Rejected);
    assert_eq!(status.problem.unwrap().retry_after_seconds, Some(3));
    assert_eq!(
        fixture
            .send(transport.clone(), fixture.request.clone())
            .await
            .wait()
            .await
            .unwrap()
            .state,
        MailDispatchState::Rejected
    );
    assert_eq!(transport.count.load(Ordering::SeqCst), 1);
    let session = fixture
        .runtime
        .session(
            &fixture.connections,
            ServiceProvider::Gmail,
            "synthetic-connection",
        )
        .await
        .unwrap();
    let sender = VerifiedMailSender::from_profile(&session, address()).unwrap();
    let review = session
        .review_compose(
            &sender,
            "draft-a".into(),
            MailReviewRequest {
                host_id: fixture.runtime.host().as_str().into(),
                connection_id: "synthetic-connection".into(),
                expected_revision: fixture.request.revision,
            },
        )
        .await
        .unwrap();
    drop(session);
    let mut request = fixture.request.clone();
    request.operation_id = "operation-after-explicit-review".into();
    request.revision = review.revision;
    request.digest = review.digest;
    assert!(fixture
        .send(transport.clone(), request)
        .await
        .wait()
        .await
        .is_err());
    assert_eq!(transport.count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn edited_review_and_forged_operation_details_never_reach_the_provider() {
    let fixture = Fixture::new().await;
    let transport = FakeTransport::new(Mode::Accepted, false);
    for changed in ["digest", "revision", "host", "account"] {
        let mut request = fixture.request.clone();
        match changed {
            "digest" => request.digest = "f".repeat(64),
            "revision" => request.revision += 1,
            "host" => request.host_id = "foreign-host".into(),
            _ => request.connection_id = "foreign-account".into(),
        }
        assert!(fixture
            .send(transport.clone(), request)
            .await
            .wait()
            .await
            .is_err());
    }
    let mut draft = fixture
        .runtime
        .compose_draft(
            fixture.runtime.host().as_str().into(),
            fixture.mailbox.as_str().into(),
            "draft-a".into(),
        )
        .await
        .unwrap()
        .unwrap();
    draft.content.body = "Edited after review".into();
    fixture
        .runtime
        .compose_save(
            fixture.mailbox.as_str().into(),
            "draft-a".into(),
            MailDraftSave {
                host_id: fixture.runtime.host().as_str().into(),
                expected_revision: draft.revision,
                content: draft.content,
            },
        )
        .await
        .unwrap();
    assert!(fixture
        .send(transport.clone(), fixture.request.clone())
        .await
        .wait()
        .await
        .is_err());
    assert_eq!(transport.count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn prepared_content_binding_rejects_a_low_level_payload_edit_that_retains_preparation() {
    let fixture = Fixture::new().await;
    let reference = fixture.mailbox.draft("draft-a", None).unwrap();
    let (revision, payload) = fixture
        .runtime
        .cached_draft(reference.clone())
        .await
        .unwrap()
        .unwrap();
    let mut payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
    payload["content"]["body"] = serde_json::json!("changed outside typed editor");
    let revision = fixture
        .runtime
        .save_cached_draft(
            fixture.runtime.host().as_str().into(),
            reference,
            revision,
            payload.to_string(),
        )
        .await
        .unwrap();
    let session = fixture
        .runtime
        .session(
            &fixture.connections,
            ServiceProvider::Gmail,
            "synthetic-connection",
        )
        .await
        .unwrap();
    let sender = VerifiedMailSender::from_profile(&session, address()).unwrap();
    assert!(matches!(
        session
            .review_compose(
                &sender,
                "draft-a".into(),
                MailReviewRequest {
                    host_id: fixture.runtime.host().as_str().into(),
                    connection_id: "synthetic-connection".into(),
                    expected_revision: revision
                }
            )
            .await,
        Err(ComposeError::StaleReview)
    ));
    drop(session);
}

#[tokio::test]
async fn provider_mime_limit_fails_before_freezing_a_new_review_revision() {
    let fixture = Fixture::new().await;
    let mut draft = fixture
        .runtime
        .compose_draft(
            fixture.runtime.host().as_str().into(),
            fixture.mailbox.as_str().into(),
            "draft-a".into(),
        )
        .await
        .unwrap()
        .unwrap();
    draft
        .content
        .body
        .push_str("\nChanged before limited review");
    let draft = fixture
        .runtime
        .compose_save(
            fixture.mailbox.as_str().into(),
            "draft-a".into(),
            MailDraftSave {
                host_id: fixture.runtime.host().as_str().into(),
                expected_revision: draft.revision,
                content: draft.content,
            },
        )
        .await
        .unwrap();
    let session = fixture
        .runtime
        .session(
            &fixture.connections,
            ServiceProvider::Gmail,
            "synthetic-connection",
        )
        .await
        .unwrap();
    let sender = VerifiedMailSender::from_profile(&session, address()).unwrap();
    assert!(matches!(
        session
            .review_compose_with_limit(
                &sender,
                "draft-a".into(),
                MailReviewRequest {
                    host_id: fixture.runtime.host().as_str().into(),
                    connection_id: "synthetic-connection".into(),
                    expected_revision: draft.revision
                },
                10
            )
            .await,
        Err(ComposeError::Provider(MailSendProblem {
            kind: MailSendProblemKind::TooLarge,
            ..
        }))
    ));
    drop(session);
    let after = fixture
        .runtime
        .compose_draft(
            fixture.runtime.host().as_str().into(),
            fixture.mailbox.as_str().into(),
            "draft-a".into(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.revision, draft.revision);
    assert_eq!(after.content, draft.content);
}

#[tokio::test]
async fn uncertain_status_and_problem_restore_after_restart_without_replaying_frozen_bytes() {
    let fixture = Fixture::new().await;
    let transport = FakeTransport::new(Mode::Uncertain, false);
    assert!(fixture
        .send(transport.clone(), fixture.request.clone())
        .await
        .wait()
        .await
        .is_err());
    let Fixture {
        runtime,
        connections,
        mailbox,
        request,
        folder,
    } = fixture;
    drop(runtime);
    let runtime = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            match MailRuntime::open(folder.path().join("mail.sqlite3")).await {
                Ok(runtime) => break runtime,
                Err(MailError::AlreadyOwned) => tokio::task::yield_now().await,
                Err(error) => panic!("{error}"),
            }
        }
    })
    .await
    .unwrap();
    let status = runtime
        .compose_send_status(
            request.host_id.clone(),
            mailbox.as_str().into(),
            request.operation_id.clone(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status.state, MailDispatchState::Uncertain);
    assert_eq!(
        status.problem.unwrap().kind,
        MailSendProblemKind::Unconfirmed
    );
    let status = runtime
        .send_reviewed_compose(
            runtime
                .session(&connections, ServiceProvider::Gmail, "synthetic-connection")
                .await
                .unwrap(),
            transport.clone(),
            "draft-a".into(),
            request,
        )
        .wait()
        .await
        .unwrap();
    assert_eq!(status.state, MailDispatchState::Uncertain);
    assert_eq!(transport.count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cancelling_a_queued_review_prevents_the_provider_claim_and_replay() {
    let fixture = Fixture::new().await;
    let session = fixture
        .runtime
        .session(
            &fixture.connections,
            ServiceProvider::Gmail,
            "synthetic-connection",
        )
        .await
        .unwrap();
    let sender = VerifiedMailSender::from_profile(&session, address()).unwrap();
    session
        .queue_reviewed_compose(
            &sender,
            "draft-a".into(),
            fixture.request.clone(),
            MAIL_PREPARED_MIME_MAX_BYTES,
        )
        .await
        .unwrap();
    drop(session);
    let status = fixture
        .runtime
        .cancel_reviewed_compose(
            fixture.runtime.host().as_str().into(),
            fixture.mailbox.as_str().into(),
            fixture.request.operation_id.clone(),
        )
        .await
        .unwrap();
    assert_eq!(status.state, MailDispatchState::Cancelled);
    let transport = FakeTransport::new(Mode::Accepted, false);
    assert_eq!(
        fixture
            .send(transport.clone(), fixture.request.clone())
            .await
            .wait()
            .await
            .unwrap()
            .state,
        MailDispatchState::Cancelled
    );
    assert_eq!(transport.count.load(Ordering::SeqCst), 0);
}
