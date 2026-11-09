use super::*;

fn enable(consent: &MonitorConsent) -> (String, String) {
    let epoch = consent.status().epoch;
    let token = uuid::Uuid::new_v4().to_string();
    consent
        .apply(ActivityMonitorCommand::Enable {
            epoch: epoch.clone(),
            token: token.clone(),
        })
        .unwrap();
    (epoch, token)
}

#[test]
fn startup_cannot_authorize_a_probe() {
    let consent = MonitorConsent::new();
    assert!(!consent.status().enabled);
    assert!(consent.acquire().is_none());
}

#[test]
fn an_unused_challenge_cannot_enable_after_its_client_disappears() {
    let consent = MonitorConsent::new();
    let epoch = consent.offer().epoch;
    assert!(consent
        .apply_at(
            ActivityMonitorCommand::Enable {
                epoch,
                token: uuid::Uuid::new_v4().to_string()
            },
            Instant::now() + Duration::from_millis(ACTIVITY_MONITOR_LEASE_MS + 1)
        )
        .is_err());
    assert!(consent.acquire().is_none());
}

#[test]
fn delayed_enable_cannot_extend_its_original_challenge_deadline() {
    let consent = MonitorConsent::new();
    let epoch = consent.offer().epoch;
    let status = consent
        .apply_at(
            ActivityMonitorCommand::Enable {
                epoch,
                token: uuid::Uuid::new_v4().to_string(),
            },
            Instant::now() + Duration::from_secs(20),
        )
        .unwrap();
    assert!(status.expires_in_ms <= 10_000);
}

#[test]
fn probe_errors_belong_only_to_the_current_permission() {
    let consent = MonitorConsent::new();
    let (epoch, _) = enable(&consent);
    let guard = consent.acquire().unwrap();
    guard
        .0
        .report_probe_error(Some("synthetic input unavailable"));
    assert_eq!(
        consent.status().probe_error.as_deref(),
        Some("synthetic input unavailable")
    );
    guard.0.report_probe_error(None);
    assert!(consent.status().probe_error.is_none());
    consent
        .apply(ActivityMonitorCommand::Disable { epoch })
        .unwrap();
    guard.0.report_probe_error(Some("stale failure"));
    assert!(consent.status().probe_error.is_none());
}

#[tokio::test]
async fn disable_fences_delayed_enable_and_renew_and_waits_for_probe_shutdown() {
    let consent = MonitorConsent::new();
    let (epoch, token) = enable(&consent);
    let guard = consent.acquire().unwrap();
    assert!(guard.0.authorized());
    assert!(consent.acquire().is_none());
    consent
        .apply(ActivityMonitorCommand::Disable {
            epoch: epoch.clone(),
        })
        .unwrap();
    assert!(!guard.0.authorized());
    assert!(consent
        .apply(ActivityMonitorCommand::Renew {
            epoch: epoch.clone(),
            token: token.clone()
        })
        .is_err());
    assert!(consent
        .apply(ActivityMonitorCommand::Enable { epoch, token })
        .is_err());
    assert!(
        tokio::time::timeout(Duration::from_millis(20), consent.wait_stopped())
            .await
            .is_err()
    );
    drop(guard);
    consent.wait_stopped().await.unwrap();
    assert!(consent.acquire().is_none());
}

#[test]
fn expiry_cannot_be_revived_by_a_late_renewal() {
    let consent = MonitorConsent::new();
    let (epoch, token) = enable(&consent);
    let now = Instant::now() + Duration::from_millis(ACTIVITY_MONITOR_LEASE_MS + 1);
    assert!(consent
        .apply_at(
            ActivityMonitorCommand::Renew {
                epoch: epoch.clone(),
                token: token.clone()
            },
            now
        )
        .is_err());
    assert!(consent
        .apply(ActivityMonitorCommand::Enable { epoch, token })
        .is_err());
    assert!(consent.acquire().is_none());
}

