//! Agent domain enums and the model catalog shared by every surface.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::UnknownVariant;

/// How much the agent may do without asking. Wire values are the Claude
/// CLI's: `"default"`, `"acceptEdits"`, `"plan"`, `"bypassPermissions"`.
/// Deserializing rejects anything else; use [`PermissionMode::parse_lossy`]
/// for saved preferences that should fall back to asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    /// Ask before every tool the CLI's rules don't already allow.
    #[default]
    Default,
    /// Approve file edits without asking.
    AcceptEdits,
    /// Plan only; nothing is changed.
    Plan,
    /// Never ask. Only for sandboxes.
    BypassPermissions,
}

impl PermissionMode {
    /// The wire string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::AcceptEdits => "acceptEdits",
            Self::Plan => "plan",
            Self::BypassPermissions => "bypassPermissions",
        }
    }

    /// `s` as a mode, or [`PermissionMode::Default`] (ask first) when it is
    /// not one: the safe reading of a missing or corrupted preference.
    pub fn parse_lossy(s: &str) -> Self {
        s.parse().unwrap_or_default()
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::AcceptEdits => "Auto-accept",
            Self::Plan => "Plan",
            Self::BypassPermissions => "YOLO",
        }
    }

    /// Lucide-style icon name. Render via `<Icon name=… />`.
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Default => "shield",
            Self::AcceptEdits => "shield-check",
            Self::Plan => "clipboard-list",
            Self::BypassPermissions => "hand-metal",
        }
    }

    /// One-line description for the access-mode menus. `provider` is the CLI
    /// that will run the turn; `None` (the Settings default, which applies to
    /// every model) notes where Codex behaves differently.
    ///
    /// Codex has no way to ask for approval through bruhs, so its Default is
    /// read-only rather than "ask first" (see `bruhs-core`'s `codex_access`).
    pub fn description(self, provider: Option<AgentProvider>) -> &'static str {
        match (self, provider) {
            (Self::Default, Some(AgentProvider::Codex)) => "Read-only: Codex can't ask first",
            (Self::Default, Some(AgentProvider::Claude)) => "Ask before edits and commands",
            (Self::Default, None) => "Ask before edits and commands; Codex is read-only",
            (Self::AcceptEdits, Some(AgentProvider::Codex)) => {
                "Edit and run commands in the sandbox"
            }
            (Self::AcceptEdits, _) => "Approve edits, ask for commands",
            (Self::Plan, _) => "Read-only, propose a plan",
            (Self::BypassPermissions, Some(AgentProvider::Codex)) => "Never ask, no sandbox",
            (Self::BypassPermissions, _) => "Never ask",
        }
    }

    /// Tooltip for the composer's access-mode chip. Explains why a Codex
    /// turn in Default mode can't edit, and how to let it.
    pub fn chip_title(self, provider: AgentProvider) -> &'static str {
        match (self, provider) {
            (Self::Default, AgentProvider::Codex) => {
                "Codex can't ask for approval, so Default is read-only. Pick Auto-accept to let it edit."
            }
            _ => "Access mode",
        }
    }

    /// The mode after this one in [`PermissionMode::all`] order, wrapping
    /// around: the status bar's click-to-cycle.
    pub fn next(self) -> Self {
        match self {
            Self::Default => Self::AcceptEdits,
            Self::AcceptEdits => Self::Plan,
            Self::Plan => Self::BypassPermissions,
            Self::BypassPermissions => Self::Default,
        }
    }

    pub fn all() -> &'static [PermissionMode] {
        &[
            Self::Default,
            Self::AcceptEdits,
            Self::Plan,
            Self::BypassPermissions,
        ]
    }
}

impl FromStr for PermissionMode {
    type Err = UnknownVariant;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::all()
            .iter()
            .copied()
            .find(|mode| mode.as_str() == s)
            .ok_or_else(|| UnknownVariant::new("permission mode", s))
    }
}

