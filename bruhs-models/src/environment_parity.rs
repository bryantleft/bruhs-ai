//! Shared models for managed relay, offline webhook holding, Simlock device pools,
//! durable device access policies, and WSL backend management.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::local_tools::DevicePlatform;

/// Durable agent access policy for simulators and emulators across sessions/restarts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceAccessPolicy {
    #[default]
    Ask,
    ThreadOnly,
    Persistent,
    Off,
}

impl DeviceAccessPolicy {
    pub const CHOICES: &[(&str, &str)] = &[
        ("ask", "Ask per session"),
        ("thread_only", "Current thread only"),
        ("persistent", "Remember across restarts"),
        ("off", "Off"),
    ];

    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::ThreadOnly => "thread_only",
            Self::Persistent => "persistent",
            Self::Off => "off",
        }
    }

    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "ask" => Some(Self::Ask),
            "thread_only" => Some(Self::ThreadOnly),
            "persistent" => Some(Self::Persistent),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
}

/// Configuration for a remote Simlock simulator/emulator lease pool host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SimlockHostSpec {
    pub id: Uuid,
    pub label: String,
    pub origin: String,
    pub max_devices: u8,
    pub lease_minutes: u16,
}

impl Default for SimlockHostSpec {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            label: String::new(),
            origin: String::new(),
            max_devices: 2,
            lease_minutes: 30,
        }
    }
}

impl SimlockHostSpec {
    /// Validate host label, origin URL scheme/authority, and bounded lease limits.
    pub fn validate(&self) -> Result<(), &'static str> {
        let label = self.label.trim();
        if label.is_empty() || label.chars().count() > 64 || label.chars().any(char::is_control) {
            return Err("Enter a host label between 1 and 64 characters.");
        }
        let origin = self.origin.trim();
        let rest = origin
            .strip_prefix("https://")
            .or_else(|| origin.strip_prefix("http://"))
            .ok_or("Enter an http:// or https:// Simlock origin.")?;
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        if authority.is_empty()
            || authority.contains('@')
            || origin
                .chars()
                .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
        {
            return Err("Simlock origin must have a host and no credentials or whitespace.");
        }
        if !(1..=16).contains(&self.max_devices) {
            return Err("Max devices must be between 1 and 16.");
        }
        if !(5..=240).contains(&self.lease_minutes) {
            return Err("Lease duration must be between 5 and 240 minutes.");
        }
        Ok(())
    }
}

/// Active lease against a Simlock device host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimlockLease {
    pub lease_id: Uuid,
    pub host_id: Uuid,
    pub host_label: String,
    pub device_id: String,
    pub platform: DevicePlatform,
    pub thread_id: String,
    pub acquired_at_ms: i64,
    pub expires_at_ms: i64,
}

/// Request to acquire a Simlock device lease for a thread.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimlockLeaseRequest {
    pub host_id: Uuid,
    pub device_id: String,
    pub platform: DevicePlatform,
    pub thread_id: String,
}

/// Reachability probe outcome for a configured Simlock host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimlockProbeResult {
    pub host_id: Uuid,
    pub reachable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    pub detail: String,
}

/// Full Simlock pool + durable device policy state returned by the server.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SimlockPoolSnapshot {
    pub policy: DeviceAccessPolicy,
    pub hosts: Vec<SimlockHostSpec>,
    pub active_leases: Vec<SimlockLease>,
}

/// Request body to update durable device access policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevicePolicyUpdate {
    pub policy: DeviceAccessPolicy,
}

fn default_true() -> bool {
    true
}

fn default_hold_hours() -> u16 {
    24
}

/// Managed cloud relay / tunnel preferences for this environment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RelayConfig {
    pub enabled: bool,
    pub relay_url: String,
    pub environment_label: String,
    pub publish_agent_activity: bool,
    #[serde(default = "default_true")]
    pub preview_tunnel: bool,
    #[serde(default = "default_true")]
    pub require_device_approval: bool,
    pub hold_webhooks_while_offline: bool,
    #[serde(default = "default_hold_hours")]
    pub max_hold_hours: u16,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            relay_url: String::new(),
            environment_label: String::new(),
            publish_agent_activity: false,
            preview_tunnel: true,
            require_device_approval: true,
            hold_webhooks_while_offline: false,
            max_hold_hours: default_hold_hours(),
        }
    }
}

