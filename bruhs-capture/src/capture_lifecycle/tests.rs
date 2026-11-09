use super::CaptureLifecycle;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc, Arc,
};
use std::time::Duration;

// Blocking fixtures have deadlines, so a failed assertion cannot leave the
// runtime waiting indefinitely for a synthetic hardware worker.
const DEADLINE: Duration = Duration::from_secs(3);

#[tokio::test(flavor = "current_thread")]
async fn hardware_wait_keeps_executor_responsive_and_serializes_following_stop() {
    let lifecycle = CaptureLifecycle::default();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let order = Arc::new(AtomicUsize::new(0));
    let first = lifecycle.clone();
    let first_order = order.clone();
    let start = tokio::spawn(async move {
        first
            .run(move || {
                started_tx.send(()).unwrap();
                release_rx.recv_timeout(DEADLINE).unwrap();
                assert_eq!(first_order.fetch_add(1, Ordering::SeqCst), 0);
            })
            .await
            .unwrap();
    });
    tokio::time::timeout(DEADLINE, started_rx)
        .await
        .unwrap()
        .unwrap();
    assert!(lifecycle.try_enter().is_none());
    // This timer runs on the only executor thread while hardware is blocked.
    tokio::time::timeout(DEADLINE, tokio::time::sleep(Duration::from_millis(5)))
        .await
        .unwrap();
    let next = lifecycle.clone();
    let next_order = order.clone();
    let stop = tokio::spawn(async move {
        next.run(move || assert_eq!(next_order.fetch_add(1, Ordering::SeqCst), 1))
            .await
            .unwrap();
    });
    tokio::task::yield_now().await;
    assert_eq!(order.load(Ordering::SeqCst), 0);
    release_tx.send(()).unwrap();
    start.await.unwrap();
    stop.await.unwrap();
    assert_eq!(order.load(Ordering::SeqCst), 2);
    assert!(lifecycle.try_enter().is_some());
}

#[tokio::test]
async fn disconnected_request_keeps_reservation_until_hardware_finishes() {
    let lifecycle = CaptureLifecycle::default();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let operation = lifecycle.clone();
    let request = tokio::spawn(async move {
        operation
            .run(move || {
                started_tx.send(()).unwrap();
                release_rx.recv_timeout(DEADLINE).unwrap();
            })
            .await
            .unwrap();
    });
    tokio::time::timeout(DEADLINE, started_rx)
        .await
        .unwrap()
        .unwrap();
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    assert!(
        lifecycle.try_enter().is_none(),
        "disconnect released an active operation"
    );
    release_tx.send(()).unwrap();
    tokio::time::timeout(DEADLINE, lifecycle.run(|| ()))
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn cancelled_queued_request_never_runs_and_panics_release_reservations() {
    let lifecycle = CaptureLifecycle::default();
    let held = lifecycle.enter().await;
    let called = Arc::new(AtomicUsize::new(0));
    let call_count = called.clone();
    let waiting = lifecycle.clone();
    let queued = tokio::spawn(async move {
        waiting
            .run(move || {
                call_count.fetch_add(1, Ordering::SeqCst);
            })
            .await
            .unwrap();
    });
    tokio::task::yield_now().await;
    queued.abort();
    assert!(queued.await.unwrap_err().is_cancelled());
    drop(held);
    lifecycle.run(|| ()).await.unwrap();
    assert_eq!(called.load(Ordering::SeqCst), 0);
    assert!(lifecycle
        .run(|| panic!("synthetic hardware failure"))
        .await
        .unwrap_err()
        .is_panic());
    assert!(lifecycle.try_enter().is_some());
}
