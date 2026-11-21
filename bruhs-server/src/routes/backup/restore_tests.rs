//! The routes that restore a backup, over HTTP through the real router and
//! its guard: temporary roots, an in-memory keychain, fast key derivation,
//! fake bruh browsers, a live crew outside the roots and a fake
//! [`RestoreControl`] that records what it was asked. Nothing here touches
//! the owner's service, keychain or data.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use axum::http::StatusCode;
use axum::Extension;
use bruhs_backup::{Header, Kdf, KdfParams, Kind};
use bruhs_core::backup::{
    apply_pending, pending, pre_restores, restore_waits, ApplyOutcome, BackupHost, CreateRequest,
    JobHold, Output, Roots, UploadKind,
};
use bruhs_core::local_tools::{BruhComputer, FakePages, LocalToolsHost};
use bruhs_core::reactive::ReactiveRuntime;
use bruhs_core::secrets::SecretStore;
use bruhs_core::AppState;
use bruhs_models::backup::{
    ApplyAccepted, BackupApp, BackupJob, BackupOptions, BackupOverview, JobState, RestorePreview,
    RestoreSource, RestoreStatus, UploadId, BACKUP_BUSY, CONFIRM_REPLACE, GONE, INVALID_REQUEST,
    KEYCHAIN_SET_UNREADABLE, NOTHING_TO_PUT_BACK, QUIT_DESKTOP_FIRST, REACTIVE_DB_RELATIVE,
    RECORDING_FIRST, RESTORE_AT_NEXT_START, RESTORE_NOT_STARTED, RESTORE_PENDING,
    SERVICE_NOT_STOPPED, SERVICE_TOO_OLD, UPLOAD_TYPE, WRONG_PASSWORD,
};
use bruhs_models::crew::{BruhStatus, CREW_PROTOCOL};
use bruhs_models::reactive::{ReactiveAgent, ReactiveChange, ReactiveCommand};
use reqwest::Method;
use serde_json::{json, Value};
use zeroize::Zeroizing;

use super::tests::{canary, container, error, write, PASSWORD, WEB_DB_VERSION};
use crate::restore_control::{Control, RestoreControl};

const BRUH: &str = "bruh-1";
const NOTE: &str = "notes/note.txt";

/// How the fake control answers.
#[derive(Clone, Copy)]
struct Service {
    /// The installed service's crew protocol.
    protocol: Option<u32>,
    loaded: bool,
    /// This process is the service.
    own: bool,
    fail_spawn: bool,
    fail_stop: bool,
    /// The failing spawn takes `restore.lock` and keeps it, so the restore
    /// can't be taken back.
    hold_lock: bool,
}

impl Default for Service {
    fn default() -> Self {
        Self {
            protocol: Some(CREW_PROTOCOL),
            loaded: true,
            own: false,
            fail_spawn: false,
            fail_stop: false,
            hold_lock: false,
        }
    }
}

/// Records each call, and what was in place when the applier spawned and
/// the service stopped.
struct FakeControl {
    service: Mutex<Service>,
    events: Mutex<Vec<&'static str>>,
    /// `stop_service` waits until this opens.
    gate: (Mutex<bool>, Condvar),
    /// `service_installed_protocol` waits until this opens, after setting
    /// `asked`: an apply still checking.
    status_gate: (Mutex<bool>, Condvar),
    asked: Mutex<bool>,
    roots: Roots,
    pages: FakePages,
    /// Whether `pending.json` was there when the applier spawned.
    pending_at_spawn: Mutex<Option<bool>>,
    /// How many times the bruh's browser had closed when the service
    /// stopped.
    closes_at_stop: Mutex<Option<usize>>,
    /// `restore.lock`, while a failing spawn holds it.
    held: Mutex<Option<std::fs::File>>,
    /// The job hold the server keeps until it exits; [`Self::reset`]
    /// drops it, as a restart does.
    exit_hold: Mutex<Option<JobHold>>,
}

fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl FakeControl {
    fn events(&self) -> Vec<&'static str> {
        locked(&self.events).clone()
    }

    fn record(&self, event: &'static str) {
        locked(&self.events).push(event);
    }

    fn set_gate(&self, open: bool) {
        *locked(&self.gate.0) = open;
        self.gate.1.notify_all();
    }

    fn set_status_gate(&self, open: bool) {
        *locked(&self.status_gate.0) = open;
        self.status_gate.1.notify_all();
    }

    fn set(&self, change: impl FnOnce(&mut Service)) {
        change(&mut locked(&self.service));
    }

    fn reset(&self) {
        locked(&self.events).clear();
        *locked(&self.pending_at_spawn) = None;
        *locked(&self.closes_at_stop) = None;
        locked(&self.exit_hold).take();
    }
}

/// Waits until `gate` opens.
fn wait_open(gate: &(Mutex<bool>, Condvar)) {
    let open = gate
        .1
        .wait_while(locked(&gate.0), |open| !*open)
        .unwrap_or_else(PoisonError::into_inner);
    drop(open);
}

impl RestoreControl for FakeControl {
    fn service_installed_protocol(&self) -> Option<u32> {
        *locked(&self.asked) = true;
        wait_open(&self.status_gate);
        locked(&self.service).protocol
    }

    fn service_loaded(&self) -> bool {
        locked(&self.service).loaded
    }

