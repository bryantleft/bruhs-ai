use super::*;
use std::sync::Arc;

async fn record(queue: Arc<RecoveryQueue>, start: f64) {
    tokio::task::spawn_blocking(move || {
        queue.record_failure_blocking("synthetic", start, start + 10.0)
    })
    .await
    .unwrap()
    .unwrap();
}

#[tokio::test]
async fn async_processing_and_blocking_stop_share_validation_and_attempt_fencing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recovery.sqlite3");
    let queue = Arc::new(RecoveryQueue::new(Some(path.clone())));
    assert!(queue
        .record_failure("../invalid", 10.0, 20.0)
        .await
        .is_err());
    assert!(queue.record_failure("synthetic", 20.0, 10.0).await.is_err());
    assert!(
        !path.exists(),
        "Invalid processing receipts must not create storage"
    );
    queue.record_failure("synthetic", 10.0, 20.0).await.unwrap();
    let original = queue.failure_page(0).await.unwrap();
    record(queue.clone(), 10.0).await;
    assert_eq!(queue.failure_page(0).await.unwrap(), original);
    queue.record_failure("synthetic", 30.0, 40.0).await.unwrap();
    queue.acknowledge_failures(original.clone()).await.unwrap();
    queue.acknowledge_failures(original).await.unwrap();
    drop(queue);
    let remaining = RecoveryQueue::new(Some(path))
        .failure_page(0)
        .await
        .unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].started_at_ms, 30.0);
}

#[tokio::test]
async fn failure_survives_reopen_and_stale_ack_cannot_erase_another_attempt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recovery.sqlite3");
    let queue = Arc::new(RecoveryQueue::new(Some(path.clone())));
    record(queue.clone(), 10.0).await;
    let first = queue.failure_page(0).await.unwrap();
    record(queue.clone(), 10.0).await;
    assert_eq!(queue.failure_page(0).await.unwrap(), first);
    record(queue.clone(), 20.0).await;
    queue.acknowledge_failures(first.clone()).await.unwrap();
    queue.acknowledge_failures(first).await.unwrap();
    drop(queue);
    let reopened = RecoveryQueue::new(Some(path));
    let remaining = reopened.failure_page(0).await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].started_at_ms, 20.0);
    let mut wrong = remaining.clone();
    wrong[0].token = uuid::Uuid::new_v4().to_string();
    reopened.acknowledge_failures(wrong).await.unwrap();
    assert_eq!(reopened.failure_page(0).await.unwrap(), remaining);
    reopened.acknowledge_failures(remaining).await.unwrap();
    assert!(reopened.failure_page(0).await.unwrap().is_empty());
}

#[tokio::test]
async fn version_one_upgrade_preserves_completion_receipts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recovery.sqlite3");
    // Use the production private file creator, then emulate the previous schema.
    let connection = schema::open(&path).unwrap();
    connection.execute_batch("DROP TABLE capture_failures; PRAGMA user_version=1;
        INSERT INTO recovery(meeting_id, token) VALUES('older', '00000000-0000-0000-0000-000000000001');").unwrap();
    drop(connection);
    let queue = Arc::new(RecoveryQueue::new(Some(path)));
    record(queue.clone(), 10.0).await;
    assert_eq!(queue.page(0).await.unwrap()[0].meeting_id, "older");
    assert_eq!(queue.failure_page(0).await.unwrap().len(), 1);
}

#[tokio::test]
async fn invalid_failure_ack_is_atomic_and_future_schema_is_retained() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recovery.sqlite3");
    let queue = Arc::new(RecoveryQueue::new(Some(path.clone())));
    record(queue.clone(), 10.0).await;
    let original = queue.failure_page(0).await.unwrap();
    let mut bad = original[0].clone();
    bad.failed_at_ms = f64::INFINITY;
    assert!(queue
        .acknowledge_failures(vec![original[0].clone(), bad])
        .await
        .is_err());
    assert_eq!(queue.failure_page(0).await.unwrap(), original);
    drop(queue);
    let connection = schema::open(&path).unwrap();
    connection.pragma_update(None, "user_version", 999).unwrap();
    drop(connection);
    assert!(matches!(
        RecoveryQueue::new(Some(path)).failure_page(0).await,
        Err(RecoveryError::UnknownFormat)
    ));
}
