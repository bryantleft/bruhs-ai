use super::*;

async fn session(f: &Fixture) -> MailSession {
    f.runtime
        .session(
            &f.connections,
            ServiceProvider::Gmail,
            "synthetic-connection",
        )
        .await
        .unwrap()
}

async fn reject(f: &Fixture) -> Result<MailSendStatus, ComposeError> {
    f.runtime
        .reject_unqueued_compose(
            f.mailbox.as_str().into(),
            "draft-a".into(),
            f.request.clone(),
            MailSendProblem::new(MailSendProblemKind::SignIn),
        )
        .await
}

struct DeniedProfile;
#[async_trait::async_trait]
impl MailSendTransport for DeniedProfile {
    fn provider(&self) -> ServiceProvider {
        ServiceProvider::Gmail
    }
    fn mime_limit_bytes(&self) -> usize {
        MAIL_PREPARED_MIME_MAX_BYTES
    }
    async fn verified_sender(
        &self,
        _: &MailSession,
    ) -> Result<VerifiedMailSender, MailSendProblem> {
        Err(MailSendProblem::new(MailSendProblemKind::SignIn))
    }
    async fn send(&self, _: &MailSession, _: MailDispatch) -> ProviderSendOutcome {
        panic!("a denied sender profile must not obtain a dispatch")
    }
}

