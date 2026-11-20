//! `send_mail` (D17, ruling F16): one email through the compose path the
//! owner's own sends use. It creates a draft, saves the bruh's content over
//! it, verifies the sender in a current session, reviews it and hands the
//! reviewed revision to the send journal's one-attempt dispatch, all inside
//! one budget. Nothing is ever retried.
//!
//! Until the journal holds the operation nothing can have been sent, so a
//! failure or a spent budget deletes the draft and answers
//! [`MAIL_NOT_SENT`]. Once it holds it, the answer follows the journal's
//! state, and a spent budget answers [`MAIL_MAY_HAVE_SENT`] while the
//! dispatch finishes in its own task. Nothing here logs content or
//! recipients (D26).

use super::{AppsError, MailSendOutcome};
use crate::connections::Connections;
use crate::mail::compose::{ComposeError, MailSendProblem, MailSendTransport};
use crate::mail::{MailRuntime, MailSession};
use bruhs_models::connections::ServiceProvider;
use bruhs_models::crew::{
    MailSendInput, MAIL_MAY_HAVE_SENT, MAIL_NOT_SENT, MAIL_QUEUED, MAIL_REPLY_UNKNOWN, MAIL_SENT,
};
use bruhs_models::mail::{
    MailComposeContent, MailDispatchState, MailDraftCreate, MailDraftSave, MailReviewRequest,
    MailSendRequest, MailSendStatus,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// How long cleaning up after a spent budget may take: deleting the draft
/// or reading the journal, each a local cache call.
const SETTLE_BUDGET: Duration = Duration::from_secs(5);

/// The mailbox a send goes out from, as the host resolved it.
pub(super) struct Mailbox<'a> {
    pub id: &'a str,
    pub provider: ServiceProvider,
    pub connection_id: &'a str,
}

/// One `send_mail`: the draft and operation ids it owns, and whether the
/// dispatch has started, which decides what a failure can mean.
struct MailSend<'a> {
    mail: &'a MailRuntime,
    host: String,
    mailbox: Mailbox<'a>,
    client_id: String,
    operation_id: String,
    dispatched: AtomicBool,
}

/// Sends `input` from `mailbox` through `transport`, answering within
/// `budget` plus [`SETTLE_BUDGET`].
pub(super) async fn send(
    mail: &MailRuntime,
    connections: &Connections,
    mailbox: Mailbox<'_>,
    transport: Arc<dyn MailSendTransport>,
    input: MailSendInput,
    budget: Duration,
) -> Result<MailSendOutcome, AppsError> {
    let send = MailSend {
        mail,
        host: mail.host().as_str().to_owned(),
        mailbox,
        client_id: format!("bruh-{}", uuid::Uuid::new_v4().simple()),
        operation_id: uuid::Uuid::new_v4().to_string(),
        dispatched: AtomicBool::new(false),
    };
    match tokio::time::timeout(budget, send.run(connections, transport, input)).await {
        Ok(result) => result,
        Err(_) => match tokio::time::timeout(SETTLE_BUDGET, send.settle(None, true)).await {
            Ok(result) => result,
            // The cache didn't answer: say what is safe to assume.
            Err(_) if send.dispatched.load(Ordering::SeqCst) => {
                Err(AppsError::TimedOut(MAIL_MAY_HAVE_SENT))
            }
            Err(_) => Err(AppsError::TimedOut(MAIL_NOT_SENT)),
        },
    }
}

