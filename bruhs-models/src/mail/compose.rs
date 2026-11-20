//! Editable compose text and provider-neutral review/receipt wire contracts.
//! Incomplete recipient text remains savable; sending requires separate checks.

mod reply;
pub use reply::reply_content;

use super::MailAddress;
use email_address::EmailAddress;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fmt, str::FromStr as _};

pub const MAIL_DRAFT_ATTACHMENT_MAX_BYTES: usize = 16 << 20;
pub const MAIL_DRAFT_MAX_ATTACHMENTS: usize = 20;
pub const MAIL_COMPOSE_BODY_MAX_BYTES: usize = 1 << 20;
pub const MAIL_COMPOSE_RECIPIENTS_MAX_BYTES: usize = 32 << 10;
pub const MAIL_COMPOSE_SUBJECT_MAX_BYTES: usize = 8 << 10;
pub const MAIL_COMPOSE_MAX_RECIPIENTS: usize = 100;
pub const MAIL_PREPARED_MIME_MAX_BYTES: usize = 32 << 20;

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MailComposeContent {
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    pub body: String,
    pub attachments: Vec<String>,
}

impl fmt::Debug for MailComposeContent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MailComposeContent")
            .field("body_bytes", &self.body.len())
            .field("attachments", &self.attachments.len())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailRecipientField {
    To,
    Cc,
    Bcc,
}
impl fmt::Display for MailRecipientField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::To => "To",
            Self::Cc => "Cc",
            Self::Bcc => "Bcc",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MailComposeError {
    #[error("Keep each recipient field within 32 KiB.")]
    RecipientsTooLarge,
    #[error("Keep the subject within 8 KiB.")]
    SubjectTooLarge,
    #[error("Keep the message body within 1 MiB.")]
    BodyTooLarge,
    #[error("Keep this draft within 20 attached files.")]
    TooManyAttachments,
    #[error("An attachment reference is invalid or repeated. Choose the file again.")]
    InvalidAttachment,
    #[error("Remove line breaks or control characters from the subject before sending.")]
    InvalidSubject,
    #[error("Remove unsupported control characters from the message body before sending.")]
    InvalidBody,
    #[error(
        "Check recipient {index} in {field}. Use an email address, optionally with a display name."
    )]
    InvalidRecipient {
        field: MailRecipientField,
        index: usize,
    },
    #[error("Add at least one recipient in To, Cc or Bcc before reviewing the email.")]
    MissingRecipients,
    #[error("Keep the email within 100 recipients.")]
    TooManyRecipients,
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailRecipients {
    pub to: Vec<MailAddress>,
    pub cc: Vec<MailAddress>,
    pub bcc: Vec<MailAddress>,
}
impl fmt::Debug for MailRecipients {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MailRecipients")
            .field("to_count", &self.to.len())
            .field("cc_count", &self.cc.len())
            .field("bcc_count", &self.bcc.len())
            .finish()
    }
}

impl MailComposeContent {
    /// Bounds only: do not discard incomplete or invalid user text while saving.
    pub fn validate_storage(&self) -> Result<(), MailComposeError> {
        if [&self.to, &self.cc, &self.bcc]
            .iter()
            .any(|field| field.len() > MAIL_COMPOSE_RECIPIENTS_MAX_BYTES)
        {
            return Err(MailComposeError::RecipientsTooLarge);
        }
        if self.subject.len() > MAIL_COMPOSE_SUBJECT_MAX_BYTES {
            return Err(MailComposeError::SubjectTooLarge);
        }
        if self.body.len() > MAIL_COMPOSE_BODY_MAX_BYTES {
            return Err(MailComposeError::BodyTooLarge);
        }
        if self.attachments.len() > MAIL_DRAFT_MAX_ATTACHMENTS {
            return Err(MailComposeError::TooManyAttachments);
        }
        let mut ids = HashSet::new();
        for id in &self.attachments {
            if id.is_empty()
                || id.len() > 256
                || !id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
                || !ids.insert(id)
            {
                return Err(MailComposeError::InvalidAttachment);
            }
        }
        Ok(())
    }

