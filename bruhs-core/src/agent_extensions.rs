//! Private local ACP configuration and an append-only measured-usage journal.
use bruhs_models::{
    agent::AgentProvider,
    agent_extensions::{
        ExtensionConfig, QuotaWindow, SharedUsageHistory, UsageEstimate, UsageHistoryPeriod,
        UsageModelPricing, UsageModelPricingChange, UsageRecord, UsageSnapshot,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::{BufRead, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

static WRITER: Mutex<()> = Mutex::new(());
pub mod availability;
pub mod environment;
pub mod history;
pub mod instances;
pub mod kiro;
pub mod quota;
pub mod registry;

pub fn directory() -> std::io::Result<PathBuf> {
    if let Some(path) = std::env::var_os("BRUHS_WORKBENCH_DIR") {
        if path.is_empty() {
            return Err(std::io::Error::other("BRUHS_WORKBENCH_DIR is empty."));
        }
        return Ok(PathBuf::from(path));
    }
    directories::ProjectDirs::from("dev", "bruhs", "bruhs-ai")
        .map(|dirs| dirs.data_local_dir().join("workbench"))
        .ok_or_else(|| std::io::Error::other("Private application data directory is unavailable."))
}

pub(crate) fn private_directory(root: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(root)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub fn load_config_at(root: &Path) -> std::io::Result<ExtensionConfig> {
    match std::fs::read(root.join("agents.json")) {
        Ok(bytes) => {
            let config: ExtensionConfig = serde_json::from_slice(&bytes)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            config
                .validate()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            Ok(config)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ExtensionConfig::default()),
        Err(e) => Err(e),
    }
}

pub fn load_config() -> std::io::Result<ExtensionConfig> {
    load_config_at(&directory()?)
}

pub fn save_config_at(root: &Path, config: &ExtensionConfig) -> std::io::Result<()> {
    config.validate().map_err(std::io::Error::other)?;
    let _lock = lock_config(root)?;
    // Workbench can replace a damaged config. A device handle must still
    // belong to a readable prior configuration or be supplied by the editor.
    let previous = match load_config_at(root) {
        Ok(config) => config,
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => ExtensionConfig::default(),
        Err(e) => return Err(e),
    };
    environment::persist(root, &previous, config, Default::default())
}

fn lock_config(root: &Path) -> std::io::Result<std::fs::File> {
    private_directory(root)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("agents.lock"))?;
    lock.lock()?;
    Ok(lock)
}

/// Change one existing instance without replacing settings from another editor.
pub fn set_acp_enabled(id: &str, enabled: bool) -> std::io::Result<()> {
    let root = directory()?;
    let _lock = lock_config(&root)?;
    let mut config = load_config_at(&root)?;
    let agent = config
        .acp
        .iter_mut()
        .find(|agent| agent.id == id)
        .ok_or_else(|| {
            std::io::Error::other("This ACP instance was removed. Refresh provider status.")
        })?;
    agent.enabled = enabled;
    write_config(&root, &config)
}

/// Return only the settings owned by the Usage price editor.
pub fn usage_model_pricing_at(root: &Path) -> std::io::Result<BTreeMap<String, UsageModelPricing>> {
    let config = load_config_at(root)?;
    let mut rows: BTreeMap<_, UsageModelPricing> = config
        .prices
        .into_iter()
        .map(|(model, price)| {
            (
                model,
                UsageModelPricing {
                    price: Some(price),
                    alias: None,
                },
            )
        })
        .collect();
    for (model, alias) in config.usage_model_aliases {
        rows.entry(model).or_default().alias = Some(alias);
    }
    Ok(rows)
}

/// Commit all rows together, keeping provider settings and unrelated models intact.
pub fn edit_usage_model_prices_at(
    root: &Path,
    changes: Vec<UsageModelPricingChange>,
) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};

    let _lock = lock_config(root)?;
    let mut config = load_config_at(root)?;
    let mut seen = std::collections::BTreeSet::new();
    let mut changed = false;
    for UsageModelPricingChange {
        model,
        expected,
        edit,
    } in changes
    {
        if !bruhs_models::agent_extensions::valid_usage_model_id(&model) {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "Enter a valid model ID.",
            ));
        }
        if !seen.insert(model.clone()) {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!("Duplicate price change for {model}."),
            ));
        }
        let current = config.usage_model_pricing(&model);
        if current == edit {
            continue;
        }
        if current != expected {
            return Err(Error::new(
                ErrorKind::AlreadyExists,
                format!("{model} changed elsewhere. Refresh saved values before retrying."),
            ));
        }
        if let Some(price) = edit.price {
            config.prices.insert(model.clone(), price);
        } else {
            config.prices.remove(&model);
        }
        if let Some(alias) = edit.alias {
            config.usage_model_aliases.insert(model, alias);
        } else {
            config.usage_model_aliases.remove(&model);
        }
        changed = true;
    }
    if changed {
        config
            .validate()
            .map_err(|error| Error::new(ErrorKind::InvalidInput, error))?;
        write_config(root, &config)?;
    }
    Ok(())
}

