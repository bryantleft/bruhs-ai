use super::*;
use bruhs_models::{workspace_bridge::BridgeRequest, AgentDomain};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;

fn read(domain: AgentDomain) -> BridgeRequest {
    BridgeRequest::Read {
        domain,
        query: String::new(),
        offset: 0,
    }
}

/// The token is only ever in the private file; tests read it from there,
/// as `bruhs-mcp` does.
fn token_of(enabled: &SessionEnabled) -> String {
    let file: Value =
        serde_json::from_slice(&std::fs::read(&enabled.connection_path).unwrap()).unwrap();
    file["token"].as_str().unwrap().to_owned()
}

async fn hello(frames: &mut (impl Stream<Item = RelayFrame> + Unpin)) -> String {
    match frames.next().await {
        Some(RelayFrame::Hello { stream_id }) => stream_id,
        other => panic!("expected hello, got {other:?}"),
    }
}

#[tokio::test]
async fn a_session_binds_to_one_stream_and_its_file_holds_the_only_copy_of_the_token() {
    let relay = WorkspaceRelay::new("http://127.0.0.1:3001");
    let mut frames = Box::pin(relay.attach().unwrap());
    let stream_id = hello(&mut frames).await;
    assert_eq!(stream_id.len(), 64);
    assert!(matches!(
        relay.enable("not-a-stream"),
        Err(EnableError::StreamNotOpen)
    ));
    let enabled = relay.enable(&stream_id).unwrap();
    assert!(!enabled.replaced);
    let encoded = serde_json::to_string(&enabled).unwrap();
    let token = token_of(&enabled);
    assert!(
        !encoded.contains(&token),
        "the tab never receives the token"
    );
    let mode = std::fs::metadata(&enabled.connection_path)
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
    let file: Value =
        serde_json::from_slice(&std::fs::read(&enabled.connection_path).unwrap()).unwrap();
    assert_eq!(file["url"], "http://127.0.0.1:3001");

    let call = tokio::spawn({
        let relay = relay.clone();
        let token = token.clone();
        async move { relay.call(&token, read(AgentDomain::Tasks)).await }
    });
    let Some(RelayFrame::Request {
        session_id,
        request_id,
        request,
    }) = frames.next().await
    else {
        panic!("request frame");
    };
    assert_eq!(session_id, enabled.session_id);
    assert_eq!(request, read(AgentDomain::Tasks));
    // Only the bound stream may answer.
    assert!(!relay.reply(RelayReply {
        stream_id: "f".repeat(64),
        session_id: session_id.clone(),
        request_id: request_id.clone(),
        response: BridgeResponse::Ok(json!({"forged":true})),
    }));
    assert!(relay.reply(RelayReply {
        stream_id: stream_id.clone(),
        session_id: session_id.clone(),
        request_id: request_id.clone(),
        response: BridgeResponse::Ok(json!({"records":[]})),
    }));
    assert_eq!(
        call.await.unwrap(),
        CallOutcome::Answered(BridgeResponse::Ok(json!({"records":[]})))
    );
    // A second answer to the same request is refused.
    assert!(!relay.reply(RelayReply {
        stream_id,
        session_id,
        request_id,
        response: BridgeResponse::Ok(json!({})),
    }));
    assert_eq!(
        relay.call(&"0".repeat(64), read(AgentDomain::Tasks)).await,
        CallOutcome::Unauthorized
    );
}

#[tokio::test]
async fn closing_the_stream_revokes_the_session_and_fails_waiting_calls() {
    let relay = WorkspaceRelay::new("http://127.0.0.1:3001");
    let mut frames = Box::pin(relay.attach().unwrap());
    let stream_id = hello(&mut frames).await;
    let enabled = relay.enable(&stream_id).unwrap();
    let token = token_of(&enabled);
    let call = tokio::spawn({
        let relay = relay.clone();
        let token = token.clone();
        async move { relay.call(&token, read(AgentDomain::Chat)).await }
    });
    assert!(matches!(
        frames.next().await,
        Some(RelayFrame::Request { .. })
    ));
    // The tab closed, reloaded or was hidden.
    drop(frames);
    assert_eq!(call.await.unwrap(), CallOutcome::Disconnected);
    assert!(!relay.session_active());
    assert!(
        !std::path::Path::new(&enabled.connection_path).exists(),
        "the connection file goes with the session"
    );
    assert_eq!(
        relay.call(&token, read(AgentDomain::Chat)).await,
        CallOutcome::Unauthorized
    );
}

