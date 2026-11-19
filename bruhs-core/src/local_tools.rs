//! Dedicated local verification tools. A grant is required before every action.
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

use bruhs_models::local_tools::*;
use tokio::sync::watch;
use uuid::Uuid;

mod browser;
mod computer;
mod devices;
mod network;
mod process;
mod screencast;
#[cfg(test)]
mod tests;
mod video;

pub use computer::{BruhComputer, FillValue};
#[cfg(any(test, feature = "test-support"))]
pub use computer::{FakePages, PageLaunch};
#[cfg(test)]
pub(crate) use network::interface_addresses_for_tests;
pub(crate) use network::is_this_computer;
pub use process::ProcessPolicy;
pub use screencast::{ComputerWatch, Viewer};
pub use video::DeviceVideo;

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("Local tool access was revoked. Allow the tool again before continuing.")]
    Revoked,
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Unavailable(String),
    /// The host refuses this, whatever the request's shape (403).
    #[error("{0}")]
    Refused(String),
    /// What the request names doesn't exist (404).
    #[error("{0}")]
    NotFound(String),
    #[error("Local tool I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Browser connection failed: {0}")]
    Socket(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("Local tool returned invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Local tool request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Local tool worker stopped: {0}")]
    Worker(#[from] tokio::task::JoinError),
}

struct GrantState {
    grant: ToolGrant,
    revoked: watch::Sender<bool>,
}

type BrowserHandle = Arc<tokio::sync::Mutex<browser::Browser>>;
type BrowserSessions = HashMap<String, (String, BrowserHandle)>;

pub struct LocalToolsHost {
    grants: Mutex<HashMap<String, GrantState>>,
    browsers: tokio::sync::Mutex<BrowserSessions>,
    opening: Mutex<HashSet<String>>,
    video: Mutex<HashSet<String>>,
    /// Bruh computers. Their grants and pages never enter `grants` or
    /// `browsers`, so Verification and the thread routes can't see them.
    computers: computer::Computers,
    launcher: computer::Launcher,
    /// The owner's upstream proxies, read at each bruh browser launch.
    network: fn() -> Result<network::Upstream, ToolError>,
    client: reqwest::Client,
    policy: ProcessPolicy,
    /// The host itself, for tasks that outlive a call, such as a bruh's
    /// window watcher. It never keeps the host alive.
    this: Weak<Self>,
}

impl LocalToolsHost {
    pub fn new() -> Result<Arc<Self>, ToolError> {
        Self::with_policy(ProcessPolicy::default())
    }