impl fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which CLI runs a turn. Wire values: `"claude"`, `"codex"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentProvider {
    Claude,
    Codex,
}

impl AgentProvider {
    /// The wire string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    /// The provider that serves `model_id`; see [`provider_for_model_id`].
    pub fn for_model(model_id: &str) -> Self {
        provider_for_model_id(model_id)
    }
}

impl FromStr for AgentProvider {
    type Err = UnknownVariant;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "claude" => Ok(Self::Claude),
            "codex" => Ok(Self::Codex),
            _ => Err(UnknownVariant::new("agent provider", s)),
        }
    }
}

impl fmt::Display for AgentProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Who wrote a chat message. Wire values: `"user"`, `"assistant"`, `"tool"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    /// Tool output recorded as its own message (desktop transcripts).
    Tool,
}

impl Role {
    /// The wire string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }

    /// `s` as a role, or [`Role::Assistant`] when it is not one, the way the
    /// desktop has always read saved transcripts.
    pub fn parse_lossy(s: &str) -> Self {
        s.parse().unwrap_or(Self::Assistant)
    }
}

impl FromStr for Role {
    type Err = UnknownVariant;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user" => Ok(Self::User),
            "assistant" => Ok(Self::Assistant),
            "tool" => Ok(Self::Tool),
            _ => Err(UnknownVariant::new("message role", s)),
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One selectable model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelInfo {
    /// Id passed to the provider CLI (`--model`).
    pub id: &'static str,
    /// Name shown in pickers.
    pub label: &'static str,
    pub provider: AgentProvider,
}

/// Every model the pickers offer, in display order. The single source for
/// the web picker, the desktop picker and provider routing.
pub const MODEL_CATALOG: &[ModelInfo] = &[
    ModelInfo {
        id: "gpt-6.1-sol",
        label: "GPT-6.1 Sol",
        provider: AgentProvider::Codex,
    },
    ModelInfo {
        id: "gpt-5.5",
        label: "GPT-5.5",
        provider: AgentProvider::Codex,
    },
    ModelInfo {
        id: "gpt-5.4",
        label: "GPT-5.4",
        provider: AgentProvider::Codex,
    },
    ModelInfo {
        id: "gpt-5.3-codex",
        label: "GPT-5.3 Codex",
        provider: AgentProvider::Codex,
    },
    ModelInfo {
        id: "claude-fable-5",
        label: "Claude Fable 5",
        provider: AgentProvider::Claude,
    },
    ModelInfo {
        id: "claude-opus-4-8",
        label: "Claude Opus 4.8",
        provider: AgentProvider::Claude,
    },
    ModelInfo {
        id: "claude-sonnet-4-6",
        label: "Claude Sonnet 4.6",
        provider: AgentProvider::Claude,
    },
    ModelInfo {
        id: "claude-haiku-4-5",
        label: "Claude Haiku 4.5",
        provider: AgentProvider::Claude,
    },
];

/// Model used when neither the request nor a saved preference names one.
pub const DEFAULT_AGENT_MODEL: &str = "claude-sonnet-4-6";

/// The catalog entry for `id`.
pub fn model_info(id: &str) -> Option<&'static ModelInfo> {
    MODEL_CATALOG.iter().find(|model| model.id == id)
}

/// Bare model aliases the Claude CLI resolves itself (`--model opus`).
const CLAUDE_MODEL_ALIASES: &[&str] = &["default", "fable", "opus", "sonnet", "haiku", "opusplan"];

