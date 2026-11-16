//! App-owned compose work. Per-draft writers retain newer edits across route
//! changes; a restored send id is read for status and is never posted again.

use crate::{
    api::mail_compose as api,
    storage::mail_compose::{self as local, LocalDraft, LocalFile, Scope},
};
use bruhs_models::{
    connections::{ConnectionState, ServiceProvider},
    mail::*,
    num::ToIntSaturating as _,
};
use leptos::prelude::*;
use std::{cell::RefCell, collections::HashMap, rc::Rc};
use wasm_bindgen::JsCast as _;

mod sent;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Ready,
    Local,
    Server,
    Upload,
    Review,
    Send,
    Status,
    Reconcile,
    SentUnconfirmed,
    SentError,
    Error,
}

struct Job {
    value: LocalDraft,
    local_revision: Option<u64>,
    sequence: u64,
    saved_sequence: u64,
    running: bool,
    phase: Phase,
    blobs: HashMap<String, web_sys::Blob>,
    review: Option<MailSendReview>,
    error: Option<String>,
}

#[derive(Clone, Copy)]
pub(crate) struct MailComposeState {
    pub visible: RwSignal<bool>,
    pub active: RwSignal<Option<LocalDraft>>,
    pub loading: RwSignal<bool>,
    pub busy: RwSignal<bool>,
    pub locked: RwSignal<bool>,
    pub files_locked: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
    pub status: RwSignal<String>,
    pub review: RwSignal<Option<MailSendReview>>,
    pub list_visible: RwSignal<bool>,
    pub local_rows: RwSignal<Vec<local::LocalDraftRow>>,
    pub server_rows: RwSignal<Option<MailDraftsPage>>,
    generation: RwSignal<u64>,
    alive: RwSignal<bool>,
    unsaved: RwSignal<bool>,
    list_scope: RwSignal<Option<Scope>>,
    list_provider: RwSignal<Option<ServiceProvider>>,
    pub local_more: RwSignal<bool>,
    jobs: StoredValue<Rc<RefCell<HashMap<String, Job>>>, LocalStorage>,
}

pub(super) fn provide() {
    let state = MailComposeState {
        visible: RwSignal::new(false),
        active: RwSignal::new(None),
        loading: RwSignal::new(false),
        busy: RwSignal::new(false),
        locked: RwSignal::new(false),
        files_locked: RwSignal::new(false),
        error: RwSignal::new(None),
        status: RwSignal::new(String::new()),
        review: RwSignal::new(None),
        list_visible: RwSignal::new(false),
        local_rows: RwSignal::new(Vec::new()),
        server_rows: RwSignal::new(None),
        generation: RwSignal::new(0),
        alive: RwSignal::new(true),
        unsaved: RwSignal::new(false),
        list_scope: RwSignal::new(None),
        list_provider: RwSignal::new(None),
        local_more: RwSignal::new(false),
        jobs: StoredValue::new_local(Rc::new(RefCell::new(HashMap::new()))),
    };
    on_cleanup(move || state.alive.set(false));
    provide_context(state);
}

fn key(value: &LocalDraft) -> String {
    value.scope().body_key(&value.client_id).unwrap_or_default()
}

