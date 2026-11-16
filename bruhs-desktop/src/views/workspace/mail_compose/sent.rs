//! Explicit lookup only; normal restore/status work never calls this module.
use super::*;
use bruhs_models::connections::ServiceConnection;

const CONTEXT_CHANGED: &str = "The email context changed. Open the owning draft and check saved send status. Nothing was resent.";
const WRONG_ACCOUNT: &str =
    "Open the mail server and account that own this send before checking Sent. Nothing was resent.";
const RECONNECT: &str =
    "Reconnect the email account that owns this send, then check Sent again. Nothing was resent.";

#[derive(Clone, PartialEq, Eq)]
struct Target {
    origin: String,
    host: String,
    mailbox: String,
    client: String,
    provider: ServiceProvider,
    operation: String,
    draft: String,
    revision: i64,
    sequence: u64,
}

impl Target {
    fn capture(value: &DeviceDraft, sequence: u64) -> Option<Self> {
        let status = value.latest_send.as_ref()?;
        if status.state != MailDispatchState::Uncertain
            || status.host_id != value.scope.host_id
            || status.mailbox_id != value.scope.mailbox_id
            || status.operation_id.is_empty()
            || status.draft_id.is_empty()
            || status.draft_revision <= 0
            || !matches!(
                value.provider,
                ServiceProvider::Gmail | ServiceProvider::Outlook
            )
            || value.pending_send.as_ref().is_some_and(|pending| {
                pending.host_id != status.host_id
                    || pending.operation_id != status.operation_id
                    || pending.revision != status.draft_revision
            })
        {
            return None;
        }
        Some(Self {
            origin: value.scope.origin.clone(),
            host: value.scope.host_id.clone(),
            mailbox: value.scope.mailbox_id.clone(),
            client: value.client_id.clone(),
            provider: value.provider,
            operation: status.operation_id.clone(),
            draft: status.draft_id.clone(),
            revision: status.draft_revision,
            sequence,
        })
    }

    fn start(job: &Job) -> Option<Self> {
        if job.running
            || job.review.is_some()
            || job.selecting_files
            || !matches!(
                job.phase,
                Phase::Ready | Phase::Error | Phase::SentUnconfirmed | Phase::SentError
            )
        {
            return None;
        }
        Self::capture(&job.value, job.sequence)
    }

    fn current(&self, job: &Job) -> bool {
        job.phase == Phase::Reconcile
            && !job.running
            && Self::capture(&job.value, job.sequence).as_ref() == Some(self)
    }

    fn response_matches(&self, status: &MailSendStatus) -> bool {
        status.host_id == self.host
            && status.mailbox_id == self.mailbox
            && status.operation_id == self.operation
            && status.draft_id == self.draft
            && status.draft_revision == self.revision
            && matches!(
                status.state,
                MailDispatchState::Accepted | MailDispatchState::Uncertain
            )
    }

    fn request(
        &self,
        snapshot: &MailboxesSnapshot,
        connection: ServiceConnection,
    ) -> Result<MailSentReconcileRequest, String> {
        if snapshot.host_id != self.host
            || !snapshot
                .mailboxes
                .iter()
                .any(|row| row.mailbox_id == self.mailbox && row.provider == self.provider)
        {
            return Err(WRONG_ACCOUNT.into());
        }
        if connection.provider != self.provider || connection.state != ConnectionState::Connected {
            return Err(RECONNECT.into());
        }
        let connection_id = connection.id.filter(|id| !id.is_empty()).ok_or(RECONNECT)?;
        Ok(MailSentReconcileRequest {
            host_id: self.host.clone(),
            connection_id,
        })
    }
}

pub(super) fn available(job: &Job) -> bool {
    Target::start(job).is_some()
}

fn lookup_error(error: crate::server_client::ServerError) -> String {
    use crate::server_client::ServerError;
    match error {
        ServerError::Status { code, .. } => match code.as_u16() {
            401 | 403 => RECONNECT,
            409 => WRONG_ACCOUNT,
            429 => "The provider is limiting requests. Wait before checking Sent again. Nothing was resent.",
            _ => "Sent could not be checked. Check saved send status or try Check Sent again. Nothing was resent.",
        },
        _ => "Sent could not be checked. Check saved send status or try Check Sent again. Nothing was resent.",
    }.into()
}

async fn current_request(
    client: &ServerClient,
    target: &Target,
) -> Result<MailSentReconcileRequest, String> {
    let snapshot = client.mailboxes().await.map_err(lookup_error)?;
    if snapshot.host_id != target.host
        || !snapshot
            .mailboxes
            .iter()
            .any(|row| row.mailbox_id == target.mailbox && row.provider == target.provider)
    {
        return Err(WRONG_ACCOUNT.into());
    }
    let connection = client
        .mail_connection(target.provider)
        .await
        .map_err(lookup_error)?;
    target.request(&snapshot, connection)
}

