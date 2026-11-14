//! Private local ACP configuration and an append-only measured-usage journal.
use bruhs_models::agent_extensions::{ExtensionConfig, QuotaWindow, UsageRecord, UsageSnapshot};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::{BufRead, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

static WRITER: Mutex<()> = Mutex::new(());
pub mod history;

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

fn private_directory(root: &Path) -> std::io::Result<()> {
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
            let config: ExtensionConfig =
                serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
            config.validate().map_err(std::io::Error::other)?;
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
    let _guard = WRITER
        .lock()
        .map_err(|_| std::io::Error::other("Agent configuration lock failed."))?;
    private_directory(root)?;
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
                    provider: "codex".into(),
                    account: account.clone(),
                    name: name.into(),
                    used_percent: used,
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
            provider: "claude".into(),
            account: "current".into(),
            name: info["rateLimitType"].as_str().unwrap_or("account").into(),
            used_percent: used * 100.0,
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
    let config = load_config_at(root)?;
    let mut snapshot = UsageSnapshot {
        imported: history::snapshot_at(root, &config.prices)?,
        ..UsageSnapshot::default()
    };
    let file = match std::fs::File::open(root.join("usage.jsonl")) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(snapshot),
        Err(e) => return Err(e),
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut limits: BTreeMap<(String, String, String), QuotaWindow> = BTreeMap::new();
    for (line_number, line) in std::io::BufReader::new(file).lines().enumerate() {
        let line = line?;
        let entry:Entry=serde_json::from_str(&line).map_err(|e|std::io::Error::other(format!("Usage journal line {} is invalid: {e}. Restore the private usage journal from backup.",line_number+1)))?;
        match entry {
            Entry::Turn(record) if seen.insert(record.id.clone()) => {
                let price = config.prices.get(&record.model);
                snapshot
                    .by_model
                    .entry(record.model.clone())
                    .or_default()
                    .add(&record, price);
                snapshot
                    .by_project
                    .entry(record.project.clone())
                    .or_default()
                    .add(&record, price);
                let day = chrono::DateTime::from_timestamp(record.at, 0)
                    .ok_or_else(|| std::io::Error::other("Usage timestamp is invalid."))?
                    .format("%Y-%m-%d")
                    .to_string();
                snapshot.by_day.entry(day).or_default().add(&record, price);
            }
            Entry::Turn(_) => {}
            Entry::Limit(window) => {
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
        }
    }
    snapshot.limits = limits.into_values().collect();
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn journal_deduplicates_a_reconnected_completion_and_retains_unknown_price() {
        let root = tempfile::tempdir().unwrap();
        for _ in 0..2 {
            record_at(
                root.path(),
                UsageRecord {
                    id: "receipt".into(),
                    thread_id: "thread".into(),
                    model: "model".into(),
                    project: "project".into(),
                    at: 0,
                    input: 100,
                    output: 20,
                    cache_read: 30,
                    cache_write: 0,
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