#[tokio::test]
async fn another_tab_replaces_the_session_and_the_first_is_told() {
    let relay = WorkspaceRelay::new("http://127.0.0.1:3001");
    let mut first = Box::pin(relay.attach().unwrap());
    let mut second = Box::pin(relay.attach().unwrap());
    let first_id = hello(&mut first).await;
    let second_id = hello(&mut second).await;
    assert_ne!(first_id, second_id);
    let old = relay.enable(&first_id).unwrap();
    let old_token = token_of(&old);
    let new = relay.enable(&second_id).unwrap();
    assert!(new.replaced);
    assert_eq!(
        first.next().await,
        Some(RelayFrame::Revoked {
            session_id: old.session_id.clone(),
            reason: RevokeReason::Replaced,
        })
    );
    assert!(!std::path::Path::new(&old.connection_path).exists());
    assert_eq!(
        relay.call(&old_token, read(AgentDomain::Tasks)).await,
        CallOutcome::Unauthorized
    );
    // Disabling needs the bound stream and the session ID.
    assert!(!relay.disable(&first_id, &new.session_id));
    assert!(!relay.disable(&second_id, &old.session_id));
    assert!(relay.disable(&second_id, &new.session_id));
    assert!(!relay.session_active());
    assert!(!std::path::Path::new(&new.connection_path).exists());
}

#[tokio::test(start_paused = true)]
async fn a_silent_tab_times_out_and_a_full_queue_is_busy() {
    let relay = WorkspaceRelay::new("http://127.0.0.1:3001");
    let mut frames = Box::pin(relay.attach().unwrap());
    let stream_id = hello(&mut frames).await;
    let token = token_of(&relay.enable(&stream_id).unwrap());
    let calls: Vec<_> = (0..MAX_PENDING_REQUESTS)
        .map(|_| {
            let relay = relay.clone();
            let token = token.clone();
            tokio::spawn(async move { relay.call(&token, read(AgentDomain::Tasks)).await })
        })
        .collect();
    for _ in 0..MAX_PENDING_REQUESTS {
        assert!(matches!(
            frames.next().await,
            Some(RelayFrame::Request { .. })
        ));
    }
    assert_eq!(
        relay.call(&token, read(AgentDomain::Tasks)).await,
        CallOutcome::Busy
    );
    for call in calls {
        assert_eq!(call.await.unwrap(), CallOutcome::TimedOut);
    }
    // Timed-out requests free their places.
    let retry = tokio::spawn({
        let relay = relay.clone();
        async move { relay.call(&token, read(AgentDomain::Tasks)).await }
    });
    assert!(matches!(
        frames.next().await,
        Some(RelayFrame::Request { .. })
    ));
    drop(frames);
    assert_eq!(retry.await.unwrap(), CallOutcome::Disconnected);
}

/// The real router, served on a loopback port so the relay's origin and the
/// connection file name the server a client can reach.
async fn serve() -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    let directory = tempfile::tempdir().unwrap();
    let config = crate::config::Config::from_settings(crate::config::Settings {
        port: Some(&port),
        web_dist: directory.path().join("missing").to_str(),
        ..Default::default()
    })
    .unwrap();
    let app = crate::router(crate::config::ServerState {
        app: Arc::new(bruhs_core::AppState::new()),
        config: Arc::new(config),
        updates: bruhs_core::updates::UpdateChecker::new(bruhs_core::updates::BuildInfo::new(
            "", "", "unused",
        )),
    });
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://127.0.0.1:{port}"), server)
}

/// Reads frames named `workspace_bridge` off an open live events stream.
struct LiveStream {
    response: reqwest::Response,
    buffer: String,
}

impl LiveStream {
    async fn open(base: &str) -> Self {
        let response = reqwest::Client::new()
            .get(format!("{base}{}", bruhs_models::api::live_events::PATH))
            .header("origin", base)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        Self {
            response,
            buffer: String::new(),
        }
    }

    async fn next_relay_frame(&mut self) -> RelayFrame {
        loop {
            while let Some(end) = self.buffer.find("\n\n") {
                let frame: String = self.buffer.drain(..end + 2).collect();
                let lines: Vec<&str> = frame.lines().collect();
                if lines.contains(&"event: workspace_bridge") {
                    let data = lines
                        .iter()
                        .find_map(|line| line.strip_prefix("data: "))
                        .unwrap();
                    return serde_json::from_str(data).unwrap();
                }
            }
            let chunk = tokio::time::timeout(Duration::from_secs(10), self.response.chunk())
                .await
                .expect("frame before timeout")
                .unwrap()
                .expect("stream open");
            self.buffer.push_str(&String::from_utf8_lossy(&chunk));
        }
    }
}

