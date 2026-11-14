//! Isolated actual-router fixtures. No process startup/background services,
//! provider calls, capture requests or personal storage are used.

mod fixture;

use std::time::Instant;

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};

use fixture::{Fixture, Frames};

#[derive(Clone, Copy, Debug)]
enum Case {
    Health,
    All,
    ThreadHit,
    ThreadMiss,
    DeleteFinished,
    DeleteRunning,
    DeleteMissing,
    TasksOpened,
    CombinedOpened,
    TasksChanged,
    CombinedChanged,
}

const CASES: [Case; 10] = [
    Case::All,
    Case::ThreadHit,
    Case::ThreadMiss,
    Case::DeleteFinished,
    Case::DeleteRunning,
    Case::DeleteMissing,
    Case::TasksOpened,
    Case::CombinedOpened,
    Case::TasksChanged,
    Case::CombinedChanged,
];

impl Case {
    const fn name(self) -> &'static str {
        match self {
            Self::Health => "health",
            Self::All => "tasks-all",
            Self::ThreadHit => "tasks-thread-hit",
            Self::ThreadMiss => "tasks-thread-miss",
            Self::DeleteFinished => "delete-finished",
            Self::DeleteRunning => "delete-running",
            Self::DeleteMissing => "delete-missing",
            Self::TasksOpened => "tasks-stream-opened",
            Self::CombinedOpened => "combined-stream-opened",
            Self::TasksChanged => "tasks-stream-changed",
            Self::CombinedChanged => "combined-stream-changed",
        }
    }

    const fn stream(self) -> Option<(&'static str, bool)> {
        match self {
            Self::TasksOpened | Self::TasksChanged => Some(("/api/tasks/stream", false)),
            Self::CombinedOpened | Self::CombinedChanged => Some(("/api/events/stream", true)),
            _ => None,
        }
    }

    const fn changes(self) -> bool {
        matches!(self, Self::TasksChanged | Self::CombinedChanged)
    }
}

#[derive(Serialize)]
struct Measurement {
    case: &'static str,
    records: usize,
    boundary: &'static str,
    samples_ms: Vec<f64>,
    warmups: usize,
    normalized_response_sha256: String,
    response_bytes: usize,
    normalized_response_bytes: usize,
    status: u16,
}

struct Observation {
    elapsed_ms: f64,
    normalized: Value,
    response_bytes: usize,
    status: u16,
}

async fn finite(fixture: &Fixture, case: Case) -> Observation {
    let ephemeral = match case {
        Case::DeleteFinished | Case::DeleteRunning => {
            let task = fixture.extra_task();
            if matches!(case, Case::DeleteFinished) {
                fixture.state.registry().mark_stream_ended("http-event");
            }
            Some(task)
        }
        _ => None,
    };
    let request = match case {
        Case::Health => fixture.client.get(fixture.url("/api/health")),
        Case::All => fixture.client.get(fixture.url("/api/tasks")),
        Case::ThreadHit => fixture
            .client
            .get(fixture.url("/api/tasks"))
            .query(&[("thread_id", fixture::SPARSE_THREAD)]),
        Case::ThreadMiss => fixture
            .client
            .get(fixture.url("/api/tasks"))
            .query(&[("thread_id", "missing-http-thread")]),
        Case::DeleteFinished | Case::DeleteRunning => fixture.client.delete(fixture.url(&format!(
            "/api/tasks/{}",
            ephemeral.as_ref().expect("prepared deletion target").id
        ))),
        Case::DeleteMissing => fixture
            .client
            .delete(fixture.url("/api/tasks/missing-http-task")),
        _ => unreachable!("finite route case"),
    }
    .build()
    .expect("fixture request");
    let start = Instant::now();
    let response = fixture
        .client
        .execute(request)
        .await
        .expect("HTTP response");
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let body = response.bytes().await.expect("complete finite body");
    let elapsed_ms = start.elapsed().as_secs_f64() * 1_000.0;
    fixture::assert_headers(&headers);
    let value = if body.is_empty() {
        Value::Null
    } else {
        assert_eq!(headers[reqwest::header::CONTENT_TYPE], "application/json");
        serde_json::from_slice(&body).expect("finite JSON reply")
    };
    let normalized = match case {
        Case::Health => {
            assert_eq!(status, 200);
            fixture::assert_health(&value);
            value
        }
        Case::All | Case::ThreadHit | Case::ThreadMiss => {
            assert_eq!(status, 200);
            let filter = match case {
                Case::ThreadHit => Some(fixture::SPARSE_THREAD),
                Case::ThreadMiss => Some("missing-http-thread"),
                _ => None,
            };
            fixture.canonical_tasks(value, filter, None)
        }
        Case::DeleteFinished => {
            assert_eq!(status, 204);
            assert!(body.is_empty());
            assert!(fixture
                .state
                .registry()
                .get(&ephemeral.as_ref().expect("finished target").id)
                .is_none());
            Value::Null
        }
        Case::DeleteRunning => {
            assert_eq!(status, 409);
            let task = ephemeral.as_ref().expect("running target");
            assert_eq!(
                value,
                json!({"error": format!("background task {} is still running", task.id)})
            );
            assert_eq!(
                fixture
                    .state
                    .registry()
                    .get(&task.id)
                    .expect("running task retained")
                    .status,
                bruhs_core::TaskStatus::Running
            );
            json!({"error": "background task <fixture-id> is still running"})
        }
        Case::DeleteMissing => {
            assert_eq!(status, 404);
            assert_eq!(
                value,
                json!({"error": "no background task missing-http-task"})
            );
            value
        }
        _ => unreachable!("finite validation case"),
    };
    if let Some(task) = ephemeral {
        fixture.cleanup(&task);
    }
    assert_eq!(
        fixture.state.registry().get_all().len(),
        fixture.tasks.len()
    );
    Observation {
        elapsed_ms,
        normalized,
        response_bytes: body.len(),
        status,
    }
}

