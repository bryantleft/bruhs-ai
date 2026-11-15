use super::*;
use crate::AppState;

const BODY: &str =
    r#"{"to":["recipient@example.test"],"subject":"Synthetic","body":"PRIVATE-TEST-BODY"}"#;

async fn fixture() -> (tempfile::TempDir, MailRuntime, Connections) {
    let directory = tempfile::tempdir().unwrap();
    let runtime = MailRuntime::open(directory.path().join("mail.sqlite3"))
        .await
        .unwrap();
    let connections =
        Connections::test_connected(ServiceProvider::Gmail, "private-test-token").await;
    (directory, runtime, connections)
}

async fn session(runtime: &MailRuntime, connections: &Connections) -> MailSession {
    runtime
        .session(connections, ServiceProvider::Gmail, "synthetic-connection")
        .await
        .unwrap()
}

#[tokio::test]
async fn server_attachment_is_explicit_and_cannot_replace_the_canonical_runtime() {
    let state = AppState::builder()
        .connections(Connections::in_memory())
        .build();
    assert!(state.mail().is_none());
    let (_directory, runtime, _) = fixture().await;
    let host = runtime.host().clone();
    state.attach_mail(runtime.clone()).unwrap();
    assert!(matches!(
        state.attach_mail(runtime),
        Err(MailError::AlreadyAttached)
    ));
    assert_eq!(state.mail().unwrap().host(), &host);
}

#[tokio::test]
async fn private_store_is_exclusive_and_host_identity_survives_reopen() {
    let (directory, runtime, _) = fixture().await;
    let path = directory.path().join("mail.sqlite3");
    let host = runtime.host().clone();
    assert!(matches!(
        MailRuntime::open(&path).await,
        Err(MailError::AlreadyOwned)
    ));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let alias = directory.path().join("alias.sqlite3");
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        assert!(matches!(
            MailRuntime::open(&alias).await,
            Err(MailError::AlreadyOwned)
        ));
        let hard_link = directory.path().join("hard-link.sqlite3");
        std::fs::hard_link(&path, &hard_link).unwrap();
        assert!(matches!(
            MailRuntime::open(&hard_link).await,
            Err(MailError::InvalidPath)
        ));
        std::fs::remove_file(hard_link).unwrap();
    }
    drop(runtime);
    let reopened = MailRuntime::open(&path).await.unwrap();
    assert_eq!(reopened.host(), &host);
}

#[tokio::test]
async fn current_broker_access_is_required_and_disconnect_waits_for_the_session_lease() {
    let (_directory, runtime, connections) = fixture().await;
    assert!(matches!(
        runtime
            .session(&connections, ServiceProvider::Gmail, "forged-generation")
            .await,
        Err(MailError::Authorization(ConnectionError::SignIn))
    ));
    assert!(matches!(
        runtime
            .session(
                &connections,
                ServiceProvider::Google,
                "synthetic-connection"
            )
            .await,
        Err(MailError::Cache(CacheError::Identity(
            MailIdentityError::NotMailProvider
        )))
    ));
    let active = session(&runtime, &connections).await;
    let public = format!("{:?}", active.authorization());
    for private in ["private-test-token", "stable-subject", "you@example.test"] {
        assert!(!public.contains(private));
    }
    let other = connections.clone();
    let mut disconnect =
        tokio::spawn(async move { other.disconnect(ServiceProvider::Gmail).await });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), &mut disconnect)
            .await
            .is_err()
    );
    drop(active);
    tokio::time::timeout(std::time::Duration::from_secs(2), disconnect)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(
        runtime
            .session(&connections, ServiceProvider::Gmail, "synthetic-connection")
            .await,
        Err(MailError::Authorization(ConnectionError::SignIn))
    ));
}