fn write_config(root: &Path, config: &ExtensionConfig) -> std::io::Result<()> {
    let mut temporary = tempfile::NamedTempFile::new_in(root)?;
    serde_json::to_writer_pretty(&mut temporary, config).map_err(std::io::Error::other)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(root.join("agents.json"))
        .map_err(std::io::Error::other)?;
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum Entry {
    Turn(UsageRecord),
    Limit(QuotaWindow),
}

fn append(root: &Path, entry: &Entry) -> std::io::Result<()> {
    let _guard = WRITER
        .lock()
        .map_err(|_| std::io::Error::other("Usage journal lock failed."))?;
    private_directory(root)?;
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(root.join("usage.jsonl"))?;
    let mut bytes = serde_json::to_vec(entry).map_err(std::io::Error::other)?;
    bytes.push(b'\n');
    file.write_all(&bytes)?;
    file.sync_data()
}

pub fn record_at(root: &Path, record: UsageRecord) -> std::io::Result<()> {
    append(root, &Entry::Turn(record))
}

pub fn limits_from_codex(result: &serde_json::Value, now: i64) -> Vec<QuotaWindow> {
    let buckets: Vec<(String, &serde_json::Value)> = result["rateLimitsByLimitId"]
        .as_object()
        .filter(|m| !m.is_empty())
        .map_or_else(
            || vec![("current".into(), &result["rateLimits"])],
            |m| m.iter().map(|(k, v)| (k.clone(), v)).collect(),
        );
    let mut out = Vec::new();
    for (account, bucket) in buckets {
        for name in ["primary", "secondary"] {
            let window = &bucket[name];
            if let Some(used) = window["usedPercent"]
                .as_f64()
                .filter(|n| n.is_finite() && (0.0..=100.0).contains(n))
            {
                out.push(QuotaWindow {
                    window_id: None,
                    provider: "codex".into(),
                    account: account.clone(),
                    name: name.into(),
                    used_percent: used,
                    duration_minutes: Some(
                        window["windowDurationMins"]
                            .as_u64()
                            .and_then(|n| u32::try_from(n).ok())
                            .filter(|n| *n > 0)
                            .unwrap_or_else(|| match (name, bucket["planType"].as_str()) {
                                ("secondary", _) => 10080,
                                (_, Some("free" | "go")) => 43200,
                                _ => 300,
                            }),
                    ),
                    resets_at: window["resetsAt"].as_i64(),
                    observed_at: now,
                });
            }
        }
    }
    out
}

pub fn record_claude_limit(event: &serde_json::Value) -> std::io::Result<()> {
    if event["type"] != "rate_limit_event" {
        return Ok(());
    }
    let info = &event["rate_limit_info"];
    let Some(used) = info["utilization"]
        .as_f64()
        .filter(|n| n.is_finite() && (0.0..=1.0).contains(n))
    else {
        return Ok(());
    };
    append(
        &directory()?,
        &Entry::Limit(QuotaWindow {
            window_id: None,
            provider: "claude".into(),
            account: "current".into(),
            name: info["rateLimitType"].as_str().unwrap_or("account").into(),
            used_percent: used * 100.0,
            duration_minutes: None,
            resets_at: info["resetsAt"].as_i64(),
            observed_at: chrono::Utc::now().timestamp(),
        }),
    )
}

pub async fn refresh_codex() -> std::io::Result<Vec<QuotaWindow>> {
    // Same metadata-only probe and deadline as next-message suggestion admission.
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        crate::prompt_suggestion::quota::read_codex_limits(),
    )
    .await
    .map_err(|_| {
        std::io::Error::other(
            "Codex usage check timed out. Check the installed CLI login and retry.",
        )
    })??;
    let limits = limits_from_codex(&result, chrono::Utc::now().timestamp());
    let saved = limits.clone();
    tokio::task::spawn_blocking(move || {
        let root = directory()?;
        for limit in saved {
            append(&root, &Entry::Limit(limit))?;
        }
        Ok::<(), std::io::Error>(())
    })
    .await
    .map_err(std::io::Error::other)??;
    Ok(limits)
}

