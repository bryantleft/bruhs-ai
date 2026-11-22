//! Subscription usage for the first-party Grok CLI sign-in.
use super::{grouping::Identity, AccountUsage};
use bruhs_models::{agent::AgentProvider, agent_extensions::QuotaWindow};
use chrono::{DateTime, FixedOffset};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
    time::Duration,
};
use zeroize::{Zeroize, Zeroizing};

pub(super) const BILLING_URL: &str = "https://cli-chat-proxy.grok.com/v1/billing?format=credits";
const CURRENT: &str = "https://auth.x.ai::b1a00492-073a-47ea-816f-4c329264a828";
const LEGACY: &str = "https://accounts.x.ai/sign-in";
const MAX_BYTES: usize = 2 * 1024 * 1024;
const CUSTOM_SETTINGS: &[&str] = &[
    "XAI_API_KEY",
    "GROK_OIDC_ISSUER",
    "GROK_OIDC_CLIENT_ID",
    "GROK_OAUTH2_ISSUER",
    "GROK_OAUTH2_CLIENT_ID",
    "GROK_OAUTH2_PRINCIPAL_TYPE",
    "GROK_OAUTH2_PRINCIPAL_ID",
    "GROK_AUTH_PROVIDER_COMMAND",
    "GROK_LOCAL_AUTH",
    "GROK_CLI_CHAT_PROXY_BASE_URL",
    "GROK_MODELS_BASE_URL",
    "GROK_CONFIG",
    "GROK_CONFIG_PATH",
];

fn failed() -> io::Error {
    io::Error::other("Grok could not read usage limits.")
}

#[derive(Deserialize)]
struct Credential {
    key: Option<String>,
    auth_mode: Option<String>,
    email: Option<String>,
}
impl Drop for Credential {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

fn read_file(path: &Path) -> io::Result<Option<Zeroizing<String>>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(failed()),
    };
    let mut content = Zeroizing::new(String::new());
    file.take(MAX_BYTES as u64 + 1)
        .read_to_string(&mut content)
        .map_err(|_| failed())?;
    if content.len() > MAX_BYTES {
        return Err(failed());
    }
    Ok(Some(content))
}

