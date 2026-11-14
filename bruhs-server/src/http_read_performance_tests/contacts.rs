//! Private contact writes, including events, complete state and SQLite reopen checks.

mod cases;

use super::*;
use bruhs_core::meetings::SqliteMeetingStore;
use cases::{Case, Model};

#[derive(Serialize)]
struct WriteMeasurement {
    case: &'static str,
    action: &'static str,
    backend: &'static str,
    background_records: usize,
    shape: &'static str,
    seeded_records: usize,
    final_records: usize,
    samples_ms: Vec<f64>,
    warmups: usize,
    response_bytes: Vec<usize>,
    status: u16,
    response_sequence_sha256: String,
    event_sequence_sha256: String,
    final_state_sha256: String,
}

fn fingerprint(value: &impl Serialize) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).expect("complete fixture value"))
    )
}

async fn measure_write(
    case: Case,
    backend: &'static str,
    background_records: usize,
    shape: &'static str,
    warmups: usize,
    samples: usize,
) -> WriteMeasurement {
    let directory = tempfile::tempdir().expect("private contact write fixture");
    let database = directory.path().join("meetings.sqlite3");
    let store: Arc<dyn MeetingStore> = match backend {
        "memory" => Arc::new(InMemoryMeetingStore::new()),
        "sqlite" => Arc::new(
            SqliteMeetingStore::open(&database)
                .await
                .expect("private SQLite"),
        ),
        _ => unreachable!("declared backend"),
    };
    let mut model = Model::seed(case, background_records, shape, warmups + samples, &store).await;
    let seeded_records = model.people.len();
    let host = Host::new(directory.path(), Some(store.clone())).await;
    let mut events = host.state.subscribe_meeting_events();
    let mut intervals = Vec::new();
    let mut lengths = Vec::new();
    let mut responses = Vec::new();
    let mut emitted = Vec::new();
    let mut status = None;
    for index in 0..warmups + samples {
        let expected = model.next(case, index, backend);
        assert!(expected.path.starts_with("/api/people"));
        let request = host
            .client
            .request(expected.method, format!("{}{}", host.base, expected.path))
            .json(&expected.body)
            .build()
            .expect("contact write request");
        let start = Instant::now();
        let response = host
            .client
            .execute(request)
            .await
            .expect("contact write response");
        let code = response.status().as_u16();
        let headers = response.headers().clone();
        let body = response.bytes().await.expect("complete contact response");
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(code, expected.status, "{} status", case.name());
        assert_eq!(headers[reqwest::header::CONTENT_TYPE], "application/json");
        assert_eq!(headers[reqwest::header::CACHE_CONTROL], "no-store");
        assert_eq!(headers[reqwest::header::X_FRAME_OPTIONS], "DENY");
        assert_eq!(headers[reqwest::header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        let value: Value = serde_json::from_slice(&body).expect("complete contact JSON");
        assert_eq!(value, expected.response, "{} full reply", case.name());
        if let Some(expected_event) = expected.event {
            assert_eq!(
                events.try_recv().expect("committed contact event"),
                expected_event
            );
            emitted.push(expected_event);
        }
        assert!(
            matches!(
                events.try_recv(),
                Err(tokio::sync::broadcast::error::TryRecvError::Empty)
            ),
            "no extra or failed-write event"
        );
        model.verify(&store).await;
        assert!(status.is_none_or(|prior| prior == code));
        status = Some(code);
        responses.push(value);
        lengths.push(body.len());
        if index >= warmups {
            intervals.push(elapsed);
        }
    }
    host.close().await;
    model.verify(&store).await;
    drop(store);
    if backend == "sqlite" {
        let reopened: Arc<dyn MeetingStore> = Arc::new(
            SqliteMeetingStore::open(&database)
                .await
                .expect("reopen committed contacts"),
        );
        model.verify(&reopened).await;
        drop(reopened);
    }
    let result = WriteMeasurement {
        case: case.name(),
        action: case.action(),
        backend,
        background_records,
        shape,
        seeded_records,
        final_records: model.people.len(),
        samples_ms: intervals,
        warmups,
        response_bytes: lengths,
        status: status.expect("observed write status"),
        response_sequence_sha256: fingerprint(&responses),
        event_sequence_sha256: fingerprint(&emitted),
        final_state_sha256: fingerprint(&model),
    };
    let path = directory.path().to_owned();
    directory.close().expect("remove private write fixture");
    assert!(!path.exists(), "private write fixture cleanup");
    result
}

async fn run(
    warmups: usize,
    samples: usize,
    large_count: usize,
    reverse: bool,
) -> Vec<WriteMeasurement> {
    let mut backends = ["memory", "sqlite"];
    let mut datasets = [
        (0, "ordinary"),
        (large_count, "ordinary"),
        (0, "alias-heavy"),
        (0, "observation-heavy"),
    ];
    let mut cases = Case::ALL;
    if reverse {
        backends.reverse();
        datasets.reverse();
        cases.reverse();
    }
    let mut rows = Vec::new();
    for backend in backends {
        for (background, shape) in datasets {
            for case in cases {
                rows.push(measure_write(case, backend, background, shape, warmups, samples).await);
            }
        }
    }
    rows
}

#[tokio::test]
async fn contact_routes_preserve_replies_versions_events_and_reopened_state() {
    assert_eq!(run(0, 2, 3, false).await.len(), 160);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "private actual HTTP writes; run optimized without builders or other measurements"]
async fn contact_write_cost() {
    let smoke = std::env::var_os("BRUHS_PERF_CONTACT_HTTP_SMOKE").is_some();
    let reverse = std::env::var("BRUHS_PERF_CONTACT_HTTP_ORDER").is_ok_and(|v| v == "reverse");
    let (warmups, samples) = if smoke { (1, 2) } else { (5, 30) };
    let rows = run(warmups, samples, 1001, reverse).await;
    assert_eq!(rows.len(), 160);
    let output =
        std::env::var_os("BRUHS_PERF_CONTACT_HTTP_OUTPUT").expect("explicit contact results path");
    std::fs::write(output, serde_json::to_vec_pretty(&json!({
        "schema": 1, "smoke": smoke, "reverse": reverse, "cases": rows,
        "runtime": {"flavor":"multi_thread","workers":2,"requests":"serial"},
        "scope": "Actual production router and pooled private loopback client; private memory/disk SQLite stores. Client execute through full body only. Setup/seeding/request construction, exact response/header/event/full-state checks, SQLite reopen, hashing and cleanup excluded. Each variant owns a fresh store. Validation reads between requests warm the store. Fresh creates grow the store; disjoint fresh merges shrink it; seeded/final counts and every response length are recorded. One direct event subscriber; no live SSE fan-out, concurrency, cold storage, UI, provider or capture claim.",
        "versions": "Fixed client seed version 4000000000000 is ahead of the real clock; existing monotonic version logic increments it exactly. No clock replacement or response normalization. Exhaustion uses i64::MAX. Hashes cover all warmup plus measured complete ordered response/event sequences and complete final state; smoke and primary hashes are compared only within their own sample-count cohorts."
    })).expect("contact measurement JSON")).expect("write contact measurements");
    println!("CONTACT_HTTP_COST 160 variants; full replies, committed events, complete state, reopen and cleanup passed");
}