impl MailComposeState {
    fn data(self) -> Rc<RefCell<HashMap<String, Job>>> {
        self.jobs.with_value(Rc::clone)
    }
    fn live(self) -> bool {
        self.alive.try_get_untracked() == Some(true)
    }
    fn selected(self, k: &str) -> bool {
        self.live()
            && self
                .active
                .with_untracked(|value| value.as_ref().is_some_and(|value| key(value) == k))
    }
    fn publish(self, k: &str, data: &Rc<RefCell<HashMap<String, Job>>>) {
        if !self.live() {
            return;
        }
        let (pending, view) = {
            let jobs = data.borrow();
            let pending = jobs
                .values()
                .any(|job| job.saved_sequence != job.sequence || !job.blobs.is_empty());
            let view = jobs.get(k).map(|job| {
                (
                    job.value.clone(),
                    job.error.clone().or_else(|| {
                        if job.review.is_none() && job.phase == Phase::Ready {
                            job.value
                                .latest_send
                                .as_ref()
                                .and_then(|status| status.problem.as_ref().map(ToString::to_string))
                        } else {
                            None
                        }
                    }),
                    job.review.clone(),
                    job.phase,
                    job.running,
                    job.saved_sequence == job.sequence && job.blobs.is_empty(),
                )
            });
            (pending, view)
        };
        self.unsaved.set(pending);
        if !self.selected(k) {
            return;
        }
        let Some((value, error, review, phase, running, local_saved)) = view else {
            return;
        };
        let consumed = value.latest_send.as_ref().is_some_and(|status| {
            status.state == MailDispatchState::Accepted
                && status.draft_revision == value.server_revision
                && !value.dirty
        });
        let locked = matches!(
            phase,
            Phase::Review | Phase::Send | Phase::Status | Phase::Reconcile
        ) || review.is_some()
            || value.pending_send.is_some()
            || consumed;
        let status = match phase {
            Phase::Local => "Saving on this device…",
            Phase::Server => "Saving on the mail server…",
            Phase::Upload => "Uploading the selected file…",
            Phase::Review => "Preparing the saved email for review…",
            Phase::Send => "Sending the reviewed email. You can leave Email.",
            Phase::Status => "Checking saved send status…",
            Phase::Reconcile => "Checking Sent for the reviewed email. Nothing will be resent.",
            Phase::SentUnconfirmed => "No matching Sent copy was verified. Sending remains unconfirmed; nothing was resent.",
            Phase::SentError => "Sent could not be checked. Check saved send status or try Check Sent again.",
            Phase::Error => "Your draft remains here. Resolve the error before continuing.",
            Phase::Ready if !local_saved => "Waiting to save on this device…",
            Phase::Ready if consumed => {
                "The provider accepted this email. Recipient delivery is not confirmed."
            }
            Phase::Ready if value.pending_send.is_none() && value.latest_send.as_ref().is_some_and(|status| status.state == MailDispatchState::Accepted && (status.draft_revision != value.server_revision || value.dirty)) => {
                "The reviewed version was accepted. Newer edits remain a draft; recipient delivery is not confirmed."
            }
            Phase::Ready if value.pending_send.is_some() => {
                "The send needs checking. It will not be sent again automatically."
            }
            Phase::Ready if value.dirty => "Saved on this device. Waiting for the mail server.",
            Phase::Ready if value.server_revision == 0 => "Saved on this device.",
            Phase::Ready => "Saved on this device and the mail server.",
        };
        self.active.set(Some(value));
        self.error.set(error);
        self.review.set(review);
        self.busy.set(
            running
                || matches!(
                    phase,
                    Phase::Review | Phase::Send | Phase::Status | Phase::Reconcile
                ),
        );
        self.locked.set(locked);
        self.files_locked.set(locked || phase == Phase::Upload);
        self.status.set(status.into());
    }

    fn install(self, value: LocalDraft, revision: Option<u64>) {
        let k = key(&value);
        let data = self.data();
        data.borrow_mut().entry(k.clone()).or_insert(Job {
            value: value.clone(),
            local_revision: revision,
            sequence: 0,
            saved_sequence: if revision.is_some() { 0 } else { u64::MAX },
            running: false,
            phase: Phase::Ready,
            blobs: HashMap::new(),
            review: None,
            error: None,
        });
        self.active.set(Some(value));
        self.visible.set(true);
        self.list_visible.set(false);
        self.loading.set(false);
        self.publish(&k, &data);
    }

    fn release_current(self) {
        self.active.set(None);
        self.review.set(None);
        self.data().borrow_mut().retain(|_, job| {
            job.value.dirty
                || job.running
                || matches!(
                    job.phase,
                    Phase::Review | Phase::Send | Phase::Status | Phase::Reconcile
                )
                || job.saved_sequence != job.sequence
                || !job.blobs.is_empty()
                || job.value.pending_send.is_some()
        });
    }

    pub fn new_message(
        self,
        scope: Scope,
        provider: ServiceProvider,
        reply: Option<MailReplyTarget>,
        content: MailComposeContent,
    ) {
        if !self.live() {
            return;
        }
        self.generation
            .update(|value| *value = value.wrapping_add(1));
        self.release_current();
        let value = LocalDraft {
            version: 1,
            origin: scope.origin,
            host_id: scope.host_id,
            mailbox_id: scope.mailbox_id,
            provider,
            client_id: uuid::Uuid::new_v4().to_string(),
            reply,
            content,
            server_revision: 0,
            files: Vec::new(),
            pending_remove: Vec::new(),
            pending_send: None,
            latest_send: None,
            dirty: true,
            updated_at_ms: crate::clock::now_ms(),
        };
        self.install(value, None);
        self.kick();
    }

