use super::*;

enum Step {
    Created(MailComposeDraft),
    Uploaded(String, MailDraftFile),
    Saved(MailComposeDraft),
    Removed(String),
    Done,
}
impl MailComposer {
    fn fail(&mut self, k: &str, error: String, cx: &mut Context<'_, Self>) {
        if let Some(job) = self.jobs.get_mut(k) {
            job.running = false;
            job.phase = Phase::Error;
            job.error = Some(error);
        }
        cx.notify();
    }
    pub(super) fn kick(&mut self, k: String, cx: &mut Context<'_, Self>) {
        if self.quitting {
            return;
        }
        let Some(store) = self.store.clone() else {
            return;
        };
        let Some(job) = self.jobs.get_mut(&k) else {
            return;
        };
        if job.running
            || matches!(
                job.phase,
                Phase::Review | Phase::Send | Phase::Status | Phase::Reconcile
            )
        {
            return;
        }
        job.running = true;
        job.error = None;
        cx.spawn(async move |this, cx| loop {
            let snapshot = this.update(cx, |this, cx| {
                if this.quitting {
                    return None;
                }
                let job = this.jobs.get_mut(&k)?;
                job.phase = Phase::Local;
                cx.notify();
                Some((
                    job.value.clone(),
                    job.revision,
                    job.sequence,
                    job.added
                        .iter()
                        .map(|(id, bytes)| DeviceFileBytes {
                            id: id.clone(),
                            bytes: bytes.clone(),
                        })
                        .collect::<Vec<_>>(),
                    this.client.clone(),
                ))
            });
            let Ok(Some((value, revision, sequence, added, client))) = snapshot else {
                return;
            };
            let saved_ids = added.iter().map(|file| file.id.clone()).collect::<Vec<_>>();
            let local = store.clone();
            let candidate = value.clone();
            let work = cx.update(|cx| {
                operation(cx, async move {
                    local
                        .save(candidate, revision, added)
                        .await
                        .map_err(|e| e.to_string())
                })
            });
            let record = match work.await {
                Ok(record) => record,
                Err(error) => {
                    let _ = this.update(cx, |this, cx| this.fail(&k, error, cx));
                    return;
                }
            };
            let next = this.update(cx, |this, _| {
                if this.quitting {
                    return None;
                }
                let job = this.jobs.get_mut(&k)?;
                job.revision = Some(record.revision);
                job.saved = sequence;
                for id in saved_ids {
                    job.added.remove(&id);
                }
                Some(job.sequence != sequence)
            });
            match next {
                Ok(Some(true)) => continue,
                Ok(Some(false)) => {}
                _ => return,
            }
            let local = store.clone();
            let stage = if value.pending_send.is_some() {
                Phase::Local
            } else if value.server_revision == 0 || !value.pending_remove.is_empty() {
                Phase::Server
            } else if value.files.iter().any(|file| file.server.is_none()) {
                Phase::Upload
            } else {
                Phase::Server
            };
            let _ = this.update(cx, |this, cx| {
                if let Some(job) = this.jobs.get_mut(&k) {
                    job.phase = stage;
                }
                cx.notify();
            });
            let work = cx.update(|cx| {
                operation(cx, async move {
                    // Persist recovery acknowledgements without resuming any
                    // server writer while the earlier send remains unresolved.
                    if value.pending_send.is_some() {
                        return Ok(Step::Done);
                    }
                    if value.server_revision == 0 {
                        return client
                            .mail_draft_create(
                                &value.scope.mailbox_id,
                                &MailDraftCreate {
                                    host_id: value.scope.host_id,
                                    client_id: value.client_id,
                                    reply: value.reply,
                                },
                            )
                            .await
                            .map(Step::Created)
                            .map_err(server_error);
                    }
                    if value.dirty
                        && (!value.pending_remove.is_empty()
                            || value.files.iter().all(|file| file.server.is_some()))
                    {
                        return save_attempt(&client, &value).await.map(Step::Saved);
                    }
                    if let Some(id) = value.pending_remove.first() {
                        client
                            .mail_draft_remove_upload(
                                &value.scope.host_id,
                                &value.scope.mailbox_id,
                                &value.client_id,
                                id,
                            )
                            .await
                            .map_err(server_error)?;
                        return Ok(Step::Removed(id.clone()));
                    }
                    if let Some(file) = value.files.iter().find(|file| file.server.is_none()) {
                        let bytes = local
                            .file(
                                value.scope.clone(),
                                value.client_id.clone(),
                                file.id.clone(),
                            )
                            .await
                            .map_err(|e| e.to_string())?;
                        return client
                            .mail_draft_upload(
                                &value.scope.host_id,
                                &value.scope.mailbox_id,
                                &value.client_id,
                                &file.id,
                                &file.file_name,
                                bytes,
                            )
                            .await
                            .map(|uploaded| Step::Uploaded(file.id.clone(), uploaded))
                            .map_err(server_error);
                    }
                    Ok(Step::Done)
                })
            });
            match work.await {
                Ok(step) => {
                    let done = matches!(step, Step::Done);
                    let result = this.update(cx, |this, cx| {
                        let Some(job) = this.jobs.get_mut(&k) else {
                            return;
                        };
                        match step {
                            Step::Created(draft) => {
                                job.value.server_revision = draft.revision;
                                job.value.latest_send = draft.latest_send;
                            }
                            Step::Uploaded(id, file) => {
                                if let Some(selected) = job
                                    .value
                                    .files
                                    .iter_mut()
                                    .find(|selected| selected.id == id)
                                {
                                    if selected.size_bytes != file.size_bytes {
                                        job.running = false;
                                        job.phase = Phase::Error;
                                        job.error = Some(
                                            "The uploaded file did not match its selected size."
                                                .into(),
                                        );
                                        return;
                                    }
                                    selected.server = Some(file);
                                } else {
                                    job.value.pending_remove.push(file.id);
                                }
                                job.value.dirty = true;
                                job.value.content.attachments = job
                                    .value
                                    .files
                                    .iter()
                                    .filter_map(|file| {
                                        file.server.as_ref().map(|file| file.id.clone())
                                    })
                                    .collect();
                            }
                            Step::Saved(draft) => {
                                job.value.server_revision = draft.revision;
                                job.value.latest_send = draft.latest_send;
                                if job.sequence == sequence {
                                    job.value.dirty = false;
                                }
                            }
                            Step::Removed(id) => job.value.pending_remove.retain(|old| old != &id),
                            Step::Done => {
                                job.running = false;
                                job.phase = Phase::Ready;
                                cx.notify();
                                return;
                            }
                        }
                        job.sequence = job.sequence.wrapping_add(1);
                        cx.notify();
                    });
                    if result.is_err() || done {
                        return;
                    }
                }
                Err(error) => {
                    let _ = this.update(cx, |this, cx| this.fail(&k, error, cx));
                    return;
                }
            }
        })
        .detach();
    }
    pub(super) fn review(&mut self, cx: &mut Context<'_, Self>) {
        let Some(k) = self.active.clone() else {
            return;
        };
        let Some(job) = self.jobs.get_mut(&k) else {
            return;
        };
        if job.running
            || job.locked()
            || job.value.dirty
            || job.value.server_revision == 0
            || job.value.files.iter().any(|file| file.server.is_none())
        {
            return;
        }
        if let Err(error) = job.value.content.validate_send() {
            job.error = Some(error.to_string());
            cx.notify();
            return;
        }
        job.phase = Phase::Review;
        job.error = None;
        let value = job.value.clone();
        let client = self.client.clone();
        let work = operation(cx, async move {
            let connection = client
                .mail_connection(value.provider)
                .await
                .map_err(server_error)?;
            let id = connection
                .id
                .filter(|_| connection.state == ConnectionState::Connected)
                .ok_or_else(|| {
                    "Reconnect the account that owns this draft before reviewing.".to_string()
                })?;
            client
                .mail_draft_review(
                    &value.scope.mailbox_id,
                    &value.client_id,
                    &MailReviewRequest {
                        host_id: value.scope.host_id,
                        connection_id: id,
                        expected_revision: value.server_revision,
                    },
                )
                .await
                .map_err(server_error)
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(review) => {
                        if let Some(job) = this.jobs.get_mut(&k) {
                            job.value.server_revision = review.revision;
                            job.review = Some(review);
                            job.phase = Phase::Ready;
                            job.sequence = job.sequence.wrapping_add(1);
                        }
                        this.focus_review = true;
                        this.kick(k, cx);
                    }
                    Err(error) => this.fail(&k, error, cx),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn send(&mut self, cx: &mut Context<'_, Self>) {
        let (Some(k), Some(store)) = (self.active.clone(), self.store.clone()) else {
            return;
        };
        let Some(job) = self.jobs.get_mut(&k) else {
            return;
        };
        if job.running
            || job.value.pending_send.is_some()
            || job.sequence != job.saved
            || matches!(
                job.phase,
                Phase::Review | Phase::Send | Phase::Status | Phase::Reconcile
            )
        {
            return;
        }
        let Some(review) = job.review.clone() else {
            return;
        };
        let request = MailSendRequest {
            host_id: review.host_id,
            connection_id: review.connection_id,
            operation_id: uuid::Uuid::new_v4().to_string(),
            revision: review.revision,
            digest: review.digest,
        };
        job.value.pending_send = Some(request.clone());
        job.sequence = job.sequence.wrapping_add(1);
        job.phase = Phase::Send;
        job.error = None;
        let value = job.value.clone();
        let revision = job.revision;
        let sequence = job.sequence;
        let client = self.client.clone();
        let save = operation(cx, async move {
            store
                .save(value.clone(), revision, vec![])
                .await
                .map(|record| (value, record.revision))
                .map_err(|e| e.to_string())
        });
        cx.spawn(async move |this, cx| {
            let (value, revision) = match save.await {
                Ok(value) => value,
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        if let Some(job) = this.jobs.get_mut(&k) {
                            job.value.pending_send = None;
                            job.sequence = job.sequence.wrapping_add(1);
                        }
                        this.fail(&k, error, cx);
                    });
                    return;
                }
            };
            let _ = this.update(cx, |this, _| {
                if let Some(job) = this.jobs.get_mut(&k) {
                    job.revision = Some(revision);
                    job.saved = sequence;
                }
            });
            let send = cx.update(|cx| {
                operation(cx, async move {
                    client
                        .mail_draft_send(&value.scope.mailbox_id, &value.client_id, &request)
                        .await
                        .map_err(server_error)
                })
            });
            let result = send.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(status) => this.finish(&k, status, cx),
                    Err(error) => {
                        this.fail(&k, error, cx);
                        this.check_status(k, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn finish(&mut self, k: &str, status: MailSendStatus, cx: &mut Context<'_, Self>) {
        let Some(job) = self.jobs.get_mut(k) else {
            return;
        };
        if job
            .value
            .pending_send
            .as_ref()
            .is_some_and(|request| request.operation_id != status.operation_id)
        {
            return;
        }
        if job.value.latest_send.as_ref().is_some_and(|old| {
            old.operation_id == status.operation_id
                && old.state == MailDispatchState::Accepted
                && status.state != MailDispatchState::Accepted
        }) {
            return;
        }
        if !matches!(
            status.state,
            MailDispatchState::Queued
                | MailDispatchState::Dispatching
                | MailDispatchState::Uncertain
        ) {
            job.value.pending_send = None;
        }
        job.error = status.problem.as_ref().map(ToString::to_string);
        job.value.latest_send = Some(status);
        job.review = None;
        job.phase = Phase::Ready;
        job.sequence = job.sequence.wrapping_add(1);
        self.kick(k.to_owned(), cx);
    }
    pub(super) fn check_status(&mut self, k: String, cx: &mut Context<'_, Self>) {
        let Some(job) = self.jobs.get_mut(&k) else {
            return;
        };
        if job.running
            || matches!(
                job.phase,
                Phase::Review | Phase::Send | Phase::Status | Phase::Reconcile
            )
        {
            return;
        }
        let Some(request) = job.value.pending_send.clone() else {
            return;
        };
        job.phase = Phase::Status;
        let scope = job.value.scope.clone();
        let client = self.client.clone();
        let work = operation(cx, async move {
            client
                .mail_send_status(&scope.host_id, &scope.mailbox_id, &request.operation_id)
                .await
                .map_err(server_error)
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(status) => this.finish(&k, status, cx),
                    Err(error) => this.fail(&k, error, cx),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn cancel(&mut self, cx: &mut Context<'_, Self>) {
        let Some(k) = self.active.clone() else {
            return;
        };
        let Some(job) = self.jobs.get_mut(&k) else {
            return;
        };
        if job.running
            || matches!(
                job.phase,
                Phase::Review | Phase::Send | Phase::Status | Phase::Reconcile
            )
            || job
                .value
                .latest_send
                .as_ref()
                .is_none_or(|status| status.state != MailDispatchState::Queued)
        {
            return;
        }
        let Some(request) = job.value.pending_send.clone() else {
            return;
        };
        let scope = job.value.scope.clone();
        let client = self.client.clone();
        job.phase = Phase::Status;
        let work = operation(cx, async move {
            client
                .mail_send_cancel(&scope.host_id, &scope.mailbox_id, &request.operation_id)
                .await
                .map_err(server_error)
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(status) => this.finish(&k, status, cx),
                Err(error) => this.fail(&k, error, cx),
            });
        })
        .detach();
    }
    pub(super) fn remove(&mut self, id: String, cx: &mut Context<'_, Self>) {
        let Some(k) = self.active.clone() else {
            return;
        };
        let Some(job) = self.jobs.get_mut(&k) else {
            return;
        };
        if job.locked() || job.phase == Phase::Upload {
            return;
        }
        if let Some(index) = job.value.files.iter().position(|file| file.id == id) {
            let file = job.value.files.remove(index);
            job.added.remove(&id);
            if let Some(server) = file.server {
                job.value.pending_remove.push(server.id);
            }
        }
        job.value.content.attachments = job
            .value
            .files
            .iter()
            .filter_map(|file| file.server.as_ref().map(|file| file.id.clone()))
            .collect();
        job.value.dirty = true;
        job.value.updated_at_ms = EpochMs::now().as_millis();
        job.sequence = job.sequence.wrapping_add(1);
        self.kick(k, cx);
    }
    pub(super) fn attach(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(k) = self.active.clone() else {
            return;
        };
        let Some(job) = self.jobs.get_mut(&k) else {
            return;
        };
        if job.locked() || job.phase == Phase::Upload {
            return;
        }
        job.selecting_files = true;
        cx.notify();
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach email files".into()),
        });
        let this = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let paths = match picker.await {
                    Ok(Ok(Some(paths))) => paths,
                    Ok(Ok(None)) => {
                        let _ = this.update(cx, |this, cx| {
                            if let Some(job) = this.jobs.get_mut(&k) {
                                job.selecting_files = false;
                            }
                            cx.notify();
                        });
                        return;
                    }
                    _ => {
                        let _ = this.update(cx, |this, cx| {
                            if let Some(job) = this.jobs.get_mut(&k) {
                                job.selecting_files = false;
                            }
                            this.fail(
                                &k,
                                "The file dialog could not be opened. Retry choosing files.".into(),
                                cx,
                            )
                        });
                        return;
                    }
                };
                let loaded = cx.background_spawn(async move { read_files(paths) }).await;
                let _ = this.update(cx, |this, cx| {
                    if let Some(job) = this.jobs.get_mut(&k) {
                        job.selecting_files = false;
                    }
                    match loaded {
                        Ok(files) => {
                            let Some(job) = this.jobs.get_mut(&k) else {
                                return;
                            };
                            if job.locked() {
                                job.error=Some("The email state changed while reading files. Choose the files again before reviewing.".into());
                                cx.notify();
                                return;
                            }
                            let total = job
                                .value
                                .files
                                .iter()
                                .map(|file| file.size_bytes)
                                .sum::<u64>()
                                .saturating_add(
                                    files
                                        .iter()
                                        .map(|(_, _, bytes)| {
                                            u64::try_from(bytes.len()).unwrap_or(u64::MAX)
                                        })
                                        .sum::<u64>(),
                                );
                            if job.value.files.len().saturating_add(files.len())
                                > MAIL_DRAFT_MAX_ATTACHMENTS
                                || total
                                    > u64::try_from(MAIL_DRAFT_ATTACHMENT_MAX_BYTES)
                                        .unwrap_or(u64::MAX)
                            {
                                job.error = Some(
                                    "Keep this draft within 20 files and 16 MiB combined.".into(),
                                );
                                cx.notify();
                                return;
                            }
                            for (id, name, bytes) in files {
                                job.value.files.push(DeviceDraftFile {
                                    id: id.clone(),
                                    file_name: name,
                                    size_bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                                    server: None,
                                });
                                job.added.insert(id, bytes);
                            }
                            job.value.dirty = true;
                            job.value.updated_at_ms = EpochMs::now().as_millis();
                            job.sequence = job.sequence.wrapping_add(1);
                            this.kick(k, cx);
                        }
                        Err(error) => this.fail(&k, error, cx),
                    }
                    cx.notify();
                });
            })
            .detach();
    }
}
fn read_files(paths: Vec<PathBuf>) -> Result<Vec<(String, String, Vec<u8>)>, String> {
    use std::io::Read as _;
    if paths.len() > MAIL_DRAFT_MAX_ATTACHMENTS {
        return Err("Choose up to 20 files.".into());
    }
    let mut total = 0usize;
    let mut files = Vec::new();
    for path in paths {
        let file = std::fs::File::open(&path)
            .map_err(|_| "A selected file could not be opened. Choose it again.".to_string())?;
        if !file
            .metadata()
            .map_err(|_| "A selected file could not be inspected.".to_string())?
            .is_file()
        {
            return Err("Choose regular files for this email.".into());
        }
        let mut bytes = Vec::new();
        file.take(
            u64::try_from(
                MAIL_DRAFT_ATTACHMENT_MAX_BYTES
                    .saturating_sub(total)
                    .saturating_add(1),
            )
            .unwrap_or(u64::MAX),
        )
        .read_to_end(&mut bytes)
        .map_err(|_| "A selected file could not be read. Choose it again.".to_string())?;
        total = total.saturating_add(bytes.len());
        if total > MAIL_DRAFT_ATTACHMENT_MAX_BYTES {
            return Err("Keep draft files within 16 MiB combined.".into());
        }
        let name = attachment_file_name(&path.file_name().unwrap_or_default().to_string_lossy());
        files.push((uuid::Uuid::new_v4().to_string(), name, bytes));
    }
    Ok(files)
}

pub(super) async fn save_attempt(
    client: &ServerClient,
    value: &DeviceDraft,
) -> Result<MailComposeDraft, String> {
    let result = client
        .mail_draft_save(
            &value.scope.mailbox_id,
            &value.client_id,
            &MailDraftSave {
                host_id: value.scope.host_id.clone(),
                expected_revision: value.server_revision,
                content: value.content.clone(),
            },
        )
        .await;
    match result {
        Ok(saved) => Ok(saved),
        Err(error) => match client
            .mail_draft(
                &value.scope.host_id,
                &value.scope.mailbox_id,
                &value.client_id,
            )
            .await
        {
            Ok(saved)
                if saved.host_id == value.scope.host_id
                    && saved.mailbox_id == value.scope.mailbox_id
                    && saved.client_id == value.client_id
                    && saved.reply == value.reply
                    && saved.revision > value.server_revision
                    && saved.content == value.content =>
            {
                Ok(saved)
            }
            _ => Err(server_error(error)),
        },
    }
}