    fn is_service_process(&self) -> bool {
        locked(&self.service).own
    }

    fn stop_service(&self) -> Result<bool, String> {
        wait_open(&self.gate);
        *locked(&self.closes_at_stop) = Some(self.pages.closes(BRUH));
        self.record("stop_service");
        let service = *locked(&self.service);
        if service.fail_stop {
            Err("refused".into())
        } else {
            Ok(service.loaded)
        }
    }

    fn spawn_applier(&self) -> std::io::Result<()> {
        *locked(&self.pending_at_spawn) = Some(pending(&self.roots).is_some());
        self.record("spawn_applier");
        let service = *locked(&self.service);
        if service.hold_lock {
            let lock = std::fs::File::open(self.roots.core.join("backups/restore.lock")).unwrap();
            lock.lock().unwrap();
            *locked(&self.held) = Some(lock);
        }
        if service.fail_spawn {
            Err(std::io::Error::other("refused"))
        } else {
            Ok(())
        }
    }

    fn exit_soon(&self, hold: JobHold) {
        self.record("exit_soon");
        *locked(&self.exit_hold) = Some(hold);
    }
}

struct Fixture {
    base: String,
    client: reqwest::Client,
    server: tokio::task::JoinHandle<()>,
    host: BackupHost,
    roots: Roots,
    store: SecretStore,
    reactive: ReactiveRuntime,
    /// The live crew's database, outside the roots like a
    /// `BRUHS_REACTIVE_DB` override, so the test's own apply isn't kept
    /// waiting by it.
    reactive_db: PathBuf,
    pages: FakePages,
    tools: Arc<LocalToolsHost>,
    app: Arc<AppState>,
    control: Arc<FakeControl>,
    _directory: tempfile::TempDir,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.control.set_gate(true);
        self.control.set_status_gate(true);
        self.server.abort();
    }
}

impl Fixture {
    async fn new() -> Self {
        Self::with(false, Service::default()).await
    }

    /// With a capture daemon that records when `recording`, and a service
    /// that answers as `service` says.
    async fn with(recording: bool, service: Service) -> Self {
        Self::with_host(recording, service, |host| host).await
    }

    /// [`Self::with`], with the backup host as `host` makes it.
    async fn with_host(
        recording: bool,
        service: Service,
        host: impl FnOnce(BackupHost) -> BackupHost,
    ) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let base_dir = std::fs::canonicalize(directory.path()).unwrap();
        let home = base_dir.join("home");
        let roots = Roots {
            core: home.join("core"),
            desktop: home.join("desktop"),
            config: home.join("core"),
            home,
        };
        for root in [&roots.core, &roots.desktop] {
            std::fs::create_dir_all(root).unwrap();
        }
        write(&roots.core.join(NOTE), b"today");
        let store = SecretStore::in_memory();
        let host = host(BackupHost::new(roots.clone(), store.clone()).for_tests(recording));

        let crew = base_dir.join("crew");
        std::fs::create_dir_all(crew.join("work")).unwrap();
        let reactive_db = crew.join("reactive.sqlite3");
        let reactive = ReactiveRuntime::open(&reactive_db).await.unwrap();
        live_crew(&reactive, &reactive_db, &crew.join("work")).await;

        let (tools, pages) = LocalToolsHost::with_fake_pages();
        tools
            .sync_bruhs(vec![BruhComputer {
                agent_id: BRUH.into(),
                name: "Bruh".into(),
                paused: false,
                profile_dir: crew.join("browser"),
            }])
            .await;
        tools.open_computer(BRUH).await.unwrap();

