//! Cursor dashboard percentages include bonus usage that spend-based estimates miss.
use super::AccountUsage;
use bruhs_models::{agent::AgentProvider, agent_extensions::QuotaWindow};
use reqwest::{Client, RequestBuilder};
use serde::{de::DeserializeOwned, Deserialize};
use std::{io, time::Duration};
use url::{Host, Url};
use zeroize::Zeroizing;

const DEFAULT_ENDPOINT: &str = "https://api2.cursor.sh/";
const TOKEN: &str = "CURSOR_AUTH_TOKEN";
const API_KEY: &str = "CURSOR_API_KEY";
const ENDPOINT: &str = "CURSOR_API_ENDPOINT";

fn failed() -> io::Error {
    io::Error::other("Cursor could not read usage limits.")
}

pub(crate) async fn read() -> Option<AccountUsage> {
    if crate::agent_extensions::availability::require_enabled(AgentProvider::Cursor).is_err() {
        return None;
    }
    let checked_at = chrono::Utc::now();
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let bindings = tokio::task::spawn_blocking(crate::secret_sources::load)
            .await
            .map_err(|_| failed())?
            .map_err(|_| failed())?;
        let setting = async |name: &str| -> io::Result<Option<Zeroizing<String>>> {
            Ok(bindings
                .provider_variable("cursor", name)
                .await
                .map_err(|_| io::Error::other("Could not resolve the Cursor settings in Secrets."))?
                .map(Zeroizing::new)
                .filter(|v| !v.trim().is_empty()))
        };
        let token = setting(TOKEN).await?;
        let api_key = if token.is_none() {
            setting(API_KEY).await?
        } else {
            None
        };
        if token.is_none() && api_key.is_none() {
            return Ok(None);
        }
        let configured = setting(ENDPOINT).await?;
        let endpoint = endpoint(configured.as_ref().map_or(DEFAULT_ENDPOINT, |s| s.trim()))?;
        let default = Url::parse(DEFAULT_ENDPOINT).expect("static URL");
        let client = super::report::client()?;
        let windows = fetch(
            &client,
            &endpoint,
            &default,
            token,
            api_key,
            checked_at.timestamp(),
        )
        .await?;
        Ok(Some(windows))
    })
    .await
    .unwrap_or_else(|_| Err(failed()));
    Some(AccountUsage {
        zero_spend_cap: false,
        label: "Cursor".into(),
        id: "cursor".into(),
        provider: "Cursor",
        plan: None,
        identity: Default::default(),
        checked_at: checked_at.timestamp_millis(),
        credits: None,
        external: None,
        windows: result.transpose()?,
    })
}

fn endpoint(value: &str) -> io::Result<Url> {
    let fail = || {
        io::Error::other("Check the Cursor API endpoint. Use HTTPS or a local loopback address.")
    };
    let mut base = Url::parse(value).map_err(|_| fail())?;
    let local = match base.host() {
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        Some(Host::Domain(name)) => name == "localhost",
        None => false,
    };
    if !(base.scheme() == "https" || base.scheme() == "http" && local)
        || base.host().is_none()
        || !base.username().is_empty()
        || base.password().is_some()
        || base.query().is_some()
        || base.fragment().is_some()
    {
        return Err(fail());
    }
    if !base.path().ends_with('/') {
        base.path_segments_mut().map_err(|_| fail())?.push("");
    }
    Ok(base)
}

async fn fetch(
    client: &Client,
    endpoint: &Url,
    default: &Url,
    token: Option<Zeroizing<String>>,
    api_key: Option<Zeroizing<String>>,
    observed_at: i64,
) -> io::Result<Vec<QuotaWindow>> {
    let token = match token {
        Some(token) => token,
        None => {
            if endpoint != default {
                return Err(io::Error::other(
                    "Cursor usage for an API key requires the default Cursor endpoint.",
                ));
            }
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Exchange {
                access_token: String,
            }
            let key = api_key.ok_or_else(failed)?;
            let request = client
                .post(
                    default
                        .join("auth/exchange_user_api_key")
                        .map_err(|_| failed())?,
                )
                .bearer_auth(key.trim());
            let exchange: Exchange = post(request).await?;
            let token = Zeroizing::new(exchange.access_token);
            if token.trim().is_empty() {
                return Err(failed());
            }
            token
        }
    };
    let request = client
        .post(
            endpoint
                .join("aiserver.v1.DashboardService/GetCurrentPeriodUsage")
                .map_err(|_| failed())?,
        )
        .bearer_auth(token.trim())
        .header("connect-protocol-version", "1")
        .header("x-cursor-client-type", "cli");
    let usage: Dashboard = post(request).await?;
    Ok(usage.windows(observed_at))
}

