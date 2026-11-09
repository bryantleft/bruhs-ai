//! Public connection metadata. OAuth credentials stay in the native broker.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceProvider {
    Google,
    Microsoft,
    Github,
}

impl ServiceProvider {
    pub const ALL: [Self; 3] = [Self::Google, Self::Microsoft, Self::Github];

    pub const fn id(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::Microsoft => "microsoft",
            Self::Github => "github",
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Google => "Google Workspace",
            Self::Microsoft => "Microsoft 365",
            Self::Github => "GitHub",
        }
    }

    pub const fn access_summary(self) -> &'static str {
        match self {
            Self::Google => "Read calendar events and identify your account",
            Self::Microsoft => "Read basic calendar details and identify your account",
            Self::Github => "Read your profile and organization memberships",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Disconnected,
    Connected,
    NeedsSignIn,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceConnection {
    pub id: Option<String>,
    pub provider: ServiceProvider,
    pub configured: bool,
    pub account: Option<String>,
    pub scopes: Vec<String>,
    pub state: ConnectionState,
}

/// Private UI response; do not forward this authorization URL to model events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionStart {
    pub id: String,
    pub auth_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptState {
    Pending,
    Connected,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionAttempt {
    pub id: String,
    pub state: AttemptState,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionLogin {
    #[serde(default)]
    pub thread_id: Option<crate::ids::ThreadId>,
    #[serde(default)]
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceOperation {
    pub provider: ServiceProvider,
    pub reason: String,
    pub env_var: String,
    pub command: Vec<String>,
}

impl ServiceOperation {
    pub fn secret_operation(&self) -> crate::secrets::SecretOperation {
        crate::secrets::SecretOperation {
            name: self.provider.name().into(),
            reason: self.reason.clone(),
            env_var: self.env_var.clone(),
            command: self.command.clone(),
        }
    }
}