    pub fn with_policy(policy: ProcessPolicy) -> Result<Arc<Self>, ToolError> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(crate::capture_client::CONNECT_TIMEOUT)
            .build()?;
        let launcher = computer::browser_launcher(client.clone(), policy);
        Self::with_parts(
            policy,
            client,
            launcher,
            network::Upstream::from_env,
            computer::platform_refusal(),
        )
    }

    /// `refusal` says why no bruh browser opens on this system, if none does.
    fn with_parts(
        policy: ProcessPolicy,
        client: reqwest::Client,
        launcher: computer::Launcher,
        network: fn() -> Result<network::Upstream, ToolError>,
        refusal: Option<&'static str>,
    ) -> Result<Arc<Self>, ToolError> {
        if policy.timeout.is_zero() || policy.max_output_bytes == 0 {
            return Err(ToolError::Invalid(
                "Local tools need a nonzero command deadline and capture budget.".into(),
            ));
        }
        Ok(Arc::new_cyclic(|this| Self {
            grants: Mutex::new(HashMap::new()),
            browsers: tokio::sync::Mutex::new(HashMap::new()),
            opening: Mutex::new(HashSet::new()),
            video: Mutex::new(HashSet::new()),
            computers: computer::Computers::new(refusal),
            launcher,
            network,
            client,
            policy,
            this: this.clone(),
        }))
    }

    /// Only user-facing permission controls call this, never agent tools.
    /// A bruh's browser comes from its Computer switch, never from a grant.
    pub fn grant(&self, request: GrantRequest) -> Result<ToolGrant, ToolError> {
        let principal = request.principal.trim();
        if principal.is_empty() || principal.chars().any(char::is_control) {
            return Err(ToolError::Invalid(
                "Choose a named task or user before allowing tools.".into(),
            ));
        }
        if principal.starts_with(BRUH_PRINCIPAL_PREFIX) {
            return Err(ToolError::Invalid(
                bruhs_models::crew::TURN_ON_IN_SETTINGS.into(),
            ));
        }
        let mut grants = self.grants.lock().map_err(|_| {
            ToolError::Unavailable("Tool permission state is unavailable; restart Bruhs.".into())
        })?;
        if let Some(existing) = grants
            .values()
            .find(|state| state.grant.principal == principal && state.grant.scope == request.scope)
        {
            return Ok(existing.grant.clone());
        }
        let grant = ToolGrant {
            id: Uuid::new_v4().to_string(),
            principal: principal.into(),
            scope: request.scope,
        };
        let (revoked, _) = watch::channel(false);
        grants.insert(
            grant.id.clone(),
            GrantState {
                grant: grant.clone(),
                revoked,
            },
        );
        drop(grants);
        Ok(grant)
    }

    pub fn require(&self, id: &str, scope: ToolScope) -> Result<watch::Receiver<bool>, ToolError> {
        self.grants
            .lock()
            .map_err(|_| ToolError::Revoked)?
            .get(id)
            .filter(|state| state.grant.scope == scope)
            .map(|state| state.revoked.subscribe())
            .ok_or(ToolError::Revoked)
    }

    pub async fn revoke(&self, id: &str) -> Result<(), ToolError> {
        if let Some(state) = self
            .grants
            .lock()
            .map_err(|_| ToolError::Revoked)?
            .remove(id)
        {
            let _ = state.revoked.send(true);
        }
        let browsers: Vec<_> = self
            .browsers
            .lock()
            .await
            .iter()
            .filter(|(_, (grant, _))| grant == id)
            .map(|(id, (_, browser))| (id.clone(), Arc::clone(browser)))
            .collect();
        for (session_id, browser) in browsers {
            let mut browser = browser.lock().await;
            if browser.info.grant_id == id {
                browser.stop().await?;
                drop(browser);
                self.browsers.lock().await.remove(&session_id);
            }
        }
        Ok(())
    }

    pub async fn snapshot(&self) -> Result<LocalToolsSnapshot, ToolError> {
        let grants = self
            .grants
            .lock()
            .map_err(|_| ToolError::Revoked)?
            .values()
            .map(|state| state.grant.clone())
            .collect();
        let sessions: Vec<_> = self
            .browsers
            .lock()
            .await
            .values()
            .map(|(_, browser)| Arc::clone(browser))
            .collect();
        let mut browsers = Vec::with_capacity(sessions.len());
        for session in sessions {
            browsers.push(session.lock().await.info.clone());
        }
        Ok(LocalToolsSnapshot { grants, browsers })
    }

    pub async fn start_browser(&self, request: BrowserStart) -> Result<BrowserSession, ToolError> {
        let mut revoked = self.require(&request.grant_id, ToolScope::Browser)?;
        validate_browser_url(&request.url).map_err(|error| ToolError::Invalid(error.into()))?;
        // One browser per grant prevents accidental repeated launches from a double-click.
        if self
            .browsers
            .lock()
            .await
            .values()
            .any(|(grant, _)| grant == &request.grant_id)
        {
            return Err(ToolError::Invalid(
                "This grant already has a browser. Use its existing session or revoke it first."
                    .into(),
            ));
        }
        let launch = BrowserLaunch::acquire(&self.opening, &request.grant_id)?;
        let options = browser::LaunchOptions::verification();
        let browser = tokio::select! {
            biased;
            _ = revoked.changed() => return Err(ToolError::Revoked),
            result = browser::Browser::start(&self.client, request, self.policy, &options) => result?,
        };
        let info = browser.info.clone();
        // A revoked launch must not become a usable session.
        if *revoked.borrow() {
            return Err(ToolError::Revoked);
        }
        self.browsers.lock().await.insert(
            info.id.clone(),
            (
                info.grant_id.clone(),
                Arc::new(tokio::sync::Mutex::new(browser)),
            ),
        );
        drop(launch);
        Ok(info)
    }

    pub async fn browser_command(&self, request: BrowserCommand) -> Result<ToolReply, ToolError> {
        let mut revoked = self.require(&request.grant_id, ToolScope::Browser)?;
        let session = self
            .browsers
            .lock()
            .await
            .get(&request.session_id)
            .map(|(_, browser)| Arc::clone(browser))
            .ok_or_else(|| {
                ToolError::Invalid("This browser session has ended. Start another browser.".into())
            })?;
        let mut browser = tokio::select! { biased; _ = revoked.changed() => return Err(ToolError::Revoked), browser = session.lock() => browser };
        if browser.info.grant_id != request.grant_id {
            return Err(ToolError::Revoked);
        }
        let reply = tokio::select! {
            biased;
            _ = revoked.changed() => Err(ToolError::Revoked),
            result = browser.command(request.command, self.policy) => result,
        };
        drop(browser);
        reply
    }

    pub async fn devices(&self, grant_id: &str) -> Result<DeviceInventory, ToolError> {
        let mut revoked = self.require(grant_id, ToolScope::Devices)?;
        tokio::select! { biased; _ = revoked.changed() => Err(ToolError::Revoked), result = devices::inventory(self.policy) => result }
    }

    pub async fn device_command(&self, request: DeviceCommand) -> Result<ToolReply, ToolError> {
        let mut revoked = self.require(&request.grant_id, ToolScope::Devices)?;
        tokio::select! { biased; _ = revoked.changed() => Err(ToolError::Revoked), result = devices::execute(request, self.policy) => result }
    }

    pub fn command_timeout(&self) -> Duration {
        self.policy.timeout
    }

    pub async fn start_device_video(
        self: &Arc<Self>,
        request: DeviceVideoStart,
    ) -> Result<DeviceVideo, ToolError> {
        video::start(self, request).await
    }
}

