//! Public metadata and private submissions for a harness credential request.

use crate::ids::ThreadId;
use serde::{Deserialize, Serialize};

/// The operation the user must review before supplying a credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretOperation {
    pub name: String,
    pub reason: String,
    pub env_var: String,
    /// Executable followed by literal arguments. Shells must be explicit.
    pub command: Vec<String>,
}

impl SecretOperation {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self
            .name
            .chars()
            .chain(self.reason.chars())
            .chain(self.command.iter().flat_map(|arg| arg.chars()))
            .any(is_format_control)
        {
            return Err("Credential request text cannot contain invisible formatting controls.");
        }
        if self.name.trim().is_empty() || self.reason.trim().is_empty() {
            return Err("A credential name and reason are required.");
        }
        if self.command.first().is_none_or(|s| s.trim().is_empty())
            || self.command.iter().any(|s| s.contains('\0'))
        {
            return Err("Provide an executable and literal arguments without NUL characters.");
        }
        if self
            .name
            .chars()
            .chain(self.reason.chars())
            .any(char::is_control)
        {
            return Err("Credential names and reasons cannot contain control characters.");
        }
        let mut chars = self.env_var.chars();
        let valid = chars
            .next()
            .is_some_and(|c| c.is_ascii_uppercase() || c == '_')
            && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
        if !valid
            || matches!(
                self.env_var.as_str(),
                "PATH"
                    | "HOME"
                    | "SHELL"
                    | "ENV"
                    | "BASH_ENV"
                    | "ZDOTDIR"
                    | "IFS"
                    | "SHELLOPTS"
                    | "BASHOPTS"
                    | "NODE_OPTIONS"
                    | "PYTHONPATH"
                    | "PYTHONHOME"
            )
            || ["LD_", "DYLD_", "BRUHS_"]
                .iter()
                .any(|prefix| self.env_var.starts_with(prefix))
        {
            return Err(
                "Use an uppercase credential variable, not a process-control or Bruhs variable.",
            );
        }
        Ok(())
    }
}

fn is_format_control(c: char) -> bool {
    matches!(c, '\u{00ad}' | '\u{061c}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')
}

/// Preserve readable Unicode while making controls in host-supplied paths visible.
pub fn review_text(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_control() || is_format_control(c) {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    out
}

/// Safe to stream and display. This type never contains a credential value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretRequest {
    pub id: String,
    pub thread_id: ThreadId,
    pub operation: SecretOperation,
    pub cwd: String,
    pub saved_available: bool,
    #[serde(default)]
    pub storage_unavailable: bool,
}

/// Sent only to the private submission endpoint, never to the provider.
/// Deliberately has no Debug or Clone implementation.
#[derive(Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum SecretDecision {
    Provide { value: String, save: bool },
    UseSaved,
    ForgetSaved,
    Decline,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretSubmission {
    pub thread_id: ThreadId,
    pub request_id: String,
    pub decision: SecretDecision,
}

/// Metadata only. Neither listings nor mutation responses include a credential value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedCredential {
    pub id: String,
    pub name: String,
    pub project_path: String,
    pub state: SavedCredentialState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedCredentialState {
    Saved,
    NeedsAttention,
}

/// Write-only replacement, separate from all public metadata types.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialReplacement {
    pub value: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_review_cannot_be_reordered_by_unicode_format_controls() {
        let operation = SecretOperation {
            name: "Key".into(),
            reason: "Deploy".into(),
            env_var: "API_KEY".into(),
            command: vec!["safe\u{202e}destructive".into()],
        };
        assert!(operation.validate().is_err());
        assert_eq!(
            review_text("/tmp/\u{202e}hidden\n"),
            "/tmp/\\u{202e}hidden\\n"
        );
        assert_eq!(review_text("/tmp/日本語"), "/tmp/日本語");
    }

    #[test]
    fn credential_requests_reject_missing_operations_and_process_control_variables() {
        let mut operation = SecretOperation {
            name: "Service key".into(),
            reason: "Deploy this project".into(),
            env_var: "SERVICE_API_KEY".into(),
            command: vec!["deploy".into()],
        };
        assert!(operation.validate().is_ok());
        operation.command.clear();
        assert!(operation.validate().is_err());
        operation.command.push("deploy".into());
        for name in [
            "PATH",
            "HOME",
            "BASH_ENV",
            "DYLD_INSERT_LIBRARIES",
            "LD_PRELOAD",
            "invalid=name",
        ] {
            operation.env_var = name.into();
            assert!(operation.validate().is_err(), "{name}");
        }
    }
}
