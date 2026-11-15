use super::{projection, wire, MailSession, OutlookClient, OutlookError};
use bruhs_models::{
    connections::ServiceProvider,
    mail::{MailAttachment, MAIL_ATTACHMENT_MAX_BYTES},
};

/// Downloaded bytes are inert data; the UI separately chooses a destination.
pub struct OutlookAttachment {
    pub metadata: MailAttachment,
    pub bytes: Vec<u8>,
    /// Item exports are MIME and may differ from the stored item's byte count.
    pub item_export_size: Option<u64>,
}

impl OutlookClient {
    /// A canonical cached reference is resolved only inside the leased mailbox.
    /// Reference attachments never authorize fetching their external location.
    pub async fn attachment(
        &self,
        session: &MailSession,
        external_thread: &str,
        canonical_attachment: &str,
    ) -> Result<OutlookAttachment, OutlookError> {
        let mailbox = session.authorization().mailbox();
        if mailbox.provider() != ServiceProvider::Outlook {
            return Err(OutlookError::WrongProvider);
        }
        let cached = session
            .runtime
            .connected_thread(
                session.runtime.host.as_str().to_owned(),
                mailbox.as_str().to_owned(),
                external_thread.to_owned(),
            )
            .await?
            .ok_or(OutlookError::NotFound)?;
        let (message, metadata) = cached
            .connected_mail
            .messages
            .iter()
            .find_map(|message| {
                message
                    .attachments
                    .iter()
                    .find(|item| item.id == canonical_attachment)
                    .map(|item| (message, item))
            })
            .ok_or(OutlookError::NotFound)?;
        if metadata.size_bytes > MAIL_ATTACHMENT_MAX_BYTES {
            return Err(OutlookError::TooLarge);
        }
        let external = metadata
            .external_id
            .as_deref()
            .ok_or(OutlookError::UnsupportedAttachment)?;
        let url = self.queried(
            &["messages", &message.external_id, "attachments", external],
            &[("$select", super::ATTACHMENT_FIELDS)],
        )?;
        let current: wire::Attachment = self.get(session, url).await?;
        let file = current.kind == "#microsoft.graph.fileAttachment";
        let current = projection::attachment(mailbox, &message.external_id, current)?;
        if current.external_id.is_none() {
            return Err(OutlookError::UnsupportedAttachment);
        }
        if current.id != metadata.id
            || current.size_bytes != metadata.size_bytes
            || current.external_id != metadata.external_id
        {
            return Err(OutlookError::InvalidResponse);
        }
        let url = self.url(&[
            "messages",
            &message.external_id,
            "attachments",
            external,
            "$value",
        ])?;
        let bytes = self.bytes(session, url).await?;
        let size = u64::try_from(bytes.len()).map_err(|_| OutlookError::TooLarge)?;
        if size > MAIL_ATTACHMENT_MAX_BYTES || (file && size != metadata.size_bytes) {
            return Err(OutlookError::InvalidResponse);
        }
        let mut metadata = metadata.clone();
        // Item attachments are exported MIME; the export's actual byte count can
        // differ from Graph's size of the stored Outlook item.
        metadata.size_bytes = size;
        Ok(OutlookAttachment {
            metadata,
            bytes,
            item_export_size: (!file).then_some(size),
        })
    }
}
