//! The actual desktop caller, router, TCP transport and isolated stores together.
use super::*;
use crate::desktop_server_client::{ServerClient, ServerError};
use bruhs_core::meetings::SqliteMeetingStore;
use bruhs_models::{LocalFocus, LocalFocusObservation, Meeting};
use std::{collections::BTreeSet, net::SocketAddr, sync::Mutex};

#[derive(Clone, Copy)]
enum Case {
    Read,
    Append,
    Retry,
    ReadMissing,
    AppendMissing,
    Invalid,
}
impl Case {
    const ALL: [Self; 6] = [
        Self::Read,
        Self::Append,
        Self::Retry,
        Self::ReadMissing,
        Self::AppendMissing,
        Self::Invalid,
    ];
    fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Append => "append-fresh",
            Self::Retry => "append-retry",
            Self::ReadMissing => "read-missing",
            Self::AppendMissing => "append-missing",
            Self::Invalid => "append-invalid",
        }
    }
    fn action(self) -> &'static str {
        match self {
            Self::Read | Self::ReadMissing => "GET /api/meetings/{id}/local-focus",
            _ => "POST /api/meetings/{id}/local-focus",
        }
    }
    fn status(self) -> u16 {
        match self {
            Self::AppendMissing => 404,
            Self::Invalid => 400,
            _ => 200,
        }
    }
}

fn sample(id: &str, meeting: &str, at: i64, wide: bool) -> LocalFocusObservation {
    LocalFocusObservation {
        id: id.into(),
        meeting_id: meeting.into(),
        observed_at_ms: at,
        focus: LocalFocus::Available {
            app_name: if wide {
                "Ω".repeat(128)
            } else {
                "Fixture \"Ω\" 😀".into()
            },
            bundle_id: Some(if wide {
                "界".repeat(85)
            } else {
                "test.fixture".into()
            }),
        },
    }
}
fn fingerprint(value: &impl Serialize) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).expect("full native focus value"))
    )
}

#[derive(Serialize)]
struct Measurement {
    case: &'static str,
    action: &'static str,
    backend: &'static str,
    shape: &'static str,
    usage: &'static str,
    seeded_rows: usize,
    final_rows: usize,
    samples_ms: Vec<f64>,
    all_request_ms: Vec<f64>,
    client_construction_ms: Vec<f64>,
    warmups: usize,
    server_observed_peers: Vec<SocketAddr>,
    unique_tcp_connections: usize,
    decoded_result_bytes: Vec<usize>,
    result_sequence_sha256: String,
    final_state_sha256: String,
}

async fn verify(store: &Arc<dyn MeetingStore>, meeting: &Meeting, rows: &[LocalFocusObservation]) {
    assert_eq!(
        store.list_meetings().await.expect("all private meetings"),
        vec![meeting.clone()]
    );
    assert_eq!(
        store
            .list_local_focus(&meeting.id)
            .await
            .expect("full saved focus"),
        rows
    );
    assert!(store
        .list_people()
        .await
        .expect("no private people")
        .is_empty());
    assert!(store
        .list_segments(&meeting.id)
        .await
        .expect("no private transcript")
        .is_empty());
    assert!(store
        .removed_segment_ids(&meeting.id)
        .await
        .expect("no tombstones")
        .is_empty());
    assert!(store
        .pending_summary_generation(&meeting.id)
        .await
        .expect("no pending generation")
        .is_none());
}

fn status_error(error: ServerError, status: u16) -> Value {
    assert_eq!(
        error.to_string(),
        format!("Local context request → {status}")
    );
    match error {
        ServerError::Status {
            request,
            code,
            body,
        } => {
            assert_eq!(code.as_u16(), status);
            assert_eq!(request, "Local context request");
            assert!(body.is_empty());
            json!({"status":status,"request":request,"body":body})
        }
        other => panic!("expected status {status}, got {other:?}"),
    }
}

