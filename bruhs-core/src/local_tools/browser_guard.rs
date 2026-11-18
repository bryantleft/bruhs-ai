//! A separate browser-level CDP connection protects all execution targets.
//!
//! Page-only headers do not cover popups, out-of-process frames or workers.
//! Auto-attachment pauses each target before script execution, installs the
//! marker used by the host API guard, recursively protects descendants, then
//! resumes it. The tool API exposes JavaScript, never raw browser-protocol calls.
//! A bruh's browser also has downloads denied here, on the browser-level
//! session, before any target runs: a page can end its own session (a popup,
//! then `window.close()`), but not this one, whose loss kills Chrome.

use std::{collections::HashMap, sync::Arc};

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::{
    net::TcpStream,
    process::Child,
    sync::{oneshot, watch, Mutex},
    time::Instant,
};
use tokio_tungstenite::{
    tungstenite::{protocol::WebSocketConfig, Message},
    MaybeTlsStream, WebSocketStream,
};

use super::{ProcessPolicy, ToolError};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub(super) struct BrowserGuard {
    task: tokio::task::JoinHandle<()>,
    failure: watch::Receiver<Option<String>>,
    child: Arc<Mutex<Child>>,
    /// Whether losing the guard kills Chrome. Only a graceful close clears it.
    armed: bool,
}

impl Drop for BrowserGuard {
    fn drop(&mut self) {
        if self.armed {
            if let Ok(mut child) = self.child.try_lock() {
                let _ = child.start_kill();
            }
        }
        self.task.abort();
    }
}

impl BrowserGuard {
    pub async fn start(
        endpoint: &str,
        initial_target: &str,
        child: Arc<Mutex<Child>>,
        policy: ProcessPolicy,
        deny_downloads: bool,
    ) -> Result<Self, ToolError> {
        let config = WebSocketConfig::default()
            .max_message_size(Some(policy.max_output_bytes))
            .max_frame_size(Some(policy.max_output_bytes));
        let (socket, _) =
            tokio_tungstenite::connect_async_with_config(endpoint, Some(config), false).await?;
        let (ready_tx, ready_rx) = oneshot::channel();
        let (failure_tx, failure) = watch::channel(None);
        let initial_target = initial_target.to_owned();
        let protected_child = child.clone();
        let task = tokio::spawn(async move {
            let result = protect(socket, &initial_target, policy, deny_downloads, ready_tx).await;
            let reason = match result {
                Ok(()) => "Chrome closed its browser protection connection.".into(),
                Err(error) => format!("Browser protection failed: {error}"),
            };
            failure_tx.send_replace(Some(reason));
            // Losing this connection can release paused targets. Terminate the
            // entire isolated browser, even if no agent command is in flight.
            let _ = protected_child.lock().await.kill().await;
        });
        let guard = Self {
            task,
            failure,
            child: child.clone(),
            armed: true,
        };
        if ready_rx.await.is_err() {
            let _ = child.lock().await.kill().await;
            return Err(ToolError::Unavailable(
                "Chrome could not protect its initial page and workers.".into(),
            ));
        }
        guard.check()?;
        Ok(guard)
    }

    /// Stop protecting without killing Chrome, for a graceful close: the
    /// guard would otherwise kill it as soon as `Browser.close` drops the
    /// guard's connection, before Chrome writes its profile. Returns once the
    /// guard task can no longer kill.
    pub async fn disarm(&mut self) {
        self.armed = false;
        self.task.abort();
        let _ = (&mut self.task).await;
    }

