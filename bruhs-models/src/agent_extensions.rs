//! Portable configuration and measured usage for local agent runtimes.
use crate::num::ToFloatLossy;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AcpInstance {
    pub id: String,
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub models: Vec<String>,
    /// Provider-native mode IDs. ACP modes are not an operating-system sandbox.
    #[serde(default)]
    pub modes: BTreeMap<String, String>,
    /// ACP v1 config-option identifiers. Omission uses `mode` / `model`.
    #[serde(default)]
    pub mode_config_id: Option<String>,
    #[serde(default)]
    pub model_config_id: Option<String>,
    #[serde(default)]
    pub enabled: bool,
}

impl AcpInstance {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty()
            || !self
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            return Err("ACP ID must contain only letters, numbers, '-' and '_'.".into());
        }
        if self.name.trim().is_empty()
            || self.command.trim().is_empty()
            || self.command.starts_with('-')
            || self.command.contains('\0')
            || self.args.iter().any(|a| a.contains('\0'))
        {
            return Err(
                "ACP needs a name, an executable, and literal arguments without NUL bytes.".into(),
            );
        }
        for model in &self.models {
            format!("acp/{}/{model}", self.id)
                .parse::<crate::agent::ModelId>()
                .map_err(|e| e.to_string())?;
        }
        if [&self.mode_config_id, &self.model_config_id]
            .into_iter()
            .flatten()
            .any(|id| id.is_empty() || id.chars().any(char::is_control))
        {
            return Err(
                "ACP selector IDs must be nonempty identifiers without control characters.".into(),
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelPrice {
    pub input_per_million: f64,
    pub output_per_million: f64,
    #[serde(default)]
    pub cache_read_per_million: f64,
    #[serde(default)]
    pub cache_write_per_million: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionConfig {
    #[serde(default)]
    pub acp: Vec<AcpInstance>,
    #[serde(default)]
    pub prices: BTreeMap<String, ModelPrice>,
}

impl ExtensionConfig {
    pub fn validate(&self) -> Result<(), String> {
        let mut ids = std::collections::BTreeSet::new();
        for agent in &self.acp {
            agent.validate()?;
            if !ids.insert(&agent.id) {
                return Err(format!("Duplicate ACP instance {}.", agent.id));
            }
        }
        for price in self.prices.values() {
            if [
                price.input_per_million,
                price.output_per_million,
                price.cache_read_per_million,
                price.cache_write_per_million,
            ]
            .iter()
            .any(|n| !n.is_finite() || *n < 0.0)
            {
                return Err(
                    "Model prices must be finite, non-negative USD per million tokens.".into(),
                );
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageRecord {
    pub id: String,
    pub thread_id: String,
    pub model: String,
    pub project: String,
    pub at: i64,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UsageTotal {
    pub turns: u64,
    pub unpriced_turns: u64,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub estimated_usd: Option<f64>,
}

impl UsageTotal {
    pub fn add(&mut self, record: &UsageRecord, price: Option<&ModelPrice>) {
        self.turns = self.turns.saturating_add(1);
        self.input = self.input.saturating_add(record.input);
        self.output = self.output.saturating_add(record.output);
        self.cache_read = self.cache_read.saturating_add(record.cache_read);
        self.cache_write = self.cache_write.saturating_add(record.cache_write);
        if price.is_none() {
            self.unpriced_turns = self.unpriced_turns.saturating_add(1);
            self.estimated_usd = None;
        }
        if let Some(p) = price.filter(|_| self.unpriced_turns == 0) {
            self.estimated_usd = Some(
                self.estimated_usd.unwrap_or(0.0)
                    + (record.input.to_f64_lossy() * p.input_per_million
                        + record.output.to_f64_lossy() * p.output_per_million
                        + record.cache_read.to_f64_lossy() * p.cache_read_per_million
                        + record.cache_write.to_f64_lossy() * p.cache_write_per_million)
                        / 1_000_000.0,
            );
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QuotaWindow {
    pub provider: String,
    pub account: String,
    pub name: String,
    pub used_percent: f64,
    pub resets_at: Option<i64>,
    pub observed_at: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UsageSnapshot {
    pub by_model: BTreeMap<String, UsageTotal>,
    pub by_project: BTreeMap<String, UsageTotal>,
    pub by_day: BTreeMap<String, UsageTotal>,
    pub limits: Vec<QuotaWindow>,
    pub errors: Vec<String>,
    #[serde(default)]
    pub imported: ImportedUsageSnapshot,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ImportedUsageSnapshot {
    pub by_model: BTreeMap<String, UsageTotal>,
    pub by_project: BTreeMap<String, UsageTotal>,
    pub by_day: BTreeMap<String, UsageTotal>,
    pub records: usize,
    pub imported_at: Option<i64>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageHistorySource {
    Claude,
    Codex,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageHistoryRequest {
    pub source: UsageHistorySource,
    pub root: Option<String>,
    pub max_files: usize,
    pub max_bytes: u64,
    #[serde(default)]
    pub offset: usize,
    /// Returned by the first page; binds later offsets to the same sorted file inventory.
    #[serde(default)]
    pub inventory: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageHistoryPreview {
    pub id: String,
    pub source: UsageHistorySource,
    pub root: String,
    pub inventory: String,
    pub files_read: usize,
    pub bytes_read: u64,
    pub next_offset: Option<usize>,
    /// A cut file is retried from its beginning with at least this scan budget.
    pub minimum_next_bytes: Option<u64>,
    pub limited: bool,
    pub warnings: Vec<String>,
    pub usage: ImportedUsageSnapshot,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_prices_remain_unknown_and_cache_is_not_counted_as_input() {
        let record = UsageRecord {
            id: "turn".into(),
            thread_id: "thread".into(),
            model: "local".into(),
            project: "p".into(),
            at: 0,
            input: 100,
            output: 20,
            cache_read: 500,
            cache_write: 30,
        };
        let mut total = UsageTotal::default();
        total.add(&record, None);
        assert_eq!(total.input, 100);
        assert_eq!(total.cache_read, 500);
        assert_eq!(total.estimated_usd, None);
        let mut priced = UsageTotal::default();
        priced.add(
            &record,
            Some(&ModelPrice {
                input_per_million: 2.0,
                output_per_million: 10.0,
                cache_read_per_million: 0.2,
                cache_write_per_million: 2.0,
            }),
        );
        assert!((priced.estimated_usd.unwrap() - 0.00056).abs() < 1e-12);
    }
    #[test]
    fn rejects_duplicate_agents_and_invalid_prices() {
        let agent = AcpInstance {
            id: "a".into(),
            name: "Agent".into(),
            command: "agent".into(),
            args: vec![],
            models: vec!["default".into()],
            modes: BTreeMap::new(),
            mode_config_id: None,
            model_config_id: None,
            enabled: false,
        };
        assert!(agent.validate().is_ok());
        assert!(ExtensionConfig {
            acp: vec![agent.clone(), agent],
            prices: BTreeMap::new()
        }
        .validate()
        .is_err());
        assert!(ExtensionConfig {
            acp: vec![],
            prices: BTreeMap::from([(
                "m".into(),
                ModelPrice {
                    input_per_million: -1.0,
                    ..ModelPrice::default()
                }
            )])
        }
        .validate()
        .is_err());
    }
}
