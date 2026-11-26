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

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelPrice {
    pub input_per_million: f64,
    pub output_per_million: f64,
    #[serde(default)]
    pub cache_read_per_million: f64,
    #[serde(default)]
    pub cache_write_per_million: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fast: Option<TokenPrice>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenPrice {
    pub input_per_million: f64,
    pub output_per_million: f64,
    #[serde(default)]
    pub cache_read_per_million: f64,
    #[serde(default)]
    pub cache_write_per_million: f64,
}

impl TokenPrice {
    fn valid(self) -> bool {
        [
            self.input_per_million,
            self.output_per_million,
            self.cache_read_per_million,
            self.cache_write_per_million,
        ]
        .into_iter()
        .all(|rate| rate.is_finite() && rate >= 0.)
    }
}

impl ModelPrice {
    pub fn standard(&self) -> TokenPrice {
        TokenPrice {
            input_per_million: self.input_per_million,
            output_per_million: self.output_per_million,
            cache_read_per_million: self.cache_read_per_million,
            cache_write_per_million: self.cache_write_per_million,
        }
    }

    fn effective(&self, speed: UsageSpeed) -> TokenPrice {
        if speed == UsageSpeed::Fast {
            self.fast.unwrap_or_else(|| self.standard())
        } else {
            self.standard()
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageModelPricing {
    pub price: Option<ModelPrice>,
    pub alias: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageModelPricingChange {
    pub model: String,
    pub expected: UsageModelPricing,
    pub edit: UsageModelPricing,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageModelFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<UsageModelMatch>>,
    #[serde(default)]
    pub exclude: Vec<UsageModelMatch>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageModelMatch {
    #[serde(default)]
    pub provider: Option<crate::agent::AgentProvider>,
    pub model: String,
}

impl UsageModelFilter {
    pub fn allows(&self, provider: crate::agent::AgentProvider, model: &str) -> bool {
        let matches = |rule: &UsageModelMatch| {
            rule.model == model && rule.provider.is_none_or(|expected| expected == provider)
        };
        self.include
            .as_ref()
            .is_none_or(|rules| rules.iter().any(matches))
            && !self.exclude.iter().any(matches)
    }

    fn valid(&self) -> bool {
        self.include
            .iter()
            .flatten()
            .chain(&self.exclude)
            .all(|rule| valid_usage_model_id(&rule.model))
    }
}

#[cfg(test)]
mod usage_filter_tests {
    use super::*;
    use crate::agent::AgentProvider;

    #[test]
    fn malformed_filter_is_ignored_but_an_empty_include_matches_nothing() {
        let invalid: ExtensionConfig = serde_json::from_str(
            r#"{
            "usage_model_filter": {"include":[{"provider":"unknown","model":"m"}]},
            "prices":{"m":{"input_per_million":1,"output_per_million":2}}
        }"#,
        )
        .unwrap();
        assert!(invalid.usage_model_filter.is_none());
        assert!(invalid.prices.contains_key("m"));
        let empty: ExtensionConfig = serde_json::from_str(
            r#"{
            "usage_model_filter": {"include":[]}
        }"#,
        )
        .unwrap();
        assert!(!empty
            .usage_model_filter
            .unwrap()
            .allows(AgentProvider::Codex, "m"));
    }
}

fn deserialize_usage_model_filter<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<UsageModelFilter>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value::<UsageModelFilter>(value)
        .ok()
        .filter(UsageModelFilter::valid))
}

pub fn valid_usage_model_id(model: &str) -> bool {
    !model.is_empty() && model == model.trim() && !model.chars().any(char::is_control)
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionConfig {
    #[serde(default)]
    pub acp: Vec<AcpInstance>,
    #[serde(default)]
    pub prices: BTreeMap<String, ModelPrice>,
    /// Display and price a recorded model under another model's ID.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub usage_model_aliases: BTreeMap<String, String>,
    #[serde(
        default,
        deserialize_with = "deserialize_usage_model_filter",
        skip_serializing_if = "Option::is_none"
    )]
    pub usage_model_filter: Option<UsageModelFilter>,
}

impl ExtensionConfig {
    pub fn usage_model_pricing(&self, model: &str) -> UsageModelPricing {
        UsageModelPricing {
            price: self.prices.get(model).cloned(),
            alias: self.usage_model_aliases.get(model).cloned(),
        }
    }

    pub fn usage_model<'a>(&'a self, model: &'a str) -> &'a str {
        self.resolve_usage_model(model).unwrap_or(model)
    }

    fn resolve_usage_model<'a>(&'a self, mut model: &'a str) -> Option<&'a str> {
        for _ in 0..=self.usage_model_aliases.len() {
            match self.usage_model_aliases.get(model) {
                Some(target) => model = target,
                None => return Some(model),
            }
        }
        None
    }

    pub fn validate(&self) -> Result<(), String> {
        let mut ids = std::collections::BTreeSet::new();
        for agent in &self.acp {
            agent.validate()?;
            if !ids.insert(&agent.id) {
                return Err(format!("Duplicate ACP instance {}.", agent.id));
            }
        }
        for price in self.prices.values() {
            if !price.standard().valid() || price.fast.is_some_and(|rates| !rates.valid()) {
                return Err(
                    "Model prices must be finite, non-negative USD per million tokens.".into(),
                );
            }
        }
        for (source, target) in &self.usage_model_aliases {
            if !valid_usage_model_id(source) || !valid_usage_model_id(target) {
                return Err("Model mappings require valid model IDs.".into());
            }
            if self.resolve_usage_model(source).is_none() {
                return Err(format!("The model mapping for {source} forms a loop."));
            }
        }
        if self
            .usage_model_filter
            .as_ref()
            .is_some_and(|filter| !filter.valid())
        {
            return Err("Usage filters require valid model IDs.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageRecord {
    pub id: String,
    pub thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub model: String,
    pub project: String,
    pub at: i64,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    #[serde(default, skip_serializing_if = "UsageSpeed::is_unknown")]
    pub requested_speed: UsageSpeed,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli_cost_usd: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageSpeed {
    Standard,
    Fast,
    #[default]
    Unknown,
}

impl UsageSpeed {
    fn is_unknown(&self) -> bool {
        *self == Self::Unknown
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UsageEstimate {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

impl UsageEstimate {
    pub fn for_record(record: &UsageRecord, price: &ModelPrice) -> Self {
        Self::for_rates(record, price.effective(record.requested_speed))
    }

    fn for_rates(record: &UsageRecord, price: TokenPrice) -> Self {
        Self {
            input: record.input.to_f64_lossy() * price.input_per_million / 1_000_000.,
            output: record.output.to_f64_lossy() * price.output_per_million / 1_000_000.,
            cache_read: record.cache_read.to_f64_lossy() * price.cache_read_per_million
                / 1_000_000.,
            cache_write: record.cache_write.to_f64_lossy() * price.cache_write_per_million
                / 1_000_000.,
        }
    }

    pub fn total(&self) -> f64 {
        self.input + self.output + self.cache_read + self.cache_write
    }

    pub fn merge(&mut self, other: Self) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priced_usd: Option<f64>,
}

impl UsageTotal {
    pub fn add(&mut self, record: &UsageRecord, price: Option<&ModelPrice>) {
        self.turns = self.turns.saturating_add(1);
        self.input = self.input.saturating_add(record.input);
        self.output = self.output.saturating_add(record.output);
        self.cache_read = self.cache_read.saturating_add(record.cache_read);
        self.cache_write = self.cache_write.saturating_add(record.cache_write);
        if let Some(price) = price {
            self.priced_usd = Some(
                self.priced_subtotal().unwrap_or_default()
                    + UsageEstimate::for_record(record, price).total(),
            );
        } else {
            self.unpriced_turns = self.unpriced_turns.saturating_add(1);
            self.estimated_usd = None;
        }
        if self.unpriced_turns == 0 {
            self.estimated_usd = self.priced_usd;
        }
    }

    pub fn priced_subtotal(&self) -> Option<f64> {
        self.priced_usd.or(self.estimated_usd)
    }

    fn merge(&mut self, other: Self) {
        self.priced_usd = match (self.priced_subtotal(), other.priced_subtotal()) {
            (Some(left), Some(right)) => Some(left + right),
            (left, right) => left.or(right),
        };
        self.turns = self.turns.saturating_add(other.turns);
        self.unpriced_turns = self.unpriced_turns.saturating_add(other.unpriced_turns);
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
        self.cache_read = self.cache_read.saturating_add(other.cache_read);
        self.cache_write = self.cache_write.saturating_add(other.cache_write);
        self.estimated_usd = match (self.estimated_usd, other.estimated_usd) {
            (Some(left), Some(right)) => Some(left + right),
            _ => None,
        };
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageLimitSnapshot {
    #[serde(default)]
    pub environments: Vec<String>,
    #[serde(default)]
    pub providers: Vec<String>,
    pub accounts: Vec<UsageLimitAccount>,
    pub notices: Vec<String>,
    pub external: Vec<UsageManagementPage>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageManagementPage {
    pub label: String,
    pub url: String,
    pub sources: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageLimitAccount {
    pub provider: String,
    pub label: String,
    pub email: Option<String>,
    pub workspace: Option<String>,
    pub on_device: bool,
    pub via: Vec<String>,
    pub zero_spend_cap: bool,
    pub plan: Option<String>,
    pub sources: Vec<String>,
    pub windows: Vec<QuotaWindow>,
    pub credits: usize,
    pub reset: Option<UsageCreditChoice>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageCreditChoice {
    pub handle: uuid::Uuid,
    pub hub: String,
    pub expires: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UsageSnapshot {
    pub by_model: BTreeMap<String, UsageTotal>,
    pub by_project: BTreeMap<String, UsageTotal>,
    pub by_day: BTreeMap<String, UsageTotal>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub by_hour: BTreeMap<i64, UsageTotal>,
    pub limits: Vec<QuotaWindow>,
    pub errors: Vec<String>,
    #[serde(default)]
    pub imported: ImportedUsageSnapshot,
    #[serde(default)]
    pub breakdown: UsageHistoryBreakdown,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UsageHistoryBreakdown {
    pub by_day_provider: BTreeMap<String, BTreeMap<String, UsageTotal>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub by_hour_provider: BTreeMap<i64, BTreeMap<String, UsageTotal>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sessions_by_provider: BTreeMap<String, u64>,
    #[serde(default)]
    pub cache_savings: CacheSavings,
    pub cost_by_type: UsageEstimate,
    pub cost_by_speed: BTreeMap<UsageSpeed, f64>,
    #[serde(default)]
    pub fast_pricing: FastPricing,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub cli_costs: BTreeMap<String, CliCost>,
}

/// Cost reported by the provider CLI. Subscription billing is independent.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CliCost {
    pub runs: u64,
    pub usd: f64,
}

impl CliCost {
    pub fn add(&mut self, usd: f64) {
        self.runs = self.runs.saturating_add(1);
        self.usd += usd;
    }
}

/// Coverage and estimated price difference for turns that requested Fast.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FastPricing {
    pub tier_priced_turns: u64,
    pub standard_fallback_turns: u64,
    pub unpriced_turns: u64,
    pub premium_usd: f64,
    #[serde(default)]
    pub includes_legacy_report: bool,
}

impl FastPricing {
    pub fn add_fast_turn(
        &mut self,
        record: &UsageRecord,
        price: Option<&ModelPrice>,
        estimate: Option<&UsageEstimate>,
    ) {
        match (price, estimate) {
            (Some(price), Some(estimate)) if price.fast.is_some() => {
                self.tier_priced_turns = self.tier_priced_turns.saturating_add(1);
                self.premium_usd +=
                    estimate.total() - UsageEstimate::for_rates(record, price.standard()).total();
            }
            (Some(_), Some(_)) => {
                self.standard_fallback_turns = self.standard_fallback_turns.saturating_add(1)
            }
            _ => self.unpriced_turns = self.unpriced_turns.saturating_add(1),
        }
    }

    fn recognize_legacy(&mut self, has_fast_cost: bool) {
        self.includes_legacy_report |=
            has_fast_cost && self.tier_priced_turns == 0 && self.standard_fallback_turns == 0;
    }

    fn merge(&mut self, other: Self) {
        self.tier_priced_turns = self
            .tier_priced_turns
            .saturating_add(other.tier_priced_turns);
        self.standard_fallback_turns = self
            .standard_fallback_turns
            .saturating_add(other.standard_fallback_turns);
        self.unpriced_turns = self.unpriced_turns.saturating_add(other.unpriced_turns);
        self.premium_usd += other.premium_usd;
        self.includes_legacy_report |= other.includes_legacy_report;
    }
}

/// Estimated difference between charging cached input at the uncached rate and its configured cache rates.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CacheSavings {
    pub priced_tokens: u64,
    pub estimated_usd: f64,
}

impl CacheSavings {
    pub fn add(&mut self, record: &UsageRecord, price: &ModelPrice, estimate: &UsageEstimate) {
        let cached = record.cache_read.saturating_add(record.cache_write);
        self.priced_tokens = self.priced_tokens.saturating_add(cached);
        self.estimated_usd += cached.to_f64_lossy()
            * price.effective(record.requested_speed).input_per_million
            / 1_000_000.
            - estimate.cache_read
            - estimate.cache_write;
    }

    fn merge(&mut self, other: Self) {
        self.priced_tokens = self.priced_tokens.saturating_add(other.priced_tokens);
        self.estimated_usd += other.estimated_usd;
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct UsageHistoryPeriod {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub days: Option<u16>,
    /// End of the rolling 24-hour period in Unix seconds, exclusive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_24h_ending_at: Option<i64>,
    pub provider: Option<crate::agent::AgentProvider>,
}

/// Aggregate history that an environment can share without project or thread identifiers.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SharedUsageHistory {
    pub by_model: BTreeMap<String, UsageTotal>,
    pub by_day: BTreeMap<String, UsageTotal>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub by_hour: BTreeMap<i64, UsageTotal>,
    pub breakdown: UsageHistoryBreakdown,
}

impl From<UsageSnapshot> for SharedUsageHistory {
    fn from(snapshot: UsageSnapshot) -> Self {
        Self {
            by_model: snapshot.by_model,
            by_day: snapshot.by_day,
            by_hour: snapshot.by_hour,
            breakdown: snapshot.breakdown,
        }
    }
}

impl SharedUsageHistory {
    pub fn merge(&mut self, mut other: Self) {
        self.breakdown
            .fast_pricing
            .recognize_legacy(self.breakdown.cost_by_speed.contains_key(&UsageSpeed::Fast));
        other.breakdown.fast_pricing.recognize_legacy(
            other
                .breakdown
                .cost_by_speed
                .contains_key(&UsageSpeed::Fast),
        );
        self.breakdown
            .fast_pricing
            .merge(other.breakdown.fast_pricing);
        fn merge_totals<K: Ord>(
            totals: &mut BTreeMap<K, UsageTotal>,
            incoming: BTreeMap<K, UsageTotal>,
        ) {
            use std::collections::btree_map::Entry;
            for (key, total) in incoming {
                match totals.entry(key) {
                    Entry::Occupied(mut entry) => entry.get_mut().merge(total),
                    Entry::Vacant(entry) => {
                        entry.insert(total);
                    }
                }
            }
        }
        merge_totals(&mut self.by_model, other.by_model);
        merge_totals(&mut self.by_day, other.by_day);
        merge_totals(&mut self.by_hour, other.by_hour);
        for (day, providers) in other.breakdown.by_day_provider {
            merge_totals(
                self.breakdown.by_day_provider.entry(day).or_default(),
                providers,
            );
        }
        for (hour, providers) in other.breakdown.by_hour_provider {
            merge_totals(
                self.breakdown.by_hour_provider.entry(hour).or_default(),
                providers,
            );
        }
        for (provider, cost) in other.breakdown.cli_costs {
            let total = self.breakdown.cli_costs.entry(provider).or_default();
            total.runs = total.runs.saturating_add(cost.runs);
            total.usd += cost.usd;
        }
        for (provider, sessions) in other.breakdown.sessions_by_provider {
            let total = self
                .breakdown
                .sessions_by_provider
                .entry(provider)
                .or_default();
            *total = total.saturating_add(sessions);
        }
        self.breakdown
            .cache_savings
            .merge(other.breakdown.cache_savings);
        self.breakdown
            .cost_by_type
            .merge(other.breakdown.cost_by_type);
        for (speed, cost) in other.breakdown.cost_by_speed {
            *self.breakdown.cost_by_speed.entry(speed).or_default() += cost;
        }
    }
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
            session_id: None,
            model: "local".into(),
            project: "p".into(),
            at: 0,
            input: 100,
            output: 20,
            cache_read: 500,
            cache_write: 30,
            requested_speed: Default::default(),
            cli_cost_usd: None,
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
                fast: None,
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
            ..Default::default()
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
            )]),
            ..Default::default()
        }
        .validate()
        .is_err());
    }
}
