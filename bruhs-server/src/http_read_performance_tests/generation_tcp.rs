//! Finite generation receipts through real TCP, with verified connection reuse.
mod cases;
mod model;

use super::*;
use bruhs_core::meetings::SqliteMeetingStore;
use bruhs_models::{SummaryGenerationReceipt, SummaryGenerationRequest, SummaryGenerationState};
use cases::Case;
use std::{collections::BTreeSet, net::SocketAddr, sync::Mutex};

#[derive(Serialize)]
struct Measurement {
    case: String,
    action: &'static str,
    backend: &'static str,
    shape: &'static str,
    transport: &'static str,
    history_receipts: usize,
    samples_ms: Vec<f64>,
    response_headers_ms: Vec<f64>,
    first_body_chunk_ms: Vec<f64>,
    warmups: usize,
    request_body_bytes: Vec<usize>,
    response_body_bytes: Vec<usize>,
    response_chunks: Vec<usize>,
    server_observed_peers: Vec<SocketAddr>,
    unique_tcp_connections: usize,
    status: u16,
    response_sequence_sha256: String,
}

fn fingerprint(value: &impl Serialize) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).expect("full finite JSON"))
    )
}

async fn fixture(
    backend: &'static str,
    shape: &'static str,
    transport: &'static str,
    history: usize,
    warmups: usize,
    samples: usize,
    reverse: bool,
) -> Value {
    let directory = tempfile::tempdir().expect("private generation TCP fixture");
    let database = directory.path().join("generations.sqlite3");
    let store: Arc<dyn MeetingStore> = match backend {
        "memory" => Arc::new(InMemoryMeetingStore::new()),
        "sqlite" => Arc::new(
            SqliteMeetingStore::open(&database)
                .await
                .expect("private generation SQLite"),
        ),
        _ => unreachable!("declared backend"),
    };
    let history = if shape == "history" { history } else { 0 };
    let mut model = model::Model::seed(&store, history, shape == "large-summary").await;
    let seeded_receipts = model.receipts();
    let peers = Arc::new(Mutex::new(Vec::new()));
    let host =
        Host::new_with_peers(directory.path(), Some(store.clone()), Some(peers.clone())).await;
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .http1_only()
        .timeout(std::time::Duration::from_secs(5));
    if transport == "fresh" {
        builder = builder.pool_max_idle_per_host(0);
    }
    let client = builder.build().expect("explicit TCP client");
    let mut events = host.state.subscribe_meeting_events();
    let mut cases = Case::ALL;
    if reverse {
        cases.reverse();
    }
    let mut rows = Vec::new();
    let mut fixture_peers = BTreeSet::new();
    for case in cases {
        let mut full = Vec::new();
        let mut headers_times = Vec::new();
        let mut first_times = Vec::new();
        let mut request_lengths = Vec::new();
        let mut response_lengths = Vec::new();
        let mut chunk_counts = Vec::new();
        let mut observed = Vec::new();
        let mut responses = Vec::new();
        let mut status = None;
        for index in 0..warmups + samples {
            model.prepare(case, index, &store).await;
            let (method, path, body) = case.request(index);
            let body = body.map(|value| serde_json::to_vec(&value).expect("request JSON"));
            let request_length = body.as_ref().map_or(0, Vec::len);
            let mut request = client.request(method, format!("{}{path}", host.base));
            if let Some(body) = body {
                request = request
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(body);
            }
            let request = request.build().expect("finite generation request");
            let start = Instant::now();
            let mut response = client
                .execute(request)
                .await
                .expect("finite generation headers");
            let headers_elapsed = start.elapsed().as_secs_f64() * 1000.0;
            let code = response.status().as_u16();
            let headers = response.headers().clone();
            let first = response.chunk().await.expect("first decoded body chunk");
            let first_elapsed = start.elapsed().as_secs_f64() * 1000.0;
            let mut body = first.as_ref().map_or_else(Vec::new, |chunk| chunk.to_vec());
            let mut chunks = usize::from(first.is_some());
            while let Some(chunk) = response.chunk().await.expect("complete decoded body") {
                body.extend_from_slice(&chunk);
                chunks += 1;
            }
            let full_elapsed = start.elapsed().as_secs_f64() * 1000.0;
            let (expected_status, expected) = model.apply(case, index);
            assert_eq!(code, expected_status, "{} status", case.name());
            assert_eq!(headers[reqwest::header::CONTENT_TYPE], "application/json");
            assert_eq!(headers[reqwest::header::CACHE_CONTROL], "no-store");
            assert_eq!(headers[reqwest::header::X_FRAME_OPTIONS], "DENY");
            assert_eq!(headers[reqwest::header::X_CONTENT_TYPE_OPTIONS], "nosniff");
            assert!(!headers.contains_key(reqwest::header::CONTENT_ENCODING));
            assert_eq!(
                headers[reqwest::header::CONTENT_LENGTH]
                    .to_str()
                    .expect("body length")
                    .parse::<usize>()
                    .expect("numeric length"),
                body.len()
            );
            let actual: Value = serde_json::from_slice(&body).expect("complete response JSON");
            assert_eq!(actual, expected, "{} full ordered reply", case.name());
            let peer = {
                let mut log = peers.lock().expect("observed TCP log");
                assert_eq!(log.len(), 1, "one request reached router");
                log.pop().expect("server observed source socket")
            };
            if transport == "fresh" {
                assert!(
                    fixture_peers.insert(peer),
                    "each fresh request gets a new TCP socket"
                );
            } else {
                fixture_peers.insert(peer);
                assert_eq!(fixture_peers.len(), 1, "pooled TCP socket reused");
            }
            observed.push(peer);
            assert!(
                matches!(
                    events.try_recv(),
                    Err(tokio::sync::broadcast::error::TryRecvError::Empty)
                ),
                "receipt operations emit no meeting event"
            );
            model.verify_subject(case, index, &store).await;
            assert!(status.is_none_or(|previous| previous == code));
            status = Some(code);
            responses.push(expected);
            request_lengths.push(request_length);
            response_lengths.push(body.len());
            chunk_counts.push(chunks);
            if index >= warmups {
                full.push(full_elapsed);
                headers_times.push(headers_elapsed);
                first_times.push(first_elapsed);
            }
        }
        let unique = observed.iter().collect::<BTreeSet<_>>().len();
        rows.push(Measurement {
            case: case.name(),
            action: case.action(),
            backend,
            shape,
            transport,
            history_receipts: history,
            samples_ms: full,
            response_headers_ms: headers_times,
            first_body_chunk_ms: first_times,
            warmups,
            request_body_bytes: request_lengths,
            response_body_bytes: response_lengths,
            response_chunks: chunk_counts,
            server_observed_peers: observed,
            unique_tcp_connections: unique,
            status: status.expect("observed status"),
            response_sequence_sha256: fingerprint(&responses),
        });
    }
    model.verify(&store).await;
    let state = model.fingerprint();
    let final_receipts = model.receipts();
    drop(client);
    drop(events);
    host.close().await;
    drop(store);
    if backend == "sqlite" {
        let reopened: Arc<dyn MeetingStore> = Arc::new(
            SqliteMeetingStore::open(&database)
                .await
                .expect("reopen private SQLite"),
        );
        model.verify(&reopened).await;
        drop(reopened);
    }
    let path = directory.path().to_owned();
    directory.close().expect("private generation cleanup");
    assert!(!path.exists());
    json!({"backend":backend,"shape":shape,"transport":transport,"seeded_receipts":seeded_receipts,"final_receipts":final_receipts,"final_state_sha256":state,"cases":rows})
}

