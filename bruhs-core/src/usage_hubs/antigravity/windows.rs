//! Normalize the two quota formats returned by Antigravity.
use bruhs_models::agent_extensions::QuotaWindow;
use serde_json::Value;

fn label(v: &Value) -> Option<&str> {
    v.as_str().map(str::trim).filter(|s| !s.is_empty())
}

fn window(
    id: &str,
    name: String,
    minutes: Option<u32>,
    quota: &Value,
    account: &str,
    at_ms: i64,
) -> Option<QuotaWindow> {
    let left = quota["remainingFraction"].as_f64()?.clamp(0., 1.);
    let reset = quota["resetTime"]
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok());
    Some(QuotaWindow {
        window_id: Some(id.into()),
        name,
        duration_minutes: minutes,
        provider: "Antigravity".into(),
        account: account.into(),
        used_percent: (1. - left) * 100.,
        resets_at: reset.map(|r| r.timestamp()),
        observed_at: at_ms / 1000,
    })
}
pub(in crate::usage_hubs) fn summary(data: &Value, account: &str, at_ms: i64) -> Vec<QuotaWindow> {
    let groups = data["groups"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let groups = if groups
        .iter()
        .any(|g| g["buckets"].as_array().is_some_and(|b| !b.is_empty()))
    {
        groups
    } else {
        std::slice::from_ref(data)
    };
    let mut rows: Vec<QuotaWindow> = Vec::new();
    for group in groups {
        for bucket in group["buckets"].as_array().into_iter().flatten() {
            let Some(id) = label(&bucket["bucketId"]) else {
                continue;
            };
            if bucket["disabled"] == true {
                continue;
            }
            let group_name = label(&group["displayName"]);
            let family = if id.starts_with("gemini-argon-") {
                group_name.unwrap_or("Gemini Argon")
            } else if id.starts_with("gemini-carbon-") {
                group_name.unwrap_or("Gemini Carbon")
            } else if id.starts_with("gemini-neon-") {
                group_name.unwrap_or("Gemini Neon")
            } else if id.starts_with("gemini-") {
                group_name.unwrap_or("Gemini")
            } else if id.starts_with("3p-") {
                "Claude & GPT (shared)"
            } else {
                group_name.unwrap_or(id)
            };
            let (period, minutes) = match bucket["window"].as_str() {
                Some("5h") => ("5-hour", Some(300)),
                Some("weekly") => ("Weekly", Some(10080)),
                _ => (label(&bucket["displayName"]).unwrap_or("Quota"), None),
            };
            let Some(mut row) = window(
                id,
                format!("{family} · {period}"),
                minutes,
                bucket,
                account,
                at_ms,
            ) else {
                continue;
            };
            // A full bucket with a rolling placeholder has no real reset yet.
            if bucket["remainingFraction"].as_f64() == Some(1.)
                && bucket.get("description").is_none()
            {
                if let (Some(duration), Some(reset)) = (
                    minutes,
                    bucket["resetTime"]
                        .as_str()
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()),
                ) {
                    if reset
                        .timestamp_millis()
                        .abs_diff(at_ms + i64::from(duration) * 60_000)
                        <= 60_000
                    {
                        row.resets_at = None;
                    }
                }
            }
            if let Some(previous) = rows.iter_mut().find(|w| w.window_id == row.window_id) {
                *previous = row;
            } else {
                rows.push(row);
            }
        }
    }
    rows
}
pub(super) fn models(data: &Value, account: &str, at_ms: i64) -> Vec<QuotaWindow> {
    let mut rows: Vec<QuotaWindow> = Vec::new();
    for (id, model) in data["models"].as_object().into_iter().flatten() {
        let provider = model["modelProvider"].as_str();
        let family = if id.starts_with("gemini-") || provider == Some("MODEL_PROVIDER_GOOGLE") {
            "Gemini"
        } else if id.starts_with("claude-") || provider == Some("MODEL_PROVIDER_ANTHROPIC") {
            "Claude"
        } else if id.starts_with("gpt-") || provider == Some("MODEL_PROVIDER_OPENAI") {
            "GPT"
        } else {
            continue;
        };
        for (field, suffix, period, minutes) in [
            ("quotaInfo", "quota", "Reported quota", None),
            ("weeklyQuotaInfo", "weekly", "Weekly", Some(10080)),
        ] {
            let Some(row) = window(
                &format!("{}-{suffix}", family.to_lowercase()),
                format!("{family} · {period}"),
                minutes,
                &model[field],
                account,
                at_ms,
            ) else {
                continue;
            };
            if let Some(prev) = rows.iter_mut().find(|w| w.window_id == row.window_id) {
                if row.used_percent > prev.used_percent {
                    *prev = row;
                }
            } else {
                rows.push(row);
            }
        }
    }
    rows
}