        let control = Arc::new(FakeControl {
            service: Mutex::new(service),
            events: Mutex::default(),
            gate: (Mutex::new(true), Condvar::new()),
            status_gate: (Mutex::new(true), Condvar::new()),
            asked: Mutex::default(),
            roots: roots.clone(),
            pages: pages.clone(),
            pending_at_spawn: Mutex::default(),
            closes_at_stop: Mutex::default(),
            held: Mutex::default(),
            exit_hold: Mutex::default(),
        });
        let state = Arc::new(
            AppState::builder()
                .secret_store(SecretStore::in_memory())
                .connections(bruhs_core::connections::Connections::in_memory())
                .build(),
        );
        let config = crate::config::Config::from_settings(crate::config::Settings {
            port: Some("0"),
            web_dist: base_dir.join("missing").to_str(),
            ..Default::default()
        })
        .unwrap();
        let as_control: Control = control.clone();
        let app = crate::router(crate::config::ServerState {
            app: Arc::clone(&state),
            config: Arc::new(config),
            updates: bruhs_core::updates::UpdateChecker::new(bruhs_core::updates::BuildInfo::new(
                "", "", "unused",
            )),
        })
        .layer(Extension(host.clone()))
        .layer(Extension(reactive.clone()))
        .layer(Extension(Arc::clone(&tools)))
        .layer(Extension(as_control));
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
            store,
            reactive,
            reactive_db,
            pages,
            tools,
            app: state,
            control,
            _directory: directory,
        }
    }

    fn request(&self, method: &str, path: &str) -> reqwest::RequestBuilder {
        self.client.request(
            Method::from_bytes(method.as_bytes()).unwrap(),
            format!("{}{path}", self.base),
        )
    }

    async fn send(request: reqwest::RequestBuilder) -> (StatusCode, String) {
        let response = request.send().await.unwrap();
        (response.status(), response.text().await.unwrap())
    }

    async fn post(&self, path: &str, body: &Value) -> (StatusCode, String) {
        Self::send(self.request("POST", path).json(body)).await
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

    /// A web backup of the data as it is now, as the browser downloads it.
    async fn backup_file(&self) -> Vec<u8> {
        let (status, text) = self
            .post(
                "/api/backup",
                &json!({ "password": PASSWORD, "app": "web" }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{text}");
        let job: BackupJob = serde_json::from_str(&text).unwrap();
        let JobState::Created { file } = self.finished(&job.id).await else {
            panic!("the backup wasn't made");
        };
        let response = self
            .request("GET", &file.download.unwrap())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        response.bytes().await.unwrap().to_vec()
    }

    /// Uploads a backup file to restore: its id.
    async fn upload(&self, file: Vec<u8>) -> String {
        let (status, text) = Self::send(
            self.request("POST", "/api/backup/restore/uploads")
                .header("content-type", "application/octet-stream")
                .body(file),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{text}");
        let UploadId { id } = serde_json::from_str(&text).unwrap();
        id
    }

    /// Opens `source` with `password` and waits for the job to end.
    async fn open(&self, source: Value, password: &str) -> (String, JobState) {
        let (status, text) = self
            .post(
                "/api/backup/restore",
                &json!({ "source": source, "password": password }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{text}");
        let job: BackupJob = serde_json::from_str(&text).unwrap();
        let state = self.finished(&job.id).await;
        (job.id, state)
    }

    /// Backs up "restored" in the note, writes "today" over it, and
    /// stages the backup: its preview.
    async fn staged(&self) -> RestorePreview {
        write(&self.roots.core.join(NOTE), b"restored");
        let file = self.backup_file().await;
        write(&self.roots.core.join(NOTE), b"today");
        let id = self.upload(file).await;
        let (_, state) = self
            .open(json!({ "type": "upload", "id": id }), PASSWORD)
            .await;
        let JobState::Opened { preview } = state else {
            panic!("the backup didn't open: {state:?}");
        };
        assert!(
            self.host.uploaded(UploadKind::Restore, &id).is_none(),
            "the upload goes once opened"
        );
        *preview
    }

    async fn apply(
        &self,
        source: Value,
        replace: bool,
        from_desktop: bool,
    ) -> (StatusCode, String) {
        self.post(
            "/api/backup/restore/apply",
            &json!({ "source": source, "replace": replace, "from_desktop": from_desktop }),
        )
        .await
    }

    /// Runs the detached applier's one try.
    async fn apply_pending(&self) -> ApplyOutcome {
        let (roots, store) = (self.roots.clone(), self.store.clone());
        tokio::task::spawn_blocking(move || apply_pending(&roots, &store, None))
            .await
            .unwrap()
            .unwrap()
    }

    async fn status(&self) -> RestoreStatus {
        let (status, text) = Self::send(self.request("GET", "/api/backup/restore/status")).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        serde_json::from_str(&text).unwrap()
    }

    fn note(&self) -> String {
        std::fs::read_to_string(self.roots.core.join(NOTE)).unwrap()
    }

    /// The live crew as the apply left it: whether `BRUH` is paused, its
    /// grants and its run's status.
    async fn crew(&self) -> (bool, i64, String) {
        let snapshot = self.reactive.snapshot().await.unwrap();
        let paused = snapshot
            .bruhs
            .iter()
            .any(|bruh| bruh.status == BruhStatus::Paused);
        assert!(!snapshot.bruhs.is_empty());
        let db = rusqlite::Connection::open(&self.reactive_db).unwrap();
        let grants = db
            .query_row(
                "SELECT count(*) FROM reactive_approval_grants WHERE agent_id=?1",
                [BRUH],
                |row| row.get(0),
            )
            .unwrap();
        let run = db
            .query_row(
                "SELECT status FROM reactive_runs WHERE agent_id=?1",
                [BRUH],
                |row| row.get(0),
            )
            .unwrap();
        (paused, grants, run)
    }
}

/// `BRUH` with a standing grant and a run in flight, written as a running
/// server leaves them.
async fn live_crew(runtime: &ReactiveRuntime, db: &Path, work: &Path) {
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveAgent {
                agent: ReactiveAgent {
                    id: BRUH.into(),
                    name: "Bruh".into(),
                    instructions: "Prepare drafts for review.".into(),
                    memory: String::new(),
                    model: "claude-sonnet-4-6".into(),
                    project_path: work.to_string_lossy().into_owned(),
                    enabled: true,
                },
            },
        })
        .await
        .unwrap();
    runtime
        .command(ReactiveCommand::Chat {
            agent_id: BRUH.into(),
            message: "Draft the weekly notes".into(),
            run_id: None,
        })
        .await
        .unwrap();
    let db = rusqlite::Connection::open(db).unwrap();
    db.execute(
        "UPDATE reactive_runs SET status='running', payload=json_set(payload,'$.status','running')",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO reactive_approval_grants(agent_id,fingerprint,created) VALUES(?1,'fp',1)",
        [BRUH],
    )
    .unwrap();
}

/// Stages a backup of the fixture's own data through a host beside its
/// own (a recording fixture's host makes no backups): its id.
async fn stage_beside(fixture: &Fixture) -> String {
    let host = BackupHost::new(fixture.roots.clone(), fixture.store.clone()).for_tests(false);
    let path = fixture.roots.home.join("beside.bruhsbak");
    let job = host
        .start_create(CreateRequest {
            password: Zeroizing::new(PASSWORD.into()),
            options: BackupOptions::default(),
            app: BackupApp::Desktop,
            web_part: None,
            web_db_version: None,
            output: Output::File {
                path: path.clone(),
                overwrite: false,
            },
        })
        .await
        .unwrap();
    assert!(matches!(
        ended(&host, &job.id).await,
        JobState::Created { .. }
    ));
    let job = host
        .start_open(path, Zeroizing::new(PASSWORD.into()), false)
        .unwrap();
    assert!(matches!(
        ended(&host, &job.id).await,
        JobState::Opened { .. }
    ));
    job.id
}

/// A core host's job once it stops running.
async fn ended(host: &BackupHost, id: &str) -> JobState {
    loop {
        let state = host.job(id).unwrap().state;
        if !matches!(state, JobState::Running { .. }) {
            return state;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn staged(id: &str) -> Value {
    json!({ "type": "staged", "id": id })
}

fn kept(stamp: &str) -> Value {
    json!({ "type": "pre_restore", "stamp": stamp })
}

#[tokio::test]
async fn a_restore_opens_previews_and_applies_only_when_confirmed() {
    let fixture = Fixture::new().await;
    let preview = fixture.staged().await;
    assert_eq!(preview.made_on, BackupApp::Web);
    let (_, text) = Fixture::send(fixture.request("GET", "/api/backup")).await;
    let overview: BackupOverview = serde_json::from_str(&text).unwrap();
    assert_eq!(
        overview.staged.map(|staged| staged.preview.id),
        Some(preview.id.clone())
    );

    // Not confirmed: nothing is scheduled or stopped.
    assert_eq!(
        fixture.apply(staged(&preview.id), false, false).await,
        (StatusCode::BAD_REQUEST, error(CONFIRM_REPLACE))
    );
    assert!(pending(&fixture.roots).is_none());
    assert!(fixture.control.events().is_empty());
    assert_eq!(fixture.pages.closes(BRUH), 0);
    // Nor is a body that isn't a request.
    let (status, text) = fixture
        .post(
            "/api/backup/restore/apply",
            &json!({ "source": staged(&preview.id), "replace": true, "surprise": 1 }),
        )
        .await;
    assert_eq!(
        (status, text),
        (StatusCode::BAD_REQUEST, error(INVALID_REQUEST))
    );
    assert!(pending(&fixture.roots).is_none());

    let (status, text) = fixture.apply(staged(&preview.id), true, false).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let accepted: ApplyAccepted = serde_json::from_str(&text).unwrap();
    assert!(accepted.service_stopped);
    assert_eq!(
        pending(&fixture.roots).map(|pending| pending.source),
        Some(RestoreSource::Staged {
            id: preview.id.clone()
        })
    );
    // Ruling F22 a: scheduled, then the applier, then the runs and
    // browsers, then the service, then the shutdown.
    assert_eq!(
        fixture.control.events(),
        ["spawn_applier", "stop_service", "exit_soon"]
    );
    assert_eq!(*locked(&fixture.control.pending_at_spawn), Some(true));
    assert_eq!(*locked(&fixture.control.closes_at_stop), Some(1));
    // Ruling F19: the run in flight ended as a server stop leaves it; the
    // bruh isn't paused and keeps its grant.
    assert_eq!(fixture.crew().await, (false, 1, "interrupted".to_owned()));

    // The live data stays until the applier runs.
    assert_eq!(fixture.note(), "today");
    assert!(matches!(
        fixture.apply_pending().await,
        ApplyOutcome::Applied(_)
    ));
    assert_eq!(fixture.note(), "restored");
    assert_eq!(fixture.crew().await, (false, 1, "interrupted".to_owned()));
}

#[tokio::test]
async fn apply_refuses_unsafe_moments() {
    // A meeting records.
    let recording = Fixture::with(true, Service::default()).await;
    let id = stage_beside(&recording).await;
    assert_eq!(
        recording.apply(staged(&id), true, false).await,
        (StatusCode::CONFLICT, error(RECORDING_FIRST))
    );
    assert!(pending(&recording.roots).is_none());
    assert!(recording.control.events().is_empty());
    assert_eq!(recording.pages.closes(BRUH), 0);

    let fixture = Fixture::with(
        false,
        Service {
            protocol: Some(CREW_PROTOCOL - 1),
            ..Service::default()
        },
    )
    .await;
    let preview = fixture.staged().await;
    let refused = |answer: (StatusCode, String), copy: &str| {
        assert_eq!(answer, (StatusCode::CONFLICT, error(copy)));
        assert!(pending(&fixture.roots).is_none(), "{copy}");
        assert!(fixture.control.events().is_empty(), "{copy}");
        assert_eq!(fixture.pages.closes(BRUH), 0, "{copy}");
    };

    // A job runs: an open whose key takes a while to derive.
    let slow = Header::new(
        Kind::Full,
        Kdf::Argon2id(KdfParams {
            passes: 10,
            ..KdfParams::NATIVE
        }),
        [3; 23],
    );
    let mut file = slow.to_bytes().to_vec();
    file.extend_from_slice(&[0; 64]);
    let id = fixture.upload(file).await;
    let (status, text) = fixture
        .post(
            "/api/backup/restore",
            &json!({ "source": { "type": "upload", "id": id }, "password": PASSWORD }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let job: BackupJob = serde_json::from_str(&text).unwrap();
    refused(
        fixture.apply(staged(&preview.id), true, false).await,
        BACKUP_BUSY,
    );
    assert_eq!(
        fixture.finished(&job.id).await,
        JobState::Failed {
            message: WRONG_PASSWORD.into()
        }
    );

    // The desktop runs and the web asks.
    let lock = bruhs_core::backup::take_instance_lock(&bruhs_core::backup::instance_lock_path(
        &fixture.roots.desktop,
    ))
    .unwrap()
    .unwrap();
    refused(
        fixture.apply(staged(&preview.id), true, false).await,
        QUIT_DESKTOP_FIRST,
    );
    // A page that says it is the desktop isn't (final review C-M3).
    refused(
        Fixture::send(
            fixture
                .request("POST", "/api/backup/restore/apply")
                .header("origin", &fixture.base)
                .json(&json!({
                    "source": staged(&preview.id),
                    "replace": true,
                    "from_desktop": true,
                })),
        )
        .await,
        QUIT_DESKTOP_FIRST,
    );
    // The service runs an older crew protocol than the backup's data.
    refused(
        fixture.apply(staged(&preview.id), true, true).await,
        SERVICE_TOO_OLD,
    );
    // With a service that is up to date, the desktop's own request goes
    // on while it holds its lock.
    fixture
        .control
        .set(|service| service.protocol = Some(CREW_PROTOCOL));
    let (status, text) = fixture.apply(staged(&preview.id), true, true).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert!(pending(&fixture.roots).is_some());
    drop(lock);
}

/// Final review C-I1: a relative `BRUHS_REACTIVE_DB` hides the server's
/// lock, so the apply is refused before anything is scheduled; no start
/// then waits on a restore.
#[tokio::test]
async fn a_relative_reactive_database_is_refused_up_front() {
    let fixture = Fixture::with_host(false, Service::default(), |host| {
        host.with_env(|name| (name == "BRUHS_REACTIVE_DB").then(|| "r.sqlite3".into()))
    })
    .await;
    let preview = fixture.staged().await;
    assert_eq!(
        fixture.apply(staged(&preview.id), true, false).await,
        (StatusCode::CONFLICT, error(REACTIVE_DB_RELATIVE))
    );
    assert!(pending(&fixture.roots).is_none());
    assert!(!restore_waits(&fixture.roots), "a start finds nothing");
    assert!(fixture.control.events().is_empty());
    assert_eq!(fixture.pages.closes(BRUH), 0);
    assert_eq!(fixture.crew().await, (false, 1, "running".to_owned()));
    // A full path's lock can be checked: the apply goes ahead.
    let full = fixture.reactive_db.clone();
    let fixture_full = Fixture::with_host(false, Service::default(), move |host| {
        host.with_env(move |name| (name == "BRUHS_REACTIVE_DB").then(|| full.clone().into()))
    })
    .await;
    let preview = fixture_full.staged().await;
    let (status, text) = fixture_full.apply(staged(&preview.id), true, false).await;
    assert_eq!(status, StatusCode::OK, "{text}");
}

/// Final review C-M2: from the moment an apply starts checking, no job
/// starts; once the restore is handed over, none starts again before the
/// server exits.
#[tokio::test]
async fn no_job_starts_while_an_apply_checks() {
    let fixture = Fixture::new().await;
    let preview = fixture.staged().await;
    fixture.control.set_status_gate(false);
    let create = async {
        while !*locked(&fixture.control.asked) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let answer = fixture
            .post(
                "/api/backup",
                &json!({ "password": PASSWORD, "app": "web" }),
            )
            .await;
        fixture.control.set_status_gate(true);
        answer
    };
    let (applied, created) = tokio::join!(fixture.apply(staged(&preview.id), true, false), create);
    assert_eq!(created, (StatusCode::CONFLICT, error(BACKUP_BUSY)));
    assert_eq!(applied.0, StatusCode::OK, "{}", applied.1);
    assert!(fixture.host.hold_jobs().is_err(), "held until the exit");
}

#[tokio::test]
async fn a_restore_that_cannot_hand_over_is_taken_back() {
    let fixture = Fixture::with(
        false,
        Service {
            fail_spawn: true,
            ..Service::default()
        },
    )
    .await;
    let preview = fixture.staged().await;
    // The applier doesn't start: nothing stops.
    assert_eq!(
        fixture.apply(staged(&preview.id), true, false).await,
        (StatusCode::SERVICE_UNAVAILABLE, error(RESTORE_NOT_STARTED))
    );
    assert!(pending(&fixture.roots).is_none());
    assert_eq!(fixture.control.events(), ["spawn_applier"]);
    assert_eq!(fixture.pages.closes(BRUH), 0);
    assert_eq!(fixture.crew().await, (false, 1, "running".to_owned()));

    // The service doesn't stop: the restore is taken back, the server
    // runs on and the crew's worker can start again.
    fixture.control.reset();
    fixture.control.set(|service| {
        service.fail_spawn = false;
        service.fail_stop = true;
    });
    assert_eq!(
        fixture.apply(staged(&preview.id), true, false).await,
        (StatusCode::SERVICE_UNAVAILABLE, error(SERVICE_NOT_STOPPED))
    );
    assert!(pending(&fixture.roots).is_none());
    assert_eq!(fixture.control.events(), ["spawn_applier", "stop_service"]);
    assert_eq!(fixture.note(), "today");
    // B Task 12 I1: the halt is undone. The worker runs again, and the
    // next sync gives the closed browser a fresh slot that opens.
    assert_eq!(fixture.pages.closes(BRUH), 1);
    assert!(
        fixture.reactive.start(Arc::clone(&fixture.app)).is_err(),
        "the worker runs again"
    );
    fixture
        .tools
        .sync_bruhs(vec![BruhComputer {
            agent_id: BRUH.into(),
            name: "Bruh".into(),
            paused: false,
            profile_dir: fixture.reactive_db.with_file_name("browser"),
        }])
        .await;
    fixture.tools.open_computer(BRUH).await.unwrap();
    assert!(matches!(
        fixture.apply_pending().await,
        ApplyOutcome::NothingPending
    ));
}

/// B Task 12 M2: a restore that can't be taken back says it finishes at
/// the next start.
#[tokio::test]
async fn a_restore_that_cannot_be_taken_back_says_so() {
    let fixture = Fixture::with(
        false,
        Service {
            fail_spawn: true,
            hold_lock: true,
            ..Service::default()
        },
    )
    .await;
    let preview = fixture.staged().await;
    assert_eq!(
        fixture.apply(staged(&preview.id), true, false).await,
        (
            StatusCode::SERVICE_UNAVAILABLE,
            error(RESTORE_AT_NEXT_START)
        )
    );
    assert!(pending(&fixture.roots).is_some(), "it stays scheduled");
    assert_eq!(fixture.pages.closes(BRUH), 0);
}

#[tokio::test]
async fn the_service_process_stops_itself_last() {
    let fixture = Fixture::with(
        false,
        Service {
            own: true,
            ..Service::default()
        },
    )
    .await;
    let preview = fixture.staged().await;
    // Stopping the service waits until the answer arrived: an apply that
    // stopped before it answered never answers.
    fixture.control.set_gate(false);
    let answer = tokio::time::timeout(
        Duration::from_secs(20),
        fixture.apply(staged(&preview.id), true, false),
    )
    .await
    .expect("the service answers before it stops");
    assert_eq!(answer.0, StatusCode::OK, "{}", answer.1);
    let accepted: ApplyAccepted = serde_json::from_str(&answer.1).unwrap();
    assert!(accepted.service_stopped);
    assert_eq!(fixture.control.events(), ["spawn_applier"]);

    fixture.control.set_gate(true);
    for _ in 0..200 {
        if fixture.control.events().len() == 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        fixture.control.events(),
        ["spawn_applier", "stop_service", "exit_soon"]
    );
    assert_eq!(*locked(&fixture.control.closes_at_stop), Some(1));
}

#[tokio::test]
async fn a_wrong_password_answers_without_echo() {
    let fixture = Fixture::new().await;
    let secret = canary();
    let file = fixture.backup_file().await;
    let id = fixture.upload(file.clone()).await;
    let mut answers = Vec::new();
    let (status, text) = fixture
        .post(
            "/api/backup/restore",
            &json!({ "source": { "type": "upload", "id": id }, "password": secret }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let job: BackupJob = serde_json::from_str(&text).unwrap();
    answers.push(text);
    assert_eq!(
        fixture.finished(&job.id).await,
        JobState::Failed {
            message: WRONG_PASSWORD.into()
        }
    );
    answers.push(
        Fixture::send(fixture.request("GET", &format!("/api/backup/jobs/{}", job.id)))
            .await
            .1,
    );
    answers.push(Fixture::send(fixture.request("GET", "/api/backup")).await.1);
    answers.push(
        Fixture::send(fixture.request("GET", "/api/backup/restore/status"))
            .await
            .1,
    );
    // Bodies that aren't requests.
    for body in [
        json!({ "source": { "type": "upload", "id": id }, "password": secret, "surprise": 1 }),
        json!({ "source": { "type": "elsewhere" }, "password": secret }),
    ] {
        let (status, text) = fixture.post("/api/backup/restore", &body).await;
        assert_eq!(
            (status, &text),
            (StatusCode::BAD_REQUEST, &error(INVALID_REQUEST))
        );
        answers.push(text);
    }
    for answer in &answers {
        assert!(
            !answer.contains(&secret),
            "an answer holds the password: {answer}"
        );
    }
    assert!(fixture.host.staged().is_none(), "nothing was staged");

    // The right password opens it; a staged restore can be dropped once.
    let id = fixture.upload(file).await;
    let (job, state) = fixture
        .open(json!({ "type": "upload", "id": id }), PASSWORD)
        .await;
    assert!(matches!(state, JobState::Opened { .. }), "{state:?}");
    let path = format!("/api/backup/restore/{job}");
    assert_eq!(
        Fixture::send(fixture.request("DELETE", &path)).await.0,
        StatusCode::NO_CONTENT
    );
    assert!(fixture.host.staged().is_none());
    assert_eq!(
        Fixture::send(fixture.request("DELETE", &path)).await,
        (StatusCode::NOT_FOUND, error(GONE))
    );
}

#[tokio::test]
async fn uploads_take_raw_bytes_only() {
    let fixture = Fixture::new().await;
    let incoming = fixture.roots.core.join("backups/incoming");
    for kind in [Some("application/json"), None] {
        let mut request = fixture
            .request("POST", "/api/backup/restore/uploads")
            .body(b"BRUHSBAK".to_vec());
        if let Some(kind) = kind {
            request = request.header("content-type", kind);
        }
        assert_eq!(
            Fixture::send(request).await,
            (StatusCode::UNSUPPORTED_MEDIA_TYPE, error(UPLOAD_TYPE)),
            "{kind:?}"
        );
    }
    assert!(!incoming.exists() || std::fs::read_dir(&incoming).unwrap().count() == 0);
    // A file the desktop names must be a regular file at an absolute path.
    for path in ["relative.bruhsbak", "/nowhere/at/all.bruhsbak"] {
        let (status, _) = fixture
            .post(
                "/api/backup/restore",
                &json!({ "source": { "type": "path", "path": path }, "password": PASSWORD }),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }
    let folder = fixture.roots.home.join("a-folder");
    std::fs::create_dir_all(&folder).unwrap();
    let (status, _) = fixture
        .post(
            "/api/backup/restore",
            &json!({ "source": { "type": "path", "path": folder }, "password": PASSWORD }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Home A, where a backup with a browser part is made: its file and the
/// part's bytes.
async fn backup_from_another_home(out: &Path) -> (PathBuf, Vec<u8>) {
    let directory = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(directory.path()).unwrap();
    let home = base.join("home-a");
    let roots = Roots {
        core: home.join("core"),
        desktop: home.join("desktop"),
        config: home.join("core"),
        home,
    };
    for root in [&roots.core, &roots.desktop] {
        std::fs::create_dir_all(root).unwrap();
    }
    write(&roots.core.join(NOTE), b"restored");
    let host = BackupHost::new(roots, SecretStore::in_memory()).for_tests(false);
    let part = container(Kind::WebPart, br#"{"version":1,"stores":{}}"#);
    let mut upload = host.begin_upload(UploadKind::WebPart).unwrap();
    upload.write(&part).unwrap();
    let id = upload.finish().unwrap();
    let path = out.join("from-a.bruhsbak");
    let job = host
        .start_create(CreateRequest {
            password: Zeroizing::new(PASSWORD.into()),
            options: BackupOptions::default(),
            app: BackupApp::Web,
            web_part: host.uploaded(UploadKind::WebPart, &id),
            web_db_version: Some(WEB_DB_VERSION),
            output: Output::File {
                path: path.clone(),
                overwrite: false,
            },
        })
        .await
        .unwrap();
    loop {
        let state = host.job(&job.id).unwrap().state;
        match state {
            JobState::Running { .. } => tokio::time::sleep(Duration::from_millis(20)).await,
            JobState::Created { .. } => break,
            other => panic!("the backup wasn't made: {other:?}"),
        }
    }
    drop(directory);
    (path, part)
}

#[tokio::test]
async fn status_and_the_web_part() {
    let fixture = Fixture::new().await;
    let (file, part) = backup_from_another_home(&fixture.roots.home).await;
    // The desktop names a file on this computer; it stays where it is.
    let (id, state) = fixture
        .open(json!({ "type": "path", "path": file }), PASSWORD)
        .await;
    let JobState::Opened { preview } = state else {
        panic!("the backup didn't open: {state:?}");
    };
    assert!(preview.has_web_part);
    assert!(file.is_file());
    let (status, text) = fixture.apply(staged(&id), true, false).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let waiting = fixture.status().await;
    assert_eq!(
        waiting.pending.map(|pending| pending.source),
        Some(RestoreSource::Staged { id: id.clone() })
    );
    assert!(waiting.web_part.is_none());
    assert!(matches!(
        fixture.apply_pending().await,
        ApplyOutcome::Applied(_)
    ));
    assert_eq!(fixture.note(), "restored");

    // The status names the part with the rewrites from home A to here.
    let done = fixture.status().await;
    assert!(done.pending.is_none());
    assert!(done.result.is_some());
    let web = done.web_part.unwrap();
    assert_eq!(web.bytes, u64::try_from(part.len()).unwrap());
    assert!(
        web.rewrites
            .iter()
            .any(|rule| rule.to == fixture.roots.home.to_string_lossy()),
        "{:?}",
        web.rewrites
    );
    let response = fixture
        .request("GET", "/api/backup/restore/web-part")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-type"],
        "application/octet-stream"
    );
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(
        response.bytes().await.unwrap().to_vec(),
        part,
        "byte for byte"
    );

    // The browser's data from before its restore joins the newest set.
    let before = container(Kind::WebPart, br#"{"version":1,"before":true}"#);
    let (status, text) = Fixture::send(
        fixture
            .request("POST", "/api/backup/restore/web-part/previous")
            .header("content-type", "application/octet-stream")
            .body(before.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{text}");
    let newest = pre_restores(&fixture.roots).remove(0);
    assert!(newest.has_web_part);
    let kept_part = fixture.roots.core.join(format!(
        "backups/pre-restore-{}/meta/web-part.bin",
        newest.stamp
    ));
    assert_eq!(std::fs::read(kept_part).unwrap(), before);
    // Anything but a browser part is refused.
    let (status, _) = Fixture::send(
        fixture
            .request("POST", "/api/backup/restore/web-part/previous")
            .header("content-type", "application/octet-stream")
            .body(container(Kind::Full, b"not a part")),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Once restored, the browser drops the server's copy.
    assert_eq!(
        Fixture::send(fixture.request("DELETE", "/api/backup/restore/web-part"))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert!(fixture.status().await.web_part.is_none());
    assert!(fixture.status().await.result.is_some());
    assert_eq!(
        Fixture::send(fixture.request("GET", "/api/backup/restore/web-part")).await,
        (StatusCode::NOT_FOUND, error(GONE))
    );
    assert_eq!(
        Fixture::send(fixture.request("DELETE", "/api/backup/restore/web-part")).await,
        (StatusCode::NOT_FOUND, error(GONE))
    );
}

#[tokio::test]
async fn kept_sets_can_be_deleted_and_put_back() {
    let fixture = Fixture::new().await;
    let preview = fixture.staged().await;
    let (status, text) = fixture.apply(staged(&preview.id), true, false).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert!(matches!(
        fixture.apply_pending().await,
        ApplyOutcome::Applied(_)
    ));
    assert_eq!(fixture.note(), "restored");
    let first = pre_restores(&fixture.roots).remove(0).stamp;

    // Put back: a restore of the kept set.
    fixture.control.reset();
    let (status, text) = fixture.apply(kept(&first), true, false).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(
        pending(&fixture.roots).map(|pending| pending.source),
        Some(RestoreSource::PreRestore {
            stamp: first.clone()
        })
    );
    assert_eq!(
        fixture.control.events(),
        ["spawn_applier", "stop_service", "exit_soon"]
    );
    // The set a waiting put back uses isn't deleted.
    let path = |stamp: &str| format!("/api/backup/pre-restore/{stamp}");
    assert_eq!(
        Fixture::send(fixture.request("DELETE", &path(&first))).await,
        (StatusCode::CONFLICT, error(RESTORE_PENDING))
    );
    assert!(matches!(
        fixture.apply_pending().await,
        ApplyOutcome::Applied(_)
    ));
    assert_eq!(fixture.note(), "today");
    let sets = pre_restores(&fixture.roots);
    assert_eq!(sets.len(), 1, "the put back consumed its set");
    let second = sets[0].stamp.clone();
    assert_ne!(second, first);

    // A set whose delete stopped after its key went: its put back answers
    // KEYCHAIN_SET_UNREADABLE at once, stopping nothing, and Delete it
    // again finishes it (B Task 8 R3-M2).
    bruhs_core::secrets::forget_seal_key(&fixture.store, &format!("pre-restore:{second}")).unwrap();
    fixture.control.reset();
    assert_eq!(
        fixture.apply(kept(&second), true, false).await,
        (StatusCode::CONFLICT, error(KEYCHAIN_SET_UNREADABLE))
    );
    assert!(pending(&fixture.roots).is_none());
    assert!(fixture.control.events().is_empty());
    assert_eq!(
        Fixture::send(fixture.request("DELETE", &path(&second)))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert!(pre_restores(&fixture.roots).is_empty());
    assert_eq!(
        Fixture::send(fixture.request("DELETE", &path(&second))).await,
        (StatusCode::NOT_FOUND, error(GONE))
    );
    for stamp in ["2026-11-20-1200", "2026-11-20-1200-ABCD", "..%2Fx"] {
        assert_eq!(
            Fixture::send(fixture.request("DELETE", &path(stamp))).await,
            (StatusCode::NOT_FOUND, error(GONE)),
            "{stamp}"
        );
    }
}

/// A set that holds only what a stopped restore parked offers only
/// Delete.
#[tokio::test]
async fn a_set_with_nothing_to_put_back_offers_only_delete() {
    let fixture = Fixture::new().await;
    let preview = fixture.staged().await;
    let (status, text) = fixture.apply(staged(&preview.id), true, false).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert!(matches!(
        fixture.apply_pending().await,
        ApplyOutcome::Applied(_)
    ));
    let stamp = pre_restores(&fixture.roots).remove(0).stamp;
    // Its info names nothing but what a rollback parked.
    let info = fixture
        .roots
        .core
        .join(format!("backups/pre-restore-{stamp}/meta/info.json"));
    let mut record: Value = serde_json::from_slice(&std::fs::read(&info).unwrap()).unwrap();
    for root in record["roots"].as_array_mut().unwrap() {
        root["names"] = json!([]);
    }
    std::fs::write(&info, serde_json::to_vec(&record).unwrap()).unwrap();
    fixture.control.reset();
    assert_eq!(
        fixture.apply(kept(&stamp), true, false).await,
        (StatusCode::CONFLICT, error(NOTHING_TO_PUT_BACK))
    );
    assert!(pending(&fixture.roots).is_none());
    assert!(fixture.control.events().is_empty());
    assert_eq!(
        Fixture::send(fixture.request("DELETE", &format!("/api/backup/pre-restore/{stamp}")))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
}
