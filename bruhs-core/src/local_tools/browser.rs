use std::{
    collections::VecDeque,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

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

#[path = "browser_input.rs"]
mod browser_input;
use browser_input::{click_events, key_events, wheel_event, KEY, MOUSE, TEXT};

/// How a browser launches: where its profile lives, whether it shows a
/// window, and the proxy its traffic goes through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LaunchOptions {
    /// A kept profile (a bruh's), created `0700` when missing. `None` is a
    /// temporary profile removed with the browser.
    pub profile: Option<PathBuf>,
    pub headless: bool,
    /// The network guard. Loopback goes through it too.
    pub proxy: Option<SocketAddr>,
}

impl LaunchOptions {
    /// A thread's verification browser: a temporary profile in a window,
    /// with no proxy.
    pub fn verification() -> Self {
        Self {
            profile: None,
            headless: false,
            proxy: None,
        }
    }
}

/// Chrome's command line for a launch on `profile`.
pub(super) fn launch_args(
    options: &LaunchOptions,
    profile: &Path,
    user_agent: &str,
) -> Vec<String> {
    let mut args = vec![
        format!("--user-data-dir={}", profile.display()),
        format!("--user-agent={user_agent}"),
    ];
    args.extend(
        [
            "--remote-debugging-port=0",
            "--remote-debugging-address=127.0.0.1",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-extensions",
        ]
        .map(String::from),
    );
    if options.profile.is_some() {
        // A kept profile opens both headless and in a window. Headless Chrome
        // uses the mock keychain on macOS and a window the real one, and
        // cookies encrypted under one key can't be read under the other, so
        // every launch of a kept profile pins the same key store.
        args.extend(["--use-mock-keychain", "--password-store=basic"].map(String::from));
    }
    if options.headless {
        args.push("--headless=new".into());
        args.push(format!(
            "--window-size={},{}",
            COMPUTER_VIEWPORT.width, COMPUTER_VIEWPORT.height
        ));
    }
    if let Some(proxy) = options.proxy {
        args.push(format!("--proxy-server=http://{proxy}"));
        // Chrome bypasses the proxy for loopback unless told not to.
        args.push("--proxy-bypass-list=<-loopback>".into());
        args.push("--force-webrtc-ip-handling-policy=disable_non_proxied_udp".into());
    }
    args.push("about:blank".into());
    args
}

enum Profile {
    Temporary(tempfile::TempDir),
    Kept(PathBuf),
}

impl Profile {
    async fn open(kept: Option<&Path>) -> Result<Self, ToolError> {
        let Some(path) = kept else {
            return Ok(Self::Temporary(
                tempfile::Builder::new()
                    .prefix("bruhs-browser-")
                    .tempdir()?,
            ));
        };
        let mut folder = tokio::fs::DirBuilder::new();
        folder.recursive(true);
        #[cfg(unix)]
        folder.mode(0o700);
        folder.create(path).await?;
        Ok(Self::Kept(path.to_owned()))
    }

    fn path(&self) -> &Path {
        match self {
            Self::Temporary(folder) => folder.path(),
            Self::Kept(path) => path,
        }
    }
}

