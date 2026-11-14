//! Device-scoped projection of the running native desktop. No credential-form data.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

/// Matches the local workspace bridge's input frame.
pub const REQUEST_BYTES: usize = 64 * 1024;
/// Half a frame bounds phone snapshots and leaves room for metadata.
pub const RESPONSE_BYTES: usize = REQUEST_BYTES / 2;
/// Worst-case JSON escaping needs six bytes per input byte; leave two shares for metadata.
pub const TEXT_BYTES: usize = RESPONSE_BYTES / 8;
/// Reuses the local workspace bridge's page count.
pub const PAGE_SIZE: usize = crate::reactive::HISTORY_LIMIT;
/// Matches the desktop client's initial SSE network cadence. One snapshot per tick.
pub const REFRESH_MS: u64 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MobileScope {
    Read,
    Prompt,
    Approve,
    Diff,
}
pub type MobileScopes = BTreeSet<MobileScope>;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MobileDevice {
    pub id: Uuid,
    pub name: String,
    pub scopes: MobileScopes,
    pub expires_at_ms: i64,
    pub last_seen_ms: Option<i64>,
    pub revoked: bool,
}
/// Private bootstrap input. Deliberately has no Debug implementation.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MobilePairInput {
    pub secret: String,
    pub name: String,
}
/// Private issuance response. Raw tokens never enter the desktop's device catalog.
#[derive(Deserialize, Serialize)]
pub struct MobilePairResult {
    pub token: String,
    pub device: MobileDevice,
    pub next_sequence: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MobileSessionInfo {
    pub device: MobileDevice,
    pub next_sequence: u64,
    pub last_command: Option<MobileCommandReceipt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MobileCommand {
    pub id: Uuid,
    pub sequence: u64,
    pub action: MobileAction,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MobileAction {
    Prompt {
        thread_id: String,
        text: String,
    },
    Permission {
        thread_id: String,
        turn_generation: u64,
        request_id: String,
        review_hash: String,
        decision: MobileDecision,
    },
}
impl MobileAction {
    pub fn scope(&self) -> MobileScope {
        match self {
            Self::Prompt { .. } => MobileScope::Prompt,
            Self::Permission { .. } => MobileScope::Approve,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MobileDecision {
    Allow,
    Deny,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum MobileCommandOutcome {
    Pending,
    Accepted {
        queued: bool,
    },
    Rejected {
        reason: String,
    },
    /// Reservation reached disk, but completion cannot be established. Never replay it.
    Unconfirmed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MobileCommandReceipt {
    pub id: Uuid,
    pub sequence: u64,
    pub outcome: MobileCommandOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MobileThreadState {
    Idle,
    Running,
    NeedsApproval,
    LaptopAction,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MobileThread {
    pub id: String,
    pub title: String,
    pub project: String,
    pub updated_at_ms: i64,
    pub state: MobileThreadState,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MobileThreadPage {
    pub threads: Vec<MobileThread>,
    pub next_offset: Option<usize>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MobileMessage {
    pub index: usize,
    pub role: String,
    pub text: String,
    pub byte_offset: usize,
    pub full_bytes: usize,
    pub streaming: bool,
    pub reference: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MobilePermission {
    pub request_id: String,
    pub turn_generation: u64,
    pub tool: String,
    pub input: String,
    pub review_hash: String,
    /// Oversized/incomplete reviews can be denied, but must be allowed on the laptop.
    pub can_allow: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MobileThreadSnapshot {
    pub thread: MobileThread,
    pub messages: Vec<MobileMessage>,
    pub next_before: Option<usize>,
    pub permission: Option<MobilePermission>,
    pub queued_prompts: usize,
    pub laptop_action_required: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MobileTextPage {
    pub text: String,
    pub offset: usize,
    pub next_offset: Option<usize>,
    pub revision: String,
    pub source_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MobileRead {
    Threads {
        offset: usize,
    },
    Thread {
        thread_id: String,
        before: Option<usize>,
    },
    Message {
        thread_id: String,
        index: usize,
        reference: String,
        offset: usize,
        revision: Option<String>,
    },
    Diff {
        thread_id: String,
        offset: usize,
        revision: Option<String>,
    },
}
impl MobileRead {
    pub fn scope(&self) -> MobileScope {
        match self {
            Self::Diff { .. } => MobileScope::Diff,
            _ => MobileScope::Read,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum MobileReadReply {
    Threads(MobileThreadPage),
    Thread(MobileThreadSnapshot),
    Text(MobileTextPage),
}

/// Byte-based pages preserve UTF-8 and never copy the full message first.
pub fn text_slice(
    text: &str,
    offset: usize,
    budget: usize,
) -> Result<(&str, Option<usize>), &'static str> {
    if offset > text.len() || !text.is_char_boundary(offset) {
        return Err("This text page changed. Reload it before continuing.");
    }
    let mut end = offset.saturating_add(budget).min(text.len());
    while end > offset && !text.is_char_boundary(end) {
        end -= 1;
    }
    if end == offset && offset < text.len() {
        return Err("The text page budget cannot fit its next character.");
    }
    Ok((&text[offset..end], (end < text.len()).then_some(end)))
}
pub fn text_tail(text: &str, budget: usize) -> (usize, &str) {
    let mut start = text.len().saturating_sub(budget);
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    (start, &text[start..])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mobile_text_pages_preserve_utf8_and_bad_offsets_are_actionable() {
        let text = "A🦀東京Z";
        let mut offset = 0;
        let mut restored = String::new();
        loop {
            let (part, next) = text_slice(text, offset, 4).unwrap();
            restored.push_str(part);
            if let Some(next) = next {
                offset = next;
            } else {
                break;
            }
        }
        assert_eq!(restored, text);
        assert!(text_slice(text, 2, 4).is_err());
        let (offset, tail) = text_tail(text, 5);
        assert_eq!(&text[offset..], tail);
        assert!(tail.len() <= 5);
    }
}
