use super::*;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use bruhs_core::SpeakerLinkJobs;
use serde::Deserialize;
use tokio::sync::Mutex;

fn receipt(sequence: i64, id: &str) -> CaptureRecoveryReceipt {
    CaptureRecoveryReceipt {
        sequence,
        token: "00000000-0000-4000-8000-000000000001".into(),
        meeting_id: id.into(),
    }
}

#[derive(Default)]
struct Source {
    pending: Vec<CaptureRecoveryReceipt>,
    acks: Vec<CaptureRecoveryReceipt>,
    reject_ack: bool,
    invalid_body: Option<String>,
}

struct Fixture {
    url: String,
    source: Arc<Mutex<Source>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    fn daemon(&self) -> bruhs_core::capture_client::DaemonEndpoint {
        bruhs_core::capture_client::DaemonEndpoint::new(self.url.clone(), None)
    }

    async fn start(pending: Vec<CaptureRecoveryReceipt>) -> Self {
        #[derive(Deserialize)]
        struct Cursor {
            after: i64,
        }
        let source = Arc::new(Mutex::new(Source {
            pending,
            ..Default::default()
        }));
        let app = Router::new()
            .route(
                "/recovery",
                get(
                    async |State(source): State<Arc<Mutex<Source>>>,
                           Query(cursor): Query<Cursor>| {
                        let source = source.lock().await;
                        source.invalid_body.clone().unwrap_or_else(|| {
                            serde_json::to_string(&CaptureRecoveryPage {
                                receipts: source
                                    .pending
                                    .iter()
                                    .filter(|receipt| receipt.sequence > cursor.after)
                                    .take(CAPTURE_RECOVERY_PAGE_SIZE)
                                    .cloned()
                                    .collect(),
                            })
                            .unwrap()
                        })
                    },
                ),
            )
            .route(
                "/recovery/ack",
                post(
                    async |State(source): State<Arc<Mutex<Source>>>,
                           Json(receipts): Json<Vec<CaptureRecoveryReceipt>>| {
                        let mut source = source.lock().await;
                        if source.reject_ack {
                            return StatusCode::SERVICE_UNAVAILABLE;
                        }
                        source.pending.retain(|receipt| !receipts.contains(receipt));
                        source.acks.extend(receipts);
                        StatusCode::NO_CONTENT
                    },
                ),
            )
            .with_state(source.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { url, source, task }
    }
}

async fn create(state: &AppState, id: &str) {
    let mut meeting = bruhs_models::Meeting::new("Synthetic", 1.0);
    meeting.id = id.into();
    state.meetings().create_meeting(meeting).await.unwrap();
}

#[tokio::test]
async fn server_restart_discovers_completion_without_sse_or_browser() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.sqlite3");
    let state = AppState::builder()
        .meetings(Arc::new(
            bruhs_core::SqliteMeetingStore::open(&path).await.unwrap(),
        ))
        .build();
    create(&state, "offline").await;
    drop(state);
    let fixture = Fixture::start(vec![receipt(1, "offline")]).await;
    let state = AppState::builder()
        .meetings(Arc::new(
            bruhs_core::SqliteMeetingStore::open(&path).await.unwrap(),
        ))
        .build();
    assert_eq!(
        transfer_page(&state, &reqwest::Client::new(), &fixture.daemon(), 0)
            .await
            .unwrap(),
        1
    );
    assert!(fixture.source.lock().await.pending.is_empty());
    drop(state);
    let store = bruhs_core::SqliteMeetingStore::open(&path).await.unwrap();

    assert_eq!(
        store
            .next_speaker_link(i64::MAX)
            .await
            .unwrap()
            .unwrap()
            .meeting_id,
        "offline"
    );
}

#[tokio::test]
async fn unknown_meetings_are_retained_without_starving_later_pages() {
    let state = AppState::new();
    create(&state, "known").await;
    let mut pending: Vec<_> = (1..=64)
        .map(|sequence| receipt(sequence, &format!("unknown-{sequence}")))
        .collect();
    pending.push(receipt(65, "known"));
    let fixture = Fixture::start(pending).await;
    let client = reqwest::Client::new();
    let next = transfer_page(&state, &client, &fixture.daemon(), 0)
        .await
        .unwrap();
    assert_eq!(next, 64);
    assert!(fixture.source.lock().await.acks.is_empty());
    assert_eq!(
        transfer_page(&state, &client, &fixture.daemon(), next)
            .await
            .unwrap(),
        65
    );
    assert_eq!(fixture.source.lock().await.acks, vec![receipt(65, "known")]);
    assert_eq!(
        transfer_page(&state, &client, &fixture.daemon(), 65)
            .await
            .unwrap(),
        0
    );
    create(&state, "unknown-1").await;
    transfer_page(&state, &client, &fixture.daemon(), 0)
        .await
        .unwrap();
    assert_eq!(fixture.source.lock().await.pending.len(), 63);
}

#[tokio::test]
async fn rejected_ack_retains_both_copies_and_retry_finishes_transfer() {
    let state = AppState::new();
    create(&state, "retry").await;
    let fixture = Fixture::start(vec![receipt(1, "retry")]).await;
    fixture.source.lock().await.reject_ack = true;
    let client = reqwest::Client::new();
    assert_eq!(
        transfer_page(&state, &client, &fixture.daemon(), 0)
            .await
            .unwrap(),
        1
    );
    assert_eq!(fixture.source.lock().await.pending.len(), 1);
    assert!(state
        .meetings()
        .next_speaker_link(i64::MAX)
        .await
        .unwrap()
        .is_some());
    fixture.source.lock().await.reject_ack = false;
    transfer_page(&state, &client, &fixture.daemon(), 0)
        .await
        .unwrap();
    assert!(fixture.source.lock().await.pending.is_empty());
}

#[tokio::test]
async fn oversized_or_malformed_pages_do_not_enqueue_or_ack_prefix() {
    let state = AppState::new();
    create(&state, "valid").await;
    let fixture = Fixture::start(vec![receipt(1, "valid"), receipt(1, "duplicate-sequence")]).await;
    let client = reqwest::Client::new();
    assert!(matches!(
        transfer_page(&state, &client, &fixture.daemon(), 0).await,
        Err(RecoveryError::InvalidReceipt(1))
    ));
    assert!(state
        .meetings()
        .next_speaker_link(i64::MAX)
        .await
        .unwrap()
        .is_none());
    fixture.source.lock().await.invalid_body = Some("x".repeat(MAX_PAGE_BYTES + 1));
    assert!(matches!(
        transfer_page(&state, &client, &fixture.daemon(), 0).await,
        Err(RecoveryError::TooLarge)
    ));
    // A malformed body keeps the decode error as its source.
    fixture.source.lock().await.invalid_body = Some("{".into());
    let error = transfer_page(&state, &client, &fixture.daemon(), 0)
        .await
        .unwrap_err();
    assert!(matches!(error, RecoveryError::Malformed(_)));
    assert!(std::error::Error::source(&error).is_some());
    assert!(fixture.source.lock().await.acks.is_empty());
}

#[test]
fn receipt_validation_rejects_paths_invalid_tokens_and_oversized_batches() {
    let mut invalid_token = receipt(1, "valid");
    invalid_token.token = "x".repeat(36);
    for receipts in [
        vec![receipt(0, "valid")],
        vec![receipt(1, "../path")],
        vec![invalid_token],
        (1..=65)
            .map(|sequence| receipt(sequence, "valid"))
            .collect(),
    ] {
        assert!(validate_page(&CaptureRecoveryPage { receipts }, 0).is_err());
    }
}
