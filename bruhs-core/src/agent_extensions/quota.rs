//! Current account windows for the thread details card. Reading Codex quota
//! uses the CLI's account metadata interface and does not start a conversation.
use super::{limits_from_codex, QuotaWindow};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::{self, BufRead},
    path::Path,
    time::{Duration, Instant},
};

#[derive(Clone)]
pub(crate) struct QuotaObservation {
    pub zero_spend_cap: bool,
    pub windows: Vec<QuotaWindow>,
    pub plan: Option<String>,
    pub email: Option<String>,
    pub workspace: Option<String>,
    pub checked_at: i64,
}
type WindowCache = tokio::sync::Mutex<Option<(Instant, QuotaObservation)>>;
static CODEX: WindowCache = WindowCache::const_new(None);
static CLAUDE: WindowCache = WindowCache::const_new(None);

pub async fn provider_windows(
    provider: bruhs_models::agent::AgentProvider,
) -> io::Result<Vec<QuotaWindow>> {
    use bruhs_models::agent::AgentProvider;
    super::availability::require_enabled(provider).map_err(io::Error::other)?;
    let reading = match provider {
        AgentProvider::OpenCode => crate::usage_hubs::opencode_usage().await,
        AgentProvider::Grok => crate::usage_hubs::grok_usage().await,
        AgentProvider::Antigravity => crate::usage_hubs::antigravity_usage().await,
        AgentProvider::Cursor => crate::usage_hubs::cursor_usage().await,
        _ => return Ok(provider_observation(provider).await?.windows),
    };
    reading.map_or_else(|| Ok(Vec::new()), |reading| reading.windows)
}
pub(crate) async fn provider_observation(
    provider: bruhs_models::agent::AgentProvider,
) -> io::Result<QuotaObservation> {
    use bruhs_models::agent::AgentProvider::*;
    super::availability::require_enabled(provider).map_err(io::Error::other)?;
    let mut cache = match provider {
        Codex => &CODEX,
        Claude => &CLAUDE,
        _ => return Err(io::Error::other("Unsupported usage provider")),
    }
    .lock()
    .await;
    if let Some((checked, windows)) = cache.as_ref() {
        if checked.elapsed() < Duration::from_secs(30) {
            return Ok(windows.clone());
        }
    }
    let checked_at = chrono::Utc::now();
    let now = checked_at.timestamp();
    let mut zero_spend_cap = false;
    let mut plan = None;
    let mut email = None;
    let mut workspace = None;
    let windows = if provider == Codex {
        let result = tokio::time::timeout(
            Duration::from_secs(12),
            crate::prompt_suggestion::quota::read_codex_quota(true),
        )
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Codex quota lookup timed out"))??;
        plan = result.0["rateLimitsByLimitId"]["codex"]["planType"]
            .as_str()
            .or_else(|| result.0["rateLimits"]["planType"].as_str())
            .map(str::to_owned);
        email = result.1;
        workspace = result.0["accountId"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        account_windows(&result.0, now)
    } else {
        let response = tokio::time::timeout(
            Duration::from_secs(12),
            crate::agent::claude_usage_metadata(),
        )
        .await
        .ok()
        .flatten();
        if let Some((rows, account, zero_cap)) = response.and_then(|(data, account)| {
            Some((
                claude_windows(&data, now)?,
                account,
                claude_zero_spend_cap(&data["rate_limits"]),
            ))
        }) {
            zero_spend_cap = zero_cap;
            email = account.email;
            plan = account.plan;
            rows
        } else {
            let mut rows = tokio::task::spawn_blocking(|| claude_windows_at(&super::directory()?))
                .await
                .map_err(io::Error::other)??;
            rows.retain(|w| w.resets_at.is_some_and(|t| t > now));
            if rows.is_empty() {
                return Err(io::Error::other("Claude usage metadata unavailable"));
            }
            rows
        }
    };
    let observation = QuotaObservation {
        zero_spend_cap,
        plan,
        windows,
        email,
        workspace,
        checked_at: checked_at.timestamp_millis(),
    };
    *cache = Some((Instant::now(), observation.clone()));
    Ok(observation)
}

pub(crate) fn claude_zero_spend_cap(limits: &Value) -> bool {
    let extra = &limits["extra_usage"];
    extra["is_enabled"] == true && extra["monthly_limit"].as_f64() == Some(0.0)
}

pub(crate) fn claude_windows(result: &Value, now: i64) -> Option<Vec<QuotaWindow>> {
    if result["rate_limits_available"] == false {
        return None;
    }
    let limits = result.get("rate_limits")?.as_object()?;
    let mut rows = Vec::new();
    let mut add = |name: String, used: Option<f64>, reset: Option<&str>, duration| {
        if let Some(used) = used.filter(|x| x.is_finite() && *x >= 0.) {
            rows.push(QuotaWindow {
                window_id: None,
                provider: "claude".into(),
                account: "current".into(),
                name,
                used_percent: used.min(100.),
                duration_minutes: duration,
                observed_at: now,
                resets_at: reset
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|d| d.timestamp()),
            });
        }
    };
    for (id, mins) in [("five_hour", 300), ("seven_day", 10080)] {
        if let Some(w) = limits.get(id) {
            add(
                id.into(),
                w["utilization"].as_f64(),
                w["resets_at"].as_str(),
                Some(mins),
            );
        }
    }
    if let Some(scoped) = limits.get("model_scoped").and_then(Value::as_array) {
        for w in scoped.iter().take(128) {
            if let Some(label) = w["display_name"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                add(
                    format!("seven_day_{}", label.chars().take(100).collect::<String>()),
                    w["utilization"].as_f64(),
                    w["resets_at"].as_str(),
                    Some(10080),
                );
            }
        }
    }
    let ratio = |used: &Value, limit: &Value| {
        Some(used.as_f64()? * 100. / limit.as_f64().filter(|n| *n > 0.)?)
    };
    let spend = &result["rate_limits"]["spend"];
    let used = &spend["used"];
    let limit = &spend["limit"];
    let amount = if spend["enabled"] == true
        && used["currency"].as_str().is_some_and(|s| !s.is_empty())
        && used["currency"] == limit["currency"]
        && used["exponent"].is_u64()
        && used["exponent"] == limit["exponent"]
    {
        ratio(&used["amount_minor"], &limit["amount_minor"])
    } else {
        None
    };
    let extra = &result["rate_limits"]["extra_usage"];
    let amount = amount.or_else(|| {
        if extra["is_enabled"] == true {
            ratio(&extra["used_credits"], &extra["monthly_limit"])
                .or_else(|| extra["utilization"].as_f64())
        } else {
            None
        }
    });
    add("monthly_spend".into(), amount, None, None);
    Some(rows)
}

