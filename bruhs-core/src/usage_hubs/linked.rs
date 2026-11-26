//! Subscription readings shared by a paired desktop. Only the owning machine's providers are queried.
use super::{grouping::Identity, AccountUsage};
use crate::linked_projects::connection::{self, Destination, SavedHost};
use bruhs_models::remote_hosts::RemoteHostSnapshot;
use futures_util::{stream, StreamExt};
use std::{
    collections::BTreeMap,
    future::Future,
    io,
    path::{Path, PathBuf},
};

use bruhs_models::{
    agent::AgentProvider,
    agent_extensions::{
        QuotaWindow, SharedUsageHistory, UsageHistoryPeriod, UsageModelPricing,
        UsageModelPricingChange,
    },
};
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
            .collect())
    })
    .await
    .map_err(io::Error::other)?
}

async fn session(
    host: &SavedHost,
    tunnels: Option<&RemoteHostSnapshot>,
) -> Result<(reqwest::Client, reqwest::Url, zeroize::Zeroizing<String>), String> {
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
    Ok((client, url, token))
}

async fn fetch(
    host: &SavedHost,
    tunnels: Option<&RemoteHostSnapshot>,
) -> Result<Vec<SharedAccount>, String> {
    let (client, url, token) = session(host, tunnels).await?;
    read(client, url, &token).await
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
    let hosts: Vec<_> = saved(&path)
        .await?
        .into_iter()
        .filter(|h| h.usage)
        .collect();
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
                .any(|c| c.usage && c.credential == h.credential && c.destination == h.destination)
        })
        .map(|(h, accounts)| HostReport {
            name: h.name,
            accounts,
        })
        .collect())
}

#[derive(Default)]
pub struct HistoryReports {
    pub environments: Vec<String>,
    pub reports: Vec<(String, Result<SharedUsageHistory, String>)>,
}

pub async fn history_reports(
    path: Option<PathBuf>,
    environment: Option<&str>,
    period: UsageHistoryPeriod,
    tunnels: impl Future<Output = Option<RemoteHostSnapshot>>,
) -> io::Result<HistoryReports> {
    let Some(path) = path else {
        return Ok(HistoryReports::default());
    };
    let selected: Vec<_> = saved(&path)
        .await?
        .into_iter()
        .filter(|h| h.usage_history && environment.is_none_or(|name| name == h.name))
        .collect();
    let tunnels = if selected
        .iter()
        .any(|h| matches!(h.destination, Destination::Ssh { .. }))
    {
        tunnels.await
    } else {
        None
    };
    let tunnels = tunnels.as_ref();
    let results = stream::iter(selected.into_iter().map(|host| async move {
        let report = fetch_history(&host, tunnels, period).await;
        (host, report)
    }))
    .buffered(4)
    .collect::<Vec<_>>()
    .await;
    let current = saved(&path).await?;
    Ok(HistoryReports {
        environments: current
            .iter()
            .filter(|h| h.usage_history)
            .map(|h| h.name.clone())
            .collect(),
        reports: results
            .into_iter()
            .filter_map(|(host, report)| {
                current
                    .iter()
                    .find(|h| {
                        h.usage_history
                            && h.credential == host.credential
                            && h.destination == host.destination
                    })
                    .map(|h| (h.name.clone(), report))
            })
            .collect(),
    })
}

