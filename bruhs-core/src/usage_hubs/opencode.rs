//! Read the local OpenCode Console workspace through the installed CLI.
mod legacy;
use super::{grouping::Identity, AccountUsage};
use bruhs_models::{agent::AgentProvider, agent_extensions::QuotaWindow};
use reqwest::{Client, StatusCode};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{io, path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::Command,
    time::timeout,
};
use url::Url;
use uuid::Uuid;

#[derive(Clone, Serialize)]
pub struct ExternalUsage {
    pub label: String,
    pub url: String,
}

fn error() -> io::Error {
    io::Error::other("OpenCode could not read workspace usage.")
}

pub(crate) async fn read() -> Option<AccountUsage> {
    let provider = AgentProvider::OpenCode;
    if crate::agent_extensions::availability::require_enabled(provider).is_err() {
        return None;
    }
    let program = crate::agent::opencode_executable().ok()?;
    let result = timeout(Duration::from_secs(15), async {
        let dir = tempfile::tempdir().map_err(|_| error())?;
        let mut version = command(&program, dir.path());
        let mut child = match version.arg("--version").spawn() {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            other => other.map_err(|_| error())?,
        };
        #[cfg(unix)]
        let mut version_group = crate::agent::ProcessGroup::of(&child);
        let mut text = String::new();
        child
            .stdout
            .take()
            .ok_or_else(error)?
            .take(4096)
            .read_to_string(&mut text)
            .await
            .map_err(|_| error())?;
        let status = child.wait().await.map_err(|_| error())?;
        #[cfg(unix)]
        version_group.release();
        if !status.success() {
            return Err(error());
        }
        if text.trim().trim_start_matches('v').split('.').next() == Some("1") {
            return legacy::read().await;
        }
        match server_usage(&program, dir.path()).await? {
            ServerReply::MissingRoute => legacy::read().await,
            ServerReply::Reported(Some(workspace)) => Ok(Some(workspace.account())),
            ServerReply::Reported(None) => Ok(Some(account(
                "OpenCode".into(),
                None,
                None,
                Err(io::Error::other(
                    "No OpenCode Console workspace is signed in.",
                )),
            ))),
        }
    })
    .await
    .unwrap_or_else(|_| Err(error()));
    result.unwrap_or_else(|_| Some(account("OpenCode".into(), None, None, Err(error()))))
}

fn command(program: &Path, dir: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    crate::connections::protect_command(&mut command);
    command
}

enum ServerReply {
    MissingRoute,
    Reported(Option<Console>),
}
async fn server_usage(program: &Path, dir: &Path) -> io::Result<ServerReply> {
    let password = Uuid::new_v4().to_string();
    let mut cmd = command(program, dir);
    let port = std::net::TcpListener::bind(("127.0.0.1", 0))?
        .local_addr()?
        .port();
    cmd.args(["serve", "--hostname=127.0.0.1", &format!("--port={port}")])
        .env_remove("OPENCODE_SERVER_PASSWORD")
        .env("OPENCODE_PASSWORD", &password);
    let mut child = cmd.spawn().map_err(|_| error())?;
    #[cfg(unix)]
    let _group = crate::agent::ProcessGroup::of(&child);
    let mut stdout = BufReader::new(child.stdout.take().ok_or_else(error)?.take(64 * 1024)).lines();
    let base = loop {
        let line = stdout
            .next_line()
            .await
            .map_err(|_| error())?
            .ok_or_else(error)?;
        if let Some((_, address)) = line.to_lowercase().split_once("server listening on ") {
            let url = Url::parse(address.trim()).map_err(|_| error())?;
            if url.scheme() != "http"
                || url.host_str() != Some("127.0.0.1")
                || url.port() != Some(port)
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err(error());
            }
            break url;
        }
    };
    let client = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|_| error())?;
    let response = client
        .get(
            base.join("/api/experimental/usage/limits")
                .map_err(|_| error())?,
        )
        .basic_auth("opencode", Some(password))
        .query(&[("location[directory]", dir)])
        .send()
        .await
        .map_err(|_| error())?;
    let status = response.status();
    let html = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|s| {
            s.split(';')
                .next()
                .is_some_and(|s| s.trim().eq_ignore_ascii_case("text/html"))
        });
    if matches!(
        status,
        StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
    ) || (status == StatusCode::OK && html)
    {
        return Ok(ServerReply::MissingRoute);
    }
    let data: serde_json::Value = body(response).await?;
    let field = data.get("data").ok_or_else(error)?;
    Ok(ServerReply::Reported(
        serde_json::from_value(field.clone()).map_err(|_| error())?,
    ))
}
async fn body<T: DeserializeOwned>(mut response: reqwest::Response) -> io::Result<T> {
    if !response.status().is_success() {
        return Err(error());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| error())? {
        if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
            return Err(error());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| error())
}

