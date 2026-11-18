use std::{collections::VecDeque, process::Stdio, sync::Arc};

use base64::Engine;
use bruhs_models::local_tools::*;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::TcpStream,
    process::{Child, Command},
    sync::Mutex,
};
use tokio_tungstenite::{
    tungstenite::{protocol::WebSocketConfig, Message},
    MaybeTlsStream, WebSocketStream,
};

use super::{
    process::{self, ProcessPolicy},
    ToolError,
};

#[path = "browser_guard.rs"]
mod browser_guard;
use browser_guard::BrowserGuard;

pub(super) struct Browser {
    pub info: BrowserSession,
    child: Arc<Mutex<Child>>,
    _profile: tempfile::TempDir,
    logs: tokio::task::JoinHandle<()>,
    guard: BrowserGuard,
    cdp: Cdp,
}

impl Drop for Browser {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.try_lock() {
            let _ = child.start_kill();
        }
        self.logs.abort();
    }
}

impl Browser {
    pub async fn start(
        client: &reqwest::Client,
        request: BrowserStart,
        policy: ProcessPolicy,
    ) -> Result<Self, ToolError> {
        let executable = browser_executable()?;
        let user_agent = browser_user_agent(&executable, policy).await?;
        let profile = tempfile::Builder::new()
            .prefix("bruhs-browser-")
            .tempdir()?;
        let mut child = Command::new(executable)
            .arg(format!("--user-data-dir={}", profile.path().display()))
            .arg(format!("--user-agent={user_agent}"))
            .args([
                "--remote-debugging-port=0",
                "--remote-debugging-address=127.0.0.1",
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-extensions",
                "about:blank",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let stderr = child.stderr.take().ok_or_else(|| {
            ToolError::Unavailable("Chrome did not provide its debugging endpoint.".into())
        })?;
        let mut lines = BufReader::new(stderr).lines();
        let endpoint = tokio::time::timeout(policy.timeout, async {
            while let Some(line) = lines.next_line().await? {
                if let Some(endpoint) = line.strip_prefix("DevTools listening on ") { return Ok::<_, ToolError>(endpoint.to_string()); }
            }
            Err(ToolError::Unavailable("Chrome exited before opening its debugging endpoint. Check that a graphical desktop is available.".into()))
        }).await.map_err(|_| ToolError::Unavailable("Chrome did not start before the local-tool deadline. Close stalled Chrome processes and retry.".into()))??;
        let endpoint =
            url::Url::parse(&endpoint).map_err(|error| ToolError::Invalid(error.to_string()))?;
        if endpoint.scheme() != "ws" || endpoint.host_str() != Some("127.0.0.1") {
            return Err(ToolError::Invalid(
                "Chrome reported a non-loopback debugging endpoint.".into(),
            ));
        }
        let port = endpoint
            .port()
            .ok_or_else(|| ToolError::Invalid("Chrome reported no debugging port.".into()))?;
        let logs = tokio::spawn(async move {
            // Discard Chrome diagnostics after startup without retaining browsing data.
            while let Ok(Some(_)) = lines.next_line().await {}
        });
        let child = Arc::new(Mutex::new(child));
        let result = async {
            let mut response = client
                .get(format!("http://127.0.0.1:{port}/json/list"))
                .send()
                .await?
                .error_for_status()?;
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                if body.len().saturating_add(chunk.len()) > policy.max_output_bytes {
                    return Err(ToolError::Unavailable(
                        "Chrome's target list exceeded the capture budget.".into(),
                    ));
                }
                body.extend_from_slice(&chunk);
            }
            let targets: Vec<Value> = serde_json::from_slice(&body)?;
            let target = targets
                .iter()
                .find(|value| value["type"] == "page")
                .ok_or_else(|| ToolError::Unavailable("Chrome opened no page target.".into()))?;
            let socket = target["webSocketDebuggerUrl"].as_str().ok_or_else(|| {
                ToolError::Unavailable("Chrome's page has no debugging connection.".into())
            })?;
            let socket_url =
                url::Url::parse(socket).map_err(|error| ToolError::Invalid(error.to_string()))?;
            if socket_url.host_str() != Some("127.0.0.1")
                || socket_url.port() != Some(port)
                || socket_url.scheme() != "ws"
            {
                return Err(ToolError::Invalid(
                    "Chrome's page target escaped its loopback endpoint.".into(),
                ));
            }
            let target_id = target["id"].as_str().ok_or_else(|| {
                ToolError::Unavailable("Chrome's page has no target identity.".into())
            })?;
            // Only the initial about:blank page has run so far. Pause and mark
            // every page/worker before any untrusted navigation or script runs.
            let guard =
                BrowserGuard::start(endpoint.as_str(), target_id, child.clone(), policy).await?;
            let mut cdp = Cdp::connect(socket, policy).await?;
            cdp.call("Page.enable", json!({})).await?;
            cdp.call("Page.setLifecycleEventsEnabled", json!({"enabled":true}))
                .await?;
            cdp.call("Runtime.enable", json!({})).await?;
            cdp.call("Network.enable", json!({})).await?;
            cdp.call(
                "Network.setExtraHTTPHeaders",
                json!({"headers":{"x-bruhs-verification-browser":"1"}}),
            )
            .await?;
            cdp.call("Log.enable", json!({})).await?;
            cdp.navigate(&request.url).await?;
            guard.check()?;
            Ok::<_, ToolError>((cdp, guard))
        };
        let (cdp, guard) = match tokio::time::timeout(policy.timeout, result).await {
            Ok(Ok(ready)) => ready,
            result => {
                logs.abort();
                let _ = child.lock().await.kill().await;
                return Err(match result {
                    Ok(Err(error)) => error,
                    _ => ToolError::Unavailable(
                        "Chrome's debugging connection did not become ready.".into(),
                    ),
                });
            }
        };
        Ok(Self {
            info: BrowserSession {
                id: uuid::Uuid::new_v4().to_string(),
                grant_id: request.grant_id,
                url: request.url,
            },
            child,
            _profile: profile,
            logs,
            guard,
            cdp,
        })
    }

    pub async fn stop(&mut self) -> Result<(), ToolError> {
        let mut child = self.child.lock().await;
        if child.try_wait()?.is_none() {
            child.kill().await?;
        }
        drop(child);
        self.logs.abort();
        Ok(())
    }

    pub async fn command(
        &mut self,
        action: BrowserAction,
        policy: ProcessPolicy,
    ) -> Result<ToolReply, ToolError> {
        self.guard.check()?;
        let operation = async {
            let value = match action {
                BrowserAction::Navigate { url } => {
                    validate_browser_url(&url).map_err(|error| ToolError::Invalid(error.into()))?;
                    let result = self.cdp.navigate(&url).await?;
                    self.info.url = url;
                    result
                }
                BrowserAction::Snapshot => self.evaluate(r#"({url:location.href,title:document.title,text:document.body?.innerText,controls:[...document.querySelectorAll('a,button,input,textarea,select,[role=button]')].map(e=>({tag:e.tagName,text:e.innerText||e.getAttribute('aria-label')||'',id:e.id,name:e.name,type:e.type,href:e.href})),readyState:document.readyState})"#).await?,
                BrowserAction::Evaluate { expression } => self.evaluate(&expression).await?,
                BrowserAction::Click { selector } => {
                    let selector = serde_json::to_string(&selector)?;
                    self.evaluate(&format!(r#"(()=>{{const es=document.querySelectorAll({selector});if(es.length!==1)throw new Error('Selector must match exactly one element; found '+es.length);const e=es[0];if(e.disabled)throw new Error('Element is disabled');e.scrollIntoView({{block:'center'}});e.click();return {{clicked:true}}}})()"#)).await?
                }
                BrowserAction::Fill { selector, text } => {
                    let selector = serde_json::to_string(&selector)?;
                    let text = serde_json::to_string(&text)?;
                    self.evaluate(&format!(r#"(()=>{{const es=document.querySelectorAll({selector});if(es.length!==1)throw new Error('Selector must match exactly one element; found '+es.length);const e=es[0];if(e.disabled||e.readOnly)throw new Error('Element is not editable');const p=e instanceof HTMLTextAreaElement?HTMLTextAreaElement.prototype:e instanceof HTMLInputElement?HTMLInputElement.prototype:null;if(!p)throw new Error('Choose an input or textarea');Object.getOwnPropertyDescriptor(p,'value').set.call(e,{text});e.dispatchEvent(new Event('input',{{bubbles:true}}));e.dispatchEvent(new Event('change',{{bubbles:true}}));return {{filled:true}}}})()"#)).await?
                }
                BrowserAction::Screenshot => {
                    let value = self.cdp.call("Page.captureScreenshot", json!({"format":"png","captureBeyondViewport":false})).await?;
                    let data = value["data"].as_str().ok_or_else(|| ToolError::Invalid("Chrome returned no screenshot data.".into()))?;
                    let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(|error| ToolError::Invalid(format!("Chrome returned an invalid screenshot: {error}")))?;
                    return super::screenshot(bytes).await;
                }
                BrowserAction::Console => {
                    self.cdp.call("Runtime.evaluate", json!({"expression":"void 0"})).await?;
                    json!({"events":self.cdp.console})
                }
                BrowserAction::Network => {
                    self.cdp.call("Runtime.evaluate", json!({"expression":"void 0"})).await?;
                    json!({"events":self.cdp.network})
                }
                BrowserAction::ClickAt { .. }
                | BrowserAction::Type { .. }
                | BrowserAction::Key { .. }
                | BrowserAction::Scroll { .. } => {
                    return Err(ToolError::Invalid(
                        "This browser action is not available yet.".into(),
                    ))
                }
            };
            Ok(ToolReply { value, image: None })
        };
        let reply = tokio::time::timeout(policy.timeout, operation)
            .await
            .map_err(|_| {
                ToolError::Unavailable(
                    "Browser action timed out. Check the page and retry or revoke browser access."
                        .into(),
                )
            })?;
        self.guard.check()?;
        reply
    }

    async fn evaluate(&mut self, expression: &str) -> Result<Value, ToolError> {
        let result = self
            .cdp
            .call(
                "Runtime.evaluate",
                json!({"expression":expression,"returnByValue":true,"awaitPromise":true,"userGesture":true}),
            )
            .await?;
        if let Some(exception) = result.get("exceptionDetails") {
            return Err(ToolError::Invalid(format!(
                "Browser evaluation failed: {exception}"
            )));
        }
        Ok(result["result"]["value"].clone())
    }
}

pub(super) struct Cdp {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    sequence: u64,
    console: VecDeque<Value>,
    network: VecDeque<Value>,
    event_bytes: usize,
    budget: usize,
    loaded: Option<String>,
}

impl Cdp {
    pub async fn connect(endpoint: &str, policy: ProcessPolicy) -> Result<Self, ToolError> {
        let config = WebSocketConfig::default()
            .max_message_size(Some(policy.max_output_bytes))
            .max_frame_size(Some(policy.max_output_bytes));
        let (socket, _) =
            tokio_tungstenite::connect_async_with_config(endpoint, Some(config), false).await?;
        Ok(Self {
            socket,
            sequence: 0,
            console: VecDeque::new(),
            network: VecDeque::new(),
            event_bytes: 0,
            budget: policy.max_output_bytes,
            loaded: None,
        })
    }

    pub async fn navigate(&mut self, url: &str) -> Result<Value, ToolError> {
        let result = self.call("Page.navigate", json!({"url":url})).await?;
        if let Some(error) = result["errorText"].as_str() {
            return Err(ToolError::Unavailable(format!(
                "Browser navigation failed: {error}"
            )));
        }
        if let Some(loader) = result["loaderId"].as_str() {
            while self.loaded.as_deref() != Some(loader) {
                let frame = self.socket.next().await.ok_or_else(|| {
                    ToolError::Unavailable("Chrome closed while loading the page.".into())
                })??;
                if let Message::Text(text) = frame {
                    self.observe(serde_json::from_str(&text)?, text.len());
                }
            }
        }
        Ok(result)
    }

    pub async fn call(&mut self, method: &str, params: Value) -> Result<Value, ToolError> {
        self.sequence = self.sequence.checked_add(1).ok_or_else(|| {
            ToolError::Invalid("Browser command sequence exhausted. Start a new browser.".into())
        })?;
        let id = self.sequence;
        self.socket
            .send(Message::Text(
                json!({"id":id,"method":method,"params":params})
                    .to_string()
                    .into(),
            ))
            .await?;
        while let Some(frame) = self.socket.next().await {
            let frame = frame?;
            let Message::Text(text) = frame else {
                continue;
            };
            let value: Value = serde_json::from_str(&text)?;
            if value["id"] == id {
                if let Some(error) = value.get("error") {
                    return Err(ToolError::Invalid(format!(
                        "Chrome rejected {method}: {error}"
                    )));
                }
                return Ok(value["result"].clone());
            }
            self.observe(value, text.len());
        }
        Err(ToolError::Unavailable(
            "Chrome closed the debugging connection. Start another browser.".into(),
        ))
    }

    fn observe(&mut self, value: Value, bytes: usize) {
        if let Some(method) = value["method"].as_str() {
            if method == "Page.lifecycleEvent" && value["params"]["name"] == "load" {
                self.loaded = value["params"]["loaderId"].as_str().map(str::to_owned);
            }
            let network = method.starts_with("Network.");
            if !network
                && !method.starts_with("Log.")
                && method != "Runtime.consoleAPICalled"
                && method != "Runtime.exceptionThrown"
            {
                return;
            }
            if self.event_bytes.saturating_add(bytes) > self.budget {
                self.console.clear();
                self.network.clear();
                self.event_bytes = 0;
                // Retention resets are explicit, never an apparently complete empty log.
                let marker = json!({"method":"bruhs.logRetentionReset","reason":"Local browser event capture budget reached"});
                self.event_bytes = marker.to_string().len().saturating_mul(2);
                self.console.push_back(marker.clone());
                self.network.push_back(marker);
            }
            self.event_bytes = self.event_bytes.saturating_add(bytes);
            if network {
                self.network.push_back(value);
            } else {
                self.console.push_back(value);
            }
        }
    }
}

fn browser_executable() -> Result<String, ToolError> {
    if let Some(path) = std::env::var_os("BRUHS_BROWSER_BIN") {
        if path.is_empty() {
            return Err(ToolError::Invalid(
                "BRUHS_BROWSER_BIN is empty. Set it to a Chrome or Chromium executable.".into(),
            ));
        }
        return Ok(path.to_string_lossy().into_owned());
    }
    let candidates = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/usr/bin/google-chrome",
        "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
    ];
    candidates.iter().find(|path| process::path_exists(path)).map(|path| (*path).into()).ok_or_else(|| ToolError::Unavailable("Install Chrome or Chromium, or set BRUHS_BROWSER_BIN to its executable. Bruhs opens a separate profile and keeps Chrome's sandbox enabled.".into()))
}

async fn browser_user_agent(executable: &str, policy: ProcessPolicy) -> Result<String, ToolError> {
    let output = process::run(executable, &["--version".into()], policy).await?;
    let output = process::text(&output)?;
    let version = output
        .split_whitespace()
        .find(|value| {
            value.contains('.')
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte == b'.')
        })
        .ok_or_else(|| {
            ToolError::Unavailable(
                "Chrome returned no version for the verification browser identity.".into(),
            )
        })?;
    let platform = if cfg!(target_os = "macos") {
        "Macintosh; Intel Mac OS X 10_15_7"
    } else if cfg!(target_os = "windows") {
        "Windows NT 10.0; Win64; x64"
    } else {
        "X11; Linux x86_64"
    };
    // This launch-wide identity also reaches WebSocket handshakes, which do
    // not consistently inherit CDP extra headers. Keep the installed Chrome
    // version and normal UA shape for application browser detection.
    Ok(format!(
        "Mozilla/5.0 ({platform}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{version} Safari/537.36 BruhsVerification/1"
    ))
}
