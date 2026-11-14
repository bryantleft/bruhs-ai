//! Provider-free parser and two-hop SSE diagnostics; no app startup or capture work.

use super::{parse_sse_data, FrameBuffer};
use crate::http_read_performance_tests::Host;
use axum::{body::Bytes, extract::ConnectInfo, http::StatusCode, routing::get, Router};
use bruhs_core::capture_client::DaemonEndpoint;
use bruhs_models::MeetingEvent;
use futures_core::Stream;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::{convert::Infallible, net::SocketAddr, pin::Pin, sync::Arc, time::Instant};
use tokio_stream::StreamExt as _;

const TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

// Both historical owned frames and current borrowed frames use this same driver.
fn payload(frame: impl AsRef<[u8]>) -> Option<String> {
    parse_sse_data(frame.as_ref())
}

fn decode<'a>(chunks: impl Iterator<Item = &'a [u8]>) -> (Vec<Option<String>>, usize) {
    let mut buffer = FrameBuffer::default();
    let mut decoded = Vec::new();
    for chunk in chunks {
        buffer.push(chunk);
        while let Some(frame) = buffer.next_frame() {
            decoded.push(payload(frame));
        }
    }
    (decoded, buffer.len())
}

fn parser_input(events: usize) -> (Vec<u8>, Vec<Option<String>>) {
    let mut bytes = Vec::new();
    let mut expected = Vec::new();
    for index in 0..events {
        let newline = if index % 2 == 0 { "\n" } else { "\r\n" };
        if index % 17 == 0 {
            bytes.extend_from_slice(format!(": ping{newline}{newline}").as_bytes());
            expected.push(None);
        }
        let first = format!("{index}: café 🐻 {}", "x".repeat(64));
        let second = format!("  tail {index}");
        bytes.extend_from_slice(
            format!("data: {first}{newline}data:{second}{newline}{newline}").as_bytes(),
        );
        expected.push(Some(format!("{first}\n tail {index}")));
    }
    bytes.extend_from_slice(b"data: \xff\n\ndata: unfinished\r\n\r");
    expected.push(None);
    (bytes, expected)
}

#[test]
fn every_two_chunk_split_preserves_frames_and_an_incomplete_tail() {
    let (input, expected) = parser_input(3);
    for split in 0..=input.len() {
        let chunks = [&input[..split], &input[split..]];
        let (actual, remaining) = decode(chunks.into_iter());
        assert_eq!(actual, expected, "split {split}");
        assert_eq!(remaining, b"data: unfinished\r\n\r".len(), "split {split}");
    }
}

#[derive(Serialize)]
struct Row {
    case: String,
    events: usize,
    producer_chunk_bytes: usize,
    payload_bytes: usize,
    output_sha256: String,
    all_intervals_ms: Vec<f64>,
    warmups: usize,
    samples_ms: Vec<f64>,
    delivery: Vec<Delivery>,
}

#[derive(Serialize)]
struct Delivery {
    delivered_positions: Vec<usize>,
    skipped_events: usize,
    reconciliation_frames: usize,
    input_bytes: usize,
    expected_output_bytes: usize,
    observed_output_bytes: usize,
    observed_output_sha256: String,
}

fn parser_rows(warmups: usize, samples: usize, reverse: bool) -> Vec<Row> {
    let mut cases = vec![];
    for events in [1, 32, 512, 8192] {
        for chunk_bytes in [1, 257, usize::MAX] {
            cases.push((events, chunk_bytes));
        }
    }
    if reverse {
        cases.reverse();
    }
    cases
        .into_iter()
        .map(|(events, chunk_bytes)| {
            let (input, expected) = parser_input(events);
            let expected_bytes = serde_json::to_vec(&expected).unwrap();
            let mut all_intervals_ms = Vec::new();
            for _ in 0..warmups + samples {
                let start = Instant::now();
                let (actual, remaining) = decode(std::hint::black_box(&input).chunks(chunk_bytes));
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                assert_eq!(actual, expected);
                assert_eq!(remaining, b"data: unfinished\r\n\r".len());
                all_intervals_ms.push(elapsed);
            }
            Row {
                case: format!("parser/{events}/{}", chunk_bytes.min(input.len())),
                events,
                producer_chunk_bytes: chunk_bytes.min(input.len()),
                payload_bytes: input.len(),
                output_sha256: digest(&expected_bytes),
                samples_ms: all_intervals_ms[warmups..].to_vec(),
                warmups,
                all_intervals_ms,
                delivery: Vec::new(),
            }
        })
        .collect()
}

type ReplyBytes = Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>;

struct Incoming {
    bytes: ReplyBytes,
    buffer: Vec<u8>,
}

