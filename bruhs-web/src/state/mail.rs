//! App-owned mail navigation and jobs. Page unmounts never dispose these signals
//! or silently cancel a provider page after the user has started sync.
use bruhs_models::{
    connections::{ConnectionState, ServiceProvider},
    mail::*,
    InboxFilter,
};
use leptos::prelude::*;

type ReadAbort = StoredValue<Option<web_sys::AbortController>, LocalStorage>;

fn abort_read(read: ReadAbort) {
    if let Some(abort) = read.try_update_value(Option::take).flatten() {
        abort.abort();
    }
}

fn begin_read(read: ReadAbort) -> Result<web_sys::AbortController, crate::api::ApiError> {
    abort_read(read);
    let abort = web_sys::AbortController::new().map_err(|_| {
        crate::api::ApiError::Network("The browser cannot cancel mail requests.".into())
    })?;
    read.set_value(Some(abort.clone()));
    Ok(abort)
}

#[derive(Clone, Copy)]
pub(crate) struct MailState {
    pub visible: RwSignal<bool>,
    pub snapshot: RwSignal<Option<MailboxesSnapshot>>,
    pub mailbox: RwSignal<Option<String>>,
    pub query: RwSignal<String>,
    applied_query: RwSignal<String>,
    pub filter: RwSignal<InboxFilter>,
    pub page: RwSignal<u64>,
    pub rows: RwSignal<Option<MailThreadsPage>>,
    pub selected_thread: RwSignal<Option<String>>,
    pub thread: RwSignal<Option<ConnectedMailThread>>,
    pub message: RwSignal<usize>,
    pub accounts_loading: RwSignal<bool>,
    pub rows_loading: RwSignal<bool>,
    pub thread_loading: RwSignal<bool>,
    pub syncing: RwSignal<bool>,
    pub stopping: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
    pub sync_error: RwSignal<Option<String>>,
    pub download_error: RwSignal<Option<String>>,
    pub downloading: RwSignal<Option<String>>,
    pub notice: RwSignal<Option<String>>,
    generation: RwSignal<u64>,
    detail_generation: RwSignal<u64>,
    rows_abort: ReadAbort,
    detail_abort: ReadAbort,
    alive: RwSignal<bool>,
}

pub(super) fn provide() {
    let state = MailState {
        visible: RwSignal::new(false),
        snapshot: RwSignal::new(None),
        mailbox: RwSignal::new(None),
        query: RwSignal::new(String::new()),
        applied_query: RwSignal::new(String::new()),
        filter: RwSignal::new(InboxFilter::Inbox),
        page: RwSignal::new(0),
        rows: RwSignal::new(None),
        selected_thread: RwSignal::new(None),
        thread: RwSignal::new(None),
        message: RwSignal::new(0),
        accounts_loading: RwSignal::new(false),
        rows_loading: RwSignal::new(false),
        thread_loading: RwSignal::new(false),
        syncing: RwSignal::new(false),
        stopping: RwSignal::new(false),
        error: RwSignal::new(None),
        sync_error: RwSignal::new(None),
        download_error: RwSignal::new(None),
        downloading: RwSignal::new(None),
        notice: RwSignal::new(None),
        generation: RwSignal::new(0),
        detail_generation: RwSignal::new(0),
        rows_abort: StoredValue::new_local(None),
        detail_abort: StoredValue::new_local(None),
        alive: RwSignal::new(true),
    };
    on_cleanup(move || {
        state.alive.set(false);
        abort_read(state.rows_abort);
        abort_read(state.detail_abort);
    });
    provide_context(state);
}

