//! Bounded RFC MIME generation from checked sender/recipient/file inputs.
//! Bytes are generated once for durable review, rather than regenerated at send.

use super::ComposeError;
use crate::mail::{
    CacheError, DraftAttachmentBytes, MailAuthorizationRef, MailCacheHostId, MailDraftRef,
    MailSession,
};
use bruhs_models::{mail::*, EpochMs};
use mail_builder::{headers::address::Address, MessageBuilder};
use mail_parser::MessageParser;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    collections::HashSet,
    fmt,
    io::{self, Write},
};

/// Constructed only by a mail adapter after reading the current leased profile.
/// Its identity is checked against an actual session again during preparation.
#[derive(Clone)]
pub struct VerifiedMailSender {
    authorization: MailAuthorizationRef,
    from: MailAddress,
}

impl fmt::Debug for VerifiedMailSender {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerifiedMailSender").finish_non_exhaustive()
    }
}

impl VerifiedMailSender {
    pub(crate) fn check_session(&self, session: &MailSession) -> Result<(), ComposeError> {
        self.authorization
            .validate_current(session.authorization())
            .map_err(CacheError::from)?;
        Ok(())
    }
    pub(in crate::mail) fn from_profile(
        session: &MailSession,
        from: MailAddress,
    ) -> Result<Self, ComposeError> {
        let check = MailComposeContent {
            to: from.address.clone(),
            ..Default::default()
        }
        .validate_send()
        .map_err(|_| ComposeError::InvalidSender)?;
        if check.to.len() != 1
            || !check.to[0].name.is_empty()
            || check.to[0].address != from.address
            || from.name.chars().any(char::is_control)
            || from.name.len() > 8192
        {
            return Err(ComposeError::InvalidSender);
        }
        Ok(Self {
            authorization: session.authorization().clone(),
            from,
        })
    }

    pub fn address(&self) -> &MailAddress {
        &self.from
    }
}

pub struct PreparedMime {
    content_sha256: String,
    from: MailAddress,
    recipients: MailRecipients,
    files: Vec<MailDraftFile>,
    message_id: String,
    reply_thread: Option<String>,
    reply_headers: Option<(String, Vec<String>)>,
    sha256: String,
    bytes: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedMetadata {
    version: u8,
    content_sha256: String,
    from: MailAddress,
    recipients: MailRecipients,
    files: Vec<MailDraftFile>,
    message_id: String,
    reply_thread: Option<String>,
    reply_headers: Option<(String, Vec<String>)>,
    sha256: String,
    size_bytes: usize,
}

impl fmt::Debug for PreparedMime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedMime")
            .field("bytes", &self.bytes.len())
            .field("sha256", &self.sha256)
            .finish_non_exhaustive()
    }
}

impl PreparedMime {
    /// Private storage only. This representation is never accepted from HTTP.
    pub(crate) fn into_stored(self) -> Result<(String, Vec<u8>), ComposeError> {
        let metadata = PreparedMetadata {
            version: 1,
            content_sha256: self.content_sha256,
            from: self.from,
            recipients: self.recipients,
            files: self.files,
            message_id: self.message_id,
            reply_thread: self.reply_thread,
            reply_headers: self.reply_headers,
            sha256: self.sha256,
            size_bytes: self.bytes.len(),
        };
        Ok((
            serde_json::to_string(&metadata).map_err(|_| ComposeError::PreparedCorrupt)?,
            self.bytes,
        ))
    }

    pub(crate) fn from_stored(metadata: &str, bytes: Vec<u8>) -> Result<Self, ComposeError> {
        if metadata.len() > 256 << 10 || bytes.len() > MAIL_PREPARED_MIME_MAX_BYTES {
            return Err(ComposeError::PreparedCorrupt);
        }
        let metadata: PreparedMetadata =
            serde_json::from_str(metadata).map_err(|_| ComposeError::PreparedCorrupt)?;
        if metadata.version != 1
            || metadata.size_bytes != bytes.len()
            || metadata.sha256 != format!("{:x}", Sha256::digest(&bytes))
            || header_id(&metadata.message_id).as_deref() != Some(metadata.message_id.as_str())
            || metadata.files.len() > MAIL_DRAFT_MAX_ATTACHMENTS
        {
            return Err(ComposeError::PreparedCorrupt);
        }
        Ok(Self {
            content_sha256: metadata.content_sha256,
            from: metadata.from,
            recipients: metadata.recipients,
            files: metadata.files,
            message_id: metadata.message_id,
            reply_thread: metadata.reply_thread,
            reply_headers: metadata.reply_headers,
            sha256: metadata.sha256,
            bytes,
        })
    }