impl RelayConfig {
    /// Validate relay URL and retention window.
    pub fn validate(&self) -> Result<(), &'static str> {
        let url = self.relay_url.trim();
        if self.enabled || !url.is_empty() {
            let rest = url
                .strip_prefix("https://")
                .ok_or("Managed relay URL must use https://.")?;
            let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
            if authority.is_empty()
                || authority.contains('@')
                || url
                    .chars()
                    .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
            {
                return Err("Relay URL must have a valid host and no credentials or whitespace.");
            }
        }
        if !(1..=24).contains(&self.max_hold_hours) {
            return Err("Offline webhook retention must be between 1 and 24 hours.");
        }
        Ok(())
    }
}

/// Public summary of a webhook held while this environment was offline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeldWebhookSummary {
    pub id: Uuid,
    pub task_id: String,
    pub delivery_key: String,
    pub received_at_ms: i64,
    pub expires_at_ms: i64,
    pub uri: String,
    pub body_bytes: usize,
}

/// Request to enqueue a webhook in the offline holding mailbox.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoldWebhookRequest {
    pub task_id: String,
    pub delivery_key: String,
    #[serde(default)]
    pub uri: String,
    #[serde(default)]
    pub headers: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub query: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub body: String,
}

/// Result of flushing held offline webhooks.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlushWebhooksResponse {
    pub flushed: usize,
    pub expired: usize,
    pub failed: usize,
    pub remaining: usize,
}

/// Current managed relay state and offline webhook mailbox summary.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RelaySnapshot {
    pub config: RelayConfig,
    pub held_webhook_count: usize,
    pub held_webhooks: Vec<HeldWebhookSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_flushed_at_ms: Option<i64>,
}

/// Running state of a discovered WSL distribution.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WslDistroState {
    Running,
    Stopped,
    Installing,
    #[default]
    Unknown,
}

/// Single WSL distribution entry reported by `wsl.exe --list --verbose`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WslDistro {
    pub name: String,
    pub state: WslDistroState,
    pub version: u8,
    pub is_default: bool,
}

/// Persisted WSL backend preferences (Windows hosts).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WslBackendConfig {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_distro: Option<String>,
    pub stop_windows_backend: bool,
}

/// Lifecycle action requested for a WSL distribution backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WslAction {
    Start,
    Stop,
    Restart,
}

/// Full WSL backend status returned by the server.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WslBackendStatus {
    pub platform_supported: bool,
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running_distro: Option<String>,
    pub distros: Vec<WslDistro>,
    pub config: WslBackendConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simlock_host_validation_rejects_bad_origins_and_limits() {
        let mut host = SimlockHostSpec {
            label: "Lab Studio".into(),
            origin: "https://simlock.tailnet.ts.net:4700".into(),
            max_devices: 4,
            lease_minutes: 30,
            ..Default::default()
        };
        assert!(host.validate().is_ok());

        host.origin = "ftp://bad.host".into();
        assert!(host.validate().is_err());

        host.origin = "https://user:pass@simlock.test".into();
        assert!(host.validate().is_err());

        host.origin = "https://simlock.test".into();
        host.max_devices = 0;
        assert!(host.validate().is_err());

        host.max_devices = 2;
        host.lease_minutes = 999;
        assert!(host.validate().is_err());
    }

    #[test]
    fn relay_config_requires_https_when_enabled() {
        let mut cfg = RelayConfig::default();
        assert!(cfg.validate().is_ok());

        cfg.enabled = true;
        assert!(cfg.validate().is_err());

        cfg.relay_url = "http://insecure.example".into();
        assert!(cfg.validate().is_err());

        cfg.relay_url = "https://relay.bruhs.example".into();
        assert!(cfg.validate().is_ok());
    }
}
