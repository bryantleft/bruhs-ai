//! Subscription readings shared by a paired desktop. Only the owning machine's providers are queried.
use super::{grouping::Identity, AccountUsage};
use crate::linked_projects::connection::{self, Destination, SavedHost};
use bruhs_models::remote_hosts::RemoteHostSnapshot;
use futures_util::{stream, StreamExt};
use std::{
    io,
    path::{Path, PathBuf},
};

use bruhs_models::{agent::AgentProvider, agent_extensions::QuotaWindow};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct SharedAccount {
    provider: AgentProvider,
    label: String,
    email: Option<String>,
    workspace: Option<String>,
    fingerprint: Option<String>,
    plan: Option<String>,
    checked_at: i64,
    zero_spend_cap: bool,
    windows: Option<Vec<QuotaWindow>>,
}

pub async fn device_report() -> Vec<SharedAccount> {
    super::device_accounts()
        .await
        .into_iter()
        .filter_map(|account| {
            let provider = account.provider.to_ascii_lowercase().parse().ok()?;
            Some(SharedAccount {
                provider,
                label: account.label,
                email: account.identity.email,
                workspace: account.identity.workspace,
                fingerprint: account.identity.fingerprint,
                plan: account.plan,
                checked_at: account.checked_at,
                zero_spend_cap: account.zero_spend_cap,
                windows: account.windows.ok(),
            })
        })
        .collect()
}

impl SharedAccount {
    pub fn into_account(self) -> AccountUsage {
        let provider = match self.provider {
            AgentProvider::Claude => "Claude",
            AgentProvider::Codex => "Codex",
            AgentProvider::Cursor => "Cursor",
            AgentProvider::OpenCode => "OpenCode",
            AgentProvider::Antigravity => "Antigravity",
            AgentProvider::Grok => "Grok",
            AgentProvider::Acp => "ACP",
        };
        AccountUsage {
            id: self.provider.as_str().into(),
            provider,
            label: self.label,
            checked_at: self.checked_at,
            plan: self.plan,
            zero_spend_cap: self.zero_spend_cap,
            windows: self.windows.ok_or_else(|| {
                std::io::Error::other("The environment could not read these limits.")
            }),
            identity: Identity {
                email: self.email,
                workspace: self.workspace,
                fingerprint: self.fingerprint,
            },
            external: None,
            credits: None,
        }
    }
}

pub struct HostReport {
    pub name: String,
    pub accounts: Result<Vec<SharedAccount>, String>,
}

#[derive(Default, Deserialize)]
struct DesktopLinks {
    #[serde(default)]
    project_links: Option<ProjectLinks>,
}
#[derive(Deserialize)]
struct ProjectLinks {
    hosts: Vec<SavedHost>,
}

async fn saved(path: &Path) -> io::Result<Vec<SavedHost>> {
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || {
        let file = match std::fs::File::open(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
            other => other?,
        };
        let settings: DesktopLinks = serde_json::from_reader(file)?;
        Ok(settings
            .project_links
            .into_iter()
            .flat_map(|links| links.hosts)
            .filter(|h| h.usage)
            .collect())
    })
    .await
    .map_err(io::Error::other)?
}

async fn fetch(
    host: &SavedHost,
    tunnels: Option<&RemoteHostSnapshot>,
) -> Result<Vec<SharedAccount>, String> {
    let (url, tunnel) = match &host.destination {
        Destination::Https(url) => (connection::endpoint(url, false)?, None),
        Destination::Ssh { .. } => {
            let socket = tunnels.and_then(|s| host.destination.socket(s));
            (
                connection::endpoint("", true)?,
                Some((host.name.as_str(), socket)),
            )
        }
    };
    let client = connection::client(tunnel)?;
    let id = host.credential;
    let destination = host.destination.clone();
    let token = tokio::task::spawn_blocking(move || connection::project_token(id, &destination))
        .await
        .map_err(|_| "Could not open the environment credential.")??;
    read(client, url, token.as_str()).await
}

