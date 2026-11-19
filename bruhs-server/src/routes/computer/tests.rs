use std::{sync::Arc, time::Duration};

use axum::{http::StatusCode, Extension};
use bruhs_core::local_tools::{FakePages, LocalToolsHost};
use bruhs_core::reactive::ReactiveRuntime;
use bruhs_core::PermissionDecision;
use bruhs_models::crew::{
    AccessLevel, BruhAccess, BruhLook, ACTION_STOPPED, COMPUTER_OWNER_ONLY, LIVE_VIEW_ENDED,
    LOGIN_WRONG_ORIGIN, MAX_VIEWERS, NOT_FOR_BRUHS, PAGE_CHANGED, STEP_FROM_A_PAGE,
    STEP_NOT_APPROVED, TAKE_CONTROL_FIRST, TURN_ON_IN_SETTINGS, VIEWERS_FULL,
};
use bruhs_models::crew::{FILL_MAY_HAVE_RUN, LOGINS_LIST_FAILED, VAULT_READ_FAILED};
use bruhs_models::local_tools::{
    screencast_path, ComputerInput, ComputerState, Control, MouseButton, MouseKind,
    ScreencastMessage, FRAME_ACK,
};
use bruhs_models::local_tools::{BrowserAction, ToolReply};
use bruhs_models::reactive::{ReactiveAgent, ReactiveChange, ReactiveCommand, ReactiveRun};
use bruhs_models::vault::{SecretField, OWNER_ONLY, OWNER_ONLY_CARD, OWNER_ONLY_DELETE_BRUH};

use crate::error::AppError;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest, Message};

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

const APP_ORIGIN: &str = "http://127.0.0.1:3001";
/// The web app's origin as `BRUHS_WEB_ORIGIN` names it.
const CONFIGURED_ORIGIN: &str = "https://bruhs.localhost";
const SHOP: &str = "https://shop.test/in";