#[tokio::test]
async fn prequeue_different_operation_cannot_hide_an_uncertain_send() {
    let f = Fixture::new().await;
    let transport = FakeTransport::new(Mode::Uncertain, false);
    assert!(f
        .send(transport.clone(), f.request.clone())
        .await
        .wait()
        .await
        .is_err());
    let before = f.status().await;
    let request = MailSendRequest {
        operation_id: "second-operation".into(),
        ..f.request.clone()
    };
    let result = f
        .runtime
        .reject_unqueued_compose(
            f.mailbox.as_str().into(),
            "draft-a".into(),
            request,
            MailSendProblem::new(MailSendProblemKind::SignIn),
        )
        .await;
    let draft = f
        .runtime
        .compose_draft(
            f.request.host_id.clone(),
            f.mailbox.as_str().into(),
            "draft-a".into(),
        )
        .await
        .unwrap()
        .unwrap();
    let restored = draft.latest_send.unwrap();
    assert_eq!(
        restored.state,
        MailDispatchState::Uncertain,
        "a later rejected id must not hide the actual uncertain attempt"
    );
    assert_eq!(restored.operation_id, before.operation_id);
    assert!(result.is_err());
    assert!(f
        .runtime
        .compose_send_status(
            f.request.host_id.clone(),
            f.mailbox.as_str().into(),
            "second-operation".into()
        )
        .await
        .unwrap()
        .is_none());
    assert_eq!(transport.count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn prequeue_profile_failure_is_durable_and_original_operation_never_replays() {
    let f = Fixture::new().await;
    let result = f
        .runtime
        .send_reviewed_compose(
            session(&f).await,
            Arc::new(DeniedProfile),
            "draft-a".into(),
            f.request.clone(),
        )
        .wait()
        .await;
    assert!(matches!(
        result,
        Err(ComposeError::Provider(MailSendProblem {
            kind: MailSendProblemKind::SignIn,
            ..
        }))
    ));
    assert_eq!(f.status().await.state, MailDispatchState::Rejected);
    assert_eq!(
        f.status().await.problem.unwrap().kind,
        MailSendProblemKind::SignIn
    );
    let connection = rusqlite::Connection::open(f.folder.path().join("mail.sqlite3")).unwrap();
    let stored: (String, String) = connection
        .query_row(
            "SELECT state,receipt FROM send_operations WHERE id=?1",
            [&f.request.operation_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(stored.0, "rejected");
    assert!(!stored.1.contains("Exact reviewed body"));
    let transport = FakeTransport::new(Mode::Accepted, false);
    assert_eq!(
        f.send(transport.clone(), f.request.clone())
            .await
            .wait()
            .await
            .unwrap()
            .state,
        MailDispatchState::Rejected
    );
    assert_eq!(transport.count.load(Ordering::SeqCst), 0);
    let lease = session(&f).await;
    let sender = VerifiedMailSender::from_profile(&lease, address()).unwrap();
    let review = lease
        .review_compose(
            &sender,
            "draft-a".into(),
            MailReviewRequest {
                host_id: f.request.host_id.clone(),
                connection_id: f.request.connection_id.clone(),
                expected_revision: f.request.revision,
            },
        )
        .await
        .unwrap();
    drop(lease);
    let request = MailSendRequest {
        operation_id: "explicit-new-operation".into(),
        revision: review.revision,
        digest: review.digest,
        ..f.request.clone()
    };
    assert_eq!(
        f.send(transport.clone(), request)
            .await
            .wait()
            .await
            .unwrap()
            .state,
        MailDispatchState::Accepted
    );
    assert_eq!(transport.count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn prequeue_rejection_and_low_level_queue_share_the_same_payload_fence() {
    let f = Fixture::new().await;
    let lease = session(&f).await;
    let auth = lease.authorization().clone();
    let client = "draft-a";
    let mailbox = f.mailbox.clone();
    let review = f
        .runtime
        .compose_call(false, move |cache| {
            let snapshot = cache.compose_snapshot(mailbox.as_str(), client)?;
            Ok(cache
                .review_send(&auth, &snapshot.reference, snapshot.revision)
                .map_err(MailError::from)?)
        })
        .await
        .unwrap();
    assert_eq!(reject(&f).await.unwrap().state, MailDispatchState::Rejected);
    let auth = lease.authorization().clone();
    drop(lease);
    let id = f.request.operation_id.clone();
    f.runtime
        .compose_call(true, move |cache| {
            let stored = cache
                .queue_send(&id, &review, &auth, EpochMs::now())
                .map_err(MailError::from)?;
            assert_eq!(stored.state, SendState::Rejected);
            assert_eq!(stored.payload(), review.payload());
            assert!(matches!(
                cache
                    .claim_send(&id, &auth, EpochMs::now())
                    .map_err(MailError::from)?,
                DispatchClaim::NotQueued
            ));
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn prequeue_negative_fence_never_overwrites_queued_dispatching_accepted_or_uncertain() {
    for target in [
        MailDispatchState::Queued,
        MailDispatchState::Dispatching,
        MailDispatchState::Accepted,
        MailDispatchState::Uncertain,
    ] {
        let f = Fixture::new().await;
        if target == MailDispatchState::Accepted || target == MailDispatchState::Uncertain {
            let transport = FakeTransport::new(
                if target == MailDispatchState::Accepted {
                    Mode::Accepted
                } else {
                    Mode::Uncertain
                },
                false,
            );
            let _result = f.send(transport, f.request.clone()).await.wait().await;
        } else {
            let lease = session(&f).await;
            let sender = VerifiedMailSender::from_profile(&lease, address()).unwrap();
            lease
                .queue_reviewed_compose(
                    &sender,
                    "draft-a".into(),
                    f.request.clone(),
                    MAIL_PREPARED_MIME_MAX_BYTES,
                )
                .await
                .unwrap();
            if target == MailDispatchState::Dispatching {
                assert!(lease
                    .claim_reviewed_compose(
                        &sender,
                        "draft-a".into(),
                        f.request.clone(),
                        MAIL_PREPARED_MIME_MAX_BYTES
                    )
                    .await
                    .unwrap()
                    .is_some());
            }
            drop(lease);
        }
        let before = f.status().await;
        let recovered = reject(&f).await.unwrap();
        assert_eq!(
            serde_json::to_value(&recovered).unwrap(),
            serde_json::to_value(&before).unwrap()
        );
        assert_eq!(recovered.state, target);
        assert_eq!(
            serde_json::to_value(f.status().await).unwrap(),
            serde_json::to_value(before).unwrap()
        );
    }
}

#[tokio::test]
async fn prequeue_invalid_bindings_and_storage_failure_do_not_create_a_rejection() {
    let f = Fixture::new().await;
    for field in ["host", "connection", "revision", "digest", "operation"] {
        let mut request = f.request.clone();
        match field {
            "host" => request.host_id = "foreign-host".into(),
            "connection" => request.connection_id = "foreign-connection".into(),
            "revision" => request.revision += 1,
            "digest" => request.digest = "0".repeat(64),
            "operation" => request.operation_id = "invalid/operation".into(),
            _ => unreachable!(),
        }
        assert!(
            f.runtime
                .reject_unqueued_compose(
                    f.mailbox.as_str().into(),
                    "draft-a".into(),
                    request,
                    MailSendProblem::new(MailSendProblemKind::SignIn)
                )
                .await
                .is_err(),
            "{field}"
        );
    }
    for (mailbox, client) in [
        ("unknown-mailbox", "draft-a"),
        (f.mailbox.as_str(), "foreign-client"),
        (f.mailbox.as_str(), "invalid/client"),
    ] {
        assert!(f
            .runtime
            .reject_unqueued_compose(
                mailbox.into(),
                client.into(),
                f.request.clone(),
                MailSendProblem::new(MailSendProblemKind::SignIn)
            )
            .await
            .is_err());
    }
    assert!(f
        .runtime
        .compose_send_status(
            f.request.host_id.clone(),
            f.mailbox.as_str().into(),
            f.request.operation_id.clone()
        )
        .await
        .unwrap()
        .is_none());
    let connection = rusqlite::Connection::open(f.folder.path().join("mail.sqlite3")).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_negative_fence BEFORE INSERT ON send_operations BEGIN SELECT RAISE(ABORT,'synthetic storage failure'); END;").unwrap();
    assert!(reject(&f).await.is_err());
    assert!(f
        .runtime
        .compose_send_status(
            f.request.host_id.clone(),
            f.mailbox.as_str().into(),
            f.request.operation_id.clone()
        )
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn prequeue_concurrent_queue_and_rejection_have_one_durable_winner() {
    let f = Fixture::new().await;
    let lease = session(&f).await;
    let sender = VerifiedMailSender::from_profile(&lease, address()).unwrap();
    let (rejected, queued) = tokio::join!(
        reject(&f),
        lease.queue_reviewed_compose(
            &sender,
            "draft-a".into(),
            f.request.clone(),
            MAIL_PREPARED_MIME_MAX_BYTES
        )
    );
    let rejected = rejected.unwrap();
    let queued = queued.unwrap();
    assert_eq!(rejected.state, queued.state);
    assert!(matches!(
        queued.state,
        MailDispatchState::Queued | MailDispatchState::Rejected
    ));
    assert_eq!(f.status().await.state, queued.state);
    let claim = lease
        .claim_reviewed_compose(
            &sender,
            "draft-a".into(),
            f.request.clone(),
            MAIL_PREPARED_MIME_MAX_BYTES,
        )
        .await
        .unwrap();
    drop(lease);
    assert_eq!(claim.is_some(), queued.state == MailDispatchState::Queued);
}

#[tokio::test]
async fn prequeue_different_operation_preserves_queued_and_dispatching_draft_status() {
    for target in [MailDispatchState::Queued, MailDispatchState::Dispatching] {
        let f = Fixture::new().await;
        let lease = session(&f).await;
        let sender = VerifiedMailSender::from_profile(&lease, address()).unwrap();
        lease
            .queue_reviewed_compose(
                &sender,
                "draft-a".into(),
                f.request.clone(),
                MAIL_PREPARED_MIME_MAX_BYTES,
            )
            .await
            .unwrap();
        if target == MailDispatchState::Dispatching {
            assert!(lease
                .claim_reviewed_compose(
                    &sender,
                    "draft-a".into(),
                    f.request.clone(),
                    MAIL_PREPARED_MIME_MAX_BYTES
                )
                .await
                .unwrap()
                .is_some());
        }
        drop(lease);
        let request = MailSendRequest {
            operation_id: "second-operation".into(),
            ..f.request.clone()
        };
        assert!(f
            .runtime
            .reject_unqueued_compose(
                f.mailbox.as_str().into(),
                "draft-a".into(),
                request,
                MailSendProblem::new(MailSendProblemKind::Permission)
            )
            .await
            .is_err());
        let draft = f
            .runtime
            .compose_draft(
                f.request.host_id.clone(),
                f.mailbox.as_str().into(),
                "draft-a".into(),
            )
            .await
            .unwrap()
            .unwrap();
        let status = draft.latest_send.unwrap();
        assert_eq!(status.operation_id, f.request.operation_id);
        assert_eq!(status.state, target);
        assert!(f
            .runtime
            .compose_send_status(
                f.request.host_id.clone(),
                f.mailbox.as_str().into(),
                "second-operation".into()
            )
            .await
            .unwrap()
            .is_none());
    }
}

#[tokio::test]
async fn prequeue_existing_rejection_is_immutable_and_foreign_bindings_cannot_replace_it() {
    let f = Fixture::new().await;
    let before = serde_json::to_value(reject(&f).await.unwrap()).unwrap();
    for (client, field) in [
        ("foreign-client", "none"),
        ("draft-a", "digest"),
        ("draft-a", "connection"),
        ("draft-a", "revision"),
    ] {
        let mut request = f.request.clone();
        match field {
            "digest" => request.digest = "0".repeat(64),
            "connection" => request.connection_id = "foreign-connection".into(),
            "revision" => request.revision += 1,
            _ => (),
        }
        assert!(f
            .runtime
            .reject_unqueued_compose(
                f.mailbox.as_str().into(),
                client.into(),
                request,
                MailSendProblem::new(MailSendProblemKind::Permission)
            )
            .await
            .is_err());
        assert_eq!(serde_json::to_value(f.status().await).unwrap(), before);
    }
    let repeated = f
        .runtime
        .reject_unqueued_compose(
            f.mailbox.as_str().into(),
            "draft-a".into(),
            f.request.clone(),
            MailSendProblem::new(MailSendProblemKind::Permission),
        )
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(repeated).unwrap(), before);
}
