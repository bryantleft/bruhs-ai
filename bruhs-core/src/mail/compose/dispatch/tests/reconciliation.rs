use super::*;
use crate::mail::compose::reconciliation::{MailSentLookup, SentLookupOutcome, SentLookupTarget};

struct Lookup {
    raw: Option<Vec<u8>>,
    count: AtomicUsize,
}

#[async_trait::async_trait]
impl MailSentLookup for Lookup {
    fn provider(&self) -> ServiceProvider {
        ServiceProvider::Gmail
    }
    async fn find_sent(
        &self,
        session: &MailSession,
        target: Arc<SentLookupTarget>,
    ) -> Result<SentLookupOutcome, MailSendProblem> {
        target.validate_session(session)?;
        self.count.fetch_add(1, Ordering::SeqCst);
        let Some(raw) = self.raw.clone() else {
            return Ok(SentLookupOutcome::Unconfirmed);
        };
        let receipt = ProviderAcceptance {
            message_id: Some("matched-sent-id".into()),
            thread_id: Some("matched-sent-thread".into()),
            request_id: None,
        };
        Ok(
            match target.verify_sent_copy(session, receipt, raw).await? {
                Some(proof) => SentLookupOutcome::Matched(proof),
                None => SentLookupOutcome::Unconfirmed,
            },
        )
    }
}