    pub fn check(&self) -> Result<(), ToolError> {
        if let Some(reason) = self.failure.borrow().as_ref() {
            return Err(ToolError::Unavailable(reason.clone()));
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Step {
    /// `Browser.setDownloadBehavior` deny, before `Root` attaches anything.
    Downloads,
    Root,
    Network,
    Marker,
    Descendants,
    Resume,
    WorkerNetwork,
    WorkerMarker,
    WorkerResume,
}

struct Pending {
    session: Option<String>,
    target: Option<String>,
    step: Step,
    deadline: Instant,
}

struct Protection {
    socket: Socket,
    sequence: u64,
    pending: HashMap<u64, Pending>,
    policy: ProcessPolicy,
}

impl Protection {
    async fn send(
        &mut self,
        session: Option<String>,
        target: Option<String>,
        step: Step,
    ) -> Result<(), ToolError> {
        let (method, params) = match step {
            Step::Downloads => ("Browser.setDownloadBehavior", json!({"behavior":"deny"})),
            Step::Root | Step::Descendants => (
                "Target.setAutoAttach",
                json!({
                    "autoAttach":true,"waitForDebuggerOnStart":true,"flatten":true,
                    "filter":[{"type":"browser","exclude":true},{"type":"tab","exclude":true},{"type":"browser_ui","exclude":true},{}]
                }),
            ),
            Step::Network | Step::WorkerNetwork => ("Network.enable", json!({})),
            Step::Marker | Step::WorkerMarker => (
                "Network.setExtraHTTPHeaders",
                json!({"headers":{"x-bruhs-verification-browser":"1"}}),
            ),
            Step::Resume | Step::WorkerResume => ("Runtime.runIfWaitingForDebugger", json!({})),
        };
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| ToolError::Invalid("Browser protection sequence exhausted.".into()))?;
        let id = self.sequence;
        let mut message = json!({"id":id,"method":method,"params":params});
        if let Some(session) = &session {
            message["sessionId"] = session.clone().into();
        }
        self.socket
            .send(Message::Text(message.to_string().into()))
            .await?;
        self.pending.insert(
            id,
            Pending {
                session,
                target,
                step,
                deadline: Instant::now() + self.policy.timeout,
            },
        );
        Ok(())
    }
}

async fn protect(
    socket: Socket,
    initial_target: &str,
    policy: ProcessPolicy,
    deny_downloads: bool,
    ready_tx: oneshot::Sender<()>,
) -> Result<(), ToolError> {
    let mut state = Protection {
        socket,
        sequence: 0,
        pending: HashMap::new(),
        policy,
    };
    let mut ready_tx = Some(ready_tx);
    let mut root_ready = false;
    let mut initial_ready = false;
    // Nothing is attached, so nothing resumes, until the denial is in place.
    let first = if deny_downloads {
        Step::Downloads
    } else {
        Step::Root
    };
    state.send(None, None, first).await?;
    loop {
        let deadline = state.pending.values().map(|request| request.deadline).min();
        let frame = tokio::select! {
            frame = state.socket.next() => frame,
            _ = tokio::time::sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => {
                return Err(ToolError::Unavailable("Chrome did not acknowledge target protection before the deadline.".into()));
            }
        };
        let Some(frame) = frame else {
            return Ok(());
        };
        let Message::Text(text) = frame? else {
            continue;
        };
        let value: Value = serde_json::from_str(&text)?;
        if let Some(id) = value["id"].as_u64() {
            let Some(pending) = state.pending.remove(&id) else {
                continue;
            };
            if let Some(error) = value.get("error") {
                return Err(ToolError::Unavailable(format!(
                    "Chrome rejected target protection: {error}"
                )));
            }
            let next = match pending.step {
                Step::Downloads => Some(Step::Root),
                Step::Root => {
                    root_ready = true;
                    None
                }
                Step::Network => Some(Step::Marker),
                Step::Marker => Some(Step::Descendants),
                Step::Descendants => Some(Step::Resume),
                Step::Resume => {
                    if pending.target.as_deref() == Some(initial_target) {
                        initial_ready = true;
                    }
                    None
                }
                Step::WorkerNetwork | Step::WorkerMarker | Step::WorkerResume => None,
            };
            if let Some(next) = next {
                state.send(pending.session, pending.target, next).await?;
            }
            if root_ready && initial_ready {
                if let Some(ready) = ready_tx.take() {
                    let _ = ready.send(());
                }
            }
        } else if value["method"] == "Target.attachedToTarget" {
            let params = &value["params"];
            let session = params["sessionId"].as_str().ok_or_else(|| {
                ToolError::Invalid("Chrome attached a target without a session.".into())
            })?;
            let target = params["targetInfo"]["targetId"].as_str().ok_or_else(|| {
                ToolError::Invalid("Chrome attached a target without an identity.".into())
            })?;
            if params["targetInfo"]["type"] == "service_worker" {
                // Chromium acknowledges service-worker network setup only
                // after the worker resumes. Queue the network configuration
                // ahead of resume on the same ordered connection, then check
                // every acknowledgement. Service workers cannot spawn child
                // workers; recursively auto-attaching here reattaches itself.
                // https://github.com/puppeteer/puppeteer/blob/main/packages/puppeteer-core/src/cdp/TargetManager.ts
                for step in [Step::WorkerNetwork, Step::WorkerMarker, Step::WorkerResume] {
                    state
                        .send(Some(session.into()), Some(target.into()), step)
                        .await?;
                }
            } else {
                state
                    .send(Some(session.into()), Some(target.into()), Step::Network)
                    .await?;
            }
        } else if value["method"] == "Target.detachedFromTarget" {
            // A popup can close during setup. Its pending acknowledgements no
            // longer matter; it has no running target to release or protect.
            let session = value["params"]["sessionId"].as_str();
            state
                .pending
                .retain(|_, request| request.session.as_deref() != session);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn read(socket: &mut WebSocketStream<tokio::net::TcpStream>) -> Value {
        serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap()
    }

    async fn write(socket: &mut WebSocketStream<tokio::net::TcpStream>, value: Value) {
        socket
            .send(Message::Text(value.to_string().into()))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn protects_descendants_before_resume_and_kills_browser_on_disconnect() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (configured_tx, configured_rx) = oneshot::channel();
        let (close_tx, close_rx) = oneshot::channel();
        let fixture = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let root = read(&mut socket).await;
            assert_eq!(root["method"], "Target.setAutoAttach");
            assert_eq!(root["params"]["waitForDebuggerOnStart"], true);
            write(&mut socket, json!({"method":"Target.attachedToTarget","params":{"sessionId":"page-session","targetInfo":{"targetId":"initial","type":"page"},"waitingForDebugger":false}})).await;
            write(&mut socket, json!({"id":root["id"],"result":{}})).await;
            write(&mut socket, json!({"method":"Target.attachedToTarget","params":{"sessionId":"service-session","targetInfo":{"targetId":"service","type":"service_worker"},"waitingForDebugger":true}})).await;
            let mut commands: HashMap<String, Vec<String>> = HashMap::new();
            let mut deferred = Vec::new();
            for _ in 0..11 {
                let request = read(&mut socket).await;
                let session = request["sessionId"].as_str().unwrap();
                let method = request["method"].as_str().unwrap();
                commands
                    .entry(session.into())
                    .or_default()
                    .push(method.into());
                if method == "Network.setExtraHTTPHeaders" {
                    assert_eq!(
                        request["params"]["headers"]["x-bruhs-verification-browser"],
                        "1"
                    );
                }
                if method == "Target.setAutoAttach" {
                    assert_eq!(request["params"]["waitForDebuggerOnStart"], true);
                    if session == "page-session" {
                        write(&mut socket, json!({"sessionId":"page-session","method":"Target.attachedToTarget","params":{"sessionId":"worker-session","targetInfo":{"targetId":"child","type":"worker"},"waitingForDebugger":true}})).await;
                    }
                }
                if session == "service-session" {
                    if method != "Runtime.runIfWaitingForDebugger" {
                        deferred.push(request);
                        continue;
                    }
                    for request in deferred.drain(..) {
                        write(
                            &mut socket,
                            json!({"id":request["id"],"sessionId":"service-session","result":{}}),
                        )
                        .await;
                    }
                }
                write(
                    &mut socket,
                    json!({"id":request["id"],"sessionId":request["sessionId"],"result":{}}),
                )
                .await;
            }
            for session in ["page-session", "worker-session"] {
                assert_eq!(
                    commands[session],
                    [
                        "Network.enable",
                        "Network.setExtraHTTPHeaders",
                        "Target.setAutoAttach",
                        "Runtime.runIfWaitingForDebugger"
                    ]
                );
            }
            assert_eq!(
                commands["service-session"],
                [
                    "Network.enable",
                    "Network.setExtraHTTPHeaders",
                    "Runtime.runIfWaitingForDebugger"
                ]
            );
            configured_tx.send(()).unwrap();
            close_rx.await.unwrap();
            socket.close(None).await.unwrap();
        });
        let child = Arc::new(Mutex::new(
            tokio::process::Command::new("/bin/sleep")
                .arg("60")
                .kill_on_drop(true)
                .spawn()
                .unwrap(),
        ));
        let mut guard = BrowserGuard::start(
            &format!("ws://{address}"),
            "initial",
            child.clone(),
            ProcessPolicy::default(),
            false,
        )
        .await
        .unwrap();
        configured_rx.await.unwrap();
        guard.check().unwrap();
        assert!(child.lock().await.try_wait().unwrap().is_none());
        close_tx.send(()).unwrap();
        guard.failure.changed().await.unwrap();
        assert!(guard.check().is_err());
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while child.lock().await.try_wait().unwrap().is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        fixture.await.unwrap();
    }

    #[tokio::test]
    async fn rejected_protection_never_resumes_a_target() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let fixture = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let root = read(&mut socket).await;
            write(&mut socket, json!({"method":"Target.attachedToTarget","params":{"sessionId":"page-session","targetInfo":{"targetId":"initial","type":"page"},"waitingForDebugger":false}})).await;
            write(&mut socket, json!({"id":root["id"],"result":{}})).await;
            let request = read(&mut socket).await;
            assert_eq!(request["method"], "Network.enable");
            write(&mut socket, json!({"id":request["id"],"sessionId":"page-session","error":{"code":-32601,"message":"Unsupported domain"}})).await;
            while let Some(Ok(Message::Text(message))) = socket.next().await {
                let value: Value = serde_json::from_str(&message).unwrap();
                assert_ne!(value["method"], "Runtime.runIfWaitingForDebugger");
            }
        });
        let child = Arc::new(Mutex::new(
            tokio::process::Command::new("/bin/sleep")
                .arg("60")
                .kill_on_drop(true)
                .spawn()
                .unwrap(),
        ));
        assert!(BrowserGuard::start(
            &format!("ws://{address}"),
            "initial",
            child.clone(),
            ProcessPolicy::default(),
            false
        )
        .await
        .is_err());
        assert!(child.lock().await.try_wait().unwrap().is_some());
        fixture.await.unwrap();
    }

    #[tokio::test]
    async fn a_bruh_browser_denies_downloads_before_any_target_resumes() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let fixture = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let mut methods = Vec::new();
            loop {
                let request = read(&mut socket).await;
                let method = request["method"].as_str().unwrap().to_owned();
                if method == "Browser.setDownloadBehavior" {
                    assert!(
                        request.get("sessionId").is_none(),
                        "the browser-level session"
                    );
                    assert_eq!(request["params"], json!({"behavior":"deny"}));
                }
                if method == "Target.setAutoAttach" && request.get("sessionId").is_none() {
                    write(&mut socket, json!({"method":"Target.attachedToTarget","params":{"sessionId":"page-session","targetInfo":{"targetId":"initial","type":"page"},"waitingForDebugger":true}})).await;
                }
                write(
                    &mut socket,
                    json!({"id":request["id"],"sessionId":request["sessionId"],"result":{}}),
                )
                .await;
                methods.push(method.clone());
                if method == "Runtime.runIfWaitingForDebugger" {
                    break;
                }
            }
            assert_eq!(methods[0], "Browser.setDownloadBehavior");
            assert_eq!(methods[1], "Target.setAutoAttach");
            while socket.next().await.is_some() {}
        });
        let child = Arc::new(Mutex::new(
            tokio::process::Command::new("/bin/sleep")
                .arg("60")
                .kill_on_drop(true)
                .spawn()
                .unwrap(),
        ));
        let mut guard = BrowserGuard::start(
            &format!("ws://{address}"),
            "initial",
            child.clone(),
            ProcessPolicy::default(),
            true,
        )
        .await
        .unwrap();
        guard.check().unwrap();
        guard.disarm().await;
        drop(guard);
        child.lock().await.kill().await.unwrap();
        fixture.await.unwrap();
    }
}
