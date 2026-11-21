//! The routes that make a backup, over HTTP through the real router and
//! its guard: temporary roots, an in-memory keychain, fast key derivation
//! and a capture daemon that isn't recording. Every response is read as
//! text and checked for the passwords sent.

use std::ffi::OsString;
use std::io::{Cursor, Read as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::http::StatusCode;
use axum::Extension;
use bruhs_backup::{Decryptor, Encryptor, Header, Kdf, KdfParams, Key, Kind, Secret};
use bruhs_core::backup::{BackupHost, Roots, UploadKind};
use bruhs_core::secrets::SecretStore;
use bruhs_core::AppState;
use bruhs_models::backup::{
    normalize_password, valid_id, BackupJob, BackupOverview, JobState, UploadId,
    DESTINATION_INSIDE, GONE, INVALID_REQUEST, NOT_A_BACKUP, OWNER_ONLY_BACKUP, PASSWORD_SHORT,
    SOURCE_MISSING, WEB_PART_TYPE, WEB_TOO_LARGE, WEB_UPLOAD_MAX, WRONG_PASSWORD,
};
use reqwest::Method;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tracing_subscriber::layer::{Layer as _, SubscriberExt as _};

use crate::routes::apps::tests::Logs;

/// Whether the process-wide capture records now.
static CAPTURING: AtomicBool = AtomicBool::new(false);

/// What the process logs, from every thread and at every level, while the
/// returned guard lives. A thread-local capture misses the server's events
/// when another test thread registers their callsites first, so this one
/// is global; outside the guard it records nothing.
fn capture_process_logs() -> (&'static Logs, Capturing) {
    static LOGS: OnceLock<Logs> = OnceLock::new();
    let logs = LOGS.get_or_init(|| {
        let logs = Logs::default();
        let writer = logs.clone();
        let layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .with_filter(tracing_subscriber::filter::filter_fn(|_| {
                CAPTURING.load(Ordering::SeqCst)
            }));
        tracing::subscriber::set_global_default(tracing_subscriber::registry().with(layer))
            .expect("no other test sets a global subscriber");
        logs
    });
    assert!(
        !CAPTURING.swap(true, Ordering::SeqCst),
        "one capture at a time"
    );
    Logs::refresh();
    (logs, Capturing)
}

/// Stops the process-wide capture when dropped.
struct Capturing;

impl Drop for Capturing {
    fn drop(&mut self) {
        CAPTURING.store(false, Ordering::SeqCst);
    }
}

/// A loopback page the shared guard lets through but that isn't the app.
const OTHER_PAGE: &str = "http://localhost:5173";
pub(super) const PASSWORD: &str = "correct horse battery";
/// The IndexedDB version a test's web part names.
pub(super) const WEB_DB_VERSION: u32 = 13;

/// Every route of D20, with an id or stamp of the right shape.
const ROUTES: [(&str, &str); 15] = [
    ("GET", "/api/backup"),
    ("POST", "/api/backup/web-parts"),
    ("POST", "/api/backup"),
    ("GET", "/api/backup/jobs/0123456789abcdef0123456789abcdef"),
    (
        "DELETE",
        "/api/backup/jobs/0123456789abcdef0123456789abcdef",
    ),
    ("GET", "/api/backup/files/0123456789abcdef0123456789abcdef"),
    ("POST", "/api/backup/restore/uploads"),
    ("POST", "/api/backup/restore"),
    (
        "DELETE",
        "/api/backup/restore/0123456789abcdef0123456789abcdef",
    ),
    ("POST", "/api/backup/restore/apply"),
    ("GET", "/api/backup/restore/status"),
    ("GET", "/api/backup/restore/web-part"),
    ("DELETE", "/api/backup/restore/web-part"),
    ("POST", "/api/backup/restore/web-part/previous"),
    ("DELETE", "/api/backup/pre-restore/2026-11-20-1200-ab12"),
];

type Env = Arc<dyn Fn(&str) -> Option<OsString> + Send + Sync>;

struct Fixture {
    base: String,
    client: reqwest::Client,
    server: tokio::task::JoinHandle<()>,
    host: BackupHost,
    roots: Roots,
    _directory: tempfile::TempDir,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Fixture {
    async fn new() -> Self {
        Self::with_env(Arc::new(|_| None)).await
    }

    /// With the `BRUHS_*` overrides `env` names.
    async fn with_env(env: Env) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join("home");
        let roots = Roots {
            core: home.join("core"),
            desktop: home.join("desktop"),
            config: home.join("core"),
            home,
        };
        for root in [&roots.core, &roots.desktop] {
            std::fs::create_dir_all(root).unwrap();
        }
        write(&roots.core.join("notes/note.txt"), b"today");
        let host = BackupHost::new(roots.clone(), SecretStore::in_memory())
            .for_tests(false)
            .with_env(move |name| env(name));
        let state = Arc::new(
            AppState::builder()
                .secret_store(SecretStore::in_memory())
                .connections(bruhs_core::connections::Connections::in_memory())
                .build(),
        );
        let config = crate::config::Config::from_settings(crate::config::Settings {
            port: Some("0"),
            web_dist: directory.path().join("missing").to_str(),
            ..Default::default()
        })
        .unwrap();
        let app = crate::router(crate::config::ServerState {
            app: state,
            config: Arc::new(config),
            updates: bruhs_core::updates::UpdateChecker::new(bruhs_core::updates::BuildInfo::new(
                "", "", "unused",
            )),
        })
        .layer(Extension(host.clone()));
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
            host,
            roots,
            _directory: directory,
        }
    }

    fn request(&self, method: &str, path: &str) -> reqwest::RequestBuilder {
        self.client.request(
            Method::from_bytes(method.as_bytes()).unwrap(),
            format!("{}{path}", self.base),
        )
    }

    /// The status and raw body of `request`.
    async fn send(request: reqwest::RequestBuilder) -> (StatusCode, String) {
        let response = request.send().await.unwrap();
        (response.status(), response.text().await.unwrap())
    }

    async fn create(&self, body: &Value) -> (StatusCode, String) {
        Self::send(self.request("POST", "/api/backup").json(body)).await
    }

    /// Uploads `part` as the browser's part: its id.
    async fn upload(&self, part: Vec<u8>) -> String {
        let (status, text) = Self::send(
            self.request("POST", "/api/backup/web-parts")
                .header("content-type", "application/octet-stream")
                .body(part),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{text}");
        let UploadId { id } = serde_json::from_str(&text).unwrap();
        id
    }

    /// Polls a job until it stops running.
    async fn finished(&self, id: &str) -> JobState {
        for _ in 0..600 {
            let (status, text) =
                Self::send(self.request("GET", &format!("/api/backup/jobs/{id}"))).await;
            assert_eq!(status, StatusCode::OK, "{text}");
            let job: BackupJob = serde_json::from_str(&text).unwrap();
            if !matches!(job.state, JobState::Running { .. }) {
                return job.state;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the job never finished");
    }
}

pub(super) fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

pub(super) fn error(text: &str) -> String {
    json!({ "error": text }).to_string()
}

/// A random value no response or log line may hold.
pub(super) fn canary() -> String {
    format!("canary-{}", uuid::Uuid::new_v4().simple())
}

/// `plain` in a container of `kind`, locked with [`PASSWORD`].
pub(super) fn container(kind: Kind, plain: &[u8]) -> Vec<u8> {
    let header = Header::new(kind, Kdf::Argon2id(KdfParams::FLOOR), [7; 23]);
    let key = Key::derive(&normalize_password(PASSWORD), &header).unwrap();
    let mut writer = Encryptor::new(Vec::new(), &header, &key).unwrap();
    std::io::Write::write_all(&mut writer, plain).unwrap();
    writer.finish().unwrap()
}

/// Every file of a full backup by its entry path, decrypted with
/// [`PASSWORD`].
fn entries(file: &[u8]) -> Vec<(String, Vec<u8>)> {
    let password = normalize_password(PASSWORD);
    let mut decryptor =
        Decryptor::open(Cursor::new(file), Secret::Password(&password), Kind::Full).unwrap();
    let mut files = Vec::new();
    {
        let mut archive = tar::Archive::new(&mut decryptor);
        for entry in archive.entries().unwrap() {
            let mut entry = entry.unwrap();
            let path = String::from_utf8(entry.path_bytes().into_owned()).unwrap();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            files.push((path, bytes));
        }
    }
    let mut rest = Vec::new();
    decryptor.read_to_end(&mut rest).unwrap();
    decryptor.finish().unwrap();
    files
}

#[tokio::test]
async fn pages_can_not_make_or_restore_backups() {
    let fixture = Fixture::new().await;
    for (method, path) in ROUTES {
        let (status, text) =
            Fixture::send(fixture.request(method, path).header("origin", OTHER_PAGE)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {text}");
        assert_eq!(text, error(OWNER_ONLY_BACKUP), "{method} {path}");

        let (status, text) = Fixture::send(fixture.request(method, path)).await;
        assert_ne!(status, StatusCode::FORBIDDEN, "{method} {path}: {text}");
        let (status, text) = Fixture::send(
            fixture
                .request(method, path)
                .header("origin", &fixture.base),
        )
        .await;
        assert_ne!(status, StatusCode::FORBIDDEN, "{method} {path}: {text}");
    }
}

#[tokio::test]
async fn a_web_backup_downloads_once() {
    let fixture = Fixture::new().await;
    let part = container(Kind::WebPart, br#"{"version":1,"stores":{}}"#);
    let id = fixture.upload(part.clone()).await;
    assert!(valid_id(&id));

    let (status, text) = fixture
        .create(&json!({
            "password": PASSWORD,
            "app": "web",
            "web_part": id,
            "web_db_version": WEB_DB_VERSION,
        }))
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let job: BackupJob = serde_json::from_str(&text).unwrap();
    let JobState::Created { file } = fixture.finished(&job.id).await else {
        panic!("the backup wasn't made");
    };
    let download = file.download.clone().unwrap();
    assert!(file.path.is_none(), "the web gets no path");

    let response = fixture.request("GET", &download).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers().clone();
    let bytes = response.bytes().await.unwrap();
    assert_eq!(headers["content-type"], "application/octet-stream");
    assert_eq!(
        headers["content-disposition"],
        format!("attachment; filename=\"{}\"", file.name).as_str()
    );
    assert_eq!(headers["content-length"], bytes.len().to_string().as_str());
    assert_eq!(headers["cache-control"], "no-store");
    assert_eq!(u64::try_from(bytes.len()).unwrap(), file.bytes);
    let files = entries(&bytes);
    let web = files
        .iter()
        .find(|(path, _)| path == "web/part.bin")
        .unwrap();
    assert_eq!(web.1, part, "the browser's part rides along byte for byte");
    let note = files
        .iter()
        .find(|(path, _)| path == "core/notes/note.txt")
        .unwrap();
    assert_eq!(note.1, b"today");

    // The file goes once it was sent in full.
    let mut second = (StatusCode::OK, String::new());
    for _ in 0..50 {
        second = Fixture::send(fixture.request("GET", &download)).await;
        if second.0 == StatusCode::NOT_FOUND {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(second, (StatusCode::NOT_FOUND, error(GONE)));

    // A finished job can be dropped; an unknown one is gone.
    let (status, _) =
        Fixture::send(fixture.request("DELETE", &format!("/api/backup/jobs/{}", job.id))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let unknown = format!("/api/backup/jobs/{}", fixture.host.random_id());
    assert_eq!(
        Fixture::send(fixture.request("DELETE", &unknown)).await,
        (StatusCode::NOT_FOUND, error(GONE))
    );
}

#[tokio::test]
async fn a_download_cut_short_stays_for_another_try() {
    let fixture = Fixture::new().await;
    // Larger than the socket buffers can take at once.
    write(&fixture.roots.core.join("big.bin"), &vec![5; 24 << 20]);
    let (status, text) = fixture
        .create(&json!({ "password": PASSWORD, "app": "web" }))
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let job: BackupJob = serde_json::from_str(&text).unwrap();
    let JobState::Created { file } = fixture.finished(&job.id).await else {
        panic!("the backup wasn't made");
    };
    let download = file.download.unwrap();

    // Read the headers and one chunk, then hang up.
    let address = fixture.base.trim_start_matches("http://");
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    stream
        .write_all(format!("GET {download} HTTP/1.1\r\nHost: {address}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut buffer = vec![0; 8192];
    let read = stream.read(&mut buffer).await.unwrap();
    assert!(String::from_utf8_lossy(&buffer[..read]).starts_with("HTTP/1.1 200"));
    drop(stream);
    tokio::time::sleep(Duration::from_millis(200)).await;

    let response = fixture.request("GET", &download).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.bytes().await.unwrap();
    assert_eq!(u64::try_from(bytes.len()).unwrap(), file.bytes);
}

#[tokio::test]
async fn bodies_never_echo() {
    let (logs, _capturing) = capture_process_logs();
    let fixture = Fixture::new().await;
    let secret = canary();

    let (status, text) = fixture
        .create(&json!({ "password": secret, "app": "web", "surprise": 1 }))
        .await;
    assert_eq!(
        (status, text),
        (StatusCode::BAD_REQUEST, error(INVALID_REQUEST))
    );
    let (status, text) = Fixture::send(
        fixture
            .request("POST", "/api/backup")
            .header("content-type", "application/json")
            .body(format!("{{\"password\": \"{secret}\", \"app\": ")),
    )
    .await;
    assert_eq!(
        (status, text),
        (StatusCode::BAD_REQUEST, error(INVALID_REQUEST))
    );
    let (status, text) = fixture
        .create(&json!({ "password": secret, "app": "mobile" }))
        .await;
    assert_eq!(
        (status, text),
        (StatusCode::BAD_REQUEST, error(INVALID_REQUEST))
    );
    let (status, text) = fixture
        .create(&json!({ "password": "short", "app": "desktop" }))
        .await;
    assert_eq!(
        (status, text),
        (StatusCode::BAD_REQUEST, error(PASSWORD_SHORT))
    );

    // A web part needs the IndexedDB version it was dumped from.
    let id = fixture
        .upload(container(Kind::WebPart, br#"{"version":1}"#))
        .await;
    let (status, text) = fixture
        .create(&json!({ "password": secret, "app": "web", "web_part": id }))
        .await;
    assert_eq!(
        (status, text),
        (StatusCode::BAD_REQUEST, error(INVALID_REQUEST))
    );
    // An upload that isn't there.
    let (status, text) = fixture
        .create(&json!({
            "password": secret,
            "app": "web",
            "web_part": fixture.host.random_id(),
            "web_db_version": WEB_DB_VERSION,
        }))
        .await;
    assert_eq!((status, text), (StatusCode::NOT_FOUND, error(GONE)));

    // The restore side: opening with the canary, as a body that isn't a
    // request, and as one whose password opens nothing.
    for body in [
        json!({ "source": { "type": "upload", "id": id }, "password": secret, "x": 1 }),
        json!({ "source": { "type": "elsewhere" }, "password": secret }),
    ] {
        let (status, text) =
            Fixture::send(fixture.request("POST", "/api/backup/restore").json(&body)).await;
        assert_eq!(
            (status, text),
            (StatusCode::BAD_REQUEST, error(INVALID_REQUEST))
        );
    }
    let (status, text) = Fixture::send(
        fixture
            .request("POST", "/api/backup/restore")
            .header("content-type", "application/json")
            .body(format!("{{\"password\": \"{secret}\", \"source\": ")),
    )
    .await;
    assert_eq!(
        (status, text),
        (StatusCode::BAD_REQUEST, error(INVALID_REQUEST))
    );
    let (status, text) =
        Fixture::send(fixture.request("POST", "/api/backup/restore").json(&json!({
            "source": { "type": "path", "path": format!("/nowhere/{secret}") },
            "password": secret,
        })))
        .await;
    assert_eq!(
        (status, text),
        (StatusCode::NOT_FOUND, error(SOURCE_MISSING))
    );
    let (status, text) = Fixture::send(fixture.request("POST", "/api/backup/restore/apply").json(
        &json!({
            "source": { "type": "staged", "id": fixture.host.random_id() },
            "replace": true,
            "password": secret,
        }),
    ))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(!text.contains(&secret), "{text}");
    let upload = Fixture::send(
        fixture
            .request("POST", "/api/backup/restore/uploads")
            .header("content-type", "application/octet-stream")
            .body(container(Kind::Full, b"not a tar")),
    )
    .await;
    assert_eq!(upload.0, StatusCode::OK, "{}", upload.1);
    let UploadId { id: upload } = serde_json::from_str(&upload.1).unwrap();
    let (status, text) = Fixture::send(
        fixture
            .request("POST", "/api/backup/restore")
            .json(&json!({ "source": { "type": "upload", "id": upload }, "password": secret })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert!(!text.contains(&secret), "{text}");
    let opening: BackupJob = serde_json::from_str(&text).unwrap();
    assert_eq!(
        fixture.finished(&opening.id).await,
        JobState::Failed {
            message: WRONG_PASSWORD.into()
        }
    );
    let (_, text) =
        Fixture::send(fixture.request("GET", &format!("/api/backup/jobs/{}", opening.id))).await;
    assert!(!text.contains(&secret), "{text}");

    // A request only this test makes, so the capture is known to hold
    // this test's requests.
    let marker = format!("/api/backup/jobs/{}", fixture.host.random_id());
    let (status, _) = Fixture::send(fixture.request("GET", &marker)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        logs.text().contains(&marker),
        "the capture missed the requests"
    );

    // A backup made with the canary as its password: nothing it answers
    // holds it.
    let mut answers = Vec::new();
    let (status, text) = fixture
        .create(&json!({ "password": secret, "app": "web" }))
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let job: BackupJob = serde_json::from_str(&text).unwrap();
    answers.push(text);
    loop {
        let (status, text) =
            Fixture::send(fixture.request("GET", &format!("/api/backup/jobs/{}", job.id))).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        let state: BackupJob = serde_json::from_str(&text).unwrap();
        answers.push(text);
        if !matches!(state.state, JobState::Running { .. }) {
            assert!(matches!(state.state, JobState::Created { .. }), "{state:?}");
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    answers.push(Fixture::send(fixture.request("GET", "/api/backup")).await.1);
    for answer in &answers {
        assert!(
            !answer.contains(&secret),
            "an answer holds the password: {answer}"
        );
    }

    assert!(
        !logs.text().contains(&secret),
        "the log holds the password: {}",
        logs.text()
    );
}

#[tokio::test]
async fn a_desktop_destination_is_checked_before_the_job() {
    let fixture = Fixture::new().await;
    let inside = fixture.roots.core.join("mine.bruhsbak");
    let (status, text) = fixture
        .create(&json!({
            "password": PASSWORD,
            "app": "desktop",
            "destination": inside.to_str().unwrap(),
        }))
        .await;
    assert_eq!(
        (status, text),
        (StatusCode::BAD_REQUEST, error(DESTINATION_INSIDE))
    );
    let (_, text) = Fixture::send(fixture.request("GET", "/api/backup")).await;
    let overview: BackupOverview = serde_json::from_str(&text).unwrap();
    assert!(overview.job.is_none(), "no job started");

    let outside: PathBuf = fixture.roots.home.join("mine");
    let (status, text) = fixture
        .create(&json!({
            "password": PASSWORD,
            "app": "desktop",
            "destination": outside.to_str().unwrap(),
        }))
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let job: BackupJob = serde_json::from_str(&text).unwrap();
    let JobState::Created { file } = fixture.finished(&job.id).await else {
        panic!("the backup wasn't made");
    };
    assert_eq!(file.download, None);
    let saved = fixture.roots.home.join("mine.bruhsbak");
    assert!(saved.is_file());
    assert_eq!(
        file.path.map(PathBuf::from),
        Some(std::fs::canonicalize(saved).unwrap())
    );
}

#[tokio::test]
async fn ids_are_checked() {
    let fixture = Fixture::new().await;
    let upper = "0123456789ABCDEF0123456789ABCDEF";
    let short = "0123456789abcdef0123456789abcde";
    for route in ["jobs", "files"] {
        for id in ["..%2Fx", upper, short] {
            let path = format!("/api/backup/{route}/{id}");
            let (status, text) = Fixture::send(fixture.request("GET", &path)).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {text}");
        }
        let path = format!("/api/backup/{route}/../x");
        let (status, _) = Fixture::send(fixture.request("GET", &path)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }
    for id in ["..%2Fx", upper, short] {
        let path = format!("/api/backup/jobs/{id}");
        let (status, text) = Fixture::send(fixture.request("DELETE", &path)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {text}");
    }
}

#[tokio::test]
async fn the_overview_names_sizes_and_overrides() {
    let outside = tempfile::tempdir().unwrap();
    let db: OsString = outside.path().join("reactive.sqlite3").into();
    let fixture = Fixture::with_env(Arc::new(move |name| {
        (name == "BRUHS_REACTIVE_DB").then(|| db.clone())
    }))
    .await;
    write(&fixture.roots.core.join("audio/m1.wav"), &[1; 1234]);
    write(&fixture.roots.core.join("audio/m1.jsonl"), b"{}\n");

    let (status, text) = Fixture::send(fixture.request("GET", "/api/backup")).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let overview: BackupOverview = serde_json::from_str(&text).unwrap();
    assert!(overview.supported);
    assert_eq!(overview.outside_roots, ["BRUHS_REACTIVE_DB"]);
    assert_eq!(overview.sizes.audio_bytes, 1234, "media files only");
    assert!(
        overview.sizes.data_bytes >= 3 + 5,
        "the transcript and the note"
    );
    assert!(overview.job.is_none());
    assert!(overview.last.is_none());
}

#[tokio::test]
async fn web_parts_are_checked_and_capped() {
    let fixture = Fixture::new().await;
    let incoming = fixture.roots.core.join("backups/incoming");
    for body in [
        b"not a backup, just some bytes".to_vec(),
        container(Kind::Full, b"a whole backup"),
        Vec::new(),
    ] {
        let (status, text) = Fixture::send(
            fixture
                .request("POST", "/api/backup/web-parts")
                .header("content-type", "application/octet-stream")
                .body(body),
        )
        .await;
        assert_eq!(
            (status, text),
            (StatusCode::BAD_REQUEST, error(NOT_A_BACKUP))
        );
    }
    assert_eq!(
        std::fs::read_dir(&incoming).unwrap().count(),
        0,
        "refused parts leave nothing behind"
    );

    // Only raw bytes are taken.
    for kind in [Some("text/plain"), Some("application/json"), None] {
        let mut request = fixture
            .request("POST", "/api/backup/web-parts")
            .body(container(Kind::WebPart, b"{}"));
        if let Some(kind) = kind {
            request = request.header("content-type", kind);
        }
        let (status, text) = Fixture::send(request).await;
        assert_eq!(
            (status, text),
            (StatusCode::UNSUPPORTED_MEDIA_TYPE, error(WEB_PART_TYPE)),
            "{kind:?}"
        );
    }
    assert_eq!(std::fs::read_dir(&incoming).unwrap().count(), 0);

    let id = fixture.upload(container(Kind::WebPart, b"{}")).await;
    assert!(fixture.host.uploaded(UploadKind::WebPart, &id).is_some());

    // The route's limit is `WEB_UPLOAD_MAX`: a body declared larger is
    // refused before it is read; one of exactly that size is read (and
    // here cut short).
    let answer = declared(&fixture, WEB_UPLOAD_MAX + 1).await;
    assert!(answer.starts_with("HTTP/1.1 400"), "{answer}");
    assert!(answer.contains(&error(WEB_TOO_LARGE)), "{answer}");
    let answer = declared(&fixture, WEB_UPLOAD_MAX).await;
    assert!(answer.starts_with("HTTP/1.1 400"), "{answer}");
    assert!(answer.contains(&error(INVALID_REQUEST)), "{answer}");
    assert_eq!(std::fs::read_dir(&incoming).unwrap().count(), 1);
}

/// Posts a web part that declares `bytes` but sends only a few, then
/// stops sending: the raw answer.
async fn declared(fixture: &Fixture, bytes: u64) -> String {
    let address = fixture.base.trim_start_matches("http://");
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    stream
        .write_all(
            format!(
                "POST /api/backup/web-parts HTTP/1.1\r\nHost: {address}\r\n\
                 Content-Type: application/octet-stream\r\nContent-Length: {bytes}\r\n\r\n\
                 BRUHSBAK"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    stream.shutdown().await.unwrap();
    let mut answer = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut answer))
        .await
        .unwrap()
        .unwrap();
    String::from_utf8_lossy(&answer).into_owned()
}

#[test]
fn the_manifest_names_the_release_health_reports() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let axum::Json(health) = runtime.block_on(crate::routes::health::get_health());
    assert_eq!(super::app_version(), health.version);
}

#[tokio::test]
async fn start_sweeps_before_it_answers() {
    let directory = tempfile::tempdir().unwrap();
    let tmp = directory.path().join("core/backups/tmp");
    write(&tmp.join("left.sqlite3"), b"snapshot");
    let roots = Roots {
        home: directory.path().to_path_buf(),
        core: directory.path().join("core"),
        desktop: directory.path().join("desktop"),
        config: directory.path().join("core"),
    };
    let host = BackupHost::new(roots, SecretStore::in_memory());
    let host = super::sweep_at_start(host, BackupHost::sweep).await;
    assert!(!tmp.join("left.sqlite3").exists());

    // A sweep that panics doesn't stop the server's start.
    write(&tmp.join("again.sqlite3"), b"snapshot");
    let host = super::sweep_at_start(host, |_| panic!("the sweep broke")).await;
    assert!(tmp.join("again.sqlite3").exists());
    assert_eq!(host.roots().core, directory.path().join("core"));
}

#[tokio::test]
async fn a_streamed_web_part_is_capped() {
    let fixture = Fixture::new().await;
    // One write past the cap, with no declared length.
    let mut part = container(Kind::WebPart, b"{}");
    part.resize(usize::try_from(WEB_UPLOAD_MAX + 1).unwrap(), 0);
    let body = reqwest::Body::wrap_stream(futures_util::stream::once(async move {
        Ok::<_, std::io::Error>(part)
    }));
    let (status, text) = Fixture::send(
        fixture
            .request("POST", "/api/backup/web-parts")
            .header("content-type", "application/octet-stream")
            .body(body),
    )
    .await;
    assert_eq!(
        (status, text),
        (StatusCode::BAD_REQUEST, error(WEB_TOO_LARGE))
    );
    let incoming = fixture.roots.core.join("backups/incoming");
    assert_eq!(std::fs::read_dir(&incoming).unwrap().count(), 0);
}