/// The provider that serves `model_id`: the catalog's answer when the model
/// is listed, else Claude for the Claude CLI's id forms, else Codex.
///
/// Claude id forms, each optionally followed by a context-window suffix such
/// as `[1m]` (`opus[1m]`, `claude-sonnet-4-5[1m]`): `claude-*` ids (including
/// Vertex `claude-…@date`), the CLI's bare aliases (`opus`, `sonnet`, …) and
/// Bedrock ids and ARNs (`us.anthropic.claude-…`).
pub fn provider_for_model_id(model_id: &str) -> AgentProvider {
    if let Some(model) = model_info(model_id) {
        return model.provider;
    }
    let base = strip_context_suffix(model_id);
    if let Some(model) = model_info(base) {
        return model.provider;
    }
    if base.starts_with("claude-")
        || base.contains("anthropic.claude-")
        || CLAUDE_MODEL_ALIASES.contains(&base)
    {
        AgentProvider::Claude
    } else {
        AgentProvider::Codex
    }
}

/// `model_id` without a trailing `[…]` context-window suffix (`opus[1m]` is
/// `opus`).
fn strip_context_suffix(model_id: &str) -> &str {
    model_id
        .strip_suffix(']')
        .and_then(|rest| rest.rfind('[').map(|open| &model_id[..open]))
        .filter(|base| !base.is_empty())
        .unwrap_or(model_id)
}

/// Longest model id accepted, in bytes.
pub const MAX_MODEL_ID_LEN: usize = 128;

/// A model id that is safe to hand to a provider CLI as `--model <id>`:
/// non-empty, at most [`MAX_MODEL_ID_LEN`] bytes, ASCII letters, digits and
/// `. _ - : / [ ]` only (catalog ids like `claude-sonnet-4-6`, aliases like
/// `opus` or `opus[1m]`, dated ids), and not starting with `-`, so it can
/// never read as a flag. It need not be in [`MODEL_CATALOG`]: a saved
/// preference may name a model the catalog dropped, and the CLI decides
/// whether it exists.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ModelId(String);

/// Why a string is not a [`ModelId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ModelIdError {
    #[error("model id is empty")]
    Empty,
    #[error("model id is longer than {MAX_MODEL_ID_LEN} bytes")]
    TooLong,
    #[error("model id must not start with '-'")]
    LeadingDash,
    #[error("model id contains {0:?}, which is not allowed in model ids")]
    InvalidCharacter(char),
}

impl ModelId {
    /// [`DEFAULT_AGENT_MODEL`].
    pub fn default_agent_model() -> Self {
        Self(DEFAULT_AGENT_MODEL.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }

    /// The provider that serves this model; see [`provider_for_model_id`].
    pub fn provider(&self) -> AgentProvider {
        provider_for_model_id(&self.0)
    }
}

impl TryFrom<String> for ModelId {
    type Error = ModelIdError;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        if raw.is_empty() {
            return Err(ModelIdError::Empty);
        }
        if raw.len() > MAX_MODEL_ID_LEN {
            return Err(ModelIdError::TooLong);
        }
        if raw.starts_with('-') {
            return Err(ModelIdError::LeadingDash);
        }
        if let Some(c) = raw.chars().find(|c| {
            !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '/' | '[' | ']'))
        }) {
            return Err(ModelIdError::InvalidCharacter(c));
        }
        Ok(Self(raw))
    }
}

impl FromStr for ModelId {
    type Err = ModelIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s.to_string())
    }
}

impl From<ModelId> for String {
    fn from(id: ModelId) -> Self {
        id.0
    }
}