impl MailSend<'_> {
    async fn run(
        &self,
        connections: &Connections,
        transport: Arc<dyn MailSendTransport>,
        input: MailSendInput,
    ) -> Result<MailSendOutcome, AppsError> {
        let (session, request) = match self.prepare(connections, &*transport, input).await {
            Ok(prepared) => prepared,
            Err(error) => {
                // No dispatch started, so the journal can't hold this send.
                self.discard().await;
                return Err(not_sent(&error));
            }
        };
        self.dispatched.store(true, Ordering::SeqCst);
        let result = self
            .mail
            .send_reviewed_compose(session, transport, self.client_id.clone(), request)
            .wait()
            .await;
        match result {
            // The dispatch returned on its own: a queued or dispatching
            // send is still on its way.
            Ok(status) => Ok(self.outcome(&status, None, true)),
            Err(ComposeError::Provider(problem)) => self.settle(Some(problem), false).await,
            Err(_) => self.settle(None, false).await,
        }
    }

    /// Creates the draft, saves the bruh's content over it and reviews it
    /// in a current session with a verified sender: the owner's review
    /// route, step for step. For a reply the draft keeps the thread's
    /// subject, which the review requires (F16).
    async fn prepare(
        &self,
        connections: &Connections,
        transport: &dyn MailSendTransport,
        input: MailSendInput,
    ) -> Result<(MailSession, MailSendRequest), ComposeError> {
        let mailbox_id = self.mailbox.id.to_owned();
        let draft = self
            .mail
            .compose_create(
                mailbox_id.clone(),
                MailDraftCreate {
                    host_id: self.host.clone(),
                    client_id: self.client_id.clone(),
                    reply: input.reply.clone(),
                },
            )
            .await?;
        let subject = match input.reply {
            Some(_) => draft.content.subject.clone(),
            None => input.subject.unwrap_or_default(),
        };
        let draft = self
            .mail
            .compose_save(
                mailbox_id.clone(),
                self.client_id.clone(),
                MailDraftSave {
                    host_id: self.host.clone(),
                    expected_revision: draft.revision,
                    content: MailComposeContent {
                        to: input.to,
                        cc: input.cc,
                        bcc: input.bcc,
                        subject,
                        body: input.body,
                        attachments: Vec::new(),
                    },
                },
            )
            .await?;
        let session = self
            .mail
            .session(
                connections,
                self.mailbox.provider,
                self.mailbox.connection_id,
            )
            .await?;
        if session.authorization().mailbox().as_str() != mailbox_id {
            return Err(ComposeError::StaleReview);
        }
        let sender = transport.verified_sender(&session).await?;
        let review = session
            .review_compose_with_limit(
                &sender,
                self.client_id.clone(),
                MailReviewRequest {
                    host_id: self.host.clone(),
                    connection_id: self.mailbox.connection_id.to_owned(),
                    expected_revision: draft.revision,
                },
                transport.mime_limit_bytes(),
            )
            .await?;
        let request = MailSendRequest {
            host_id: review.host_id,
            connection_id: review.connection_id,
            operation_id: self.operation_id.clone(),
            revision: review.revision,
            digest: review.digest,
        };
        Ok((session, request))
    }

    /// After a failed or timed out dispatch: when the journal holds the
    /// operation, its state answers, and a send left queued or dispatching
    /// may have gone out; otherwise the draft is deleted in the same step,
    /// so a dispatch still on its way can't queue it, and nothing was sent.
    async fn settle(
        &self,
        problem: Option<MailSendProblem>,
        timed_out: bool,
    ) -> Result<MailSendOutcome, AppsError> {
        let unsent = |message| {
            Err(if timed_out {
                AppsError::TimedOut(message)
            } else {
                AppsError::Unavailable(message)
            })
        };
        let kept = self
            .mail
            .compose_discard_unsent(
                self.host.clone(),
                self.mailbox.id.to_owned(),
                self.client_id.clone(),
            )
            .await;
        match kept {
            Ok(true) => unsent(MAIL_NOT_SENT),
            Ok(false) => match self
                .mail
                .compose_send_status(
                    self.host.clone(),
                    self.mailbox.id.to_owned(),
                    self.operation_id.clone(),
                )
                .await
            {
                Ok(Some(status)) => Ok(self.outcome(&status, problem, false)),
                // Another send of this draft: this one wasn't queued.
                Ok(None) => unsent(MAIL_NOT_SENT),
                Err(_) => unsent(MAIL_MAY_HAVE_SENT),
            },
            Err(_) if !self.dispatched.load(Ordering::SeqCst) => unsent(MAIL_NOT_SENT),
            Err(_) => unsent(MAIL_MAY_HAVE_SENT),
        }
    }

    /// Deletes the draft of a send that never reached the journal. A
    /// failure leaves it in the owner's drafts, where they can delete it.
    async fn discard(&self) {
        if self
            .mail
            .compose_discard_unsent(
                self.host.clone(),
                self.mailbox.id.to_owned(),
                self.client_id.clone(),
            )
            .await
            .is_err()
        {
            tracing::warn!("Could not delete a bruh's unsent draft");
        }
    }

    /// What the bruh is told for a journaled send: sent, queued (only when
    /// the dispatch returned normally and the send is on its way), may
    /// have sent (left queued or dispatching by a failure or the budget,
    /// or unconfirmed), or the provider's problem when it was refused.
    fn outcome(
        &self,
        status: &MailSendStatus,
        problem: Option<MailSendProblem>,
        in_flight: bool,
    ) -> MailSendOutcome {
        let message = match status.state {
            MailDispatchState::Accepted => MAIL_SENT.to_owned(),
            MailDispatchState::Queued | MailDispatchState::Dispatching if in_flight => {
                MAIL_QUEUED.to_owned()
            }
            MailDispatchState::Queued
            | MailDispatchState::Dispatching
            | MailDispatchState::Uncertain => MAIL_MAY_HAVE_SENT.to_owned(),
            MailDispatchState::NeedsReview
            | MailDispatchState::Cancelled
            | MailDispatchState::Rejected => status
                .problem
                .clone()
                .or(problem)
                .map_or_else(|| MAIL_NOT_SENT.to_owned(), |problem| problem.to_string()),
        };
        MailSendOutcome {
            state: status.state,
            operation_id: Some(self.operation_id.clone()),
            message,
        }
    }
}

/// A failure before the dispatch started: nothing was sent (F16). A reply
/// to a message the store doesn't hold, or a provider problem such as an
/// expired sign in, says so, since trying again can't help.
fn not_sent(error: &ComposeError) -> AppsError {
    match error {
        ComposeError::InvalidReply => AppsError::Invalid(MAIL_REPLY_UNKNOWN),
        ComposeError::Provider(problem) => AppsError::Unavailable(problem.message()),
        ComposeError::Mail(_) | ComposeError::PreparationWorker | ComposeError::PreparedCorrupt => {
            AppsError::Unavailable(MAIL_NOT_SENT)
        }
        _ => AppsError::Invalid(MAIL_NOT_SENT),
    }
}

#[cfg(test)]
#[path = "mail_tests.rs"]
mod tests;
