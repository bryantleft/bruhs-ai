//! Isolated HTTP/SQLite acceptance: a stopped capture daemon is not an empty transcript.
use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use bruhs_core::MeetingRecords;
use bruhs_core::PeopleStore;
use bruhs_core::SqliteMeetingStore;
use bruhs_models::{Meeting, TranscriptSegment};
use reqwest::Client;
use std::sync::Arc;
use std::{path::Path, process::Stdio, time::Duration};
use tokio::process::{Child, Command};
type CaptureReply = Arc<tokio::sync::RwLock<(StatusCode, serde_json::Value)>>;

#[cfg(unix)]
#[path = "transcript_restart/summary_http.rs"]
mod summary_http;

struct Capture {
    url: String,
    reply: CaptureReply,
    task: tokio::task::JoinHandle<()>,
}

impl Capture {
    async fn start(segments: Vec<TranscriptSegment>) -> Self {
        let reply = Arc::new(tokio::sync::RwLock::new((
            StatusCode::OK,
            serde_json::json!({"segments":segments}),
        )));
        let state = reply.clone();
        let app = Router::new()
            .route(
                "/segments/{id}",
                get(async |State(reply): State<CaptureReply>| {
                    let (status, value) = reply.read().await.clone();
                    (status, Json(value))
                }),
            )
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind isolated capture fixture");
        let url = format!("http://{}", listener.local_addr().expect("capture address"));
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve capture fixture");
        });
        Self { url, reply, task }
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct Server {
    child: Child,
    url: String,
    log: tempfile::NamedTempFile,
}

impl Server {
    async fn start(database: &Path, capture_url: &str, client: &Client) -> Self {
        Self::start_with_provider(database, capture_url, client, None).await
    }

    async fn start_with_provider(
        database: &Path,
        capture_url: &str,
        client: &Client,
        provider_directory: Option<&Path>,
    ) -> Self {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("reserve isolated server port");
        let port = listener.local_addr().expect("server address").port();
        drop(listener);
        // Explicit override lets the same private HTTP fixtures verify the
        // production binary without starting or replacing the user's server.
        let binary = std::env::var_os("BRUHS_TEST_SERVER_BINARY")
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_bruhs-server").into());
        let mut command = Command::new(binary);
        let log = tempfile::NamedTempFile::new().expect("create private server diagnostic log");
        if let Some(directory) = provider_directory {
            command.env("PATH", directory);
        }
        let child = command
            .env("BRUHS_SERVER_PORT", port.to_string())
            .env("BRUHS_MEETING_DB", database)
            .env("BRUHS_CAPTURE_URL", capture_url)
            // Never read the real daemon token: point at a file that is absent.
            .env(
                "BRUHS_CAPTURE_TOKEN_FILE",
                database.with_extension("no-daemon-token"),
            )
            .env_remove("BRUHS_WEB_ORIGIN")
            .env_remove("BRUHS_API_URL")
            .env_remove("BRUHS_GOOGLE_CLIENT_ID")
            .env_remove("BRUHS_GOOGLE_CLIENT_SECRET")
            .stdin(Stdio::null())
            .stdout(log.as_file().try_clone().expect("open server stdout log"))
            .stderr(log.as_file().try_clone().expect("open server stderr log"))
            .kill_on_drop(true)
            .spawn()
            .expect("start isolated server");
        let mut server = Self {
            child,
            url: format!("http://127.0.0.1:{port}"),
            log,
        };
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if let Some(status) = server.child.try_wait().expect("inspect server process") {
                    panic!("isolated server exited {status}: {}", server.diagnostics());
                }
                if client
                    .get(format!("{}/api/meetings", server.url))
                    .send()
                    .await
                    .is_ok()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "isolated server did not become ready: {}",
                server.diagnostics()
            )
        });
        server
    }

    fn diagnostics(&self) -> String {
        use std::io::{Read, Seek, SeekFrom};
        let mut file = self.log.reopen().expect("read private server log");
        let length = file.metadata().expect("inspect private server log").len();
        file.seek(SeekFrom::Start(length.saturating_sub(8192)))
            .expect("seek to bounded server diagnostics");
        let mut bytes = Vec::new();
        file.take(8192)
            .read_to_end(&mut bytes)
            .expect("read bounded server diagnostics");
        String::from_utf8_lossy(&bytes).into_owned()
    }

    async fn stop(&mut self) {
        self.child.kill().await.expect("stop isolated server");
        self.child.wait().await.expect("reap isolated server");
    }
}