#[tokio::test]
async fn draft_edits_and_files_remain_local_after_disconnect_and_reject_another_host() {
    let (_directory, runtime, connections) = fixture().await;
    let active = session(&runtime, &connections).await;
    let reference = active
        .authorization()
        .mailbox()
        .draft("local-files", None)
        .unwrap();
    drop(active);
    connections
        .disconnect(ServiceProvider::Gmail)
        .await
        .unwrap();
    let host = runtime.host().as_str().to_owned();
    assert!(matches!(
        runtime
            .save_cached_draft("another-host".into(), reference.clone(), 0, BODY.into())
            .await,
        Err(MailError::Cache(CacheError::HostChanged))
    ));
    assert!(runtime
        .cached_draft(reference.clone())
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        runtime
            .save_cached_draft(host.clone(), reference.clone(), 0, BODY.into())
            .await
            .unwrap(),
        1
    );
    let upload = || DraftAttachmentUpload {
        client_id: "upload-a".into(),
        file_name: "../private-東京.bin".into(),
        bytes: b"unsent local bytes".to_vec(),
    };
    assert!(matches!(
        runtime
            .store_draft_attachment("another-host".into(), reference.clone(), upload())
            .await,
        Err(MailError::Cache(CacheError::HostChanged))
    ));
    assert!(runtime
        .draft_attachments(host.clone(), reference.clone())
        .await
        .unwrap()
        .is_empty());
    let mut hints = runtime.subscribe();
    let metadata = runtime
        .store_draft_attachment(host.clone(), reference.clone(), upload())
        .await
        .unwrap();
    assert!(hints.has_changed().unwrap());
    drop(hints.borrow_and_update());
    assert_eq!(
        runtime
            .cached_draft(reference.clone())
            .await
            .unwrap()
            .unwrap()
            .0,
        1
    );
    assert_eq!(
        runtime
            .draft_attachment(host.clone(), reference.clone(), metadata.id.clone())
            .await
            .unwrap()
            .unwrap()
            .bytes,
        b"unsent local bytes"
    );
    assert!(matches!(
        runtime
            .draft_attachment(
                "another-host".into(),
                reference.clone(),
                metadata.id.clone()
            )
            .await,
        Err(MailError::Cache(CacheError::HostChanged))
    ));
    assert!(matches!(
        runtime
            .remove_draft_attachment(
                "another-host".into(),
                reference.clone(),
                metadata.id.clone()
            )
            .await,
        Err(MailError::Cache(CacheError::HostChanged))
    ));
    assert!(runtime
        .remove_draft_attachment(host, reference, metadata.id)
        .await
        .unwrap());
    assert!(hints.has_changed().unwrap());
}

#[tokio::test]
async fn local_draft_save_retries_keep_a_newer_edit_without_provider_access() {
    let (_directory, runtime, connections) = fixture().await;
    let active = session(&runtime, &connections).await;
    let reference = active
        .authorization()
        .mailbox()
        .draft("new-local-draft", None)
        .unwrap();
    drop(active);
    connections
        .disconnect(ServiceProvider::Gmail)
        .await
        .unwrap();
    let host = runtime.host().as_str().to_owned();
    assert_eq!(
        runtime
            .save_cached_draft(host.clone(), reference.clone(), 0, BODY.into())
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        runtime
            .save_cached_draft(host.clone(), reference.clone(), 0, BODY.into())
            .await
            .unwrap(),
        1,
        "An identical response-lost retry is idempotent"
    );
    let newer = r#"{"body":"Newer unsent local edit"}"#;
    assert_eq!(
        runtime
            .save_cached_draft(host.clone(), reference.clone(), 1, newer.into())
            .await
            .unwrap(),
        2
    );
    assert!(matches!(
        runtime
            .save_cached_draft(host, reference.clone(), 1, BODY.into())
            .await,
        Err(MailError::Cache(CacheError::RevisionConflict))
    ));
    assert_eq!(
        runtime.cached_draft(reference).await.unwrap(),
        Some((2, newer.into()))
    );
}