    pub fn open(self, scope: Scope, provider: ServiceProvider, client: String) {
        self.generation
            .update(|value| *value = value.wrapping_add(1));
        let generation = self.generation.get_untracked();
        self.visible.set(true);
        self.list_visible.set(false);
        self.loading.set(true);
        self.error.set(None);
        self.review.set(None);
        let data = self.data();
        let k = scope.body_key(&client).unwrap_or_default();
        if let Some(value) = data.borrow().get(&k).map(|job| job.value.clone()) {
            self.active.set(Some(value));
            self.loading.set(false);
            self.publish(&k, &data);
            return;
        }
        wasm_bindgen_futures::spawn_local(async move {
            let cached = local::load(&scope, &client).await;
            if !self.live() || self.generation.get_untracked() != generation {
                return;
            }
            match cached {
                Ok(Some(record)) => {
                    let pending = record.value.pending_send.is_some();
                    let dirty = record.value.dirty;
                    self.install(record.value, Some(record.revision));
                    if pending {
                        self.check_send();
                    } else if dirty {
                        self.kick();
                    } else {
                        self.refresh_clean(k, data);
                    }
                }
                Ok(None) => match api::mail_draft(&scope.host_id, &scope.mailbox_id, &client).await
                {
                    Ok(draft) if self.live() && self.generation.get_untracked() == generation => {
                        self.install(LocalDraft::from_server(scope, provider, draft), None);
                        self.kick();
                    }
                    Err(error) if self.live() && self.generation.get_untracked() == generation => {
                        self.loading.set(false);
                        self.error.set(Some(error.to_string()));
                    }
                    _ => {}
                },
                Err(error) => {
                    self.loading.set(false);
                    self.error.set(Some(error.to_string()));
                }
            }
        });
    }

    fn refresh_clean(self, k: String, data: Rc<RefCell<HashMap<String, Job>>>) {
        let Some((value, sequence)) = data
            .borrow()
            .get(&k)
            .map(|job| (job.value.clone(), job.sequence))
        else {
            return;
        };
        wasm_bindgen_futures::spawn_local(async move {
            match api::mail_draft(&value.host_id, &value.mailbox_id, &value.client_id).await {
                Ok(remote) => {
                    {
                        let mut jobs = data.borrow_mut();
                        let Some(job) = jobs.get_mut(&k) else {
                            return;
                        };
                        if job.sequence != sequence
                            || job.phase != Phase::Ready
                            || job.running
                            || job.value.dirty
                            || job.value.pending_send.is_some()
                            || job.review.is_some()
                        {
                            return;
                        }
                        if remote.revision == job.value.server_revision
                            && remote
                                .latest_send
                                .as_ref()
                                .map(|s| (&s.operation_id, s.state))
                                == job
                                    .value
                                    .latest_send
                                    .as_ref()
                                    .map(|s| (&s.operation_id, s.state))
                        {
                            return;
                        }
                        let mut fresh =
                            LocalDraft::from_server(value.scope(), value.provider, remote);
                        // Preserve device Blob keys for unchanged server uploads.
                        for file in &mut fresh.files {
                            if let Some(old) = job.value.files.iter().find(|old| {
                                old.server
                                    .as_ref()
                                    .is_some_and(|server| server.id == file.id)
                            }) {
                                file.id = old.id.clone();
                            }
                        }
                        job.value = fresh;
                        job.sequence = job.sequence.wrapping_add(1);
                    }
                    self.publish(&k, &data);
                    self.kick_key(k, data);
                }
                Err(error) => self.fail(&k, &data, error.to_string()),
            }
        });
    }

    pub fn edit(self, change: impl FnOnce(&mut MailComposeContent)) {
        let Some(value) = self.active.get_untracked() else {
            return;
        };
        let k = key(&value);
        let data = self.data();
        {
            let mut jobs = data.borrow_mut();
            let Some(job) = jobs.get_mut(&k) else { return };
            if matches!(
                job.phase,
                Phase::Review | Phase::Send | Phase::Status | Phase::Reconcile
            ) || job.review.is_some()
                || job.value.pending_send.is_some()
            {
                return;
            }
            change(&mut job.value.content);
            job.value.dirty = true;
            job.value.updated_at_ms = job.value.updated_at_ms.max(crate::clock::now_ms());
            job.sequence = job.sequence.wrapping_add(1);
            job.review = None;
            job.error = None;
        }
        self.publish(&k, &data);
        self.kick_key(k, data);
    }

