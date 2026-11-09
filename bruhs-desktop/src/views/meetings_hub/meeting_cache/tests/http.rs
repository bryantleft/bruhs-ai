use super::*;
use std::sync::atomic::{AtomicU8, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Server {
    base: String,
    mode: Arc<AtomicU8>,
    worker: tokio::task::JoinHandle<()>,
}

impl Server {
    fn new(runtime: &Runtime) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("owned listener");
        listener.set_nonblocking(true).expect("nonblocking");
        let base = format!("http://{}", listener.local_addr().expect("address"));
        let mode = Arc::new(AtomicU8::new(0));
        let state = mode.clone();
        let worker = runtime.spawn(async move {
            let listener = tokio::net::TcpListener::from_std(listener).expect("async listener");
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut bytes = Vec::new();
                loop {
                    let mut chunk = [0; 1024];
                    let size = stream.read(&mut chunk).await.expect("request read");
                    assert!(size > 0 && bytes.len() < 8192);
                    bytes.extend_from_slice(&chunk[..size]);
                    if bytes.windows(4).any(|part| part == b"\r\n\r\n") { break; }
                }
                let request = String::from_utf8_lossy(&bytes);
                let state = state.load(Ordering::SeqCst);
                let (status, body) = if state == 1 {
                    ("503 Service Unavailable", "{}".to_string())
                } else if request.starts_with("GET /api/meetings/cached-meeting/segment-removals ") {
                    ("200 OK", "[]".to_string())
                } else if request.starts_with("GET /api/meetings/cached-meeting/segments ") {
                    let mut record = segment();
                    if state == 2 { record.meeting_id = "foreign".into(); }
                    ("200 OK", serde_json::to_string(&vec![record]).expect("transcript"))
                } else {
                    assert!(request.starts_with("GET /api/meetings "), "unexpected request: {request}");
                    ("200 OK", serde_json::to_string(&if state == 3 { vec![] } else { vec![meeting()] }).expect("meetings"))
                };
                let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                stream.write_all(response.as_bytes()).await.expect("reply");
            }
        });
        Self { base, mode, worker }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.worker.abort();
    }
}

#[gpui::test]
async fn native_transcript_http_reply_cannot_discard_fragments_received_after_request_started(
    cx: &mut TestAppContext,
) {
    let hub = fixture(cx);
    let server = hub.update(cx, |hub, cx| {
        let server = Server::new(&hub.runtime);
        hub.client = Arc::new(ServerClient::new(&server.base, &server.base).unwrap());
        hub.install_meeting_test_snapshot(meeting(), vec![segment()], cx);
        hub.ensure_segments("cached-meeting", cx);
        hub.receive_segment(
            "cached-meeting".into(),
            TranscriptSegment {
                id: "new-live".into(),
                text: "Received after GET began".into(),
                partial: true,
                ..segment()
            },
            cx,
        );
        server
    });
    cx.condition(&hub, |hub, _| hub.notice.is_some()).await;
    hub.update(cx, |hub, _| {
        assert_eq!(hub.segments["cached-meeting"].len(), 2);
        assert!(hub
            .notice
            .as_ref()
            .unwrap()
            .message
            .contains("New speech arrived"));
        assert!(hub.meeting_cache.fragments.contains_key("cached-meeting"));
    });
    drop(server);
}

#[gpui::test]
async fn native_meeting_cache_http_refresh_retries_loaded_transcript_and_preserves_it_on_failure(
    cx: &mut TestAppContext,
) {
    let hub = fixture(cx);
    let server = hub.update(cx, |hub, cx| {
        let server = Server::new(&hub.runtime);
        hub.client = Arc::new(ServerClient::new(&server.base, &server.base).unwrap());
        hub.refresh_meetings(cx);
        server
    });
    cx.condition(&hub, |hub, _| !hub.meetings.is_empty()).await;
    hub.update(cx, |hub, cx| hub.ensure_segments("cached-meeting", cx));
    cx.condition(&hub, |hub, _| hub.segments.contains_key("cached-meeting"))
        .await;
    let cache = hub.update(cx, |hub, _| hub.meeting_cache());
    for mode in [1, 2] {
        server.mode.store(mode, Ordering::SeqCst);
        hub.update(cx, |hub, cx| {
            hub.notice = None;
            hub.ensure_segments("cached-meeting", cx);
        });
        cx.condition(&hub, |hub, _| hub.notice.is_some()).await;
        hub.update(cx, |hub, _| assert_eq!(hub.meeting_cache(), cache));
    }
    // Only an unreachable server is offline (a failure status means it is
    // up); either way the cached meetings stay.
    let unreachable = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("free port");
        format!("http://{}", listener.local_addr().expect("address"))
    };
    hub.update(cx, |hub, cx| {
        hub.client = Arc::new(ServerClient::new(&unreachable, &unreachable).unwrap());
        hub.refresh_meetings(cx);
    });
    cx.condition(&hub, |hub, _| !hub.server_online).await;
    hub.update(cx, |hub, _| assert_eq!(hub.meeting_cache(), cache));
    server.mode.store(3, Ordering::SeqCst);
    hub.update(cx, |hub, cx| {
        hub.client = Arc::new(ServerClient::new(&server.base, &server.base).unwrap());
        hub.refresh_meetings(cx);
    });
    cx.condition(&hub, |hub, _| hub.meetings.is_empty()).await;
    hub.update(cx, |hub, _| {
        assert_eq!(hub.meeting_cache(), MeetingCache::default())
    });
}