struct Fixture {
    runtime: ReactiveRuntime,
    host: Arc<LocalToolsHost>,
    fake: FakePages,
    base: String,
    client: reqwest::Client,
    server: tokio::task::JoinHandle<()>,
    _directory: tempfile::TempDir,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Fixture {
    /// The server with fake bruh browsers, and bruh `a`: a Full Claude
    /// bruh whose Computer is off.
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let runtime = ReactiveRuntime::open(directory.path().join("automation.sqlite3"))
            .await
            .unwrap();
        let (host, fake) = LocalToolsHost::with_fake_pages();
        runtime.attach_computers(Arc::clone(&host)).unwrap();
        // The keychain is in memory, and `op` a stand-in that answers
        // every reference after 5 s.
        let op = directory.path().join("op");
        std::fs::write(&op, "#!/bin/sh\n/bin/sleep 5\nprintf 'op-value'\n").unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&op, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let state = Arc::new(
            bruhs_core::AppState::builder()
                .secret_store(bruhs_core::secrets::SecretStore::in_memory())
                .one_password(bruhs_core::secrets::OnePassword::at(
                    op,
                    Duration::from_secs(10),
                ))
                .connections(bruhs_core::connections::Connections::in_memory())
                .build(),
        );
        runtime.attach_vault(state.secrets().clone()).unwrap();
        let config = crate::config::Config::from_settings(crate::config::Settings {
            port: Some("0"),
            web_origin: Some(CONFIGURED_ORIGIN),
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
        .layer(Extension(runtime.clone()))
        .layer(Extension(Arc::clone(&host)));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let fixture = Self {
            runtime,
            host,
            fake,
            base,
            client,
            server,
            _directory: directory,
        };
        fixture
            .command(ReactiveCommand::Apply {
                change: ReactiveChange::SaveAgent {
                    agent: ReactiveAgent {
                        id: "a".into(),
                        name: "Pix".into(),
                        instructions: "Book the flight.".into(),
                        memory: String::new(),
                        model: "claude-sonnet-4-6".into(),
                        project_path: fixture._directory.path().to_string_lossy().into_owned(),
                        enabled: true,
                    },
                },
            })
            .await;
        fixture
            .command(ReactiveCommand::SetBruh {
                agent_id: "a".into(),
                look: BruhLook::default(),
                access: BruhAccess {
                    level: AccessLevel::Full,
                    ..BruhAccess::default()
                },
                effort: None,
            })
            .await;
        fixture
    }

    async fn command(&self, command: ReactiveCommand) {
        self.runtime.command(command).await.unwrap();
    }

    /// Turns `a`'s Computer on and waits for the host to have it.
    async fn computer_on(&self) {
        self.command(ReactiveCommand::SetComputer {
            agent_id: "a".into(),
            on: true,
        })
        .await;
        self.eventually(|| async {
            self.get("/api/reactive/agents/a/computer")
                .await
                .0
                .is_success()
        })
        .await;
    }

    async fn eventually<F: std::future::Future<Output = bool>>(
        &self,
        mut check: impl FnMut() -> F,
    ) {
        for _ in 0..500 {
            if check().await {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the condition never held");
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        let response = self
            .client
            .get(format!("{}{path}", self.base))
            .send()
            .await
            .unwrap();
        (response.status(), response.json().await.unwrap())
    }

    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.post_with(path, body, None).await
    }

    async fn post_with(
        &self,
        path: &str,
        body: Value,
        origin: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut request = self.client.post(format!("{}{path}", self.base)).json(&body);
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        let response = request.send().await.unwrap();
        (response.status(), response.json().await.unwrap())
    }

    /// `method path` with `body`, from a page at `origin` (none: the
    /// desktop's native client): the status and the raw body.
    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
        origin: Option<&str>,
    ) -> (StatusCode, String) {
        let mut request = self.client.request(method, format!("{}{path}", self.base));
        if let Some(body) = body {
            request = request.json(&body);
        }
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        let response = request.send().await.unwrap();
        (response.status(), response.text().await.unwrap())
    }

    /// A Global Saved login for [`SHOP`] with `password`, granted to `a`:
    /// its id.
    async fn granted_login(&self, password: &str) -> String {
        let (status, body) = self
            .post(
                "/api/vault",
                json!({
                    "name": "Shop",
                    "scope": {"type": "global"},
                    "secret": {"kind": "login", "url": SHOP, "fields": {
                        "source": "saved",
                        "username": "owner@shop.test",
                        "password": password,
                    }},
                }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let id = body["id"].as_str().unwrap().to_owned();
        self.command(ReactiveCommand::SetVault {
            agent_id: "a".into(),
            entries: vec![id.clone()],
        })
        .await;
        id
    }

    /// The gate's answer to `command` from `run`, as the worker asks it.
    async fn gate(&self, run: &ReactiveRun, command: &Value) -> Option<PermissionDecision> {
        self.runtime
            .gate_browser_step_for_tests(run, &full(), &json!({ "command": command }))
            .await
            .unwrap()
    }

    /// Opens `a`'s browser and returns its state.
    async fn open(&self) -> ComputerState {
        let (status, body) = self
            .post("/api/reactive/agents/a/computer", json!({"action": "open"}))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        serde_json::from_value(body).unwrap()
    }

    async fn step(&self, command: Value) -> (StatusCode, Value) {
        self.post(
            "/api/reactive/agents/a/computer/command",
            json!({ "command": command }),
        )
        .await
    }

    /// A running run of `a`, as the worker would have claimed it.
    async fn running(&self) -> ReactiveRun {
        self.command(ReactiveCommand::Chat {
            agent_id: "a".into(),
            message: "Book the flight".into(),
            run_id: None,
        })
        .await;
        self.runtime.claim_run_for_tests().await.unwrap().unwrap()
    }

    /// Connects to the live view of `session`, with these headers.
    async fn connect(
        &self,
        session: &str,
        headers: &[(&'static str, &str)],
    ) -> Result<Socket, StatusCode> {
        let url = format!(
            "{}{}",
            self.base.replacen("http", "ws", 1),
            screencast_path(session)
        );
        let mut request = url.into_client_request().unwrap();
        for (name, value) in headers {
            request.headers_mut().insert(*name, value.parse().unwrap());
        }
        match tokio_tungstenite::connect_async(request).await {
            Ok((socket, response)) => {
                assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
                Ok(socket)
            }
            Err(tungstenite::Error::Http(response)) => Err(response.status()),
            Err(error) => panic!("the live view didn't answer: {error}"),
        }
    }
}

/// The socket's next message, within a few seconds.
async fn next(socket: &mut Socket) -> Message {
    tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .expect("a live view message")
        .expect("the live view stays open")
        .unwrap()
}

async fn next_text(socket: &mut Socket) -> ScreencastMessage {
    match next(socket).await {
        Message::Text(text) => serde_json::from_str(&text).unwrap(),
        other => panic!("expected a text message, got {other:?}"),
    }
}

/// Nothing arrives for a short while.
async fn quiet(socket: &mut Socket) {
    let message = tokio::time::timeout(Duration::from_millis(300), socket.next()).await;
    assert!(message.is_err(), "unexpected message: {message:?}");
}

fn full() -> BruhAccess {
    BruhAccess {
        level: AccessLevel::Full,
        ..BruhAccess::default()
    }
}

/// A random value no response may hold.
fn canary(label: &str) -> String {
    format!("{label}-{}", uuid::Uuid::new_v4().simple())
}

fn fill_password(login_id: &str) -> Value {
    json!({
        "action": "fill",
        "selector": "#password",
        "secret": {"login_id": login_id, "field": "password"},
    })
}

fn press() -> ComputerInput {
    ComputerInput::Mouse {
        kind: MouseKind::Down,
        x: 10.0,
        y: 10.0,
        button: MouseButton::Left,
        clicks: 1,
        modifiers: 0,
    }
}

async fn send_input(socket: &mut Socket, input: &ComputerInput) {
    let message = json!({"type": "input", "input": input});
    socket
        .send(Message::text(message.to_string()))
        .await
        .unwrap();
}

#[tokio::test]
async fn the_computer_routes_need_computer_on() {
    let fixture = Fixture::new().await;
    let (status, body) = fixture.get("/api/reactive/agents/a/computer").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], TURN_ON_IN_SETTINGS);
    let (status, body) = fixture.get("/api/reactive/agents/nobody/computer").await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::NOT_FOUND, Some(TURN_ON_IN_SETTINGS))
    );

    fixture.computer_on().await;
    let (status, body) = fixture.get("/api/reactive/agents/a/computer").await;
    assert_eq!(status, StatusCode::OK);
    let state: ComputerState = serde_json::from_value(body).unwrap();
    assert_eq!(state.session_id, None);
    assert!(fixture.open().await.session_id.is_some());

    let (status, body) = fixture
        .step(json!({"action": "evaluate", "expression": "1"}))
        .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::FORBIDDEN, Some(NOT_FOR_BRUHS))
    );
}