pub fn snapshot_at(root: &Path) -> std::io::Result<UsageSnapshot> {
    read_snapshot(root, None, None, true)
}

#[derive(Clone, Copy)]
pub enum UsageHistoryRange {
    Daily(chrono::NaiveDate),
    Last24h { end: i64 },
}

impl UsageHistoryRange {
    pub fn from_period(period: UsageHistoryPeriod) -> Option<Self> {
        let now = chrono::Utc::now();
        match (period.days, period.last_24h_ending_at) {
            (Some(days @ 1..=90), None) => now
                .date_naive()
                .checked_sub_days(chrono::Days::new(u64::from(days - 1)))
                .map(Self::Daily),
            // Linked environments use the desktop's boundary for the same rolling window.
            (None, Some(end)) if now.timestamp().abs_diff(end) <= 3600 => {
                Some(Self::Last24h { end })
            }
            _ => None,
        }
    }
}

pub fn snapshot_for_period_at(
    root: &Path,
    range: UsageHistoryRange,
    provider: Option<AgentProvider>,
) -> std::io::Result<UsageSnapshot> {
    read_snapshot(root, Some(range), provider, true)
}

pub fn shared_history_at(
    root: &Path,
    range: UsageHistoryRange,
    provider: Option<AgentProvider>,
) -> std::io::Result<SharedUsageHistory> {
    read_snapshot(root, Some(range), provider, false).map(SharedUsageHistory::from)
}

