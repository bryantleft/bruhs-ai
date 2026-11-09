use super::*;
use std::sync::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(crate) struct ServerState {
    pub meeting: Meeting,
    pub posts: usize,
    pub patches: usize,
    pub gets: usize,
    pub mode: u8,
    pub delay_ms: u64,
    pub bodies: Vec<serde_json::Value>,
    pub intent_file: Option<std::path::PathBuf>,
    pub observed_intent: bool,
    pub fail_result_save: bool,
    pub generation_receipt: Option<bruhs_models::SummaryGenerationReceipt>,
    pub receipt_gets: usize,
    pub receipt_deletes: usize,
    pub receipt_status: u16,
    pub receipt_payload: Option<serde_json::Value>,
    pub pending_payload: Option<serde_json::Value>,
    pub pending_gets: usize,
}

pub(crate) struct ActionServer {
    pub base: String,
    pub state: Arc<Mutex<ServerState>>,
    worker: tokio::task::JoinHandle<()>,
}

pub(crate) fn meeting() -> Meeting {
    let mut meeting = Meeting::new("Native summary fixture", 10.0);
    meeting.id = "action-meeting".into();
    meeting.updated_at = 20;
    meeting
}

pub(crate) fn summary() -> bruhs_models::MeetingSummary {
    bruhs_models::MeetingSummary {
        tldr: "Generated from fixture transcript 🗒".into(),
        action_items: vec![],
        decisions: vec![],
        topics: vec![],
        generated_by: "synthetic".into(),
        generated_at: 21,
    }
}

impl ActionServer {
    pub fn new(runtime: &Runtime) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("owned listener");
        listener.set_nonblocking(true).expect("nonblocking");
        let base = format!("http://{}", listener.local_addr().expect("address"));
        let state = Arc::new(Mutex::new(ServerState {
            meeting: meeting(),
            posts: 0,
            patches: 0,
            gets: 0,
            mode: 0,
            delay_ms: 0,
            bodies: vec![],
            intent_file: None,
            observed_intent: false,
            fail_result_save: false,
            generation_receipt: None,
            receipt_gets: 0,
            receipt_deletes: 0,
            receipt_status: 200,
            receipt_payload: None,
            pending_payload: None,
            pending_gets: 0,
        }));
        let requests = state.clone();
        let worker = runtime.spawn(async move {
            let listener = tokio::net::TcpListener::from_std(listener).expect("async listener");
            let mut handlers = tokio::task::JoinSet::new();
            while let Ok((stream, _)) = listener.accept().await {
                if handlers.len() >= 16 {
                    let _ = handlers.join_next().await;
                }
                let requests = requests.clone();
                handlers.spawn(serve(stream, requests));
                while handlers.try_join_next().is_some() {}
            }
        });
        Self {
            base,
            state,
            worker,
        }
    }
}

impl Drop for ActionServer {
    fn drop(&mut self) {
        self.worker.abort();
    }
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> String {
    let mut bytes = Vec::new();
    loop {
        let mut buffer = [0; 1024];
        let count = stream.read(&mut buffer).await.expect("request read");
        assert!(count > 0 && bytes.len() < 8192, "bounded fixture request");
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(end) = bytes.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]);
            let length = headers
                .lines()
                .find_map(|line| {
                    line.to_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().expect("length"))
                })
                .unwrap_or(0);
            if bytes.len() >= end + 4 + length {
                break;
            }
        }
    }
    String::from_utf8(bytes).expect("request UTF-8")
}

async fn serve(mut stream: tokio::net::TcpStream, state: Arc<Mutex<ServerState>>) {
    let request = read_request(&mut stream).await;
    let (status, body, delay) = response(&request, &mut state.lock().expect("fixture state"));
    tokio::time::sleep(Duration::from_millis(delay)).await;
    let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    // A deadline test is allowed to disconnect its owned request.
    let _ = stream.write_all(response.as_bytes()).await;
}

