//! Typed local drafts and current-session review. No operation in this module
//! performs a provider send; dispatch must claim the durable journal separately.

use super::{record::DraftPayload, ComposeError, MailMimeInput, VerifiedMailSender};
use crate::mail::{cache::MailCache, ActiveCall, MailError, MailRuntime, MailSession};
use bruhs_models::mail::*;
use std::sync::Arc;

impl MailRuntime {
    pub(super) fn compose_host(&self, expected: &str) -> Result<(), ComposeError> {
        if expected == self.host().as_str() {
            Ok(())
        } else {
            Err(MailError::Cache(crate::mail::CacheError::HostChanged).into())
        }
    }

    // Like call(), the blocking job owns the cache guard and process lock even
    // if its HTTP caller disappears. Errors never publish a successful change.
    pub(super) async fn compose_call<T: Send + 'static>(
        &self,
        changed: bool,
        operation: impl FnOnce(&mut MailCache) -> Result<T, ComposeError> + Send + 'static,
    ) -> Result<T, ComposeError> {
        let mut work = ActiveCall {
            cache: self.cache.clone().lock_owned().await,
            _owner: Arc::clone(&self.owner),
        };
        let changes = self.changes.clone();
        tokio::task::spawn_blocking(move || {
            let result = operation(&mut work.cache);
            if changed && result.is_ok() {
                changes.send_modify(|revision| *revision = revision.saturating_add(1));
            }
            drop(work);
            result
        })
        .await
        .map_err(|_| ComposeError::Mail(MailError::Worker))?
    }

    pub async fn compose_create(
        &self,
        mailbox_id: String,
        request: MailDraftCreate,
    ) -> Result<MailComposeDraft, ComposeError> {
        self.compose_host(&request.host_id)?;
        self.compose_call(true, move |cache| {
            cache.compose_create(&mailbox_id, &request)
        })
        .await
    }

    pub async fn compose_draft(
        &self,
        expected_host: String,
        mailbox_id: String,
        client_id: String,
    ) -> Result<Option<MailComposeDraft>, ComposeError> {
        self.compose_host(&expected_host)?;
        self.compose_call(false, move |cache| {
            cache.compose_draft(&mailbox_id, &client_id)
        })
        .await
    }

    pub async fn compose_save(
        &self,
        mailbox_id: String,
        client_id: String,
        request: MailDraftSave,
    ) -> Result<MailComposeDraft, ComposeError> {
        self.compose_host(&request.host_id)?;
        request.content.validate_storage()?;
        self.compose_call(true, move |cache| {
            cache.compose_save(&mailbox_id, &client_id, &request)
        })
        .await
    }

    /// Deletes a draft that no send reached; `Ok(false)` when the send
    /// journal holds an operation for it, which keeps it (ruling F16).
    pub(crate) async fn compose_discard_unsent(
        &self,
        expected_host: String,
        mailbox_id: String,
        client_id: String,
    ) -> Result<bool, ComposeError> {
        self.compose_host(&expected_host)?;
        self.compose_call(true, move |cache| {
            cache.compose_discard_unsent(&mailbox_id, &client_id)
        })
        .await
    }

    pub async fn compose_drafts(
        &self,
        expected_host: String,
        mailbox_id: String,
        page: u64,
        expected_revision: Option<i64>,
    ) -> Result<MailDraftsPage, ComposeError> {
        self.compose_host(&expected_host)?;
        self.compose_call(false, move |cache| {
            cache.compose_drafts(&mailbox_id, page, expected_revision)
        })
        .await
    }

    pub async fn compose_upload(
        &self,
        expected_host: String,
        mailbox_id: String,
        client_id: String,
        upload: crate::mail::DraftAttachmentUpload,
    ) -> Result<MailDraftFile, ComposeError> {
        self.compose_host(&expected_host)?;
        self.compose_call(true, move |cache| {
            let draft = cache.compose_snapshot(&mailbox_id, &client_id)?;
            let file = cache.store_draft_attachment(&draft.reference, upload)?;
            Ok(MailDraftFile {
                id: file.id,
                file_name: file.file_name,
                size_bytes: file.size_bytes,
                sha256: file.sha256,
            })
        })
        .await
    }

    pub async fn compose_remove_upload(
        &self,
        expected_host: String,
        mailbox_id: String,
        client_id: String,
        attachment_id: String,
    ) -> Result<(), ComposeError> {
        self.compose_host(&expected_host)?;
        self.compose_call(true, move |cache| {
            let draft = cache.compose_snapshot(&mailbox_id, &client_id)?;
            cache.remove_draft_attachment(&draft.reference, &attachment_id)?;
            Ok(())
        })
        .await
    }
}

