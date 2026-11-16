//! Workspace-owned native email editing. Writers and send receipts outlive the
//! visible Inbox route; reopening a pending operation reads status only.
use super::*;
use bruhs_core::mail::device::{
    DeviceDraft, DeviceDraftFile, DeviceDraftRecord, DeviceDraftRow, DeviceDraftScope,
    DeviceFileBytes, MailDeviceStore,
};
use bruhs_models::{
    connections::{ConnectionState, ServiceProvider},
    mail::*,
};
use std::{collections::HashMap, future::Future};

mod render;

mod sent;
#[cfg(test)]
mod tests;
mod work;

pub(super) enum ComposeEvent {
    Close,
}
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
    value: DeviceDraft,
    revision: Option<i64>,
    sequence: u64,
    saved: u64,
    running: bool,
    selecting_files: bool,
    phase: Phase,
    added: HashMap<String, Vec<u8>>,
    review: Option<MailSendReview>,
    error: Option<String>,
}
impl Job {
    fn locked(&self) -> bool {
        self.selecting_files
            || self.value.pending_send.is_some()
            || self.value.consumed()
            || self.review.is_some()
            || matches!(
                self.phase,
                Phase::Review | Phase::Send | Phase::Status | Phase::Reconcile
            )
    }
}
#[derive(Clone)]
enum Action {
    Close,
    Retry,
    Review,
    EditReview,
    Send,
    Status,
    CheckSent,
    Cancel,
    Attach,
    Remove(String),
    Open(String),
    Page(u64),
}

