use super::{content, wire, GmailClient, GmailError, MailSession, MAX_RESPONSE_BYTES};
use bruhs_models::{connections::ServiceProvider, mail::MailAttachment};

/// Explicitly downloaded data. It is never rendered, logged or opened by the
/// provider adapter. The caller chooses a destination after the user's action.
pub struct GmailAttachment {
    pub metadata: MailAttachment,
    pub bytes: Vec<u8>,
}

impl GmailClient {
    /// Resolve a canonical cached attachment inside the currently leased account.
    /// No caller-supplied provider URL, message id or attachment id grants access.
    pub async fn attachment(
        &self,
        session: &MailSession,
        external_thread: &str,
        canonical_attachment: &str,
    ) -> Result<GmailAttachment, GmailError> {
        let mailbox = session.authorization().mailbox();
        if mailbox.provider() != ServiceProvider::Gmail {
            return Err(GmailError::WrongProvider);
        }
        let cached = session
            .runtime
            .connected_thread(
                session.runtime.host.as_str().to_owned(),
                mailbox.as_str().to_owned(),
                external_thread.to_owned(),
            )
            .await?
            .ok_or(GmailError::NotFound)?;
        let (message, metadata) = cached
            .connected_mail
            .messages
            .iter()
            .find_map(|message| {
                message
                    .attachments
                    .iter()
                    .find(|attachment| attachment.id == canonical_attachment)
                    .map(|attachment| (message, attachment))
            })
            .ok_or(GmailError::NotFound)?;
        let expected_size = metadata.size_bytes;
        if expected_size > u64::try_from(MAX_RESPONSE_BYTES).unwrap_or(u64::MAX) {
            return Err(GmailError::TooLarge);
        }
        let metadata = metadata.clone();
        let body = if let Some(external_id) = metadata.external_id.as_deref() {
            self.get(
                session,
                &["messages", &message.external_id, "attachments", external_id],
                &[],
            )
            .await?
        } else {
            // Gmail may embed a small attachment directly in its MIME part.
            // Fetch only its owning message, never the rest of the mailbox.
            let current: wire::Message = self
                .get(
                    session,
                    &["messages", &message.external_id],
                    &[("format", "full")],
                )
                .await?;
            if current.id != message.external_id || current.thread_id != external_thread {
                return Err(GmailError::InvalidResponse);
            }
            let part = part(&current.payload, &metadata.part_id)?;
            if !content::is_attachment(part) || part.body.size != expected_size {
                return Err(GmailError::InvalidResponse);
            }
            if let Some(external_id) = part.body.attachment_id.as_deref() {
                self.get(
                    session,
                    &["messages", &message.external_id, "attachments", external_id],
                    &[],
                )
                .await?
            } else {
                // Empty leaf files are valid. A MIME container with children
                // and no complete body is not an empty file we can download.
                if part.body.data.as_deref().is_none_or(str::is_empty) && !part.parts.is_empty() {
                    return Err(GmailError::InvalidResponse);
                }
                part.body.clone()
            }
        };
        if body.size != expected_size {
            return Err(GmailError::InvalidResponse);
        }
        let bytes = match body.data.as_deref() {
            Some(data) => content::decode(data)?,
            None if body.size == 0 => Vec::new(),
            None => return Err(GmailError::InvalidResponse),
        };
        if u64::try_from(bytes.len()).map_err(|_| GmailError::TooLarge)? != expected_size {
            return Err(GmailError::InvalidResponse);
        }
        Ok(GmailAttachment { metadata, bytes })
    }
}

fn part<'a>(root: &'a wire::Part, part_id: &str) -> Result<&'a wire::Part, GmailError> {
    let mut pending = vec![(root, 0)];
    let mut found = None;
    while let Some((part, depth)) = pending.pop() {
        // Same MIME-depth budget used by full-thread projection.
        if depth > 64 {
            return Err(GmailError::TooLarge);
        }
        if part.part_id == part_id {
            if found.is_some() {
                return Err(GmailError::InvalidResponse);
            }
            found = Some(part);
        }
        pending.extend(part.parts.iter().map(|child| (child, depth + 1)));
    }
    found.ok_or(GmailError::InvalidResponse)
}