pub(super) struct Browser {
    pub info: BrowserSession,
    child: Arc<Mutex<Child>>,
    _profile: Profile,
    /// A kept profile's pid file, removed when the browser closes.
    pid_file: Option<PathBuf>,
    logs: tokio::task::JoinHandle<()>,
    guard: BrowserGuard,
    cdp: Cdp,
    /// The browser-level `ws://…/devtools/browser/…` endpoint.
    endpoint: String,
    /// The page target's `webSocketDebuggerUrl`.
    page_socket: String,
    viewport: Viewport,
    policy: ProcessPolicy,
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
        options: &LaunchOptions,
    ) -> Result<Self, ToolError> {
        let executable = browser_executable()?;
        let user_agent = browser_user_agent(&executable, policy).await?;
        let profile = Profile::open(options.profile.as_deref()).await?;
        if let Some(kept) = options.profile.as_deref() {
            release_profile(kept, policy).await?;
        }
        let mut child = Command::new(executable)
            .args(launch_args(options, profile.path(), &user_agent))
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
            if options.profile.is_some() {
                // Only bruhs keep a profile. Their browsers never write files
                // to this computer behind the owner's back.
                cdp.call("Browser.setDownloadBehavior", json!({"behavior":"deny"}))
                    .await?;
            }
            let viewport = if options.headless {
                cdp.call(
                    "Emulation.setDeviceMetricsOverride",
                    json!({"width":COMPUTER_VIEWPORT.width,"height":COMPUTER_VIEWPORT.height,"deviceScaleFactor":1,"mobile":false}),
                )
                .await?;
                COMPUTER_VIEWPORT
            } else {
                // Measured on about:blank, before any page script can run.
                measured_viewport(&mut cdp).await?
            };
            cdp.navigate(&request.url).await?;
            guard.check()?;
            Ok::<_, ToolError>((cdp, guard, socket.to_owned(), viewport))
        };
        let (cdp, guard, page_socket, viewport) =
            match tokio::time::timeout(policy.timeout, result).await {
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
        let pid_file = match options.profile.as_deref() {
            Some(kept) => write_pid_file(kept, &child).await,
            None => None,
        };
        Ok(Self {
            info: BrowserSession {
                id: uuid::Uuid::new_v4().to_string(),
                grant_id: request.grant_id,
                url: request.url,
            },
            child,
            _profile: profile,
            pid_file,
            logs,
            guard,
            cdp,
            endpoint: endpoint.into(),
            page_socket,
            viewport,
            policy,
        })
    }

    /// The page target's debugging socket, for a second connection such as
    /// the live view's.
    pub fn page_socket(&self) -> &str {
        &self.page_socket
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

    /// Ask Chrome to quit, so it writes cookies and the rest of the profile
    /// to disk, and kill it if it hasn't exited within `grace`.
    pub async fn close(&mut self, grace: Duration) -> Result<(), ToolError> {
        // The guard kills Chrome as soon as its connection drops, which
        // `Browser.close` does first.
        self.guard.disarm().await;
        let deadline = tokio::time::Instant::now() + grace;
        let (endpoint, policy) = (self.endpoint.clone(), self.policy);
        let _ = tokio::time::timeout_at(deadline, async move {
            let mut browser = Cdp::connect(&endpoint, policy).await?;
            // Chrome can drop the connection before it replies; the exit
            // below is what counts.
            let _ = browser.call("Browser.close", json!({})).await;
            Ok::<_, ToolError>(())
        })
        .await;
        let mut child = self.child.lock().await;
        let exited = tokio::time::timeout_at(deadline, child.wait()).await;
        if !matches!(exited, Ok(Ok(_))) {
            child.kill().await?;
        }
        drop(child);
        self.logs.abort();
        if let Some(pid_file) = self.pid_file.take() {
            let _ = tokio::fs::remove_file(pid_file).await;
        }
        Ok(())
    }

    #[cfg(test)]
    pub async fn exit_status(&self) -> Option<std::process::ExitStatus> {
        self.child.lock().await.try_wait().ok().flatten()
    }

    pub async fn command(
        &mut self,
        action: BrowserAction,
        policy: ProcessPolicy,
    ) -> Result<ToolReply, ToolError> {
        self.guard.check()?;
        let navigated = match &action {
            BrowserAction::Navigate { url } => Some(url.clone()),
            _ => None,
        };
        let reply = tokio::time::timeout(policy.timeout, act(&mut self.cdp, action, self.viewport))
            .await
            .map_err(|_| {
                ToolError::Unavailable(
                    "Browser action timed out. Check the page and retry or revoke browser access."
                        .into(),
                )
            })?;
        self.guard.check()?;
        if let (Ok(_), Some(url)) = (&reply, navigated) {
            self.info.url = url;
        }
        reply
    }

    /// The page's address now (`location.href`).
    pub async fn current_url(&mut self) -> Result<String, ToolError> {
        self.guard.check()?;
        let href = tokio::time::timeout(self.policy.timeout, self.cdp.evaluate("location.href"))
            .await
            .map_err(|_| {
                ToolError::Unavailable("Chrome didn't say which page is open.".into())
            })??;
        href.as_str()
            .map(str::to_owned)
            .ok_or_else(|| ToolError::Unavailable("Chrome didn't say which page is open.".into()))
    }

    /// One input from the owner's live view. Navigation follows the agent's
    /// URL rules.
    pub async fn owner_input(&mut self, input: &ComputerInput) -> Result<(), ToolError> {
        validate_input(input, self.viewport).map_err(|error| ToolError::Invalid(error.into()))?;
        self.guard.check()?;
        let cdp = &mut self.cdp;
        let operation = async {
            match input {
                ComputerInput::Navigate { url } => {
                    cdp.navigate(url).await?;
                }
                ComputerInput::Back => {
                    cdp.evaluate("history.back()").await?;
                }
                ComputerInput::Reload => {
                    cdp.call("Page.reload", json!({})).await?;
                }
                _ => {
                    for (method, params) in browser_input::owner_events(input) {
                        cdp.call(method, params).await?;
                    }
                }
            }
            Ok::<_, ToolError>(())
        };
        tokio::time::timeout(self.policy.timeout, operation)
            .await
            .map_err(|_| {
                ToolError::Unavailable("The page didn't take the input in time.".into())
            })??;
        self.guard.check()?;
        if let ComputerInput::Navigate { url } = input {
            self.info.url.clone_from(url);
        }
        Ok(())
    }
}