async fn run(warmups: usize, samples: usize, history: usize, reverse: bool) -> Vec<Value> {
    let mut backends = ["memory", "sqlite"];
    let mut shapes = ["ordinary", "history", "large-summary"];
    let mut transports = ["pooled", "fresh"];
    if reverse {
        backends.reverse();
        shapes.reverse();
        transports.reverse();
    }
    let mut fixtures = Vec::new();
    for backend in backends {
        for shape in shapes {
            for transport in transports {
                fixtures.push(
                    fixture(
                        backend, shape, transport, history, warmups, samples, reverse,
                    )
                    .await,
                );
            }
        }
    }
    fixtures
}

#[tokio::test]
async fn generation_tcp_preserves_receipts_and_proves_socket_reuse() {
    assert_eq!(run(0, 2, 3, false).await.len(), 12);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "private generation TCP timings; run optimized without builders or other measurements"]
async fn generation_tcp_cost() {
    let smoke = std::env::var_os("BRUHS_PERF_GENERATION_TCP_SMOKE").is_some();
    let reverse =
        std::env::var("BRUHS_PERF_GENERATION_TCP_ORDER").is_ok_and(|value| value == "reverse");
    let (warmups, samples) = if smoke { (1, 2) } else { (5, 30) };
    let fixtures = run(warmups, samples, 1001, reverse).await;
    let output =
        std::env::var_os("BRUHS_PERF_GENERATION_TCP_OUTPUT").expect("explicit TCP result path");
    std::fs::write(output, serde_json::to_vec_pretty(&json!({
        "schema":1,"smoke":smoke,"reverse":reverse,"fixtures":fixtures,
        "runtime":{"flavor":"multi_thread","workers":2,"requests":"serial"},
        "scope":"Production router, HTTP/1.1 over private IPv4 loopback TCP, private memory/disk SQLite, reqwest chunk assembly. Pooled versus idle pooling disabled; every server-observed source socket checked. Durations start before execute and end at response headers, first decoded body chunk, and complete assembled decoded body. Headers/first chunk are application observations, not packet-level TTFB. Fresh samples include TCP establishment; pooled primary samples reuse one warm connection. Setup/client/request construction, pending-receipt seeding, validations, hashing, SQLite reopen and cleanup excluded. Validation warms storage. Payload sizes are decoded bodies, not total wire traffic. No DNS, TLS, remote link/provider/capture, concurrency, loss, cold-storage or UI claim.",
        "semantics":"Every full decoded reply/status/cache/security/length header checked. Every request verifies target receipt, pending identity and unchanged meeting, with no emitted meeting event. Full known meetings/transcripts/removals/local-context/people/receipts checked at seed/end and after SQLite reopen. Fresh pending identities are seeded before each request; fresh fence identities have never been seen. Terminal successes must retain full summaries. No normalization. Response sequence hashes cover warmups plus samples; compare smoke separately."
    })).expect("TCP measurement JSON")).expect("write TCP measurements");
    println!("GENERATION_TCP_COST 324 variants; complete responses/state, socket reuse, reopen and cleanup passed");
}
