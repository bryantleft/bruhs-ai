//! Browser-to-server-to-daemon transport, with private owned TCP peers at both hops.

use super::*;
use axum::{
    body::Bytes,
    extract::ConnectInfo,
    http::{header, HeaderMap, StatusCode, Uri},
    response::IntoResponse,
};
use bruhs_core::capture_client::{DaemonEndpoint, PROXY_REPLY_LIMIT};
use std::{collections::BTreeSet, net::SocketAddr, sync::Mutex};

const TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const NEXT_TOKEN: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

struct Scenario {
    name: String,
    method: Method,
    path: &'static str,
    upstream_path: &'static str,
    request: Bytes,
    request_type: Option<&'static str>,
    reply: Bytes,
    reply_type: &'static str,
    reply_status: u16,
    expected_status: u16,
    expected: Bytes,
    expected_type: &'static str,
    upstream_requests: bool,
    down: bool,
}

fn json_bytes(value: Value) -> Bytes {
    serde_json::to_vec(&value).expect("fixture JSON").into()
}

fn gateway(message: &str) -> Bytes {
    json_bytes(json!({"error":message}))
}

fn scenarios() -> Vec<Scenario> {
    let mut cases = Vec::new();
    for (name, method, path, upstream_path, request) in [
        (
            "start",
            Method::POST,
            "/api/capture/start",
            "/capture/start",
            br#"{"meeting_id":"meeting","started_at_ms":42.0}"#.as_slice(),
        ),
        (
            "stop",
            Method::POST,
            "/api/capture/stop",
            "/capture/stop",
            b"".as_slice(),
        ),
        (
            "monitor-read",
            Method::GET,
            "/api/capture/activity-monitor",
            "/activity-monitor",
            b"".as_slice(),
        ),
        (
            "monitor-command",
            Method::POST,
            "/api/capture/activity-monitor",
            "/activity-monitor",
            br#"{"operation":"disable","epoch":"fixture"}"#.as_slice(),
        ),
    ] {
        for shape in ["ordinary", "refused", "reply-limit", "oversized-reply"] {
            let reply = match shape {
                "ordinary" => Bytes::from_static(br#"{"ok":true,"fixture":"daemon-transport"}"#),
                "refused" => Bytes::from_static(b"private daemon refusal"),
                "reply-limit" => Bytes::from(vec![0x85; PROXY_REPLY_LIMIT]),
                "oversized-reply" => Bytes::from(vec![0x85; PROXY_REPLY_LIMIT + 1]),
                _ => unreachable!(),
            };
            let (expected_status, expected, expected_type) = match shape {
                "refused" => (502, gateway("capture daemon rejected the server's token; restart the capture daemon or check BRUHS_CAPTURE_TOKEN_FILE"), "application/json"),
                "oversized-reply" => (502, gateway("capture daemon request failed"), "application/json"),
                _ => (200, reply.clone(), if shape == "reply-limit" { "application/octet-stream" } else { "application/json" }),
            };
            cases.push(Scenario {
                name: format!("{name}/{shape}"),
                method: method.clone(),
                path,
                upstream_path,
                request: Bytes::copy_from_slice(request),
                request_type: None,
                reply,
                reply_type: if shape == "ordinary" {
                    "application/json"
                } else {
                    "application/octet-stream"
                },
                reply_status: if shape == "refused" { 401 } else { 200 },
                expected_status,
                expected,
                expected_type,
                upstream_requests: true,
                down: false,
            });
        }
        if matches!(name, "start" | "monitor-command") {
            for size in [64 * 1024, crate::MAX_BODY_BYTES] {
                let reply = Bytes::from_static(b"invalid request payload");
                cases.push(Scenario {
                    name: format!("{name}/rejected-{size}"),
                    method: method.clone(),
                    path,
                    upstream_path,
                    request: Bytes::from(vec![b'x'; size]),
                    request_type: Some("application/octet-stream"),
                    reply: reply.clone(),
                    reply_type: "text/plain; charset=utf-8",
                    reply_status: 422,
                    expected_status: 422,
                    expected: reply,
                    expected_type: "text/plain; charset=utf-8",
                    upstream_requests: true,
                    down: false,
                });
            }
        }
    }
    let over = Scenario {
        name: "start/request-over-limit".into(),
        method: Method::POST,
        path: "/api/capture/start",
        upstream_path: "/capture/start",
        request: Bytes::from(vec![b'x'; crate::MAX_BODY_BYTES + 1]),
        request_type: Some("application/json"),
        reply: Bytes::new(),
        reply_type: "application/json",
        reply_status: 200,
        expected_status: 413,
        expected: Bytes::from_static(b"Failed to buffer the request body: length limit exceeded"),
        expected_type: "text/plain; charset=utf-8",
        upstream_requests: false,
        down: false,
    };
    cases.push(over);
    let status = json!({"recording":false,"meeting_id":null,"started_at_ms":null,
        "system_audio_permission":"granted","system_audio_capturing":false});
    for shape in ["idle", "recording", "malformed", "refused", "unreachable"] {
        let mut reply = status.clone();
        if shape == "recording" {
            reply["recording"] = json!(true);
            reply["meeting_id"] = json!("meeting");
            reply["started_at_ms"] = json!(42.0);
        }
        let mut expected = reply.clone();
        expected["running"] = json!(true);
        let (expected_status, expected) = match shape {
            "malformed" => (502, gateway("capture daemon status request failed")),
            "refused" => (502, gateway("capture daemon rejected the server's token; restart the capture daemon or check BRUHS_CAPTURE_TOKEN_FILE")),
            "unreachable" => (200, json_bytes(json!({"running":false,"recording":false,"meeting_id":null,
                "started_at_ms":null,"system_audio_permission":"","system_audio_capturing":false}))),
            _ => (200, json_bytes(expected)),
        };
        cases.push(Scenario {
            name: format!("status/{shape}"),
            method: Method::GET,
            path: "/api/capture/status",
            upstream_path: "/status",
            request: Bytes::new(),
            request_type: None,
            reply: if shape == "malformed" {
                Bytes::from_static(b"{")
            } else {
                json_bytes(reply)
            },
            reply_type: "application/json",
            reply_status: if shape == "refused" { 401 } else { 200 },
            expected_status,
            expected,
            expected_type: "application/json",
            upstream_requests: shape != "unreachable",
            down: shape == "unreachable",
        });
    }
    assert_eq!(cases.len(), 26);
    cases
}

struct Received {
    method: Method,
    path: String,
    authorization: Option<String>,
    content_type: Option<String>,
    body: Bytes,
    peer: SocketAddr,
}

struct Daemon {
    base: String,
    seen: Arc<Mutex<Vec<Received>>>,
    server: Option<tokio::task::JoinHandle<()>>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    _reservation: Option<tokio::net::TcpSocket>,
}

impl Daemon {
    async fn new(case: &Scenario) -> Self {
        let seen = Arc::new(Mutex::new(Vec::new()));
        if case.down {
            let socket = tokio::net::TcpSocket::new_v4().expect("private refused socket");
            socket
                .bind("127.0.0.1:0".parse().expect("loopback"))
                .expect("reserve refused port");
            return Self {
                base: format!("http://{}", socket.local_addr().expect("reserved address")),
                seen,
                server: None,
                shutdown: None,
                _reservation: Some(socket),
            };
        }
        let record = seen.clone();
        let reply = case.reply.clone();
        let content_type = case.reply_type;
        let status = StatusCode::from_u16(case.reply_status).expect("fixture status");
        let router = axum::Router::new().fallback(
            move |ConnectInfo(peer): ConnectInfo<SocketAddr>,
                  method: Method,
                  uri: Uri,
                  headers: HeaderMap,
                  body: Bytes| {
                let record = record.clone();
                let reply = reply.clone();
                async move {
                    let text = |name| {
                        headers.get(name).map(|v: &axum::http::HeaderValue| {
                            v.to_str().expect("fixture header").to_owned()
                        })
                    };
                    record.lock().expect("daemon log").push(Received {
                        method,
                        path: uri.path().to_owned(),
                        authorization: text(header::AUTHORIZATION),
                        content_type: text(header::CONTENT_TYPE),
                        body,
                        peer,
                    });
                    (status, [(header::CONTENT_TYPE, content_type)], reply).into_response()
                }
            },
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("private daemon listener");
        let base = format!("http://{}", listener.local_addr().expect("daemon address"));
        let (shutdown, receiver) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async {
                let _ = receiver.await;
            })
            .await
            .expect("daemon server");
        });
        Self {
            base,
            seen,
            server: Some(server),
            shutdown: Some(shutdown),
            _reservation: None,
        }
    }

    async fn close(mut self) {
        if let Some(sender) = self.shutdown.take() {
            sender.send(()).expect("daemon shutdown");
        }
        if let Some(mut server) = self.server.take() {
            match tokio::time::timeout(std::time::Duration::from_secs(5), &mut server).await {
                Ok(result) => result.expect("daemon task"),
                Err(_) => {
                    server.abort();
                    panic!("daemon fixture shutdown deadline");
                }
            }
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        if let Some(sender) = self.shutdown.take() {
            let _ = sender.send(());
        }
        if let Some(server) = &self.server {
            server.abort();
        }
    }
}