async fn stream(fixture: &Fixture, case: Case) -> Observation {
    let (path, combined) = case.stream().expect("stream case");
    let request = fixture
        .client
        .get(fixture.url(path))
        .build()
        .expect("SSE request");
    let start = Instant::now();
    let response = fixture.client.execute(request).await.expect("SSE response");
    let status = response.status().as_u16();
    let mut reader = Frames::new(response);
    let initial = reader.opened(combined).await;
    let opened_ms = start.elapsed().as_secs_f64() * 1_000.0;
    assert_eq!(status, 200);
    fixture::assert_headers(reader.headers());
    assert_eq!(
        reader.headers()[reqwest::header::CONTENT_TYPE],
        "text/event-stream"
    );
    let normalized_initial = fixture.canonical_opened(&initial, combined);
    let (elapsed_ms, normalized, response_bytes, extra) = if case.changes() {
        let start = Instant::now();
        let task = fixture.extra_task();
        let frame = reader.next().await;
        let elapsed_ms = start.elapsed().as_secs_f64() * 1_000.0;
        assert_eq!(frame.event(), combined.then_some("tasks"));
        let normalized = fixture.canonical_tasks(frame.value(), None, Some(&task));
        (elapsed_ms, normalized, frame.bytes, Some(task))
    } else {
        (
            opened_ms,
            normalized_initial,
            initial.iter().map(|frame| frame.bytes).sum(),
            None,
        )
    };
    drop(reader);
    if let Some(task) = extra {
        fixture.cleanup(&task);
    }
    assert_eq!(
        fixture.state.registry().get_all().len(),
        fixture.tasks.len()
    );
    Observation {
        elapsed_ms,
        normalized,
        response_bytes,
        status,
    }
}

async fn measure(fixture: &Fixture, case: Case, warmups: usize, samples: usize) -> Measurement {
    let mut samples_ms = Vec::with_capacity(samples);
    let mut reference: Option<(String, usize, usize, u16)> = None;
    for index in 0..warmups + samples {
        let observation = if case.stream().is_some() {
            stream(fixture, case).await
        } else {
            finite(fixture, case).await
        };
        let encoded = serde_json::to_vec(&observation.normalized).expect("canonical response");
        let digest = format!("{:x}", Sha256::digest(&encoded));
        let signature = (
            digest,
            observation.response_bytes,
            encoded.len(),
            observation.status,
        );
        if let Some(expected) = &reference {
            assert_eq!(&signature, expected, "{} stable body/status", case.name());
        } else {
            reference = Some(signature);
        }
        if index >= warmups {
            samples_ms.push(observation.elapsed_ms);
        }
    }
    let (normalized_response_sha256, response_bytes, normalized_response_bytes, status) =
        reference.expect("at least one sample");
    Measurement {
        case: case.name(),
        records: fixture.tasks.len(),
        boundary: if case.changes() {
            "registry registration through complete changed SSE frame"
        } else if case.stream().is_some() {
            "client execute through complete initial semantic SSE frames"
        } else {
            "client execute through complete finite HTTP body"
        },
        samples_ms,
        warmups,
        normalized_response_sha256,
        response_bytes,
        normalized_response_bytes,
        status,
    }
}

