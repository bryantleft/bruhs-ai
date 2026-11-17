use std::sync::Arc;

use bruhs_core::google_calendar::{
    cursor::CalendarFiles, tokens::TokenStore, GoogleCalendarHandle,
};
use bruhs_core::secrets::SecretStore;
use bruhs_core::{AppState, BackgroundTask, TaskKind};
use serde_json::{json, Value};
use tokio::task::JoinHandle;

use crate::config::{Config, ServerState, Settings};

pub(super) const SPARSE_THREAD: &str = "http/sparse Ω";

/// The combined stream opens with the task snapshot, the calendar and
/// meetings handshakes and, on Unix, the workspace relay's hello frame
/// (`crate::workspace_relay`).
const COMBINED_OPENING_FRAMES: usize = if cfg!(unix) { 4 } else { 3 };

pub(super) struct Fixture {
    pub(super) state: Arc<AppState>,
    pub(super) tasks: Vec<BackgroundTask>,
    pub(super) client: reqwest::Client,
    base: String,
    server: Option<JoinHandle<()>>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    _directory: tempfile::TempDir,
}

impl Fixture {
    pub(super) async fn new(records: usize) -> Self {
        let directory = tempfile::tempdir().expect("private HTTP fixture");
        let state = Arc::new(
            AppState::builder()
                .secret_store(SecretStore::in_memory())
                .connections(bruhs_core::connections::Connections::in_memory())
                .google_calendar(Arc::new(GoogleCalendarHandle::new(
                    None,
                    TokenStore::in_memory(),
                    CalendarFiles::at(directory.path()),
                )))
                .build(),
        );
        let tasks = (0..records)
            .map(|index| {
                state.registry().register(
                    if index + 1 == records {
                        SPARSE_THREAD
                    } else {
                        "http-background"
                    },
                    TaskKind::Shell,
                    format!("fixture-{index:06}"),
                    json!({"fixture_ordinal": index, "text": "private HTTP fixture Ω"}),
                )
            })
            .collect();
        let missing_spa = directory.path().join("no-spa");
        let config = Config::from_settings(Settings {
            port: Some("0"),
            web_dist: missing_spa.to_str(),
            ..Settings::default()
        })
        .expect("explicit fixture config");
        let router =
            crate::router(ServerState {
                app: state.clone(),
                config: Arc::new(config),
                updates: bruhs_core::updates::UpdateChecker::new(
                    bruhs_core::updates::BuildInfo::new("", "", "fixture-unused"),
                ),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("private loopback listener");
        let base = format!("http://{}", listener.local_addr().expect("fixture address"));
        let (shutdown, shutdown_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("fixture HTTP server");
        });
        // The existing stream integration tests use the same five-second
        // liveness deadline. It is a fixture failure bound, not a latency budget.
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("fixture client");
        Self {
            state,
            tasks,
            client,
            base,
            server: Some(server),
            shutdown: Some(shutdown),
            _directory: directory,
        }
    }

    pub(super) fn url(&self, path: &str) -> String {
        assert!(
            path == "/api/health"
                || path == "/api/tasks"
                || path.starts_with("/api/tasks/")
                || path == "/api/events/stream"
                || path == "/api/events/stream?resume=true",
            "fixture route whitelist: {path}"
        );
        format!("{}{path}", self.base)
    }

    pub(super) fn extra_task(&self) -> BackgroundTask {
        self.state.registry().register(
            "http-event",
            TaskKind::Shell,
            format!("fixture-{:06}", self.tasks.len()),
            json!({"fixture_ordinal": self.tasks.len(), "text": "private HTTP fixture Ω"}),
        )
    }

    pub(super) fn cleanup(&self, task: &BackgroundTask) {
        if self.state.registry().get(&task.id).is_some() {
            self.state.registry().mark_stream_ended("http-event");
            self.state
                .registry()
                .remove(&task.id)
                .expect("remove fixture task");
        }
    }

    pub(super) fn canonical_tasks(
        &self,
        actual: Value,
        filter: Option<&str>,
        extra: Option<&BackgroundTask>,
    ) -> Value {
        let mut expected: Vec<Value> = self
            .tasks
            .iter()
            .chain(extra)
            .filter(|task| filter.is_none_or(|thread| task.thread_id == thread))
            .map(|task| serde_json::to_value(task).expect("fixture task JSON"))
            .collect();
        let mut actual = actual.as_array().expect("task array").clone();
        expected.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        actual.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        assert_eq!(actual, expected, "entire task objects and exact membership");
        for task in &mut actual {
            let ordinal = task["meta"]["fixture_ordinal"]
                .as_u64()
                .expect("fixture ordinal");
            task["id"] = json!(format!("fixture-task-{ordinal:06}"));
            task["started_at"] = json!(0);
            if !task["ended_at"].is_null() {
                task["ended_at"] = json!(0);
            }
        }
        actual.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        Value::Array(actual)
    }

    pub(super) fn canonical_opened(&self, frames: &[Frame], combined: bool) -> Value {
        assert_eq!(
            frames.len(),
            if combined { COMBINED_OPENING_FRAMES } else { 1 }
        );
        let task = frames
            .iter()
            .find(|frame| frame.event() == combined.then_some("tasks"))
            .expect("task snapshot");
        let tasks = self.canonical_tasks(task.value(), None, None);
        if combined {
            for name in ["calendar", "meetings"] {
                let frame = frames
                    .iter()
                    .find(|frame| frame.event() == Some(name))
                    .expect("named handshake");
                assert_eq!(frame.value(), json!({"kind": "stream_opened"}));
            }
            if cfg!(unix) {
                // The workspace relay's hello names this stream with a fresh
                // secret, so only its shape is canonical.
                let hello = frames
                    .iter()
                    .find(|frame| frame.event() == Some("workspace_bridge"))
                    .expect("workspace relay hello")
                    .value();
                assert_eq!(hello["kind"], "hello");
                assert!(hello["stream_id"]
                    .as_str()
                    .is_some_and(|id| id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit())));
            }
            json!({"tasks": tasks, "calendar": {"kind": "stream_opened"}, "meetings": {"kind": "stream_opened"}})
        } else {
            tasks
        }
    }