async fn measure(
    case: &Scenario,
    usage: &str,
    warmups: usize,
    samples: usize,
    rotate: bool,
) -> Value {
    let directory = tempfile::tempdir().expect("private capture fixture");
    let token_path = directory.path().join("daemon-token");
    std::fs::write(&token_path, TOKEN).expect("synthetic private token");
    let daemon = Daemon::new(case).await;
    let peers = Arc::new(Mutex::new(Vec::new()));
    let host = Host::new_with_capture_daemon(
        directory.path(),
        None,
        Some(peers.clone()),
        Some(DaemonEndpoint::new(&daemon.base, Some(token_path.clone()))),
    )
    .await;
    let mut events = host.state.subscribe_meeting_events();
    let mut intervals = Vec::new();
    let mut constructor_ms = Vec::new();
    let mut upstream_peers = Vec::new();
    let mut response_sizes = Vec::new();
    let mut results = Sha256::new();
    let mut requests = Sha256::new();
    for index in 0..warmups + samples {
        if rotate && index == 1 {
            std::fs::write(&token_path, NEXT_TOKEN).expect("rotate token");
        }
        let owned;
        let client = if usage == "fresh" {
            let start = Instant::now();
            owned = reqwest::Client::builder()
                .no_proxy()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .expect("fresh browser transport");
            constructor_ms.push(start.elapsed().as_secs_f64() * 1000.0);
            &owned
        } else {
            &host.client
        };
        let mut request = client
            .request(case.method.clone(), format!("{}{}", host.base, case.path))
            .header(
                header::AUTHORIZATION,
                "Bearer browser-credential-must-not-reach-daemon",
            )
            .header(header::ORIGIN, "http://127.0.0.1:8080")
            .body(case.request.clone());
        if let Some(content_type) = case.request_type {
            request = request.header(header::CONTENT_TYPE, content_type);
        }
        let request = request.build().expect("browser request");
        let start = Instant::now();
        let response = client
            .execute(request)
            .await
            .expect("two-hop capture response");
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let body = response
            .bytes()
            .await
            .expect("complete capture response body");
        intervals.push(start.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(status, case.expected_status, "{} status", case.name);
        assert_eq!(
            headers[header::CONTENT_TYPE],
            case.expected_type,
            "{} response type",
            case.name
        );
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        assert_eq!(headers[header::X_FRAME_OPTIONS], "DENY");
        if case.expected_type == "application/json" {
            let actual: Value = serde_json::from_slice(&body).expect("complete capture JSON");
            let expected: Value =
                serde_json::from_slice(&case.expected).expect("fixture expected JSON");
            assert_eq!(actual, expected, "{} full JSON", case.name);
        } else {
            assert_eq!(
                body, case.expected,
                "{} complete opaque response",
                case.name
            );
        }
        assert!(!body
            .windows(TOKEN.len())
            .any(|part| part == TOKEN.as_bytes()));
        results.update(status.to_le_bytes());
        results.update(&body);
        response_sizes.push(body.len());
        let log = std::mem::take(&mut *daemon.seen.lock().expect("daemon request log"));
        assert_eq!(
            log.len(),
            usize::from(case.upstream_requests),
            "{} upstream count",
            case.name
        );
        for received in log {
            assert_eq!(received.method, case.method);
            assert_eq!(received.path, case.upstream_path);
            let token = if rotate && index >= 1 {
                NEXT_TOKEN
            } else {
                TOKEN
            };
            assert_eq!(
                received.authorization.as_deref(),
                Some(format!("Bearer {token}").as_str())
            );
            assert_eq!(
                received.body, case.request,
                "{} forwarded request bytes",
                case.name
            );
            assert_eq!(
                received.content_type.as_deref(),
                if case.request.is_empty() {
                    None
                } else {
                    Some(case.request_type.unwrap_or("application/json"))
                }
            );
            requests.update(received.method.as_str());
            requests.update(&received.path);
            requests.update(&received.body);
            upstream_peers.push(received.peer.to_string());
        }
        assert!(host
            .state
            .meetings()
            .list_meetings()
            .await
            .expect("meetings unchanged")
            .is_empty());
        assert!(host
            .state
            .meetings()
            .list_people()
            .await
            .expect("people unchanged")
            .is_empty());
        assert!(matches!(
            events.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
    }
    let incoming: Vec<_> = peers
        .lock()
        .expect("browser peer log")
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(incoming.len(), warmups + samples);
    let incoming_count = incoming.iter().collect::<BTreeSet<_>>().len();
    if usage == "fresh" {
        assert_eq!(incoming_count, warmups + samples);
    } else if case.name != "start/request-over-limit" {
        assert_eq!(incoming_count, 1);
    }
    let row = json!({"case":case.name,"action":format!("{} {}",case.method,case.path),"usage":usage,
        "request_bytes":case.request.len(),"upstream_reply_bytes":case.reply.len(),"status":case.expected_status,
        "response_bytes":response_sizes,"result_sequence_sha256":format!("{:x}",results.finalize()),
        "forwarded_sequence_sha256":format!("{:x}",requests.finalize()),
        "samples_ms":intervals[warmups..],"all_request_ms":intervals,"warmups":warmups,
        "fresh_client_construction_ms":constructor_ms,"server_observed_browser_peers":incoming,
        "daemon_observed_server_peers":upstream_peers,"incoming_connections":incoming_count,
        "upstream_connections":upstream_peers.iter().collect::<BTreeSet<_>>().len(),
        "upstream_requests":usize::from(case.upstream_requests)*(warmups+samples),"rotated_token":rotate});
    host.close().await;
    daemon.close().await;
    assert_eq!(
        std::fs::read_to_string(&token_path).expect("retained token"),
        if rotate { NEXT_TOKEN } else { TOKEN }
    );
    directory.close().expect("private capture cleanup");
    row
}

enum ScenarioSet {
    All,
    ListeningDaemon,
}

async fn run(
    warmups: usize,
    samples: usize,
    reverse: bool,
    rotate: bool,
    selection: ScenarioSet,
) -> Vec<Value> {
    let mut cases = scenarios();
    if matches!(selection, ScenarioSet::ListeningDaemon) {
        cases.retain(|case| !case.down);
    }
    let mut usages = ["pooled", "fresh"];
    if reverse {
        cases.reverse();
        usages.reverse();
    }
    let mut rows = Vec::new();
    for usage in usages {
        for case in &cases {
            rows.push(measure(case, usage, warmups, samples, rotate).await);
        }
    }
    rows
}

#[tokio::test]
async fn two_hop_capture_transport_preserves_payloads_errors_limits_and_rotated_tokens() {
    assert_eq!(run(0, 2, false, true, ScenarioSet::All).await.len(), 52);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "optimized two-hop payload timing; unavailable deadline remains in full correctness test"]
async fn capture_proxy_network_cost() {
    let smoke = std::env::var_os("BRUHS_PERF_CAPTURE_PROXY_SMOKE").is_some();
    let reverse = std::env::var("BRUHS_PERF_CAPTURE_PROXY_ORDER").is_ok_and(|v| v == "reverse");
    let (warmups, samples) = if smoke { (1, 2) } else { (5, 30) };
    let rows = run(
        warmups,
        samples,
        reverse,
        false,
        ScenarioSet::ListeningDaemon,
    )
    .await;
    assert_eq!(rows.len(), 50);
    let output =
        std::env::var_os("BRUHS_PERF_CAPTURE_PROXY_OUTPUT").expect("capture fixture output path");
    std::fs::write(output, serde_json::to_vec_pretty(&json!({"schema":1,"smoke":smoke,"reverse":reverse,
        "cases":rows,"runtime":{"flavor":"multi_thread","workers":2,"requests":"serial"},
        "omitted_latency_cases":["status/unreachable"],
        "scope":"Actual production router, real private browser-server-daemon TCP hops. Synthetic private token file re-read by actual shared daemon client. Fake daemon records complete requests and supplies deterministic status/opaque reply bodies; no capture hardware, real recording, provider, DNS/TLS or remote network. 25 listening-daemon scenarios x pooled/fresh incoming transport. The unchanged unavailable deadline remains in the full26-scenario correctness test and prior capture-proxy1 raw evidence; it is excluded only from this copy-latency cohort. Public execute-through-complete-response bytes clock includes both network hops, token read, request/response buffering. Request building, fresh incoming client construction, full checks/hashes and cleanup excluded. Full response and forwarded bytes, headers/status, unchanged empty meetings/people and no events checked every call. 1MiB input cap and64KiB reply cap cases explicit. Default daemon proxy/redirect/timeout policy unchanged; no numerical or concurrent/impaired/remote acceptance."})).expect("capture results JSON")).expect("write capture results");
}