impl MailSession {
    pub async fn review_compose(
        &self,
        sender: &VerifiedMailSender,
        client_id: String,
        request: MailReviewRequest,
    ) -> Result<MailSendReview, ComposeError> {
        self.review_compose_with_limit(sender, client_id, request, MAIL_PREPARED_MIME_MAX_BYTES)
            .await
    }

    pub async fn review_compose_with_limit(
        &self,
        sender: &VerifiedMailSender,
        client_id: String,
        request: MailReviewRequest,
        mime_limit_bytes: usize,
    ) -> Result<MailSendReview, ComposeError> {
        sender.check_session(self)?;
        if mime_limit_bytes == 0 || mime_limit_bytes > MAIL_PREPARED_MIME_MAX_BYTES {
            return Err(ComposeError::InvalidDraft);
        }
        self.runtime.compose_host(&request.host_id)?;
        if request.connection_id != self.authorization().connection_id() {
            return Err(ComposeError::StaleReview);
        }
        let mailbox_id = self.authorization().mailbox().as_str().to_owned();
        let mut snapshot = self
            .runtime
            .compose_call(false, move |cache| {
                let snapshot = cache.compose_snapshot(&mailbox_id, &client_id)?;
                if cache.compose_has_pending(&snapshot.reference)? {
                    return Err(ComposeError::SendPending);
                }
                Ok(snapshot)
            })
            .await?;
        let can_reuse = snapshot.payload.prepared.as_ref().is_some_and(|prepared| {
            prepared.connection_id == request.connection_id
                && (snapshot.revision == request.expected_revision
                    || prepared.source_revision == request.expected_revision)
        });
        if !can_reuse {
            if snapshot.revision != request.expected_revision {
                return Err(ComposeError::StaleReview);
            }
            let prepared = self
                .prepare_mime(
                    sender,
                    MailMimeInput {
                        reference: snapshot.reference.clone(),
                        source_revision: snapshot.revision,
                        content: snapshot.payload.content.clone(),
                        reply: snapshot.payload.reply.clone(),
                    },
                )
                .await?;
            if prepared.bytes().len() > mime_limit_bytes {
                return Err(
                    super::MailSendProblem::new(super::MailSendProblemKind::TooLarge).into(),
                );
            }
            let authorization = self.authorization().clone();
            snapshot = self
                .runtime
                .compose_call(true, move |cache| {
                    cache.compose_freeze(&authorization, snapshot, prepared)
                })
                .await?;
        }
        let authorization = self.authorization().clone();
        let review = self
            .runtime
            .compose_call(false, move |cache| {
                let reference = snapshot
                    .payload
                    .prepared
                    .as_ref()
                    .ok_or(ComposeError::StaleReview)?;
                let prepared = cache.compose_prepared(&snapshot.reference, reference)?;
                prepared.check_content(&snapshot.payload.content)?;
                if prepared.bytes().len() > mime_limit_bytes {
                    return Err(
                        super::MailSendProblem::new(super::MailSendProblemKind::TooLarge).into(),
                    );
                }
                cache.compose_check_reply(&snapshot, &prepared)?;
                if cache.compose_has_pending(&snapshot.reference)? {
                    return Err(ComposeError::SendPending);
                }
                let journal = cache
                    .review_send(&authorization, &snapshot.reference, snapshot.revision)
                    .map_err(MailError::from)?;
                Ok(MailSendReview {
                    host_id: cache.host().as_str().into(),
                    mailbox_id: snapshot.reference.mailbox.as_str().into(),
                    client_id: snapshot.reference.client_id,
                    draft_id: snapshot.reference.canonical_id,
                    revision: snapshot.revision,
                    connection_id: authorization.connection_id().into(),
                    digest: journal.digest().into(),
                    from: prepared.from().clone(),
                    recipients: prepared.recipients().clone(),
                    content: snapshot.payload.content,
                    files: prepared.files().to_vec(),
                    message_id: prepared.message_id().into(),
                    mime_size_bytes: u64::try_from(prepared.bytes().len())
                        .map_err(|_| ComposeError::PreparedTooLarge)?,
                    mime_sha256: prepared.sha256().into(),
                })
            })
            .await?;
        if review.from != *sender.address() {
            return Err(ComposeError::StaleReview);
        }
        Ok(review)
    }
}

pub(crate) struct ComposeSnapshot {
    pub reference: crate::mail::MailDraftRef,
    pub revision: i64,
    pub payload: DraftPayload,
}

#[cfg(test)]
mod tests;