#[tokio::test]
async fn async_receipt_commit_preserves_a_newer_draft_and_never_claims_delivery() {
    let (directory, runtime, connections) = fixture().await;
    let active = session(&runtime, &connections).await;
    let mailbox = active.authorization().mailbox().clone();
    let draft = mailbox.draft("client-compose", None).unwrap();
    assert_eq!(
        active
            .save_draft(draft.clone(), 0, BODY.into())
            .await
            .unwrap(),
        1
    );
    let review = active.review_send(draft.clone(), 1).await.unwrap();
    active.queue_send("op-1".into(), review).await.unwrap();
    assert!(matches!(
        active.claim_send("op-1".into()).await.unwrap(),
        DispatchClaim::Start(_)
    ));
    let newer = BODY.replace("PRIVATE-TEST-BODY", "NEWER-PRIVATE-BODY");
    active
        .save_draft(draft.clone(), 1, newer.clone())
        .await
        .unwrap();
    active
        .checkpoint_provider_draft("op-1".into(), "provider-draft".into())
        .await
        .unwrap();
    let accepted = active
        .record_acceptance(
            "op-1".into(),
            ProviderAcceptance {
                message_id: Some("provider-message".into()),
                thread_id: Some("provider-thread".into()),
                request_id: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(accepted.state, SendState::Accepted);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(accepted.receipt.as_deref().unwrap()).unwrap()
            ["delivery"],
        "unknown"
    );
    assert_eq!(
        runtime.cached_draft(draft.clone()).await.unwrap(),
        Some((2, newer.clone()))
    );
    let host = runtime.host().clone();
    drop(active);
    drop(runtime);
    let reopened = MailRuntime::open(directory.path().join("mail.sqlite3"))
        .await
        .unwrap();
    assert_eq!(
        reopened.cached_draft(draft).await.unwrap(),
        Some((2, newer))
    );
    assert_eq!(
        reopened
            .send_operation("op-1".into(), host, mailbox)
            .await
            .unwrap()
            .unwrap()
            .state,
        SendState::Accepted
    );
}

#[tokio::test]
async fn exclusive_restart_marks_interrupted_dispatch_uncertain_without_replaying_it() {
    let (directory, runtime, connections) = fixture().await;
    let active = session(&runtime, &connections).await;
    let draft = active
        .authorization()
        .mailbox()
        .draft("client-compose", None)
        .unwrap();
    active
        .save_draft(draft.clone(), 0, BODY.into())
        .await
        .unwrap();
    let review = active.review_send(draft, 1).await.unwrap();
    active.queue_send("op-1".into(), review).await.unwrap();
    assert!(matches!(
        active.claim_send("op-1".into()).await.unwrap(),
        DispatchClaim::Start(_)
    ));
    drop(active);
    drop(runtime);
    let reopened = MailRuntime::open(directory.path().join("mail.sqlite3"))
        .await
        .unwrap();
    let resumed = session(&reopened, &connections).await;
    assert!(matches!(
        resumed.claim_send("op-1".into()).await.unwrap(),
        DispatchClaim::NeedsReconciliation
    ));
    drop(resumed);
}

#[tokio::test]
async fn concurrent_async_autosaves_preserve_one_winner_and_keep_failed_writes_out_of_change_hints()
{
    let (_directory, runtime, connections) = fixture().await;
    let active = session(&runtime, &connections).await;
    let draft = active
        .authorization()
        .mailbox()
        .draft("client-compose", None)
        .unwrap();
    let changes = runtime.subscribe();
    let previous = *changes.borrow();
    let other = BODY.replace("PRIVATE-TEST-BODY", "OTHER-PRIVATE-BODY");
    let (left, right) = tokio::join!(
        active.save_draft(draft.clone(), 0, BODY.into()),
        active.save_draft(draft.clone(), 0, other.clone()),
    );
    drop(active);
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
    assert!(matches!(
        left.as_ref().err().or(right.as_ref().err()).unwrap(),
        MailError::Cache(CacheError::RevisionConflict)
    ));
    let payload = if left.is_ok() { BODY.to_owned() } else { other };
    assert_eq!(
        runtime.cached_draft(draft).await.unwrap(),
        Some((1, payload))
    );
    assert_eq!(*changes.borrow(), previous + 1);
}

#[tokio::test]
async fn aborting_the_caller_keeps_the_blocking_write_owner_and_publishes_its_commit() {
    let (directory, runtime, connections) = fixture().await;
    let active = session(&runtime, &connections).await;
    let draft = active
        .authorization()
        .mailbox()
        .draft("client-compose", None)
        .unwrap();
    drop(active);
    let mut changes = runtime.subscribe();
    changes.borrow_and_update();
    let (started, waiting) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let work = runtime.clone();
    let saved = draft.clone();
    let caller = tokio::spawn(async move {
        work.call(true, move |cache| {
            started.send(()).map_err(|_| MailError::Worker)?;
            released
                .recv_timeout(std::time::Duration::from_secs(3))
                .map_err(|_| MailError::Worker)?;
            Ok(cache.save_mail_draft(&saved, 0, BODY)?)
        })
        .await
    });
    waiting.await.unwrap();
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    drop(runtime);
    let path = directory.path().join("mail.sqlite3");
    let second_owner = MailRuntime::open(&path).await;
    release.send(()).unwrap();
    assert!(matches!(second_owner, Err(MailError::AlreadyOwned)));
    tokio::time::timeout(std::time::Duration::from_secs(3), changes.changed())
        .await
        .unwrap()
        .unwrap();
    let reopened = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            match MailRuntime::open(&path).await {
                Err(MailError::AlreadyOwned) => tokio::task::yield_now().await,
                result => break result.unwrap(),
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(
        reopened.cached_draft(draft).await.unwrap(),
        Some((1, BODY.into()))
    );
}

#[test]
fn storage_diagnostics_redact_values_from_sqlite_errors() {
    let error = CacheError::Storage(rusqlite::Error::InvalidParameterName(
        "PRIVATE-TEST-BODY".into(),
    ));
    assert!(!format!("{error:?}").contains("PRIVATE-TEST-BODY"));
    assert!(!error.to_string().contains("PRIVATE-TEST-BODY"));
}
