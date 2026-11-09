use super::*;
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Mutex,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(super) struct Server {
    pub base: String,
    pub mode: Arc<AtomicU8>,
    requests: Arc<Mutex<Vec<String>>>,
    worker: tokio::task::JoinHandle<()>,
}

impl Server {
    fn new(runtime: &Runtime) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let mode = Arc::new(AtomicU8::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = requests.clone();
        let state = mode.clone();
        let worker = runtime.spawn(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut bytes = Vec::new();
                loop {
                    let mut chunk = [0; 1024];
                    let size = socket.read(&mut chunk).await.unwrap();
                    assert!(size > 0 && bytes.len() < 8192);
                    bytes.extend_from_slice(&chunk[..size]);
                    if bytes.windows(4).any(|value| value == b"\r\n\r\n") { break; }
                }
                let request = String::from_utf8(bytes).unwrap();
                let line = request.lines().next().unwrap();
                log.lock().unwrap().push(line.to_owned());
                assert!(line.starts_with("GET "), "Recovery attempted a mutation: {line}");
                let path = line.split_whitespace().nth(1).unwrap();
                let mode = state.load(Ordering::SeqCst);
                let (status, body) = reply(path, mode);
                let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        Self {
            base,
            mode,
            requests,
            worker,
        }
    }

    pub fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.worker.abort();
    }
}

fn reply(path: &str, mode: u8) -> (&'static str, String) {
    if path == "/api/people" {
        return ("200 OK", "[]".into());
    }
    if mode == 1 {
        return ("503 Service Unavailable", "{}".into());
    }
    let path = path.strip_prefix("/api/meetings/").expect("meeting read");
    if path.ends_with("/segment-removals") {
        return ("200 OK", "[]".into());
    }
    if let Some(id) = path.strip_suffix("/segments") {
        let mut records = recovered_segments(id);
        if mode == 2 {
            records[0].meeting_id = "foreign".into();
        }
        if mode == 3 {
            records[0].text = "Rewritten finalized words".into();
        }
        ("200 OK", serde_json::to_string(&records).unwrap())
    } else if mode == 4 {
        ("404 Not Found", "{}".into())
    } else {
        (
            "200 OK",
            serde_json::to_string(&meeting(path, true)).unwrap(),
        )
    }
}

pub(super) fn meeting(id: &str, ended: bool) -> Meeting {
    let mut meeting = Meeting::new("Reconnect fixture", 10.0);
    meeting.id = id.into();
    meeting.updated_at = if ended { 21 } else { 20 };
    if ended {
        meeting.status = MeetingStatus::Ended;
        meeting.ended_at_ms = Some(20.0);
    }
    meeting
}

pub(super) fn segment(id: &str) -> TranscriptSegment {
    TranscriptSegment {
        id: format!("{id}-saved"),
        meeting_id: id.into(),
        text: "Saved words".into(),
        start_ms: 0,
        end_ms: 10,
        speaker_id: None,
        confidence: 0.9,
        partial: false,
        speaker_confidence: None,
    }
}

pub(super) fn recovered_segments(id: &str) -> Vec<TranscriptSegment> {
    let mut missed = segment(id);
    missed.id = format!("{id}-missed");
    missed.text = "Missed while disconnected 🗒".into();
    missed.start_ms = 10;
    missed.end_ms = 20;
    vec![segment(id), missed]
}

pub(super) fn fixture(cx: &mut TestAppContext) -> (gpui::Entity<MeetingsHub>, Server) {
    cx.executor().allow_parking();
    let runtime = Arc::new(Runtime::new().unwrap());
    let server = Server::new(&runtime);
    let hub = cx.new(|cx| {
        MeetingsHub::new(
            ServerClient::new(&server.base, &server.base).unwrap(),
            runtime,
            MeetingPrefs::default(),
            cx,
        )
    });
    hub.update(cx, |hub, cx| {
        hub.install_meeting_test_snapshot(
            meeting("reconnect", false),
            vec![segment("reconnect")],
            cx,
        );
        hub.stream_recovery.last_requested = Some("reconnect".into());
    });
    (hub, server)
}