fn segment() -> TranscriptSegment {
    TranscriptSegment {
        id: "saved-segment".into(),
        meeting_id: "saved-meeting".into(),
        speaker_id: Some("me".into()),
        start_ms: 0,
        end_ms: 1000,
        text: "Preserve this real saved transcript across server restart.".into(),
        confidence: 1.0,
        partial: false,
        speaker_confidence: None,
    }
}

#[tokio::test]
async fn foreign_browser_requests_cannot_read_or_mutate_the_local_workspace() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("origin-boundary.sqlite3");
    let client = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let mut server = Server::start(&database, "http://127.0.0.1:0", &client).await;
    let endpoint = format!("{}/api/people", server.url);
    for origin in [
        "https://foreign.example",
        "null",
        "http://localhost.foreign.example",
    ] {
        let response = client
            .get(&endpoint)
            .header("origin", origin)
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "foreign read: {origin}"
        );
        assert!(!response
            .headers()
            .contains_key("access-control-allow-origin"));
        let response = client.post(&endpoint).header("origin", origin)
            .json(&serde_json::json!({"id":"foreign-write", "display_name":"Must not exist", "created_at":1,"updated_at":1}))
            .send().await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "foreign write: {origin}"
        );
    }
    let response = client.get(&endpoint).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap(),
        serde_json::json!([])
    );
    let local = client
        .get(&endpoint)
        .header("origin", "http://localhost:8088")
        .send()
        .await
        .unwrap();
    assert_eq!(local.status(), StatusCode::OK);
    assert_eq!(
        local.headers()["access-control-allow-origin"],
        "http://localhost:8088"
    );
    // A `*.localhost` dev name is trusted only when configured
    // (BRUHS_WEB_ORIGIN): any local site can use one.
    let unconfigured = client
        .get(&endpoint)
        .header("origin", "http://bruhs.localhost:8088")
        .send()
        .await
        .unwrap();
    assert_eq!(unconfigured.status(), StatusCode::FORBIDDEN);
    assert_eq!(local.headers()["cache-control"], "no-store");
    let hostile_host = client
        .get(&endpoint)
        .header("host", "rebind.foreign.example")
        .send()
        .await
        .unwrap();
    assert_eq!(hostile_host.status(), StatusCode::FORBIDDEN);
    let no_origin = client
        .get(&endpoint)
        .header("sec-fetch-site", "cross-site")
        .send()
        .await
        .unwrap();
    assert_eq!(no_origin.status(), StatusCode::FORBIDDEN);
    let oauth = client
        .get(format!("{}/api/calendar/google/oauth/callback", server.url))
        .header("sec-fetch-site", "cross-site")
        .send()
        .await
        .unwrap();
    assert_eq!(oauth.status(), StatusCode::OK);
    assert!(oauth
        .text()
        .await
        .unwrap()
        .contains("Missing code or state parameter"));
    server.stop().await;
}

#[tokio::test]
async fn failed_summary_regeneration_preserves_existing_observations() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("summary.sqlite3");
    let store = SqliteMeetingStore::open(&database).await.unwrap();
    let meeting = Meeting::new("Summary recovery", 0.0);
    store.create_meeting(meeting.clone()).await.unwrap();
    let mut person = bruhs_models::PersonProfile::new("Summary fixture person");
    person.observations.push(bruhs_models::PersonObservation {
        at_meeting_id: meeting.id.clone(),
        kind: bruhs_models::ObservationKind::Topic,
        text: "Keep this observation if regeneration fails.".into(),
        created_at: 1,
    });
    let person = store.upsert_person(person).await.unwrap();
    let client = Client::new();
    let mut server = Server::start(&database, "http://127.0.0.1:0", &client).await;
    let response = client
        .post(format!(
            "{}/api/meetings/{}/summarize",
            server.url, meeting.id
        ))
        .json(&serde_json::json!({"replace_observations":true}))
        .send()
        .await
        .unwrap();
    assert!(!response.status().is_success());
    server.stop().await;
    assert_eq!(store.get_person(&person.id).await.unwrap(), Some(person));
    assert_eq!(store.get_meeting(&meeting.id).await.unwrap(), Some(meeting));
}

