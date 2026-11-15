//! Connected-mail metadata alongside the existing provider-neutral Inbox view.
//! These records contain content and provenance, never connection credentials.

use crate::{connections::ServiceProvider, InboxCard, InboxConversation, InboxFilter};
use serde::{Deserialize, Serialize};

mod compose;
pub use compose::*;

/// Shared attachment buffer ceiling; the Gmail JSON transport also enforces its
/// own 64 MiB encoded response limit before decoding any part.
pub const MAIL_ATTACHMENT_MAX_BYTES: u64 = 64 << 20;

/// A suggested basename, never a filesystem destination or a rendered URL.
pub fn attachment_file_name(name: &str) -> String {
    let name: String = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .chars()
        .filter(|ch| !ch.is_control())
        .collect();
    if name.trim().is_empty() || matches!(name.as_str(), "." | "..") {
        "attachment".into()
    } else {
        name
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailAddress {
    pub name: String,
    pub address: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailEnvelope {
    pub from: Vec<MailAddress>,
    pub to: Vec<MailAddress>,
    pub cc: Vec<MailAddress>,
    pub bcc: Vec<MailAddress>,
    pub reply_to: Vec<MailAddress>,
    pub subject: String,
    pub message_id: Option<String>,
    pub in_reply_to: Vec<String>,
    pub references: Vec<String>,
    pub headers: Vec<MailHeader>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailAttachment {
    pub id: String,
    pub part_id: String,
    pub external_id: Option<String>,
    pub file_name: String,
    pub content_type: String,
    pub size_bytes: u64,
    pub content_id: Option<String>,
    pub inline: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailMessageDetail {
    pub id: String,
    pub external_id: String,
    pub label_ids: Vec<String>,
    pub envelope: MailEnvelope,
    pub attachments: Vec<MailAttachment>,
    /// The safe plain-text projection was derived from an HTML alternative.
    pub body_from_html: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectedMailMetadata {
    pub provider: ServiceProvider,
    pub mailbox_id: String,
    pub external_thread_id: String,
    pub messages: Vec<MailMessageDetail>,
}

/// Canonical server record. A client replica applies its explicit host namespace
/// separately; an imported Email label does not establish this provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectedMailThread {
    #[serde(flatten)]
    pub conversation: InboxConversation,
    pub connected_mail: ConnectedMailMetadata,
}

pub fn connected_mail_card(
    conversation: &InboxConversation,
    metadata: &ConnectedMailMetadata,
) -> InboxCard {
    let mut card = InboxCard::from(conversation);
    let incoming = conversation
        .messages
        .iter()
        .rev()
        .find(|message| message.direction == crate::MessageDirection::Incoming);
    let detail = incoming
        .or_else(|| conversation.messages.last())
        .and_then(|message| {
            metadata
                .messages
                .iter()
                .find(|detail| detail.id == message.id)
        });
    if let Some(detail) = detail {
        let address = if incoming.is_some() {
            detail.envelope.from.first()
        } else {
            detail.envelope.to.first()
        };
        card.sender = address.map_or_else(
            || {
                if incoming.is_some() {
                    "Unknown sender"
                } else {
                    "Unknown recipient"
                }
                .to_owned()
            },
            |address| {
                if address.name.is_empty() {
                    address.address.clone()
                } else {
                    address.name.clone()
                }
            },
        );
    }
    card
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxSummary {
    pub mailbox_id: String,
    pub provider: ServiceProvider,
    pub last_connection_id: String,
    pub account_label: String,
    pub revision: i64,
    pub threads: u64,
    pub drafts: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxesSnapshot {
    pub host_id: String,
    pub mailboxes: Vec<MailboxSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailBrowseQuery {
    pub host_id: String,
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub filter: InboxFilter,
    #[serde(default)]
    pub page: u64,
    pub expected_revision: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailThreadRow {
    pub card: InboxCard,
    pub external_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailThreadsPage {
    pub host_id: String,
    pub mailbox_id: String,
    pub revision: i64,
    pub page: u64,
    pub total_matches: u64,
    pub rows: Vec<MailThreadRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GmailSyncRequest {
    pub host_id: String,
    pub connection_id: String,
    pub mailbox_id: Option<String>,
    #[serde(default)]
    pub restart: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailSyncResult {
    pub host_id: String,
    pub mailbox_id: String,
    pub revision: i64,
    pub threads_updated: usize,
    pub full_scan_complete: bool,
    pub caught_up: bool,
}

#[cfg(test)]
mod tests {
    use super::attachment_file_name;
    #[test]
    fn attachment_names_remain_unicode_basenames_without_paths_or_control_bytes() {
        for (input, expected) in [
            ("../../設計.pdf", "設計.pdf"),
            ("C:\\mail\\café.txt", "café.txt"),
            ("folder/name\r\n\0.txt", "name.txt"),
            ("", "attachment"),
            ("..", "attachment"),
            ("folder/", "attachment"),
        ] {
            assert_eq!(attachment_file_name(input), expected);
        }
    }
}