#[derive(Deserialize)]
struct Console {
    workspace: Workspace,
    go: Option<Go>,
    budgets: Vec<Budget>,
}
#[derive(Deserialize)]
struct Workspace {
    id: String,
    name: Option<String>,
    #[serde(rename = "usageURL")]
    url: String,
}
#[derive(Deserialize)]
struct Go {
    rolling: Meter,
    weekly: Meter,
    monthly: Meter,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Meter {
    percent: f64,
    limited: bool,
    resets_at: i64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Budget {
    id: String,
    limit: f64,
    spent: f64,
    scope: String,
    per: String,
    coverage: String,
    models: Vec<Model>,
    shared: bool,
    resets_at: i64,
}
#[derive(Deserialize)]
struct Model {
    #[serde(rename = "providerID")]
    provider_id: String,
    #[serde(rename = "modelID")]
    model_id: String,
    name: String,
    spent: Option<f64>,
}

fn window(
    label: &str,
    window_id: Option<String>,
    name: String,
    used: f64,
    duration: Option<u32>,
    reset: i64,
) -> QuotaWindow {
    QuotaWindow {
        window_id,
        provider: "OpenCode".into(),
        account: label.into(),
        name,
        used_percent: used.clamp(0., 100.),
        duration_minutes: duration,
        resets_at: Some(reset),
        observed_at: chrono::Utc::now().timestamp(),
    }
}
fn account(
    label: String,
    external: Option<ExternalUsage>,
    identity: Option<String>,
    windows: io::Result<Vec<QuotaWindow>>,
) -> AccountUsage {
    AccountUsage {
        zero_spend_cap: false,
        id: "opencode".into(),
        label,
        provider: "OpenCode",
        plan: None,
        identity: Identity {
            fingerprint: identity,
            ..Default::default()
        },
        checked_at: chrono::Utc::now().timestamp_millis(),
        credits: None,
        windows,
        external,
    }
}
impl Console {
    fn account(self) -> AccountUsage {
        let workspace = self.workspace;
        let label = workspace
            .name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&workspace.id)
            .to_owned();
        let external = Url::parse(&workspace.url)
            .ok()
            .filter(|u| {
                matches!(u.scheme(), "http" | "https")
                    && u.host().is_some()
                    && u.username().is_empty()
                    && u.password().is_none()
            })
            .map(|u| ExternalUsage {
                label: label.clone(),
                url: u.to_string(),
            });
        let mut windows = Vec::new();
        if let Some(go) = self.go {
            for (name, meter, duration) in [
                ("Session", go.rolling, Some(300)),
                ("Weekly", go.weekly, Some(10080)),
                ("Monthly", go.monthly, None),
            ] {
                windows.push(window(
                    &label,
                    None,
                    format!("Go · {name}"),
                    if meter.limited { 100. } else { meter.percent },
                    duration,
                    meter.resets_at.div_euclid(1000),
                ));
            }
        }
        for budget in self.budgets {
            let scope = if budget.shared || budget.scope == "combined" {
                "Shared"
            } else {
                "Individual"
            };
            let percent = |spent| {
                if budget.limit <= 0. {
                    100.
                } else {
                    100. * spent / budget.limit
                }
            };
            let mut push = |id: String, covered: &str, spent| {
                windows.push(window(
                    &label,
                    Some(id),
                    format!("Console · {scope} · {covered}"),
                    percent(spent),
                    None,
                    budget.resets_at.div_euclid(1000),
                ))
            };
            if budget.per == "model"
                && budget.coverage == "models"
                && !budget.models.is_empty()
                && budget.models.iter().all(|m| m.spent.is_some())
            {
                for m in budget.models {
                    push(
                        format!("budget_{}_{}_{}", budget.id, m.provider_id, m.model_id),
                        &m.name,
                        m.spent.unwrap_or(budget.spent),
                    );
                }
            } else {
                let mut covered = if budget.coverage == "all" {
                    "All spend".to_owned()
                } else if budget.models.is_empty() {
                    "Models".to_owned()
                } else {
                    budget
                        .models
                        .iter()
                        .map(|m| m.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                if budget.per == "model" {
                    covered.push_str(" · Most used model");
                }
                push(format!("budget_{}", budget.id), &covered, budget.spent);
            }
        }
        account(
            label,
            external,
            Some(format!("workspace:{}", workspace.id)),
            Ok(windows),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn workspace_page_is_independent_of_the_quota_windows() {
        let page = |url: &str| {
            serde_json::from_value::<Console>(json!({
                "workspace":{"id":"demo", "name":" Team ", "usageURL":url},
                "budgets":[]
            }))
            .unwrap()
            .account()
        };
        let report = page("https://console.example.test/usage");
        assert!(report.windows.unwrap().is_empty());
        assert_eq!(report.external.unwrap().label, "Team");
        for invalid in [
            "javascript:alert(1)",
            "https://key@example.test/usage",
            "not a URL",
        ] {
            assert!(page(invalid).external.is_none());
        }
    }

    #[test]
    fn shared_budgets_with_the_same_label_keep_distinct_window_ids() {
        let budget = |id: &str, spent: f64| {
            json!({
                "id":id, "limit":10.0, "spent":spent, "scope":"combined", "per":"set",
                "coverage":"all","models":[], "shared":true, "resetsAt":2000000000000i64
            })
        };
        let report = serde_json::from_value::<Console>(json!({
            "workspace":{"id":"demo","usageURL":"https://console.example.test"},
            "budgets":[budget("first",2.),budget("second",15.)]
        }))
        .unwrap()
        .account();
        let windows = report.windows.unwrap();
        assert_eq!(windows[0].name, windows[1].name);
        assert_eq!(windows[0].window_id.as_deref(), Some("budget_first"));
        assert_eq!(windows[1].window_id.as_deref(), Some("budget_second"));
        assert_eq!(windows[0].used_percent, 20.);
        assert_eq!(windows[1].used_percent, 100.);
        assert_eq!(windows[0].resets_at, Some(2000000000));
    }
}