impl MailComposer {
    fn sent_current(&self, k: &str, target: &Target, generation: u64) -> bool {
        !self.quitting
            && self.active.as_deref() == Some(k)
            && self.generation == generation
            && self.client.mail_compose_origin() == target.origin
            && self.jobs.get(k).is_some_and(|job| target.current(job))
    }

    fn sent_failure(
        &mut self,
        k: &str,
        target: &Target,
        generation: u64,
        message: String,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(job) = self.jobs.get_mut(k) {
            if target.current(job) {
                job.phase = Phase::SentError;
                job.error = Some(message);
                self.focus_sent_feedback = Some((k.to_owned(), generation));
            }
        }
        cx.notify();
    }

    /// A user action, never invoked by restore, retry-save, status GET or polling.
    pub(super) fn check_sent(&mut self, cx: &mut Context<'_, Self>) {
        if self.quitting || self.loading {
            return;
        }
        let (Some(k), Some(store)) = (self.active.clone(), self.store.clone()) else {
            return;
        };
        let generation = self.generation;
        let target = {
            let Some(job) = self.jobs.get_mut(&k) else {
                return;
            };
            let Some(target) = Target::start(job) else {
                return;
            };
            if self.client.mail_compose_origin() != target.origin {
                job.phase = Phase::SentError;
                job.error = Some(WRONG_ACCOUNT.into());
                self.focus_sent_feedback = Some((k, generation));
                cx.notify();
                return;
            }
            job.phase = Phase::Reconcile;
            job.error = None;
            target
        };
        let client = self.client.clone();
        let preflight_target = target.clone();
        let preflight = operation(cx, async move {
            current_request(&client, &preflight_target).await
        });
        cx.spawn(async move |this, cx| {
            let request = preflight.await;
            let ready = this.update(cx, |this, cx| {
                if !this.sent_current(&k, &target, generation) {
                    this.sent_failure(&k, &target, generation, CONTEXT_CHANGED.into(), cx); return None;
                }
                match request {
                    Ok(request) => Some((this.client.clone(), request)),
                    Err(message) => { this.sent_failure(&k, &target, generation, message, cx); None }
                }
            });
            let Ok(Some((client, request))) = ready else { return; };
            let mailbox = target.mailbox.clone(); let operation_id = target.operation.clone();
            let work = cx.update(|cx| operation(cx, async move {
                client.mail_send_reconcile(&mailbox, &operation_id, &request).await.map_err(lookup_error)
            }));
            let result = work.await;
            let snapshot = this.update(cx, |this, cx| {
                if !this.sent_current(&k, &target, generation) {
                    this.sent_failure(&k, &target, generation, CONTEXT_CHANGED.into(), cx); return None;
                }
                let status = match result {
                    Ok(status) if target.response_matches(&status) => status,
                    Ok(_) => { this.sent_failure(&k, &target, generation, CONTEXT_CHANGED.into(), cx); return None; }
                    Err(message) => { this.sent_failure(&k, &target, generation, message, cx); return None; }
                };
                let job = this.jobs.get_mut(&k)?;
                if status.state == MailDispatchState::Uncertain {
                    // Keep the full original operation/problem and copy exact.
                    job.phase = Phase::SentUnconfirmed; job.error = None;
                    this.focus_sent_feedback = Some((k.clone(), generation)); cx.notify(); return None;
                }
                job.value.latest_send = Some(status);
                job.value.pending_send = None;
                job.sequence = job.sequence.wrapping_add(1);
                job.running = true;
                let snapshot = (job.value.clone(), job.revision, job.sequence,
                    job.added.iter().map(|(id, bytes)| DeviceFileBytes { id: id.clone(), bytes: bytes.clone() }).collect::<Vec<_>>());
                cx.notify(); Some(snapshot)
            });
            let Ok(Some((value, revision, sequence, added))) = snapshot else { return; };
            let saved_ids = added.iter().map(|file| file.id.clone()).collect::<Vec<_>>();
            // Only persist the receipt locally; no server autosave or refresh.
            let save = cx.update(|cx| operation(cx, async move { store.save(value, revision, added).await.map_err(|e| e.to_string()) }));
            let result = save.await;
            let _ = this.update(cx, |this, cx| {
                let Some(job) = this.jobs.get_mut(&k) else { return; };
                if job.phase != Phase::Reconcile || job.sequence != sequence { return; }
                job.running = false;
                match result {
                    Ok(record) => {
                        job.revision = Some(record.revision); job.saved = sequence;
                        for id in saved_ids { job.added.remove(&id); }
                        job.phase = Phase::Ready;
                        job.error = job.value.dirty.then(|| "Your newer edits remain on this device.".into());
                    }
                    Err(error) => {
                        job.phase = Phase::Error;
                        job.error = Some(format!("Sent was verified, but its status could not be saved on this device. Keep this editor open. {error}"));
                    }
                }
                this.focus_sent_feedback = Some((k, generation)); cx.notify();
            });
        }).detach();
        cx.notify();
    }
}

#[cfg(test)]
mod tests;
