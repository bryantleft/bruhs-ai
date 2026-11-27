//! A project grant owns the threads and send receipts it creates.
use super::*;
use bruhs_models::{
    coordination::linked_thread::{
        CheckoutContext, LinkedAttachment, LinkedProjectReply, LinkedThread, LinkedThreadAction,
        LinkedThreadSummary, StartThread,
    },
    delivery::generation::{model_options, GenerationOptions},
    logic::{
        attachments::UploadedFile,
        composer_text::{visible_prompt, TextAttachment},
    },
};
use sha2::{Digest, Sha256};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct LinkedThreadAccess {
    grant: String,
    #[serde(default)]
    request_id: Option<ThreadId>,
    pub(super) checkout: CheckoutContext,
    #[serde(default)]
    pub(super) model_options: GenerationOptions,
    start_digest: String,
    #[serde(default)]
    replies: BTreeMap<uuid::Uuid, String>,
    #[serde(default)]
    pub(super) message_attachments: BTreeMap<usize, Vec<UploadedFile>>,
}

pub(super) fn visible_linked_message(message: &str, files: &[UploadedFile]) -> String {
    if files.is_empty() {
        return message.trim().to_owned();
    }
    let named: Vec<_> = files
        .iter()
        .map(|file| TextAttachment {
            name: file.name.clone(),
            text: String::new(),
        })
        .collect();
    visible_prompt(message, &named)
}

fn grant<'a>(data: &'a Data, token: &str) -> Result<&'a str, CoordinationError> {
    let grant = access::active_grant(data, token)?;
    if !grant.start_project_threads {
        return Err(CoordinationError::Unauthorized);
    }
    Ok(&grant.id)
}

fn access<'a>(
    data: &'a Data,
    token: &str,
    id: &ThreadId,
) -> Result<&'a LinkedThreadAccess, CoordinationError> {
    let owner = grant(data, token)?;
    data.linked_threads
        .get(id)
        .filter(|entry| entry.grant == owner)
        .ok_or_else(|| not_found(id))
}