#[tokio::test]
async fn all_shutdown_waiters_observe_probe_drain() {
    let consent = MonitorConsent::new();
    let (epoch, _) = enable(&consent);
    let guard = consent.acquire().unwrap();
    consent
        .apply(ActivityMonitorCommand::Disable { epoch })
        .unwrap();
    let a = consent.clone();
    let b = consent.clone();
    let first = tokio::spawn(async move { a.wait_stopped().await });
    let second = tokio::spawn(async move { b.wait_stopped().await });
    tokio::task::yield_now().await;
    drop(guard);
    tokio::time::timeout(Duration::from_millis(200), async {
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
    })
    .await
    .unwrap();
}

#[test]
fn wrong_owner_cannot_renew_and_new_daemon_does_not_accept_old_grants() {
    let consent = MonitorConsent::new();
    let (epoch, token) = enable(&consent);
    assert!(consent
        .apply(ActivityMonitorCommand::Renew {
            epoch: epoch.clone(),
            token: "wrong".into()
        })
        .is_err());
    let restarted = MonitorConsent::new();
    assert!(restarted
        .apply(ActivityMonitorCommand::Renew { epoch, token })
        .is_err());
    assert!(restarted.acquire().is_none());
}

/// A sampler that counts its calls and signals each one on `sampled`.
fn counting_sampler(
    calls: &Arc<std::sync::atomic::AtomicUsize>,
    sampled: tokio::sync::mpsc::UnboundedSender<()>,
    check: impl Fn(&SamplePermit) + Send + Sync + 'static,
) -> Arc<impl Fn(SamplePermit) -> Result<f32, String> + Send + Sync + 'static> {
    let seen = calls.clone();
    Arc::new(move |permit: SamplePermit| {
        check(&permit);
        seen.fetch_add(1, Ordering::SeqCst);
        // The test may have stopped listening; the count still records it.
        let _ = sampled.send(());
        Ok(0.0)
    })
}

// Paused clock: a sleep in these tests returns only after every other task is
// idle, and covers several of the worker's poll intervals in virtual time.
const SETTLE: Duration = Duration::from_secs(5);

#[tokio::test(start_paused = true)]
async fn worker_never_calls_sampler_without_consent_and_stops_after_revocation() {
    let (_root, state) = crate::state::isolated_state();
    let state = Arc::new(state);
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (sampled_tx, mut sampled) = tokio::sync::mpsc::unbounded_channel();
    let sampler = counting_sampler(&calls, sampled_tx, |permit| {
        assert!(permit.authorized());
    });
    let worker = tokio::spawn(crate::activity_monitor::run(state.clone(), sampler));
    tokio::time::sleep(SETTLE).await;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let (epoch, _) = enable(&state.monitor);
    tokio::time::timeout(SETTLE, sampled.recv())
        .await
        .unwrap()
        .unwrap();
    state
        .monitor
        .apply(ActivityMonitorCommand::Disable { epoch })
        .unwrap();
    state.monitor.wait_stopped().await.unwrap();
    let stopped = calls.load(Ordering::SeqCst);
    tokio::time::sleep(SETTLE).await;
    assert_eq!(calls.load(Ordering::SeqCst), stopped);
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
}

#[tokio::test(start_paused = true)]
async fn recording_transition_and_activity_probe_cannot_open_microphone_concurrently() {
    let (_root, state) = crate::state::isolated_state();
    let state = Arc::new(state);
    let reservation = state.lifecycle.enter().await;
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let probe_state = state.clone();
    let (sampled_tx, mut sampled) = tokio::sync::mpsc::unbounded_channel();
    let sampler = counting_sampler(&calls, sampled_tx, move |permit| {
        assert!(permit.authorized());
        assert!(probe_state.lifecycle.try_enter().is_none());
    });
    let (epoch, _) = enable(&state.monitor);
    let worker = tokio::spawn(crate::activity_monitor::run(state.clone(), sampler));
    tokio::time::sleep(SETTLE).await;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    drop(reservation);
    tokio::time::timeout(SETTLE, sampled.recv())
        .await
        .unwrap()
        .unwrap();
    state
        .monitor
        .apply(ActivityMonitorCommand::Disable { epoch })
        .unwrap();
    state.monitor.wait_stopped().await.unwrap();
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
    assert!(state.lifecycle.try_enter().is_some());
}