fn read_snapshot(
    root: &Path,
    range: Option<UsageHistoryRange>,
    provider: Option<AgentProvider>,
    include_private: bool,
) -> std::io::Result<UsageSnapshot> {
    let config = load_config_at(root)?;
    let mut snapshot = UsageSnapshot {
        imported: if include_private {
            history::snapshot_at(root, &config)?
        } else {
            Default::default()
        },
        ..UsageSnapshot::default()
    };
    let today = chrono::Utc::now().date_naive();
    let file = match std::fs::File::open(root.join("usage.jsonl")) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(snapshot),
        Err(e) => return Err(e),
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut sessions: BTreeMap<&str, std::collections::BTreeSet<String>> = BTreeMap::new();
    let mut limits: BTreeMap<(String, String, String), QuotaWindow> = BTreeMap::new();
    for (line_number, line) in std::io::BufReader::new(file).lines().enumerate() {
        let line = line?;
        let entry:Entry=serde_json::from_str(&line).map_err(|e|std::io::Error::other(format!("Usage journal line {} is invalid: {e}. Restore the private usage journal from backup.",line_number+1)))?;
        match entry {
            Entry::Turn(record) if seen.insert(record.id.clone()) => {
                let day = chrono::DateTime::from_timestamp(record.at, 0)
                    .ok_or_else(|| std::io::Error::other("Usage timestamp is invalid."))?
                    .date_naive();
                let runtime = AgentProvider::for_model(&record.model);
                if range.is_some_and(|range| match range {
                    UsageHistoryRange::Daily(since) => day < since || day > today,
                    UsageHistoryRange::Last24h { end } => {
                        record.at < end - 86_400 || record.at >= end
                    }
                }) || provider.is_some_and(|provider| provider != runtime)
                    || config
                        .usage_model_filter
                        .as_ref()
                        .is_some_and(|filter| !filter.allows(runtime, &record.model))
                {
                    continue;
                }
                let model = config.usage_model(&record.model);
                let price = config.prices.get(model);
                snapshot
                    .by_model
                    .entry(model.to_owned())
                    .or_default()
                    .add(&record, price);
                if include_private {
                    snapshot
                        .by_project
                        .entry(record.project.clone())
                        .or_default()
                        .add(&record, price);
                }
                let breakdown = &mut snapshot.breakdown;
                let providers = if let Some(UsageHistoryRange::Last24h { .. }) = range {
                    let hour = record.at.div_euclid(3600) * 3600;
                    snapshot
                        .by_hour
                        .entry(hour)
                        .or_default()
                        .add(&record, price);
                    Some(breakdown.by_hour_provider.entry(hour).or_default())
                } else {
                    let day = day.to_string();
                    snapshot
                        .by_day
                        .entry(day.clone())
                        .or_default()
                        .add(&record, price);
                    range.map(|_| breakdown.by_day_provider.entry(day).or_default())
                };
                if let Some(providers) = providers {
                    if let Some(usd) = record
                        .cli_cost_usd
                        .filter(|usd| usd.is_finite() && *usd >= 0.)
                    {
                        breakdown
                            .cli_costs
                            .entry(runtime.as_str().into())
                            .or_default()
                            .add(usd);
                    }
                    providers
                        .entry(runtime.as_str().into())
                        .or_default()
                        .add(&record, price);
                    let estimate = price.map(|price| UsageEstimate::for_record(&record, price));
                    if record.requested_speed == bruhs_models::agent_extensions::UsageSpeed::Fast {
                        breakdown
                            .fast_pricing
                            .add_fast_turn(&record, price, estimate.as_ref());
                    }
                    if let (Some(price), Some(estimate)) = (price, estimate) {
                        *breakdown
                            .cost_by_speed
                            .entry(record.requested_speed)
                            .or_default() += estimate.total();
                        breakdown.cache_savings.add(&record, price, &estimate);
                        breakdown.cost_by_type.merge(estimate);
                    }
                    if let Some(session) = record.session_id.filter(|id| !id.is_empty()) {
                        sessions
                            .entry(runtime.as_str())
                            .or_default()
                            .insert(session);
                    }
                }
            }
            Entry::Turn(_) => {}
            Entry::Limit(window) if include_private => {
                let key = (
                    window.provider.clone(),
                    window.account.clone(),
                    window.name.clone(),
                );
                if limits
                    .get(&key)
                    .is_none_or(|old| old.observed_at <= window.observed_at)
                {
                    limits.insert(key, window);
                }
            }
            Entry::Limit(_) => {}
        }
    }
    snapshot.limits = limits.into_values().collect();
    snapshot.breakdown.sessions_by_provider = sessions
        .into_iter()
        .map(|(provider, ids)| {
            (
                provider.to_owned(),
                u64::try_from(ids.len()).unwrap_or(u64::MAX),
            )
        })
        .collect();
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit_usage_model_pricing_at(
        root: &Path,
        model: &str,
        expected: &UsageModelPricing,
        edit: UsageModelPricing,
    ) -> std::io::Result<()> {
        edit_usage_model_prices_at(
            root,
            vec![UsageModelPricingChange {
                model: model.into(),
                expected: expected.clone(),
                edit,
            }],
        )
    }

    #[test]
    fn usage_price_batch_checks_every_row_and_validates_final_mappings() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path();
        let change = |model: &str, expected: Option<&str>, alias: Option<&str>| {
            let mapped = |target: Option<&str>| UsageModelPricing {
                price: None,
                alias: target.map(str::to_owned),
            };
            UsageModelPricingChange {
                model: model.into(),
                expected: mapped(expected),
                edit: mapped(alias),
            }
        };
        let a = "codex/a";
        let b = "codex/b";
        assert!(edit_usage_model_prices_at(
            path,
            vec![
                change(a, None, Some(b)),
                change(b, Some(a), Some("codex/c"))
            ],
        )
        .is_err());
        assert!(usage_model_pricing_at(path).unwrap().is_empty());
        assert!(edit_usage_model_prices_at(
            path,
            vec![change(a, None, Some(b)), change(b, None, Some(a))],
        )
        .is_err());
        assert!(usage_model_pricing_at(path).unwrap().is_empty());
        edit_usage_model_prices_at(path, vec![change(a, None, Some(b))]).unwrap();
        let reverse = vec![change(b, None, Some(a)), change(a, Some(b), None)];
        edit_usage_model_prices_at(path, reverse.clone()).unwrap();
        edit_usage_model_prices_at(path, reverse).unwrap();
        let saved = usage_model_pricing_at(path).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[b].alias.as_deref(), Some(a));
    }

    #[test]
    fn usage_price_edits_apply_to_history_without_overwriting_other_model_settings() {
        use bruhs_models::agent_extensions::ModelPrice;

        let root = tempfile::tempdir().unwrap();
        let path = root.path();
        std::fs::write(
            path.join("agents.json"),
            r#"{"usage_model_filter":{"exclude":[
            {"provider":"codex","model":"codex/skip"}
        ]}}"#,
        )
        .unwrap();
        let price = UsageModelPricing {
            price: Some(ModelPrice {
                input_per_million: 3.0,
                ..Default::default()
            }),
            alias: None,
        };
        let map = UsageModelPricing {
            price: None,
            alias: Some("codex/default".into()),
        };
        let empty = UsageModelPricing::default();
        edit_usage_model_pricing_at(path, "codex/default", &empty, price.clone()).unwrap();
        edit_usage_model_pricing_at(path, "codex/preview", &empty, map.clone()).unwrap();
        edit_usage_model_pricing_at(path, "codex/skip", &empty, map.clone()).unwrap();
        edit_usage_model_pricing_at(path, "codex/preview", &empty, map).unwrap();
        assert!(edit_usage_model_pricing_at(path, "codex/preview", &empty, price).is_err());

        let loop_back = UsageModelPricing {
            price: None,
            alias: Some("codex/preview".into()),
        };
        let original = usage_model_pricing_at(path).unwrap();
        assert!(edit_usage_model_pricing_at(
            path,
            "codex/default",
            &original["codex/default"],
            loop_back
        )
        .is_err());
        assert_eq!(usage_model_pricing_at(path).unwrap(), original);

        let today = chrono::Utc::now();
        for (id, model) in [
            ("a", "codex/preview"),
            ("b", "codex/skip"),
            ("c", "codex/default"),
        ] {
            record_at(
                path,
                UsageRecord {
                    id: id.into(),
                    thread_id: id.into(),
                    session_id: Some(id.into()),
                    model: model.into(),
                    project: "fixture".into(),
                    at: today.timestamp(),
                    input: 1_000_000,
                    output: 0,
                    cache_read: 0,
                    cache_write: 0,
                    requested_speed: Default::default(),
                    cli_cost_usd: Some(1.0),
                },
            )
            .unwrap();
        }
        let range = UsageHistoryRange::Daily(today.date_naive());
        let snapshot = shared_history_at(path, range, Some(AgentProvider::Codex)).unwrap();
        assert_eq!(snapshot.by_model.len(), 1);
        assert_eq!(snapshot.by_model["codex/default"].turns, 2);
        assert_eq!(
            snapshot.by_model["codex/default"]
                .estimated_usd
                .map(|price| price.to_string()),
            Some("6".into())
        );
        assert_eq!(snapshot.breakdown.cli_costs["codex"].runs, 2);

        edit_usage_model_pricing_at(path, "codex/default", &original["codex/default"], empty)
            .unwrap();
        let after = shared_history_at(path, range, None).unwrap();
        assert_eq!(after.by_model["codex/default"].unpriced_turns, 2);
    }
    #[test]
    fn journal_deduplicates_a_reconnected_completion_and_retains_unknown_price() {
        let root = tempfile::tempdir().unwrap();
        for _ in 0..2 {
            record_at(
                root.path(),
                UsageRecord {
                    id: "receipt".into(),
                    thread_id: "thread".into(),
                    session_id: None,
                    model: "model".into(),
                    project: "project".into(),
                    at: 0,
                    input: 100,
                    output: 20,
                    cache_read: 30,
                    cache_write: 0,
                    requested_speed: Default::default(),
                    cli_cost_usd: None,
                },
            )
            .unwrap();
        }
        let snapshot = snapshot_at(root.path()).unwrap();
        assert_eq!(snapshot.by_model["model"].turns, 1);
        assert_eq!(snapshot.by_model["model"].estimated_usd, None);
        assert_eq!(snapshot.by_day["1970-01-01"].input, 100);
    }

    #[test]
    fn period_breakdown_preserves_priced_costs_and_legacy_speed_across_environments() {
        use bruhs_models::agent_extensions::UsageSpeed;

        let root = tempfile::tempdir().unwrap();
        let today = chrono::Utc::now();
        let since = today.date_naive();
        let day = since.to_string();
        std::fs::write(root.path().join("agents.json"), r#"{"prices": {
            "gpt-5.5": {"input_per_million": 4, "output_per_million": 8, "cache_read_per_million": 1, "cache_write_per_million": 2},
            "claude-sonnet-4-6": {"input_per_million": 3, "output_per_million": 5}
        }}"#).unwrap();
        let records = [
            ("gpt-5.5", UsageSpeed::Fast, today.timestamp()),
            ("claude-sonnet-4-6", UsageSpeed::Unknown, today.timestamp()),
            ("codex/unknown", UsageSpeed::Unknown, today.timestamp()),
            (
                "gpt-5.5",
                UsageSpeed::Fast,
                (today - chrono::Days::new(1)).timestamp(),
            ),
        ];
        for (id, (model, requested_speed, at)) in records.into_iter().enumerate() {
            record_at(
                root.path(),
                UsageRecord {
                    id: id.to_string(),
                    thread_id: "private-thread".into(),
                    session_id: Some("private-session".into()),
                    project: "private-project".into(),
                    model: model.into(),
                    at,
                    input: 1_000_000,
                    output: 1_000_000,
                    cache_read: 1_000_000,
                    cache_write: 1_000_000,
                    requested_speed,
                    cli_cost_usd: model.starts_with("claude").then_some(9.),
                },
            )
            .unwrap();
        }
        let mut history = shared_history_at(
            root.path(),
            UsageHistoryRange::Daily(since),
            Some(AgentProvider::Claude),
        )
        .unwrap();
        assert_eq!(history.by_day[&day].turns, 1);
        history.merge(
            shared_history_at(
                root.path(),
                UsageHistoryRange::Daily(since),
                Some(AgentProvider::Codex),
            )
            .unwrap(),
        );
        assert_eq!(history.by_day[&day].unpriced_turns, 1);
        assert_eq!(history.by_day[&day].estimated_usd, None);
        assert_eq!(
            history.by_day[&day].priced_subtotal().unwrap().to_string(),
            "23"
        );
        assert_eq!(history.breakdown.by_day_provider[&day]["codex"].turns, 2);
        assert_eq!(history.breakdown.by_day_provider[&day]["claude"].turns, 1);
        assert_eq!(history.breakdown.cost_by_type.total().to_string(), "23");
        assert_eq!(history.breakdown.cli_costs["claude"].runs, 1);
        assert_eq!(history.breakdown.cli_costs["claude"].usd.to_string(), "9");
        assert_eq!(history.breakdown.sessions_by_provider["codex"], 1);
        assert_eq!(history.breakdown.sessions_by_provider["claude"], 1);
        assert_eq!(history.breakdown.cache_savings.priced_tokens, 4_000_000);
        assert_eq!(
            history.breakdown.cache_savings.estimated_usd.to_string(),
            "11"
        );
        assert_eq!(
            history.breakdown.cost_by_speed[&UsageSpeed::Fast].to_string(),
            "15"
        );
        assert_eq!(
            history.breakdown.cost_by_speed[&UsageSpeed::Unknown].to_string(),
            "8"
        );
        assert!(!serde_json::to_string(&history)
            .unwrap()
            .contains("private-"));
    }
    #[test]
    fn fast_rates_keep_the_premium_and_coverage_separate_from_usage_cost() {
        use bruhs_models::agent_extensions::UsageSpeed;

        let root = tempfile::tempdir().unwrap();
        let now = chrono::Utc::now();
        std::fs::write(root.path().join("agents.json"), r#"{"prices": {
            "gpt-5.5": {"input_per_million": 1, "output_per_million": 2, "cache_read_per_million": 0.5, "cache_write_per_million": 0.75,
                "fast": {"input_per_million": 2, "output_per_million": 4, "cache_read_per_million": 1, "cache_write_per_million": 1.5}},
            "codex/fallback": {"input_per_million": 1, "output_per_million": 1}
        }}"#).unwrap();
        for (index, (model, requested_speed)) in [
            ("gpt-5.5", UsageSpeed::Standard),
            ("gpt-5.5", UsageSpeed::Fast),
            ("codex/fallback", UsageSpeed::Fast),
            ("codex/unpriced", UsageSpeed::Fast),
        ]
        .into_iter()
        .enumerate()
        {
            record_at(
                root.path(),
                UsageRecord {
                    id: index.to_string(),
                    thread_id: "session".into(),
                    session_id: None,
                    model: model.into(),
                    project: "test".into(),
                    at: now.timestamp(),
                    input: 1_000_000,
                    output: 1_000_000,
                    cache_read: 1_000_000,
                    cache_write: 1_000_000,
                    requested_speed,
                    cli_cost_usd: None,
                },
            )
            .unwrap();
        }
        let history = shared_history_at(
            root.path(),
            UsageHistoryRange::Daily(now.date_naive()),
            None,
        )
        .unwrap();
        let breakdown = &history.breakdown;
        assert_eq!(
            history.by_model["gpt-5.5"]
                .estimated_usd
                .unwrap()
                .to_string(),
            "12.75"
        );
        assert_eq!(breakdown.cost_by_type.total().to_string(), "14.75");
        assert_eq!(
            breakdown.cost_by_speed[&UsageSpeed::Fast].to_string(),
            "10.5"
        );
        assert_eq!(breakdown.fast_pricing.tier_priced_turns, 1);
        assert_eq!(breakdown.fast_pricing.standard_fallback_turns, 1);
        assert_eq!(breakdown.fast_pricing.unpriced_turns, 1);
        assert_eq!(breakdown.fast_pricing.premium_usd.to_string(), "4.25");
        assert_eq!(breakdown.cache_savings.estimated_usd.to_string(), "4.25");

        let mut old = SharedUsageHistory::default();
        old.breakdown.cost_by_speed.insert(UsageSpeed::Fast, 3.);
        old.merge(history);
        assert!(old.breakdown.fast_pricing.includes_legacy_report);
        assert_eq!(old.breakdown.fast_pricing.premium_usd.to_string(), "4.25");
    }

    #[test]
    fn rolling_history_filters_exact_boundaries_before_aggregating_hours() {
        let root = tempfile::tempdir().unwrap();
        let end = chrono::Utc::now().timestamp().div_euclid(3600) * 3600 + 234;
        let start = end - 86_400;
        for (input, at) in [(8, start - 1), (1, start), (2, end - 1), (4, end)] {
            record_at(
                root.path(),
                UsageRecord {
                    id: input.to_string(),
                    thread_id: "private-thread".into(),
                    session_id: None,
                    model: "gpt-5.5".into(),
                    project: "private-project".into(),
                    at,
                    input,
                    output: 0,
                    cache_read: 0,
                    cache_write: 0,
                    requested_speed: Default::default(),
                    cli_cost_usd: None,
                },
            )
            .unwrap();
        }
        let range = UsageHistoryRange::Last24h { end };
        let shared = shared_history_at(root.path(), range, Some(AgentProvider::Codex)).unwrap();
        let local = snapshot_for_period_at(root.path(), range, Some(AgentProvider::Codex)).unwrap();
        assert_eq!(local.by_model["gpt-5.5"].input, 3);
        assert!(shared.by_day.is_empty());
        let first_hour = start.div_euclid(3600) * 3600;
        let last_hour = end.div_euclid(3600) * 3600;
        assert_eq!(shared.by_hour.len(), 2);
        assert_eq!(
            shared.breakdown.by_hour_provider[&first_hour]["codex"].input,
            1
        );
        assert_eq!(
            shared.breakdown.by_hour_provider[&last_hour]["codex"].input,
            2
        );
    }

    #[test]
    fn quota_is_reported_not_inferred_from_missing_metadata() {
        assert!(limits_from_codex(&serde_json::json!({}), 10).is_empty());
        let windows = limits_from_codex(
            &serde_json::json!({"rateLimits":{"primary":{"usedPercent":25.0,"resetsAt":100}}}),
            10,
        );
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].resets_at, Some(100));
    }
}
