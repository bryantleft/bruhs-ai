use super::super::identity::MailboxId;
use super::*;
use bruhs_models::connections::ServiceProvider;

const BODY: &str =
    r#"{"to":["recipient@example.test"],"subject":"Synthetic","body":"SYNTHETIC-PRIVATE-BODY"}"#;
fn now(offset: i64) -> EpochMs {
    EpochMs::new(1_700_000_000_000 + offset).unwrap()
}
fn auth(subject: &str, generation: &str) -> MailAuthorizationRef {
    let mailbox =
        MailboxId::from_broker_identity(ServiceProvider::Gmail, &"a".repeat(64), subject).unwrap();
    MailAuthorizationRef::from_broker_access(mailbox, generation).unwrap()
}
struct Fixture {
    cache: MailCache,
    folder: tempfile::TempDir,
    auth: MailAuthorizationRef,
    draft: MailDraftRef,
}
fn fixture() -> Fixture {
    let folder = tempfile::tempdir().unwrap();
    let mut cache = MailCache::open(
        &folder.path().join("mail.sqlite3"),
        &MailCacheHostId::from_host_record("host-a").unwrap(),
    )
    .unwrap();
    let auth = auth("account", "auth-a");
    cache.bind(&auth).unwrap();
    let draft = auth.mailbox().draft("client-draft-a", None).unwrap();
    cache.save_mail_draft(&draft, 0, BODY).unwrap();
    Fixture {
        cache,
        folder,
        auth,
        draft,
    }
}
fn queued(f: &mut Fixture) -> SendReview {
    let review = f.cache.review_send(&f.auth, &f.draft, 1).unwrap();
    f.cache
        .queue_send("op-a", &review, &f.auth, now(0))
        .unwrap();
    review
}
fn receipt() -> ProviderAcceptance {
    ProviderAcceptance {
        message_id: Some("provider-message".into()),
        thread_id: Some("provider-thread".into()),
        request_id: Some("request-id".into()),
    }
}