fn start_digest(start: &StartThread) -> Result<String, CoordinationError> {
    Ok(digest(&serde_json::to_vec(start).map_err(storage)?))
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

// Remote request IDs must never select an ordinary conversation or its
// provider session in the host's process.
fn host_thread_id(request: &ThreadId) -> Result<ThreadId, CoordinationError> {
    ThreadId::try_from(format!(
        "linked-project-{}",
        digest(request.as_str().as_bytes())
    ))
    .map_err(storage)
}

fn linked_id(data: &Data, id: &ThreadId) -> Result<ThreadId, CoordinationError> {
    let hashed = host_thread_id(id)?;
    // Older records only retained the host ID. They can still be reopened
    // through discovery, with the same grant check as new records.
    if data.linked_threads.contains_key(id) && !data.linked_threads.contains_key(&hashed) {
        Ok(id.clone())
    } else {
        Ok(hashed)
    }
}

fn replay(
    data: &Data,
    token: &str,
    id: &ThreadId,
    digest: &str,
) -> Result<bool, CoordinationError> {
    grant(data, token)?;
    if !data.linked_threads.contains_key(id) {
        return Ok(false);
    }
    let entry = access(data, token, id)?;
    if entry.start_digest != digest {
        return Err(conflict(
            "This conversation already started with a different first message.",
        ));
    }
    if !data.tasks.contains_key(id) {
        return Err(not_found(id));
    }
    Ok(true)
}

pub(super) fn record_start(
    data: &mut Data,
    request: ProjectThreadStart,
) -> Result<bool, CoordinationError> {
    if replay(data, &request.token, &request.id, &request.digest)? {
        return Ok(false);
    }
    if data.tasks.contains_key(&request.id) {
        return Err(conflict("That conversation ID is already in use."));
    }
    let owner = grant(data, &request.token)?.to_owned();
    let mut message_attachments = BTreeMap::new();
    if !request.attachments.is_empty() {
        message_attachments.insert(0, request.attachments);
    }
    data.linked_threads.insert(
        request.id,
        LinkedThreadAccess {
            grant: owner,
            request_id: Some(request.request_id),
            checkout: request.checkout,
            model_options: request.model_options,
            start_digest: request.digest,
            replies: BTreeMap::new(),
            message_attachments,
        },
    );
    Ok(true)
}

pub(super) struct ProjectThreadStart {
    token: String,
    pub id: ThreadId,
    request_id: ThreadId,
    pub(super) checkout: CheckoutContext,
    model_options: GenerationOptions,
    attachments: Vec<UploadedFile>,
    digest: String,
}

fn reply_digest(reply: &LinkedProjectReply) -> Result<String, CoordinationError> {
    Ok(digest(
        &serde_json::to_vec(&(&reply.message, &reply.attachments)).map_err(storage)?,
    ))
}

pub(super) fn reply_seen(
    data: &Data,
    token: &str,
    id: &ThreadId,
    reply: &LinkedProjectReply,
) -> Result<bool, CoordinationError> {
    let entry = access(data, token, id)?;
    let Some(receipt) = entry.replies.get(&reply.request_id) else {
        return Ok(false);
    };
    if receipt != &reply_digest(reply)? {
        return Err(conflict(
            "This reply was already sent with different content.",
        ));
    }
    if !data.tasks.contains_key(id) {
        return Err(not_found(id));
    }
    Ok(true)
}

pub(super) fn record_reply(
    data: &mut Data,
    id: &ThreadId,
    reply: &LinkedProjectReply,
    message_index: usize,
    attachments: Vec<UploadedFile>,
) -> Result<(), CoordinationError> {
    let digest = reply_digest(reply)?;
    if let Some(entry) = data.linked_threads.get_mut(id) {
        entry.replies.insert(reply.request_id, digest);
        if !attachments.is_empty() {
            entry.message_attachments.insert(message_index, attachments);
        }
    }
    Ok(())
}

impl CoordinationRuntime {
    pub async fn upload_project_thread_attachment(
        &self,
        token: &str,
        id: &ThreadId,
        name: String,
        bytes: Vec<u8>,
    ) -> Result<LinkedAttachment, CoordinationError> {
        let (owner, host_id) = {
            let data = self.0.data.lock().await;
            let owner = grant(&data, token)?.to_owned();
            let host_id = linked_id(&data, id)?;
            if data.linked_threads.contains_key(&host_id) {
                access(&data, token, &host_id)?;
            } else if data.tasks.contains_key(&host_id) {
                return Err(conflict("That conversation ID is already in use."));
            }
            drop(data);
            (owner, host_id)
        };
        tokio::task::spawn_blocking(move || {
            crate::uploads::write_linked_thread_upload(&owner, host_id.as_str(), &name, &bytes)
        })
        .await
        .map_err(storage)?
        .map_err(invalid)
    }

    pub async fn linked_start_replayed(
        &self,
        token: &str,
        id: &ThreadId,
        start: &StartThread,
    ) -> Result<bool, CoordinationError> {
        let digest = start_digest(start)?;
        let data = self.0.data.lock().await;
        replay(&data, token, &host_thread_id(id)?, &digest)
    }

    /// Start in a checkout already resolved and checked by the project endpoint.
    pub async fn start_project_thread(
        &self,
        token: String,
        id: ThreadId,
        project_path: String,
        start: StartThread,
    ) -> Result<ThreadId, CoordinationError> {
        let digest = start_digest(&start)?;
        if !start.options.valid_for(&start.checkout.model) {
            return Err(invalid("Choose model options supported by the host model."));
        }
        let host_id = host_thread_id(&id)?;
        let owner = {
            let data = self.0.data.lock().await;
            if replay(&data, &token, &host_id, &digest)? {
                return Ok(id);
            }
            grant(&data, &token)?.to_owned()
        };
        let resolved = if start.attachments.is_empty() {
            Vec::new()
        } else {
            let attachments = start.attachments.clone();
            let host_key = host_id.as_str().to_owned();
            tokio::task::spawn_blocking(move || {
                crate::uploads::resolve_linked_thread_uploads(&owner, &host_key, &attachments)
            })
            .await
            .map_err(storage)?
            .map_err(invalid)?
        };
        let visible = visible_linked_message(&start.message, &resolved);
        self.create_with_origin(
            NewTask {
                title: bruhs_models::logic::portable::title_from_prompt(&visible),
                project_path,
                prompt: visible,
                model: start.checkout.model.clone(),
                effort: None,
                execution_timeout_seconds: None,
                permission_mode: PermissionMode::Default,
                isolation: Isolation::Shared,
                spec_id: None,
            },
            None,
            Relationship::Root,
            CreationOrigin::Linked(Box::new(ProjectThreadStart {
                token,
                id: host_id,
                request_id: id.clone(),
                checkout: start.checkout,
                model_options: start.options,
                attachments: resolved,
                digest,
            })),
        )
        .await?;
        Ok(id)
    }

    pub async fn project_thread(
        &self,
        token: &str,
        id: &ThreadId,
    ) -> Result<LinkedThread, CoordinationError> {
        let data = self.0.data.lock().await;
        let host_id = linked_id(&data, id)?;
        let entry = access(&data, token, &host_id)?;
        let task = data.tasks.get(&host_id).ok_or_else(|| not_found(id))?;
        let thread = LinkedThread {
            id: id.clone(),
            title: task.title.clone(),
            checkout: entry.checkout.clone(),
            state: task.state,
            messages: task.messages.clone(),
            live_output: task.live_output.clone(),
            error: task.error.clone(),
            approvals: if access::active_grant(&data, token)?.approve_project_threads {
                task.approvals.clone()
            } else {
                Vec::new()
            },
            options: entry.model_options.clone(),
        };
        drop(data);
        Ok(thread)
    }

    pub async fn reply_to_project_thread(
        &self,
        token: &str,
        id: &ThreadId,
        reply: LinkedProjectReply,
    ) -> Result<(), CoordinationError> {
        let (owner, host_id) = {
            let data = self.0.data.lock().await;
            let host_id = linked_id(&data, id)?;
            if reply_seen(&data, token, &host_id, &reply)? {
                return Ok(());
            }
            let owner = access(&data, token, &host_id)?.grant.clone();
            drop(data);
            (owner, host_id)
        };
        let resolved = if reply.attachments.is_empty() {
            Vec::new()
        } else {
            let attachments = reply.attachments.clone();
            let host_key = host_id.as_str().to_owned();
            tokio::task::spawn_blocking(move || {
                crate::uploads::resolve_linked_thread_uploads(&owner, &host_key, &attachments)
            })
            .await
            .map_err(storage)?
            .map_err(invalid)?
        };
        let visible = visible_linked_message(&reply.message, &resolved);
        self.send_with_receipt(&host_id, visible, Some((token, &reply, resolved)))
            .await
    }

    pub async fn linked_reply_replayed(
        &self,
        token: &str,
        id: &ThreadId,
        reply: &LinkedProjectReply,
    ) -> Result<bool, CoordinationError> {
        let data = self.0.data.lock().await;
        reply_seen(&data, token, &linked_id(&data, id)?, reply)
    }

    pub async fn project_threads(
        &self,
        token: &str,
    ) -> Result<Vec<LinkedThreadSummary>, CoordinationError> {
        let data = self.0.data.lock().await;
        let owner = grant(&data, token)?;
        let mut threads: Vec<_> = data
            .linked_threads
            .iter()
            .filter(|(_, entry)| entry.grant == owner)
            .filter_map(|(id, entry)| {
                let task = data
                    .tasks
                    .get(id)
                    .filter(|task| task.archived_at.is_none())?;
                Some(LinkedThreadSummary {
                    id: entry.request_id.as_ref().unwrap_or(id).clone(),
                    title: task.title.clone(),
                    checkout: entry.checkout.clone(),
                    updated_at: task.updated_at,
                })
            })
            .collect();
        drop(data);
        threads.sort_by_key(|thread| std::cmp::Reverse(thread.updated_at));
        Ok(threads)
    }

    pub async fn share_project_thread(
        &self,
        token: &str,
        id: &ThreadId,
    ) -> Result<IssuedAccess, CoordinationError> {
        let mut data = self.0.data.lock().await;
        let host_id = linked_id(&data, id)?;
        access(&data, token, &host_id)?;
        let parent = access::active_grant(&data, token)?.clone();
        if !parent.share_project_threads {
            return Err(CoordinationError::Unauthorized);
        }
        if !data.tasks.contains_key(&host_id) {
            return Err(not_found(id));
        }
        let issued = self.issue_linked_guest(&mut data, parent, host_id).await?;
        drop(data);
        Ok(issued)
    }

    pub async fn act_on_project_thread(
        &self,
        token: &str,
        id: &ThreadId,
        action: LinkedThreadAction,
    ) -> Result<(), CoordinationError> {
        let mut data = self.0.data.lock().await;
        let host_id = linked_id(&data, id)?;
        access(&data, token, &host_id)?;
        if matches!(&action, LinkedThreadAction::Approval { .. })
            && !access::active_grant(&data, token)?.approve_project_threads
        {
            return Err(CoordinationError::Unauthorized);
        }
        let task = data.tasks.get_mut(&host_id).ok_or_else(|| not_found(id))?;
        match action {
            LinkedThreadAction::Stop => {
                if !task.state.active() && task.state != TaskState::Ready {
                    drop(data);
                    return Ok(());
                }
                task.state = TaskState::Cancelled;
                task.updated_at = EpochMs::now();
                task.resume_at = None;
                task.approvals.clear();
                if !task.live_output.is_empty() {
                    task.messages.push(TaskMessage {
                        role: "assistant".into(),
                        text: std::mem::take(&mut task.live_output),
                        at: task.updated_at,
                    });
                }
                self.persist(&mut data).await?;
                drop(data);
                if let Some(cancel) = self.0.cancels.lock().await.get(&host_id) {
                    cancel.send_replace(true);
                }
            }
            LinkedThreadAction::Approval { request_id, allow } => {
                if !task
                    .approvals
                    .iter()
                    .any(|approval| approval.id == request_id)
                {
                    return Err(conflict(
                        "That approval is no longer pending for this conversation.",
                    ));
                }
                // The approval belongs to this task and the grant is still valid.
                let decision = if allow {
                    crate::PermissionDecision::Allow
                } else {
                    crate::PermissionDecision::Deny { reason: None }
                };
                self.0
                    .app
                    .resolve_permission(&request_id, decision)
                    .await
                    .map_err(|error| conflict(&error.to_string()))?;
                answered_approval(task, &request_id);
                self.persist(&mut data).await?;
                drop(data);
            }
            LinkedThreadAction::SetModel { model } => {
                if task.state.active() || task.state == TaskState::Ready {
                    return Err(conflict(
                        "Wait for the current run to finish before switching models.",
                    ));
                }
                if task.model == model {
                    drop(data);
                    return Ok(());
                }
                task.model = model.clone();
                task.session_id = None;
                task.effort = None;
                task.updated_at = EpochMs::now();
                if let Some(entry) = data.linked_threads.get_mut(&host_id) {
                    entry.checkout.model = model;
                    entry.model_options = GenerationOptions::default();
                }
                self.persist(&mut data).await?;
                drop(data);
            }
            LinkedThreadAction::SetModelOption { model, kind, value } => {
                if task.model != model || task.state.active() || task.state == TaskState::Ready {
                    return Err(conflict("Refresh the conversation and wait for its current run before changing options."));
                }
                let option = model_options(&model)
                    .into_iter()
                    .find(|option| option.kind == kind)
                    .ok_or_else(|| invalid("This model does not offer that option."))?;
                let entry = data
                    .linked_threads
                    .get_mut(&host_id)
                    .ok_or_else(|| not_found(id))?;
                if !entry.model_options.set(&option, &value) {
                    return Err(invalid("Choose a value supported by this model."));
                }
                self.persist(&mut data).await?;
                drop(data);
            }
        }
        Ok(())
    }
}

pub(super) async fn check_branch(
    path: &Path,
    expected: &Option<String>,
) -> Result<(), CoordinationError> {
    if &checkout_branch(path).await != expected {
        return Err(conflict(
            "The checkout is on a different branch. Review the host branch before sending.",
        ));
    }
    Ok(())
}

pub async fn checkout_branch(path: &Path) -> Option<String> {
    crate::git::status(path)
        .await
        .ok()
        .and_then(|status| status.branch)
}