#[tokio::test]
async fn saved_transcript_is_returned_after_restart_without_capture_daemon() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("transcripts.sqlite3");
    let store = SqliteMeetingStore::open(&database).await.unwrap();
    let mut meeting = Meeting::new("Saved meeting", 1.0);
    meeting.id = "saved-meeting".into();
    store.create_meeting(meeting).await.unwrap();
    store.append_segment(segment()).await.unwrap();
    drop(store);
    let client = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let mut server = Server::start(&database, "http://127.0.0.1:0", &client).await;
    let response = client
        .get(format!(
            "{}/api/meetings/saved-meeting/segments",
            server.url
        ))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let saved: Vec<TranscriptSegment> = response.json().await.unwrap();
    server.stop().await;
    assert!(status.is_success());
    assert_eq!(saved, vec![segment()]);
}

#[tokio::test]
async fn captured_transcript_is_committed_before_http_success_and_survives_capture_and_server_restart(
) {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("transcripts.sqlite3");
    let store = SqliteMeetingStore::open(&database).await.unwrap();
    let mut meeting = Meeting::new("Capture fixture", 1.0);
    meeting.id = "saved-meeting".into();
    store.create_meeting(meeting).await.unwrap();
    let capture = Capture::start(vec![segment()]).await;
    let client = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let mut server = Server::start(&database, &capture.url, &client).await;
    let url = format!("{}/api/meetings/saved-meeting/segments", server.url);
    let response = client.get(&url).send().await.unwrap();
    assert_eq!(response.headers()["x-bruhs-transcript-source"], "capture");
    assert_eq!(
        response.json::<Vec<TranscriptSegment>>().await.unwrap(),
        vec![segment()]
    );
    assert_eq!(
        store.list_segments("saved-meeting").await.unwrap(),
        vec![segment()]
    );

    let mut diarized = segment();
    diarized.speaker_id = Some("them_1".into());
    *capture.reply.write().await = (StatusCode::OK, serde_json::json!({"segments":[diarized]}));
    assert_eq!(
        client
            .get(&url)
            .send()
            .await
            .unwrap()
            .json::<Vec<TranscriptSegment>>()
            .await
            .unwrap(),
        vec![diarized.clone()]
    );
    for payload in [
        serde_json::json!({"segments":[]}),
        serde_json::json!({"segments":[segment(), segment()]}),
    ] {
        *capture.reply.write().await = (StatusCode::OK, payload);
        let response = client.get(&url).send().await.unwrap();
        assert_eq!(response.headers()["x-bruhs-transcript-refresh"], "rejected");
        assert_eq!(
            response.json::<Vec<TranscriptSegment>>().await.unwrap(),
            vec![diarized.clone()]
        );
    }
    drop(capture);
    server.stop().await;
    let mut reopened = Server::start(&database, "http://127.0.0.1:0", &client).await;
    let response = client
        .get(format!(
            "{}/api/meetings/saved-meeting/segments",
            reopened.url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.headers()["x-bruhs-transcript-source"], "saved");
    let retained = response.json::<Vec<TranscriptSegment>>().await.unwrap();
    reopened.stop().await;
    assert_eq!(retained, vec![diarized]);
}

#[tokio::test]
async fn unavailable_uncached_transcript_and_unknown_meeting_are_not_successful_empty_lists() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("transcripts.sqlite3");
    let store = SqliteMeetingStore::open(&database).await.unwrap();
    let mut meeting = Meeting::new("Uncached fixture", 1.0);
    meeting.id = "saved-meeting".into();
    store.create_meeting(meeting).await.unwrap();
    let client = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let mut server = Server::start(&database, "http://127.0.0.1:0", &client).await;
    let missing = client
        .get(format!("{}/api/meetings/unknown/segments", server.url))
        .send()
        .await
        .unwrap()
        .status();
    let uncached = client
        .get(format!(
            "{}/api/meetings/saved-meeting/segments",
            server.url
        ))
        .send()
        .await
        .unwrap()
        .status();
    server.stop().await;
    assert_eq!(missing, StatusCode::NOT_FOUND);
    assert_eq!(uncached, StatusCode::SERVICE_UNAVAILABLE);
}