async fn measure(
    case: Case,
    backend: &'static str,
    shape: &'static str,
    usage: &'static str,
    scale: usize,
    warmups: usize,
    samples: usize,
) -> Measurement {
    let directory = tempfile::tempdir().expect("private native focus fixture");
    let database = directory.path().join("focus.sqlite3");
    let store: Arc<dyn MeetingStore> = match backend {
        "memory" => Arc::new(InMemoryMeetingStore::new()),
        "sqlite" => Arc::new(
            SqliteMeetingStore::open(&database)
                .await
                .expect("private native SQLite"),
        ),
        _ => unreachable!("declared backend"),
    };
    let mut meeting = Meeting::new("Native focus Ω", 42.0);
    meeting.id = "meeting".into();
    meeting.created_at = 42;
    meeting.updated_at = 4_000_000_000_000;
    assert_eq!(
        store
            .create_meeting(meeting.clone())
            .await
            .expect("seed native meeting"),
        meeting
    );
    let count = if shape == "ordinary" { 3 } else { scale };
    let wide = shape == "wide-history";
    let mut rows = Vec::new();
    for index in 0..count {
        let row = sample(
            &format!("focus-{index:06}"),
            "meeting",
            i64::try_from(index + 1).expect("fixture time"),
            wide,
        );
        assert!(row.is_valid());
        store
            .append_local_focus(row.clone())
            .await
            .expect("seed private focus");
        rows.push(row);
    }
    verify(&store, &meeting, &rows).await;
    let peers = Arc::new(Mutex::new(Vec::new()));
    let host =
        Host::new_with_peers(directory.path(), Some(store.clone()), Some(peers.clone())).await;
    let mut events = host.state.subscribe_meeting_events();
    let mut construction = Vec::new();
    let persistent = if usage == "persistent" {
        let start = Instant::now();
        let client = ServerClient::new(&host.base, "http://127.0.0.1:9").expect("native client");
        construction.push(start.elapsed().as_secs_f64() * 1000.0);
        Some(client)
    } else {
        None
    };
    let mut intervals = Vec::new();
    let mut all = Vec::new();
    let mut observed = Vec::new();
    let mut responses = Vec::new();
    let mut lengths = Vec::new();
    for index in 0..warmups + samples {
        let fresh = if usage == "new-client" {
            let start = Instant::now();
            let client =
                ServerClient::new(&host.base, "http://127.0.0.1:9").expect("fresh native client");
            construction.push(start.elapsed().as_secs_f64() * 1000.0);
            Some(client)
        } else {
            None
        };
        let client = fresh.as_ref().or(persistent.as_ref()).expect("caller mode");
        let mut proposed = if matches!(case, Case::Retry) {
            rows[0].clone()
        } else {
            sample(
                &format!("appended-{index:06}"),
                if matches!(case, Case::AppendMissing) {
                    "missing"
                } else {
                    "meeting"
                },
                100_000 + i64::try_from(index).expect("fixture time"),
                wide,
            )
        };
        if matches!(case, Case::Invalid) {
            proposed.observed_at_ms = 0;
        }
        let start = Instant::now();
        let result = match case {
            Case::Read | Case::ReadMissing => client
                .list_local_focus(if matches!(case, Case::ReadMissing) {
                    "missing"
                } else {
                    "meeting"
                })
                .await
                .map(Some),
            _ => client.append_local_focus(&proposed).await.map(|()| None),
        };
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        let response = if matches!(case, Case::AppendMissing | Case::Invalid) {
            status_error(result.expect_err("native rejected request"), case.status())
        } else {
            let actual = result.expect("successful native caller");
            if matches!(case, Case::Read) {
                assert_eq!(actual.as_deref(), Some(rows.as_slice()));
            } else if matches!(case, Case::ReadMissing) {
                assert_eq!(actual.as_deref(), Some([].as_slice()));
            } else {
                assert!(actual.is_none());
                if matches!(case, Case::Append) {
                    rows.push(proposed);
                }
            }
            json!(actual)
        };
        let peer = {
            let mut log = peers.lock().expect("native socket witness");
            assert_eq!(log.len(), 1);
            log.pop().expect("actual native TCP source")
        };
        observed.push(peer);
        verify(&store, &meeting, &rows).await;
        assert!(matches!(
            events.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
        lengths.push(
            serde_json::to_vec(&response)
                .expect("decoded native result")
                .len(),
        );
        responses.push(response);
        all.push(elapsed);
        if index >= warmups {
            intervals.push(elapsed);
        }
    }
    let final_state = fingerprint(
        &json!({"meeting":meeting,"focus":rows,"people":[],"transcript":[],"removals":[],"pending":null}),
    );
    drop(persistent);
    drop(events);
    host.close().await;
    drop(store);
    if backend == "sqlite" {
        let reopened: Arc<dyn MeetingStore> = Arc::new(
            SqliteMeetingStore::open(&database)
                .await
                .expect("reopen private native fixture"),
        );
        verify(&reopened, &meeting, &rows).await;
        drop(reopened);
    }
    let result = Measurement {
        case: case.name(),
        action: case.action(),
        backend,
        shape,
        usage,
        seeded_rows: count,
        final_rows: rows.len(),
        samples_ms: intervals,
        all_request_ms: all,
        client_construction_ms: construction,
        warmups,
        unique_tcp_connections: observed.iter().collect::<BTreeSet<_>>().len(),
        server_observed_peers: observed,
        decoded_result_bytes: lengths,
        result_sequence_sha256: fingerprint(&responses),
        final_state_sha256: final_state,
    };
    let path = directory.path().to_owned();
    directory.close().expect("remove native focus fixture");
    assert!(!path.exists());
    result
}

async fn run(warmups: usize, samples: usize, scale: usize, reverse: bool) -> Vec<Measurement> {
    let mut cases = Case::ALL;
    let mut backends = ["memory", "sqlite"];
    let mut shapes = ["ordinary", "history", "wide-history"];
    let mut usages = ["persistent", "new-client"];
    if reverse {
        cases.reverse();
        backends.reverse();
        shapes.reverse();
        usages.reverse();
    }
    let mut rows = Vec::new();
    for backend in backends {
        for shape in shapes {
            for usage in usages {
                for case in cases {
                    rows.push(measure(case, backend, shape, usage, scale, warmups, samples).await);
                }
            }
        }
    }
    rows
}

#[tokio::test]
async fn actual_native_focus_caller_preserves_state_and_errors() {
    assert_eq!(run(0, 2, 4, false).await.len(), 72);
}

#[tokio::test]
async fn actual_native_focus_transport_is_shared_across_clones() {
    let directory = tempfile::tempdir().expect("private clone fixture");
    let peers = Arc::new(Mutex::new(Vec::new()));
    let store: Arc<dyn MeetingStore> = Arc::new(InMemoryMeetingStore::new());
    let mut meeting = Meeting::new("clone", 42.0);
    meeting.id = "meeting".into();
    store.create_meeting(meeting).await.expect("clone meeting");
    let host = Host::new_with_peers(directory.path(), Some(store), Some(peers.clone())).await;
    let client = ServerClient::new(&host.base, "http://127.0.0.1:9").expect("clone native client");
    let cloned = client.clone();
    assert!(client
        .list_local_focus("meeting")
        .await
        .expect("first read")
        .is_empty());
    assert!(cloned
        .list_local_focus("meeting")
        .await
        .expect("clone read")
        .is_empty());
    let log = peers.lock().expect("clone peers").clone();
    assert_eq!(log.len(), 2);
    assert_eq!(
        log[0], log[1],
        "clones must reuse the dedicated native transport"
    );
    drop(client);
    drop(cloned);
    host.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "actual native local-focus caller through private router; optimized, no builders"]
async fn native_local_focus_cost() {
    let smoke = std::env::var_os("BRUHS_PERF_NATIVE_FOCUS_SMOKE").is_some();
    let reverse =
        std::env::var("BRUHS_PERF_NATIVE_FOCUS_ORDER").is_ok_and(|value| value == "reverse");
    let (warmups, samples) = if smoke { (1, 2) } else { (5, 30) };
    let rows = run(warmups, samples, 1001, reverse).await;
    let output =
        std::env::var_os("BRUHS_PERF_NATIVE_FOCUS_OUTPUT").expect("explicit native focus output");
    std::fs::write(output,serde_json::to_vec_pretty(&json!({"schema":1,"smoke":smoke,"reverse":reverse,"cases":rows,
        "scope":"Actual production desktop ServerClient module compiled in server tests, production HTTP router over private IPv4 TCP, private memory/disk SQLite. Persistent caller or a newly constructed caller per request. Public caller start through complete returned decoded result/status error, including local URL validation/dedicated client initialization and client JSON decode. General ServerClient construction measured separately outside requests, once for persistent and once per call for new-client mode. First request and all warmups retained separately; primary excludes5 warmups. Request model preparation, store/event/error/result checks, fingerprints, reopen and cleanup excluded. Validation warms storage. No GPUI/OS probe/provider/capture/remote/DNS/TLS/wire-byte/concurrent/cold-storage acceptance. Decoded result bytes describe reserialized return values, not HTTP wire/body bytes.",
        "runtime":{"flavor":"multi_thread","workers":2,"requests":"serial"},
        "semantics":"Full ordered observations or exact redacted typed status errors, every request unchanged meeting/known focus/people/transcript/removals/pending and no emitted events, plus SQLite reopen and cleanup. Appends fresh35 identities, retries exact seeded identity, invalid/missing unchanged. No normalization. Full result sequences/state cover warmups+primary; compare smoke separately. Source sockets recorded, pooling behavior assessed outside timing."})).expect("native focus measurements")).expect("write native focus data");
    println!("NATIVE_FOCUS_COST 72 complete caller/router/store variants passed");
}
