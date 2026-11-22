use super::credits::ResetCredits;
use super::{credits, normalize_url, Hub};
use bruhs_models::agent_extensions::QuotaWindow;
use futures_util::{stream, StreamExt};
use reqwest::{Client, RequestBuilder};
use serde_json::{json, Value};
use std::{io, time::Duration};
use url::Url;

pub struct AccountUsage {
    pub zero_spend_cap: bool,
    pub external: Option<super::ExternalUsage>,
    pub plan: Option<String>,
    pub(super) identity: super::grouping::Identity,
    pub(super) checked_at: i64,
    pub id: String,
    pub label: String,
    pub credits: Option<ResetCredits>,
    pub provider: &'static str,
    pub windows: io::Result<Vec<QuotaWindow>>,
}

/// Read local provider limits, workspace usage pages and explicit provider errors.
/// Journal observations stay local because they carry no account identity.
pub async fn device_accounts() -> Vec<AccountUsage> {
    use bruhs_models::agent::AgentProvider;
    let read = |provider: AgentProvider, label: &'static str| async move {
        let observation = crate::agent_extensions::quota::provider_observation(provider)
            .await
            .ok()?;
        if observation.windows.is_empty()
            && observation.email.is_none()
            && observation.workspace.is_none()
        {
            return None;
        }
        Some(AccountUsage {
            zero_spend_cap: observation.zero_spend_cap,
            external: None,
            id: provider.as_str().to_owned(),
            label: observation
                .email
                .clone()
                .unwrap_or_else(|| label.to_owned()),
            provider: label,
            plan: observation.plan,
            checked_at: observation.checked_at,
            identity: super::grouping::Identity {
                email: observation.email,
                workspace: observation.workspace,
                fingerprint: None,
            },
            credits: None,
            windows: Ok(observation.windows),
        })
    };
    let (opencode, cursor, grok, antigravity, codex, claude) = tokio::join!(
        super::opencode::read(),
        super::cursor::read(),
        super::grok::read(),
        super::antigravity::read(),
        read(AgentProvider::Codex, "Codex"),
        read(AgentProvider::Claude, "Claude")
    );
    [codex, claude, opencode, cursor, grok, antigravity]
        .into_iter()
        .flatten()
        .collect()
}
pub(super) fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

pub(super) async fn management_response(request: RequestBuilder) -> io::Result<reqwest::Response> {
    let response = request.send().await.map_err(|e| {
        io::Error::other(if e.is_timeout() {
            "The hub did not answer within 15 seconds."
        } else {
            "Could not connect to the hub."
        })
    })?;
    if !response.status().is_success() {
        return Err(io::Error::other(format!(
            "The hub answered with HTTP {}.",
            response.status().as_u16()
        )));
    }
    Ok(response)
}

async fn response_json(request: RequestBuilder) -> io::Result<Value> {
    let mut response = management_response(request).await?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| io::Error::other("The hub response was interrupted."))?
    {
        if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
            return Err(io::Error::other("The hub response was too large."));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| io::Error::other("The hub returned invalid JSON."))
}