#[test]
fn reviewed_queue_is_idempotent_and_noop_autosave_keeps_its_revision() {
    let mut f = fixture();
    let review = f.cache.review_send(&f.auth, &f.draft, 1).unwrap();
    assert_eq!(f.cache.save_mail_draft(&f.draft,0,r#"{ "body":"SYNTHETIC-PRIVATE-BODY", "subject":"Synthetic", "to":["recipient@example.test"] }"#).unwrap(),1);
    let current = f.cache.review_send(&f.auth, &f.draft, 1).unwrap();
    assert_eq!(review.digest(), current.digest());
    assert_eq!(
        f.cache
            .queue_send("op-a", &review, &f.auth, now(0))
            .unwrap()
            .state,
        SendState::Queued
    );
    assert_eq!(
        f.cache
            .queue_send("op-a", &current, &f.auth, now(1))
            .unwrap()
            .state,
        SendState::Queued
    );
    let count: i64 = f
        .cache
        .connection
        .query_row("SELECT count(*) FROM send_operations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn an_operation_id_cannot_be_reused_for_changed_reviewed_content() {
    let mut f = fixture();
    let original = queued(&mut f);
    f.cache
        .save_mail_draft(
            &f.draft,
            1,
            r#"{"to":["other@example.test"],"body":"different"}"#,
        )
        .unwrap();
    let changed = f.cache.review_send(&f.auth, &f.draft, 2).unwrap();
    assert!(matches!(
        f.cache.queue_send("op-a", &changed, &f.auth, now(1)),
        Err(OperationError::OperationConflict)
    ));
    let record = f
        .cache
        .send_operation("op-a", f.cache.host(), f.auth.mailbox())
        .unwrap()
        .unwrap();
    assert_eq!(record.payload(), original.payload());
}

#[test]
fn another_operation_id_cannot_duplicate_pending_or_accepted_draft_content() {
    let mut f = fixture();
    let review = queued(&mut f);
    assert!(
        matches!(f.cache.queue_send("op-b",&review,&f.auth,now(1)),Err(OperationError::DuplicateDraftOperation { operation_id }) if operation_id=="op-a")
    );
    assert!(matches!(
        f.cache.claim_send("op-a", &f.auth, now(2)).unwrap(),
        DispatchClaim::Start(_)
    ));
    f.cache
        .record_send_acceptance("op-a", &f.auth, &receipt(), now(3))
        .unwrap();
    assert!(matches!(
        f.cache.queue_send("op-b", &review, &f.auth, now(4)),
        Err(OperationError::DuplicateDraftOperation { .. })
    ));
    let deliberate_new_draft = f
        .auth
        .mailbox()
        .draft("a-new-user-compose-action", None)
        .unwrap();
    f.cache
        .save_mail_draft(&deliberate_new_draft, 0, BODY)
        .unwrap();
    let second = f
        .cache
        .review_send(&f.auth, &deliberate_new_draft, 1)
        .unwrap();
    assert_eq!(
        f.cache
            .queue_send("op-b", &second, &f.auth, now(5))
            .unwrap()
            .state,
        SendState::Queued
    );
}

#[test]
fn edits_after_review_are_retained_and_refuse_queueing_the_old_snapshot() {
    let mut f = fixture();
    let review = f.cache.review_send(&f.auth, &f.draft, 1).unwrap();
    f.cache
        .save_mail_draft(&f.draft, 1, r#"{"body":"newer unsent edit"}"#)
        .unwrap();
    assert!(matches!(
        f.cache.queue_send("op-a", &review, &f.auth, now(1)),
        Err(OperationError::ReviewChanged)
    ));
    assert!(f
        .cache
        .send_operation("op-a", f.cache.host(), f.auth.mailbox())
        .unwrap()
        .is_none());
    assert!(f
        .cache
        .mail_draft(&f.draft)
        .unwrap()
        .unwrap()
        .1
        .contains("newer unsent edit"));
}

#[test]
fn edits_after_queue_block_dispatch_and_allow_a_new_review() {
    let mut f = fixture();
    queued(&mut f);
    f.cache
        .save_mail_draft(&f.draft, 1, r#"{"body":"newly reviewed version"}"#)
        .unwrap();
    assert!(matches!(
        f.cache.claim_send("op-a", &f.auth, now(1)).unwrap(),
        DispatchClaim::NeedsReview
    ));
    let current = f.cache.review_send(&f.auth, &f.draft, 2).unwrap();
    f.cache
        .queue_send("op-b", &current, &f.auth, now(2))
        .unwrap();
    assert!(matches!(
        f.cache.claim_send("op-b", &f.auth, now(3)).unwrap(),
        DispatchClaim::Start(_)
    ));
}

#[test]
fn a_second_worker_cannot_claim_an_already_dispatching_send() {
    let mut f = fixture();
    queued(&mut f);
    let mut second =
        MailCache::open(&f.folder.path().join("mail.sqlite3"), f.cache.host()).unwrap();
    assert!(matches!(
        f.cache.claim_send("op-a", &f.auth, now(1)).unwrap(),
        DispatchClaim::Start(_)
    ));
    assert!(matches!(
        second.claim_send("op-a", &f.auth, now(2)).unwrap(),
        DispatchClaim::InProgress
    ));
}

#[test]
fn a_stopped_worker_restarts_uncertain_and_cannot_automatically_resend() {
    let mut f = fixture();
    queued(&mut f);
    f.cache.claim_send("op-a", &f.auth, now(1)).unwrap();
    f.cache
        .checkpoint_provider_draft("op-a", &f.auth, "opaque-draft-id", now(2))
        .unwrap();
    drop(f.cache);
    let mut cache = MailCache::open(
        &f.folder.path().join("mail.sqlite3"),
        &MailCacheHostId::from_host_record("ignored-new-host").unwrap(),
    )
    .unwrap();
    assert_eq!(cache.recover_interrupted_dispatches(now(3)).unwrap(), 1);
    assert_eq!(cache.recover_interrupted_dispatches(now(4)).unwrap(), 0);
    assert!(matches!(
        cache.claim_send("op-a", &f.auth, now(5)).unwrap(),
        DispatchClaim::NeedsReconciliation
    ));
    assert!(matches!(
        cache.cancel_queued_send("op-a", &cache.host().clone(), f.auth.mailbox(), now(6)),
        Err(OperationError::InvalidTransition)
    ));
    let record = cache
        .send_operation("op-a", cache.host(), f.auth.mailbox())
        .unwrap()
        .unwrap();
    assert_eq!(record.provider_draft_id.as_deref(), Some("opaque-draft-id"));
    assert!(cache.mail_draft(&f.draft).unwrap().is_some());
}

#[test]
fn provider_acknowledgement_records_acceptance_with_delivery_unknown() {
    let mut f = fixture();
    let review = queued(&mut f);
    f.cache.claim_send("op-a", &f.auth, now(1)).unwrap();
    let ack = ProviderAcceptance {
        message_id: None,
        thread_id: None,
        request_id: Some("synthetic-202-request".into()),
    };
    let accepted = f
        .cache
        .record_send_acceptance("op-a", &f.auth, &ack, now(2))
        .unwrap();
    let data: Value = serde_json::from_str(accepted.receipt.as_deref().unwrap()).unwrap();
    assert_eq!(data["delivery"], "unknown");
    assert!(data.get("delivered").is_none());
    assert!(f.cache.mail_draft(&f.draft).unwrap().is_none());
    assert!(matches!(
        f.cache.save_mail_draft(&f.draft, 1, BODY),
        Err(CacheError::DraftConsumed)
    ));
    assert_eq!(
        f.cache
            .queue_send("op-a", &review, &f.auth, now(3))
            .unwrap()
            .state,
        SendState::Accepted
    );
    assert!(matches!(
        f.cache.claim_send("op-a", &f.auth, now(4)).unwrap(),
        DispatchClaim::AlreadyAccepted(_)
    ));
    f.cache
        .record_send_acceptance("op-a", &f.auth, &ack, now(5))
        .unwrap();
}

#[test]
fn acceptance_of_the_reviewed_revision_preserves_a_newer_draft_edit() {
    let mut f = fixture();
    queued(&mut f);
    f.cache.claim_send("op-a", &f.auth, now(1)).unwrap();
    f.cache
        .save_mail_draft(
            &f.draft,
            1,
            r#"{"body":"Newer edit while the prior revision was sending"}"#,
        )
        .unwrap();
    f.cache
        .record_send_acceptance("op-a", &f.auth, &receipt(), now(2))
        .unwrap();
    assert_eq!(
        f.cache.mail_draft(&f.draft).unwrap().unwrap(),
        (
            2,
            r#"{"body":"Newer edit while the prior revision was sending"}"#.into()
        )
    );
}

#[test]
fn host_account_and_reauthorization_fences_refuse_dispatch() {
    let mut f = fixture();
    let review = queued(&mut f);
    let folder = tempfile::tempdir().unwrap();
    let mut foreign = MailCache::open(
        &folder.path().join("other.sqlite3"),
        &MailCacheHostId::from_host_record("host-b").unwrap(),
    )
    .unwrap();
    foreign.bind(&f.auth).unwrap();
    assert!(matches!(
        foreign.queue_send("op-a", &review, &f.auth, now(1)),
        Err(OperationError::HostChanged)
    ));
    let different = auth("different-account", "different-auth");
    f.cache.bind(&different).unwrap();
    assert!(matches!(
        f.cache.claim_send("op-a", &different, now(2)),
        Err(OperationError::Cache(CacheError::Identity(
            MailIdentityError::MailboxChanged
        )))
    ));
    let reconnected = auth("account", "auth-new");
    f.cache.bind(&reconnected).unwrap();
    assert!(matches!(
        f.cache.claim_send("op-a", &f.auth, now(3)),
        Err(OperationError::Cache(CacheError::AuthorizationChanged))
    ));
    assert!(matches!(
        f.cache.claim_send("op-a", &reconnected, now(4)),
        Err(OperationError::Cache(CacheError::Identity(
            MailIdentityError::AuthorizationChanged
        )))
    ));
    assert_eq!(
        f.cache
            .send_operation("op-a", f.cache.host(), f.auth.mailbox())
            .unwrap()
            .unwrap()
            .state,
        SendState::Queued
    );
}

#[test]
fn provider_draft_checkpoint_is_immutable_and_rejects_secret_upload_urls() {
    let mut f = fixture();
    queued(&mut f);
    f.cache.claim_send("op-a", &f.auth, now(1)).unwrap();
    f.cache
        .checkpoint_provider_draft("op-a", &f.auth, "opaque-draft", now(2))
        .unwrap();
    f.cache
        .checkpoint_provider_draft("op-a", &f.auth, "opaque-draft", now(3))
        .unwrap();
    assert!(matches!(
        f.cache
            .checkpoint_provider_draft("op-a", &f.auth, "another-draft", now(4)),
        Err(OperationError::OperationConflict)
    ));
    assert!(matches!(
        f.cache.checkpoint_provider_draft(
            "op-a",
            &f.auth,
            "https://upload.example.test/?token=synthetic",
            now(5)
        ),
        Err(OperationError::InvalidReview)
    ));
    let record = f
        .cache
        .send_operation("op-a", f.cache.host(), f.auth.mailbox())
        .unwrap()
        .unwrap();
    assert_eq!(record.provider_draft_id.as_deref(), Some("opaque-draft"));
}

#[test]
fn failed_acceptance_commit_keeps_draft_and_requires_reconciliation() {
    let mut f = fixture();
    queued(&mut f);
    f.cache.claim_send("op-a", &f.auth, now(1)).unwrap();
    f.cache.connection.execute_batch("CREATE TRIGGER fail_consume BEFORE UPDATE OF state ON drafts BEGIN SELECT RAISE(ABORT,'synthetic receipt transaction failure'); END;").unwrap();
    let error = f
        .cache
        .record_send_acceptance("op-a", &f.auth, &receipt(), now(2))
        .unwrap_err();
    assert!(matches!(error, OperationError::AcceptanceNotSaved(_)));
    assert!(error.to_string().contains("provider accepted"));
    let row = f
        .cache
        .send_operation("op-a", f.cache.host(), f.auth.mailbox())
        .unwrap()
        .unwrap();
    assert_eq!(row.state, SendState::Dispatching);
    assert!(row.receipt.is_none());
    assert!(f.cache.mail_draft(&f.draft).unwrap().is_some());
    f.cache
        .connection
        .execute_batch("DROP TRIGGER fail_consume")
        .unwrap();
    drop(f.cache);
    let mut recovered = MailCache::open(
        &f.folder.path().join("mail.sqlite3"),
        &MailCacheHostId::from_host_record("ignored").unwrap(),
    )
    .unwrap();
    recovered.recover_interrupted_dispatches(now(3)).unwrap();
    assert!(matches!(
        recovered.claim_send("op-a", &f.auth, now(4)).unwrap(),
        DispatchClaim::NeedsReconciliation
    ));
    assert_eq!(
        recovered
            .record_send_acceptance("op-a", &f.auth, &receipt(), now(5))
            .unwrap()
            .state,
        SendState::Accepted
    );
}

#[test]
fn cancellation_and_definite_rejection_do_not_discard_the_draft() {
    let mut f = fixture();
    let review = queued(&mut f);
    f.cache
        .cancel_queued_send("op-a", &f.cache.host().clone(), f.auth.mailbox(), now(1))
        .unwrap();
    assert!(matches!(
        f.cache.claim_send("op-a", &f.auth, now(2)).unwrap(),
        DispatchClaim::NotQueued
    ));
    f.cache
        .queue_send("op-b", &review, &f.auth, now(3))
        .unwrap();
    f.cache.claim_send("op-b", &f.auth, now(4)).unwrap();
    assert!(matches!(
        f.cache
            .cancel_queued_send("op-b", &f.cache.host().clone(), f.auth.mailbox(), now(5)),
        Err(OperationError::InvalidTransition)
    ));
    f.cache
        .record_definite_send_rejection("op-b", &f.auth, now(6))
        .unwrap();
    assert!(matches!(
        f.cache.claim_send("op-b", &f.auth, now(7)).unwrap(),
        DispatchClaim::NotQueued
    ));
    assert!(f.cache.mail_draft(&f.draft).unwrap().is_some());
    assert_eq!(
        f.cache
            .queue_send("op-c", &review, &f.auth, now(8))
            .unwrap()
            .state,
        SendState::Queued
    );
}

#[test]
fn debug_output_keeps_the_reviewed_body_out_of_diagnostics() {
    let mut f = fixture();
    let review = queued(&mut f);
    let operation = f
        .cache
        .send_operation("op-a", f.cache.host(), f.auth.mailbox())
        .unwrap()
        .unwrap();
    assert!(!format!("{review:?}").contains("SYNTHETIC-PRIVATE-BODY"));
    assert!(!format!("{operation:?}").contains("SYNTHETIC-PRIVATE-BODY"));
    assert!(review.payload().contains("SYNTHETIC-PRIVATE-BODY"));
}

#[test]
fn concurrent_clients_cannot_create_two_sends_for_one_reviewed_draft() {
    let mut f = fixture();
    let review = f.cache.review_send(&f.auth, &f.draft, 1).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let workers: Vec<_> = (0..2)
        .map(|index| {
            let path = f.folder.path().join("mail.sqlite3");
            let host = f.cache.host().clone();
            let auth = f.auth.clone();
            let review = review.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut cache = MailCache::open(&path, &host).unwrap();
                barrier.wait();
                cache.queue_send(&format!("concurrent-{index}"), &review, &auth, now(1))
            })
        })
        .collect();
    let results: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(OperationError::DuplicateDraftOperation { .. })))
            .count(),
        1
    );
    let count: i64 = f
        .cache
        .connection
        .query_row("SELECT count(*) FROM send_operations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
}