async fn lease(f: &Fixture) -> MailSession {
    f.runtime
        .session(
            &f.connections,
            ServiceProvider::Gmail,
            "synthetic-connection",
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn sent_lookup_positive_copy_accepts_without_another_dispatch_and_is_idempotent() {
    let f = Fixture::new().await;
    let transport = FakeTransport::new(Mode::Uncertain, false);
    assert!(f
        .send(transport.clone(), f.request.clone())
        .await
        .wait()
        .await
        .is_err());
    let raw = transport.bytes.lock().unwrap()[0].clone();
    let lookup = Lookup {
        raw: Some(raw),
        count: AtomicUsize::new(0),
    };
    let session = lease(&f).await;
    for _ in 0..2 {
        let status = session
            .reconcile_uncertain_send(
                f.request.host_id.clone(),
                f.request.operation_id.clone(),
                &lookup,
            )
            .await
            .unwrap();
        assert_eq!(status.state, MailDispatchState::Accepted);
        assert_eq!(
            status.provider_message_id.as_deref(),
            Some("matched-sent-id")
        );
    }
    drop(session);
    assert_eq!(lookup.count.load(Ordering::SeqCst), 1);
    assert_eq!(transport.count.load(Ordering::SeqCst), 1);
    assert_eq!(
        f.send(transport.clone(), f.request.clone())
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
async fn sent_lookup_missing_or_mismatched_copy_keeps_uncertainty_and_its_receipt() {
    let f = Fixture::new().await;
    let transport = FakeTransport::new(Mode::Uncertain, false);
    assert!(f
        .send(transport.clone(), f.request.clone())
        .await
        .wait()
        .await
        .is_err());
    let mut changed = transport.bytes.lock().unwrap()[0].clone();
    changed.push(b'x');
    let before = serde_json::to_value(f.status().await).unwrap();
    let session = lease(&f).await;
    for raw in [None, Some(changed)] {
        let lookup = Lookup {
            raw,
            count: AtomicUsize::new(0),
        };
        let status = session
            .reconcile_uncertain_send(
                f.request.host_id.clone(),
                f.request.operation_id.clone(),
                &lookup,
            )
            .await
            .unwrap();
        assert_eq!(serde_json::to_value(status).unwrap(), before);
        assert_eq!(lookup.count.load(Ordering::SeqCst), 1);
    }
    drop(session);
    assert_eq!(transport.count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn sent_lookup_preserves_newer_text_and_refuses_a_foreign_host_without_reading_provider() {
    let f = Fixture::new().await;
    let transport = FakeTransport::new(Mode::Uncertain, false);
    assert!(f
        .send(transport.clone(), f.request.clone())
        .await
        .wait()
        .await
        .is_err());
    let raw = transport.bytes.lock().unwrap()[0].clone();
    let mut draft = f
        .runtime
        .compose_draft(
            f.request.host_id.clone(),
            f.mailbox.as_str().into(),
            "draft-a".into(),
        )
        .await
        .unwrap()
        .unwrap();
    draft.content.body = "Newer unsent text".into();
    let newer = f
        .runtime
        .compose_save(
            f.mailbox.as_str().into(),
            "draft-a".into(),
            MailDraftSave {
                host_id: f.request.host_id.clone(),
                expected_revision: draft.revision,
                content: draft.content,
            },
        )
        .await
        .unwrap();
    let lookup = Lookup {
        raw: Some(raw),
        count: AtomicUsize::new(0),
    };
    let session = lease(&f).await;
    assert!(session
        .reconcile_uncertain_send(
            "foreign-host".into(),
            f.request.operation_id.clone(),
            &lookup
        )
        .await
        .is_err());
    assert_eq!(lookup.count.load(Ordering::SeqCst), 0);
    let status = session
        .reconcile_uncertain_send(
            f.request.host_id.clone(),
            f.request.operation_id.clone(),
            &lookup,
        )
        .await
        .unwrap();
    drop(session);
    assert_eq!(status.state, MailDispatchState::Accepted);
    let kept = f
        .runtime
        .compose_draft(
            f.request.host_id.clone(),
            f.mailbox.as_str().into(),
            "draft-a".into(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(kept.revision, newer.revision);
    assert_eq!(kept.content.body, "Newer unsent text");
}

fn operation_identity(f: &Fixture) -> (String, String, String) {
    let connection = rusqlite::Connection::open(f.folder.path().join("mail.sqlite3")).unwrap();
    connection
        .query_row(
            "SELECT connection_id,payload_digest,payload FROM send_operations WHERE id=?1",
            [&f.request.operation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
}

#[tokio::test]
async fn same_account_new_broker_lease_can_only_reconcile_not_reauthorize_dispatch() {
    let f = Fixture::new().await;
    let transport = FakeTransport::new(Mode::Uncertain, false);
    assert!(f
        .send(transport.clone(), f.request.clone())
        .await
        .wait()
        .await
        .is_err());
    let before = operation_identity(&f);
    let raw = transport.bytes.lock().unwrap()[0].clone();
    let connections = Connections::test_connected_with_id(
        ServiceProvider::Gmail,
        "renewed-synthetic-token",
        "renewed-connection",
    )
    .await;
    let session = f
        .runtime
        .session(&connections, ServiceProvider::Gmail, "renewed-connection")
        .await
        .unwrap();
    assert_eq!(session.authorization().mailbox(), &f.mailbox);
    let sender = VerifiedMailSender::from_profile(&session, address()).unwrap();
    // Neither the old id nor a substituted id may obtain an ordinary claim.
    for request in [
        f.request.clone(),
        MailSendRequest {
            connection_id: "renewed-connection".into(),
            ..f.request.clone()
        },
    ] {
        assert!(matches!(
            session
                .claim_reviewed_compose(
                    &sender,
                    "draft-a".into(),
                    request.clone(),
                    MAIL_PREPARED_MIME_MAX_BYTES
                )
                .await,
            Err(ComposeError::StaleReview)
        ));
        assert!(matches!(
            session
                .queue_reviewed_compose(
                    &sender,
                    "draft-a".into(),
                    request,
                    MAIL_PREPARED_MIME_MAX_BYTES
                )
                .await,
            Err(ComposeError::StaleReview)
        ));
    }
    let lookup = Lookup {
        raw: Some(raw),
        count: AtomicUsize::new(0),
    };
    let status = session
        .reconcile_uncertain_send(
            f.request.host_id.clone(),
            f.request.operation_id.clone(),
            &lookup,
        )
        .await
        .unwrap();
    drop(session);
    assert_eq!(status.state, MailDispatchState::Accepted);
    assert_eq!(operation_identity(&f), before);
    assert_eq!(lookup.count.load(Ordering::SeqCst), 1);
    assert_eq!(transport.count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn definite_existing_states_never_trigger_a_sent_lookup() {
    for mode in [Mode::Accepted, Mode::Rejected] {
        let f = Fixture::new().await;
        let transport = FakeTransport::new(mode, false);
        let _ = f
            .send(transport.clone(), f.request.clone())
            .await
            .wait()
            .await;
        let before = serde_json::to_value(f.status().await).unwrap();
        let lookup = Lookup {
            raw: None,
            count: AtomicUsize::new(0),
        };
        let session = lease(&f).await;
        let after = session
            .reconcile_uncertain_send(
                f.request.host_id.clone(),
                f.request.operation_id.clone(),
                &lookup,
            )
            .await
            .unwrap();
        drop(session);
        assert_eq!(serde_json::to_value(after).unwrap(), before);
        assert_eq!(lookup.count.load(Ordering::SeqCst), 0);
        assert_eq!(transport.count.load(Ordering::SeqCst), 1);
    }
}