pub async fn report(hub: &Hub) -> io::Result<Vec<AccountUsage>> {
    let key = hub.checked_key().await?;
    fetch(normalize_url(&hub.url)?, &key).await
}
pub(super) fn client() -> io::Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(io::Error::other)
}
pub(super) async fn accounts(
    client: &Client,
    management: &Url,
    key: &str,
) -> io::Result<Vec<Value>> {
    let list = response_json(
        client
            .get(management.join("auth-files").map_err(io::Error::other)?)
            .bearer_auth(key),
    )
    .await?;
    #[derive(serde::Deserialize)]
    struct AccountList {
        files: Vec<Value>,
    }
    serde_json::from_value::<AccountList>(list)
        .map(|list| list.files)
        .map_err(|_| io::Error::other("The hub returned an invalid account list."))
}
async fn fetch(base: Url, key: &str) -> io::Result<Vec<AccountUsage>> {
    let client = client()?;
    let management = base.join("v0/management/").map_err(io::Error::other)?;
    let files = accounts(&client, &management, key).await?;
    let endpoint = management.join("api-call").map_err(io::Error::other)?;
    let accounts = files
        .iter()
        .filter_map(|file| {
            if file["disabled"] == true {
                return None;
            }
            let provider = match file["provider"].as_str()? {
                "codex" => "Codex",
                "claude" => "Claude",
                "xai" => "Grok",
                "antigravity" => "Antigravity",
                _ => return None,
            };
            let (client, endpoint) = (&client, &endpoint);
            Some(async move {
                let checked_at = chrono::Utc::now().timestamp_millis();
                let label = file["email"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .or(file["id"].as_str())
                    .unwrap_or(provider)
                    .to_owned();
                let (plan, windows, zero_spend_cap) =
                    match probe(client, endpoint, key, file, provider, &label).await {
                        Ok(reading) => (reading.plan, Ok(reading.windows), reading.zero_spend_cap),
                        Err(error) => (None, Err(error), false),
                    };
                let credits = if provider == "Codex" && windows.is_ok() {
                    credits::available(client, endpoint, key, file).await.ok()
                } else {
                    None
                };
                let mut identity = super::grouping::Identity {
                    email: text(&file["email"]),
                    workspace: text(&file["id_token"]["chatgpt_account_id"]),
                    fingerprint: None,
                };
                if provider == "Antigravity" && windows.as_ref().is_ok_and(|w| !w.is_empty()) {
                    if let Ok(info) = account_call(
                        client,
                        endpoint,
                        key,
                        file,
                        "https://www.googleapis.com/oauth2/v2/userinfo",
                        json!({"Authorization":"Bearer $TOKEN$"}),
                        None,
                    )
                    .await
                    {
                        if let Some(id) = text(&info["id"]) {
                            identity = super::grouping::Identity {
                                fingerprint: Some(super::antigravity::google_account(&id)),
                                ..Default::default()
                            };
                        }
                    }
                }
                AccountUsage {
                    zero_spend_cap,
                    external: None,
                    plan,
                    identity,
                    checked_at,
                    id: file["id"].as_str().unwrap_or_default().into(),
                    credits,
                    label,
                    provider,
                    windows,
                }
            })
        })
        .collect::<Vec<_>>();
    Ok(stream::iter(accounts).buffered(4).collect().await)
}

struct HubReading {
    plan: Option<String>,
    windows: Vec<QuotaWindow>,
    zero_spend_cap: bool,
}

async fn probe(
    client: &Client,
    endpoint: &Url,
    key: &str,
    file: &Value,
    provider: &'static str,
    account: &str,
) -> io::Result<HubReading> {
    let token = "Bearer $TOKEN$";
    let (url, header, data) = match provider {
        "Codex" => (
            "https://chatgpt.com/backend-api/wham/usage",
            codex_headers(file),
            None,
        ),
        "Claude" => (
            "https://api.anthropic.com/api/oauth/usage",
            json!({"Authorization":token, "anthropic-beta":"oauth-2025-04-20","User-Agent":"claude-code/2.1.273"}),
            None,
        ),
        "Grok" => (
            super::grok::BILLING_URL,
            json!({"Authorization":token,"x-xai-token-auth":"xai-grok-cli","x-grok-client-version":"1.0.77","User-Agent":"grok-cli/1.0.77"}),
            None,
        ),
        _ => {
            let project = file["project_id"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| {
                    io::Error::other("The hub did not report an Antigravity project.")
                })?;
            let agent = super::antigravity::user_agent();
            (
                "https://daily-cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary",
                json!({"Authorization":token,"Content-Type":"application/json","User-Agent":agent}),
                Some(json!({"project":project})),
            )
        }
    };
    let requested_at = chrono::Utc::now().timestamp_millis();
    let mut usage = account_call(client, endpoint, key, file, url, header, data)
        .await
        .map_err(|_| io::Error::other("The hub could not read this account’s usage."))?;
    let plan = (provider == "Codex")
        .then(|| {
            text(&usage["plan_type"])
                .or_else(|| text(&file["id_token"]["plan_type"]))
                .or_else(|| text(&file["id_token"]["chatgpt_plan_type"]))
        })
        .flatten();
    let windows = if provider == "Grok" {
        // Some hubs serialize observed reset dates as epoch seconds.
        if usage["config"]["creditUsagePercent"].is_number() {
            for key in ["start", "end"] {
                if let Some(date) = usage["config"]["currentPeriod"][key]
                    .as_i64()
                    .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
                {
                    usage["config"]["currentPeriod"][key] = json!(date.to_rfc3339());
                }
            }
        }
        super::grok::windows(&usage, account, chrono::Utc::now().timestamp())?
    } else if provider == "Antigravity" {
        super::antigravity::windows::summary(&usage, account, requested_at)
    } else {
        windows(&usage, provider, account, plan.as_deref())
    };
    Ok(HubReading {
        plan,
        windows,
        zero_spend_cap: provider == "Claude"
            && crate::agent_extensions::quota::claude_zero_spend_cap(&usage),
    })
}

pub(super) fn codex_headers(file: &Value) -> Value {
    let mut header = json!({"Authorization":"Bearer $TOKEN$", "Content-Type":"application/json","Originator":"Codex Desktop","OpenAI-Beta":"codex-1"});
    if let Some(id) = file["id_token"]["chatgpt_account_id"].as_str() {
        header["Chatgpt-Account-Id"] = json!(id);
    }
    header
}
pub(super) async fn account_call(
    client: &Client,
    endpoint: &Url,
    key: &str,
    file: &Value,
    url: &'static str,
    header: Value,
    data: Option<Value>,
) -> io::Result<Value> {
    let index = file["auth_index"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| io::Error::other("The hub did not identify this account."))?;
    let mut body = json!({"auth_index":index,"method": if data.is_some() {"POST"} else {"GET"}, "url":url,"header":header});
    if let Some(data) = data {
        body["data"] = json!(data.to_string());
    }
    let response =
        response_json(client.post(endpoint.clone()).bearer_auth(key).json(&body)).await?;
    if !response["status_code"]
        .as_u64()
        .is_some_and(|n| (200..300).contains(&n))
    {
        return Err(io::Error::other(
            "The hub could not read this account’s usage.",
        ));
    }
    serde_json::from_str(response["body"].as_str().unwrap_or(""))
        .map_err(|_| io::Error::other("The provider returned an invalid response."))
}

fn timestamp(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| {
        chrono::DateTime::parse_from_rfc3339(v.as_str()?)
            .ok()
            .map(|t| t.timestamp())
    })
}
fn windows(v: &Value, provider: &str, account: &str, plan: Option<&str>) -> Vec<QuotaWindow> {
    let now = chrono::Utc::now().timestamp();
    let mut rows = Vec::new();
    let mut add = |name: &str, used: Option<f64>, resets_at, duration_minutes| {
        if let Some(used) = used.filter(|n| n.is_finite()) {
            rows.push(QuotaWindow {
                window_id: None,
                name: name.into(),
                account: account.into(),
                provider: provider.into(),
                used_percent: used.clamp(0., 100.),
                duration_minutes,
                observed_at: now,
                resets_at,
            });
        }
    };
    match provider {
        "Codex" => {
            for (id, fallback) in [
                ("primary_window", "Primary"),
                ("secondary_window", "Secondary"),
            ] {
                let w = &v["rate_limit"][id];
                let mins = w["limit_window_seconds"]
                    .as_u64()
                    .and_then(|s| u32::try_from(s / 60).ok());
                let mins = mins.filter(|n| *n > 0).unwrap_or(match (id, plan) {
                    ("secondary_window", _) => 10080,
                    (_, Some("free" | "go")) => 43200,
                    _ => 300,
                });
                let label = match mins {
                    300 => "5-hour",
                    10080 => "Weekly",
                    43200 => "Monthly",
                    _ => fallback,
                };
                add(
                    label,
                    w["used_percent"].as_f64(),
                    timestamp(&w["reset_at"]),
                    Some(mins),
                );
            }
        }
        "Claude" => {
            let scoped = v["limits"].as_array().into_iter().flatten().filter(|w| w["kind"] == "weekly_scoped").map(|w|json!({"display_name":w["scope"]["model"]["display_name"],"utilization":w["percent"],"resets_at":w["resets_at"]})).collect::<Vec<_>>();
            let limits = json!({"rate_limits":{"five_hour":v["five_hour"],"seven_day":v["seven_day"],"model_scoped":scoped,"spend":v["spend"],"extra_usage":v["extra_usage"]}});
            rows = crate::agent_extensions::quota::claude_windows(&limits, now).unwrap_or_default();
            for w in &mut rows {
                w.account = account.into();
                w.name = match w.name.as_str() {
                    "five_hour" => "5-hour".into(),
                    "seven_day" => "Weekly".into(),
                    "monthly_spend" => "Monthly spend".into(),
                    other => other.replace("seven_day_", "Weekly · "),
                };
            }
        }
        _ => {}
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        matchers::{body_partial_json, header, method, path},
        Mock, MockServer, ResponseTemplate,
    };

    #[test]
    fn hub_urls_preserve_prefixes_and_normalize_the_management_page() {
        for value in [
            "https://example.test/team",
            "https://example.test/team/management.html/",
        ] {
            assert_eq!(
                normalize_url(value).unwrap().as_str(),
                "https://example.test/team/"
            );
        }
        for value in [
            "file:///tmp/report",
            "https://key@example.test",
            "https://example.test/?key=secret",
        ] {
            assert!(normalize_url(value).is_err());
        }
    }

    #[tokio::test]
    async fn hub_reads_enabled_accounts_and_keeps_individual_errors_local() {
        let server = MockServer::start().await;
        let files = json!({"files":[
            {"provider":"codex","auth_index":"a","id":"account-a","email":"fixture@example.test","id_token":{"chatgpt_account_id":"workspace"}},
            {"provider":"claude","auth_index":"b","id":"account-b"},
            {"provider":"xai","auth_index":"c","id":"account-c"},
            {"provider":"antigravity","auth_index":"d","id":"account-d","project_id":"demo"},
            {"provider":"codex","auth_index":"e","id":"no-access"},
            {"provider":"codex","auth_index":"f","id":"disabled","disabled":true},
            {"provider":"other","auth_index":"g","id":"unsupported"}
        ]});
        Mock::given(method("GET"))
            .and(path("/team/v0/management/auth-files"))
            .and(header("authorization", "Bearer fixture"))
            .respond_with(ResponseTemplate::new(200).set_body_json(files))
            .expect(1)
            .mount(&server)
            .await;
        let provider_cases = [
            (
                "a",
                json!({"url":"https://chatgpt.com/backend-api/wham/usage","header":{"Chatgpt-Account-Id":"workspace"}}),
                json!({"rate_limit":{"primary_window":{"used_percent":25,"limit_window_seconds":18000,"reset_at":1900000000}}}),
            ),
            (
                "b",
                json!({"url":"https://api.anthropic.com/api/oauth/usage"}),
                json!({"five_hour":{"utilization":20},"limits":[{"kind":"weekly_scoped","scope":{"model":{"display_name":"Sonnet"}},"percent":35}]}),
            ),
            (
                "c",
                json!({"url":"https://cli-chat-proxy.grok.com/v1/billing?format=credits"}),
                json!({"config":{"creditUsagePercent":40,"currentPeriod":{"type":"USAGE_PERIOD_TYPE_MONTHLY","end":"2030-02-01T00:00:00Z"}}}),
            ),
            (
                "d",
                json!({"url":"https://daily-cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary","method":"POST","data":"{\"project\":\"demo\"}"}),
                json!({"groups":[{"buckets":[{"bucketId":"gemini-fixture","window":"weekly","remainingFraction":0.25}]}]}),
            ),
            (
                "e",
                json!({}),
                json!({"private":"must never reach display"}),
            ),
        ];
        for (index, mut expected, usage) in provider_cases {
            expected["auth_index"] = json!(index);
            expected["header"]["Authorization"] = json!("Bearer $TOKEN$");
            Mock::given(method("POST"))
                .and(path("/team/v0/management/api-call"))
                .and(header("authorization", "Bearer fixture"))
                .and(body_partial_json(expected))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    json!({"status_code":if index=="e"{403}else{200},"body":usage.to_string()}),
                ))
                .expect(1)
                .mount(&server)
                .await;
        }
        Mock::given(method("POST")).and(path("/team/v0/management/api-call")).and(body_partial_json(json!({"url":"https://www.googleapis.com/oauth2/v2/userinfo","auth_index":"d","method":"GET","header":{"Authorization":"Bearer $TOKEN$"}})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status_code":200,"body":json!({"id":"google-fixture","email":"private@example.test"}).to_string()}))).expect(1).mount(&server).await;
        let accounts = fetch(
            normalize_url(&format!("{}/team", server.uri())).unwrap(),
            "fixture",
        )
        .await
        .unwrap();
        assert_eq!(
            accounts.iter().map(|a| a.provider).collect::<Vec<_>>(),
            ["Codex", "Claude", "Grok", "Antigravity", "Codex"]
        );
        for (account, pct) in accounts.iter().zip([25., 20., 40., 75.]) {
            assert_eq!(account.windows.as_ref().unwrap()[0].used_percent, pct);
        }
        assert_eq!(accounts[0].label, "fixture@example.test");
        assert_eq!(accounts[0].identity.workspace.as_deref(), Some("workspace"));
        assert_eq!(
            accounts[3].identity.fingerprint.as_deref(),
            Some("5659d52fa56cbfde4bb95ae8ca043caef67edbf974296db85c5580a11cd9eb6c")
        );
        assert!(accounts[3].identity.email.is_none());
        assert_eq!(
            accounts[1].windows.as_ref().unwrap()[1].name,
            "Weekly · Sonnet"
        );
        assert_eq!(
            accounts[4].windows.as_ref().unwrap_err().to_string(),
            "The hub could not read this account’s usage."
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 8);
    }

    #[test]
    fn codex_plan_fills_only_missing_window_durations() {
        let payload = json!({"rate_limit":{
            "primary_window":{"used_percent":42},
            "secondary_window":{"limit_window_seconds":18000,"used_percent":17}
        }});
        for (plan, expected) in [(Some("free"), 43200), (Some("plus"), 300), (None, 300)] {
            let windows = windows(&payload, "Codex", "fixture", plan);
            assert_eq!(windows[0].duration_minutes, Some(expected));
            assert_eq!(windows[1].duration_minutes, Some(300));
        }
    }

    #[tokio::test]
    async fn hub_never_follows_a_management_redirect() {
        let server = MockServer::start().await;
        let other = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(307).insert_header("location", other.uri()))
            .mount(&server)
            .await;
        assert!(fetch(normalize_url(&server.uri()).unwrap(), "fixture")
            .await
            .is_err());
        assert!(other.received_requests().await.unwrap().is_empty());
    }
}
