use super::{authority::*, MobileError, MobileResult, BROKER_CAPACITY, REQUEST_TIMEOUT};
use axum::body::Bytes;
use bruhs_models::api::QuestionUpload;
use bruhs_models::mobile::*;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, OwnedSemaphorePermit, Semaphore};
#[cfg(test)]
mod tests;

/// Holds admission until native work finishes, even if its HTTP caller goes away.
pub struct MobileResponder<T> {
    sender: oneshot::Sender<MobileResult<T>>,
    _permit: OwnedSemaphorePermit,
}
impl<T> MobileResponder<T> {
    pub fn is_closed(&self) -> bool {
        self.sender.is_closed()
    }
    pub fn send(self, result: MobileResult<T>) -> Result<(), MobileResult<T>> {
        self.sender.send(result)
    }
}

pub enum MobilePending {
    Read {
        lease: MobileLease,
        read: MobileRead,
        respond: MobileResponder<MobileReadReply>,
    },
    Command {
        lease: MobileLease,
        ticket: MobileCommandTicket,
        respond: MobileResponder<MobileCommandReceipt>,
    },
    QuestionFile {
        lease: MobileLease,
        file: MobileQuestionFile,
        respond: MobileResponder<()>,
    },
}

pub struct MobileQuestionFile {
    pub upload: QuestionUpload,
    pub turn_generation: u64,
    pub review_hash: String,
    /// No bytes means removal. Keep admission until the native handler
    /// releases the body, even if the HTTP request times out.
    pub bytes: Option<Bytes>,
    pub(super) _intake: OwnedSemaphorePermit,
}
impl MobilePending {
    pub fn is_closed(&self) -> bool {
        match self {
            Self::Read { respond, .. } => respond.is_closed(),
            Self::Command { respond, .. } => respond.is_closed(),
            Self::QuestionFile { respond, .. } => respond.is_closed(),
        }
    }
}
#[derive(Clone)]
pub struct MobileBroker(mpsc::Sender<MobilePending>, Arc<Semaphore>);
impl MobileBroker {
    pub(super) fn new() -> (Self, mpsc::Receiver<MobilePending>) {
        let (sender, receiver) = mpsc::channel(BROKER_CAPACITY);
        (
            Self(sender, Arc::new(Semaphore::new(BROKER_CAPACITY))),
            receiver,
        )
    }
    pub async fn read(
        &self,
        lease: MobileLease,
        read: MobileRead,
    ) -> MobileResult<MobileReadReply> {
        let scope = read.scope();
        lease.check(scope)?;
        let (respond, receive) = {
            let permit = self
                .1
                .clone()
                .try_acquire_owned()
                .map_err(|_| MobileError::Unavailable)?;
            let (sender, receive) = oneshot::channel();
            (
                MobileResponder {
                    sender,
                    _permit: permit,
                },
                receive,
            )
        };
        let guard = lease.clone();
        let reply = tokio::time::timeout(REQUEST_TIMEOUT, async {
            self.0
                .try_send(MobilePending::Read {
                    lease,
                    read,
                    respond,
                })
                .map_err(|_| MobileError::Unavailable)?;
            receive.await.map_err(|_| MobileError::Unavailable)?
        })
        .await
        .map_err(|_| MobileError::Unavailable)??;
        guard.check(scope)?;
        let bytes = serde_json::to_vec(&reply).map_err(|_| MobileError::Unavailable)?;
        if bytes.len() > RESPONSE_BYTES {
            return Err(MobileError::Invalid(
                "This desktop snapshot is too large. Open it on the laptop.".into(),
            ));
        }
        Ok(reply)
    }
    pub async fn command(
        &self,
        lease: MobileLease,
        command: MobileCommand,
    ) -> MobileResult<MobileCommandReceipt> {
        let authority = lease.authority();
        let (respond, receive) = {
            let permit = self
                .1
                .clone()
                .try_acquire_owned()
                .map_err(|_| MobileError::Unavailable)?;
            let (sender, receive) = oneshot::channel();
            (
                MobileResponder {
                    sender,
                    _permit: permit,
                },
                receive,
            )
        };
        let ticket = match authority.reserve(&lease, &command).await? {
            MobileReservation::Existing(receipt) => {
                // A previous HTTP task may have stopped after native acceptance but before save.
                authority.persist().await?;
                lease.check(command.action.scope())?;
                return Ok(receipt);
            }
            MobileReservation::New(ticket) => ticket,
        };
        let queued_ticket = ticket.clone();
        let queued_lease = lease.clone();
        tokio::time::timeout(REQUEST_TIMEOUT, async {
            self.0
                .try_send(MobilePending::Command {
                    lease: queued_lease,
                    ticket: queued_ticket,
                    respond,
                })
                .map_err(|_| MobileError::Unavailable)?;
            receive.await.map_err(|_| MobileError::Unavailable)?
        })
        .await
        .map_err(|_| MobileError::Unavailable)??;
        authority.finish(&lease, &ticket).await
    }

    pub(super) async fn question_file(
        &self,
        lease: MobileLease,
        file: MobileQuestionFile,
    ) -> MobileResult<()> {
        lease.check(MobileScope::Prompt)?;
        let permit = self
            .1
            .clone()
            .try_acquire_owned()
            .map_err(|_| MobileError::Unavailable)?;
        let (sender, receive) = oneshot::channel();
        self.0
            .try_send(MobilePending::QuestionFile {
                lease: lease.clone(),
                file,
                respond: MobileResponder {
                    sender,
                    _permit: permit,
                },
            })
            .map_err(|_| MobileError::Unavailable)?;
        tokio::time::timeout(REQUEST_TIMEOUT, receive)
            .await
            .map_err(|_| MobileError::Unavailable)?
            .map_err(|_| MobileError::Unavailable)??;
        lease.check(MobileScope::Prompt).map(|_| ())
    }
}