#[tokio::test]
async fn the_call_route_refuses_browsers_and_unknown_tokens() {
    let (base, server) = serve().await;
    let http = reqwest::Client::new();
    let call = format!("{base}{CALL_PATH}");
    let body = serde_json::to_vec(&read(AgentDomain::Tasks)).unwrap();
    for (name, value) in [
        ("origin", base.as_str()),
        ("sec-fetch-site", "same-origin"),
        ("sec-fetch-mode", "cors"),
    ] {
        let response = http
            .post(&call)
            .header(name, value)
            .bearer_auth("0".repeat(64))
            .body(body.clone())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{name}");
    }
    let response = http
        .post(&call)
        .bearer_auth("0".repeat(64))
        .body(body.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response.json::<BridgeResponse>().await.unwrap(),
        BridgeResponse::Error(NOT_AUTHORIZED.into())
    );
    let unauthenticated = http.post(&call).body(body).send().await.unwrap();
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);
    // Garbage without a valid token learns nothing about the decoder.
    let garbage = http
        .post(&call)
        .bearer_auth("0".repeat(64))
        .body("not json")
        .send()
        .await
        .unwrap();
    assert_eq!(garbage.status(), StatusCode::UNAUTHORIZED);
    let oversized = http
        .post(&call)
        .bearer_auth("0".repeat(64))
        .body(vec![
            b' ';
            bruhs_models::workspace_bridge::MAX_REQUEST_BYTES + 1
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
    // The tab's routes keep the server's browser origin policy.
    let foreign = http
        .post(format!("{base}{SESSION_PATH}"))
        .header("origin", "https://example.com")
        .json(&json!({"stream_id":"x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(foreign.status(), StatusCode::FORBIDDEN);
    server.abort();
}

#[tokio::test]
async fn a_client_reaches_the_tab_through_the_real_server_and_loses_it_on_close() {
    let (base, server) = serve().await;
    let http = reqwest::Client::new();
    let mut live = LiveStream::open(&base).await;
    let RelayFrame::Hello { stream_id } = live.next_relay_frame().await else {
        panic!("hello first");
    };
    let enabled: SessionEnabled = http
        .post(format!("{base}{SESSION_PATH}"))
        .header("origin", &base)
        .json(&EnableSession {
            stream_id: stream_id.clone(),
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    // bruhs-mcp's own transport, reading the copied path.
    let client = bruhs_core::workspace_bridge::BridgeClient::from_file(std::path::Path::new(
        &enabled.connection_path,
    ))
    .unwrap();
    let call = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .call(BridgeRequest::ReadPerson {
                    person_id: "p1".into(),
                    section: Default::default(),
                    offset: 0,
                })
                .await
        }
    });
    let RelayFrame::Request {
        session_id,
        request_id,
        request,
    } = live.next_relay_frame().await
    else {
        panic!("request frame");
    };
    assert!(matches!(request, BridgeRequest::ReadPerson { person_id, .. } if person_id == "p1"));
    let answered = http
        .post(format!("{base}{REPLY_PATH}"))
        .header("origin", &base)
        .json(&RelayReply {
            stream_id: stream_id.clone(),
            session_id: session_id.clone(),
            request_id,
            response: BridgeResponse::Error(
                bruhs_models::workspace_bridge::READ_NOT_GRANTED.into(),
            ),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(answered.status(), StatusCode::OK);
    assert_eq!(
        call.await.unwrap().unwrap(),
        BridgeResponse::Error(bruhs_models::workspace_bridge::READ_NOT_GRANTED.into())
    );
    let late = http
        .post(format!("{base}{REPLY_PATH}"))
        .header("origin", &base)
        .json(&RelayReply {
            stream_id: stream_id.clone(),
            session_id,
            request_id: "already-answered".into(),
            response: BridgeResponse::Ok(json!({})),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(late.status(), StatusCode::NOT_FOUND);
    // Closing the tab's stream revokes the session and its file.
    drop(live);
    let path = std::path::PathBuf::from(&enabled.connection_path);
    for _ in 0..100 {
        if !path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!path.exists(), "the closed stream revoked the session");
    assert_eq!(
        client.call(read(AgentDomain::Tasks)).await.unwrap(),
        BridgeResponse::Error(NOT_AUTHORIZED.into())
    );
    server.abort();
}
