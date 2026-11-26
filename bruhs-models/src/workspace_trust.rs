//! Shared Workspace Trust wire types and summary copy.
//!
//! Untrusted repositories keep project-local hooks, MCP servers, agent
//! settings, submodules, and background Git fetches isolated until the user
//! explicitly trusts the folder.

use serde::{Deserialize, Serialize};

pub const PATH: &str = "/api/agent/workspace-trust";
pub const SETTINGS_TITLE: &str = "Trusted folders";
pub const SETTINGS_DESCRIPTION: &str =
    "Revoke saved trust for future runs. Linked worktrees can inherit trust from a trusted main checkout.";

/// Category of executable or side-effect-capable repository configuration
/// detected by passive, symlink-safe filesystem inspection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceRiskKind {
    /// Executable Git config entries in `.git/config` (`core.fsmonitor`,
    /// `core.hooksPath`, `core.sshCommand`, `diff.*.textconv`, `filter.*`, etc.).
    GitConfig,
    /// Non-sample hook files in `.git/hooks`.
    GitHooks,
    /// Submodule definitions in `.gitmodules`.
    GitSubmodules,
    /// Project-local Claude Code settings (`.claude/settings*.json`).
    ClaudeConfig,
    /// Project-local MCP server definitions (`.mcp.json`).
    McpConfig,
    /// Project-local OpenCode config (`opencode.json*`, `.opencode/`).
    OpenCodeConfig,
    /// Other project-local agent configs (`.codex/config.toml`, `.cursor/*`).
    AgentConfig,
    /// Project scripts in `.bruhs/config.json`.
    ProjectScripts,
}

impl WorkspaceRiskKind {
    /// Short human label for one risk category.
    pub const fn short_label(self) -> &'static str {
        match self {
            Self::GitConfig => "Git config",
            Self::GitHooks => "Git hooks",
            Self::GitSubmodules => "Git submodules",
            Self::ClaudeConfig => "Claude settings",
            Self::McpConfig => "MCP servers",
            Self::OpenCodeConfig => "OpenCode config",
            Self::AgentConfig => "Agent config",
            Self::ProjectScripts => "Project scripts",
        }
    }
}

/// One detected executable repository configuration item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceRiskItem {
    pub kind: WorkspaceRiskKind,
    pub label: String,
    pub detail: String,
}

/// Trust state and passively detected risks for a project directory.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceTrustStatus {
    pub path: String,
    pub trusted: bool,
    #[serde(default)]
    pub risks: Vec<WorkspaceRiskItem>,
}

impl WorkspaceTrustStatus {
    /// Whether the chat banner should prompt the user about isolated repo configs.
    pub fn should_warn(&self) -> bool {
        !self.trusted && !self.risks.is_empty()
    }
}

/// Request body for workspace inspection or revocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceTrustQuery {
    pub path: String,
}

/// Request body for `PUT /api/agent/workspace-trust`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceTrustUpdate {
    pub path: String,
    pub trusted: bool,
}

/// Concise banner copy describing which untrusted repo configs are isolated.
pub fn banner_summary(risks: &[WorkspaceRiskItem]) -> String {
    let mut labels: Vec<&'static str> = Vec::new();
    for risk in risks {
        let label = risk.kind.short_label();
        if !labels.contains(&label) {
            labels.push(label);
        }
    }
    if labels.is_empty() {
        return "Untrusted folder — repo hooks, MCP servers, and project settings are isolated until you trust it.".to_string();
    }
    format!(
        "Untrusted folder — isolated repo configs detected ({}). Hooks, project MCP servers, submodules, and auto-fetch stay blocked until you trust it.",
        labels.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_summary_deduplicates_risk_labels_in_order() {
        let status = WorkspaceTrustStatus {
            path: "/tmp/repo".into(),
            trusted: false,
            risks: vec![
                WorkspaceRiskItem {
                    kind: WorkspaceRiskKind::GitConfig,
                    label: "Git config".into(),
                    detail: "core.fsmonitor".into(),
                },
                WorkspaceRiskItem {
                    kind: WorkspaceRiskKind::GitConfig,
                    label: "Git config".into(),
                    detail: "core.hooksPath".into(),
                },
                WorkspaceRiskItem {
                    kind: WorkspaceRiskKind::McpConfig,
                    label: "MCP servers".into(),
                    detail: ".mcp.json".into(),
                },
            ],
        };
        assert!(status.should_warn());
        let summary = banner_summary(&status.risks);
        assert!(summary.contains("(Git config, MCP servers)"), "{summary}");
    }

    #[test]
    fn trusted_or_risk_free_status_does_not_warn() {
        let clean = WorkspaceTrustStatus {
            path: "/tmp/repo".into(),
            trusted: false,
            risks: Vec::new(),
        };
        assert!(!clean.should_warn());
        let trusted = WorkspaceTrustStatus {
            path: "/tmp/repo".into(),
            trusted: true,
            risks: vec![WorkspaceRiskItem {
                kind: WorkspaceRiskKind::McpConfig,
                label: "MCP servers".into(),
                detail: ".mcp.json".into(),
            }],
        };
        assert!(!trusted.should_warn());
    }
}