#[tokio::test]
async fn the_command_route_runs_only_steps_the_gate_allowed() {
    let fixture = Fixture::new().await;
    fixture.computer_on().await;
    // A step the gate never saw.
    let snapshot = json!({"action": "snapshot"});
    let (status, body) = fixture.step(snapshot.clone()).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::FORBIDDEN, Some(STEP_NOT_APPROVED))
    );

    let run = fixture.running().await;
    fixture
        .runtime
        .allow_browser_step_for_tests(&run, snapshot.clone(), None);
    // A page posting the very step it saw is refused, and uses nothing up.
    let (status, body) = fixture
        .post_with(
            "/api/reactive/agents/a/computer/command",
            json!({ "command": snapshot }),
            Some(APP_ORIGIN),
        )
        .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::FORBIDDEN, Some(STEP_FROM_A_PAGE))
    );
    let (status, body) = fixture.step(snapshot.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["value"], json!({"ok": true}));
    // Each allowed call runs once.
    let (status, body) = fixture.step(snapshot.clone()).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::FORBIDDEN, Some(STEP_NOT_APPROVED))
    );

    // The step is matched as sent: whole-number coordinates stay whole.
    let shop = "https://shop.test/cart";
    fixture.runtime.allow_browser_step_for_tests(
        &run,
        json!({"action": "navigate", "url": shop}),
        None,
    );
    let (status, body) = fixture
        .step(json!({"action": "navigate", "url": shop}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let click = json!({"action": "click_at", "x": 100, "y": 10});
    fixture
        .runtime
        .allow_browser_step_for_tests(&run, click.clone(), Some(shop.into()));
    let (status, body) = fixture.step(click.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // The page went to GitHub after the gate looked: the input is refused,
    // and its allowance is used up.
    fixture
        .runtime
        .allow_browser_step_for_tests(&run, click.clone(), Some(shop.into()));
    fixture.fake.set_url("a", "https://github.com/o/r/pull/1");
    let (status, body) = fixture.step(click.clone()).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::FORBIDDEN, Some(PAGE_CHANGED))
    );
    fixture.fake.set_url("a", shop);
    let (status, body) = fixture.step(click.clone()).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::FORBIDDEN, Some(STEP_NOT_APPROVED))
    );

    // A stopped bruh hears that it was stopped.
    fixture
        .runtime
        .allow_browser_step_for_tests(&run, snapshot.clone(), None);
    fixture
        .command(ReactiveCommand::StopBruh {
            agent_id: "a".into(),
        })
        .await;
    fixture
        .eventually(|| async {
            fixture.step(snapshot.clone()).await.1["error"].as_str() == Some(ACTION_STOPPED)
        })
        .await;
    assert_eq!(
        fixture.fake.actions("a").len(),
        3,
        "only the snapshot, the navigation and one click ran"
    );
}