    /// Parse every nonempty token; a malformed entry never silently disappears.
    /// First occurrence wins across To, Cc and Bcc, preserving explicit placement.
    pub fn validate_send(&self) -> Result<MailRecipients, MailComposeError> {
        self.validate_storage()?;
        if self.subject.chars().any(char::is_control) {
            return Err(MailComposeError::InvalidSubject);
        }
        if self
            .body
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\r' | '\n' | '\t'))
        {
            return Err(MailComposeError::InvalidBody);
        }
        let mut seen = HashSet::new();
        let to = recipients(&self.to, MailRecipientField::To, &mut seen)?;
        let cc = recipients(&self.cc, MailRecipientField::Cc, &mut seen)?;
        let bcc = recipients(&self.bcc, MailRecipientField::Bcc, &mut seen)?;
        let count = to.len().saturating_add(cc.len()).saturating_add(bcc.len());
        if count == 0 {
            return Err(MailComposeError::MissingRecipients);
        }
        if count > MAIL_COMPOSE_MAX_RECIPIENTS {
            return Err(MailComposeError::TooManyRecipients);
        }
        Ok(MailRecipients { to, cc, bcc })
    }
}

fn tokens(value: &str, field: MailRecipientField) -> Result<Vec<&str>, MailComposeError> {
    if value.chars().any(char::is_control) {
        return Err(MailComposeError::InvalidRecipient { field, index: 1 });
    }
    let mut quoted = false;
    let mut escaped = false;
    let mut angle = false;
    let mut start = 0;
    let mut result = Vec::new();
    for (index, ch) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if quoted && ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            quoted = !quoted;
            continue;
        }
        if !quoted {
            match ch {
                '<' if !angle => angle = true,
                '>' if angle => angle = false,
                '<' | '>' => {
                    return Err(MailComposeError::InvalidRecipient {
                        field,
                        index: result.len().saturating_add(1),
                    })
                }
                ',' | ';' if !angle => {
                    let token = value[start..index].trim();
                    if !token.is_empty() {
                        result.push(token);
                    }
                    start = index + ch.len_utf8();
                }
                _ => {}
            }
        }
        if result.len() > 500 {
            return Err(MailComposeError::TooManyRecipients);
        }
    }
    if quoted || escaped || angle {
        return Err(MailComposeError::InvalidRecipient {
            field,
            index: result.len().saturating_add(1),
        });
    }
    let last = value[start..].trim();
    if !last.is_empty() {
        result.push(last);
    }
    Ok(result)
}