async fn post<T: DeserializeOwned>(request: RequestBuilder) -> io::Result<T> {
    let mut response = request
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(|_| failed())?;
    if !response.status().is_success() {
        return Err(failed());
    }
    let mut bytes = Zeroizing::new(Vec::new());
    while let Some(chunk) = response.chunk().await.map_err(|_| failed())? {
        if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
            return Err(failed());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| failed())
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Millis {
    Number(f64),
    Text(String),
}
impl Millis {
    fn positive(self) -> Option<f64> {
        let n = match self {
            Self::Number(n) => n,
            Self::Text(s) => s.trim().parse().ok()?,
        };
        (n.is_finite() && n > 0. && n <= 8.64e15).then_some(n)
    }
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Plan {
    total_percent_used: Option<f64>,
    auto_percent_used: Option<f64>,
    api_percent_used: Option<f64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Dashboard {
    billing_cycle_start: Option<Millis>,
    billing_cycle_end: Option<Millis>,
    plan_usage: Option<Plan>,
}
impl Dashboard {
    fn windows(self, observed_at: i64) -> Vec<QuotaWindow> {
        let end = self.billing_cycle_end.and_then(Millis::positive);
        let duration_minutes = end
            .zip(self.billing_cycle_start.and_then(Millis::positive))
            .map(|(end, start)| ((end - start) / 60000.).round())
            .filter(|n| (1. ..=f64::from(u32::MAX)).contains(n))
            .map(|n| {
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "filtered to finite values within 1..=u32::MAX"
                )]
                {
                    n as u32
                }
            });
        let plan = self.plan_usage.unwrap_or_default();
        [
            ("totalPercentUsed", "Overall", plan.total_percent_used),
            ("autoPercentUsed", "Cursor Models", plan.auto_percent_used),
            ("apiPercentUsed", "Other Models", plan.api_percent_used),
        ]
        .into_iter()
        .filter_map(|(id, name, value)| {
            Some(QuotaWindow {
                window_id: Some(id.into()),
                name: name.into(),
                provider: "cursor".into(),
                account: "Cursor".into(),
                observed_at,
                used_percent: value?.clamp(0., 100.),
                resets_at: end.map(|ms| {
                    #[expect(
                        clippy::cast_possible_truncation,
                        reason = "positive millisecond epoch divided by 1000 fits in i64"
                    )]
                    {
                        (ms / 1000.) as i64
                    }
                }),
                duration_minutes,
            })
        })
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::{
        matchers::{body_json, header, method, path},
        Mock, MockServer, ResponseTemplate,
    };

    const RPC: &str = "/aiserver.v1.DashboardService/GetCurrentPeriodUsage";

    #[tokio::test]
    async fn exchanges_api_key_only_at_the_default_endpoint() {
        let server = MockServer::start().await;
        let default = endpoint(&server.uri()).unwrap();
        let client = super::super::report::client().unwrap();
        let key = || Some(Zeroizing::new("fixture-key".into()));
        let other = endpoint("https://example.test/").unwrap();
        assert!(fetch(&client, &other, &default, None, key(), 1)
            .await
            .unwrap_err()
            .to_string()
            .contains("default"));
        Mock::given(method("POST"))
            .and(path("/auth/exchange_user_api_key"))
            .and(header("Authorization", "Bearer fixture-key"))
            .and(body_json(json!({})))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"accessToken":"fixture-token"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(RPC))
            .and(body_json(json!({})))
            .and(header("authorization", "Bearer fixture-token"))
            .and(header("x-cursor-client-type", "cli"))
            .and(header("connect-protocol-version", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "billingCycleStart":"1800000000000","billingCycleEnd":1802419200000_i64,
                "planUsage":{"totalPercentUsed":35.25,"autoPercentUsed":-1,"apiPercentUsed":116}
            })))
            .expect(1)
            .mount(&server)
            .await;
        let rows = fetch(&client, &default, &default, None, key(), 1)
            .await
            .unwrap();
        assert_eq!(
            rows.iter().map(|w| w.used_percent).collect::<Vec<_>>(),
            [35.25, 0., 100.]
        );
        assert_eq!(rows[0].duration_minutes, Some(28 * 1440));
        assert_eq!(rows[0].resets_at, Some(1802419200));
    }

    #[tokio::test]
    async fn token_wins_and_redirects_are_not_followed() {
        let target = MockServer::start().await;
        let source = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&target)
            .await;
        Mock::given(path(format!("/custom{RPC}")))
            .and(header("authorization", "Bearer explicit"))
            .respond_with(ResponseTemplate::new(307).insert_header("Location", target.uri()))
            .expect(1)
            .mount(&source)
            .await;
        let source = endpoint(&format!("{}/custom", source.uri())).unwrap();
        let client = super::super::report::client().unwrap();
        let err = fetch(
            &client,
            &source,
            &endpoint(DEFAULT_ENDPOINT).unwrap(),
            Some(Zeroizing::new("explicit".into())),
            Some(Zeroizing::new("unused".into())),
            1,
        )
        .await
        .unwrap_err();
        assert_eq!(err.to_string(), "Cursor could not read usage limits.");
    }

    #[test]
    fn missing_windows_and_dates_are_not_estimated_from_spend() {
        let response: Dashboard =
            serde_json::from_value(json!({"planUsage":{"totalSpend":500}})).unwrap();
        assert!(response.windows(10).is_empty());
        let response: Dashboard = serde_json::from_value(
            json!({"billingCycleEnd":"invalid","planUsage":{"apiPercentUsed":0}}),
        )
        .unwrap();
        let rows = response.windows(10);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].window_id.as_deref(), Some("apiPercentUsed"));
        assert_eq!((rows[0].duration_minutes, rows[0].resets_at), (None, None));
    }
}
