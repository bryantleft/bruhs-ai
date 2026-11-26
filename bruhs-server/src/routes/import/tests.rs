//! The import routes over HTTP, through the real router and its guard: a
//! temp batch root with the test caps, plenty of free space, and a
//! scratch agents home (F17). Nothing reads the owner's files: every
//! owner request either names a temp folder or stops before a reader.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use axum::Extension;
use bruhs_core::import::agents::AgentRoots;
use bruhs_core::import::test_support::{fixture as sample, zip_of};
use bruhs_core::import::{ImportHost, Limits};
use bruhs_core::AppState;
use bruhs_models::imports::{
    ImportBatch, ImportJob, ImportJobState, EXPORT_TOO_LARGE, FOLDER_MISSING, IMPORT_BUSY,
    IMPORT_CHAT_GONE, IMPORT_GONE, IMPORT_INVALID, OWNER_ONLY_IMPORT, TOO_MANY_FILES,
};
use bruhs_models::logic::portable::{ImportSource, ImportedThread};
use reqwest::multipart::{Form, Part};
use reqwest::Method;
use serde_json::json;

/// A loopback page the shared guard lets through but that isn't the app.
const OTHER_PAGE: &str = "http://localhost:5173";
/// An id of the right shape that names no batch.
const NO_BATCH: &str = "0123456789abcdef0123456789abcdef";

/// Every import route is restricted to the owner.
const ROUTES: &[(&str, &str)] = &[
    ("GET", "/api/import/sessions"),
    ("GET", "/api/import/thread"),
    ("POST", "/api/import/uploads"),
    (
        "GET",
        "/api/import/uploads/0123456789abcdef0123456789abcdef",
    ),
    (
        "DELETE",
        "/api/import/uploads/0123456789abcdef0123456789abcdef",
    ),
    ("POST", "/api/import/files"),
    ("GET", "/api/import/agents"),
];

