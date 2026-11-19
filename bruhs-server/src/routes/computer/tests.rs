use std::{sync::Arc, time::Duration};

use axum::{http::StatusCode, Extension};
use bruhs_core::local_tools::{FakePages, LocalToolsHost};
use bruhs_core::reactive::ReactiveRuntime;
use bruhs_models::crew::{
    AccessLevel, BruhAccess, BruhLook, ACTION_STOPPED, LIVE_VIEW_ENDED, MAX_VIEWERS, NOT_FOR_BRUHS,
    PAGE_CHANGED, STEP_FROM_A_PAGE, STEP_NOT_APPROVED, TAKE_CONTROL_FIRST, TURN_ON_IN_SETTINGS,
    VIEWERS_FULL,
};
use bruhs_models::local_tools::{
    screencast_path, ComputerInput, ComputerState, Control, MouseButton, MouseKind,
    ScreencastMessage, FRAME_ACK,
};
use bruhs_models::reactive::{ReactiveAgent, ReactiveChange, ReactiveCommand, ReactiveRun};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest, Message};

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

const APP_ORIGIN: &str = "http://127.0.0.1:3001";

struct Fixture {
    runtime: ReactiveRuntime,
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
        let config = crate::config::Config::from_settings(crate::config::Settings {
            port: Some("0"),
            web_dist: directory.path().join("missing").to_str(),
            ..Default::default()
        })
        .unwrap();
        let app = crate::router(crate::config::ServerState {
            app: Arc::new(
                bruhs_core::AppState::builder()
                    .secret_store(bruhs_core::secrets::SecretStore::in_memory())
                    .connections(bruhs_core::connections::Connections::in_memory())
                    .build(),
            ),
            config: Arc::new(config),
            updates: bruhs_core::updates::UpdateChecker::new(bruhs_core::updates::BuildInfo::new(
                "", "", "unused",
            )),
        })
        .layer(Extension(runtime.clone()))
        .layer(Extension(host));
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
    assert!(fixture
        .connect(&session, &[("origin", APP_ORIGIN)])
        .await
        .is_ok());
    // The desktop's native client sends no Origin.
    assert!(fixture.connect(&session, &[]).await.is_ok());
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
        .connect("not-a-session", &[("origin", APP_ORIGIN)])
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
        .connect(&session, &[("origin", APP_ORIGIN)])
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
        .connect(&session, &[("origin", APP_ORIGIN)])
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
