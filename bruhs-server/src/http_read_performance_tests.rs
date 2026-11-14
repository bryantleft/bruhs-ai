//! Actual-router reads over isolated stores and directories, without application startup.

mod capture_proxy;
mod contacts;
mod filesystem;
mod generation_tcp;
mod meeting_writes;
mod native_local_focus;
mod stores;

use std::{path::Path, sync::Arc, time::Instant};

use bruhs_core::{
    google_calendar::{cursor::CalendarFiles, tokens::TokenStore, GoogleCalendarHandle},
    meetings::{InMemoryMeetingStore, MeetingStore},
    secrets::SecretStore,
    AppState,
};
use reqwest::Method;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};

use crate::config::{Config, ServerState, Settings};

pub(crate) struct Host {
    pub(crate) state: Arc<AppState>,
    pub(crate) client: reqwest::Client,
    pub(crate) base: String,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    server: Option<tokio::task::JoinHandle<()>>,
}

impl Host {
    async fn new(directory: &Path, store: Option<Arc<dyn MeetingStore>>) -> Self {
        Self::new_with_peers(directory, store, None).await
    }

    pub(crate) async fn new_with_peers(
        directory: &Path,
        store: Option<Arc<dyn MeetingStore>>,
        peers: Option<Arc<std::sync::Mutex<Vec<std::net::SocketAddr>>>>,
    ) -> Self {
        Self::new_with_capture_daemon(directory, store, peers, None).await
    }

    async fn new_with_capture_daemon(
        directory: &Path,
        store: Option<Arc<dyn MeetingStore>>,
        peers: Option<Arc<std::sync::Mutex<Vec<std::net::SocketAddr>>>>,
        capture_daemon: Option<bruhs_core::capture_client::DaemonEndpoint>,
    ) -> Self {
        let state = Arc::new(
            AppState::builder()
                .secret_store(SecretStore::in_memory())
                .connections(bruhs_core::connections::Connections::in_memory())
                .meetings(store.unwrap_or_else(|| Arc::new(InMemoryMeetingStore::new())))
                .google_calendar(Arc::new(GoogleCalendarHandle::new(
                    None,
                    TokenStore::in_memory(),
                    CalendarFiles::at(directory),
                )))
                .build(),
        );
        let missing_spa = directory.join("no-spa");
        let config = Config::from_settings(Settings {
            port: Some("0"),
            web_dist: missing_spa.to_str(),
            ..Settings::default()
        })
        .expect("explicit read fixture config");
        let server_state =
            ServerState {
                app: state.clone(),
                config: Arc::new(config),
                updates: bruhs_core::updates::UpdateChecker::new(
                    bruhs_core::updates::BuildInfo::new("", "", "fixture-unused"),
                ),
            };
        let mut router = match capture_daemon {
            Some(daemon) => crate::router_with_capture_daemon(server_state, daemon),
            None => crate::router(server_state),
        };
        if let Some(peers) = peers {
            router = router.layer(axum::middleware::from_fn(
                move |axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<
                    std::net::SocketAddr,
                >,
                      request: axum::extract::Request,
                      next: axum::middleware::Next| {
                    let peers = peers.clone();
                    async move {
                        peers.lock().expect("private TCP peer log").push(peer);
                        next.run(request).await
                    }
                },
            ));
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("private read fixture listener");
        let base = format!("http://{}", listener.local_addr().expect("fixture address"));
        let (shutdown, receiver) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .with_graceful_shutdown(async {
                let _ = receiver.await;
            })
            .await
            .expect("read fixture server");
        });
        Self {
            state,
            // Match the existing HTTP integration fixture's liveness deadline.
            client: reqwest::Client::builder()
                .no_proxy()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .expect("read fixture client"),
            base,
            shutdown: Some(shutdown),
            server: Some(server),
        }
    }

    pub(crate) async fn close(mut self) {
        self.shutdown
            .take()
            .expect("shutdown sender")
            .send(())
            .expect("live listener");
        let mut server = self.server.take().expect("server task");
        match tokio::time::timeout(std::time::Duration::from_secs(5), &mut server).await {
            Ok(result) => result.expect("clean read fixture shutdown"),
            Err(_) => {
                server.abort();
                panic!("read fixture did not shut down before its liveness deadline");
            }
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        if let Some(sender) = self.shutdown.take() {
            let _ = sender.send(());
        }
        if let Some(server) = &self.server {
            server.abort();
        }
    }
}

struct ReadCase {
    name: &'static str,
    action: &'static str,
    method: Method,
    path: String,
    body: Option<Value>,
    status: u16,
    expected: Value,
    canonical: Value,
}

impl ReadCase {
    fn get(name: &'static str, action: &'static str, path: String, expected: Value) -> Self {
        Self {
            name,
            action,
            method: Method::GET,
            path,
            body: None,
            status: 200,
            canonical: expected.clone(),
            expected,
        }
    }

    fn post(name: &'static str, action: &'static str, body: Value, expected: Value) -> Self {
        Self {
            name,
            action,
            method: Method::POST,
            path: action
                .strip_prefix("POST ")
                .expect("POST action")
                .to_owned(),
            body: Some(body),
            status: 200,
            canonical: expected.clone(),
            expected,
        }
    }

    fn error(mut self, status: u16) -> Self {
        self.status = status;
        self
    }
}

