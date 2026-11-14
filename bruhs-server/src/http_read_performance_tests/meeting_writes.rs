//! Actual meeting writes and private local context over isolated stores.

mod cases;
mod model;

use super::*;
use bruhs_core::meetings::SqliteMeetingStore;
use cases::Case;
use model::Model;

#[derive(Serialize)]
struct Measurement {
    case: &'static str,
    action: &'static str,
    backend: &'static str,
    shape: &'static str,
    reference_scale: usize,
    seeded_subject_reference_count: usize,
    seeded_subject_distinct_people: usize,
    final_subject_reference_count: usize,
    final_subject_distinct_people: usize,
    seeded_meetings: usize,
    final_meetings: usize,
    samples_ms: Vec<f64>,
    warmups: usize,
    response_bytes: Vec<usize>,
    status: u16,
    canonical_response_sequence_sha256: String,
    event_sequence_sha256: String,
    canonical_final_state_sha256: String,
    normalization_witnesses: Vec<Value>,
}

fn fingerprint(value: &impl Serialize) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).expect("complete meeting fixture value"))
    )
}

async fn measure(
    case: Case,
    backend: &'static str,
    shape: &'static str,
    references: usize,
    warmups: usize,
    samples: usize,
) -> Measurement {
    let directory = tempfile::tempdir().expect("private meeting write fixture");
    let database = directory.path().join("meetings.sqlite3");
    let store: Arc<dyn MeetingStore> = match backend {
        "memory" => Arc::new(InMemoryMeetingStore::new()),
        "sqlite" => Arc::new(
            SqliteMeetingStore::open(&database)
                .await
                .expect("private meeting SQLite"),
        ),
        _ => unreachable!("declared backend"),
    };
    let mut model = Model::seed(case, shape, references, warmups + samples, &store).await;
    let seeded_meetings = model.meetings.len();
    let (seeded_subject_reference_count, seeded_subject_distinct_people) = model.reference_counts();
    let host = Host::new(directory.path(), Some(store.clone())).await;
    let mut events = host.state.subscribe_meeting_events();
    let mut intervals = Vec::new();
    let mut lengths = Vec::new();
    let mut responses = Vec::new();
    let mut emitted = Vec::new();
    let mut witnesses = Vec::new();
    let mut status = None;
    for index in 0..warmups + samples {
        let expected = case.request(index);
        assert!(expected.path.starts_with("/api/meetings"));
        let mut request = host.client.request(
            expected.method.clone(),
            format!("{}{}", host.base, expected.path),
        );
        if let Some(body) = &expected.body {
            request = request.json(body);
        }
        if expected.native {
            request = request.header("x-bruhs-local-context", "1");
        }
        let request = request.build().expect("finite meeting request");
        let not_before = chrono::Utc::now().timestamp_millis();
        let start = Instant::now();
        let response = host
            .client
            .execute(request)
            .await
            .expect("meeting response");
        let code = response.status().as_u16();
        let headers = response.headers().clone();
        let body = response.bytes().await.expect("complete meeting response");
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        let not_after = chrono::Utc::now().timestamp_millis();
        assert_eq!(code, expected.status, "{} status", case.name());
        assert_eq!(headers[reqwest::header::CACHE_CONTROL], "no-store");
        assert_eq!(headers[reqwest::header::X_FRAME_OPTIONS], "DENY");
        assert_eq!(headers[reqwest::header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        let value = if code == 204 {
            assert!(body.is_empty());
            assert!(headers.get(reqwest::header::CONTENT_TYPE).is_none());
            None
        } else {
            assert_eq!(headers[reqwest::header::CONTENT_TYPE], "application/json");
            Some(serde_json::from_slice::<Value>(&body).expect("full meeting JSON"))
        };
        let (canonical, event, normalization) = model.apply(
            case,
            index,
            backend,
            &expected,
            value,
            not_before..=not_after,
        );
        if let Some(expected) = event {
            assert_eq!(
                events.try_recv().expect("committed meeting event"),
                expected
            );
            emitted.push(expected);
        }
        assert!(
            matches!(
                events.try_recv(),
                Err(tokio::sync::broadcast::error::TryRecvError::Empty)
            ),
            "no extra or failed-write event"
        );
        model.verify(&store).await;
        assert!(status.is_none_or(|previous| previous == code));
        status = Some(code);
        responses.push(canonical);
        lengths.push(body.len());
        witnesses.extend(normalization);
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
                .expect("reopen meeting state"),
        );
        model.verify(&reopened).await;
        drop(reopened);
    }
    let (final_subject_reference_count, final_subject_distinct_people) = model.reference_counts();
    let result = Measurement {
        case: case.name(),
        action: case.action(),
        backend,
        shape,
        reference_scale: references,
        seeded_subject_reference_count,
        seeded_subject_distinct_people,
        final_subject_reference_count,
        final_subject_distinct_people,
        seeded_meetings,
        final_meetings: model.meetings.len(),
        samples_ms: intervals,
        warmups,
        response_bytes: lengths,
        status: status.expect("observed meeting status"),
        canonical_response_sequence_sha256: fingerprint(&responses),
        event_sequence_sha256: fingerprint(&emitted),
        canonical_final_state_sha256: fingerprint(&model.canonical_state()),
        normalization_witnesses: witnesses,
    };
    let path = directory.path().to_owned();
    directory.close().expect("remove private meeting fixture");
    assert!(!path.exists(), "meeting fixture cleanup");
    result
}

async fn run(warmups: usize, samples: usize, references: usize, reverse: bool) -> Vec<Measurement> {
    let mut backends = ["memory", "sqlite"];
    let mut shapes = [
        "ordinary",
        "repeated-assignees",
        "unique-assignees",
        "large-person",
        "repeated-large-person",
        "large-summary",
    ];
    let mut cases = Case::ALL;
    if reverse {
        backends.reverse();
        shapes.reverse();
        cases.reverse();
    }
    let mut result = Vec::new();
    for backend in backends {
        for shape in shapes {
            for case in cases {
                result.push(measure(case, backend, shape, references, warmups, samples).await);
            }
        }
    }
    result
}

#[tokio::test]
async fn meeting_routes_preserve_complete_state_events_and_normalization_witnesses() {
    assert_eq!(run(0, 2, 3, false).await.len(), 372);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "private actual meeting HTTP; run optimized without builders or other measurements"]
async fn meeting_write_cost() {
    let smoke = std::env::var_os("BRUHS_PERF_MEETING_HTTP_SMOKE").is_some();
    let reverse =
        std::env::var("BRUHS_PERF_MEETING_HTTP_ORDER").is_ok_and(|value| value == "reverse");
    let (warmups, samples) = if smoke { (1, 2) } else { (5, 30) };
    let rows = run(warmups, samples, 1001, reverse).await;
    assert_eq!(rows.len(), 372);
    let output =
        std::env::var_os("BRUHS_PERF_MEETING_HTTP_OUTPUT").expect("explicit meeting result path");
    std::fs::write(output,serde_json::to_vec_pretty(&json!({
        "schema":1,"smoke":smoke,"reverse":reverse,"cases":rows,
        "runtime":{"flavor":"multi_thread","workers":2,"requests":"serial"},
        "scope":"Actual production router and pooled private loopback client, private memory/disk SQLite stores, one direct meeting-event subscriber. Client execute through full body only. Setup, seeding, request construction, full response/header/event/contact/meeting/segment/removal/focus validation, normalization/hashing, reopen and cleanup excluded. Validation reads warm the store. Fresh creates/deletes and new-speaker links change cardinality deterministically. No provider/capture, live SSE delivery, UI, concurrency or cold-storage claim.",
        "canonicalization":"Every complete decoded response and known stored model is checked before normalization. Only fresh validated unique UUID-v4 meeting IDs and the in-memory skip-observations real-clock updated_at are canonicalized, with per-request witnesses. Fixed started times make create timestamps exact. Other mutation versions use future seed 4000000000000. Full canonical response/event/final-state fingerprints cover warmup plus measured sequences; compare smoke and primary separately. Raw decoded lengths remain separate."
    })).expect("meeting measurement JSON")).expect("write meeting measurements");
    println!("MEETING_HTTP_COST 372 variants; complete replies/events/state, normalization witnesses, reopen and cleanup passed");
}
