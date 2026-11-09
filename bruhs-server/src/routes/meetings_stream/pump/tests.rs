use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::{mpsc, Notify};
use tokio_stream::wrappers::ReceiverStream;

fn assigned(id: &str) -> String {
    serde_json::to_string(&MeetingEvent::SpeakersAssigned {
        meeting_id: id.into(),
    })
    .unwrap()
}

async fn create(state: &AppState, id: &str) {
    let mut meeting = bruhs_models::Meeting::new("Synthetic", 1.0);
    meeting.id = id.into();
    state.meetings().create_meeting(meeting).await.unwrap();
}

async fn drained(state: &AppState) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while state
            .meetings()
            .next_speaker_link(i64::MAX)
            .await
            .unwrap()
            .is_some()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

async fn until(mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !ready() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn links_without_any_http_subscribers() {
    let state = Arc::new(AppState::new());
    let meeting = state
        .meetings()
        .create_meeting(bruhs_models::Meeting::new("Synthetic", 1.0))
        .await
        .unwrap();
    let id = meeting.id.clone();
    let app = state.clone();
    let (sender, receiver) = mpsc::channel(1);
    let done = Arc::new(Notify::new());
    let finished = done.clone();
    let _pump = spawn(
        state.clone(),
        ReceiverStream::new(receiver),
        move |meeting_id| {
            let app = app.clone();
            let finished = finished.clone();
            async move {
                let centroids = bruhs_models::CentroidsFile {
                    linking_auto: false,
                    centroids: vec![bruhs_models::ClusterCentroid {
                        cluster_label: "them_1".into(),
                        embedding: vec![1.0, 0.0],
                        first_seen_ms: 0,
                    }],
                    ..Default::default()
                };
                assert!(linking::link_centroids(&app, &meeting_id, &centroids)
                    .await
                    .unwrap());
                finished.notify_one();
                Ok(())
            }
        },
    );
    assert_eq!(state.meeting_event_subscribers(), 0);
    sender.send(assigned(&id)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), done.notified())
        .await
        .unwrap();
    assert_eq!(state.meetings().list_people().await.unwrap().len(), 1);
    assert_eq!(
        state
            .meetings()
            .get_meeting(&id)
            .await
            .unwrap()
            .unwrap()
            .speakers
            .iter()
            .filter(|speaker| speaker.id == "them_1" && speaker.person_id.is_some())
            .count(),
        1
    );
}

