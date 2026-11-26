//! Linked desktop connection metadata and destination-bound credentials.
use bruhs_models::remote_hosts::{RemoteHostProfile, RemoteHostSnapshot, RemoteHostState};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};
use uuid::Uuid;
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum Destination {
    Https(String),
    Ssh {
        profile: Uuid,
        host: String,
        ssh_port: u16,
        remote_port: u16,
    },
}
impl Destination {
    pub fn from_profile(p: &RemoteHostProfile) -> Self {
        Self::Ssh {
            profile: p.id,
            host: p.host.clone(),
            ssh_port: p.ssh_port,
            remote_port: p.remote_port,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedHost {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub usage: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub usage_history: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub edit_usage_prices: bool,
    pub credential: Uuid,
    pub destination: Destination,
    pub name: String,
}

pub fn endpoint(origin: &str, tunneled: bool) -> Result<reqwest::Url, String> {
    let endpoint = reqwest::Url::parse(if tunneled {
        "http://localhost"
    } else {
        origin.trim()
    })
    .map_err(|_| "Enter the environment’s HTTPS address.")?;
    if (endpoint.scheme() != "https" && !tunneled)
        || endpoint.host_str().is_none()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.path() != "/"
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
    {
        return Err("Enter an HTTPS origin without a path or credentials.".into());
    }

    Ok(endpoint)
}
pub fn client(tunnel: Option<(&str, Option<PathBuf>)>) -> Result<reqwest::Client, String> {
    let builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10));
    #[cfg(unix)]
    let builder = if let Some((name, socket)) = tunnel {
        let socket = socket.ok_or_else(|| format!("Reconnect {name} in Environments."))?;
        builder.no_proxy().unix_socket(socket)
    } else {
        builder
    };
    #[cfg(not(unix))]
    if tunnel.is_some() {
        return Err("Connect using this environment’s HTTPS address.".into());
    }
    builder
        .build()
        .map_err(|_| "Could not start the environment connection.".into())
}

impl Destination {
    pub fn socket(&self, hosts: &RemoteHostSnapshot) -> Option<PathBuf> {
        hosts
            .hosts
            .iter()
            .find(|h| {
                h.state == RemoteHostState::Connected && Self::from_profile(&h.profile) == *self
            })
            .and_then(|h| h.forwarding_socket.clone())
    }
}

pub fn project_token(
    id: Uuid,
    destination: &Destination,
) -> Result<zeroize::Zeroizing<String>, String> {
    let value = keyring::Entry::new("bruhs-linked-projects", &id.to_string())
        .map_err(|_| "Could not access the OS credential store.")?
        .get_password()
        .map(zeroize::Zeroizing::new)
        .map_err(|e| match e {
            keyring::Error::NoEntry => "Link this environment again to restore its access token.",
            _ => "Unlock the OS credential store, then refresh the environment.",
        })?;
    let (issued_for, token): (Destination, String) = serde_json::from_str(&value)
        .map_err(|_| "Link this environment again to restore its access token.")?;
    if issued_for != *destination {
        return Err("The saved destination has changed. Link the environment again.".into());
    }
    Ok(zeroize::Zeroizing::new(token))
}