#[derive(Serialize)]
struct Measurement {
    case: &'static str,
    action: &'static str,
    backend: &'static str,
    records: usize,
    shape: &'static str,
    samples_ms: Vec<f64>,
    warmups: usize,
    response_bytes: usize,
    canonical_response_bytes: usize,
    canonical_response_sha256: String,
    status: u16,
}

async fn measure(
    host: &Host,
    case: ReadCase,
    backend: &'static str,
    records: usize,
    shape: &'static str,
    warmups: usize,
    samples: usize,
) -> Measurement {
    let mut intervals = Vec::new();
    let mut response_bytes = None;
    for index in 0..warmups + samples {
        assert!(
            case.path.starts_with("/api/people")
                || case.path.starts_with("/api/meetings")
                || matches!(
                    case.path.as_str(),
                    "/api/agent/list-dir"
                        | "/api/agent/files/children"
                        | "/api/agent/validate-path"
                        | "/api/agent/git-branches"
                        | "/api/agent/git-status"
                        | "/api/agent/git-diff"
                )
        );
        let mut request = host
            .client
            .request(case.method.clone(), format!("{}{}", host.base, case.path));
        if let Some(body) = &case.body {
            request = request.json(body);
        }
        let request = request.build().expect("finite read request");
        let start = Instant::now();
        let response = host
            .client
            .execute(request)
            .await
            .expect("finite read response");
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let body = response.bytes().await.expect("complete finite read body");
        let elapsed = start.elapsed().as_secs_f64() * 1_000.0;
        assert_eq!(status, case.status, "{} status", case.name);
        assert_eq!(headers[reqwest::header::CONTENT_TYPE], "application/json");
        assert_eq!(headers[reqwest::header::CACHE_CONTROL], "no-store");
        assert_eq!(headers[reqwest::header::X_FRAME_OPTIONS], "DENY");
        assert_eq!(headers[reqwest::header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        let value: Value = serde_json::from_slice(&body).expect("finite read JSON");
        assert_eq!(value, case.expected, "{} entire ordered payload", case.name);
        assert!(response_bytes.is_none_or(|length| length == body.len()));
        response_bytes = Some(body.len());
        if index >= warmups {
            intervals.push(elapsed);
        }
    }
    // Full equality is checked before using the fixture's path-normalized form.
    let canonical = serde_json::to_vec(&case.canonical).expect("canonical full response");
    Measurement {
        case: case.name,
        action: case.action,
        backend,
        records,
        shape,
        samples_ms: intervals,
        warmups,
        response_bytes: response_bytes.expect("observed response"),
        canonical_response_bytes: canonical.len(),
        canonical_response_sha256: format!("{:x}", Sha256::digest(&canonical)),
        status: case.status,
    }
}

async fn run(
    warmups: usize,
    samples: usize,
    sizes: &[usize],
    file_sizes: &[usize],
    reverse: bool,
    large_text: bool,
) -> Vec<Measurement> {
    let mut result = Vec::new();
    let mut sizes = sizes.to_vec();
    let mut backends = ["memory", "sqlite"];
    if reverse {
        sizes.reverse();
        backends.reverse();
    }
    for backend in backends {
        for &records in &sizes {
            stores::run(
                &mut result,
                backend,
                records,
                false,
                reverse,
                warmups,
                samples,
            )
            .await;
        }
        if large_text {
            stores::run(&mut result, backend, 1, true, reverse, warmups, samples).await;
        }
    }
    let mut file_sizes = file_sizes.to_vec();
    if reverse {
        file_sizes.reverse();
    }
    for records in file_sizes {
        filesystem::run(&mut result, records, reverse, warmups, samples).await;
    }
    result
}

#[tokio::test]
async fn actual_read_routes_preserve_complete_store_and_filesystem_replies() {
    let rows = run(0, 1, &[0, 1, 3], &[0, 1, 3], false, true).await;
    assert_eq!(rows.len(), 123);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "isolated actual HTTP reads; run optimized and without builders or other measurements"]
async fn read_action_cost() {
    let smoke = std::env::var_os("BRUHS_PERF_READ_HTTP_SMOKE").is_some();
    let reverse = std::env::var("BRUHS_PERF_READ_HTTP_ORDER").is_ok_and(|order| order == "reverse");
    let (warmups, samples) = if smoke { (1, 2) } else { (5, 30) };
    let rows = run(
        warmups,
        samples,
        &[0, 1, 1001],
        &[0, 1, 10001],
        reverse,
        true,
    )
    .await;
    assert_eq!(rows.len(), 123);
    let output =
        std::env::var_os("BRUHS_PERF_READ_HTTP_OUTPUT").expect("explicit read HTTP output");
    std::fs::write(output, serde_json::to_vec_pretty(&json!({
        "schema": 1, "smoke": smoke, "reverse": reverse, "cases": rows,
        "runtime": {"flavor": "multi_thread", "workers": 2, "requests": "serial"},
        "scope": "Actual production router, pooled private loopback client, in-memory or private temporary SQLite stores, and confined temporary directories/repository. Timers cover client execute through complete body. Seed/setup, request construction, JSON/header/full-payload validation, canonicalization, hashing and cleanup excluded. Warm filesystem/database caches; no cold-cache, concurrency, physical UI, capture or provider claim.",
        "canonicalization": "Every complete payload equals independently constructed expected JSON, including array order, before canonicalizing only known fixture paths. Fixed synthetic model IDs/times need no normalization. Raw decoded response bytes and status are recorded separately."
    })).expect("read results JSON")).expect("write read results");
    println!("READ_HTTP_COST 123 variants; complete ordered replies, headers and cleanup passed");
}