impl Incoming {
    // The downstream Axum encoder uses LF framing. Keep this oracle independent
    // of the production daemon parser that is under measurement.
    async fn frame(&mut self) -> Vec<u8> {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(end) = self.buffer.windows(2).position(|bytes| bytes == b"\n\n") {
                    return self.buffer.drain(..end + 2).collect();
                }
                self.buffer
                    .extend_from_slice(&self.bytes.next().await.expect("live SSE reply").unwrap());
            }
        })
        .await
        .expect("SSE fixture liveness deadline")
    }
}

#[derive(Serialize)]
struct Network {
    rows: Vec<Row>,
    upstream_requests: usize,
    upstream_peers: Vec<String>,
    downstream_peers: Vec<String>,
    incoming_handshake_ms: f64,
    pump_handshake_ms: f64,
}

async fn network_rows(warmups: usize, samples: usize, reverse: bool) -> Network {
    let directory = tempfile::tempdir().unwrap();
    let token_file = directory.path().join("private-daemon-token");
    tokio::fs::write(&token_file, TOKEN).await.unwrap();
    let (send, receive) = tokio::sync::mpsc::unbounded_channel::<Vec<Bytes>>();
    let receive = Arc::new(std::sync::Mutex::new(Some(receive)));
    let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = requests.clone();
    let upstream = Router::new().route(
        "/events",
        get(
            move |ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: axum::http::HeaderMap| {
                assert_eq!(headers["authorization"], format!("Bearer {TOKEN}"));
                seen.lock().unwrap().push(peer.to_string());
                let mut receive = receive
                    .lock()
                    .unwrap()
                    .take()
                    .expect("one upstream request");
                async move {
                    axum::response::Response::builder()
                        .status(StatusCode::OK)
                        .header("content-type", "text/event-stream")
                        .body(axum::body::Body::from_stream(async_stream::stream! {
                            while let Some(chunks) = receive.recv().await {
                                for chunk in chunks {
                                    yield Ok::<_, Infallible>(chunk);
                                }
                            }
                        }))
                        .unwrap()
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_address = listener.local_addr().unwrap();
    let upstream_task = tokio::spawn(async move {
        axum::serve(
            listener,
            upstream.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    struct Abort(tokio::task::JoinHandle<()>);
    impl Drop for Abort {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let upstream_task = Abort(upstream_task);
    let peers = Arc::new(std::sync::Mutex::new(Vec::new()));
    let host = Host::new_with_peers(directory.path(), None, Some(peers.clone())).await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let start = Instant::now();
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        client
            .get(format!("{}/api/meetings/stream", host.base))
            .send(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let mut incoming = Incoming {
        bytes: Box::pin(response.bytes_stream()),
        buffer: Vec::new(),
    };
    let opened = b"data: {\"kind\":\"stream_opened\"}\n\n";
    assert_eq!(incoming.frame().await, opened);
    let incoming_handshake_ms = start.elapsed().as_secs_f64() * 1000.0;
    let endpoint = DaemonEndpoint::new(format!("http://{upstream_address}"), Some(token_file));
    assert_eq!(endpoint.token().await.as_deref(), Some(TOKEN));
    let start = Instant::now();
    let pump = super::super::pump::spawn(
        host.state.clone(),
        super::events(endpoint, super::RetryPolicy::default()),
        |_| async { panic!("this event-only fixture must not schedule linking") },
    );
    assert_eq!(incoming.frame().await, opened);
    let pump_handshake_ms = start.elapsed().as_secs_f64() * 1000.0;
    let mut cases = vec![];
    for events in [1, 16, 128] {
        for chunk_bytes in [13, usize::MAX] {
            cases.push((events, chunk_bytes));
        }
    }
    if reverse {
        cases.reverse();
    }
    let mut rows = Vec::new();
    for (events, chunk_bytes) in cases {
        let mut all_intervals_ms = Vec::new();
        let mut output = Vec::new();
        let mut payload_bytes = 0;
        let mut input_size = 0;
        let mut delivery = Vec::new();
        for sample in 0..warmups + samples {
            let mut input = Vec::new();
            let mut expected = Vec::new();
            for event in 0..events {
                let json = serde_json::to_string(&MeetingEvent::PersonChanged {
                    person_id: format!("synthetic-{events}-{sample}-{event}-café-🐻"),
                    updated_at: i64::try_from(sample).unwrap(),
                })
                .unwrap();
                let newline = if event % 2 == 0 { "\n" } else { "\r\n" };
                input.extend_from_slice(
                    format!(": ping{newline}data: {json}{newline}{newline}").as_bytes(),
                );
                expected.push(format!("data: {json}\n\n").into_bytes());
            }
            input_size = input.len();
            let chunks = input
                .chunks(chunk_bytes)
                .map(Bytes::copy_from_slice)
                .collect();
            let start = Instant::now();
            send.send(chunks).unwrap();
            let mut actual = Vec::new();
            loop {
                let frame = incoming.frame().await;
                let last = Some(&frame) == expected.last();
                actual.push(frame);
                if last {
                    break;
                }
            }
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            let mut delivered_positions = Vec::new();
            let mut next = 0;
            let mut reconciliation_frames = 0;
            let mut gap_announced = false;
            for frame in &actual {
                if frame == opened {
                    reconciliation_frames += 1;
                    gap_announced = true;
                    continue;
                }
                let offset = expected[next..]
                    .iter()
                    .position(|candidate| candidate == frame)
                    .expect("every delivered frame is complete, ordered and expected");
                assert!(
                    offset == 0 || gap_announced,
                    "lost events require reconciliation"
                );
                next += offset;
                delivered_positions.push(next);
                next += 1;
                gap_announced = false;
            }
            assert_eq!(next, events);
            let skipped_events = events - delivered_positions.len();
            assert_eq!(skipped_events > 0, reconciliation_frames > 0);
            if events <= 16 {
                assert_eq!(actual, expected);
            }
            payload_bytes = expected.iter().map(Vec::len).sum();
            delivery.push(Delivery {
                delivered_positions,
                skipped_events,
                reconciliation_frames,
                input_bytes: input.len(),
                expected_output_bytes: payload_bytes,
                observed_output_bytes: actual.iter().map(Vec::len).sum(),
                observed_output_sha256: digest(&serde_json::to_vec(&actual).unwrap()),
            });
            output.extend(serde_json::to_vec(&expected).unwrap());
            all_intervals_ms.push(elapsed);
        }
        rows.push(Row {
            case: format!(
                "two-hop/{events}/{}",
                if chunk_bytes == usize::MAX {
                    "batch"
                } else {
                    "fragmented"
                }
            ),
            events,
            producer_chunk_bytes: chunk_bytes.min(input_size),
            payload_bytes,
            output_sha256: digest(&output),
            samples_ms: all_intervals_ms[warmups..].to_vec(),
            warmups,
            all_intervals_ms,
            delivery,
        });
    }
    assert!(host
        .state
        .meetings()
        .list_meetings()
        .await
        .unwrap()
        .is_empty());
    assert!(host
        .state
        .meetings()
        .list_people()
        .await
        .unwrap()
        .is_empty());
    assert!(incoming.buffer.is_empty());
    let upstream_peers = requests.lock().unwrap().clone();
    let downstream_peers: Vec<_> = peers
        .lock()
        .unwrap()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(upstream_peers.len(), 1);
    assert_eq!(downstream_peers.len(), 1);
    drop(incoming);
    drop(pump);
    drop(send);
    host.close().await;
    upstream_task.0.abort();
    Network {
        rows,
        upstream_requests: upstream_peers.len(),
        upstream_peers,
        downstream_peers,
        incoming_handshake_ms,
        pump_handshake_ms,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn coalesced_and_fragmented_frames_preserve_event_or_reconciliation_contract() {
    assert_eq!(parser_rows(0, 1, false).len(), 12);
    assert_eq!(network_rows(0, 1, false).await.rows.len(), 6);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "release-only provider-free SSE diagnostics"]
async fn sse_frame_performance() {
    let output = std::env::var("BRUHS_SSE_PERF_OUT").expect("explicit diagnostic output path");
    let warmups = std::env::var("BRUHS_SSE_PERF_WARMUPS")
        .unwrap_or_else(|_| "3".into())
        .parse::<usize>()
        .unwrap();
    let samples = std::env::var("BRUHS_SSE_PERF_SAMPLES")
        .unwrap_or_else(|_| "20".into())
        .parse::<usize>()
        .unwrap();
    let reverse = std::env::var_os("BRUHS_SSE_PERF_REVERSE").is_some();
    assert!(samples > 0);
    let parser = parser_rows(warmups, samples, reverse);
    let network = network_rows(warmups, samples, reverse).await;
    let result = serde_json::json!({"schema":1,"scope":"Frozen provider-free parser and two real IPv4 TCP hops: synthetic event producer, production daemon parser and pump, actual server router, reqwest downstream. No capture, UI, provider or persistent state. Producer chunk sizes do not assert TCP or reqwest chunk boundaries; byte counts are payload/framing, not wire traffic.","warmups":warmups,"samples":samples,"reverse":reverse,"parser":parser,"network":network});
    std::fs::write(output, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
}
