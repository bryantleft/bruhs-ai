//! Provider presentation helpers shared by the composer and the model dialog.
//! Which provider serves a model is `bruhs_models::agent::provider_for_model_id`,
//! the routing the server uses too.

use crate::ui::DropdownOption;
use bruhs_models::agent::{provider_for_model_id, AgentProvider};

pub(super) fn model_dropdown_option(
    id: impl Into<String>,
    label: impl Into<String>,
    provider: AgentProvider,
) -> DropdownOption {
    DropdownOption::new(id, label)
        .in_group(provider_label(provider))
        .with_mark(provider_logo_src(provider))
}

pub fn provider_logo_src_for_model(model_id: &str) -> &'static str {
    provider_logo_src(provider_for_model_id(model_id))
}

/// Display name for a provider.
pub fn provider_label(provider: AgentProvider) -> &'static str {
    match provider {
        AgentProvider::Claude => "Anthropic",
        AgentProvider::Codex => "OpenAI Codex",
        _ => provider.label(),
    }
}

/// Monochrome mark for a provider.
pub fn provider_logo_src(provider: AgentProvider) -> &'static str {
    match provider {
        AgentProvider::Claude => "/logos/claude.svg",
        AgentProvider::Codex => "/logos/openai.svg",
        AgentProvider::Cursor => "/logos/cursor.svg",
        AgentProvider::Antigravity => "/logos/antigravity.svg",
        AgentProvider::Grok => "/logos/grok.svg",
        AgentProvider::OpenCode => "/logos/opencode.svg",
        AgentProvider::Acp => "/logos/agent.svg",
    }
}

pub fn provider_agent_label_for_model(model_id: &str) -> &'static str {
    match provider_for_model_id(model_id) {
        AgentProvider::Claude => "claude agent",
        AgentProvider::Codex => "codex agent",
        AgentProvider::Cursor => "cursor agent",
        AgentProvider::Antigravity => "antigravity agent",
        AgentProvider::Grok => "grok agent",
        AgentProvider::OpenCode => "opencode agent",
        AgentProvider::Acp => "acp agent",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn providers_map_to_labels_and_logos() {
        assert_eq!(provider_label(AgentProvider::Claude), "Anthropic");
        assert_eq!(provider_label(AgentProvider::Codex), "OpenAI Codex");
        assert_eq!(
            provider_logo_src(AgentProvider::Claude),
            "/logos/claude.svg"
        );
        assert_eq!(provider_logo_src(AgentProvider::Codex), "/logos/openai.svg");
    }

    #[test]
    fn provider_label_follows_model() {
        assert_eq!(
            provider_logo_src_for_model("claude-sonnet-4-6"),
            "/logos/claude.svg"
        );
        assert_eq!(provider_logo_src_for_model("gpt-5.5"), "/logos/openai.svg");
        assert_eq!(
            provider_agent_label_for_model("claude-sonnet-4-6"),
            "claude agent"
        );
        assert_eq!(provider_agent_label_for_model("gpt-5.5"), "codex agent");
        // The shared routing: bare Claude aliases are Claude's.
        assert_eq!(provider_agent_label_for_model("opus"), "claude agent");
    }
}