impl fmt::Display for ModelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_default_is_described_as_read_only() {
        let codex = PermissionMode::Default.description(Some(AgentProvider::Codex));
        assert!(codex.contains("Read-only"), "{codex}");
        let claude = PermissionMode::Default.description(Some(AgentProvider::Claude));
        assert_eq!(claude, "Ask before edits and commands");
        let any = PermissionMode::Default.description(None);
        assert!(any.contains("Codex is read-only"), "{any}");
    }

    #[test]
    fn every_mode_has_a_description_for_every_provider() {
        for &mode in PermissionMode::all() {
            for provider in [
                None,
                Some(AgentProvider::Claude),
                Some(AgentProvider::Codex),
            ] {
                assert!(!mode.description(provider).is_empty());
            }
        }
    }

    #[test]
    fn only_codex_default_gets_the_read_only_chip_hint() {
        let hint = PermissionMode::Default.chip_title(AgentProvider::Codex);
        assert!(hint.contains("Auto-accept"), "{hint}");
        for &mode in PermissionMode::all() {
            assert_eq!(mode.chip_title(AgentProvider::Claude), "Access mode");
            if mode != PermissionMode::Default {
                assert_eq!(mode.chip_title(AgentProvider::Codex), "Access mode");
            }
        }
    }

    #[test]
    fn permission_mode_next_cycles_in_display_order() {
        let all = PermissionMode::all();
        for (index, mode) in all.iter().enumerate() {
            assert_eq!(mode.next(), all[(index + 1) % all.len()]);
        }
    }

    #[test]
    fn model_ids_accept_catalog_ids_and_aliases_but_not_flags() {
        for model in MODEL_CATALOG {
            assert_eq!(model.id.parse::<ModelId>().unwrap().as_str(), model.id);
        }
        for id in [
            "opus",
            "opus[1m]",
            "claude-sonnet-4-20250514",
            "gpt-5.3-codex",
            "o3",
        ] {
            assert!(id.parse::<ModelId>().is_ok(), "{id}");
        }
        assert_eq!("".parse::<ModelId>(), Err(ModelIdError::Empty));
        assert_eq!(
            "--dangerously-skip-permissions".parse::<ModelId>(),
            Err(ModelIdError::LeadingDash)
        );
        assert_eq!(
            "opus sonnet".parse::<ModelId>(),
            Err(ModelIdError::InvalidCharacter(' '))
        );
        assert_eq!(
            "a".repeat(MAX_MODEL_ID_LEN + 1).parse::<ModelId>(),
            Err(ModelIdError::TooLong)
        );
        assert!(serde_json::from_str::<ModelId>("\"x\ny\"").is_err());
        assert_eq!(
            serde_json::to_string(&ModelId::default_agent_model()).unwrap(),
            format!("\"{DEFAULT_AGENT_MODEL}\"")
        );
        assert_eq!(
            "opus".parse::<ModelId>().unwrap().provider(),
            AgentProvider::Claude
        );
    }

    #[test]
    fn permission_mode_wire_strings_round_trip() {
        for (mode, wire) in [
            (PermissionMode::Default, "default"),
            (PermissionMode::AcceptEdits, "acceptEdits"),
            (PermissionMode::Plan, "plan"),
            (PermissionMode::BypassPermissions, "bypassPermissions"),
        ] {
            assert_eq!(mode.as_str(), wire);
            assert_eq!(serde_json::to_string(&mode).unwrap(), format!("\"{wire}\""));
            assert_eq!(
                serde_json::from_str::<PermissionMode>(&format!("\"{wire}\"")).unwrap(),
                mode
            );
            assert_eq!(wire.parse::<PermissionMode>(), Ok(mode));
            assert_eq!(PermissionMode::parse_lossy(wire), mode);
        }
        assert!(serde_json::from_str::<PermissionMode>("\"typo\"").is_err());
        assert!("bypass".parse::<PermissionMode>().is_err());
        assert_eq!(PermissionMode::parse_lossy("typo"), PermissionMode::Default);
        assert_eq!(PermissionMode::default(), PermissionMode::Default);
    }

    #[test]
    fn provider_and_role_wire_strings() {
        assert_eq!(
            serde_json::to_string(&AgentProvider::Codex).unwrap(),
            "\"codex\""
        );
        assert_eq!("claude".parse::<AgentProvider>(), Ok(AgentProvider::Claude));
        assert!("gateway".parse::<AgentProvider>().is_err());
        for role in [Role::User, Role::Assistant, Role::Tool] {
            let wire = serde_json::to_string(&role).unwrap();
            assert_eq!(wire, format!("\"{}\"", role.as_str()));
            assert_eq!(serde_json::from_str::<Role>(&wire).unwrap(), role);
        }
        assert_eq!(Role::parse_lossy("system"), Role::Assistant);
        assert_eq!(Role::parse_lossy("user"), Role::User);
    }

    #[test]
    fn catalog_matches_both_pickers() {
        // Web `state::available_models` and desktop `DESKTOP_MODELS`.
        let rows: Vec<(&str, &str, &str)> = MODEL_CATALOG
            .iter()
            .map(|m| (m.id, m.label, m.provider.as_str()))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("gpt-6.1-sol", "GPT-6.1 Sol", "codex"),
                ("gpt-5.5", "GPT-5.5", "codex"),
                ("gpt-5.4", "GPT-5.4", "codex"),
                ("gpt-5.3-codex", "GPT-5.3 Codex", "codex"),
                ("claude-fable-5", "Claude Fable 5", "claude"),
                ("claude-opus-4-8", "Claude Opus 4.8", "claude"),
                ("claude-sonnet-4-6", "Claude Sonnet 4.6", "claude"),
                ("claude-haiku-4-5", "Claude Haiku 4.5", "claude"),
            ]
        );
        assert!(model_info(DEFAULT_AGENT_MODEL).is_some());
        let mut ids: Vec<&str> = MODEL_CATALOG.iter().map(|m| m.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), MODEL_CATALOG.len());
    }

    #[test]
    fn provider_routing() {
        // Web/desktop `provider_for_model_id` cases.
        assert_eq!(
            provider_for_model_id("claude-sonnet-4-6"),
            AgentProvider::Claude
        );
        assert_eq!(provider_for_model_id("gpt-5.5"), AgentProvider::Codex);
        // Off-catalog ids keep the prefix rule.
        assert_eq!(
            provider_for_model_id("claude-sonnet-4-20250514"),
            AgentProvider::Claude
        );
        assert_eq!(provider_for_model_id("o3"), AgentProvider::Codex);
        // Bare Claude aliases no longer route to Codex.
        assert_eq!(provider_for_model_id("opus"), AgentProvider::Claude);
        assert_eq!(AgentProvider::for_model("sonnet"), AgentProvider::Claude);
    }

    /// `ModelId` accepts `opus[1m]`; it used to fall through to Codex.
    #[test]
    fn claude_ids_with_a_context_suffix_and_other_cli_forms_route_to_claude() {
        for id in [
            "opus[1m]",
            "sonnet[1m]",
            "default",
            "opusplan",
            "claude-sonnet-4-6[1m]",
            "claude-sonnet-4-5-20250929[1m]",
            "us.anthropic.claude-sonnet-4-20250514-v1:0",
            "anthropic.claude-3-5-haiku-20241022-v1:0",
            "arn:aws:bedrock:us-east-1:123456789012:application-inference-profile/us.anthropic.claude-opus-4-1-20250805-v1:0",
        ] {
            assert!(id.parse::<ModelId>().is_ok(), "{id}");
            assert_eq!(provider_for_model_id(id), AgentProvider::Claude, "{id}");
        }
        // A Vertex id routes to Claude, though `ModelId` does not accept `@`.
        assert_eq!(
            provider_for_model_id("claude-sonnet-4@20250514"),
            AgentProvider::Claude
        );
        for id in [
            "gpt-5.5",
            "gpt-5.5[1m]",
            "o3",
            "[1m]",
            "codex-opus",
            "opus-x",
        ] {
            assert_eq!(provider_for_model_id(id), AgentProvider::Codex, "{id}");
        }
        assert_eq!(strip_context_suffix("opus[1m]"), "opus");
        assert_eq!(strip_context_suffix("opus"), "opus");
        assert_eq!(strip_context_suffix("[1m]"), "[1m]");
        assert_eq!(strip_context_suffix("opus1m]"), "opus1m]");
    }
}