#[tokio::test]
async fn foreign_pages_and_the_bruh_browser_cannot_open_a_live_view() {
    let fixture = Fixture::new().await;
    fixture.computer_on().await;
    let session = fixture.open().await.session_id.unwrap();
    assert_eq!(
        fixture
            .connect(&session, &[("origin", "http://evil.example")])
            .await
            .unwrap_err(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        fixture
            .connect(
                &session,
                &[
                    ("user-agent", "Mozilla/5.0 BruhsVerification/1"),
                    ("origin", APP_ORIGIN)
                ]
            )
            .await
            .unwrap_err(),
        StatusCode::FORBIDDEN
    );
    // Ruling BC I1: another loopback page can't watch a bruh's browser,
    // which may show a filled password.
    for origin in [APP_ORIGIN, "http://localhost:8080"] {
        assert_eq!(
            fixture
                .connect(&session, &[("origin", origin)])
                .await
                .unwrap_err(),
            StatusCode::FORBIDDEN,
            "{origin}"
        );
    }
    assert!(fixture
        .connect(&session, &[("origin", fixture.base.as_str())])
        .await
        .is_ok());
    // The desktop's native client sends no Origin.
    assert!(fixture.connect(&session, &[]).await.is_ok());
}

/// Ruling BC I1: only the owner's own app opens, takes or hands back a
/// bruh's Computer.
#[tokio::test]
async fn the_computer_answers_only_bruhs_own_pages() {
    let fixture = Fixture::new().await;
    fixture.computer_on().await;
    let path = "/api/reactive/agents/a/computer";
    for action in ["open", "take_control", "hand_back", "close"] {
        for origin in [APP_ORIGIN, "http://localhost:8080"] {
            let (status, body) = fixture
                .post_with(path, json!({ "action": action }), Some(origin))
                .await;
            assert_eq!(
                (status, body["error"].as_str()),
                (StatusCode::FORBIDDEN, Some(COMPUTER_OWNER_ONLY)),
                "{action} from {origin}"
            );
        }
    }
    let (_, state) = fixture.get(path).await;
    assert_eq!(state["session_id"], Value::Null);
    let own = fixture.base.clone();
    for origin in [None, Some(own.as_str()), Some(CONFIGURED_ORIGIN)] {
        for action in ["open", "take_control", "hand_back"] {
            let (status, body) = fixture
                .post_with(path, json!({ "action": action }), origin)
                .await;
            assert_eq!(status, StatusCode::OK, "{action} from {origin:?}: {body}");
        }
    }
}

/// A live view the service refuses opens, says why with the shared copy,
/// then closes: a browser can't read a refused upgrade's body.
#[tokio::test]
async fn a_refused_live_view_says_why_then_closes() {
    let fixture = Fixture::new().await;
    fixture.computer_on().await;
    let session = fixture.open().await.session_id.unwrap();
    let ended = |message: Option<Message>| match message {
        None | Some(Message::Close(_)) => true,
        other => panic!("expected the live view to close, got {other:?}"),
    };
    let mut unknown = fixture
        .connect("not-a-session", &[("origin", fixture.base.as_str())])
        .await
        .expect("an unknown session still opens");
    assert_eq!(
        next_text(&mut unknown).await,
        ScreencastMessage::Error {
            error: LIVE_VIEW_ENDED.into()
        }
    );
    assert!(ended(
        tokio::time::timeout(Duration::from_secs(5), unknown.next())
            .await
            .expect("the live view closes")
            .and_then(Result::ok)
    ));

    let mut watching = Vec::new();
    for _ in 0..MAX_VIEWERS {
        let mut socket = fixture.connect(&session, &[]).await.unwrap();
        assert!(matches!(
            next_text(&mut socket).await,
            ScreencastMessage::State(_)
        ));
        watching.push(socket);
    }
    let mut fifth = fixture
        .connect(&session, &[("origin", fixture.base.as_str())])
        .await
        .expect("a fifth viewer still opens");
    assert_eq!(
        next_text(&mut fifth).await,
        ScreencastMessage::Error {
            error: VIEWERS_FULL.into()
        }
    );
    assert!(ended(
        tokio::time::timeout(Duration::from_secs(5), fifth.next())
            .await
            .expect("the live view closes")
            .and_then(Result::ok)
    ));
}

#[tokio::test]
async fn the_live_view_sends_state_then_one_frame_at_a_time() {
    let fixture = Fixture::new().await;
    fixture.computer_on().await;
    let session = fixture.open().await.session_id.unwrap();
    let mut socket = fixture
        .connect(&session, &[("origin", fixture.base.as_str())])
        .await
        .unwrap();
    match next_text(&mut socket).await {
        ScreencastMessage::State(state) => {
            assert_eq!(state.session_id.as_deref(), Some(session.as_str()));
            assert_eq!(state.agent_id, "a");
        }
        other => panic!("expected the state first, got {other:?}"),
    }
    fixture.fake.publish_frame("a", vec![1]);
    assert_eq!(next(&mut socket).await, Message::binary(vec![1]));
    fixture.fake.publish_frame("a", vec![2]);
    fixture.fake.publish_frame("a", vec![3]);
    quiet(&mut socket).await;
    socket.send(Message::text(FRAME_ACK)).await.unwrap();
    assert_eq!(next(&mut socket).await, Message::binary(vec![3]));
    quiet(&mut socket).await;
}

#[tokio::test]
async fn owner_input_over_the_live_view_needs_control() {
    let fixture = Fixture::new().await;
    fixture.computer_on().await;
    let session = fixture.open().await.session_id.unwrap();
    let mut socket = fixture.connect(&session, &[]).await.unwrap();
    assert!(matches!(
        next_text(&mut socket).await,
        ScreencastMessage::State(_)
    ));

    send_input(&mut socket, &press()).await;
    assert_eq!(
        next_text(&mut socket).await,
        ScreencastMessage::Error {
            error: TAKE_CONTROL_FIRST.into()
        }
    );
    quiet(&mut socket).await;
    // A message the server can't read is answered, and the view stays.
    socket
        .send(Message::text("{\"type\":\"dance\"}"))
        .await
        .unwrap();
    assert!(matches!(
        next_text(&mut socket).await,
        ScreencastMessage::Error { .. }
    ));

    let (status, body) = fixture
        .post(
            "/api/reactive/agents/a/computer",
            json!({"action": "take_control"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    match next_text(&mut socket).await {
        ScreencastMessage::State(state) => assert_eq!(state.control, Control::Owner),
        other => panic!("expected the new state, got {other:?}"),
    }
    send_input(&mut socket, &press()).await;
    fixture
        .eventually(|| async { fixture.fake.inputs("a") == [press()] })
        .await;
}

#[tokio::test]
async fn bruh_principals_get_no_grant_from_the_verification_route() {
    let fixture = Fixture::new().await;
    let (status, body) = fixture
        .post(
            "/api/local-tools/grants",
            json!({"principal": "bruh:a", "scope": "browser"}),
        )
        .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::BAD_REQUEST, Some(TURN_ON_IN_SETTINGS))
    );
}

#[tokio::test]
async fn a_granted_fill_reaches_the_page_through_the_allowance() {
    let fixture = Fixture::new().await;
    let password = canary("pw");
    let login = fixture.granted_login(&password).await;
    fixture.computer_on().await;
    fixture.open().await;
    fixture.fake.set_url("a", SHOP);
    let run = fixture.running().await;
    let fill = fill_password(&login);
    assert_eq!(
        fixture.gate(&run, &fill).await,
        Some(PermissionDecision::Allow)
    );
    let (status, body) = fixture.step(fill.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["value"],
        json!({"filled": "password", "submitted": false})
    );
    assert!(!body.to_string().contains(&password));
    assert_eq!(
        fixture.fake.secret_fills("a"),
        [(
            "#password".to_owned(),
            SecretField::Password,
            false,
            password
        )]
    );
    // The allowance is used up.
    let (status, body) = fixture.step(fill).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::FORBIDDEN, Some(STEP_NOT_APPROVED))
    );
    assert_eq!(fixture.fake.secret_fills("a").len(), 1);
}

