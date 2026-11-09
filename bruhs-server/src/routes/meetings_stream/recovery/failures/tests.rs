use super::*;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use bruhs_core::MeetingRecords;
use bruhs_models::{
    CaptureFailurePage, CaptureFailureReceipt, Meeting, MeetingStatus, CAPTURE_RECOVERY_PAGE_SIZE,
};
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

#[derive(Default)]
struct Source {
    pending: Vec<CaptureFailureReceipt>,
    reject_ack: bool,
    invalid: Option<String>,
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

    async fn start(pending: Vec<CaptureFailureReceipt>) -> Self {
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
                "/recovery/failures",
                get(
                    async |State(source): State<Arc<Mutex<Source>>>,
                           Query(cursor): Query<Cursor>| {
                        let source = source.lock().await;
                        source.invalid.clone().unwrap_or_else(|| {
                            serde_json::to_string(&CaptureFailurePage {
                                receipts: source
                                    .pending
                                    .iter()
                                    .filter(|r| r.sequence > cursor.after)
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
                "/recovery/failures/ack",
                post(
                    async |State(source): State<Arc<Mutex<Source>>>,
                           Json(receipts): Json<Vec<CaptureFailureReceipt>>| {
                        let mut source = source.lock().await;
                        if source.reject_ack {
                            return StatusCode::SERVICE_UNAVAILABLE;
                        }
                        source.pending.retain(|r| !receipts.contains(r));
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
fn receipt(sequence: i64, id: &str) -> CaptureFailureReceipt {
    CaptureFailureReceipt {
        sequence,
        token: "00000000-0000-4000-8000-000000000001".into(),
        meeting_id: id.into(),
        started_at_ms: 10.0,
        failed_at_ms: 20.0,
    }
}
async fn create(state: &AppState, id: &str) {
    let mut meeting = Meeting::new("Synthetic failure", 10.0);
    meeting.id = id.into();
    state.meetings().create_meeting(meeting).await.unwrap();
}
fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap()
}

#[tokio::test]
async fn disconnected_failure_is_saved_before_ack_and_survives_server_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.sqlite3");
    let state = AppState::builder()
        .meetings(Arc::new(
            bruhs_core::SqliteMeetingStore::open(&path).await.unwrap(),
        ))
        .build();
    create(&state, "offline").await;
    let fixture = Fixture::start(vec![receipt(1, "offline")]).await;
    fixture.source.lock().await.reject_ack = true;
    transfer_page(&state, &client(), &fixture.daemon(), 0)
        .await
        .unwrap();
    let failed = state
        .meetings()
        .get_meeting("offline")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.status, MeetingStatus::Failed);
    assert_eq!(fixture.source.lock().await.pending.len(), 1);
    drop(state);
    let state = AppState::builder()
        .meetings(Arc::new(
            bruhs_core::SqliteMeetingStore::open(&path).await.unwrap(),
        ))
        .build();
    fixture.source.lock().await.reject_ack = false;
    transfer_page(&state, &client(), &fixture.daemon(), 0)
        .await
        .unwrap();
    assert!(fixture.source.lock().await.pending.is_empty());
    assert_eq!(
        state
            .meetings()
            .get_meeting("offline")
            .await
            .unwrap()
            .unwrap(),
        failed
    );
    drop(state);
    let reopened = bruhs_core::SqliteMeetingStore::open(&path).await.unwrap();
    assert_eq!(
        reopened.get_meeting("offline").await.unwrap().unwrap(),
        failed
    );
}

#[tokio::test]
async fn unknown_failures_do_not_starve_known_meetings_and_invalid_pages_are_atomic() {
    let state = AppState::new();
    create(&state, "known").await;
    let mut pending: Vec<_> = (1..=64)
        .map(|n| receipt(n, &format!("unknown-{n}")))
        .collect();
    pending.push(receipt(65, "known"));
    let fixture = Fixture::start(pending).await;
    let original = state
        .meetings()
        .get_meeting("known")
        .await
        .unwrap()
        .unwrap();
    fixture.source.lock().await.invalid =
        Some(serde_json::json!({"receipts": [receipt(1,"known"),receipt(1,"known")]}).to_string());
    assert!(transfer_page(&state, &client(), &fixture.daemon(), 0)
        .await
        .is_err());
    assert_eq!(
        state
            .meetings()
            .get_meeting("known")
            .await
            .unwrap()
            .unwrap(),
        original
    );
    fixture.source.lock().await.invalid = Some("x".repeat(MAX_PAGE_BYTES + 1));
    assert!(transfer_page(&state, &client(), &fixture.daemon(), 0)
        .await
        .is_err());
    fixture.source.lock().await.invalid = None;
    assert_eq!(
        transfer_page(&state, &client(), &fixture.daemon(), 0)
            .await
            .unwrap(),
        64
    );
    assert_eq!(fixture.source.lock().await.pending.len(), 65);
    assert_eq!(
        transfer_page(&state, &client(), &fixture.daemon(), 64)
            .await
            .unwrap(),
        65
    );
    assert_eq!(fixture.source.lock().await.pending.len(), 64);
    assert_eq!(
        state
            .meetings()
            .get_meeting("known")
            .await
            .unwrap()
            .unwrap()
            .status,
        MeetingStatus::Failed
    );
}

#[tokio::test]
async fn rejected_store_transition_keeps_receipt_for_retry_without_acknowledgment() {
    let state = AppState::new();
    let mut meeting = Meeting::new("Version exhausted", 10.0);
    meeting.id = "unsaved".into();
    meeting.updated_at = i64::MAX;
    state
        .meetings()
        .create_meeting(meeting.clone())
        .await
        .unwrap();
    let fixture = Fixture::start(vec![receipt(1, "unsaved")]).await;
    transfer_page(&state, &client(), &fixture.daemon(), 0)
        .await
        .unwrap();
    assert_eq!(fixture.source.lock().await.pending.len(), 1);
    assert_eq!(
        state
            .meetings()
            .get_meeting("unsaved")
            .await
            .unwrap()
            .unwrap(),
        meeting
    );
}
