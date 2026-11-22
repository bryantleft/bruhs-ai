//! Owner-managed SSH forwarding profiles. Authentication remains with OpenSSH.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteHostProfile {
    pub id: Uuid,
    pub name: String,
    /// An existing OpenSSH alias or user@host, passed as one literal argument.
    pub host: String,
    pub ssh_port: u16,
    pub local_port: u16,
    pub remote_port: u16,
    /// Explicit owner budget for SSH authentication and forward readiness.
    pub connect_timeout_seconds: u32,
}

impl RemoteHostProfile {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.name.trim().is_empty() || self.name.len() > crate::mobile::REQUEST_BYTES {
            return Err("Enter a name that fits a local control request.");
        }
        if self.host.is_empty()
            || self.host.starts_with('-')
            || self.host.len() > crate::mobile::REQUEST_BYTES
            || !self
                .host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-@[]:".contains(&b))
        {
            return Err("Use one SSH alias or user@host without whitespace or shell expressions.");
        }
        if self.ssh_port == 0 || self.remote_port == 0 || self.local_port < 1024 {
            return Err("Use nonzero remote ports and an unprivileged local port (1024–65535).");
        }
        if self.connect_timeout_seconds == 0 {
            return Err("Choose a positive connection time budget in seconds.");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteHostState {
    Disconnected,
    Connecting,
    Connected,
    Disconnecting,
    Interrupted,
    Failed,
}
impl RemoteHostState {
    pub fn active(self) -> bool {
        matches!(
            self,
            Self::Connecting | Self::Connected | Self::Disconnecting
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteHost {
    pub profile: RemoteHostProfile,
    pub state: RemoteHostState,
    pub error: Option<String>,
    pub updated_at_ms: i64,
    /// Only this endpoint is forwarded. It still requires its own pairing grant.
    pub local_url: String,
    #[serde(default)]
    pub connection_id: Option<Uuid>,
    /// Private local socket for this live SSH generation, filled by the owner service.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forwarding_socket: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RemoteHostSnapshot {
    pub hosts: Vec<RemoteHost>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum RemoteHostCommand {
    Save { profile: RemoteHostProfile },
    Connect { id: Uuid },
    Disconnect { id: Uuid },
    Delete { id: Uuid },
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profiles_cannot_inject_options_commands_or_privileged_forwards() {
        let mut profile = RemoteHostProfile {
            id: Uuid::new_v4(),
            name: "Dev box".into(),
            host: "me@devbox".into(),
            ssh_port: 22,
            local_port: 45001,
            remote_port: 45000,
            connect_timeout_seconds: 15,
        };
        assert!(profile.validate().is_ok());
        for host in [
            "-F/tmp/config",
            "me@host;whoami",
            "me@host $(id)",
            "host\nProxyCommand=evil",
        ] {
            profile.host = host.into();
            assert!(profile.validate().is_err());
        }
        profile.host = "devbox".into();
        profile.local_port = 22;
        assert!(profile.validate().is_err());
    }
}