pub(super) struct MailComposer {
    client: ServerClient,
    store: Option<MailDeviceStore>,
    store_cell: Arc<tokio::sync::OnceCell<MailDeviceStore>>,
    quitting: bool,
    opening: bool,
    store_error: Option<String>,
    jobs: HashMap<String, Job>,
    active: Option<String>,
    inputs: Vec<Entity<InputState>>,
    _subscriptions: Vec<Subscription>,
    listing: bool,
    list_scope: Option<(DeviceDraftScope, ServiceProvider)>,
    list_page: u64,
    local_rows: Vec<DeviceDraftRow>,
    server_rows: Option<MailDraftsPage>,
    loading: bool,
    generation: u64,
    feedback_focus: FocusHandle,
    surface_focus: FocusHandle,
    focus_review: bool,
    focus_sent_feedback: Option<(String, u64)>,
    feedback_scroll: ScrollHandle,
}
impl EventEmitter<ComposeEvent> for MailComposer {}
fn key(scope: &DeviceDraftScope, client: &str) -> String {
    serde_json::to_string(&(scope, client)).unwrap_or_default()
}
fn operation<T: Send + 'static>(
    cx: &App,
    future: impl Future<Output = Result<T, String>> + Send + 'static,
) -> Task<Result<T, String>> {
    // Dropping a view's waiter detaches the durable Tokio work. Provider sends
    // have their own server-owned worker and are never automatically replayed.
    let worker = cx.global::<DesktopState>().runtime.spawn(future);
    cx.background_spawn(async move {
        worker.await.map_err(|_| {
            "The email worker stopped. Keep this editor open and check saved status.".to_string()
        })?
    })
}
fn server_error(error: crate::server_client::ServerError) -> String {
    use crate::server_client::ServerError;
    match error {
        ServerError::Status{code,..}=>match code.as_u16(){
            401|403=>"Reconnect the owning email account in Connections, then review again.",
            409=>"The draft, connection or mail server changed. Your current edits remain on this device. Open the saved draft in another view before replacing them.",
            413=>"This email exceeds the provider limit. Remove files or shorten it before reviewing.",
            429=>"The provider is limiting requests. Wait before reviewing again.",
            404=>"The saved draft or send status is unavailable. Check Sent mail before starting another send.",
            400=>"Check the recipients, subject and selected files before reviewing again.",
            _=>"The mail server could not finish this request. Keep the draft and check send status before retrying.",
        }.into(),
        _=>"The mail server could not be reached or read. Your edits remain here. Check send status before starting another send.".into(),
    }
}
impl MailComposer {
    pub(super) fn new(
        client: ServerClient,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let labels = ["To", "Cc", "Bcc", "Subject", "Message"];
        let inputs = labels
            .into_iter()
            .enumerate()
            .map(|(i, label)| {
                cx.new(|cx| {
                    let input = InputState::new(window, cx).placeholder(label);
                    if i == 4 {
                        input.multi_line(true).rows(8)
                    } else {
                        input
                    }
                })
            })
            .collect::<Vec<_>>();
        let mut subscriptions: Vec<Subscription> = inputs
            .iter()
            .map(|input| {
                cx.subscribe_in(input, window, |this, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.edited(cx);
                    }
                })
            })
            .collect();
        subscriptions.push(cx.on_app_quit(|this, cx| this.flush_quit(cx)));
        Self {
            client,
            store: None,
            store_cell: Arc::new(tokio::sync::OnceCell::new()),
            quitting: false,
            opening: false,
            store_error: None,
            jobs: HashMap::new(),
            active: None,
            inputs,
            _subscriptions: subscriptions,
            listing: false,
            list_scope: None,
            list_page: 0,
            local_rows: Vec::new(),
            server_rows: None,
            loading: false,
            generation: 0,
            feedback_focus: cx.focus_handle(),
            surface_focus: cx.focus_handle(),
            focus_review: false,
            focus_sent_feedback: None,
            feedback_scroll: ScrollHandle::new(),
        }
    }
    fn flush_quit(&mut self, cx: &mut Context<'_, Self>) -> Task<()> {
        self.quitting = true;
        let values = self
            .jobs
            .values()
            .filter(|job| job.sequence != job.saved || !job.added.is_empty() || job.selecting_files)
            .map(|job| {
                (
                    job.value.clone(),
                    job.added
                        .iter()
                        .map(|(id, bytes)| DeviceFileBytes {
                            id: id.clone(),
                            bytes: bytes.clone(),
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        let store = self.store.clone();
        let cell = Arc::clone(&self.store_cell);
        let path = crate::paths::desktop_state_file().and_then(|path| {
            path.parent()
                .map(|dir| dir.join("mail-device/drafts.sqlite3"))
        });
        let work = operation(cx, async move {
            if values.is_empty() {
                return Ok(());
            }
            let store = match store {
                Some(store) => store,
                None => {
                    let path = path.ok_or_else(|| "Device draft folder unavailable".to_string())?;
                    cell.get_or_try_init(|| MailDeviceStore::open(path))
                        .await
                        .cloned()
                        .map_err(|e| e.to_string())?
                }
            };
            for (value, added) in values {
                store
                    .save_on_shutdown(value, added)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            Ok(())
        });
        cx.background_spawn(async move {
            if work.await.is_err() {
                tracing::error!("The final device email save did not complete before shutdown");
            }
        })
    }

    fn active(&self) -> Option<&Job> {
        self.active.as_ref().and_then(|key| self.jobs.get(key))
    }
    pub(super) fn pending(&self) -> bool {
        self.jobs
            .values()
            .any(|job| job.sequence != job.saved || !job.added.is_empty() || job.selecting_files)
    }
    fn ensure_store(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.store.is_some() || self.opening {
            return;
        }
        let Some(path) = crate::paths::desktop_state_file().and_then(|path| {
            path.parent()
                .map(|dir| dir.join("mail-device/drafts.sqlite3"))
        }) else {
            self.store_error = Some(
                "The device email folder is unavailable. Keep the editor open and retry.".into(),
            );
            return;
        };
        self.opening = true;
        self.store_error = None;
        let cell = Arc::clone(&self.store_cell);
        let work = operation(cx, async move {
            cell.get_or_try_init(|| MailDeviceStore::open(path))
                .await
                .cloned()
                .map_err(|e| e.to_string())
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.opening = false;
                match result {
                    Ok(store) => {
                        this.store = Some(store);
                        let keys = this.jobs.keys().cloned().collect::<Vec<_>>();
                        for key in keys {
                            this.kick(key, cx);
                        }
                        if this.listing {
                            this.load_list(window, cx);
                        }
                    }
                    Err(error) => this.store_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn hydrate(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(job) = self.active() else {
            return;
        };
        let values = [
            &job.value.content.to,
            &job.value.content.cc,
            &job.value.content.bcc,
            &job.value.content.subject,
            &job.value.content.body,
        ];
        for (input, value) in self.inputs.iter().zip(values) {
            input.update(cx, |input, cx| input.set_value(value.clone(), window, cx));
        }
        self.inputs[4].update(cx, |input, cx| input.focus(window, cx));
    }
    fn release_current(&mut self) {
        self.active = None;
        self.jobs.retain(|_, job| {
            job.value.dirty
                || job.selecting_files
                || job.running
                || matches!(
                    job.phase,
                    Phase::Review | Phase::Send | Phase::Status | Phase::Reconcile
                )
                || job.sequence != job.saved
                || !job.added.is_empty()
                || job.value.pending_send.is_some()
        });
    }
    pub(super) fn start(
        &mut self,
        scope: DeviceDraftScope,
        provider: ServiceProvider,
        reply: Option<MailReplyTarget>,
        content: MailComposeContent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.generation = self.generation.wrapping_add(1);
        self.listing = false;
        self.loading = false;
        self.release_current();
        let value = DeviceDraft::new(scope, provider, reply, content);
        let k = key(&value.scope, &value.client_id);
        self.jobs.insert(
            k.clone(),
            Job {
                value,
                revision: None,
                sequence: 0,
                saved: u64::MAX,
                running: false,
                selecting_files: false,
                phase: Phase::Ready,
                added: HashMap::new(),
                review: None,
                error: None,
            },
        );
        self.active = Some(k.clone());
        self.hydrate(window, cx);
        self.ensure_store(window, cx);
        self.kick(k, cx);
        cx.notify();
    }
    fn edited(&mut self, cx: &mut Context<'_, Self>) {
        let Some(k) = self.active.clone() else {
            return;
        };
        let Some(job) = self.jobs.get_mut(&k) else {
            return;
        };
        if job.locked() {
            return;
        }
        let mut content = job.value.content.clone();
        let values = self
            .inputs
            .iter()
            .map(|input| input.read(cx).value().to_string())
            .collect::<Vec<_>>();
        content.to = values[0].clone();
        content.cc = values[1].clone();
        content.bcc = values[2].clone();
        content.subject = values[3].clone();
        content.body = values[4].clone();
        if content == job.value.content {
            return;
        }
        job.value.content = content;
        job.value.dirty = true;
        job.value.updated_at_ms = EpochMs::now().as_millis();
        job.sequence = job.sequence.wrapping_add(1);
        job.error = None;
        self.kick(k, cx);
        cx.notify();
    }
    pub(super) fn show_drafts(
        &mut self,
        scope: DeviceDraftScope,
        provider: ServiceProvider,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.generation = self.generation.wrapping_add(1);
        self.list_scope = Some((scope, provider));
        self.list_page = 0;
        self.listing = true;
        self.release_current();
        self.local_rows.clear();
        self.server_rows = None;
        self.ensure_store(window, cx);
        self.load_list(window, cx);
        cx.notify();
    }
    fn load_list(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let (Some(store), Some((scope, _))) = (self.store.clone(), self.list_scope.clone()) else {
            return;
        };
        let client = self.client.clone();
        let page = self.list_page;
        let generation = self.generation;
        self.loading = true;
        let expected = self.server_rows.as_ref().map(|page| page.revision);
        let work = operation(cx, async move {
            let local = store
                .list(scope.clone(), page)
                .await
                .map_err(|e| e.to_string());
            let server = client
                .mail_drafts(&scope.host_id, &scope.mailbox_id, page, expected)
                .await
                .map_err(server_error);
            Ok((local, server))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, _, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match result {
                    Ok((local, server)) => {
                        match local {
                            Ok(rows) => this.local_rows = rows,
                            Err(error) => this.store_error = Some(error),
                        };
                        match server {
                            Ok(rows) => this.server_rows = Some(rows),
                            Err(error) => this.store_error = Some(error),
                        };
                    }
                    Err(error) => this.store_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn open(&mut self, id: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some((scope, provider)) = self.list_scope.clone() else {
            return;
        };
        let k = key(&scope, &id);
        if self.jobs.contains_key(&k) {
            self.active = Some(k);
            self.listing = false;
            self.hydrate(window, cx);
            cx.notify();
            return;
        }
        let Some(store) = self.store.clone() else {
            return;
        };
        let client = self.client.clone();
        self.loading = true;
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let work = operation(cx, async move {
            let local = store
                .load(scope.clone(), id.clone())
                .await
                .map_err(|e| e.to_string())?;
            if let Some(record) = local {
                if record.value.dirty || record.value.pending_send.is_some() {
                    return Ok(record);
                }
                match client
                    .mail_draft(&scope.host_id, &scope.mailbox_id, &id)
                    .await
                {
                    Ok(remote) => Ok(DeviceDraftRecord {
                        revision: record.revision,
                        value: record
                            .value
                            .refresh_from_server(remote)
                            .map_err(|e| e.to_string())?,
                    }),
                    Err(_) => Ok(record),
                }
            } else {
                let remote = client
                    .mail_draft(&scope.host_id, &scope.mailbox_id, &id)
                    .await
                    .map_err(server_error)?;
                Ok(DeviceDraftRecord {
                    revision: 0,
                    value: DeviceDraft::from_server(scope, provider, remote)
                        .map_err(|e| e.to_string())?,
                })
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(record) => {
                        let pending = record.value.pending_send.is_some();
                        let k = key(&record.value.scope, &record.value.client_id);
                        this.jobs.insert(
                            k.clone(),
                            Job {
                                value: record.value,
                                revision: (record.revision > 0).then_some(record.revision),
                                sequence: 0,
                                saved: u64::MAX,
                                running: false,
                                selecting_files: false,
                                phase: Phase::Ready,
                                added: HashMap::new(),
                                review: None,
                                error: None,
                            },
                        );
                        this.active = Some(k.clone());
                        this.listing = false;
                        this.hydrate(window, cx);
                        if pending {
                            this.check_status(k, cx);
                        } else {
                            this.kick(k, cx);
                        }
                    }
                    Err(error) => this.store_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn action(&mut self, action: Action, window: &mut Window, cx: &mut Context<'_, Self>) {
        match action {
            Action::Close => {
                self.release_current();
                self.generation = self.generation.wrapping_add(1);
                cx.emit(ComposeEvent::Close);
            }
            Action::Retry => {
                self.ensure_store(window, cx);
                if let Some(k) = self.active.clone() {
                    self.kick(k, cx);
                } else if self.listing {
                    self.load_list(window, cx);
                }
            }
            Action::Review => self.review(cx),
            Action::CheckSent => self.check_sent(cx),
            Action::Send => self.send(cx),
            Action::Status => {
                if let Some(k) = self.active.clone() {
                    self.check_status(k, cx);
                }
            }
            Action::EditReview => {
                if let Some(k) = self.active.clone() {
                    if let Some(job) = self.jobs.get_mut(&k) {
                        if !job.running
                            && !matches!(
                                job.phase,
                                Phase::Review | Phase::Send | Phase::Status | Phase::Reconcile
                            )
                        {
                            job.review = None;
                            job.phase = Phase::Ready;
                        }
                    }
                }
            }
            Action::Cancel => self.cancel(cx),
            Action::Attach => self.attach(window, cx),
            Action::Remove(id) => self.remove(id, cx),
            Action::Open(id) => self.open(id, window, cx),
            Action::Page(page) => {
                if !self.loading {
                    self.list_page = page;
                    self.load_list(window, cx);
                }
            }
        }
        cx.notify();
    }
}
