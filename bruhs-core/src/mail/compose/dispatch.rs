//! Runtime-owned, one-attempt sending from a frozen and explicitly reviewed
//! revision. Dropping the wait handle never cancels the worker or replays it.

use super::*;
use crate::mail::{DispatchClaim, MailRuntime, SendOperation, SendState};
use bruhs_models::mail::*;
use futures_util::FutureExt as _;
use std::{panic::AssertUnwindSafe, sync::Arc};
use tokio::sync::oneshot;

pub struct MailSendTask {
    receiver: oneshot::Receiver<Result<MailSendStatus, ComposeError>>,
}

impl MailSendTask {
    pub async fn wait(self) -> Result<MailSendStatus, ComposeError> {
        self.receiver
            .await
            .map_err(|_| MailSendProblem::new(MailSendProblemKind::Unconfirmed))?
    }
}

pub(crate) fn status(
    host: &str,
    mailbox: &str,
    operation: &SendOperation,
) -> Result<MailSendStatus, ComposeError> {
    let receipt: Option<serde_json::Value> = operation
        .receipt
        .as_ref()
        .map(|value| serde_json::from_str(value))
        .transpose()
        .map_err(|_| ComposeError::PreparedCorrupt)?;
    let state = match operation.state {
        SendState::Queued => MailDispatchState::Queued,
        SendState::Dispatching => MailDispatchState::Dispatching,
        SendState::Accepted => MailDispatchState::Accepted,
        SendState::Uncertain => MailDispatchState::Uncertain,
        SendState::NeedsReview => MailDispatchState::NeedsReview,
        SendState::Cancelled => MailDispatchState::Cancelled,
        SendState::Rejected => MailDispatchState::Rejected,
    };
    Ok(MailSendStatus {
        host_id: host.into(),
        mailbox_id: mailbox.into(),
        operation_id: operation.id.clone(),
        draft_id: operation.draft_id.clone(),
        draft_revision: operation.draft_revision,
        state,
        provider_message_id: receipt
            .as_ref()
            .and_then(|value| value.get("message_id"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        problem: receipt
            .as_ref()
            .and_then(|value| value.get("problem"))
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|_| ComposeError::PreparedCorrupt)?,
        provider_thread_id: receipt
            .as_ref()
            .and_then(|value| value.get("thread_id"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
    })
}

fn check_request(request: &MailSendRequest) -> Result<(), ComposeError> {
    if request.operation_id.is_empty()
        || request.operation_id.len() > 128
        || !request
            .operation_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        || request.revision <= 0
        || request.digest.len() != 64
        || !request
            .digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ComposeError::StaleReview);
    }
    Ok(())
}

impl MailSession {
    pub async fn queue_reviewed_compose(
        &self,
        sender: &VerifiedMailSender,
        client_id: String,
        request: MailSendRequest,
        mime_limit_bytes: usize,
    ) -> Result<MailSendStatus, ComposeError> {
        self.runtime.compose_host(&request.host_id)?;
        sender.check_session(self)?;
        check_request(&request)?;
        let auth = self.authorization().clone();
        let sender_address = sender.address().clone();
        if request.connection_id != auth.connection_id() {
            return Err(ComposeError::StaleReview);
        }
        self.runtime
            .compose_call(true, move |cache| {
                let expected = auth
                    .mailbox()
                    .draft(&client_id, None)
                    .map_err(crate::mail::CacheError::from)?;
                if let Some(existing) = cache
                    .send_operation(&request.operation_id, cache.host(), auth.mailbox())
                    .map_err(MailError::from)?
                {
                    if existing.draft_id != expected.canonical_id
                        || existing.draft_revision != request.revision
                        || existing.payload_digest != request.digest
                        || existing.connection_id() != auth.connection_id()
                    {
                        return Err(ComposeError::StaleReview);
                    }
                    return status(cache.host().as_str(), auth.mailbox().as_str(), &existing);
                }
                let snapshot = cache.compose_snapshot(auth.mailbox().as_str(), &client_id)?;
                if snapshot.revision != request.revision {
                    return Err(ComposeError::StaleReview);
                }
                if cache.compose_has_pending(&snapshot.reference)? {
                    return Err(ComposeError::SendPending);
                }
                let reference = snapshot
                    .payload
                    .prepared
                    .as_ref()
                    .filter(|reference| reference.connection_id == auth.connection_id())
                    .ok_or(ComposeError::StaleReview)?;
                let prepared = cache.compose_prepared(&snapshot.reference, reference)?;
                prepared.check_content(&snapshot.payload.content)?;
                cache.compose_check_reply(&snapshot, &prepared)?;
                if prepared.from() != &sender_address {
                    return Err(ComposeError::StaleReview);
                }
                if mime_limit_bytes == 0
                    || prepared.bytes().len() > mime_limit_bytes
                    || mime_limit_bytes > MAIL_PREPARED_MIME_MAX_BYTES
                {
                    return Err(MailSendProblem::new(MailSendProblemKind::TooLarge).into());
                }
                let review = cache
                    .review_send(&auth, &snapshot.reference, request.revision)
                    .map_err(MailError::from)?;
                if review.digest() != request.digest {
                    return Err(ComposeError::StaleReview);
                }
                let operation = cache
                    .queue_send(&request.operation_id, &review, &auth, EpochMs::now())
                    .map_err(MailError::from)?;
                status(cache.host().as_str(), auth.mailbox().as_str(), &operation)
            })
            .await
    }

    pub async fn claim_reviewed_compose(
        &self,
        sender: &VerifiedMailSender,
        client_id: String,
        request: MailSendRequest,
        mime_limit_bytes: usize,
    ) -> Result<Option<MailDispatch>, ComposeError> {
        self.runtime.compose_host(&request.host_id)?;
        sender.check_session(self)?;
        check_request(&request)?;
        let auth = self.authorization().clone();
        let sender_address = sender.address().clone();
        if request.connection_id != auth.connection_id() {
            return Err(ComposeError::StaleReview);
        }
        self.runtime
            .compose_call(true, move |cache| {
                let expected = auth
                    .mailbox()
                    .draft(&client_id, None)
                    .map_err(crate::mail::CacheError::from)?;
                let existing = cache
                    .send_operation(&request.operation_id, cache.host(), auth.mailbox())
                    .map_err(MailError::from)?
                    .ok_or(ComposeError::StaleReview)?;
                if existing.draft_id != expected.canonical_id
                    || existing.draft_revision != request.revision
                    || existing.payload_digest != request.digest
                    || existing.connection_id() != auth.connection_id()
                {
                    return Err(ComposeError::StaleReview);
                }
                let DispatchClaim::Start(operation) = cache
                    .claim_send(&request.operation_id, &auth, EpochMs::now())
                    .map_err(MailError::from)?
                else {
                    return Ok(None);
                };
                let prepared = (|| {
                    let snapshot = cache.compose_snapshot(auth.mailbox().as_str(), &client_id)?;
                    let reference = snapshot
                        .payload
                        .prepared
                        .as_ref()
                        .filter(|reference| reference.connection_id == auth.connection_id())
                        .ok_or(ComposeError::StaleReview)?;
                    let prepared = cache.compose_prepared(&snapshot.reference, reference)?;
                    prepared.check_content(&snapshot.payload.content)?;
                    cache.compose_check_reply(&snapshot, &prepared)?;
                    if prepared.from() != &sender_address {
                        return Err(ComposeError::StaleReview);
                    }
                    if mime_limit_bytes == 0
                        || prepared.bytes().len() > mime_limit_bytes
                        || mime_limit_bytes > MAIL_PREPARED_MIME_MAX_BYTES
                    {
                        return Err(MailSendProblem::new(MailSendProblemKind::TooLarge).into());
                    }
                    Ok(prepared)
                })();
                match prepared {
                    Ok(prepared) => {
                        MailDispatch::new(cache.host().clone(), auth, *operation, prepared)
                            .map(Some)
                    }
                    Err(error) => {
                        cache
                            .record_definite_send_rejection(
                                &request.operation_id,
                                &auth,
                                EpochMs::now(),
                            )
                            .map_err(MailError::from)?;
                        Err(error)
                    }
                }
            })
            .await
    }
}

impl MailRuntime {
    pub async fn cancel_reviewed_compose(
        &self,
        expected_host: String,
        mailbox_id: String,
        operation_id: String,
    ) -> Result<MailSendStatus, ComposeError> {
        self.compose_host(&expected_host)?;
        self.compose_call(true, move |cache| {
            cache.compose_cancel_send(&mailbox_id, &operation_id)
        })
        .await
    }
    pub async fn compose_send_status(
        &self,
        expected_host: String,
        mailbox_id: String,
        operation_id: String,
    ) -> Result<Option<MailSendStatus>, ComposeError> {
        self.compose_host(&expected_host)?;
        self.compose_call(false, move |cache| {
            cache.compose_send_status(&mailbox_id, &operation_id)
        })
        .await
    }

    /// Starts synchronously: neither HTTP cancellation nor dropping MailSendTask
    /// aborts the owned session, claim, provider attempt or receipt persistence.
    pub fn send_reviewed_compose(
        &self,
        session: MailSession,
        transport: Arc<dyn MailSendTransport>,
        client_id: String,
        request: MailSendRequest,
    ) -> MailSendTask {
        let (sender, receiver) = oneshot::channel();
        let expected_host = self.host().clone();
        let worker = tokio::spawn(async move {
            let result = async {
                if session.runtime.host() != &expected_host
                    || transport.provider() != session.authorization().mailbox().provider()
                {
                    return Err(MailSendProblem::new(MailSendProblemKind::AccountChanged).into());
                }
                let verified = transport.verified_sender(&session).await?;
                let limit = transport.mime_limit_bytes();
                let status = session
                    .queue_reviewed_compose(&verified, client_id.clone(), request.clone(), limit)
                    .await?;
                if status.state != MailDispatchState::Queued {
                    return Ok(status);
                }
                let Some(dispatch) = session
                    .claim_reviewed_compose(&verified, client_id, request.clone(), limit)
                    .await?
                else {
                    return session
                        .runtime
                        .compose_send_status(
                            request.host_id,
                            session.authorization().mailbox().as_str().into(),
                            request.operation_id,
                        )
                        .await?
                        .ok_or(ComposeError::StaleReview);
                };
                let outcome = AssertUnwindSafe(transport.send(&session, dispatch))
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|_| {
                        ProviderSendOutcome::Uncertain(MailSendProblem::new(
                            MailSendProblemKind::Unconfirmed,
                        ))
                    });
                let uncertain = matches!(&outcome, ProviderSendOutcome::Uncertain(_));
                match outcome {
                    ProviderSendOutcome::Accepted(receipt) => {
                        if session
                            .record_acceptance(request.operation_id.clone(), receipt)
                            .await
                            .is_err()
                        {
                            let auth = session.authorization().clone();
                            let id = request.operation_id.clone();
                            session
                                .runtime
                                .compose_call(true, move |cache| {
                                    cache
                                        .record_send_problem(
                                            &id,
                                            &auth,
                                            true,
                                            &MailSendProblem::new(MailSendProblemKind::Unconfirmed),
                                            EpochMs::now(),
                                        )
                                        .map_err(MailError::from)?;
                                    Ok(())
                                })
                                .await?;
                            return Err(
                                MailSendProblem::new(MailSendProblemKind::Unconfirmed).into()
                            );
                        }
                    }
                    ProviderSendOutcome::Rejected(problem)
                    | ProviderSendOutcome::Uncertain(problem) => {
                        let auth = session.authorization().clone();
                        let id = request.operation_id.clone();
                        let saved_problem = problem.clone();
                        session
                            .runtime
                            .compose_call(true, move |cache| {
                                cache
                                    .record_send_problem(
                                        &id,
                                        &auth,
                                        uncertain,
                                        &saved_problem,
                                        EpochMs::now(),
                                    )
                                    .map_err(MailError::from)?;
                                Ok(())
                            })
                            .await?;
                        return Err(problem.into());
                    }
                }
                session
                    .runtime
                    .compose_send_status(
                        request.host_id,
                        session.authorization().mailbox().as_str().into(),
                        request.operation_id,
                    )
                    .await?
                    .ok_or(ComposeError::StaleReview)
            }
            .await;
            let _ = sender.send(result);
        });
        // Tokio tasks keep running when their JoinHandle is dropped. The public
        // wait handle intentionally exposes no abort operation.
        drop(worker);
        MailSendTask { receiver }
    }
}

#[cfg(test)]
mod tests;
