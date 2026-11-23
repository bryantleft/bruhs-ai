//! Portable configuration and measured usage for local agent runtimes.
use crate::num::ToFloatLossy;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Per-thread Kiro session options. Older clients omit both values.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KiroSelection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}
impl KiroSelection {
    pub fn is_empty(&self) -> bool {
        self.agent.is_none() && self.effort.is_none()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AcpInstance {
    /// Built-in drivers using ACP retain their own launch and access configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver: Option<AcpDriver>,
    pub id: String,
    pub name: String,
    /// Optional #RRGGBB marker displayed next to this instance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<RegistryOrigin>,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "AcpEnvironment::is_empty")]
    pub environment: AcpEnvironment,
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

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AcpDriver {
    Kiro,
}

impl AcpInstance {
    pub fn kiro(id: String, name: String, binary: &str) -> Self {
        use crate::agent::PermissionMode;
        Self {
            driver: Some(AcpDriver::Kiro),
            id,
            name,
            command: if binary.trim().is_empty() {
                "kiro-cli"
            } else {
                binary.trim()
            }
            .into(),
            args: ["acp", "--agent-engine=v3", "--auth-method=cli"]
                .map(String::from)
                .into(),
            modes: [
                (PermissionMode::Default.as_str().into(), "off".into()),
                (
                    PermissionMode::BypassPermissions.as_str().into(),
                    "on".into(),
                ),
            ]
            .into(),
            mode_config_id: Some("autopilot".into()),
            model_config_id: None,
            models: Vec::new(),
            environment: Default::default(),
            accent_color: None,
            registry: None,
            enabled: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistryOrigin {
    pub agent_id: String,
    pub version: String,
    pub distribution: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable_override: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_method_id: Option<String>,
    /// Launch defaults supplied by the installed release, before any user edits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defaults: Option<RegistryDefaults>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistryDefaults {
    pub args: Vec<String>,
    pub environment: BTreeMap<String, String>,
}

/// Preserve the order of variable rows when an instance is reopened.
pub type AcpEnvironment = indexmap::IndexMap<String, AcpVariable>;

/// Only plain variables carry their value in the portable configuration.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AcpVariable {
    Plain { value: String },
    Device { revision: uuid::Uuid },
    OnePassword { source: String, reference: String },
}

/// An ACP session ID is local to the instance that issued it.
pub fn acp_session_matches_model(session: &str, model: &str) -> bool {
    let selected = model
        .strip_prefix("acp/")
        .and_then(|value| value.split_once('/'));
    let owner = session
        .strip_prefix("acp:")
        .and_then(|value| value.split_once(':'));
    matches!((selected, owner), (Some((id, _)), Some((source, native))) if id == source && !native.is_empty())
}

pub fn valid_environment_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Windows process environments compare variable names without case.
pub fn same_environment_name(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

pub fn parse_accent_color(color: &str) -> Option<u32> {
    let hex = color.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

impl AcpInstance {
    /// Keep the managed executable for repairs and upgrades even when chat uses an override.
    pub fn executable(&self) -> &str {
        self.registry
            .as_ref()
            .and_then(|origin| origin.executable_override.as_deref())
            .unwrap_or(&self.command)
    }

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
        if let Some(origin) = &self.registry {
            if origin.executable_override.as_ref().is_some_and(|value| {
                value.trim().is_empty() || value.starts_with('-') || value.contains('\0')
            }) {
                return Err("Enter an executable name or path for the registry override.".into());
            }
            if origin
                .auth_method_id
                .as_ref()
                .is_some_and(|id| id.trim().is_empty() || id.chars().any(char::is_control))
            {
                return Err("Enter an ACP authentication method ID.".into());
            }
        }
        if self
            .accent_color
            .as_deref()
            .is_some_and(|value| parse_accent_color(value).is_none())
        {
            return Err("Enter an accent color as #RRGGBB.".into());
        }
        #[cfg(windows)]
        {
            let mut names = std::collections::BTreeSet::new();
            if !self
                .environment
                .keys()
                .all(|name| names.insert(name.to_ascii_uppercase()))
            {
                return Err("Environment variable names must be unique on this device.".into());
            }
        }
        for (name, value) in &self.environment {
            if !valid_environment_name(name) {
                return Err("Variable names must begin with a letter or underscore and contain only letters, numbers, and underscores.".into());
            }
            match value {
                AcpVariable::Plain { value } if value.contains('\0') => {
                    return Err(format!("The value for {name} contains a NUL character."))
                }
                AcpVariable::OnePassword { source, reference }
                    if source.is_empty() || !reference.starts_with("op://") =>
                {
                    return Err(format!(
                        "Choose a 1Password source and reference for {name}."
                    ))
                }
                _ => {}
            }
        }
        if self.driver == Some(AcpDriver::Kiro) {
            let defaults = Self::kiro(String::new(), String::new(), "");
            if self.registry.is_some()
                || self.args != defaults.args
                || self.modes != defaults.modes
                || self.mode_config_id != defaults.mode_config_id
                || self.model_config_id.is_some()
            {
                return Err(
                    "Kiro instances require the Kiro V3 launch and access settings.".into(),
                );
            }
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
    /// Provider identity when independent windows share a display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_id: Option<String>,
    pub provider: String,
    pub account: String,
    pub name: String,
    pub used_percent: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_minutes: Option<u32>,
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
    fn environment_rows_keep_entry_order_across_serialization() {
        let raw = r##"{"id":"work","name":"Work","command":"agent","environment":{"Z":{"kind":"plain","value":"last alphabetically"},"A":{"kind":"plain","value":""}}}"##;
        let agent: AcpInstance = serde_json::from_str(raw).unwrap();
        let restored: AcpInstance =
            serde_json::from_slice(&serde_json::to_vec(&agent).unwrap()).unwrap();
        assert_eq!(
            restored
                .environment
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["Z", "A"]
        );
    }

    #[test]
    fn optional_acp_accent_round_trips_with_legacy_config() {
        let mut agent: AcpInstance = serde_json::from_value(serde_json::json!({
            "id":"local", "name":"Local", "command":"agent"
        }))
        .unwrap();
        assert!(agent.validate().is_ok());
        assert!(serde_json::to_value(&agent)
            .unwrap()
            .get("accent_color")
            .is_none());
        agent.accent_color = Some("#17B89a".into());
        assert!(agent.validate().is_ok());
        assert_eq!(
            parse_accent_color(agent.accent_color.as_ref().unwrap()),
            Some(0x17b89a)
        );
        assert_eq!(
            serde_json::from_value::<AcpInstance>(serde_json::to_value(&agent).unwrap()).unwrap(),
            agent
        );
        for invalid in ["", "#123", "#aabbccdd", "abcdef", "#+12345", "#az1234"] {
            agent.accent_color = Some(invalid.into());
            assert!(agent.validate().is_err());
        }
    }

    #[test]
    fn rejects_duplicate_agents_and_invalid_prices() {
        let agent = AcpInstance {
            driver: None,
            id: "a".into(),
            name: "Agent".into(),
            registry: None,
            accent_color: None,
            command: "agent".into(),
            args: vec![],
            environment: Default::default(),
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