    pub fn add_files(self, files: Vec<web_sys::File>) {
        let Some(value) = self.active.get_untracked() else {
            return;
        };
        let k = key(&value);
        let data = self.data();
        {
            let mut jobs = data.borrow_mut();
            let Some(job) = jobs.get_mut(&k) else { return };
            if matches!(
                job.phase,
                Phase::Review | Phase::Send | Phase::Upload | Phase::Status | Phase::Reconcile
            ) || job.review.is_some()
                || job.value.pending_send.is_some()
            {
                return;
            }
            let mut total = job
                .value
                .files
                .iter()
                .map(|file| file.size_bytes)
                .sum::<u64>();
            if job.value.files.len().saturating_add(files.len()) > MAIL_DRAFT_MAX_ATTACHMENTS {
                job.error = Some("Keep this draft within 20 files.".into());
                drop(jobs);
                self.publish(&k, &data);
                return;
            }
            let mut added = Vec::new();
            for file in files {
                let size = file.size();
                if !size.is_finite() || size < 0.0 || size.fract() != 0.0 {
                    job.error = Some("This file's size could not be read.".into());
                    drop(jobs);
                    self.publish(&k, &data);
                    return;
                }
                let size = size.to_u64_saturating();
                total = total.saturating_add(size);
                if total > u64::try_from(MAIL_DRAFT_ATTACHMENT_MAX_BYTES).unwrap_or(u64::MAX) {
                    job.error = Some("Keep draft files within 16 MiB combined.".into());
                    drop(jobs);
                    self.publish(&k, &data);
                    return;
                }
                let id = uuid::Uuid::new_v4().to_string();
                let name = attachment_file_name(&file.name());
                added.push((
                    LocalFile {
                        id: id.clone(),
                        file_name: name,
                        size_bytes: size,
                        server: None,
                    },
                    id,
                    file.unchecked_into::<web_sys::Blob>(),
                ));
            }
            for (file, id, blob) in added {
                job.value.files.push(file);
                job.blobs.insert(id, blob);
            }
            job.value.dirty = true;
            job.value.updated_at_ms = crate::clock::now_ms();
            job.sequence = job.sequence.wrapping_add(1);
            job.review = None;
            job.error = None;
        }
        self.publish(&k, &data);
        self.kick_key(k, data);
    }