fn first_party(home: &Path, system: &Path) -> io::Result<bool> {
    for path in [
        home.join("config.toml"),
        home.join("managed_config.toml"),
        home.join("requirements.toml"),
        system.join("managed_config.toml"),
        system.join("requirements.toml"),
    ] {
        if let Some(text) = read_file(&path)? {
            let config: toml::Table = text.parse().map_err(|_| failed())?;
            if ["auth", "grok_com_config", "endpoints"]
                .iter()
                .any(|key| config.contains_key(*key))
            {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn subscription(text: &str) -> io::Result<Option<Credential>> {
    let mut entries: HashMap<String, Credential> =
        serde_json::from_str(text).map_err(|_| failed())?;
    Ok(entries
        .remove(CURRENT)
        .or_else(|| entries.remove(LEGACY))
        .filter(|c| {
            c.auth_mode.as_deref() != Some("api_key")
                && c.key.as_ref().is_some_and(|s| !s.trim().is_empty())
        }))
}

async fn credential() -> io::Result<Option<Credential>> {
    let bindings = tokio::task::spawn_blocking(crate::secret_sources::load)
        .await
        .map_err(|_| failed())?
        .map_err(|_| failed())?;
    let setting = async |name: &str| -> io::Result<Option<Zeroizing<String>>> {
        Ok(bindings
            .provider_variable("grok", name)
            .await
            .map_err(|_| failed())?
            .map(Zeroizing::new)
            .filter(|s| !s.trim().is_empty()))
    };
    for name in CUSTOM_SETTINGS {
        if setting(name).await?.is_some() {
            return Ok(None);
        }
    }
    let home = setting("GROK_HOME")
        .await?
        .map(|s| PathBuf::from(s.trim()))
        .or_else(|| std::env::home_dir().map(|p| p.join(".grok")))
        .ok_or_else(failed)?;
    let profile = tokio::task::spawn_blocking(move || {
        Ok::<_, io::Error>(
            first_party(&home, Path::new("/etc/grok"))?.then(|| home.join("auth.json")),
        )
    })
    .await
    .map_err(|_| failed())??;
    let Some(path) = profile else { return Ok(None) };
    if let Some(text) = setting("GROK_AUTH").await? {
        return subscription(&text);
    }
    tokio::task::spawn_blocking(move || match read_file(&path)? {
        Some(text) => subscription(&text),
        None => Ok(None),
    })
    .await
    .map_err(|_| failed())?
}

pub(crate) async fn read() -> Option<AccountUsage> {
    crate::agent_extensions::availability::require_enabled(AgentProvider::Grok).ok()?;
    let checked_at = chrono::Utc::now();
    let mut email = None;
    let windows = match tokio::time::timeout(Duration::from_secs(10), credential()).await {
        Ok(Ok(None)) => return None,
        Ok(Ok(Some(mut credential))) => {
            email = credential
                .email
                .take()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty());
            fetch(
                BILLING_URL,
                credential.key.as_deref()?.trim(),
                email.as_deref().unwrap_or("Grok"),
                checked_at.timestamp(),
            )
            .await
        }
        _ => Err(failed()),
    };
    Some(AccountUsage {
        id: "grok".into(),
        label: email.clone().unwrap_or_else(|| "Grok".into()),
        provider: "Grok",
        identity: Identity {
            email,
            ..Default::default()
        },
        checked_at: checked_at.timestamp_millis(),
        zero_spend_cap: false,
        plan: None,
        external: None,
        credits: None,
        windows,
    })
}

async fn fetch(url: &str, token: &str, account: &str, now: i64) -> io::Result<Vec<QuotaWindow>> {
    let mut response = super::report::client()
        .map_err(|_| failed())?
        .get(url)
        .bearer_auth(token)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|_| failed())?;
    let status = response.status();
    if !status.is_success() && status != reqwest::StatusCode::FORBIDDEN {
        return Err(failed());
    }
    let mut bytes = Zeroizing::new(Vec::new());
    while let Some(chunk) = response.chunk().await.map_err(|_| failed())? {
        if bytes.len() + chunk.len() > MAX_BYTES {
            return Err(failed());
        }
        bytes.extend_from_slice(&chunk);
    }
    let body: Value = serde_json::from_slice(&bytes).map_err(|_| failed())?;
    if !status.is_success() {
        if body["error"]
            .as_str()
            .is_some_and(|s| s.contains("WKE=privacy-mode:"))
        {
            return Err(io::Error::other(
                "Switch your xAI account to Privacy Mode to show Grok usage limits.",
            ));
        }
        return Err(failed());
    }
    windows(&body, account, now)
}

// Validate a full zoned ISO timestamp before using the period to infer zero usage.
fn boundary(value: &Value) -> Option<DateTime<FixedOffset>> {
    let (date, rest) = value.as_str()?.split_once('T')?;
    let (year, month_day) = date.rsplit_once('-').and_then(|(prefix, day)| {
        let (year, month) = prefix.rsplit_once('-')?;
        Some((year, (month, day)))
    })?;
    let digits = |s: &str, len| s.len() == len && s.bytes().all(|b| b.is_ascii_digit());
    if !(digits(year, 4) || year.strip_prefix(['+', '-']).is_some_and(|y| digits(y, 6)))
        || !digits(month_day.0, 2)
        || !digits(month_day.1, 2)
    {
        return None;
    }
    let date = chrono::NaiveDate::from_ymd_opt(
        year.parse().ok()?,
        month_day.0.parse().ok()?,
        month_day.1.parse().ok()?,
    )?;
    let (clock, zone) = rest.split_at(rest.find(['Z', '+', '-'])?);
    let offset = if zone == "Z" {
        FixedOffset::east_opt(0)?
    } else {
        let (hours, mins) = zone.get(1..)?.split_once(':')?;
        if !digits(hours, 2) || !digits(mins, 2) {
            return None;
        }
        let (hours, mins): (i32, i32) = (hours.parse().ok()?, mins.parse().ok()?);
        if hours >= 24 || mins >= 60 {
            return None;
        }
        let seconds = 3600 * hours + 60 * mins;
        FixedOffset::east_opt(if zone.starts_with('-') {
            -seconds
        } else {
            seconds
        })?
    };
    let mut parts = clock.split(':');
    let (h, m) = (parts.next()?, parts.next()?);
    let (seconds, fraction) = match parts.next() {
        Some(s) => match s.split_once('.') {
            Some((s, f)) if !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()) => (s, f),
            Some(_) => return None,
            None => (s, ""),
        },
        None => ("00", ""),
    };
    if parts.next().is_some() || !digits(h, 2) || !digits(m, 2) || !digits(seconds, 2) {
        return None;
    }
    let millis = fraction
        .bytes()
        .take(3)
        .zip([100, 10, 1])
        .map(|(b, p)| u32::from(b - b'0') * p)
        .sum();
    let local = if h == "24" && m == "00" && seconds == "00" && fraction.bytes().all(|b| b == b'0')
    {
        date.succ_opt()?.and_hms_opt(0, 0, 0)?
    } else {
        date.and_hms_milli_opt(
            h.parse().ok()?,
            m.parse().ok()?,
            seconds.parse().ok()?,
            millis,
        )?
    };
    local.and_local_timezone(offset).single()
}

pub(super) fn windows(body: &Value, account: &str, now: i64) -> io::Result<Vec<QuotaWindow>> {
    let config = &body["config"];
    let period = &config["currentPeriod"];
    if !body.is_object()
        || body.get("config").is_some_and(|v| !v.is_object())
        || config
            .get("creditUsagePercent")
            .is_some_and(|v| !v.is_number())
        || config.get("currentPeriod").is_some_and(|v| !v.is_object())
        || ["type", "end"]
            .iter()
            .any(|key| period.get(*key).is_some_and(|v| !v.is_string()))
    {
        return Err(failed());
    }
    let kind = period["type"].as_str().unwrap_or("");
    let label = match kind.strip_prefix("USAGE_PERIOD_TYPE_").unwrap_or(kind) {
        "WEEKLY" => "Weekly",
        "MONTHLY" => "Monthly",
        _ => "Subscription",
    };
    let end = boundary(&period["end"]);
    let millis = boundary(&period["start"])
        .zip(end)
        .map(|(s, e)| e.timestamp_millis() - s.timestamp_millis())
        .filter(|d| *d > 0);
    let used = config["creditUsagePercent"]
        .as_f64()
        .or_else(|| (label != "Subscription" && millis.is_some()).then_some(0.));
    Ok(used
        .into_iter()
        .map(|n| QuotaWindow {
            window_id: Some("subscription".into()),
            name: label.into(),
            provider: "grok".into(),
            account: account.into(),
            used_percent: n.clamp(0., 100.),
            duration_minutes: millis
                .and_then(|d| u32::try_from((d + 30000) / 60000).ok())
                .filter(|m| *m > 0),
            resets_at: end.map(|e| e.timestamp()),
            observed_at: now,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[tokio::test]
    async fn billing_reports_privacy_mode_and_sanitizes_other_failures() {
        use wiremock::{
            matchers::{header, method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        for (route, code, body, expected) in [
            (
                "privacy",
                403,
                json!({"error":"private detail WKE=privacy-mode: value"}),
                "Switch your xAI account to Privacy Mode to show Grok usage limits.",
            ),
            (
                "denied",
                403,
                json!({"error":"private detail"}),
                "Grok could not read usage limits.",
            ),
            (
                "bad",
                200,
                json!({"config":{"creditUsagePercent":"?"}}),
                "Grok could not read usage limits.",
            ),
        ] {
            Mock::given(method("GET"))
                .and(path(format!("/{route}")))
                .and(header("authorization", "Bearer fixture"))
                .respond_with(ResponseTemplate::new(code).set_body_json(body))
                .expect(1)
                .mount(&server)
                .await;
            let error = fetch(
                &format!("{}/{route}", server.uri()),
                "fixture",
                "account",
                0,
            )
            .await
            .unwrap_err();
            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn profile_requires_first_party_subscription() {
        let base = tempfile::tempdir().unwrap();
        let home = base.path();
        let system = home.join("etc/grok");
        std::fs::create_dir_all(&system).unwrap();
        assert!(first_party(home, &system).unwrap());
        std::fs::write(
            system.join("requirements.toml"),
            "[endpoints.chat]\nurl='https://example.test'",
        )
        .unwrap();
        assert!(!first_party(home, &system).unwrap());
        let modern = json!({(CURRENT):{"key":"new"},(LEGACY):{"key":"old"}});
        assert_eq!(
            subscription(&modern.to_string())
                .unwrap()
                .unwrap()
                .key
                .as_deref(),
            Some("new")
        );
        for skip in [
            json!({(CURRENT):{"auth_mode":"api_key","key":"current"},(LEGACY):{"key":"old"}}),
            json!({(LEGACY):{"key":" "}}),
            json!({}),
        ] {
            assert!(subscription(&skip.to_string()).unwrap().is_none());
        }
    }
    #[test]
    fn zero_requires_valid_strictly_ordered_subscription_boundaries() {
        let read = |kind: &str, start: Value, end: &str| {
            windows(
                &json!({"config":{"currentPeriod":{"type":kind,"start":start,"end":end}}}),
                "fixture",
                3,
            )
            .unwrap()
        };
        let start = json!("2028-02-29T24:00Z");
        let rows = read(
            "USAGE_PERIOD_TYPE_MONTHLY",
            start.clone(),
            "2028-03-01T02:01:30+02:00",
        );
        assert_eq!(rows[0].used_percent, 0.);
        assert_eq!(rows[0].duration_minutes, Some(2));
        assert_eq!(rows[0].window_id.as_deref(), Some("subscription"));
        assert!(read("WEEKLY", start.clone(), "2028-03-01T00:00:00.001Z")[0]
            .duration_minutes
            .is_none());
        assert!(read("DAILY", start.clone(), "2028-03-02T00:00Z").is_empty());
        for invalid in [
            json!("2027-02-29T00:00Z"),
            json!("2028-01-01"),
            json!("2028-01-01T24:00:00.1Z"),
            json!("2028-02-29T00:00:60Z"),
            json!("2028-2-9T00:00Z"),
            json!(1),
            start,
        ] {
            assert!(read("MONTHLY", invalid, "2028-03-01T00:00Z").is_empty());
        }
        let rows = windows(&json!({"config":{"creditUsagePercent":125}}), "fixture", 0).unwrap();
        assert_eq!(
            (rows[0].name.as_str(), rows[0].used_percent),
            ("Subscription", 100.)
        );
        assert!(windows(&json!({"config":{"creditUsagePercent":"0"}}), "x", 0).is_err());
    }
}
