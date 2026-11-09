use super::*;

fn queue(dir: &tempfile::TempDir) -> RecoveryQueue {
    RecoveryQueue::new(Some(dir.path().join("recovery.sqlite3")))
}

#[tokio::test]
async fn committed_completion_survives_reopen_and_ack_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let original = queue(&dir);
    original.enqueue("synthetic".into()).await.unwrap();
    let receipts = original.page(0).await.unwrap();
    drop(original);
    let reopened = queue(&dir);
    assert_eq!(reopened.page(0).await.unwrap(), receipts);
    reopened.acknowledge(receipts).await.unwrap();
    drop(reopened);
    assert!(queue(&dir).page(0).await.unwrap().is_empty());
}

#[tokio::test]
async fn production_completion_publisher_retains_work_without_subscribers() {
    let dir = tempfile::tempdir().unwrap();
    let (_root, mut state) = crate::state::isolated_state();
    state.recovery = queue(&dir);
    announce_speakers_assigned(&state, "no-subscribers".into()).await;
    assert_eq!(
        state.recovery.page(0).await.unwrap()[0].meeting_id,
        "no-subscribers"
    );
    let mut receiver = state.subscribe();
    announce_speakers_assigned(&state, "connected".into()).await;
    assert!(
        matches!(receiver.recv().await.unwrap(), bruhs_models::MeetingEvent::SpeakersAssigned { meeting_id } if meeting_id == "connected")
    );
    assert_eq!(state.recovery.page(0).await.unwrap().len(), 2);
}

#[tokio::test]
async fn failed_completion_persistence_warns_without_losing_live_notification() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let (_root, mut state) = crate::state::isolated_state();
    state.recovery = RecoveryQueue::new(Some(file.path().join("unavailable")));
    let mut receiver = state.subscribe();
    announce_speakers_assigned(&state, "synthetic".into()).await;
    assert!(
        matches!(receiver.recv().await.unwrap(), bruhs_models::MeetingEvent::CaptureWarning { meeting_id, .. } if meeting_id == "synthetic")
    );
    assert!(
        matches!(receiver.recv().await.unwrap(), bruhs_models::MeetingEvent::SpeakersAssigned { meeting_id } if meeting_id == "synthetic")
    );
}

#[tokio::test]
async fn stale_and_repeated_ack_cannot_erase_newer_completion() {
    let dir = tempfile::tempdir().unwrap();
    let first = queue(&dir);
    let second = queue(&dir);
    first.enqueue("same-meeting".into()).await.unwrap();
    let stale = first.page(0).await.unwrap();
    second.enqueue("same-meeting".into()).await.unwrap();
    let latest = second.page(0).await.unwrap();
    assert_ne!(stale[0].token, latest[0].token);
    first.acknowledge(stale.clone()).await.unwrap();
    first.acknowledge(stale).await.unwrap();
    assert_eq!(second.page(0).await.unwrap(), latest);
}

#[tokio::test]
async fn pages_reach_pending_meetings_beyond_unknown_first_page() {
    let dir = tempfile::tempdir().unwrap();
    let queue = queue(&dir);
    for number in 0..139 {
        queue.enqueue(format!("synthetic-{number}")).await.unwrap();
    }
    let mut after = 0;
    let mut count = 0;
    loop {
        let page = queue.page(after).await.unwrap();
        assert!(page.len() <= CAPTURE_RECOVERY_PAGE_SIZE);
        let Some(last) = page.last() else { break };
        after = last.sequence;
        count += page.len();
    }
    assert_eq!(count, 139);
    assert_eq!(queue.page(0).await.unwrap().len(), 64);
}

#[tokio::test]
async fn invalid_request_does_not_open_storage_or_ack_valid_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let queue = queue(&dir);
    assert!(queue.enqueue("../escape".into()).await.is_err());
    assert!(queue.page(-1).await.is_err());
    assert!(!dir.path().join("recovery.sqlite3").exists());
    queue.enqueue("synthetic".into()).await.unwrap();
    let original = queue.page(0).await.unwrap();
    let mut invalid = original[0].clone();
    invalid.token.clear();
    assert!(queue
        .acknowledge(vec![original[0].clone(), invalid])
        .await
        .is_err());
    assert_eq!(queue.page(0).await.unwrap(), original);
    assert!(queue
        .acknowledge(vec![original[0].clone(); 65])
        .await
        .is_err());
}