/// What a launch on a bruh's profile says when a Chrome Bruhs didn't start
/// holds it.
pub(super) const PROFILE_IN_USE: &str =
    "Another Chrome is using this bruh's profile. Quit it and try again.";

/// A kept profile's pid file: `<home>/bruhs/<id>/browser.pid` beside
/// `<home>/bruhs/<id>/browser`.
fn pid_file(profile: &Path) -> PathBuf {
    profile.with_extension("pid")
}

/// Records the browser's pid beside its kept profile, so a Bruhs that crashed
/// can stop it at the next launch. A failure only loses that recovery.
async fn write_pid_file(profile: &Path, child: &Mutex<Child>) -> Option<PathBuf> {
    let pid = child.lock().await.id()?;
    let path = pid_file(profile);
    match tokio::fs::write(&path, pid.to_string()).await {
        Ok(()) => Some(path),
        Err(error) => {
            tracing::warn!(%error, "couldn't record the bruh browser's pid");
            None
        }
    }
}

/// Before a kept profile opens. A Chrome that a crashed Bruhs left running on
/// it (named by the pid file, with this profile on its command line) is
/// stopped. Any other live Chrome holding the profile's lock is an error:
/// a new launch would hand its URL to that Chrome and exit.
#[cfg(unix)]
pub(super) async fn release_profile(
    profile: &Path,
    policy: ProcessPolicy,
) -> Result<(), ToolError> {
    let pid_file = pid_file(profile);
    if let Some(pid) = tokio::fs::read_to_string(&pid_file)
        .await
        .ok()
        .and_then(|text| process_id(text.trim()))
    {
        if alive(pid) && uses_profile(pid, profile, policy).await == Some(true) {
            stop_process(pid).await;
        }
        let _ = tokio::fs::remove_file(&pid_file).await;
    }
    // Chrome's lock is a symlink to `<hostname>-<pid>`.
    let holder = std::fs::read_link(profile.join("SingletonLock"))
        .ok()
        .and_then(|target| {
            let target = target.to_string_lossy().into_owned();
            target.rsplit_once('-').and_then(|(_, pid)| process_id(pid))
        });
    if let Some(pid) = holder {
        if alive(pid) && uses_profile(pid, profile, policy).await != Some(false) {
            return Err(ToolError::Unavailable(PROFILE_IN_USE.into()));
        }
    }
    Ok(())
}

#[cfg(not(unix))]
pub(super) async fn release_profile(
    _profile: &Path,
    _policy: ProcessPolicy,
) -> Result<(), ToolError> {
    Ok(())
}

/// A pid that names one ordinary process: never 0, 1 or a negative value,
/// which `kill` reads as a process group or every process.
#[cfg(unix)]
fn process_id(text: &str) -> Option<libc::pid_t> {
    text.parse::<libc::pid_t>().ok().filter(|pid| *pid > 1)
}