pub async fn read(
    client: reqwest::Client,
    mut url: reqwest::Url,
    token: &str,
) -> Result<Vec<SharedAccount>, String> {
    url.set_path("/api/projects/usage");
    let mut response = client
        .get(url)
        .bearer_auth(token)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|_| "Could not reach the environment.")?;
    match response.status() {
        reqwest::StatusCode::OK => (),
        reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::UNAUTHORIZED => {
            return Err("Usage access was denied. Link a current grant in Environments.".into());
        }
        _ => return Err("The environment could not report its usage limits.".into()),
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "The environment’s usage response was interrupted.")?
    {
        if body.len() + chunk.len() > 1024 * 1024 {
            return Err("The environment’s usage response is too large.".into());
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body)
        .map_err(|_| "The environment returned unreadable usage limits.".into())
}

pub async fn reports(
    path: Option<PathBuf>,
    tunnels: Option<&crate::remote_hosts::RemoteHostManager>,
) -> io::Result<Vec<HostReport>> {
    let Some(path) = path else { return Ok(vec![]) };
    let hosts = saved(&path).await?;
    if hosts.is_empty() {
        return Ok(vec![]);
    }
    let tunnels = if let Some(manager) = tunnels.filter(|_| {
        hosts
            .iter()
            .any(|h| matches!(h.destination, Destination::Ssh { .. }))
    }) {
        Some(manager.snapshot().await)
    } else {
        None
    };
    let tunnels = tunnels.as_ref();
    let results = stream::iter(hosts.into_iter().map(move |host| async move {
        let accounts = fetch(&host, tunnels).await;
        (host, accounts)
    }))
    .buffered(4)
    .collect::<Vec<_>>()
    .await;
    let current = saved(&path).await?;
    Ok(results
        .into_iter()
        .filter(|(h, _)| {
            current
                .iter()
                .any(|c| c.credential == h.credential && c.destination == h.destination)
        })
        .map(|(h, accounts)| HostReport {
            name: h.name,
            accounts,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::{
        matchers::{header, method, path},
        Mock, MockServer, ResponseTemplate,
    };

    #[cfg(unix)]
    #[tokio::test]
    async fn linked_usage_ignores_unshared_and_legacy_links_and_names_disconnected_hosts() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("desktop.json");
        let id = uuid::Uuid::new_v4();
        let legacy = json!({"name":"Legacy","credential":id,"destination":{"Https":"https://legacy.invalid"}});
        let mut disabled = legacy.clone();
        disabled["usage"] = false.into();
        let ssh = json!({"name":"Studio","credential":id,"usage":true,
            "destination":{"Ssh":{"profile":id,"host":"studio","ssh_port":22,"remote_port":45000}}});
        std::fs::write(
            &file,
            json!({"project_links":{"hosts":[legacy,disabled,ssh]}}).to_string(),
        )
        .unwrap();
        let reports = reports(Some(file), None).await.unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].name, "Studio");
        assert_eq!(
            reports[0].accounts.as_ref().err().unwrap(),
            "Reconnect Studio in Environments."
        );
    }

    #[tokio::test]
    async fn linked_usage_rejects_oversized_responses_and_discards_remote_error_bodies() {
        let server = MockServer::start().await;
        for (status, body, expected) in [
            (200, " ".repeat(1024 * 1024 + 1), "too large"),
            (
                403,
                "private remote diagnostic".into(),
                "Usage access was denied",
            ),
            (200, "not a quota report".into(), "unreadable"),
        ] {
            let mock = Mock::given(method("GET"))
                .and(path("/api/projects/usage"))
                .and(header("authorization", "Bearer fixture"))
                .respond_with(ResponseTemplate::new(status).set_body_string(body))
                .expect(1)
                .mount_as_scoped(&server)
                .await;
            let error = read(
                connection::client(None).unwrap(),
                server.uri().parse().unwrap(),
                "fixture",
            )
            .await
            .err()
            .expect("invalid reports fail");
            assert!(error.contains(expected));
            assert!(!error.contains("private remote diagnostic"));
            drop(mock);
        }
    }
}