fn account_windows(result: &Value, now: i64) -> Vec<QuotaWindow> {
    let account = result["rateLimitsByLimitId"]
        .get("codex")
        .unwrap_or(&result["rateLimits"]);
    if account["limitId"].as_str().is_some_and(|id| id != "codex") {
        return Vec::new();
    }
    let mut windows = limits_from_codex(&json!({"rateLimits": account}), now);
    let individual = &account["individualLimit"];
    if let Some(left) = individual["remainingPercent"]
        .as_f64()
        .filter(|n| (0.0..=100.0).contains(n))
    {
        windows.push(QuotaWindow {
            window_id: None,
            provider: "codex".into(),
            account: "current".into(),
            name: "individual".into(),
            duration_minutes: None,
            used_percent: 100.0 - left,
            resets_at: individual["resetsAt"].as_i64(),
            observed_at: now,
        });
    }
    windows
}

pub fn claude_windows_at(root: &Path) -> io::Result<Vec<QuotaWindow>> {
    // Older installations already have these observations in the measured-usage journal.
    // Only deserialize the limit entries. Token and cost history can grow independently.
    #[derive(serde::Deserialize)]
    #[serde(tag = "kind", content = "value", rename_all = "snake_case")]
    enum Observation {
        Limit(QuotaWindow),
        #[serde(other)]
        Other,
    }
    let file = match std::fs::File::open(root.join("usage.jsonl")) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut latest: BTreeMap<(String, String), QuotaWindow> = BTreeMap::new();
    for line in io::BufReader::new(file).lines() {
        // A reader may see the tail of an in-progress append. Earlier complete samples are valid.
        if let Ok(Observation::Limit(w)) = serde_json::from_str(&line?) {
            if w.provider != "claude" || !(0.0..=100.0).contains(&w.used_percent) {
                continue;
            }
            let key = (w.account.clone(), w.name.clone());
            if latest
                .get(&key)
                .is_none_or(|old| old.observed_at <= w.observed_at)
            {
                latest.insert(key, w);
            }
        }
    }
    Ok(latest.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uses_account_allowances_not_other_codex_pools() {
        let rows = account_windows(
            &json!({"rateLimits":{"limitId":"other","primary":{"usedPercent":99}},
             "rateLimitsByLimitId":{"codex":{"planType":"free","primary":{"usedPercent":39}, "secondary":{"usedPercent":42,"windowDurationMins":500},"individualLimit":{"remainingPercent":80}}},
            }),
            10,
        );
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].duration_minutes, Some(43200));
        assert_eq!(rows[1].duration_minutes, Some(500));
        assert_eq!(rows[2].used_percent, 20.0);
    }
    #[test]
    fn monthly_zero_cap_requires_an_explicit_enabled_allowance() {
        for (extra, expected) in [
            (json!({"is_enabled":true,"monthly_limit":0.0}), true),
            (json!({"is_enabled":false,"monthly_limit":0}), false),
            (json!({"is_enabled":true,"monthly_limit":400}), false),
            (json!({"is_enabled":true}), false),
        ] {
            assert_eq!(
                claude_zero_spend_cap(&json!({"extra_usage":extra})),
                expected
            );
        }
    }

    #[test]
    fn claude_scopes_and_spend_use_reported_units() {
        let report = json!({"rate_limits_available":true, "rate_limits":{
            "five_hour":{"utilization":25,"resets_at":"2026-10-01T12:00:00Z"},
            "seven_day":null,"model_scoped":[{"display_name":"Opus","utilization":33}],
            "extra_usage":{"is_enabled":true,"monthly_limit":200,"used_credits":150},
            "spend":{"enabled":true,"used":{"amount_minor":20,"currency":"USD","exponent":2},"limit":{"amount_minor":50,"currency":"USD","exponent":2}}
        }});
        let windows = claude_windows(&report, 1).unwrap();
        assert_eq!(windows.len(), 3);
        assert!(windows[0].resets_at.is_some());
        assert_eq!(windows[1].name, "seven_day_Opus");
        assert_eq!(windows[2].used_percent, 40.);
        let mut mismatch = report;
        mismatch["rate_limits"]["spend"]["limit"]["currency"] = json!("GBP");
        assert_eq!(claude_windows(&mismatch, 1).unwrap()[2].used_percent, 75.);
        assert!(claude_windows(&json!({"rate_limits_available":false}), 1).is_none());
    }
    #[test]
    fn claude_keeps_latest_complete_observation_from_older_journal() {
        let root = tempfile::tempdir().unwrap();
        let entry = |pct, time| {
            json!({"kind":"limit","value":{"provider":"claude","account":"current","name":"five_hour","used_percent":pct,"observed_at":time,"resets_at":null}}).to_string()
        };
        std::fs::write(
            root.path().join("usage.jsonl"),
            [entry(25, 20), entry(60, 15), "{".into()].join("\n"),
        )
        .unwrap();
        let rows = claude_windows_at(root.path()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].used_percent, 25.);
    }
}