#[cfg(unix)]
fn alive(pid: libc::pid_t) -> bool {
    // SAFETY: signal 0 sends nothing; it only checks that `pid` exists.
    let found = unsafe { libc::kill(pid, 0) } == 0;
    found || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Whether `pid` runs with `--user-data-dir=<profile>`, or `None` when its
/// command line can't be read.
#[cfg(unix)]
async fn uses_profile(pid: libc::pid_t, profile: &Path, policy: ProcessPolicy) -> Option<bool> {
    let args = ["-ww", "-o", "command=", "-p"].map(String::from);
    let mut args = args.to_vec();
    args.push(pid.to_string());
    let output = process::run("ps", &args, policy).await.ok()?;
    let line = String::from_utf8_lossy(&output);
    let flag = format!("--user-data-dir={}", profile.display());
    Some(line.match_indices(&flag).any(|(at, _)| {
        line[at + flag.len()..]
            .chars()
            .next()
            .is_none_or(char::is_whitespace)
    }))
}

/// SIGTERM, so Chrome writes its profile, then SIGKILL after the close grace.
#[cfg(unix)]
async fn stop_process(pid: libc::pid_t) {
    let grace = Duration::from_millis(bruhs_models::crew::CLOSE_GRACE_MS);
    for signal in [libc::SIGTERM, libc::SIGKILL] {
        // SAFETY: `pid` is a single process (`process_id` refuses group and
        // broadcast values) whose command line names this bruh's profile.
        unsafe { libc::kill(pid, signal) };
        let deadline = tokio::time::Instant::now() + grace;
        while alive(pid) && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if !alive(pid) {
            return;
        }
    }
}

/// The page's text, title and controls. It never reads input values, so a
/// filled password stays out of the agent's context.
pub(super) const SNAPSHOT_EXPRESSION: &str = r#"({url:location.href,title:document.title,text:document.body?.innerText,controls:[...document.querySelectorAll('a,button,input,textarea,select,[role=button]')].map(e=>({tag:e.tagName,text:e.innerText||e.getAttribute('aria-label')||'',id:e.id,name:e.name,type:e.type,href:e.href})),readyState:document.readyState})"#;

/// Run one agent action on a page. Real input actions are checked against
/// `viewport` and their budgets first.
pub(super) async fn act(
    cdp: &mut Cdp,
    action: BrowserAction,
    viewport: Viewport,
) -> Result<ToolReply, ToolError> {
    if matches!(
        action,
        BrowserAction::ClickAt { .. }
            | BrowserAction::Type { .. }
            | BrowserAction::Key { .. }
            | BrowserAction::Scroll { .. }
    ) {
        validate_action(&action, viewport).map_err(|error| ToolError::Invalid(error.into()))?;
    }
    let value = match action {
        BrowserAction::Navigate { url } => {
            validate_browser_url(&url).map_err(|error| ToolError::Invalid(error.into()))?;
            cdp.navigate(&url).await?
        }
        BrowserAction::Snapshot => cdp.evaluate(SNAPSHOT_EXPRESSION).await?,
        BrowserAction::Evaluate { expression } => cdp.evaluate(&expression).await?,
        BrowserAction::Click { selector } => click(cdp, &selector, viewport).await?,
        BrowserAction::Fill { selector, text } => {
            let element = one_element(&selector)?;
            let text = serde_json::to_string(&text)?;
            cdp.evaluate(&format!(r#"(()=>{{{element}if(e.disabled||e.readOnly)throw new Error('Element is not editable');const p=e instanceof HTMLTextAreaElement?HTMLTextAreaElement.prototype:e instanceof HTMLInputElement?HTMLInputElement.prototype:null;if(!p)throw new Error('Choose an input or textarea');Object.getOwnPropertyDescriptor(p,'value').set.call(e,{text});e.dispatchEvent(new Event('input',{{bubbles:true}}));e.dispatchEvent(new Event('change',{{bubbles:true}}));return {{filled:true}}}})()"#)).await?
        }
        BrowserAction::Screenshot => {
            let value = cdp
                .call(
                    "Page.captureScreenshot",
                    json!({"format":"png","captureBeyondViewport":false}),
                )
                .await?;
            let data = value["data"]
                .as_str()
                .ok_or_else(|| ToolError::Invalid("Chrome returned no screenshot data.".into()))?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|error| {
                    ToolError::Invalid(format!("Chrome returned an invalid screenshot: {error}"))
                })?;
            return super::screenshot(bytes).await;
        }
        BrowserAction::Console => {
            cdp.call("Runtime.evaluate", json!({"expression":"void 0"}))
                .await?;
            json!({"events":cdp.console})
        }
        BrowserAction::Network => {
            cdp.call("Runtime.evaluate", json!({"expression":"void 0"}))
                .await?;
            json!({"events":cdp.network})
        }
        BrowserAction::ClickAt { x, y } => {
            for event in click_events(x, y) {
                cdp.call(MOUSE, event).await?;
            }
            json!({"clicked":true,"via":"input","x":x,"y":y})
        }
        BrowserAction::Type { text, selector } => {
            if let Some(selector) = selector {
                let element = one_element(&selector)?;
                cdp.evaluate(&format!(r#"(()=>{{{element}if(!(e instanceof HTMLInputElement||e instanceof HTMLTextAreaElement||e.isContentEditable))throw new Error('Choose an input, textarea or editable element');if(e.disabled||e.readOnly)throw new Error('Element is not editable');e.focus();const a=document.activeElement;if(a!==e&&!(a&&a.isContentEditable&&a.contains(e)))throw new Error('Element did not take focus');return true}})()"#)).await?;
            }
            cdp.call(TEXT, json!({"text":text})).await?;
            json!({"typed":text.chars().count()})
        }
        BrowserAction::Key { key } => {
            let mapped = cdp_key(&key)
                .ok_or_else(|| ToolError::Invalid(format!("Bruhs can't press {key}.")))?;
            // An agent's key never carries modifiers.
            for event in key_events(&mapped, 0) {
                cdp.call(KEY, event).await?;
            }
            json!({"pressed":key})
        }
        BrowserAction::Scroll { dx, dy } => {
            let (x, y) = (
                f64::from(viewport.width) / 2.0,
                f64::from(viewport.height) / 2.0,
            );
            cdp.call(MOUSE, wheel_event(x, y, dx, dy)).await?;
            json!({"scrolled":true})
        }
    };
    Ok(ToolReply { value, image: None })
}

/// A script prefix binding `e` to the one element `selector` matches.
fn one_element(selector: &str) -> Result<String, ToolError> {
    let selector = serde_json::to_string(selector)?;
    Ok(format!("const es=document.querySelectorAll({selector});if(es.length!==1)throw new Error('Selector must match exactly one element; found '+es.length);const e=es[0];"))
}

/// A real click at the element's centre once it's scrolled into view, or a
/// script click when it has no box or something else covers its centre.
async fn click(cdp: &mut Cdp, selector: &str, viewport: Viewport) -> Result<Value, ToolError> {
    let element = one_element(selector)?;
    let located = cdp.evaluate(&format!(r#"(()=>{{{element}if(e.disabled)throw new Error('Element is disabled');e.scrollIntoView({{block:'center',inline:'center',behavior:'instant'}});const r=e.getBoundingClientRect(),x=r.left+r.width/2,y=r.top+r.height/2,t=document.elementFromPoint(x,y);return {{x,y,w:r.width,h:r.height,hit:!!t&&(t===e||e.contains(t))}}}})()"#)).await?;
    let centre = || {
        let (x, y) = (located["x"].as_f64()?, located["y"].as_f64()?);
        let boxed = located["w"].as_f64()? > 0.0 && located["h"].as_f64()? > 0.0;
        (boxed && located["hit"] == true).then_some((x, y))
    };
    let centre = centre()
        .filter(|&(x, y)| validate_action(&BrowserAction::ClickAt { x, y }, viewport).is_ok());
    if let Some((x, y)) = centre {
        for event in click_events(x, y) {
            cdp.call(MOUSE, event).await?;
        }
        return Ok(json!({"clicked":true,"via":"input","x":x,"y":y}));
    }
    cdp.evaluate(&format!(r#"(()=>{{{element}if(e.disabled)throw new Error('Element is disabled');e.click();return true}})()"#)).await?;
    Ok(json!({"clicked":true,"via":"script"}))
}

/// A headed window's page size in CSS px, or the computer view when Chrome
/// gives none.
async fn measured_viewport(cdp: &mut Cdp) -> Result<Viewport, ToolError> {
    let size = cdp.evaluate("[innerWidth,innerHeight]").await?;
    let side = |index: usize| {
        size[index]
            .as_u64()
            .and_then(|side| u32::try_from(side).ok())
            .filter(|side| *side > 0)
    };
    Ok(match (side(0), side(1)) {
        (Some(width), Some(height)) => Viewport { width, height },
        _ => COMPUTER_VIEWPORT,
    })
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

    async fn evaluate(&mut self, expression: &str) -> Result<Value, ToolError> {
        let result = self
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