struct Fixture {
    base: String,
    client: reqwest::Client,
    server: tokio::task::JoinHandle<()>,
    root: PathBuf,
    home: PathBuf,
    _directory: tempfile::TempDir,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("core").join("imports");
        let home = directory.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let host = ImportHost::new(root.clone())
            .with_limits(Limits::TEST)
            .with_agent_roots(AgentRoots::under(&home));
        #[cfg(unix)]
        let host = host.with_free_space(Arc::new(|_: &Path| Ok(u64::MAX)));
        let app = crate::router(server_state(directory.path())).layer(Extension(host));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap();
        Self {
            base,
            client,
            server,
            root,
            home,
            _directory: directory,
        }
    }

    fn request(&self, method: &str, path: &str) -> reqwest::RequestBuilder {
        self.client.request(
            Method::from_bytes(method.as_bytes()).unwrap(),
            format!("{}{path}", self.base),
        )
    }

    /// The status, `Cache-Control` and raw body of `request`.
    async fn send(request: reqwest::RequestBuilder) -> (StatusCode, String) {
        let (status, cache, text) = Self::send_full(request).await;
        assert_eq!(cache.as_deref(), Some("no-store"), "{status}: {text}");
        (status, text)
    }

    async fn send_full(request: reqwest::RequestBuilder) -> (StatusCode, Option<String>, String) {
        let response = request.send().await.unwrap();
        let status = response.status();
        let cache = response
            .headers()
            .get("cache-control")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        (status, cache, response.text().await.unwrap())
    }

    async fn upload(&self, form: Form) -> (StatusCode, String) {
        Self::send(self.request("POST", "/api/import/uploads").multipart(form)).await
    }

    /// Polls a read until it isn't reading.
    async fn finished(&self, id: &str) -> ImportJobState {
        for _ in 0..1200 {
            let (status, text) =
                Self::send(self.request("GET", &format!("/api/import/uploads/{id}"))).await;
            assert_eq!(status, StatusCode::OK, "{text}");
            let job: ImportJob = serde_json::from_str(&text).unwrap();
            if !matches!(job.state, ImportJobState::Reading { .. }) {
                return job.state;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the read never finished");
    }

    /// The batch folders under the root.
    fn batches(&self) -> std::collections::BTreeSet<String> {
        std::fs::read_dir(&self.root)
            .map(|listing| {
                listing
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The one batch folder made since `before`, waiting for it.
    async fn new_batch(&self, before: &std::collections::BTreeSet<String>) -> String {
        for _ in 0..1000 {
            let made: Vec<String> = self.batches().difference(before).cloned().collect();
            if let [id] = made.as_slice() {
                return id.clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("no batch folder was made");
    }

    /// The batch folders once any made since `before` are gone, waiting a
    /// little for a drop that runs off the async workers.
    async fn settled_batches(
        &self,
        before: &std::collections::BTreeSet<String>,
    ) -> std::collections::BTreeSet<String> {
        for _ in 0..200 {
            if self.batches().is_subset(before) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.batches()
    }

    /// Whether every batch folder is gone, waiting a little for a drop
    /// that runs off the async workers.
    async fn no_batches(&self) -> bool {
        for _ in 0..200 {
            let empty = match std::fs::read_dir(&self.root) {
                Ok(mut listing) => listing.next().is_none(),
                Err(_) => true,
            };
            if empty {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    }
}

/// A server's state over an in-memory keychain, serving no web build.
fn server_state(directory: &Path) -> crate::config::ServerState {
    let state = Arc::new(
        AppState::builder()
            .secret_store(bruhs_core::secrets::SecretStore::in_memory())
            .connections(bruhs_core::connections::Connections::in_memory())
            .build(),
    );
    let config = crate::config::Config::from_settings(crate::config::Settings {
        port: Some("0"),
        web_dist: directory.join("missing").to_str(),
        ..Default::default()
    })
    .unwrap();
    crate::config::ServerState {
        app: state,
        config: Arc::new(config),
        updates: bruhs_core::updates::UpdateChecker::new(bruhs_core::updates::BuildInfo::new(
            "", "", "unused",
        )),
    }
}

fn error(text: &str) -> String {
    json!({ "error": text }).to_string()
}

/// A random value no response or log line may hold.
fn canary() -> String {
    format!("canary-{}", uuid::Uuid::new_v4().simple())
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn file(bytes: Vec<u8>, name: &str) -> Part {
    Part::bytes(bytes).file_name(name.to_owned())
}

#[tokio::test]
async fn pages_can_not_import() {
    let fixture = Fixture::new().await;
    for &(method, path) in ROUTES {
        let (status, text) =
            Fixture::send(fixture.request(method, path).header("origin", OTHER_PAGE)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {text}");
        assert_eq!(text, error(OWNER_ONLY_IMPORT), "{method} {path}");

        let (status, text) = Fixture::send(fixture.request(method, path)).await;
        assert_ne!(status, StatusCode::FORBIDDEN, "{method} {path}: {text}");
        let (status, text) = Fixture::send(
            fixture
                .request(method, path)
                .header("origin", &fixture.base),
        )
        .await;
        if path == "/api/import/files" {
            // Only the desktop sends picked paths, with no Origin (M5).
            assert_eq!(
                (status, text),
                (StatusCode::FORBIDDEN, error(OWNER_ONLY_IMPORT))
            );
        } else {
            assert_ne!(status, StatusCode::FORBIDDEN, "{method} {path}: {text}");
        }
    }
}

#[tokio::test]
async fn an_upload_round_trips() {
    let fixture = Fixture::new().await;
    let export = zip_of(
        &[("conversations.json", sample("chatgpt-conversations.json"))],
        true,
    );
    let form = Form::new().part("file", file(export, "chatgpt.zip")).part(
        "file",
        file(sample("grok-prod-grok-backend.json").to_vec(), "users.json"),
    );
    let (status, text) = fixture.upload(form).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{text}");
    let job: ImportJob = serde_json::from_str(&text).unwrap();

    let batch: ImportBatch = match fixture.finished(&job.id).await {
        ImportJobState::Ready { batch } => *batch,
        other => panic!("expected a ready batch, got {other:?}"),
    };
    assert_eq!(batch.id, job.id);
    assert!(batch.best_effort);
    assert!(batch
        .candidates
        .iter()
        .any(|candidate| candidate.source == ImportSource::Grok));
    let chat = batch
        .candidates
        .iter()
        .find(|candidate| candidate.source == ImportSource::ChatGpt)
        .expect("a ChatGPT chat");

    let thread_path = format!(
        "/api/import/thread?source=chatgpt&id={}&upload={}",
        chat.id, job.id
    );
    let (status, text) = Fixture::send(fixture.request("GET", &thread_path)).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let thread: ImportedThread = serde_json::from_str(&text).unwrap();
    assert_eq!(thread.candidate.id, chat.id);
    assert!(!thread.messages.is_empty());

    // A batch is read with an export's source only.
    let local = format!(
        "/api/import/thread?source=codex&id={}&upload={}",
        chat.id, job.id
    );
    let (status, text) = Fixture::send(fixture.request("GET", &local)).await;
    assert_eq!(
        (status, text),
        (StatusCode::BAD_REQUEST, error(IMPORT_INVALID))
    );
    let unknown = format!(
        "/api/import/thread?source=chatgpt&id=nope&upload={}",
        job.id
    );
    // The batch is still here, so the line names the chat (final review B).
    let (status, text) = Fixture::send(fixture.request("GET", &unknown)).await;
    assert_eq!(
        (status, text),
        (StatusCode::NOT_FOUND, error(IMPORT_CHAT_GONE))
    );

    let job_path = format!("/api/import/uploads/{}", job.id);
    let (status, cache, text) = Fixture::send_full(fixture.request("DELETE", &job_path)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{text}");
    assert_eq!(cache.as_deref(), Some("no-store"));
    let (status, text) = Fixture::send(fixture.request("GET", &job_path)).await;
    assert_eq!((status, text), (StatusCode::NOT_FOUND, error(IMPORT_GONE)));
    let (status, text) = Fixture::send(fixture.request("GET", &thread_path)).await;
    assert_eq!((status, text), (StatusCode::NOT_FOUND, error(IMPORT_GONE)));
    let (status, text) = Fixture::send(fixture.request("DELETE", &job_path)).await;
    assert_eq!((status, text), (StatusCode::NOT_FOUND, error(IMPORT_GONE)));
    assert!(fixture.no_batches().await);
}

#[tokio::test]
async fn picked_paths_are_read_in_place() {
    let fixture = Fixture::new().await;
    let picked = fixture.home.join("Downloads/export.zip");
    let export = zip_of(
        &[("conversations.json", sample("chatgpt-conversations.json"))],
        true,
    );
    write(&picked, &export);
    let (status, text) = Fixture::send(
        fixture
            .request("POST", "/api/import/files")
            .json(&json!({ "paths": [picked] })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{text}");
    let job: ImportJob = serde_json::from_str(&text).unwrap();
    let ImportJobState::Ready { batch } = fixture.finished(&job.id).await else {
        panic!("the read failed");
    };
    assert!(!batch.candidates.is_empty());
    assert_eq!(std::fs::read(&picked).unwrap(), export);

    // `~/` reads against the home, as a scan does (M6).
    let (status, text) = Fixture::send(
        fixture
            .request("POST", "/api/import/files")
            .json(&json!({ "paths": ["~/Downloads/export.zip"] })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{text}");
    let job: ImportJob = serde_json::from_str(&text).unwrap();
    let ImportJobState::Ready { batch: from_home } = fixture.finished(&job.id).await else {
        panic!("the read failed");
    };
    assert_eq!(from_home.candidates.len(), batch.candidates.len());

    // Only absolute paths: nothing resolves against the server's folder.
    let (status, text) = Fixture::send(
        fixture
            .request("POST", "/api/import/files")
            .json(&json!({ "paths": ["Downloads/export.zip"] })),
    )
    .await;
    assert_eq!(
        (status, text),
        (StatusCode::BAD_REQUEST, error(FOLDER_MISSING))
    );

    // A page never sends them, not even the app's own (M5).
    let before = fixture.batches();
    for origin in [fixture.base.as_str(), OTHER_PAGE] {
        let (status, text) = Fixture::send(
            fixture
                .request("POST", "/api/import/files")
                .header("origin", origin)
                .json(&json!({ "paths": [picked] })),
        )
        .await;
        assert_eq!(
            (status, text),
            (StatusCode::FORBIDDEN, error(OWNER_ONLY_IMPORT)),
            "{origin}"
        );
    }
    assert_eq!(fixture.batches(), before);
}

/// A multipart body with one `file` field per item, as raw bytes.
const BOUNDARY: &str = "bruhs-import-test-boundary";

fn field_head(name: &str) -> Vec<u8> {
    format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"; \
         filename=\"x.json\"\r\nContent-Type: application/octet-stream\r\n\r\n"
    )
    .into_bytes()
}

fn body_tail() -> Vec<u8> {
    format!("\r\n--{BOUNDARY}--\r\n").into_bytes()
}

/// `chunks` streamed with no `Content-Length` (chunked transfer).
fn streamed(fixture: &Fixture, chunks: Vec<Vec<u8>>) -> reqwest::RequestBuilder {
    let stream = futures_util::stream::iter(chunks.into_iter().map(Ok::<_, std::io::Error>));
    fixture
        .request("POST", "/api/import/uploads")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(reqwest::Body::wrap_stream(stream))
}

/// Feeds a streamed request body.
type BodySender = tokio::sync::mpsc::Sender<Result<Vec<u8>, std::io::Error>>;
/// A request's status, `Cache-Control` and body, once it ends.
type Answer = tokio::task::JoinHandle<(StatusCode, Option<String>, String)>;

/// An upload whose client sent one field's start and then waits, holding
/// its body open until the returned sender drops.
fn stalled(fixture: &Fixture) -> (BodySender, Answer) {
    let (sender, receiver) = tokio::sync::mpsc::channel(4);
    let mut first = field_head("file");
    first.extend_from_slice(b"[{\"mapping\": {");
    sender.try_send(Ok(first)).unwrap();
    let stream = futures_util::stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|item| (item, receiver))
    });
    let request = fixture
        .request("POST", "/api/import/uploads")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(reqwest::Body::wrap_stream(stream));
    (sender, tokio::spawn(Fixture::send_full(request)))
}

#[tokio::test]
async fn chunked_uploads_are_read_and_capped() {
    let fixture = Fixture::new().await;
    let json = sample("chatgpt-conversations.json");
    let mut chunks = vec![field_head("file")];
    chunks.extend(json.chunks(512).map(<[u8]>::to_vec));
    chunks.push(body_tail());
    let (status, text) = Fixture::send(streamed(&fixture, chunks)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{text}");
    let job: ImportJob = serde_json::from_str(&text).unwrap();
    assert!(matches!(
        fixture.finished(&job.id).await,
        ImportJobState::Ready { .. }
    ));

    // No `Content-Length` to trust: the bytes received are counted.
    let limit = usize::try_from(Limits::TEST.upload_bytes).unwrap();
    let mut chunks = vec![field_head("file")];
    chunks.extend(std::iter::repeat_n(vec![b' '; 1 << 20], limit >> 20));
    chunks.push(vec![b' '; 1]);
    chunks.push(body_tail());
    let before = fixture.batches();
    let (status, text) = Fixture::send(streamed(&fixture, chunks)).await;
    assert_eq!(
        (status, text),
        (StatusCode::PAYLOAD_TOO_LARGE, error(EXPORT_TOO_LARGE))
    );
    assert_eq!(fixture.settled_batches(&before).await, before);
}

/// Final review B: the parser holds at most `MAX_HELD` bytes outside a
/// field's data. A header block that never ends is refused once that much
/// arrives, while its client still holds the body open, not buffered for
/// as long as the client sends.
#[tokio::test]
async fn a_field_header_that_never_ends_is_refused() {
    let fixture = Fixture::new().await;
    let (sender, receiver): (BodySender, _) = tokio::sync::mpsc::channel(4);
    let stream = futures_util::stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|item| (item, receiver))
    });
    let request = fixture
        .request("POST", "/api/import/uploads")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(reqwest::Body::wrap_stream(stream));
    let pending = tokio::spawn(Fixture::send_full(request));
    let head = format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"\r\nX-Pad: ");
    sender.send(Ok(head.into_bytes())).await.unwrap();
    let pieces = usize::try_from(super::MAX_HELD >> 20).unwrap() + 2;
    let feeder = tokio::spawn(async move {
        for _ in 0..pieces {
            if sender.send(Ok(vec![b'a'; 1 << 20])).await.is_err() {
                return;
            }
        }
        // Held open: the client never ends the body.
        std::future::pending::<()>().await;
    });
    let (status, _, text) = tokio::time::timeout(Duration::from_secs(30), pending)
        .await
        .expect("the endless header block is refused")
        .unwrap();
    feeder.abort();
    assert_eq!(
        (status, text),
        (StatusCode::BAD_REQUEST, error(IMPORT_INVALID))
    );
    assert!(fixture.no_batches().await);
}

#[tokio::test]
async fn deleting_a_stalled_upload_frees_the_read_slot() {
    let fixture = Fixture::new().await;
    let (sender, pending) = stalled(&fixture);
    let id = fixture.new_batch(&std::collections::BTreeSet::new()).await;

    let json = sample("chatgpt-conversations.json");
    let busy = || Form::new().part("file", file(json.to_vec(), "a.json"));
    let (status, text) = fixture.upload(busy()).await;
    assert_eq!((status, text), (StatusCode::CONFLICT, error(IMPORT_BUSY)));

    let (status, cache, text) =
        Fixture::send_full(fixture.request("DELETE", &format!("/api/import/uploads/{id}"))).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{text}");
    assert_eq!(cache.as_deref(), Some("no-store"));
    // The stalled request ends while its client still holds the body open.
    let (status, _, text) = tokio::time::timeout(Duration::from_secs(30), pending)
        .await
        .expect("the stalled upload ends")
        .unwrap();
    assert_eq!((status, text), (StatusCode::NOT_FOUND, error(IMPORT_GONE)));
    assert!(fixture.no_batches().await);
    let (status, text) = fixture.upload(busy()).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{text}");
    drop(sender);
}

#[tokio::test]
async fn a_client_that_goes_away_leaves_nothing() {
    let fixture = Fixture::new().await;
    let (sender, pending) = stalled(&fixture);
    fixture.new_batch(&std::collections::BTreeSet::new()).await;
    pending.abort();
    drop(sender);
    assert!(fixture.no_batches().await);
    let json = sample("chatgpt-conversations.json");
    let mut accepted = false;
    for _ in 0..200 {
        let form = Form::new().part("file", file(json.to_vec(), "a.json"));
        let (status, text) = fixture.upload(form).await;
        if status == StatusCode::ACCEPTED {
            accepted = true;
            break;
        }
        assert_eq!((status, text), (StatusCode::CONFLICT, error(IMPORT_BUSY)));
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(accepted, "the read slot was never freed");
}

/// The guard a request's upload lives in hands it to a blocking thread
/// when the request is dropped mid-wait (M1).
#[tokio::test]
async fn a_dropped_request_frees_its_upload_off_the_async_workers() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("imports");
    let host = ImportHost::new(root.clone()).with_limits(Limits::TEST);
    #[cfg(unix)]
    let host = host.with_free_space(Arc::new(|_: &Path| Ok(u64::MAX)));
    let upload = host.begin_upload().unwrap();
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    drop(super::Receiving(Some(upload)));
    let mut freed = false;
    for _ in 0..200 {
        if let Ok(next) = host.begin_upload() {
            freed = true;
            drop(next);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(freed, "the read slot was never freed");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
}

#[tokio::test]
async fn uploads_refuse_other_fields_and_caps() {
    let fixture = Fixture::new().await;
    let json = sample("chatgpt-conversations.json");

    let form = Form::new()
        .part("file", file(json.to_vec(), "a.json"))
        .part("x", file(json.to_vec(), "b.json"));
    let (status, text) = fixture.upload(form).await;
    assert_eq!(
        (status, text),
        (StatusCode::BAD_REQUEST, error(IMPORT_INVALID))
    );
    assert!(fixture.no_batches().await);

    let limit = usize::try_from(Limits::TEST.upload_bytes).unwrap();
    let form = Form::new().part("file", file(vec![b' '; limit + 1], "big.json"));
    let (status, text) = fixture.upload(form).await;
    assert_eq!(
        (status, text),
        (StatusCode::PAYLOAD_TOO_LARGE, error(EXPORT_TOO_LARGE))
    );
    assert!(fixture.no_batches().await);

    let form = (0..=Limits::TEST.upload_files).fold(Form::new(), |form, n| {
        form.part("file", file(json.to_vec(), &format!("{n}.json")))
    });
    let (status, text) = fixture.upload(form).await;
    assert_eq!(
        (status, text),
        (StatusCode::PAYLOAD_TOO_LARGE, error(TOO_MANY_FILES))
    );
    assert!(fixture.no_batches().await);

    // Not multipart at all.
    let (status, text) = Fixture::send(
        fixture
            .request("POST", "/api/import/uploads")
            .header("content-type", "application/json")
            .body("{}"),
    )
    .await;
    assert_eq!(
        (status, text),
        (StatusCode::BAD_REQUEST, error(IMPORT_INVALID))
    );

    // Each refusal freed the read slot.
    let form = Form::new().part("file", file(json.to_vec(), "a.json"));
    let (status, text) = fixture.upload(form).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{text}");
}

#[tokio::test]
async fn bodies_never_echo() {
    #[cfg(unix)]
    let (logs, _capturing) = crate::routes::backup::tests::capture_process_logs();
    let fixture = Fixture::new().await;
    let secret = canary();
    let missing = fixture.home.join(&secret).join("export.zip");

    let (status, text) = Fixture::send(
        fixture
            .request("POST", "/api/import/files")
            .json(&json!({ "paths": [missing], "surprise": secret })),
    )
    .await;
    assert_eq!(
        (status, text),
        (StatusCode::BAD_REQUEST, error(IMPORT_INVALID))
    );
    let (status, text) = Fixture::send(
        fixture
            .request("POST", "/api/import/files")
            .header("content-type", "application/json")
            .body(format!("{{\"paths\": [\"{secret}")),
    )
    .await;
    assert_eq!(
        (status, text),
        (StatusCode::BAD_REQUEST, error(IMPORT_INVALID))
    );
    let too_long = format!("/{}", "a".repeat(bruhs_models::imports::MAX_PATHS_BODY));
    let (status, text) = Fixture::send(
        fixture
            .request("POST", "/api/import/files")
            .json(&json!({ "paths": [too_long, secret] })),
    )
    .await;
    assert_eq!(
        (status, text),
        (StatusCode::BAD_REQUEST, error(IMPORT_INVALID))
    );

    let (status, text) = Fixture::send(
        fixture
            .request("POST", "/api/import/files")
            .json(&json!({ "paths": [missing] })),
    )
    .await;
    assert_eq!(
        (status, text),
        (StatusCode::BAD_REQUEST, error(FOLDER_MISSING))
    );

    let missing_text = missing.to_string_lossy().into_owned();
    let (status, text) = Fixture::send(
        fixture
            .request("GET", "/api/import/agents")
            .query(&[("source", "openclaw"), ("path", missing_text.as_str())]),
    )
    .await;
    assert_eq!(
        (status, text),
        (StatusCode::BAD_REQUEST, error(FOLDER_MISSING))
    );

    for query in [
        vec![("source", secret.as_str())],
        vec![("source", "chatgpt"), ("q", secret.as_str())],
    ] {
        let (status, text) =
            Fixture::send(fixture.request("GET", "/api/import/sessions").query(&query)).await;
        assert_eq!(
            (status, text),
            (StatusCode::BAD_REQUEST, error(IMPORT_INVALID))
        );
    }
    let (status, text) = Fixture::send(
        fixture
            .request("GET", "/api/import/thread")
            .query(&[("source", "chatgpt"), ("id", &secret)]),
    )
    .await;
    assert_eq!(
        (status, text),
        (StatusCode::BAD_REQUEST, error(IMPORT_INVALID))
    );

    #[cfg(unix)]
    assert!(!logs.text().contains(&secret), "a log holds the canary");
}

#[tokio::test]
async fn agents_scan_reads_only_persona_files() {
    let fixture = Fixture::new().await;
    let workspace = fixture.home.join(".openclaw/workspace");
    let hermes = fixture.home.join(".hermes");
    let canaries: Vec<String> = (0..8).map(|_| canary()).collect();
    let secrets = [
        ".env",
        "auth.json",
        "auth-profiles.json",
        "credentials/token",
        "config.yaml",
        "openclaw.json",
        "state.db",
        "agent.sqlite",
    ];
    for (name, canary) in secrets.iter().zip(&canaries) {
        write(&workspace.join(name), canary.as_bytes());
        write(&hermes.join(name), canary.as_bytes());
    }
    write(&workspace.join("SOUL.md"), b"Soul text: be curious.");
    write(&hermes.join("SOUL.md"), b"Hermes soul.");
    #[cfg(unix)]
    {
        let outside = fixture.home.join("outside.md");
        write(&outside, canaries[0].as_bytes());
        std::os::unix::fs::symlink(&outside, workspace.join("AGENTS.md")).unwrap();
    }

    for source in ["openclaw", "hermes"] {
        let (status, text) = Fixture::send(
            fixture
                .request("GET", "/api/import/agents")
                .query(&[("source", source)]),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{source}: {text}");
        let scan: bruhs_models::imports::AgentScan = serde_json::from_str(&text).unwrap();
        assert_eq!(scan.personas.len(), 1, "{source}");
        for canary in &canaries {
            assert!(!text.contains(canary.as_str()), "{source}");
        }
    }
    // A picked folder, with `~` read against the scratch home.
    let (status, text) = Fixture::send(
        fixture
            .request("GET", "/api/import/agents")
            .query(&[("source", "openclaw"), ("path", "~/.openclaw/workspace")]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert!(text.contains("Soul text"));

    for source in ["nope", "chatgpt", ""] {
        let (status, text) = Fixture::send(
            fixture
                .request("GET", "/api/import/agents")
                .query(&[("source", source)]),
        )
        .await;
        assert_eq!(
            (status, text),
            (StatusCode::BAD_REQUEST, error(IMPORT_INVALID)),
            "{source}"
        );
    }
}

#[tokio::test]
async fn ids_are_checked() {
    let fixture = Fixture::new().await;
    let upper = NO_BATCH.to_uppercase();
    let short = &NO_BATCH[..31];
    for id in ["..%2Fx", upper.as_str(), short, NO_BATCH] {
        for method in ["GET", "DELETE"] {
            let (status, text) =
                Fixture::send(fixture.request(method, &format!("/api/import/uploads/{id}"))).await;
            assert_eq!(
                (status, text),
                (StatusCode::NOT_FOUND, error(IMPORT_GONE)),
                "{method} {id}"
            );
        }
        let (status, text) = Fixture::send(fixture.request("GET", "/api/import/thread").query(&[
            ("source", "chatgpt"),
            ("id", "conv-lisbon-a"),
            ("upload", id),
        ]))
        .await;
        assert_eq!(
            (status, text),
            (StatusCode::NOT_FOUND, error(IMPORT_GONE)),
            "thread {id}"
        );
    }
}