async fn fetch_history(
    host: &SavedHost,
    tunnels: Option<&RemoteHostSnapshot>,
    period: UsageHistoryPeriod,
) -> Result<SharedUsageHistory, String> {
    let (client, mut url, token) = session(host, tunnels).await?;
    url.set_path("/api/projects/usage/history");
    let mut response = client
        .get(url)
        .query(&period)
        .bearer_auth(token.as_str())
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|_| "Could not reach the environment.")?;
    match response.status() {
        reqwest::StatusCode::OK => (),
        reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::UNAUTHORIZED => {
            return Err(
                "Cost and token history access was denied. Link a current grant in Environments."
                    .into(),
            );
        }
        reqwest::StatusCode::BAD_REQUEST if period.last_24h_ending_at.is_some() => {
            return Err("Hourly history is unavailable. Check that the environment is up to date and its clock is correct.".into());
        }
        _ => return Err("The environment could not report cost and token history.".into()),
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "The usage history response was interrupted.")?
    {
        if body.len() + chunk.len() > 1024 * 1024 {
            return Err("The usage history response is too large.".into());
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body)
        .map_err(|_| "The environment returned unreadable usage history.".into())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PriceDestination {
    pub id: uuid::Uuid,
    pub name: String,
    destination: Destination,
}

impl PriceDestination {
    /// A Workbench request can pin the same connection without receiving its topology.
    pub fn request_key(&self) -> Result<String, serde_json::Error> {
        use sha2::{Digest, Sha256};

        let connection = serde_json::to_vec(&(self.id, &self.destination))?;
        Ok(format!("{:x}", Sha256::digest(connection)))
    }

    pub fn needs_tunnel(&self) -> bool {
        matches!(self.destination, Destination::Ssh { .. })
    }
}

pub async fn price_destinations(path: Option<&Path>) -> io::Result<Vec<PriceDestination>> {
    let Some(path) = path else {
        return Ok(Vec::new());
    };
    Ok(saved(path)
        .await?
        .into_iter()
        .filter(|host| host.usage_history && host.edit_usage_prices)
        .map(|host| PriceDestination {
            id: host.credential,
            name: host.name,
            destination: host.destination,
        })
        .collect())
}

async fn price_session(
    path: &Path,
    target: &PriceDestination,
    tunnels: Option<&RemoteHostSnapshot>,
) -> Result<(reqwest::Client, reqwest::Url, zeroize::Zeroizing<String>), String> {
    let host = saved(path)
        .await
        .map_err(|_| "Could not read linked environments.")?
        .into_iter()
        .find(|host| {
            host.credential == target.id
                && host.destination == target.destination
                && host.usage_history
                && host.edit_usage_prices
        })
        .ok_or("The environment link changed. Reopen Model prices to use the current link.")?;
    let (client, mut url, token) = session(&host, tunnels).await?;
    url.set_path("/api/projects/usage/model-prices");
    Ok((client, url, token))
}

fn price_request_error(status: reqwest::StatusCode) -> String {
    use reqwest::StatusCode;

    match status {
        StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED => {
            "Model price access was denied. Link a current grant in Environments."
        }
        StatusCode::NOT_FOUND => "Update the environment to edit model prices.",
        StatusCode::CONFLICT => "A model price changed elsewhere. Refresh its saved values.",
        StatusCode::BAD_REQUEST => "The environment rejected the model IDs, mappings or rates.",
        _ => "The environment could not process model prices.",
    }
    .into()
}

pub async fn read_model_prices(
    path: &Path,
    target: &PriceDestination,
    tunnels: Option<&RemoteHostSnapshot>,
) -> Result<BTreeMap<String, UsageModelPricing>, String> {
    let (client, url, token) = price_session(path, target, tunnels).await?;
    let mut response = client
        .get(url)
        .bearer_auth(token.as_str())
        .send()
        .await
        .map_err(|_| "Could not reach the environment.")?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(price_request_error(response.status()));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "The model price response was interrupted.")?
    {
        if body.len() + chunk.len() > 1024 * 1024 {
            return Err("The model price response is too large.".into());
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| "The environment returned unreadable prices.".into())
}

pub async fn save_model_prices(
    path: &Path,
    target: &PriceDestination,
    tunnels: Option<&RemoteHostSnapshot>,
    changes: &[UsageModelPricingChange],
) -> Result<(), String> {
    let (client, url, token) = price_session(path, target, tunnels).await?;
    let response = client
        .post(url)
        .bearer_auth(token.as_str())
        .json(changes)
        .send()
        .await
        .map_err(|_| "Could not reach the environment.")?;
    if response.status() != reqwest::StatusCode::NO_CONTENT {
        return Err(price_request_error(response.status()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::{
        matchers::{header, method, path},
        Mock, MockServer, ResponseTemplate,
    };

    #[tokio::test]
    async fn price_destinations_exclude_history_only_and_legacy_links() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("desktop.json");
        let id = uuid::Uuid::new_v4();
        let legacy = json!({"name":"Legacy","credential":uuid::Uuid::new_v4(),
            "destination":{"Https":"https://legacy.invalid"}});
        let reader = json!({"name":"Reader","credential":uuid::Uuid::new_v4(),
            "usage_history":true,"destination":{"Https":"https://reader.invalid"}});
        let writer = json!({"name":"Studio","credential":id,"usage_history":true,
            "edit_usage_prices":true,"destination":{"Https":"https://studio.invalid"}});
        std::fs::write(
            &file,
            json!({"project_links":{"hosts":[legacy,reader,writer]}}).to_string(),
        )
        .unwrap();
        let destinations = price_destinations(Some(&file)).await.unwrap();
        assert_eq!(
            destinations,
            vec![PriceDestination {
                id,
                name: "Studio".into(),
                destination: Destination::Https("https://studio.invalid".into()),
            }]
        );
        let relinked = json!({"name":"Studio","credential":id,"usage_history":true,
            "edit_usage_prices":true,"destination":{"Https":"https://replacement.invalid"}});
        std::fs::write(
            &file,
            json!({"project_links":{"hosts":[relinked]}}).to_string(),
        )
        .unwrap();
        assert_eq!(
            save_model_prices(&file, &destinations[0], None, &[])
                .await
                .unwrap_err(),
            "The environment link changed. Reopen Model prices to use the current link."
        );
    }

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