fn display_name(name: &str) -> String {
    let name = name.trim();
    if let Some(inner) = name
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    {
        let mut result = String::new();
        let mut escaped = false;
        for ch in inner.chars() {
            if escaped {
                result.push(ch);
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else {
                result.push(ch);
            }
        }
        result
    } else {
        name.into()
    }
}

fn recipients(
    value: &str,
    field: MailRecipientField,
    seen: &mut HashSet<String>,
) -> Result<Vec<MailAddress>, MailComposeError> {
    let mut result = Vec::new();
    for (index, token) in tokens(value, field)?.into_iter().enumerate() {
        let address =
            EmailAddress::from_str(token).map_err(|_| MailComposeError::InvalidRecipient {
                field,
                index: index.saturating_add(1),
            })?;
        let key = format!(
            "{}@{}",
            address.local_part(),
            address.domain().to_lowercase()
        );
        if seen.insert(key) {
            result.push(MailAddress {
                name: display_name(address.display_part()),
                address: address.email(),
            });
        }
    }
    Ok(result)
}

/// The canonical selected message must belong to this cached provider thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MailReplyTarget {
    pub external_thread_id: String,
    pub message_id: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailDraftFile {
    pub id: String,
    pub file_name: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MailDraftCreate {
    pub host_id: String,
    pub client_id: String,
    pub reply: Option<MailReplyTarget>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MailDraftSave {
    pub host_id: String,
    pub expected_revision: i64,
    pub content: MailComposeContent,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct MailComposeDraft {
    pub host_id: String,
    pub mailbox_id: String,
    pub client_id: String,
    pub draft_id: String,
    pub revision: i64,
    pub reply: Option<MailReplyTarget>,
    pub content: MailComposeContent,
    pub uploads: Vec<MailDraftFile>,
    pub latest_send: Option<MailSendStatus>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct MailDraftSummary {
    pub client_id: String,
    pub draft_id: String,
    pub revision: i64,
    pub subject: String,
    pub recipients: String,
    pub updated_at_ms: i64,
    pub pending_send: Option<MailDispatchState>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct MailDraftsPage {
    pub host_id: String,
    pub mailbox_id: String,
    pub revision: i64,
    pub page: u64,
    pub total_matches: u64,
    pub rows: Vec<MailDraftSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailDispatchState {
    Queued,
    Dispatching,
    Accepted,
    Uncertain,
    NeedsReview,
    Cancelled,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailSendProblemKind {
    SignIn,
    Permission,
    RateLimited,
    TooLarge,
    InvalidMessage,
    AccountChanged,
    Unavailable,
    Unconfirmed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailSendProblem {
    pub kind: MailSendProblemKind,
    pub retry_after_seconds: Option<u64>,
}

impl MailSendProblem {
    pub fn new(kind: MailSendProblemKind) -> Self {
        Self {
            kind,
            retry_after_seconds: None,
        }
    }
}

impl MailSendProblem {
    /// What this problem means and what to do about it.
    pub fn message(&self) -> &'static str {
        match self.kind {
            MailSendProblemKind::SignIn => "Reconnect this mail account and review the email again.",
            MailSendProblemKind::Permission => "The provider did not permit sending. Check mail access and review again.",
            MailSendProblemKind::RateLimited => "The provider is limiting email. Wait before reviewing again.",
            MailSendProblemKind::TooLarge => "This email exceeds the selected provider's sending limit. Remove files or shorten it.",
            MailSendProblemKind::InvalidMessage => "The provider rejected this email. Check its recipients, subject and files before reviewing again.",
            MailSendProblemKind::AccountChanged => "The connected sender changed. Review the email using its owning account.",
            MailSendProblemKind::Unavailable => "The email provider could not be reached. Open send status before retrying.",
            MailSendProblemKind::Unconfirmed => "Sending could not be confirmed. Check send status and Sent mail before starting another send.",
        }
    }
}

impl fmt::Display for MailSendProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}
impl std::error::Error for MailSendProblem {}

#[derive(Clone, Serialize, Deserialize)]
pub struct MailSendStatus {
    pub host_id: String,
    pub mailbox_id: String,
    pub operation_id: String,
    pub draft_id: String,
    pub draft_revision: i64,
    pub state: MailDispatchState,
    pub provider_message_id: Option<String>,
    pub provider_thread_id: Option<String>,
    #[serde(default)]
    pub problem: Option<MailSendProblem>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct MailSendReview {
    pub host_id: String,
    pub mailbox_id: String,
    pub client_id: String,
    pub draft_id: String,
    pub revision: i64,
    pub connection_id: String,
    pub digest: String,
    pub from: MailAddress,
    pub recipients: MailRecipients,
    pub content: MailComposeContent,
    pub files: Vec<MailDraftFile>,
    pub message_id: String,
    pub mime_size_bytes: u64,
    pub mime_sha256: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MailReviewRequest {
    pub host_id: String,
    pub connection_id: String,
    pub expected_revision: i64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MailSendRequest {
    pub host_id: String,
    pub connection_id: String,
    pub operation_id: String,
    pub revision: i64,
    pub digest: String,
}

/// Explicit Sent lookup only; provider evidence is constructed inside Core.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MailSentReconcileRequest {
    pub host_id: String,
    pub connection_id: String,
}

#[cfg(test)]
mod tests;
