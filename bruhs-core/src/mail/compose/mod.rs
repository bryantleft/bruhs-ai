//! Common compose preflight. The runtime's reviewed-send journal remains the
//! authority for provider dispatch; building MIME alone never sends a message.

pub(crate) mod dispatch;
pub(crate) mod drafts;
mod mime;
pub mod reconciliation;
pub(crate) mod record;
mod transport;
#[cfg(any(test, feature = "test-support"))]
pub use dispatch::test_support;
pub use dispatch::MailSendTask;
pub use mime::{PreparedMime, VerifiedMailSender};
pub use transport::{
    MailDispatch, MailSendProblem, MailSendProblemKind, MailSendTransport, ProviderSendOutcome,
};

use super::{MailDraftRef, MailError, MailSession};
use bruhs_models::{
    mail::{MailComposeContent, MailComposeError, MailReplyTarget},
    EpochMs,
};

/// Only preparation: the reviewed-send journal separately authorizes dispatch.
pub struct MailMimeInput {
    pub reference: MailDraftRef,
    pub source_revision: i64,
    pub content: MailComposeContent,
    pub reply: Option<MailReplyTarget>,
}

impl MailSession {
    /// Load selected files and reply source from this session's canonical cache.
    /// A caller cannot provide arbitrary file bytes or a foreign source record.
    pub async fn prepare_mime(
        &self,
        sender: &VerifiedMailSender,
        input: MailMimeInput,
    ) -> Result<PreparedMime, ComposeError> {
        input.content.validate_send()?;
        if input.reference.mailbox != *self.authorization().mailbox() {
            return Err(ComposeError::InvalidDraft);
        }
        let host = self.runtime.host().as_str().to_owned();
        let reply = if let Some(target) = &input.reply {
            if input
                .reference
                .reply_to
                .as_ref()
                .map(|reference| reference.external_id.as_str())
                != Some(target.external_thread_id.as_str())
            {
                return Err(ComposeError::InvalidReply);
            }
            Some(
                self.runtime
                    .connected_thread(
                        host.clone(),
                        input.reference.mailbox.as_str().into(),
                        target.external_thread_id.clone(),
                    )
                    .await?
                    .ok_or(ComposeError::InvalidReply)?,
            )
        } else {
            None
        };
        let mut files = Vec::with_capacity(input.content.attachments.len());
        let mut total_bytes = 0usize;
        for id in &input.content.attachments {
            let file = self
                .runtime
                .draft_attachment(host.clone(), input.reference.clone(), id.clone())
                .await?
                .ok_or(ComposeError::AttachmentChanged)?;
            total_bytes = total_bytes
                .checked_add(file.bytes.len())
                .filter(|total| *total <= bruhs_models::mail::MAIL_DRAFT_ATTACHMENT_MAX_BYTES)
                .ok_or(ComposeError::AttachmentChanged)?;
            files.push(file);
        }
        // Encoding runs off the async reactor, with bounded inputs. The frozen
        // result must still be persisted and reviewed before claiming a send.
        let authority = mime::MimeAuthority {
            host: self.runtime.host().clone(),
            authorization: self.authorization().clone(),
        };
        let sender = sender.clone();
        let date = EpochMs::now();
        tokio::task::spawn_blocking(move || {
            let request = mime::MimeDraft {
                reference: &input.reference,
                source_revision: input.source_revision,
                content: &input.content,
                reply: input.reply.as_ref().zip(reply.as_ref()),
                files: &files,
                date,
            };
            mime::prepare(&authority, &sender, request)
        })
        .await
        .map_err(|_| ComposeError::PreparationWorker)?
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ComposeError {
    #[error(transparent)]
    Mail(#[from] MailError),
    #[error(transparent)]
    Validation(#[from] MailComposeError),
    #[error(transparent)]
    Provider(#[from] MailSendProblem),
    #[error("This draft could not be prepared. Refresh its saved version and retry.")]
    InvalidDraft,
    #[error("The account's sender address could not be verified. Reconnect and review again.")]
    InvalidSender,
    #[error("This reply's saved source could not be verified. Refresh it or start a new email.")]
    InvalidReply,
    #[error("Keep this reply's subject unchanged, or start a new email.")]
    ReplySubjectChanged,
    #[error("An attached file changed or is unavailable. Choose it again before reviewing.")]
    AttachmentChanged,
    #[error("The prepared email exceeds the 32 MiB MIME limit. Remove attachments or shorten it.")]
    PreparedTooLarge,
    #[error(
        "The prepared email could not be read. Edit the draft and review it again before sending."
    )]
    PreparedCorrupt,
    #[error("The draft or account changed after review. Review its current contents.")]
    StaleReview,
    #[error(
        "This draft has a pending or uncertain send. Open its send status before reviewing again."
    )]
    SendPending,
    #[error("The email preparation worker stopped. Review the draft again before sending.")]
    PreparationWorker,
}