#[tokio::test]
async fn a_fill_after_the_page_moved_is_refused_and_types_nothing() {
    let fixture = Fixture::new().await;
    let password = canary("pw");
    let login = fixture.granted_login(&password).await;
    fixture.computer_on().await;
    fixture.open().await;
    fixture.fake.set_url("a", SHOP);
    let run = fixture.running().await;
    let fill = fill_password(&login);
    assert_eq!(
        fixture.gate(&run, &fill).await,
        Some(PermissionDecision::Allow)
    );
    fixture.fake.set_url("a", "https://evil.test/");
    let (status, body) = fixture.step(fill).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::FORBIDDEN, Some(LOGIN_WRONG_ORIGIN))
    );
    assert!(!body.to_string().contains(&password));
    assert!(fixture.fake.secret_fills("a").is_empty());
}

#[tokio::test]
async fn logins_answer_without_the_page() {
    let fixture = Fixture::new().await;
    let password = canary("pw");
    let login = fixture.granted_login(&password).await;
    fixture.computer_on().await;
    let run = fixture.running().await;
    let logins = json!({"action": "logins"});
    // Never answered without the gate.
    let (status, body) = fixture.step(logins.clone()).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::FORBIDDEN, Some(STEP_NOT_APPROVED))
    );
    assert_eq!(
        fixture.gate(&run, &logins).await,
        Some(PermissionDecision::Allow)
    );
    let (status, body) = fixture.step(logins).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["value"],
        json!({"logins": [{
            "login_id": login,
            "name": "Shop",
            "url": SHOP,
            "origin": "https://shop.test",
            "fields": ["username", "password"],
        }]})
    );
    let text = body.to_string();
    assert!(!text.contains(&password) && !text.contains("owner@shop.test"));
    assert!(
        fixture.fake.launches().is_empty(),
        "the browser stays closed"
    );
}