    pub(crate) fn check_content(&self, content: &MailComposeContent) -> Result<(), ComposeError> {
        if self.content_sha256 != content_digest(content)?
            || self.recipients != content.validate_send()?
        {
            return Err(ComposeError::StaleReview);
        }
        Ok(())
    }

    pub(crate) fn check_reply_source(
        &self,
        reference: &MailDraftRef,
        content: &MailComposeContent,
        target: Option<&MailReplyTarget>,
        thread: Option<&ConnectedMailThread>,
    ) -> Result<(), ComposeError> {
        let current = reply_headers(&MimeDraft {
            reference,
            source_revision: 1,
            content,
            reply: target.zip(thread),
            files: &[],
            date: EpochMs::UNIX_EPOCH,
        })?;
        if current != self.reply_headers
            || self.reply_thread.as_deref()
                != reference
                    .reply_to
                    .as_ref()
                    .map(|thread| thread.external_id.as_str())
        {
            return Err(ComposeError::InvalidReply);
        }
        Ok(())
    }

    pub fn from(&self) -> &MailAddress {
        &self.from
    }
    pub fn recipients(&self) -> &MailRecipients {
        &self.recipients
    }
    pub fn files(&self) -> &[MailDraftFile] {
        &self.files
    }
    pub fn message_id(&self) -> &str {
        &self.message_id
    }
    pub fn reply_thread(&self) -> Option<&str> {
        self.reply_thread.as_deref()
    }
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

pub(super) struct MimeDraft<'a> {
    pub reference: &'a MailDraftRef,
    pub source_revision: i64,
    pub content: &'a MailComposeContent,
    pub reply: Option<(&'a MailReplyTarget, &'a ConnectedMailThread)>,
    pub files: &'a [DraftAttachmentBytes],
    pub date: EpochMs,
}

pub(super) struct MimeAuthority {
    pub host: MailCacheHostId,
    pub authorization: MailAuthorizationRef,
}

fn content_digest(content: &MailComposeContent) -> Result<String, ComposeError> {
    let content = serde_json::to_vec(content).map_err(|_| ComposeError::PreparedCorrupt)?;
    Ok(format!("{:x}", Sha256::digest(content)))
}

fn address(value: &MailAddress) -> Address<'_> {
    if value.name.is_empty() {
        Address::from(value.address.as_str())
    } else {
        Address::from((value.name.as_str(), value.address.as_str()))
    }
}

fn correlation_id(
    host: &MailCacheHostId,
    reference: &MailDraftRef,
    revision: i64,
) -> Result<String, ComposeError> {
    let mut hash = Sha256::new();
    hash.update(b"bruhs.mail.outgoing.message.v1");
    let revision = revision.to_string();
    for value in [
        host.as_str(),
        reference.canonical_id.as_str(),
        revision.as_str(),
    ] {
        hash.update(
            u64::try_from(value.len())
                .map_err(|_| ComposeError::InvalidDraft)?
                .to_be_bytes(),
        );
        hash.update(value.as_bytes());
    }
    Ok(format!("bruhs.{:x}@bruhs.ai", hash.finalize()))
}

fn header_id(value: &str) -> Option<String> {
    let value = value.trim();
    let value = value
        .strip_prefix('<')
        .and_then(|value| value.strip_suffix('>'))
        .unwrap_or(value);
    if value.is_empty()
        || value.len() > 900
        || !value.contains('@')
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_graphic() || matches!(byte, b'<' | b'>'))
    {
        return None;
    }
    let bytes = format!("Message-ID: <{value}>\r\n\r\n");
    let parsed = MessageParser::default().parse_headers(bytes.as_bytes())?;
    (parsed.message_id() == Some(value)).then(|| value.to_string())
}

fn reply_headers(draft: &MimeDraft<'_>) -> Result<Option<(String, Vec<String>)>, ComposeError> {
    match (&draft.reference.reply_to, draft.reply) {
        (None, None) => Ok(None),
        (Some(reference), Some((target, thread))) => {
            let metadata = &thread.connected_mail;
            if metadata.mailbox_id != reference.mailbox.as_str()
                || metadata.provider != reference.mailbox.provider()
                || metadata.external_thread_id != reference.external_id
                || target.external_thread_id != reference.external_id
                || thread.conversation.id != reference.canonical_id
                || thread.conversation.source != bruhs_models::InboxSource::Email
                || thread.conversation.external_id.as_deref()
                    != Some(reference.external_id.as_str())
            {
                return Err(ComposeError::InvalidReply);
            }
            let detail = metadata
                .messages
                .iter()
                .find(|message| message.id == target.message_id)
                .filter(|detail| {
                    thread.conversation.messages.iter().any(|message| {
                        message.id == detail.id
                            && message.external_id.as_deref() == Some(detail.external_id.as_str())
                    })
                })
                .ok_or(ComposeError::InvalidReply)?;
            if draft.content.subject != detail.envelope.subject {
                return Err(ComposeError::ReplySubjectChanged);
            }
            let anchor = detail
                .envelope
                .message_id
                .as_deref()
                .and_then(header_id)
                .ok_or(ComposeError::InvalidReply)?;
            let mut seen = HashSet::from([anchor.clone()]);
            let mut references = Vec::new();
            let mut bytes = anchor.len();
            // Limit parsing and copying too, rather than collecting a large
            // adversarial chain and repeatedly shifting/trimming it afterward.
            for value in detail.envelope.references.iter().rev().take(100) {
                if references.len() == 99 {
                    break;
                }
                if let Some(id) = header_id(value) {
                    if seen.insert(id.clone()) && bytes.saturating_add(id.len()) <= 32768 {
                        bytes += id.len();
                        references.push(id);
                    }
                }
            }
            references.reverse();
            references.push(anchor.clone());
            Ok(Some((anchor, references)))
        }
        _ => Err(ComposeError::InvalidReply),
    }
}