fn response(request: &str, state: &mut ServerState) -> (&'static str, String, u64) {
    let (method, rest) = request.split_once(' ').expect("method");
    let path = rest.split(' ').next().expect("path");
    if method == "POST" && path.ends_with("/summarize") {
        state.posts += 1;
        let body: serde_json::Value =
            serde_json::from_str(request.split_once("\r\n\r\n").expect("body").1).expect("JSON");
        if let Some(path) = &state.intent_file {
            let saved: serde_json::Value = serde_json::from_slice(
                &std::fs::read(path).expect("owned intent file before POST"),
            )
            .expect("saved JSON");
            state.observed_intent = saved["meeting_actions"]["action-meeting"]["token"]
                .as_str()
                .is_some_and(|token| {
                    uuid::Uuid::parse_str(token).is_ok()
                        && Some(token) == body["request_id"].as_str()
                        && saved["meeting_actions"]["action-meeting"]["generation"]["request_id"]
                            == body["request_id"]
                        && saved["meeting_actions"]["action-meeting"]["generation"]["meeting_id"]
                            == "action-meeting"
                        && saved["meeting_actions"]["action-meeting"]["generation"]
                            ["extract_observations"]
                            == body["extract_observations"]
                        && saved["meeting_actions"]["action-meeting"]["generation"]
                            ["replace_observations"]
                            == body["replace_observations"]
                });
            if state.fail_result_save {
                let backup = path.with_file_name("state.json.bak");
                if backup.is_file() {
                    std::fs::remove_file(&backup).expect("remove only owned fixture backup");
                }
                std::fs::create_dir(&backup).expect("owned backup failure fixture");
            }
        }
        state.generation_receipt = Some(bruhs_models::SummaryGenerationReceipt {
            request: bruhs_models::SummaryGenerationRequest {
                meeting_id: state.meeting.id.clone(),
                request_id: body["request_id"].as_str().expect("request ID").into(),
                extract_observations: body["extract_observations"].as_bool().expect("extract"),
                replace_observations: body["replace_observations"].as_bool().expect("replace"),
            },
            state: bruhs_models::SummaryGenerationState::Succeeded { summary: summary() },
        });
        state.bodies.push(body);
        state.meeting.summary = Some(summary());
        state.meeting.updated_at += 1;
        let status = if state.mode == 5 {
            "503 Service Unavailable"
        } else {
            "200 OK"
        };
        return (
            status,
            serde_json::to_string(&summary()).expect("summary"),
            state.delay_ms,
        );
    }
    if method == "PATCH" && path.ends_with("/skip_observations") {
        state.patches += 1;
        let body: serde_json::Value =
            serde_json::from_str(request.split_once("\r\n\r\n").expect("body").1).expect("JSON");
        state.meeting.skip_observations = body["skip"].as_bool().expect("skip");
        state.meeting.updated_at += 1;
        let mut updated = state.meeting.clone();
        if state.mode == 6 {
            updated.skip_observations = !updated.skip_observations;
        }
        return (
            "200 OK",
            serde_json::to_string(&updated).expect("meeting"),
            state.delay_ms,
        );
    }
    if path == "/api/people" {
        return ("200 OK", "[]".into(), 0);
    }
    if path == "/api/capture/status" {
        return (
            "200 OK",
            r#"{"running":false,"recording":false,"meeting_id":null,"started_at_ms":null}"#.into(),
            0,
        );
    }
    if path == "/api/meetings/action-meeting/segments"
        || path == "/api/meetings/action-meeting/segment-removals"
    {
        return ("200 OK", "[]".into(), 0);
    }
    if path == "/api/meetings" {
        return (
            "200 OK",
            serde_json::to_string(&vec![state.meeting.clone()]).expect("list"),
            0,
        );
    }
    if method == "GET" && path == "/api/meetings/action-meeting/generations" {
        state.pending_gets += 1;
        if state.receipt_status != 200 {
            return ("503 Service Unavailable", "{}".into(), 0);
        }
        let body = match &state.pending_payload {
            Some(payload) => payload.to_string(),
            None => "null".into(),
        };
        return ("200 OK", body, 0);
    }
    if method == "DELETE" && path.starts_with("/api/meetings/action-meeting/generations/") {
        state.receipt_deletes += 1;
        if state.receipt_status != 200 {
            return ("503 Service Unavailable", "{}".into(), 0);
        }
        if let Some(payload) = &state.receipt_payload {
            return ("200 OK", payload.to_string(), 0);
        }
        let body: serde_json::Value =
            serde_json::from_str(request.split_once("\r\n\r\n").expect("body").1).expect("JSON");
        let request_id = path.rsplit('/').next().expect("request id").to_string();
        let receipt = state.generation_receipt.get_or_insert_with(|| {
            bruhs_models::SummaryGenerationReceipt {
                request: bruhs_models::SummaryGenerationRequest {
                    meeting_id: "action-meeting".into(),
                    request_id,
                    extract_observations: body["extract_observations"].as_bool().expect("extract"),
                    replace_observations: body["replace_observations"].as_bool().expect("replace"),
                },
                state: bruhs_models::SummaryGenerationState::Cancelled,
            }
        });
        if matches!(receipt.state, bruhs_models::SummaryGenerationState::Pending) {
            receipt.state = bruhs_models::SummaryGenerationState::Cancelled;
        }
        return (
            "200 OK",
            serde_json::to_string(receipt).expect("receipt"),
            0,
        );
    }
    if method == "GET" && path.starts_with("/api/meetings/action-meeting/generations/") {
        state.receipt_gets += 1;
        if state.receipt_status != 200 {
            return ("503 Service Unavailable", "{}".into(), 0);
        }
        if let Some(payload) = &state.receipt_payload {
            return ("200 OK", payload.to_string(), 0);
        }
        return match &state.generation_receipt {
            Some(receipt) => (
                "200 OK",
                serde_json::to_string(receipt).expect("receipt"),
                0,
            ),
            None => ("404 Not Found", "{}".into(), 0),
        };
    }
    assert_eq!((method, path), ("GET", "/api/meetings/action-meeting"));
    state.gets += 1;
    if state.mode == 1 {
        return ("503 Service Unavailable", "{}".into(), 0);
    }
    let mut updated = state.meeting.clone();
    if state.mode == 2 {
        updated.id = "foreign-meeting".into();
    }
    if state.mode == 3 {
        updated.updated_at = 19;
    }
    if state.mode == 4 {
        updated.summary.as_mut().expect("summary").tldr = "Different receipt".into();
    }
    (
        "200 OK",
        serde_json::to_string(&updated).expect("meeting"),
        0,
    )
}
