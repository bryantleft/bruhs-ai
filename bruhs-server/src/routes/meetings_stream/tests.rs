use super::*;
use axum::{http::StatusCode, response::IntoResponse, routing::get, Router};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio_stream::StreamExt;

struct Fixture {
    url: String,
    task: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn start(app: Router) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { url, task }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn quick_retry() -> daemon::RetryPolicy {
    daemon::RetryPolicy {
        initial: Duration::from_millis(10),
        maximum: Duration::from_millis(20),
        headers_timeout: Duration::from_millis(40),
    }
}

async fn next_payload(stream: &mut (impl Stream<Item = String> + Unpin)) -> String {
    tokio::time::timeout(Duration::from_secs(3), stream.next())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn daemon_retries_unavailable_response_without_reconnecting_browser() {
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    let fixture = Fixture::start(Router::new().route(
        "/events",
        get(move || {
            let attempt = seen.fetch_add(1, Ordering::SeqCst);
            async move {
                if attempt == 0 {
                    StatusCode::SERVICE_UNAVAILABLE.into_response()
                } else {
                    "data: recovered\n\n".into_response()
                }
            }
        }),
    ))
    .await;
    let mut stream = Box::pin(daemon::events(
        bruhs_core::capture_client::DaemonEndpoint::new(fixture.url.clone(), None),
        quick_retry(),
    ));
    assert_eq!(
        next_payload(&mut stream).await,
        r#"{"kind":"stream_opened"}"#
    );
    assert_eq!(next_payload(&mut stream).await, "recovered");
    assert_eq!(requests.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn daemon_header_timeout_retries_without_waiting_for_stream_lifetime_timeout() {
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    let fixture = Fixture::start(Router::new().route(
        "/events",
        get(move || {
            let attempt = seen.fetch_add(1, Ordering::SeqCst);
            async move {
                if attempt == 0 {
                    std::future::pending::<axum::response::Response>().await
                } else {
                    "data: recovered after timeout\n\n".into_response()
                }
            }
        }),
    ))
    .await;
    let mut stream = Box::pin(daemon::events(
        bruhs_core::capture_client::DaemonEndpoint::new(fixture.url.clone(), None),
        quick_retry(),
    ));
    assert_eq!(
        next_payload(&mut stream).await,
        r#"{"kind":"stream_opened"}"#
    );
    assert_eq!(next_payload(&mut stream).await, "recovered after timeout");
    assert_eq!(requests.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn daemon_eof_reconnect_discards_only_the_incomplete_old_frame() {
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    let fixture = Fixture::start(Router::new().route(
        "/events",
        get(move || {
            let attempt = seen.fetch_add(1, Ordering::SeqCst);
            async move {
                if attempt == 0 {
                    "data: first\n\ndata: truncated"
                } else {
                    "data: second\n\n"
                }
            }
        }),
    ))
    .await;
    let mut stream = Box::pin(daemon::events(
        bruhs_core::capture_client::DaemonEndpoint::new(fixture.url.clone(), None),
        quick_retry(),
    ));
    assert_eq!(
        next_payload(&mut stream).await,
        r#"{"kind":"stream_opened"}"#
    );
    assert_eq!(next_payload(&mut stream).await, "first");
    assert_eq!(
        next_payload(&mut stream).await,
        r#"{"kind":"stream_opened"}"#
    );
    assert_eq!(next_payload(&mut stream).await, "second");
    assert_eq!(requests.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn bridge_forwards_server_events_while_daemon_headers_are_stalled() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let notify = entered.clone();
    let upstream = Fixture::start(Router::new().route(
        "/events",
        get(move || {
            notify.notify_one();
            std::future::pending::<StatusCode>()
        }),
    ))
    .await;
    let state = Arc::new(AppState::new());
    let url = bruhs_core::capture_client::DaemonEndpoint::new(upstream.url.clone(), None);
    let _pump = pump::spawn(
        state.clone(),
        daemon::events(url, daemon::RetryPolicy::default()),
        |_| async { Ok(()) },
    );
    let downstream = Fixture::start(
        Router::new()
            .route("/stream", get(get_meetings_stream))
            .with_state(state.clone()),
    )
    .await;
    let response = reqwest::get(format!("{}/stream", downstream.url))
        .await
        .unwrap();
    let mut bytes = response.bytes_stream();
    let first = tokio::time::timeout(Duration::from_secs(1), bytes.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(std::str::from_utf8(&first)
        .unwrap()
        .contains("stream_opened"));
    tokio::time::timeout(Duration::from_secs(1), entered.notified())
        .await
        .unwrap();
    state.emit_meeting_event(MeetingEvent::PersonChanged {
        person_id: "synthetic-person".into(),
        updated_at: 42,
    });
    let next = tokio::time::timeout(Duration::from_secs(1), bytes.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(std::str::from_utf8(&next)
        .unwrap()
        .contains("synthetic-person"));
}

#[tokio::test]
async fn bridge_channel_lag_emits_reconciliation_instead_of_silently_skipping() {
    let (sender, receiver) = tokio::sync::broadcast::channel(1);
    let mut stream = Box::pin(bridge(receiver));
    let _handshake = stream.next().await.unwrap().unwrap();
    sender.send(MeetingEvent::StreamOpened).unwrap();
    sender.send(MeetingEvent::PeopleUpdated).unwrap();
    // Serialize through the real SSE response body, not private Event fields.
    let response = Sse::new(stream.take(1)).into_response();
    let body = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    assert_eq!(
        std::str::from_utf8(&body).unwrap(),
        "data: {\"kind\":\"stream_opened\"}\n\n"
    );
}

/// Dropping the subscription cancels the in-flight daemon request; no
/// detached task keeps retrying. Explicit signals instead of sleeps: the
/// fixture fails the first request, holds the second open and reports
/// when that held request is cancelled.
#[tokio::test]
async fn dropping_daemon_subscription_stops_retry_requests() {
    struct Cancelled(Option<tokio::sync::oneshot::Sender<()>>);
    impl Drop for Cancelled {
        fn drop(&mut self) {
            if let Some(cancelled) = self.0.take() {
                // The test may have stopped listening; nothing to report then.
                cancelled.send(()).ok();
            }
        }
    }
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    let (held_tx, held_rx) = tokio::sync::oneshot::channel::<()>();
    let (cancelled_tx, cancelled_rx) = tokio::sync::oneshot::channel::<()>();
    let held = Arc::new(std::sync::Mutex::new(Some((held_tx, cancelled_tx))));
    let fixture = Fixture::start(Router::new().route(
        "/events",
        get(move || {
            let attempt = seen.fetch_add(1, Ordering::SeqCst);
            let held = if attempt == 1 {
                held.lock().unwrap().take()
            } else {
                None
            };
            async move {
                let Some((arrived, cancelled)) = held else {
                    return StatusCode::SERVICE_UNAVAILABLE.into_response();
                };
                let _guard = Cancelled(Some(cancelled));
                arrived.send(()).ok();
                std::future::pending::<axum::response::Response>().await
            }
        }),
    ))
    .await;
    let retry = daemon::RetryPolicy {
        // Long enough that the held request is never abandoned by timeout.
        headers_timeout: Duration::from_secs(600),
        ..quick_retry()
    };
    let mut stream = Box::pin(daemon::events(
        bruhs_core::capture_client::DaemonEndpoint::new(fixture.url.clone(), None),
        retry,
    ));
    // Drive the stream until the retry request is held by the fixture.
    tokio::select! {
        _ = stream.next() => panic!("the daemon never answers successfully"),
        arrived = held_rx => arrived.unwrap(),
    }
    assert_eq!(requests.load(Ordering::SeqCst), 2);
    drop(stream);
    tokio::time::timeout(Duration::from_secs(10), cancelled_rx)
        .await
        .expect("dropping the subscription cancels the held request")
        .unwrap();
    assert_eq!(requests.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn multiple_http_clients_share_one_upstream_and_disconnects_do_not_own_work() {
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    let (capture_tx, _) = tokio::sync::broadcast::channel::<String>(16);
    let capture = capture_tx.clone();
    let upstream = Fixture::start(Router::new().route(
        "/events",
        get(move || {
            seen.fetch_add(1, Ordering::SeqCst);
            let mut receiver = capture.subscribe();
            async move {
                Sse::new(async_stream::stream! {
                    while let Ok(payload) = receiver.recv().await {
                        yield Ok::<_, Infallible>(Event::default().data(payload));
                    }
                })
                .keep_alive(KeepAlive::default())
            }
        }),
    ))
    .await;
    let state = Arc::new(AppState::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let mut meeting = bruhs_models::Meeting::new("Synthetic", 1.0);
    meeting.id = "synthetic".into();
    state.meetings().create_meeting(meeting).await.unwrap();
    let linked = calls.clone();
    let _pump = pump::spawn(
        state.clone(),
        daemon::events(
            bruhs_core::capture_client::DaemonEndpoint::new(upstream.url.clone(), None),
            quick_retry(),
        ),
        move |_| {
            linked.fetch_add(1, Ordering::SeqCst);
            async { Ok(()) }
        },
    );
    let downstream = Fixture::start(
        Router::new()
            .route("/stream", get(get_meetings_stream))
            .with_state(state),
    )
    .await;
    let one = reqwest::get(format!("{}/stream", downstream.url))
        .await
        .unwrap();
    let two = reqwest::get(format!("{}/stream", downstream.url))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while capture_tx.receiver_count() != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut one = one.bytes_stream();
    let mut two = two.bytes_stream();
    let payload = serde_json::to_string(&MeetingEvent::SpeakersAssigned {
        meeting_id: "synthetic".into(),
    })
    .unwrap();
    capture_tx.send(payload.clone()).unwrap();
    for stream in [&mut one, &mut two] {
        tokio::time::timeout(Duration::from_secs(3), async {
            let mut body = String::new();
            while !body.contains("speakers_assigned") {
                body.push_str(std::str::from_utf8(&stream.next().await.unwrap().unwrap()).unwrap());
            }
        })
        .await
        .unwrap();
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    drop(one);
    drop(two);
    capture_tx.send(payload).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while calls.load(Ordering::SeqCst) != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(requests.load(Ordering::SeqCst), 1);
}