struct BoundedMime(Vec<u8>);
impl Write for BoundedMime {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self
            .0
            .len()
            .checked_add(bytes.len())
            .is_none_or(|size| size > MAIL_PREPARED_MIME_MAX_BYTES)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "MIME size limit",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn prepare(
    authority: &MimeAuthority,
    sender: &VerifiedMailSender,
    draft: MimeDraft<'_>,
) -> Result<PreparedMime, ComposeError> {
    draft
        .reference
        .validate()
        .map_err(CacheError::from)
        .map_err(crate::mail::MailError::from)?;
    sender
        .authorization
        .validate_current(&authority.authorization)
        .map_err(CacheError::from)
        .map_err(crate::mail::MailError::from)?;
    if draft.reference.mailbox != *authority.authorization.mailbox()
        || draft.source_revision <= 0
        || !(0..=253_402_300_799_999).contains(&draft.date.as_millis())
    {
        return Err(ComposeError::InvalidDraft);
    }
    let recipients = draft.content.validate_send()?;
    let reply = reply_headers(&draft)?;
    let message_id = correlation_id(&authority.host, draft.reference, draft.source_revision)?;
    let mut message = MessageBuilder::new()
        .from(address(&sender.from))
        .message_id(message_id.clone())
        .date(draft.date.as_millis().div_euclid(1000))
        .subject(draft.content.subject.as_str())
        .text_body(draft.content.body.as_str());
    if !recipients.to.is_empty() {
        message = message.to(recipients.to.iter().map(address).collect::<Vec<_>>());
    }
    if !recipients.cc.is_empty() {
        message = message.cc(recipients.cc.iter().map(address).collect::<Vec<_>>());
    }
    if !recipients.bcc.is_empty() {
        message = message.bcc(recipients.bcc.iter().map(address).collect::<Vec<_>>());
    }
    if let Some((anchor, references)) = &reply {
        message = message
            .in_reply_to(anchor.clone())
            .references(references.clone());
    }
    let mut files = Vec::new();
    let mut total = 0usize;
    for id in &draft.content.attachments {
        let file = draft
            .files
            .iter()
            .find(|file| &file.metadata.id == id)
            .ok_or(ComposeError::AttachmentChanged)?;
        total = total
            .checked_add(file.bytes.len())
            .filter(|total| *total <= MAIL_DRAFT_ATTACHMENT_MAX_BYTES)
            .ok_or(ComposeError::AttachmentChanged)?;
        if u64::try_from(file.bytes.len()).ok() != Some(file.metadata.size_bytes)
            || format!("{:x}", Sha256::digest(&file.bytes)) != file.metadata.sha256
            || file.metadata.file_name != attachment_file_name(&file.metadata.file_name)
        {
            return Err(ComposeError::AttachmentChanged);
        }
        message = message.attachment(
            "application/octet-stream",
            file.metadata.file_name.as_str(),
            file.bytes.as_slice(),
        );
        files.push(MailDraftFile {
            id: file.metadata.id.clone(),
            file_name: file.metadata.file_name.clone(),
            size_bytes: file.metadata.size_bytes,
            sha256: file.metadata.sha256.clone(),
        });
    }
    let mut output = BoundedMime(Vec::new());
    message
        .write_to(&mut output)
        .map_err(|_| ComposeError::PreparedTooLarge)?;
    let sha256 = format!("{:x}", Sha256::digest(&output.0));
    Ok(PreparedMime {
        content_sha256: content_digest(draft.content)?,
        from: sender.from.clone(),
        recipients,
        files,
        message_id,
        reply_thread: draft
            .reference
            .reply_to
            .as_ref()
            .map(|reference| reference.external_id.clone()),
        reply_headers: reply,
        sha256,
        bytes: output.0,
    })
}

#[cfg(test)]
mod tests;