impl MailState {
    pub fn download(self, attachment: MailAttachment) {
        if self.downloading.with_untracked(Option::is_some) {
            return;
        }
        let (Some(snapshot), Some(mailbox), Some(thread)) = (
            self.snapshot.get_untracked(),
            self.mailbox.get_untracked(),
            self.selected_thread.get_untracked(),
        ) else {
            return;
        };
        let Some(provider) = snapshot
            .mailboxes
            .iter()
            .find(|row| row.mailbox_id == mailbox)
            .map(|row| row.provider)
        else {
            self.download_error.set(Some(
                "Refresh saved accounts before downloading this attachment.".into(),
            ));
            return;
        };
        self.downloading.set(Some(attachment.id.clone()));
        self.download_error.set(None);
        self.notice.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            let result = async {
                let connection = crate::api::mail_connection(provider).await?;
                let connection_id = connection
                    .id
                    .filter(|_| connection.state == ConnectionState::Connected)
                    .ok_or_else(|| crate::api::ApiError::Http {
                        status: 403,
                        detail: Some(format!(
                            "Connect {} before downloading this attachment.",
                            provider.name()
                        )),
                    })?;
                crate::api::mail_attachment(
                    provider,
                    &snapshot.host_id,
                    &mailbox,
                    &thread,
                    &connection_id,
                    &attachment,
                )
                .await
            }
            .await;
            if !self.alive() {
                return;
            }
            self.downloading.set(None);
            match result {
                Ok(blob) => match crate::components::connected_inbox::download_blob(&blob, &attachment_file_name(&attachment.file_name)) {
                    Ok(()) => self.notice.set(Some("Attachment download started. Check your browser's downloads for completion.".into())),
                    Err(message) => self.download_error.set(Some(message.into())),
                },
                Err(crate::api::ApiError::Decode(_)) => self.download_error.set(Some("The attachment could not be read completely. No file download started. Refresh the thread and retry.".into())),
                Err(error) => self.download_error.set(Some(error.to_string())),
            }
        });
    }
    fn alive(self) -> bool {
        self.alive.try_get_untracked() == Some(true)
    }

    pub fn open(self) {
        self.visible.set(true);
    }

    pub fn refresh(self) {
        if !self.alive() || self.accounts_loading.get_untracked() {
            return;
        }
        self.accounts_loading.set(true);
        wasm_bindgen_futures::spawn_local(async move {
            let result = crate::api::mail_snapshot().await;
            if !self.alive() {
                return;
            }
            self.accounts_loading.set(false);
            match result {
                Ok(snapshot) => {
                    let changed_host = self.snapshot.with_untracked(|old| {
                        old.as_ref()
                            .is_some_and(|old| old.host_id != snapshot.host_id)
                    });
                    if changed_host {
                        self.clear_selection();
                        self.mailbox.set(None);
                    }
                    let selected = self.mailbox.get_untracked();
                    if !snapshot
                        .mailboxes
                        .iter()
                        .any(|mailbox| Some(&mailbox.mailbox_id) == selected.as_ref())
                    {
                        self.clear_selection();
                        self.mailbox.set(
                            snapshot
                                .mailboxes
                                .first()
                                .map(|mailbox| mailbox.mailbox_id.clone()),
                        );
                    }
                    self.snapshot.set(Some(snapshot));
                    self.error.set(None);
                    self.load_page();
                    if let Some(thread) = self.selected_thread.get_untracked() {
                        self.open_thread(thread);
                    }
                }
                Err(error) => self.error.set(Some(error.to_string())),
            }
        });
    }

    fn clear_selection(self) {
        self.generation
            .update(|value| *value = value.wrapping_add(1));
        self.detail_generation
            .update(|value| *value = value.wrapping_add(1));
        abort_read(self.rows_abort);
        abort_read(self.detail_abort);
        self.rows.set(None);
        self.thread.set(None);
        self.selected_thread.set(None);
        self.rows_loading.set(false);
        self.thread_loading.set(false);
        self.page.set(0);
    }

    pub fn select_mailbox(self, mailbox: String) {
        if self.syncing.get_untracked() {
            return;
        }
        self.clear_selection();
        self.mailbox.set(Some(mailbox));
        self.load_page();
    }

    pub fn search(self) {
        self.applied_query.set(self.query.get_untracked());
        self.clear_selection();
        self.load_page();
    }

    pub fn select_filter(self, filter: InboxFilter) {
        self.filter.set(filter);
        self.search();
    }

    pub fn select_page(self, page: u64) {
        self.page.set(page);
        self.rows.set(None);
        self.load_page();
    }

    fn load_page(self) {
        let (Some(snapshot), Some(mailbox)) =
            (self.snapshot.get_untracked(), self.mailbox.get_untracked())
        else {
            return;
        };
        let request = MailBrowseQuery {
            host_id: snapshot.host_id,
            query: self.applied_query.get_untracked(),
            filter: self.filter.get_untracked(),
            page: self.page.get_untracked(),
            expected_revision: None,
        };
        self.generation
            .update(|value| *value = value.wrapping_add(1));
        let generation = self.generation.get_untracked();
        let abort = match begin_read(self.rows_abort) {
            Ok(abort) => abort,
            Err(error) => {
                self.rows_loading.set(false);
                self.error.set(Some(error.to_string()));
                return;
            }
        };
        self.rows_loading.set(true);
        self.error.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            let result = crate::api::mail_browse(&mailbox, &request, &abort).await;
            if !self.alive() || self.generation.get_untracked() != generation {
                return;
            }
            self.rows_abort.set_value(None);
            self.rows_loading.set(false);
            match result {
                Ok(page) => {
                    self.page.set(page.page);
                    self.rows.set(Some(page));
                }
                Err(error) => self.error.set(Some(error.to_string())),
            }
        });
    }

    pub fn open_thread(self, external: String) {
        let (Some(snapshot), Some(mailbox)) =
            (self.snapshot.get_untracked(), self.mailbox.get_untracked())
        else {
            return;
        };
        let previous_message = self.thread.with_untracked(|thread| {
            thread
                .as_ref()
                .filter(|thread| thread.connected_mail.external_thread_id == external)
                .and_then(|thread| {
                    thread
                        .conversation
                        .messages
                        .get(self.message.get_untracked())
                })
                .map(|message| message.id.clone())
        });
        self.selected_thread.set(Some(external.clone()));
        self.thread.set(None);
        self.thread_loading.set(true);
        self.error.set(None);
        self.detail_generation
            .update(|value| *value = value.wrapping_add(1));
        let generation = self.detail_generation.get_untracked();
        let abort = match begin_read(self.detail_abort) {
            Ok(abort) => abort,
            Err(error) => {
                self.thread_loading.set(false);
                self.error.set(Some(error.to_string()));
                return;
            }
        };
        wasm_bindgen_futures::spawn_local(async move {
            let result =
                crate::api::mail_thread(&snapshot.host_id, &mailbox, &external, &abort).await;
            if !self.alive() || self.detail_generation.get_untracked() != generation {
                return;
            }
            self.detail_abort.set_value(None);
            self.thread_loading.set(false);
            match result {
                Ok(thread) => {
                    self.message.set(
                        previous_message
                            .and_then(|id| {
                                thread
                                    .conversation
                                    .messages
                                    .iter()
                                    .position(|message| message.id == id)
                            })
                            .unwrap_or_else(|| {
                                thread.conversation.messages.len().saturating_sub(1)
                            }),
                    );
                    self.thread.set(Some(thread));
                }
                Err(error) => self.error.set(Some(error.to_string())),
            }
        });
    }

    pub fn back(self) {
        self.detail_generation
            .update(|value| *value = value.wrapping_add(1));
        abort_read(self.detail_abort);
        self.selected_thread.set(None);
        self.thread.set(None);
        self.thread_loading.set(false);
        self.error.set(None);
    }

    pub fn stop(self) {
        if self.syncing.get_untracked() {
            self.stopping.set(true);
            self.notice
                .set(Some("Stopping after the current page is saved…".into()));
        }
    }

    pub fn sync(self, restart: bool) {
        let selected = self.mailbox.get_untracked();
        let provider = self
            .snapshot
            .with_untracked(|snapshot| {
                snapshot.as_ref().and_then(|snapshot| {
                    snapshot
                        .mailboxes
                        .iter()
                        .find(|row| Some(&row.mailbox_id) == selected.as_ref())
                        .map(|row| row.provider)
                })
            })
            .unwrap_or(ServiceProvider::Gmail);
        self.sync_provider(provider, restart);
    }

    pub fn sync_provider(self, provider: ServiceProvider, restart: bool) {
        if !matches!(provider, ServiceProvider::Gmail | ServiceProvider::Outlook) {
            self.sync_error
                .set(Some("Choose a Gmail or Outlook Mail connection.".into()));
            return;
        }
        if self.syncing.get_untracked() || self.accounts_loading.get_untracked() {
            return;
        }
        let Some(snapshot) = self.snapshot.get_untracked() else {
            self.refresh();
            return;
        };
        let selected = self.mailbox.get_untracked();
        let mailbox = snapshot
            .mailboxes
            .iter()
            .find(|mailbox| {
                Some(&mailbox.mailbox_id) == selected.as_ref() && mailbox.provider == provider
            })
            .map(|mailbox| mailbox.mailbox_id.clone());
        self.syncing.set(true);
        self.stopping.set(false);
        self.sync_error.set(None);
        self.error.set(None);
        self.notice
            .set(Some(format!("Connecting to {}…", provider.name())));
        wasm_bindgen_futures::spawn_local(async move {
            let connection = crate::api::mail_connection(provider).await;
            if !self.alive() {
                return;
            }
            let connection = match connection {
                Ok(connection) => connection,
                Err(error) => {
                    self.finish_sync(None, Some(error.to_string()));
                    return;
                }
            };
            let Some(connection_id) = connection
                .id
                .filter(|_| connection.state == ConnectionState::Connected)
            else {
                self.finish_sync(
                    None,
                    Some(format!(
                        "Connect {} in Connections, then retry sync.",
                        provider.name()
                    )),
                );
                return;
            };
            let mut request = GmailSyncRequest {
                host_id: snapshot.host_id,
                connection_id,
                mailbox_id: mailbox,
                restart,
            };
            let mut updated = 0_usize;
            loop {
                if !self.alive() {
                    return;
                }
                if self.stopping.get_untracked() {
                    self.notice.set(Some(
                        "Sync stopped. Saved pages will be used when you resume.".into(),
                    ));
                    self.finish_sync(request.mailbox_id, None);
                    return;
                }
                let result = crate::api::mail_sync(provider, &request).await;
                if !self.alive() {
                    return;
                }
                match result {
                    Ok(page) => {
                        updated = updated.saturating_add(page.threads_updated);
                        request.restart = false;
                        request.mailbox_id = Some(page.mailbox_id.clone());
                        self.notice.set(Some(if page.caught_up {
                            format!(
                                "{} sync finished. Threads updated: {updated}.",
                                provider.name()
                            )
                        } else {
                            format!(
                                "Syncing {}. {updated} threads saved so far.",
                                provider.name()
                            )
                        }));
                        if page.caught_up {
                            self.finish_sync(Some(page.mailbox_id), None);
                            return;
                        }
                    }
                    Err(error) => {
                        self.finish_sync(request.mailbox_id, Some(error.to_string()));
                        return;
                    }
                }
            }
        });
    }

    fn finish_sync(self, mailbox: Option<String>, error: Option<String>) {
        self.syncing.set(false);
        self.stopping.set(false);
        if let Some(mailbox) = mailbox {
            if self.mailbox.get_untracked().as_ref() != Some(&mailbox) {
                self.clear_selection();
                self.mailbox.set(Some(mailbox));
            }
        }
        if error.is_some() {
            self.notice.set(None);
        }
        self.sync_error.set(error);
        self.refresh();
    }
}