    pub(super) async fn close(mut self) {
        self.shutdown
            .take()
            .expect("fixture shutdown sender")
            .send(())
            .expect("fixture listener alive");
        let mut server = self.server.take().expect("live fixture server");
        match tokio::time::timeout(std::time::Duration::from_secs(5), &mut server).await {
            Ok(result) => result.expect("fixture server shuts down without panic"),
            Err(_) => {
                server.abort();
                panic!("fixture connections did not close before the liveness deadline");
            }
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(server) = &self.server {
            server.abort();
        }
    }
}

pub(super) fn assert_headers(headers: &reqwest::header::HeaderMap) {
    assert_eq!(headers[reqwest::header::CACHE_CONTROL], "no-store");
    assert_eq!(headers[reqwest::header::X_FRAME_OPTIONS], "DENY");
    assert_eq!(headers[reqwest::header::X_CONTENT_TYPE_OPTIONS], "nosniff");
}

pub(super) fn assert_health(value: &Value) {
    let release = env!("BRUHS_RELEASE_VERSION");
    assert_eq!(
        value,
        &json!({"ok": true, "app": "bruhs-server", "version": if release.is_empty() { env!("CARGO_PKG_VERSION") } else { release }, "build": env!("BRUHS_BUILD_COMMIT")})
    );
}

pub(super) struct Frame {
    raw: Vec<u8>,
    pub(super) bytes: usize,
}

impl Frame {
    pub(super) fn event(&self) -> Option<&str> {
        std::str::from_utf8(&self.raw)
            .expect("complete UTF-8 SSE frame")
            .lines()
            .find_map(|line| line.strip_prefix("event: "))
    }

    pub(super) fn value(&self) -> Value {
        let text = std::str::from_utf8(&self.raw).expect("complete UTF-8 SSE frame");
        assert!(
            text.lines().all(|line| line.is_empty()
                || line.starts_with("event: ")
                || line.starts_with("data: ")),
            "unexpected semantic SSE field"
        );
        assert!(
            text.lines()
                .filter(|line| line.starts_with("event: "))
                .count()
                <= 1,
            "one event name per frame"
        );
        let data = text
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!data.is_empty(), "expected semantic frame before keepalive");
        serde_json::from_str(&data).expect("complete SSE JSON")
    }
}

pub(super) struct Frames {
    response: reqwest::Response,
    buffered: Vec<u8>,
    searched: usize,
}

impl Frames {
    pub(super) fn new(response: reqwest::Response) -> Self {
        Self {
            response,
            buffered: Vec::new(),
            searched: 0,
        }
    }

    pub(super) fn headers(&self) -> &reqwest::header::HeaderMap {
        self.response.headers()
    }

    pub(super) async fn next(&mut self) -> Frame {
        loop {
            if let Some(end) = self.buffered[self.searched..]
                .windows(2)
                .position(|bytes| bytes == b"\n\n")
            {
                let bytes = self.searched + end + 2;
                let raw = self.buffered.drain(..bytes).collect();
                self.searched = 0;
                return Frame { raw, bytes };
            }
            self.searched = self.buffered.len().saturating_sub(1);
            let chunk = self
                .response
                .chunk()
                .await
                .expect("SSE chunk")
                .expect("stream remains open until fixture closes it");
            self.buffered.extend_from_slice(&chunk);
        }
    }

    pub(super) async fn opened(&mut self, combined: bool) -> Vec<Frame> {
        let mut frames = Vec::new();
        for _ in 0..if combined { COMBINED_OPENING_FRAMES } else { 1 } {
            frames.push(self.next().await);
        }
        frames
    }
}