async fn run(warmups: usize, samples: usize, sizes: &[usize], reverse: bool) -> Vec<Measurement> {
    let mut results = Vec::new();
    let fixture = Fixture::new(0).await;
    results.push(measure(&fixture, Case::Health, warmups, samples).await);
    fixture.close().await;
    let mut sizes = sizes.to_vec();
    if reverse {
        sizes.reverse();
    }
    for size in sizes {
        let fixture = Fixture::new(size).await;
        let mut cases = CASES;
        if reverse {
            cases.reverse();
        }
        for case in cases {
            results.push(measure(&fixture, case, warmups, samples).await);
        }
        fixture.close().await;
    }
    results
}

#[tokio::test]
async fn actual_router_preserves_task_replies_deletion_and_stream_frames() {
    let measurements = run(0, 1, &[0, 1, 3], false).await;
    assert_eq!(measurements.len(), 31);
}

#[tokio::test]
async fn actual_router_applies_origin_policy_before_task_access() {
    let fixture = Fixture::new(1).await;
    for path in ["/api/tasks", "/api/tasks/stream", "/api/events/stream"] {
        let response = fixture
            .client
            .get(fixture.url(path))
            .header(reqwest::header::ORIGIN, "https://foreign.example")
            .send()
            .await
            .expect("origin rejection");
        assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
        assert_eq!(
            response.json::<Value>().await.expect("error JSON"),
            json!({"error": "Request origin is not allowed to use this local API."})
        );
    }
    let response = fixture
        .client
        .get(fixture.url("/api/tasks"))
        .header(reqwest::header::ORIGIN, "http://localhost:8088")
        .send()
        .await
        .expect("allowed local origin");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response.headers()[reqwest::header::ACCESS_CONTROL_ALLOW_ORIGIN],
        "http://localhost:8088"
    );
    fixture.canonical_tasks(response.json().await.expect("task JSON"), None, None);
    fixture.close().await;
}

#[tokio::test]
async fn actual_combined_stream_replays_only_when_resume_is_requested() {
    let fixture = Fixture::new(1).await;
    for (path, pending) in [
        ("/api/events/stream", false),
        ("/api/events/stream?resume=true", true),
    ] {
        let response = fixture
            .client
            .get(fixture.url(path))
            .send()
            .await
            .expect("combined stream");
        let mut reader = Frames::new(response);
        let frames = reader.opened(true).await;
        fixture.canonical_opened(&frames, true);
        assert_eq!(fixture.state.google_calendar().full_sync_pending(), pending);
        drop(reader);
    }
    fixture.close().await;
}

// Two workers allow the fixture client and server to run concurrently.
// This is a fixture configuration, not production runtime sizing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "isolated release HTTP timing; run separately from builders and other measurements"]
async fn actual_http_action_cost() {
    let smoke = std::env::var_os("BRUHS_PERF_HTTP_SMOKE").is_some();
    let reverse = std::env::var("BRUHS_PERF_HTTP_ORDER").is_ok_and(|value| value == "reverse");
    let (warmups, samples) = if smoke { (1, 2) } else { (5, 30) };
    let results = run(warmups, samples, &[0, 1, 10_001], reverse).await;
    let output = std::env::var_os("BRUHS_PERF_HTTP_OUTPUT").expect("explicit HTTP result path");
    std::fs::write(output, serde_json::to_vec_pretty(&json!({
        "schema": 1, "reverse": reverse, "smoke": smoke, "cases": results,
        "runtime": {"flavor": "multi_thread", "workers": 2, "requests": "serial"},
        "normalization": "Full task objects are compared with all seeded model fields and exact membership first. Only verified generated task IDs/start/end timestamps are replaced; unordered task arrays are sorted. Error UUID replacement follows exact message validation. Fingerprints cover normalized complete payloads; raw decoded body/frame byte counts remain separate.",
        "scope": "Actual production router over private loopback HTTP with in-memory task/account state. Request construction, JSON/SSE semantic validation, normalization, hashing, seed/reset and server startup/shutdown are outside intervals. Minimal body/frame accumulation and boundary detection are inside. Changed SSE interval includes registry registration. No normal browser rendering, personal storage, background loops, provider/capture calls or cold-start claim."
    })).expect("HTTP measurement JSON")).expect("write HTTP measurements");
    println!("HTTP_ACTION_COST 31 cases; full replies/headers, stream payloads and cleanup passed");
}
