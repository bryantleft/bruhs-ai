//! Explicit Sent lookup, separate from automatic saved-status recovery.
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
    fn capture(value: &LocalDraft, sequence: u64) -> Option<Self> {
        let status = value.latest_send.as_ref()?;
        if status.state != MailDispatchState::Uncertain
            || status.host_id != value.host_id
            || status.mailbox_id != value.mailbox_id
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
            origin: value.origin.clone(),
            host: value.host_id.clone(),
            mailbox: value.mailbox_id.clone(),
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

fn lookup_error(error: crate::api::ApiError) -> String {
    match error {
        crate::api::ApiError::Http { status: 401 | 403, .. } => RECONNECT,
        crate::api::ApiError::Http { status: 409, .. } => WRONG_ACCOUNT,
        crate::api::ApiError::Http { status: 429, .. } => "The provider is limiting requests. Wait before checking Sent again. Nothing was resent.",
        _ => "Sent could not be checked. Check saved send status or try Check Sent again. Nothing was resent.",
    }.into()
}

async fn current_request(target: &Target) -> Result<MailSentReconcileRequest, String> {
    let snapshot = crate::api::mail_snapshot().await.map_err(lookup_error)?;
    // Do not even fetch another account's connection after a host change.
    if snapshot.host_id != target.host
        || !snapshot
            .mailboxes
            .iter()
            .any(|row| row.mailbox_id == target.mailbox && row.provider == target.provider)
    {
        return Err(WRONG_ACCOUNT.into());
    }
    let connection = crate::api::mail_connection(target.provider)
        .await
        .map_err(lookup_error)?;
    target.request(&snapshot, connection)
}

impl MailComposeState {
    pub fn can_check_sent(self) -> bool {
        !self.loading.get()
            && self.active.get().is_some_and(|value| {
                self.data()
                    .borrow()
                    .get(&key(&value))
                    .is_some_and(|job| Target::start(job).is_some())
            })
    }

    pub fn sent_check_failed(self) -> bool {
        self.active.get().is_some_and(|value| {
            self.data()
                .borrow()
                .get(&key(&value))
                .is_some_and(|job| job.phase == Phase::SentError)
        })
    }

    fn focus_sent_feedback(self, k: String, generation: u64) {
        queue_microtask(move || {
            if self.generation.try_get_untracked() != Some(generation) || !self.selected(&k) {
                return;
            }
            if let Ok(Some(element)) = document().query_selector("[data-mail-compose-status]") {
                if let Ok(element) = element.dyn_into::<web_sys::HtmlElement>() {
                    let _ = element.focus();
                }
            }
        });
    }

    fn sent_current(
        self,
        k: &str,
        data: &Rc<RefCell<HashMap<String, Job>>>,
        target: &Target,
        generation: u64,
    ) -> bool {
        self.selected(k)
            && self.generation.try_get_untracked() == Some(generation)
            && data.borrow().get(k).is_some_and(|job| target.current(job))
    }

    fn sent_failure(
        self,
        k: &str,
        data: &Rc<RefCell<HashMap<String, Job>>>,
        target: &Target,
        generation: u64,
        message: String,
    ) {
        if let Some(job) = data.borrow_mut().get_mut(k) {
            if target.current(job) {
                job.phase = Phase::SentError;
                job.error = Some(message);
            }
        }
        self.publish(k, data);
        self.focus_sent_feedback(k.to_owned(), generation);
    }

    /// Only the explicit Check Sent control calls this; restoration stays GET-only.
    pub fn check_sent(self) {
        if !self.live() || self.loading.get_untracked() {
            return;
        }
        let Some(value) = self.active.get_untracked() else {
            return;
        };
        let k = key(&value);
        let data = self.data();
        let target = {
            let mut jobs = data.borrow_mut();
            let Some(job) = jobs.get_mut(&k) else {
                return;
            };
            let Some(target) = Target::start(job) else {
                return;
            };
            if target.origin != crate::api::BASE_URL {
                job.phase = Phase::SentError;
                job.error = Some(WRONG_ACCOUNT.into());
                drop(jobs);
                self.publish(&k, &data);
                self.focus_sent_feedback(k.clone(), self.generation.get_untracked());
                return;
            }
            job.phase = Phase::Reconcile;
            job.error = None;
            target
        };
        let generation = self.generation.get_untracked();
        self.publish(&k, &data);
        wasm_bindgen_futures::spawn_local(async move {
            let request = current_request(&target).await;
            if !self.sent_current(&k, &data, &target, generation) {
                self.sent_failure(&k, &data, &target, generation, CONTEXT_CHANGED.into());
                return;
            }
            let request = match request {
                Ok(request) => request,
                Err(message) => {
                    self.sent_failure(&k, &data, &target, generation, message);
                    return;
                }
            };
            let result =
                api::mail_send_reconcile(&target.mailbox, &target.operation, &request).await;
            if !self.sent_current(&k, &data, &target, generation) {
                self.sent_failure(&k, &data, &target, generation, CONTEXT_CHANGED.into());
                return;
            }
            let status = match result {
                Ok(status) if target.response_matches(&status) => status,
                Ok(_) => {
                    self.sent_failure(&k, &data, &target, generation, CONTEXT_CHANGED.into());
                    return;
                }
                Err(error) => {
                    self.sent_failure(&k, &data, &target, generation, lookup_error(error));
                    return;
                }
            };
            if status.state == MailDispatchState::Uncertain {
                if let Some(job) = data.borrow_mut().get_mut(&k) {
                    if target.current(job) {
                        job.phase = Phase::SentUnconfirmed;
                        job.error = None;
                    }
                }
                self.publish(&k, &data);
                self.focus_sent_feedback(k, generation);
                return; // No mutation of the original operation/problem or local copy.
            }
            let (value, revision, sequence, blobs) = {
                let mut jobs = data.borrow_mut();
                let Some(job) = jobs.get_mut(&k) else {
                    return;
                };
                if !target.current(job) {
                    return;
                }
                job.value.latest_send = Some(status);
                job.value.pending_send = None;
                job.sequence = job.sequence.wrapping_add(1);
                job.running = true;
                (
                    job.value.clone(),
                    job.local_revision,
                    job.sequence,
                    job.blobs
                        .iter()
                        .map(|(id, blob)| (id.clone(), blob.clone()))
                        .collect::<Vec<_>>(),
                )
            };
            self.publish(&k, &data);
            // Persist only the status on this device. Never call the server
            // writer: newer text/files/revision and dirty state stay unchanged.
            let result = local::save(&value, revision, &blobs).await;
            if let Some(job) = data.borrow_mut().get_mut(&k) {
                if job.phase != Phase::Reconcile || job.sequence != sequence {
                    return;
                }
                job.running = false;
                match result {
                    Ok(record) => {
                        job.local_revision = Some(record.revision);
                        job.saved_sequence = sequence;
                        for (id, _) in blobs {
                            job.blobs.remove(&id);
                        }
                        job.phase = Phase::Ready;
                        job.error = job
                            .value
                            .dirty
                            .then(|| "Your newer edits remain on this device.".into());
                    }
                    Err(error) => {
                        job.phase = Phase::Error;
                        job.error = Some(format!("Sent was verified, but its status could not be saved on this device. Keep this editor open. {error}"));
                    }
                }
            }
            self.publish(&k, &data);
            self.focus_sent_feedback(k, generation);
        });
    }
}

#[cfg(test)]
mod tests;