struct BrowserLaunch<'a> {
    opening: &'a Mutex<HashSet<String>>,
    grant: String,
}
impl<'a> BrowserLaunch<'a> {
    fn acquire(opening: &'a Mutex<HashSet<String>>, grant: &str) -> Result<Self, ToolError> {
        if !opening
            .lock()
            .map_err(|_| ToolError::Revoked)?
            .insert(grant.into())
        {
            return Err(ToolError::Invalid(
                "This browser is already starting. Wait for its result.".into(),
            ));
        }
        Ok(Self {
            opening,
            grant: grant.into(),
        })
    }
}
impl Drop for BrowserLaunch<'_> {
    fn drop(&mut self) {
        if let Ok(mut opening) = self.opening.lock() {
            opening.remove(&self.grant);
        }
    }
}

async fn screenshot(bytes: Vec<u8>) -> Result<ToolReply, ToolError> {
    use base64::Engine;
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(ToolError::Invalid(
            "The device or browser returned a non-PNG screenshot.".into(),
        ));
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let uploaded = tokio::task::spawn_blocking(move || {
        crate::uploads::write_upload("verification.png", &bytes)
            .map_err(|error| ToolError::Unavailable(error.into()))
    })
    .await??;
    Ok(ToolReply {
        value: serde_json::json!({"path": uploaded.path}),
        image: Some(ToolImage {
            mime_type: "image/png".into(),
            base64: encoded,
            path: uploaded.path,
        }),
    })
}