    pub fn remove_file(self, id: &str) {
        let Some(value) = self.active.get_untracked() else {
            return;
        };
        let k = key(&value);
        let data = self.data();
        {
            let mut jobs = data.borrow_mut();
            let Some(job) = jobs.get_mut(&k) else { return };
            if matches!(
                job.phase,
                Phase::Review | Phase::Send | Phase::Upload | Phase::Status | Phase::Reconcile
            ) || job.review.is_some()
                || job.value.pending_send.is_some()
            {
                return;
            }
            if let Some(position) = job.value.files.iter().position(|file| file.id == id) {
                let file = job.value.files.remove(position);
                job.blobs.remove(id);
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
            job.value.updated_at_ms = crate::clock::now_ms();
            job.sequence = job.sequence.wrapping_add(1);
            job.review = None;
            job.error = None;
        }
        self.publish(&k, &data);
        self.kick_key(k, data);
    }

    pub fn retry(self) {
        self.kick();
    }
    fn kick(self) {
        if let Some(value) = self.active.get_untracked() {
            self.kick_key(key(&value), self.data());
        }
    }
    fn kick_key(self, k: String, data: Rc<RefCell<HashMap<String, Job>>>) {
        {
            let mut jobs = data.borrow_mut();
            let Some(job) = jobs.get_mut(&k) else { return };
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
        }
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                crate::storage::metadata::yield_before_save().await;
                let (value, revision, sequence, blobs) = {
                    let mut jobs = data.borrow_mut();
                    let Some(job) = jobs.get_mut(&k) else { return };
                    job.phase = Phase::Local;
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
                let saved = match local::save(&value, revision, &blobs).await {
                    Ok(record) => record,
                    Err(error) => {
                        self.fail(&k, &data, error.to_string());
                        return;
                    }
                };
                {
                    let mut jobs = data.borrow_mut();
                    let Some(job) = jobs.get_mut(&k) else { return };
                    job.local_revision = Some(saved.revision);
                    job.saved_sequence = sequence;
                    for (id, _) in &blobs {
                        job.blobs.remove(id);
                    }
                    if job.sequence != sequence {
                        continue;
                    }
                }
                let result: Result<bool, String> = async {
                    // Acknowledge recovery on this device without resuming a
                    // server writer while the earlier send remains unresolved.
                    if value.pending_send.is_some() {
                        return Ok(false);
                    }
                    if value.server_revision == 0 {
                        self.phase(&k, &data, Phase::Server);
                        let draft = api::mail_draft_create(
                            &value.mailbox_id,
                            &MailDraftCreate {
                                host_id: value.host_id.clone(),
                                client_id: value.client_id.clone(),
                                reply: value.reply.clone(),
                            },
                        )
                        .await
                        .map_err(|error| error.to_string())?;
                        let mut jobs = data.borrow_mut();
                        let job = jobs.get_mut(&k).ok_or("Draft closed")?;
                        job.value.server_revision = draft.revision;
                        job.value.latest_send = draft.latest_send;
                        job.sequence = job.sequence.wrapping_add(1);
                        return Ok(true);
                    }
                    if value.dirty
                        && (!value.pending_remove.is_empty()
                            || value.files.iter().all(|file| file.server.is_some()))
                    {
                        self.phase(&k, &data, Phase::Server);
                        let draft = save_attempt(&value).await?;
                        let mut jobs = data.borrow_mut();
                        let job = jobs.get_mut(&k).ok_or("Draft closed")?;
                        job.value.server_revision = draft.revision;
                        job.value.latest_send = draft.latest_send;
                        if job.sequence == sequence {
                            job.value.dirty = false;
                        }
                        job.sequence = job.sequence.wrapping_add(1);
                        return Ok(true);
                    }
                    if let Some(id) = value.pending_remove.first() {
                        self.phase(&k, &data, Phase::Server);
                        api::mail_draft_remove_upload(
                            &value.host_id,
                            &value.mailbox_id,
                            &value.client_id,
                            id,
                        )
                        .await
                        .map_err(|error| error.to_string())?;
                        let mut jobs = data.borrow_mut();
                        let job = jobs.get_mut(&k).ok_or("Draft closed")?;
                        job.value.pending_remove.retain(|item| item != id);
                        job.sequence = job.sequence.wrapping_add(1);
                        return Ok(true);
                    }
                    if let Some(file) = value.files.iter().find(|file| file.server.is_none()) {
                        self.phase(&k, &data, Phase::Upload);
                        let blob = local::file(&value.scope(), &value.client_id, &file.id)
                            .await
                            .map_err(|error| error.to_string())?
                            .ok_or(
                                "The saved file is unavailable. Choose it again before reviewing.",
                            )?;
                        let uploaded = api::mail_draft_upload(
                            &value.host_id,
                            &value.mailbox_id,
                            &value.client_id,
                            &file.id,
                            &file.file_name,
                            &blob,
                        )
                        .await
                        .map_err(|error| error.to_string())?;
                        if uploaded.size_bytes != file.size_bytes {
                            return Err("The uploaded file did not match its selected size.".into());
                        }
                        let mut jobs = data.borrow_mut();
                        let job = jobs.get_mut(&k).ok_or("Draft closed")?;
                        if let Some(file) =
                            job.value.files.iter_mut().find(|item| item.id == file.id)
                        {
                            file.server = Some(uploaded);
                        } else {
                            job.value.pending_remove.push(uploaded.id);
                        }
                        job.value.content.attachments = job
                            .value
                            .files
                            .iter()
                            .filter_map(|file| file.server.as_ref().map(|file| file.id.clone()))
                            .collect();
                        job.value.dirty = true;
                        job.sequence = job.sequence.wrapping_add(1);
                        return Ok(true);
                    }
                    Ok(false)
                }
                .await;
                match result {
                    Ok(true) => continue,
                    Ok(false) => {
                        let mut jobs = data.borrow_mut();
                        if let Some(job) = jobs.get_mut(&k) {
                            job.running = false;
                            job.phase = Phase::Ready;
                        }
                        drop(jobs);
                        self.publish(&k, &data);
                        return;
                    }
                    Err(error) => {
                        self.fail(&k, &data, error);
                        return;
                    }
                }
            }
        });
    }

    fn fail(self, k: &str, data: &Rc<RefCell<HashMap<String, Job>>>, error: String) {
        if let Some(job) = data.borrow_mut().get_mut(k) {
            job.running = false;
            job.phase = Phase::Error;
            job.error = Some(error);
        }
        self.publish(k, data);
    }
    fn phase(self, k: &str, data: &Rc<RefCell<HashMap<String, Job>>>, phase: Phase) {
        if let Some(job) = data.borrow_mut().get_mut(k) {
            job.phase = phase;
        }
        self.publish(k, data);
    }

    pub fn prepare_review(self) {
        let Some(value) = self.active.get_untracked() else {
            return;
        };
        let k = key(&value);
        let data = self.data();
        {
            let mut jobs = data.borrow_mut();
            let Some(job) = jobs.get_mut(&k) else { return };
            if job.running
                || matches!(
                    job.phase,
                    Phase::Review | Phase::Send | Phase::Status | Phase::Reconcile
                )
                || job.review.is_some()
                || job.value.dirty
                || job.value.files.iter().any(|file| file.server.is_none())
                || job.value.pending_send.is_some()
            {
                return;
            }
            if let Err(error) = job.value.content.validate_send() {
                job.error = Some(error.to_string());
                drop(jobs);
                self.publish(&k, &data);
                return;
            }
            job.phase = Phase::Review;
            job.error = None;
        }
        self.publish(&k, &data);
        wasm_bindgen_futures::spawn_local(async move {
            let result = async {
                let connection = crate::api::mail_connection(value.provider).await?;
                let id = connection
                    .id
                    .filter(|_| connection.state == ConnectionState::Connected)
                    .ok_or(crate::api::ApiError::Http {
                        status: 403,
                        detail: Some(
                            "Reconnect the account that owns this draft before reviewing.".into(),
                        ),
                    })?;
                api::mail_draft_review(
                    &value.mailbox_id,
                    &value.client_id,
                    &MailReviewRequest {
                        host_id: value.host_id.clone(),
                        connection_id: id,
                        expected_revision: value.server_revision,
                    },
                )
                .await
            }
            .await;
            match result {
                Ok(review) => {
                    let record = {
                        let mut jobs = data.borrow_mut();
                        let Some(job) = jobs.get_mut(&k) else { return };
                        job.value.server_revision = review.revision;
                        job.review = Some(review);
                        job.phase = Phase::Ready;
                        job.sequence = job.sequence.wrapping_add(1);
                        job.value.clone()
                    };
                    self.publish(&k, &data);
                    self.kick_key(k, data);
                    let _ = record;
                }
                Err(error) => self.fail(&k, &data, error.to_string()),
            }
        });
    }

    pub fn edit_review(self) {
        let Some(value) = self.active.get_untracked() else {
            return;
        };
        let k = key(&value);
        let data = self.data();
        if let Some(job) = data.borrow_mut().get_mut(&k) {
            if matches!(
                job.phase,
                Phase::Review | Phase::Send | Phase::Status | Phase::Reconcile
            ) || job.running
                || job.value.pending_send.is_some()
            {
                return;
            }
            job.review = None;
            job.phase = Phase::Ready;
        }
        self.publish(&k, &data);
    }

    pub fn send(self) {
        let Some(value) = self.active.get_untracked() else {
            return;
        };
        let k = key(&value);
        let data = self.data();
        let request = {
            let mut jobs = data.borrow_mut();
            let Some(job) = jobs.get_mut(&k) else { return };
            let Some(review) = job.review.as_ref() else {
                return;
            };
            if job.running || job.phase == Phase::Send || job.value.pending_send.is_some() {
                return;
            }
            let request = MailSendRequest {
                host_id: review.host_id.clone(),
                connection_id: review.connection_id.clone(),
                operation_id: uuid::Uuid::new_v4().to_string(),
                revision: review.revision,
                digest: review.digest.clone(),
            };
            job.value.pending_send = Some(request.clone());
            job.phase = Phase::Send;
            job.error = None;
            job.sequence = job.sequence.wrapping_add(1);
            request
        };
        self.publish(&k, &data);
        wasm_bindgen_futures::spawn_local(async move {
            let (snapshot, revision) = {
                let jobs = data.borrow();
                let Some(job) = jobs.get(&k) else { return };
                (job.value.clone(), job.local_revision)
            };
            match local::save(&snapshot, revision, &[]).await {
                Ok(record) => {
                    if let Some(job) = data.borrow_mut().get_mut(&k) {
                        job.local_revision = Some(record.revision);
                        job.saved_sequence = job.sequence;
                    }
                }
                Err(error) => {
                    if let Some(job) = data.borrow_mut().get_mut(&k) {
                        if job
                            .value
                            .pending_send
                            .as_ref()
                            .is_some_and(|pending| pending.operation_id == request.operation_id)
                        {
                            job.value.pending_send = None;
                            job.sequence = job.sequence.wrapping_add(1);
                        }
                    }
                    self.fail(&k, &data, error.to_string());
                    return;
                }
            }
            let result = api::mail_draft_send(&value.mailbox_id, &value.client_id, &request).await;
            match result {
                Ok(status) => self.finish_send(&k, &data, status),
                Err(error) => {
                    self.fail(&k, &data, error.to_string());
                    self.check_send_key(k, data);
                }
            }
        });
    }

    fn finish_send(
        self,
        k: &str,
        data: &Rc<RefCell<HashMap<String, Job>>>,
        status: MailSendStatus,
    ) {
        {
            let mut jobs = data.borrow_mut();
            let Some(job) = jobs.get_mut(k) else { return };
            if let Some(previous) = &job.value.latest_send {
                if previous.operation_id == status.operation_id
                    && previous.state == MailDispatchState::Accepted
                    && status.state != MailDispatchState::Accepted
                {
                    return;
                }
            }
            if job
                .value
                .pending_send
                .as_ref()
                .is_some_and(|request| request.operation_id != status.operation_id)
            {
                return;
            }
            job.value.latest_send = Some(status.clone());
            job.phase = Phase::Ready;
            job.review = None;
            if !matches!(
                status.state,
                MailDispatchState::Queued
                    | MailDispatchState::Dispatching
                    | MailDispatchState::Uncertain
            ) {
                job.value.pending_send = None;
            }
            job.error = status.problem.as_ref().map(ToString::to_string);
            job.sequence = job.sequence.wrapping_add(1);
        }
        self.publish(k, data);
        self.kick_key(k.to_owned(), Rc::clone(data));
    }

    pub fn check_send(self) {
        if let Some(value) = self.active.get_untracked() {
            self.check_send_key(key(&value), self.data());
        }
    }
    fn check_send_key(self, k: String, data: Rc<RefCell<HashMap<String, Job>>>) {
        let value = {
            let mut jobs = data.borrow_mut();
            let Some(job) = jobs.get_mut(&k) else {
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
            if job.value.pending_send.is_none() {
                return;
            }
            job.phase = Phase::Status;
            job.error = None;
            job.value.clone()
        };
        let Some(request) = value.pending_send.clone() else {
            return;
        };
        self.publish(&k, &data);
        wasm_bindgen_futures::spawn_local(async move {
            match api::mail_send_status(&value.host_id, &value.mailbox_id, &request.operation_id)
                .await
            {
                Ok(status) => self.finish_send(&k, &data, status),
                Err(error) => self.fail(&k, &data, error.to_string()),
            }
        });
    }

    pub fn show_drafts(self, scope: Scope, provider: ServiceProvider, page: u64) {
        self.list_visible.set(true);
        self.visible.set(false);
        self.release_current();
        self.local_rows.set(Vec::new());
        self.server_rows.set(None);
        self.list_scope.set(Some(scope.clone()));
        self.list_provider.set(Some(provider));
        self.error.set(None);
        self.loading.set(true);
        self.generation
            .update(|value| *value = value.wrapping_add(1));
        let generation = self.generation.get_untracked();
        wasm_bindgen_futures::spawn_local(async move {
            let (device, server) = futures_util::future::join(
                local::list(&scope, None),
                api::mail_drafts(&scope.host_id, &scope.mailbox_id, page, None),
            )
            .await;
            if !self.live() || self.generation.get_untracked() != generation {
                return;
            }
            self.loading.set(false);
            match device {
                Ok(mut rows) => {
                    self.local_more.set(rows.len() == 50);
                    // A failed device transaction still leaves the latest input in
                    // the app-owned writer. Keep it reachable from Drafts.
                    let data = self.data();
                    for job in data
                        .borrow()
                        .values()
                        .filter(|job| job.value.scope().same(&scope) && job.value.listed())
                    {
                        if let Ok(row) = job.value.row() {
                            rows.retain(|old| old.client_id != row.client_id);
                            rows.push(row);
                        }
                    }
                    rows.sort_by(|a, b| {
                        b.updated_at_ms
                            .cmp(&a.updated_at_ms)
                            .then_with(|| a.client_id.cmp(&b.client_id))
                    });
                    self.local_rows.set(rows);
                }
                Err(error) => self.error.set(Some(error.to_string())),
            }
            match server {
                Ok(rows) => self.server_rows.set(Some(rows)),
                Err(error) => self.error.set(Some(error.to_string())),
            }
        });
    }

    pub fn open_listed(self, client: String) {
        if let (Some(scope), Some(provider)) = (
            self.list_scope.get_untracked(),
            self.list_provider.get_untracked(),
        ) {
            self.open(scope, provider, client);
        }
    }

    pub fn older_local(self) {
        if self.loading.get_untracked() {
            return;
        }
        let Some(scope) = self.list_scope.get_untracked() else {
            return;
        };
        let after = self
            .local_rows
            .with_untracked(|rows| rows.iter().map(|row| row.key.clone()).max());
        let generation = self.generation.get_untracked();
        self.loading.set(true);
        wasm_bindgen_futures::spawn_local(async move {
            let result = local::list(&scope, after.as_deref()).await;
            if !self.live() || self.generation.get_untracked() != generation {
                return;
            }
            self.loading.set(false);
            match result {
                Ok(rows) => {
                    self.local_more.set(rows.len() == 50);
                    self.local_rows.update(|current| {
                        for row in rows {
                            if !current.iter().any(|old| old.client_id == row.client_id) {
                                current.push(row);
                            }
                        }
                    });
                }
                Err(error) => self.error.set(Some(error.to_string())),
            }
        });
    }

    pub fn server_page(self, page: u64) {
        if self.loading.get_untracked() {
            return;
        }
        let Some(scope) = self.list_scope.get_untracked() else {
            return;
        };
        let revision = self
            .server_rows
            .with_untracked(|page| page.as_ref().map(|page| page.revision));
        let generation = self.generation.get_untracked();
        self.loading.set(true);
        wasm_bindgen_futures::spawn_local(async move {
            let result = api::mail_drafts(&scope.host_id, &scope.mailbox_id, page, revision).await;
            if !self.live() || self.generation.get_untracked() != generation {
                return;
            }
            self.loading.set(false);
            match result {
                Ok(rows) => self.server_rows.set(Some(rows)),
                Err(error) => self.error.set(Some(error.to_string())),
            }
        });
    }

    pub fn cancel_send(self) {
        let Some(value) = self.active.get_untracked() else {
            return;
        };
        let Some(request) = value.pending_send.clone() else {
            return;
        };
        let k = key(&value);
        let data = self.data();
        {
            let mut jobs = data.borrow_mut();
            let Some(job) = jobs.get_mut(&k) else {
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
            if job
                .value
                .latest_send
                .as_ref()
                .is_none_or(|status| status.state != MailDispatchState::Queued)
            {
                return;
            }
            job.phase = Phase::Status;
        }
        self.publish(&k, &data);
        wasm_bindgen_futures::spawn_local(async move {
            match api::mail_send_cancel(&value.host_id, &value.mailbox_id, &request.operation_id)
                .await
            {
                Ok(status) => self.finish_send(&k, &data, status),
                Err(error) => self.fail(&k, &data, error.to_string()),
            }
        });
    }

    pub fn close(self) {
        self.visible.set(false);
        self.list_visible.set(false);
        self.release_current();
        self.generation
            .update(|value| *value = value.wrapping_add(1));
        queue_microtask(move || {
            let selector = "[data-mail-reader] button, [data-mail-compose-entry] button";
            if let Ok(Some(element)) = document().query_selector(selector) {
                if let Ok(element) = element.dyn_into::<web_sys::HtmlElement>() {
                    let _ = element.focus();
                }
            }
        });
    }
    pub fn pending(self) -> bool {
        self.unsaved.get()
    }
}

// A failed reply does not establish whether the PUT committed. Rebase only
// across the exact attempted content, never across somebody else's edit.
async fn save_attempt(value: &LocalDraft) -> Result<MailComposeDraft, String> {
    let result = api::mail_draft_save(
        &value.mailbox_id,
        &value.client_id,
        &MailDraftSave {
            host_id: value.host_id.clone(),
            expected_revision: value.server_revision,
            content: value.content.clone(),
        },
    )
    .await;
    match result {
        Ok(saved) => Ok(saved),
        Err(error) => {
            match api::mail_draft(&value.host_id, &value.mailbox_id, &value.client_id).await {
                Ok(saved)
                    if saved.host_id == value.host_id
                        && saved.mailbox_id == value.mailbox_id
                        && saved.client_id == value.client_id
                        && saved.reply == value.reply
                        && saved.revision > value.server_revision
                        && saved.content == value.content =>
                {
                    Ok(saved)
                }
                _ => Err(error.to_string()),
            }
        }
    }
}