#[tokio::test]
async fn stalled_linking_does_not_block_fanout_and_bursts_coalesce() {
    let state = Arc::new(AppState::new());
    create(&state, "one").await;
    let mut events = state.subscribe_meeting_events();
    let (sender, receiver) = mpsc::channel(512);
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let release = Arc::new(Notify::new());
    let unblock = release.clone();
    let _pump = spawn(state.clone(), ReceiverStream::new(receiver), move |_| {
        seen.fetch_add(1, Ordering::SeqCst);
        let unblock = unblock.clone();
        async move {
            unblock.notified().await;
            Ok(())
        }
    });
    sender.send(assigned("one")).await.unwrap();
    until(|| calls.load(Ordering::SeqCst) == 1).await;
    for _ in 0..100 {
        sender.send(assigned("one")).await.unwrap();
    }
    sender
        .send(
            serde_json::to_string(&MeetingEvent::PersonChanged {
                person_id: "sentinel".into(),
                updated_at: 1,
            })
            .unwrap(),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Ok(MeetingEvent::PersonChanged { person_id, .. }) = events.recv().await {
                assert_eq!(person_id, "sentinel");
                break;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    release.notify_one();
    until(|| calls.load(Ordering::SeqCst) == 2).await;
    release.notify_one();
    drained(&state).await;
    drop(sender);
    until(|| _pump.0.is_finished()).await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn more_than_the_old_queue_limit_is_retained_with_one_active_worker() {
    let state = Arc::new(AppState::new());
    create(&state, "active").await;
    let mut events = state.subscribe_meeting_events();
    let (sender, receiver) = mpsc::channel(1);
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let _pump = spawn(state.clone(), ReceiverStream::new(receiver), move |_| {
        seen.fetch_add(1, Ordering::SeqCst);
        std::future::pending()
    });
    sender.send(assigned("active")).await.unwrap();
    until(|| calls.load(Ordering::SeqCst) == 1).await;
    assert!(matches!(
        events.recv().await.unwrap(),
        MeetingEvent::SpeakersAssigned { .. }
    ));
    for n in 0..300 {
        create(&state, &format!("pending-{n}")).await;
        sender
            .send(assigned(&format!("pending-{n}")))
            .await
            .unwrap();
        assert!(matches!(
            events.recv().await.unwrap(),
            MeetingEvent::SpeakersAssigned { .. }
        ));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    drop(_pump);
    until(|| sender.is_closed()).await;
    let mut retained = 0;
    while let Some(job) = state.meetings().next_speaker_link(i64::MAX).await.unwrap() {
        retained += 1;
        state.meetings().finish_speaker_link(&job).await.unwrap();
    }
    assert_eq!(retained, 301);
}

#[tokio::test]
async fn dropping_owner_cancels_active_work_and_upstream() {
    struct Dropped(Arc<AtomicUsize>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let drops = Arc::new(AtomicUsize::new(0));
    let watched = drops.clone();
    let started = Arc::new(AtomicUsize::new(0));
    let seen = started.clone();
    let (sender, receiver) = mpsc::channel(1);
    let state = Arc::new(AppState::new());
    create(&state, "one").await;
    let pump = spawn(state.clone(), ReceiverStream::new(receiver), move |_| {
        let guard = Dropped(watched.clone());
        seen.fetch_add(1, Ordering::SeqCst);
        async move {
            let _guard = guard;
            std::future::pending::<Result<(), LinkError>>().await
        }
    });
    sender.send(assigned("one")).await.unwrap();
    until(|| started.load(Ordering::SeqCst) == 1).await;
    drop(pump);
    until(|| drops.load(Ordering::SeqCst) == 1 && sender.is_closed()).await;
    assert_eq!(
        state
            .meetings()
            .next_speaker_link(0)
            .await
            .unwrap()
            .unwrap()
            .meeting_id,
        "one"
    );
}

/// Runs on paused time: the worker's clock is the wall clock plus the
/// test's virtual elapsed time, so the 1 s retry backoff and the worker's
/// poll interval pass instantly and deterministically.
#[tokio::test(start_paused = true)]
async fn transient_failure_retries_without_a_new_event_and_does_not_spin() {
    let state = Arc::new(AppState::new());
    create(&state, "retry").await;
    let (sender, receiver) = mpsc::channel(1);
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let start = tokio::time::Instant::now();
    let clock = move || {
        chrono::Utc::now().timestamp_millis() + i64::try_from(start.elapsed().as_millis()).unwrap()
    };
    let _pump = spawn_with_clock(
        state.clone(),
        ReceiverStream::new(receiver),
        move |_| {
            let attempt = seen.fetch_add(1, Ordering::SeqCst);
            async move {
                if attempt == 0 {
                    Err(super::super::linking::LinkError::CentroidsUnavailable)
                } else {
                    Ok(())
                }
            }
        },
        clock,
    );
    sender.send(assigned("retry")).await.unwrap();
    until(|| calls.load(Ordering::SeqCst) == 1).await;
    // Well inside the first backoff: no second attempt (no busy retry).
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // The retry comes from the worker's own timer, without a new event.
    tokio::time::timeout(Duration::from_secs(5), async {
        while calls.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    drained(&state).await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn restart_after_link_commit_before_ack_replays_without_duplicate_profiles() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private.sqlite3");
    let app = AppState::builder()
        .meetings(Arc::new(
            bruhs_core::SqliteMeetingStore::open(&path).await.unwrap(),
        ))
        .build();
    let state = Arc::new(app);
    create(&state, "interrupted").await;
    let file = bruhs_models::CentroidsFile {
        linking_auto: false,
        centroids: vec![bruhs_models::ClusterCentroid {
            cluster_label: "them_1".into(),
            embedding: vec![1.0, 0.0],
            first_seen_ms: 0,
        }],
        ..Default::default()
    };
    let (sender, receiver) = mpsc::channel(1);
    let committed = Arc::new(Notify::new());
    let ready = committed.clone();
    let app = state.clone();
    let initial = file.clone();
    let pump = spawn(state.clone(), ReceiverStream::new(receiver), move |id| {
        let app = app.clone();
        let ready = ready.clone();
        let file = initial.clone();
        async move {
            assert!(linking::link_centroids(&app, &id, &file).await.unwrap());
            ready.notify_one();
            std::future::pending::<Result<(), LinkError>>().await
        }
    });
    sender.send(assigned("interrupted")).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), committed.notified())
        .await
        .unwrap();
    let saved = state.meetings().get_meeting("interrupted").await.unwrap();
    let people = state.meetings().list_people().await.unwrap();
    drop(pump);
    until(|| sender.is_closed()).await;
    drop(state);
    let app = AppState::builder()
        .meetings(Arc::new(
            bruhs_core::SqliteMeetingStore::open(&path).await.unwrap(),
        ))
        .build();
    let state = Arc::new(app);
    let app = state.clone();
    // No new upstream event and no UI subscribers: startup alone resumes work.
    let _pump = spawn(state.clone(), tokio_stream::pending(), move |id| {
        let app = app.clone();
        let file = file.clone();
        async move {
            assert!(!linking::link_centroids(&app, &id, &file).await.unwrap());
            Ok(())
        }
    });
    drained(&state).await;
    assert_eq!(
        state.meetings().get_meeting("interrupted").await.unwrap(),
        saved
    );
    assert_eq!(state.meetings().list_people().await.unwrap(), people);
    assert_eq!(people.len(), 1);
}