#[tokio::test]
async fn failed_enqueue_and_ack_transactions_preserve_pending_work() {
    let dir = tempfile::tempdir().unwrap();
    let queue = queue(&dir);
    queue.enqueue("synthetic".into()).await.unwrap();
    let original = queue.page(0).await.unwrap();
    let connection = Connection::open(dir.path().join("recovery.sqlite3")).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_update BEFORE UPDATE ON recovery BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;
        CREATE TRIGGER reject_delete BEFORE DELETE ON recovery BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(queue.enqueue("synthetic".into()).await.is_err());
    assert!(queue.acknowledge(original.clone()).await.is_err());
    assert_eq!(queue.page(0).await.unwrap(), original);
}

#[tokio::test]
async fn late_ack_failure_rolls_back_an_already_deleted_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let queue = queue(&dir);
    queue.enqueue("first".into()).await.unwrap();
    queue.enqueue("second".into()).await.unwrap();
    let original = queue.page(0).await.unwrap();
    let connection = Connection::open(dir.path().join("recovery.sqlite3")).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_second BEFORE DELETE ON recovery WHEN OLD.meeting_id='second' BEGIN SELECT RAISE(ABORT,'synthetic late failure'); END;").unwrap();
    assert!(queue.acknowledge(original.clone()).await.is_err());
    assert_eq!(queue.page(0).await.unwrap(), original);
}

#[tokio::test]
async fn storage_failure_is_not_reported_as_an_empty_queue() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let queue = RecoveryQueue::new(Some(file.path().join("not-a-directory.sqlite3")));
    assert!(queue.page(0).await.is_err());
    assert!(queue.enqueue("synthetic".into()).await.is_err());
}

#[tokio::test]
async fn foreign_database_is_preserved_without_adding_recovery_tables() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let connection = Connection::open(file.path()).unwrap();
    connection
        .execute_batch("CREATE TABLE unrelated(value TEXT); INSERT INTO unrelated VALUES('keep');")
        .unwrap();
    let queue = RecoveryQueue::new(Some(file.path().into()));
    assert!(matches!(
        queue.page(0).await,
        Err(RecoveryError::UnknownFormat)
    ));
    let tables: Vec<String> = connection
        .prepare("SELECT name FROM sqlite_schema WHERE type='table'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(tables, vec!["unrelated"]);
    assert_eq!(
        connection
            .query_row("SELECT value FROM unrelated", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        "keep"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn recovery_storage_is_private_and_rejects_symlinks() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private/recovery.sqlite3");
    let queue = RecoveryQueue::new(Some(path.clone()));
    queue.enqueue("synthetic".into()).await.unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let link = dir.path().join("link.sqlite3");
    symlink(&path, &link).unwrap();
    assert!(RecoveryQueue::new(Some(link)).page(0).await.is_err());
    assert_eq!(queue.page(0).await.unwrap().len(), 1);
}

#[tokio::test]
async fn committed_completion_survives_abrupt_process_exit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recovery.sqlite3");
    let status = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "recovery::tests::synthetic_process_fixture",
            "--nocapture",
        ])
        .env("BRUHS_SYNTHETIC_RECOVERY_PATH", &path)
        .kill_on_drop(true)
        .status()
        .await
        .unwrap();
    assert!(status.success());
    let reopened = RecoveryQueue::new(Some(path));
    let receipts = reopened.page(0).await.unwrap();
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].meeting_id, "process-fixture");
}

#[tokio::test]
async fn synthetic_process_fixture() {
    let Some(path) = std::env::var_os("BRUHS_SYNTHETIC_RECOVERY_PATH") else {
        return;
    };
    let queue = RecoveryQueue::new(Some(path.into()));
    queue.enqueue("process-fixture".into()).await.unwrap();
    // Deliberately skip connection/runtime destructors. No hardware was started.
    std::process::exit(0);
}

#[tokio::test]
async fn http_recovery_uses_guard_and_never_starts_capture() {
    let dir = tempfile::tempdir().unwrap();
    let (_root, mut state) = crate::state::isolated_state();
    state.recovery = queue(&dir);
    state.recovery.enqueue("http-fixture".into()).await.unwrap();
    let state = Arc::new(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (app, authorized) = crate::test_router(state.clone());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = authorized;
    let response = client
        .get(format!("{url}/recovery"))
        .header("Origin", "null")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    let response = client.get(format!("{url}/recovery")).send().await.unwrap();
    assert_eq!(response.headers()["cache-control"], "no-store");
    let page: bruhs_models::CaptureRecoveryPage = response.json().await.unwrap();
    let response = client
        .post(format!("{url}/recovery/ack"))
        .header("Origin", "https://evil.test")
        .json(&page.receipts)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    assert_eq!(state.recovery.page(0).await.unwrap(), page.receipts);
    let response = client
        .post(format!("{url}/recovery/ack"))
        .json(&page.receipts)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 204);
    assert!(state.recovery.page(0).await.unwrap().is_empty());
    assert!(state.with_current(|current| current.is_none()));
    task.abort();
}