#[tokio::test]
async fn a_request_with_an_origin_never_fills() {
    let fixture = Fixture::new().await;
    let password = canary("pw");
    let login = fixture.granted_login(&password).await;
    fixture.computer_on().await;
    fixture.open().await;
    fixture.fake.set_url("a", SHOP);
    let run = fixture.running().await;
    let fill = fill_password(&login);
    assert_eq!(
        fixture.gate(&run, &fill).await,
        Some(PermissionDecision::Allow)
    );
    for origin in [APP_ORIGIN, fixture.base.as_str()] {
        let (status, body) = fixture
            .post_with(
                "/api/reactive/agents/a/computer/command",
                json!({ "command": fill }),
                Some(origin),
            )
            .await;
        assert_eq!(
            (status, body["error"].as_str()),
            (StatusCode::FORBIDDEN, Some(STEP_FROM_A_PAGE))
        );
    }
    assert!(fixture.fake.secret_fills("a").is_empty());
    // Nothing was used up: the helper's own request still fills.
    let (status, body) = fixture.step(fill).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(fixture.fake.secret_fills("a").len(), 1);
}

/// Ruling F26: only the desktop's native client (no `Origin`), the page
/// this server serves (its own `Host`) and the configured origins change
/// the vault or a bruh's grants. Other loopback pages may still read the
/// listing and send other commands.
#[tokio::test]
async fn vault_changes_come_only_from_bruhs_own_pages() {
    use reqwest::Method;
    let fixture = Fixture::new().await;
    let own = fixture.base.clone();
    let key = |name: &str, value: &str| {
        json!({
            "name": name,
            "scope": {"type": "global"},
            "secret": {"kind": "value", "value": value},
        })
    };
    let foreign = [APP_ORIGIN, "http://localhost:8080", "http://[::1]:5173"];
    let value = canary("key");
    for origin in foreign {
        let (status, body) = fixture
            .send(
                Method::POST,
                "/api/vault",
                Some(key("Other", &value)),
                Some(origin),
            )
            .await;
        assert_eq!(
            (status, body.as_str()),
            (
                StatusCode::FORBIDDEN,
                &*format!("{{\"error\":\"{OWNER_ONLY}\"}}")
            )
        );
    }
    let mut ids = Vec::new();
    for (name, origin) in [
        ("Native", None),
        ("Own page", Some(own.as_str())),
        ("Configured", Some(CONFIGURED_ORIGIN)),
    ] {
        let (status, body) = fixture
            .send(Method::POST, "/api/vault", Some(key(name, &value)), origin)
            .await;
        assert_eq!(status, StatusCode::OK, "{name}: {body}");
        assert!(!body.contains(&value));
        let entry: Value = serde_json::from_str(&body).unwrap();
        ids.push(entry["id"].as_str().unwrap().to_owned());
    }
    // Reading is the owner's too (ruling V Task 10 I3).
    for origin in foreign {
        let (status, body) = fixture
            .send(Method::GET, "/api/vault", None, Some(origin))
            .await;
        assert_eq!(
            (status, body.as_str()),
            (
                StatusCode::FORBIDDEN,
                &*format!("{{\"error\":\"{OWNER_ONLY}\"}}")
            )
        );
    }
    for origin in [None, Some(own.as_str()), Some(CONFIGURED_ORIGIN)] {
        let (status, body) = fixture.send(Method::GET, "/api/vault", None, origin).await;
        assert_eq!(status, StatusCode::OK, "{origin:?}: {body}");
        assert!(!body.contains(&value));
        let listing: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(listing["entries"].as_array().unwrap().len(), 3);
    }

    let path = format!("/api/vault/{}", ids[0]);
    let update = json!({"secret": {"kind": "value", "value": canary("new")}});
    for (method, path, body) in [
        (Method::PUT, path.clone(), Some(update.clone())),
        (Method::DELETE, path.clone(), None),
        (Method::POST, format!("{path}/check"), None),
    ] {
        let (status, text) = fixture
            .send(method.clone(), &path, body, Some(APP_ORIGIN))
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {text}");
        assert!(text.contains(OWNER_ONLY));
    }
    let set_vault =
        |entries: &[String]| json!({"action": "set_vault", "agent_id": "a", "entries": entries});
    let (status, text) = fixture
        .send(
            Method::POST,
            "/api/reactive/commands",
            Some(set_vault(&ids)),
            Some(APP_ORIGIN),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{text}");
    assert!(text.contains(OWNER_ONLY));
    let granted = |snapshot: &Value| snapshot["bruhs"][0]["vault"].clone();
    let (_, snapshot) = fixture.get("/api/reactive").await;
    assert_eq!(granted(&snapshot), json!([]));
    // Other commands still come from any loopback page.
    let (status, text) = fixture
        .send(
            Method::POST,
            "/api/reactive/commands",
            Some(json!({"action": "set_computer", "agent_id": "a", "on": true})),
            Some(APP_ORIGIN),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    for origin in [None, Some(own.as_str()), Some(CONFIGURED_ORIGIN)] {
        let (status, text) = fixture
            .send(
                Method::POST,
                "/api/reactive/commands",
                Some(set_vault(&ids[..1])),
                origin,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{origin:?}: {text}");
    }
    let (_, snapshot) = fixture.get("/api/reactive").await;
    assert_eq!(granted(&snapshot), json!([ids[0]]));

    for (method, path, body, expected) in [
        (Method::PUT, path.clone(), Some(update), StatusCode::OK),
        (Method::POST, format!("{path}/check"), None, StatusCode::OK),
        (Method::DELETE, path.clone(), None, StatusCode::NO_CONTENT),
    ] {
        let (status, text) = fixture.send(method.clone(), &path, body, Some(&own)).await;
        assert_eq!(status, expected, "{method} {path}: {text}");
    }
}

/// Only the owner's own app answers a bruh's ask to use a vault key; any
/// loopback page still answers its other approvals.
#[tokio::test]
async fn a_vault_key_approval_is_answered_only_from_bruhs_own_pages() {
    use reqwest::Method;
    let fixture = Fixture::new().await;
    let run = fixture.running().await;
    let use_key =
        json!({"name": "DEPLOY_KEY", "reason": "Ship.", "env_var": "K", "command": ["deploy"]});
    let key = fixture
        .runtime
        .open_approval_for_tests(&run, bruhs_models::crew::BRUH_SECRET_TOOL, &use_key)
        .await
        .unwrap();
    let edit = fixture
        .runtime
        .open_approval_for_tests(&run, "Bash", &json!({"command": "make"}))
        .await
        .unwrap();
    let decide = |id: &str| json!({"action": "decide_approval", "id": id, "approve": true});
    for origin in [APP_ORIGIN, "http://localhost:8080"] {
        let (status, text) = fixture
            .send(
                Method::POST,
                "/api/reactive/commands",
                Some(decide(&key)),
                Some(origin),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{origin}: {text}");
        // Ruling BC M6: the copy says what was refused.
        assert!(text.contains(OWNER_ONLY_CARD), "{text}");
    }
    let (status, text) = fixture
        .send(
            Method::POST,
            "/api/reactive/commands",
            Some(decide(&edit)),
            Some(APP_ORIGIN),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let own = fixture.base.clone();
    let (status, text) = fixture
        .send(
            Method::POST,
            "/api/reactive/commands",
            Some(decide(&key)),
            Some(&own),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
}

/// Ruling BC I2: deleting a bruh, which deletes its own vault entries,
/// takes the owner's own app, applied or approved as a proposal.
#[tokio::test]
async fn a_bruh_is_deleted_only_from_bruhs_own_pages() {
    use reqwest::Method;
    let fixture = Fixture::new().await;
    let commands = "/api/reactive/commands";
    let delete = json!({"action": "apply", "change": {"type": "delete_agent", "id": "a"}});
    let propose = json!({"action": "propose", "change": {"type": "delete_agent", "id": "a"}});
    // Proposing changes nothing, so any loopback page may.
    let (status, text) = fixture
        .send(Method::POST, commands, Some(propose), Some(APP_ORIGIN))
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let snapshot: Value = serde_json::from_str(&text).unwrap();
    let proposal = snapshot["proposals"][0]["id"].as_str().unwrap().to_owned();
    let decide =
        |approve: bool| json!({"action": "decide_proposal", "id": proposal, "approve": approve});
    for origin in [APP_ORIGIN, "http://localhost:8080"] {
        for body in [delete.clone(), decide(true)] {
            let (status, text) = fixture
                .send(Method::POST, commands, Some(body.clone()), Some(origin))
                .await;
            assert_eq!(
                (status, text.as_str()),
                (
                    StatusCode::FORBIDDEN,
                    &*format!("{{\"error\":\"{OWNER_ONLY_DELETE_BRUH}\"}}")
                ),
                "{body} from {origin}"
            );
        }
    }
    let (_, snapshot) = fixture.get("/api/reactive").await;
    assert_eq!(snapshot["agents"].as_array().unwrap().len(), 1);
    // The owner's own app deletes it, and rejecting a delete is anyone's.
    let own = fixture.base.clone();
    let (status, text) = fixture
        .send(Method::POST, commands, Some(decide(true)), Some(&own))
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let (_, snapshot) = fixture.get("/api/reactive").await;
    assert_eq!(snapshot["agents"], json!([]));
    let (status, text) = fixture
        .send(Method::POST, commands, Some(delete), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{text}");
}

/// Ruling BC M6: a failed `logins` says the listing failed, not a read.
#[test]
fn a_failed_logins_listing_says_so() {
    let refusal = super::logins_failed(bruhs_core::reactive::ReactiveError::Storage(
        "The vault could not be read.".into(),
    ));
    assert_eq!(refusal.to_string(), LOGINS_LIST_FAILED);
    assert!(matches!(refusal, AppError::Unavailable(_)));
}

fn unavailable(result: Result<ToolReply, AppError>) -> String {
    match result {
        Err(AppError::Unavailable(message)) => message,
        Err(other) => panic!("expected 503, got {other}"),
        Ok(reply) => panic!("expected 503, got {:?}", reply.value),
    }
}

/// A read past its budget and a fill cut off while typing each answer with
/// copy that doesn't invite a retry, within the budget (ruling F24, V Task
/// 10 I1). The budgets are short here; the route uses 32 s and 55 s.
#[tokio::test]
async fn a_fill_past_its_budget_never_asks_for_a_retry() {
    let fixture = Fixture::new().await;
    let (status, body) = fixture
        .post(
            "/api/vault",
            json!({
                "name": "Shop 1Password",
                "scope": {"type": "global"},
                "secret": {"kind": "login", "url": SHOP, "fields": {
                    "source": "one_password",
                    "username": "op://P/Shop/username",
                    "password": "op://P/Shop/password",
                }},
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let slow = body["id"].as_str().unwrap().to_owned();
    let password = canary("pw");
    let saved = fixture.granted_login(&password).await;
    fixture
        .command(ReactiveCommand::SetVault {
            agent_id: "a".into(),
            entries: vec![slow.clone(), saved.clone()],
        })
        .await;
    fixture.computer_on().await;
    fixture.open().await;
    fixture.fake.set_url("a", SHOP);
    let fill = |login: &str| {
        let action: BrowserAction = serde_json::from_value(fill_password(login)).unwrap();
        let secret = action.secret_fill().unwrap().1.clone();
        (action, secret)
    };
    let short = |read_ms, total_ms| super::FillBudget {
        read: Duration::from_millis(read_ms),
        total: Duration::from_millis(total_ms),
    };

    // 1Password is slow to answer: the read's budget ends the fill.
    let (action, secret) = fill(&slow);
    let started = std::time::Instant::now();
    let result = super::vault_fill(
        &fixture.runtime,
        &fixture.host,
        "a",
        action,
        &secret,
        Some(SHOP),
        short(200, 55_000),
    )
    .await;
    assert_eq!(unavailable(result), VAULT_READ_FAILED);
    assert!(started.elapsed() < Duration::from_secs(3));

    // The page takes the keystrokes and never answers: the fill may have
    // run, so it isn't one to retry.
    let typing = fixture.fake.hold_next_action("a");
    let (action, secret) = fill(&saved);
    let started = std::time::Instant::now();
    let result = super::vault_fill(
        &fixture.runtime,
        &fixture.host,
        "a",
        action,
        &secret,
        Some(SHOP),
        short(5_000, 500),
    )
    .await;
    assert_eq!(unavailable(result), FILL_MAY_HAVE_RUN);
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(fixture.fake.secret_fills("a").is_empty());
    drop(typing);
    let (action, secret) = fill(&saved);
    super::vault_fill(
        &fixture.runtime,
        &fixture.host,
        "a",
        action,
        &secret,
        Some(SHOP),
        short(5_000, 5_000),
    )
    .await
    .unwrap();
    assert_eq!(fixture.fake.secret_fills("a").len(), 1, "one fill typed");
}
