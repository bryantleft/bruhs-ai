//! Server-owned mail stays separate from locally imported conversations.
//! Workspace owns this entity, so leaving Inbox never cancels a sync page.
use super::*;
use crate::server_client::ServerError;
use bruhs_models::{connections::ServiceProvider, mail::*, InboxFilter};
use bruhs_ui::press::on_press_view;
use gpui_component::zoom::px;
use std::future::Future;
use std::sync::atomic::AtomicBool;

#[cfg(test)]
mod tests;

/// Poll a Tokio job without entering a blocking runtime on a GPUI executor.
/// Keeping its abort guard inside the GPUI task also cancels abandoned reads.
fn mail_request<T: Send + 'static>(
    cx: &App,
    future: impl Future<Output = Result<T, ServerError>> + Send + 'static,
) -> Task<Result<T, ServerError>> {
    struct Worker<T>(tokio::task::JoinHandle<Result<T, ServerError>>);
    impl<T> Drop for Worker<T> {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let mut worker = Worker(cx.global::<DesktopState>().runtime.spawn(future));
    cx.background_spawn(async move {
        (&mut worker.0).await.map_err(|_| {
            ServerError::Invalid("The mail request worker stopped. Retry the request.")
        })?
    })
}

pub(super) enum InboxEvent {
    ShowImports,
    OpenConnections,
}

#[derive(Clone)]
enum Action {
    Imports,
    Connections,
    Refresh,
    Search,
    Sync(bool),
    SyncProvider(ServiceProvider),
    Stop,
    Mailbox(String),
    Filter(InboxFilter),
    Page(u64),
    Thread(String),
    Back,
    Message(usize),
    CopyMessage,
    Download(MailAttachment),
}

pub(super) struct ConnectedInbox {
    client: ServerClient,
    composer: Entity<super::mail_compose::MailComposer>,
    compose_visible: bool,
    _compose_subscription: Subscription,
    snapshot: Option<MailboxesSnapshot>,
    mailbox: Option<String>,
    filter: InboxFilter,
    query: Entity<InputState>,
    applied_query: String,
    page: u64,
    rows: Option<MailThreadsPage>,
    thread: Option<ConnectedMailThread>,
    message: usize,
    selected_thread: Option<String>,
    accounts_loading: bool,
    rows_loading: bool,
    thread_loading: bool,
    generation: u64,
    detail_generation: u64,
    error: Option<String>,
    sync_error: Option<String>,
    download_error: Option<String>,
    downloading: Option<String>,
    download_task: Option<Task<()>>,
    notice: Option<String>,
    feedback_scroll: ScrollHandle,
    feedback_focus: FocusHandle,
    sync_stop: Option<Arc<AtomicBool>>,
    syncing_mailbox: Option<String>,
    sync_task: Option<Task<()>>,
    accounts_task: Option<Task<()>>,
    rows_task: Option<Task<()>>,
    detail_task: Option<Task<()>>,
    _input_subscription: Subscription,
}

impl EventEmitter<InboxEvent> for ConnectedInbox {}

impl Drop for ConnectedInbox {
    fn drop(&mut self) {
        if let Some(stop) = &self.sync_stop {
            stop.store(true, AtomicOrdering::Release);
        }
    }
}

impl Workspace {
    pub(super) fn open_connected_inbox(&mut self, cx: &mut Context<'_, Self>) {
        self.inbox_mail_visible = true;
        self.inbox_import_visible = false;
        self.connected_inbox.update(cx, |view, cx| view.refresh(cx));
        cx.notify();
    }
}

impl ConnectedInbox {
    pub(super) fn new(
        client: ServerClient,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let query = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search people, subjects, and messages…")
        });
        let subscription = cx.subscribe(&query, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.search(cx);
            }
        });
        let composer =
            cx.new(|cx| super::mail_compose::MailComposer::new(client.clone(), window, cx));
        let compose_subscription = cx.subscribe(
            &composer,
            |this, _, _: &super::mail_compose::ComposeEvent, cx| {
                this.compose_visible = false;
                cx.notify();
            },
        );
        Self {
            client,
            composer,
            compose_visible: false,
            _compose_subscription: compose_subscription,
            snapshot: None,
            mailbox: None,
            filter: InboxFilter::Inbox,
            query,
            applied_query: String::new(),
            page: 0,
            rows: None,
            thread: None,
            message: 0,
            selected_thread: None,
            accounts_loading: false,
            rows_loading: false,
            thread_loading: false,
            generation: 0,
            detail_generation: 0,
            error: None,
            sync_error: None,
            download_error: None,
            downloading: None,
            download_task: None,
            notice: None,
            feedback_scroll: ScrollHandle::new(),
            feedback_focus: cx.focus_handle(),
            sync_stop: None,
            syncing_mailbox: None,
            sync_task: None,
            accounts_task: None,
            rows_task: None,
            detail_task: None,
            _input_subscription: subscription,
        }
    }

    #[cfg(test)]
    pub(super) fn compose_test_setup(
        &mut self,
        client: ServerClient,
        snapshot: MailboxesSnapshot,
    ) -> Entity<super::mail_compose::MailComposer> {
        self.client = client;
        self.mailbox = snapshot
            .mailboxes
            .first()
            .map(|mailbox| mailbox.mailbox_id.clone());
        self.snapshot = Some(snapshot);
        self.accounts_loading = false;
        self.composer.clone()
    }

    pub(super) fn refresh(&mut self, cx: &mut Context<'_, Self>) {
        if self.accounts_loading {
            return;
        }
        self.accounts_loading = true;
        let client = self.client.clone();
        let operation = mail_request(cx, async move { client.mailboxes().await });
        self.accounts_task = Some(cx.spawn(async move |this, cx| {
            let result = operation.await;
            let _ = this.update(cx, |this, cx| {
                this.accounts_loading = false;
                match result {
                    Ok(snapshot) => {
                        let changed_host = this
                            .snapshot
                            .as_ref()
                            .is_some_and(|old| old.host_id != snapshot.host_id);
                        if changed_host {
                            this.clear_selection();
                            this.mailbox = None;
                        }
                        if !snapshot
                            .mailboxes
                            .iter()
                            .any(|row| Some(&row.mailbox_id) == this.mailbox.as_ref())
                        {
                            this.clear_selection();
                            this.mailbox =
                                snapshot.mailboxes.first().map(|row| row.mailbox_id.clone());
                        }
                        this.snapshot = Some(snapshot);
                        this.error = None;
                        this.load_page(cx);
                        if let Some(id) = this.selected_thread.clone() {
                            this.open_thread(id, cx);
                        }
                    }
                    Err(error) => this.error = Some(mail_error(&error)),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn clear_selection(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.detail_generation = self.detail_generation.wrapping_add(1);
        self.rows_task = None;
        self.detail_task = None;
        self.rows_loading = false;
        self.thread_loading = false;
        self.rows = None;
        self.thread = None;
        self.selected_thread = None;
        self.page = 0;
    }

    fn search(&mut self, cx: &mut Context<'_, Self>) {
        self.applied_query = self.query.read(cx).value().to_string();
        self.clear_selection();
        self.load_page(cx);
    }

    fn load_page(&mut self, cx: &mut Context<'_, Self>) {
        let (Some(snapshot), Some(mailbox)) = (&self.snapshot, &self.mailbox) else {
            return;
        };
        let query = MailBrowseQuery {
            host_id: snapshot.host_id.clone(),
            query: self.applied_query.clone(),
            filter: self.filter,
            page: self.page,
            // Each response is one consistent cache snapshot. Refresh may have
            // raced a sync page, so do not pin browsing to an older root revision.
            expected_revision: None,
        };
        let mailbox = mailbox.clone();
        let client = self.client.clone();
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.rows_loading = true;
        self.error = None;
        let operation = mail_request(
            cx,
            async move { client.browse_mailbox(&mailbox, &query).await },
        );
        self.rows_task = Some(cx.spawn(async move |this, cx| {
            let result = operation.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.rows_loading = false;
                match result {
                    Ok(rows) => {
                        this.page = rows.page;
                        this.rows = Some(rows);
                    }
                    Err(error) => this.error = Some(mail_error(&error)),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn open_thread(&mut self, external: String, cx: &mut Context<'_, Self>) {
        let (Some(snapshot), Some(mailbox)) = (&self.snapshot, &self.mailbox) else {
            return;
        };
        let client = self.client.clone();
        let host = snapshot.host_id.clone();
        let mailbox = mailbox.clone();
        let previous_message = self
            .thread
            .as_ref()
            .filter(|thread| thread.connected_mail.external_thread_id == external)
            .and_then(|thread| thread.conversation.messages.get(self.message))
            .map(|message| message.id.clone());
        self.selected_thread = Some(external.clone());
        self.thread = None;
        self.thread_loading = true;
        self.error = None;
        self.detail_generation = self.detail_generation.wrapping_add(1);
        let generation = self.detail_generation;
        let operation = mail_request(cx, async move {
            client.mail_thread(&host, &mailbox, &external).await
        });
        self.detail_task = Some(cx.spawn(async move |this, cx| {
            let result = operation.await;
            let _ = this.update(cx, |this, cx| {
                if this.detail_generation != generation {
                    return;
                }
                this.thread_loading = false;
                match result {
                    Ok(thread) => {
                        this.message = previous_message
                            .and_then(|id| {
                                thread
                                    .conversation
                                    .messages
                                    .iter()
                                    .position(|message| message.id == id)
                            })
                            .unwrap_or_else(|| {
                                thread.conversation.messages.len().saturating_sub(1)
                            });
                        this.thread = Some(thread);
                    }
                    Err(error) => this.error = Some(mail_error(&error)),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn sync(&mut self, restart: bool, cx: &mut Context<'_, Self>) {
        let provider = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| {
                snapshot
                    .mailboxes
                    .iter()
                    .find(|row| Some(&row.mailbox_id) == self.mailbox.as_ref())
                    .map(|row| row.provider)
            })
            .unwrap_or(ServiceProvider::Gmail);
        self.sync_provider(provider, restart, cx);
    }

    fn sync_provider(
        &mut self,
        provider: ServiceProvider,
        restart: bool,
        cx: &mut Context<'_, Self>,
    ) {
        if !matches!(provider, ServiceProvider::Gmail | ServiceProvider::Outlook) {
            self.sync_error = Some("Choose a Gmail or Outlook Mail connection.".into());
            cx.notify();
            return;
        }
        if self.sync_stop.is_some() || self.accounts_loading {
            return;
        }
        let Some(snapshot) = &self.snapshot else {
            self.refresh(cx);
            return;
        };
        let host = snapshot.host_id.clone();
        let mailbox = self
            .mailbox
            .as_ref()
            .filter(|id| {
                snapshot
                    .mailboxes
                    .iter()
                    .any(|row| row.mailbox_id == **id && row.provider == provider)
            })
            .cloned();
        let client = self.client.clone();
        let stop = Arc::new(AtomicBool::new(false));
        self.sync_stop = Some(Arc::clone(&stop));
        self.syncing_mailbox = mailbox.clone();
        self.error = None;
        self.notice = Some(format!("Connecting to {}…", provider.name()));
        self.sync_error = None;
        self.sync_task = Some(cx.spawn(async move |this, cx| {
            let operation = cx.update(|cx| {
                mail_request(cx, async move { client.mail_connection(provider).await })
            });
            let connection = operation.await;
            let connection = match connection {
                Ok(connection) => connection,
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        this.finish_sync(Some(mail_error(&error)), cx)
                    });
                    return;
                }
            };
            let Some(connection_id) = connection.id.filter(|_| {
                connection.state == bruhs_models::connections::ConnectionState::Connected
            }) else {
                let _ = this.update(cx, |this, cx| {
                    this.finish_sync(
                        Some(format!(
                            "Connect {} in Connections, then retry sync.",
                            provider.name()
                        )),
                        cx,
                    )
                });
                return;
            };
            let mut request = GmailSyncRequest {
                host_id: host,
                connection_id,
                mailbox_id: mailbox,
                restart,
            };
            let mut updated = 0_usize;
            loop {
                if stop.load(AtomicOrdering::Acquire) {
                    let _ = this.update(cx, |this, cx| {
                        this.notice =
                            Some("Sync stopped. Saved pages will be used when you resume.".into());
                        this.finish_sync(None, cx);
                    });
                    break;
                }
                let Ok(client) = this.read_with(cx, |this, _| this.client.clone()) else {
                    break;
                };
                let page_request = request.clone();
                let operation = cx.update(|cx| {
                    mail_request(cx, async move {
                        client.sync_mail(provider, &page_request).await
                    })
                });
                match operation.await {
                    Ok(page) => {
                        updated = updated.saturating_add(page.threads_updated);
                        request.restart = false;
                        request.mailbox_id = Some(page.mailbox_id.clone());
                        let done = page.caught_up;
                        if this
                            .update(cx, |this, cx| {
                                this.syncing_mailbox = Some(page.mailbox_id.clone());
                                this.notice = Some(if done {
                                    format!(
                                        "{} sync finished. Threads updated: {updated}.",
                                        provider.name()
                                    )
                                } else {
                                    format!(
                                        "Syncing {}. {updated} threads saved so far.",
                                        provider.name()
                                    )
                                });
                                if done {
                                    this.finish_sync(None, cx);
                                }
                                cx.notify();
                            })
                            .is_err()
                            || done
                        {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = this.update(cx, |this, cx| {
                            this.finish_sync(Some(mail_error(&error)), cx)
                        });
                        break;
                    }
                }
            }
        }));
        cx.notify();
    }

    fn finish_sync(&mut self, error: Option<String>, cx: &mut Context<'_, Self>) {
        let mailbox = self.syncing_mailbox.take();
        self.sync_stop = None;
        if let Some(mailbox) = mailbox {
            if self.mailbox.as_ref() != Some(&mailbox) {
                self.clear_selection();
                self.mailbox = Some(mailbox);
            }
        }
        // Reading the committed cache must not clear a provider sync failure.
        if error.is_some() {
            self.notice = None;
        }
        self.sync_error = error;
        self.refresh(cx);
        cx.notify();
    }

    fn action(&mut self, action: Action, cx: &mut Context<'_, Self>) {
        match action {
            Action::Imports => cx.emit(InboxEvent::ShowImports),
            Action::Connections => cx.emit(InboxEvent::OpenConnections),
            Action::Refresh => self.refresh(cx),
            Action::Search => self.search(cx),
            Action::Sync(restart) => self.sync(restart, cx),
            Action::SyncProvider(provider) => self.sync_provider(provider, false, cx),
            Action::Stop => {
                if let Some(stop) = &self.sync_stop {
                    stop.store(true, AtomicOrdering::Release);
                    self.notice = Some("Stopping after the current page is saved…".into());
                }
            }
            Action::Mailbox(id) => {
                self.clear_selection();
                self.mailbox = Some(id);
                self.load_page(cx);
            }
            Action::Filter(filter) => {
                self.filter = filter;
                self.search(cx);
            }
            Action::Page(page) => {
                self.page = page;
                self.rows = None;
                self.load_page(cx);
            }
            Action::Thread(id) => self.open_thread(id, cx),
            Action::Back => {
                self.detail_generation = self.detail_generation.wrapping_add(1);
                self.detail_task = None;
                self.selected_thread = None;
                self.thread = None;
                self.thread_loading = false;
                self.error = None;
            }
            Action::Message(index) => self.message = index,
            Action::CopyMessage => {
                if let Some(message) = self
                    .thread
                    .as_ref()
                    .and_then(|thread| thread.conversation.messages.get(self.message))
                {
                    cx.write_to_clipboard(ClipboardItem::new_string(message.body.clone()));
                    self.notice = Some("Message text copied.".into());
                }
            }
            Action::Download(attachment) => self.download(attachment, cx),
        }
        cx.notify();
    }

    fn download(&mut self, attachment: MailAttachment, cx: &mut Context<'_, Self>) {
        if self.downloading.is_some() {
            return;
        }
        let (Some(snapshot), Some(mailbox), Some(thread)) =
            (&self.snapshot, &self.mailbox, &self.selected_thread)
        else {
            return;
        };
        let Some(provider) = snapshot
            .mailboxes
            .iter()
            .find(|row| row.mailbox_id == *mailbox)
            .map(|row| row.provider)
        else {
            self.download_error =
                Some("Refresh saved accounts before downloading this attachment.".into());
            cx.notify();
            return;
        };
        let (host, mailbox, thread, client) = (
            snapshot.host_id.clone(),
            mailbox.clone(),
            thread.clone(),
            self.client.clone(),
        );
        // The native picker owns the destination and overwrite review. Mail
        // content supplies a suggested basename, never a destination path.
        let name = attachment_file_name(&attachment.file_name);
        let directory = directories::BaseDirs::new().map_or_else(
            || PathBuf::from("."),
            |dirs| dirs.home_dir().join("Downloads"),
        );
        let picker = cx.prompt_for_new_path(&directory, Some(&name));
        self.downloading = Some(attachment.id.clone());
        self.download_error = None;
        self.notice = None;
        self.download_task = Some(cx.spawn(async move |this, cx| {
            let path = match picker.await {
                Ok(Ok(Some(path))) => path,
                result => {
                    let _ = this.update(cx, |this, cx| {
                        this.downloading = None;
                        if matches!(result, Ok(Err(_))) { this.download_error = Some("The save dialog could not be opened. Retry the attachment download.".into()); }
                        cx.notify();
                    });
                    return;
                }
            };
            let operation = cx.update(|cx| mail_request(cx, async move { client.save_provider_mail_attachment(provider,&host, &mailbox, &thread, &attachment, &path).await }));
            let result = operation.await;
            let _ = this.update(cx, |this, cx| {
                this.downloading = None;
                match result {
                    Ok(()) => this.notice = Some("Attachment saved to the selected file.".into()),
                    Err(error) => this.download_error = Some(match error {
                        ServerError::Invalid(message) => message.into(),
                        other => mail_error(&other),
                    }),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn button(
        &self,
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        action: Action,
        enabled: bool,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        let label = label.into();
        let id = id.into();
        let debug_id = id.to_string();
        web_button(
            id,
            WebButtonVariant::Ghost,
            WebButtonSize::Default,
            if enabled {
                ButtonState::Enabled
            } else {
                ButtonState::Disabled
            },
        )
        .aria_label(label.clone())
        .debug_selector(move || debug_id)
        .child(label)
        .when(enabled, |el| {
            on_press_view(el, cx, move |this, _window, cx| {
                this.action(action.clone(), cx);
            })
        })
    }

    fn compose_entry(&self, reply: Option<bool>, cx: &Context<'_, Self>) -> Div {
        let labels = if reply.is_some() {
            [("Reply", Some(false)), ("Reply all", Some(true))]
        } else {
            [("New email", None), ("Drafts", None)]
        };
        div()
            .flex()
            .gap_2()
            .children(labels.into_iter().enumerate().map(|(index, (label, all))| {
                let enabled = self.mailbox.is_some() && !self.accounts_loading;
                web_button(
                    ("desktop-compose-entry", index),
                    WebButtonVariant::Ghost,
                    WebButtonSize::Default,
                    if enabled {
                        ButtonState::Enabled
                    } else {
                        ButtonState::Disabled
                    },
                )
                .child(label)
                .aria_label(label)
                .debug_selector(move || format!("desktop-compose-entry-{label}"))
                .when(enabled, |el| {
                    on_press_view(el, cx, move |this, window, cx| {
                        this.open_composer(reply.is_none() && index == 1, all, window, cx);
                    })
                })
            }))
    }
    fn open_composer(
        &mut self,
        drafts: bool,
        reply: Option<bool>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let (Some(snapshot), Some(mailbox)) = (&self.snapshot, &self.mailbox) else {
            return;
        };
        let Some(account) = snapshot
            .mailboxes
            .iter()
            .find(|account| &account.mailbox_id == mailbox)
        else {
            return;
        };
        let scope = bruhs_core::mail::device::DeviceDraftScope {
            origin: self.client.mail_compose_origin().into(),
            host_id: snapshot.host_id.clone(),
            mailbox_id: mailbox.clone(),
        };
        let mut content = MailComposeContent::default();
        let mut target = None;
        if let Some(all) = reply {
            let Some(thread) = &self.thread else {
                return;
            };
            let Some(message) = thread.conversation.messages.get(self.message) else {
                return;
            };
            let Some(detail) = thread
                .connected_mail
                .messages
                .iter()
                .find(|detail| detail.id == message.id)
            else {
                return;
            };
            content = reply_content(detail, message.direction, &account.account_label, all);
            target = Some(MailReplyTarget {
                external_thread_id: thread.connected_mail.external_thread_id.clone(),
                message_id: message.id.clone(),
            });
        }
        let provider = account.provider;
        self.composer.update(cx, |view, cx| {
            if drafts {
                view.show_drafts(scope, provider, window, cx);
            } else {
                view.start(scope, provider, target, content, window, cx);
            }
        });
        self.compose_visible = true;
        cx.notify();
    }
    pub(super) fn compose_pending(&self, cx: &App) -> bool {
        self.composer.read(cx).pending()
    }

    fn render_rows(&self, cx: &Context<'_, Self>) -> Div {
        let has_mailbox = self.mailbox.is_some();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.compose_entry(None,cx))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.query).aria_label("Search email")),
                    )
                    .child(self.button(
                        "desktop-mail-search",
                        "Search",
                        Action::Search,
                        has_mailbox,
                        cx,
                    )),
            )
            .child(
                div()
                    .id("desktop-mail-filters")
                    .role(Role::Group)
                    .aria_label("Email filter")
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .children(InboxFilter::ALL.into_iter().map(|filter| {
                        self.button(
                            ("mail-filter", element_key(filter.label())),
                            filter.label(),
                            Action::Filter(filter),
                            has_mailbox,
                            cx,
                        )
                        .debug_selector(move || format!("desktop-mail-filter-{}", filter.label()))
                        .aria_selected(self.filter == filter)
                        .when(self.filter == filter, |button| {
                            button
                                .bg(theme::blueberry_500().opacity(0.15))
                                .text_color(theme::blueberry_400())
                        })
                    })),
            )
            .when(self.rows_loading, |el| {
                el.child(status("Loading cached email…"))
            })
            .child(
                div()
                    .border_1()
                    .border_color(theme::longan_800())
                    .rounded(px(8.0))
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .children(self.rows.iter().flat_map(|page| &page.rows).map(|row| {
                        let card = &row.card;
                        let id = row.external_id.clone();
                        let keyboard_id = id.clone();
                        div()
                            .id(("mail-row", element_key(&card.id)))
                            .role(Role::Button)
                            .tab_index(0)
                            .debug_selector(move || {
                                format!("desktop-mail-row-{}", element_key(&keyboard_id))
                            })
                            .aria_label(format!(
                                "{}, {}, {}",
                                card.sender,
                                card.subject,
                                if card.unread { "unread" } else { "read" }
                            ))
                            .px_4()
                            .py_3()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .border_b_1()
                            .border_color(theme::longan_800())
                            .cursor_pointer()
                            .hover(|el| el.bg(theme::longan_900()))
                            .focus_visible(|el| el.bg(theme::blueberry_500().opacity(0.15)))
                            .child(
                                div()
                                    .flex()
                                    .gap_3()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .font_weight(if card.unread {
                                                FontWeight::SEMIBOLD
                                            } else {
                                                FontWeight::NORMAL
                                            })
                                            .child(card.sender.clone()),
                                    )
                                    .child(meta(inbox_ui::inbox_date(card.message_at_ms))),
                            )
                            .child(div().truncate().text_color(theme::rambutan_200()).child(
                                if card.subject.is_empty() {
                                    "No subject".into()
                                } else {
                                    card.subject.clone()
                                },
                            ))
                            .child(meta(card.preview.clone()).truncate())
                            .map(|el| {
                                on_press_view(el, cx, move |this, _window, cx| {
                                    this.action(Action::Thread(id.clone()), cx)
                                })
                            })
                    }))
                    .when(
                        !self.rows_loading
                            && self.rows.as_ref().is_none_or(|page| page.rows.is_empty()),
                        |el| {
                            el.child(
                                div()
                                    .debug_selector(|| "desktop-mail-empty".into())
                                    .p_6()
                                    .child(status(if !has_mailbox {
                                        "Connect Gmail or Outlook, then sync to save mail on this server."
                                    } else if self.applied_query.trim().is_empty() {
                                        self.filter.empty_message()
                                    } else {
                                        "No matching email. Try another search or choose All."
                                    })),
                            )
                        },
                    ),
            )
            .when_some(self.rows.as_ref(), |el, page| {
                let start = page.page.saturating_mul(
                    u64::try_from(bruhs_models::INBOX_PAGE_SIZE).unwrap_or(u64::MAX),
                );
                let end = start.saturating_add(u64::try_from(page.rows.len()).unwrap_or(u64::MAX));
                el.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .items_center()
                        .justify_between()
                        .child(self.button(
                            "desktop-mail-previous",
                            "Previous",
                            Action::Page(page.page.saturating_sub(1)),
                            page.page > 0 && !self.rows_loading,
                            cx,
                        ))
                        .child(status(if page.total_matches == 0 {
                            "0 threads".into()
                        } else {
                            format!(
                                "{} to {end} of {}",
                                start.saturating_add(1),
                                page.total_matches
                            )
                        }))
                        .child(self.button(
                            "desktop-mail-next",
                            "Next",
                            Action::Page(page.page.saturating_add(1)),
                            end < page.total_matches && !self.rows_loading,
                            cx,
                        )),
                )
            })
    }

    fn render_thread(&self, cx: &Context<'_, Self>) -> Div {
        let mut view = div().flex().flex_col().gap_4().child(self.button(
            "desktop-mail-back",
            "Back to email",
            Action::Back,
            true,
            cx,
        ));
        if self.thread_loading {
            return view.child(status("Loading saved thread…"));
        }
        let Some(thread) = &self.thread else {
            return view;
        };
        let conversation = &thread.conversation;
        view = view.child(
            div()
                .text_size(px(18.0))
                .font_family(theme::HEADING_FONT_FAMILY)
                .text_color(theme::rambutan_100())
                .child(gpui::text!(
                    id = "desktop-mail-subject",
                    if conversation.subject.is_empty() {
                        "No subject".into()
                    } else {
                        conversation.subject.clone()
                    }
                )),
        );
        let Some(message) = conversation.messages.get(self.message) else {
            return view.child(status("This thread has no saved messages."));
        };
        let detail = thread
            .connected_mail
            .messages
            .iter()
            .find(|detail| detail.id == message.id);
        view = view
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .child(self.button(
                        "desktop-mail-older-message",
                        "Older message",
                        Action::Message(self.message.saturating_sub(1)),
                        self.message > 0,
                        cx,
                    ))
                    .child(status(format!(
                        "Message {} of {}",
                        self.message.saturating_add(1),
                        conversation.messages.len()
                    )))
                    .child(self.button(
                        "desktop-mail-newer-message",
                        "Newer message",
                        Action::Message(self.message.saturating_add(1)),
                        self.message.saturating_add(1) < conversation.messages.len(),
                        cx,
                    )),
            )
            .child(meta(inbox_ui::inbox_date(message.sent_at_ms)));
        view = view.child(self.button(
            "desktop-mail-copy",
            "Copy message text",
            Action::CopyMessage,
            true,
            cx,
        ));
        if let Some(detail) = detail {
            view = view
                .child(meta(format!("From: {}", addresses(&detail.envelope.from))))
                .child(meta(format!("To: {}", addresses(&detail.envelope.to))))
                .when(!detail.envelope.cc.is_empty(), |el| {
                    el.child(meta(format!("Cc: {}", addresses(&detail.envelope.cc))))
                })
                .children(detail.attachments.iter().map(|attachment| {
                    let downloadable = thread.connected_mail.provider != ServiceProvider::Outlook
                        || attachment.external_id.is_some();
                    div()
                        .id(("mail-attachment", element_key(&attachment.id)))
                        .min_w_0()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(
                            meta(format!(
                                "Attachment: {} ({} bytes)",
                                attachment.file_name, attachment.size_bytes
                            ))
                            .flex_1()
                            .truncate(),
                        )
                        .child(
                            self.button(
                                ("mail-download", element_key(&attachment.id)),
                                "Save attachment",
                                Action::Download(attachment.clone()),
                                self.downloading.is_none() && downloadable,
                                cx,
                            )
                            .aria_label(format!("Save attachment: {}", attachment.file_name)),
                        )
                        .when(!downloadable, |el| {
                            el.child(meta(
                                "Linked or unsupported attachment. Open it in Outlook.",
                            ))
                        })
                }));
        }
        view.child(
            div()
                .p_4()
                .rounded(px(8.0))
                .bg(theme::longan_900())
                .border_1()
                .border_color(theme::longan_800())
                .whitespace_normal()
                .child(gpui::text!(
                    id = "desktop-mail-message-body",
                    message.body.clone()
                )),
        )
        .child(self.compose_entry(Some(false), cx))
        .child(meta(
            "Saved plain text. Remote images and HTML are not loaded.",
        ))
    }

    fn render_status(&self, cx: &Context<'_, Self>) -> impl IntoElement {
        div()
            .id("desktop-mail-feedback")
            .role(Role::Group)
            .aria_label("Email status")
            .tab_index(0)
            .track_focus(&self.feedback_focus)
            .debug_selector(|| "desktop-mail-feedback".into())
            .min_h_0()
            .min_w_0()
            .h_auto()
            .max_h(px(128.0))
            .flex_shrink_0()
            .relative()
            .overflow_y_scroll()
            .track_scroll(&self.feedback_scroll)
            .vertical_scrollbar(&self.feedback_scroll)
            .px_4()
            .py_2()
            .flex()
            .flex_col()
            .gap_1()
            .border_b_1()
            .border_color(theme::longan_800())
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let mut offset = this.feedback_scroll.offset();
                let page = this.feedback_scroll.bounds().size.height * 0.8;
                let maximum = this.feedback_scroll.max_offset().y;
                offset.y = match event.keystroke.key.as_str() {
                    "up" => (offset.y + px(40.0)).min(px(0.0)),
                    "down" => (offset.y - px(40.0)).max(-maximum),
                    "pageup" => (offset.y + page).min(px(0.0)),
                    "pagedown" => (offset.y - page).max(-maximum),
                    "home" => px(0.0),
                    "end" => -maximum,
                    _ => return,
                };
                this.feedback_scroll.set_offset(offset);
                window.prevent_default();
                cx.stop_propagation();
                cx.notify();
            }))
            .when(self.accounts_loading, |el| {
                el.child(status("Refreshing saved accounts…"))
            })
            .when_some(self.error.clone(), |el, error| {
                el.child(
                    div()
                        .id("desktop-mail-error")
                        .role(Role::Alert)
                        .aria_label(error.clone())
                        .flex_shrink_0()
                        .text_color(theme::lychee_400())
                        .child(error),
                )
            })
            .when_some(self.sync_error.clone(), |el, error| {
                el.child(
                    div()
                        .id("desktop-mail-sync-error")
                        .role(Role::Alert)
                        .aria_label(error.clone())
                        .flex_shrink_0()
                        .text_color(theme::lychee_400())
                        .child(error),
                )
            })
            .when_some(self.download_error.clone(), |el, error| {
                el.child(
                    div()
                        .id("desktop-mail-download-error")
                        .role(Role::Alert)
                        .aria_label(error.clone())
                        .flex_shrink_0()
                        .text_color(theme::lychee_400())
                        .child(error),
                )
            })
            .when(self.downloading.is_some(), |el| {
                el.child(status(
                    "Choose a file and wait for the attachment to finish saving…",
                ))
            })
            .when_some(self.notice.clone(), |el, notice| el.child(status(notice)))
    }
}

impl Render for ConnectedInbox {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        if self.compose_visible {
            return div().size_full().child(self.composer.clone());
        }
        let syncing = self.sync_stop.is_some();
        div().debug_selector(|| "desktop-connected-inbox".into()).size_full().min_w_0().flex().flex_col()
            .font_family(theme::UI_FONT_FAMILY).text_size(px(13.0)).text_color(theme::rambutan_200()).bg(theme::longan_950())
            .child(div().px_4().py_3().flex().flex_wrap().items_center().justify_between().gap_3().border_b_1().border_color(theme::longan_800())
                .child(div().child(div().font_family(theme::HEADING_FONT_FAMILY).text_size(px(18.0)).text_color(theme::rambutan_100()).child("Email"))
                    .child(meta("Connected Gmail and Outlook, saved on this server.")))
                .child(div().flex().flex_wrap().gap_1()
                    .child(self.button("desktop-mail-imports", "Imported conversations", Action::Imports, true, cx))
                    .child(self.button("desktop-mail-connections", "Connections", Action::Connections, true, cx))
                    .child(self.button("desktop-mail-refresh", "Refresh", Action::Refresh, !self.accounts_loading, cx))
                    .child(self.button("desktop-mail-sync", if syncing { "Syncing…" } else { "Sync Gmail" }, Action::SyncProvider(ServiceProvider::Gmail), !syncing && self.snapshot.is_some() && !self.accounts_loading, cx))
                    .child(self.button("desktop-mail-sync-outlook", "Sync Outlook", Action::SyncProvider(ServiceProvider::Outlook), !syncing && self.snapshot.is_some() && !self.accounts_loading, cx))
                    .when(syncing, |el| el.child(self.button("desktop-mail-stop", "Stop sync", Action::Stop, true, cx)))))
            .when(self.compose_pending(cx),|el|el.child(status("Email edits are waiting to be saved on this device.")))
            .when(self.accounts_loading || self.error.is_some() || self.sync_error.is_some() || self.download_error.is_some() || self.downloading.is_some() || self.notice.is_some(), |el| el.child(self.render_status(cx)))
            .child(div().id("desktop-mail-scroll").debug_selector(|| "desktop-mail-scroll".into()).flex_1().min_h_0().overflow_y_scrollbar().p_4().flex().justify_center()
                .child(div().w_full().max_w(px(1024.0)).min_w_0().flex().flex_col().gap_4()
                    .child(div().id("desktop-mail-accounts").role(Role::Group).aria_label("Saved mail accounts").flex().flex_wrap().gap_1()
                        .children(self.snapshot.iter().flat_map(|snapshot| &snapshot.mailboxes).map(|mailbox| {
                            let label = if mailbox.account_label.is_empty() { mailbox.provider.name().to_string() } else { mailbox.account_label.clone() };
                            self.button(("mailbox", element_key(&mailbox.mailbox_id)), format!("{label}: {} threads", mailbox.threads), Action::Mailbox(mailbox.mailbox_id.clone()), !syncing, cx)
                                .aria_selected(self.mailbox.as_ref() == Some(&mailbox.mailbox_id))
                                .when(self.mailbox.as_ref() == Some(&mailbox.mailbox_id), |el| el.bg(theme::blueberry_500().opacity(0.15)).text_color(theme::blueberry_400()))
                        })))
                    .child(if self.selected_thread.is_some() { self.render_thread(cx) } else { self.render_rows(cx) })
                    .child(div().border_t_1().border_color(theme::longan_800()).pt_3()
                        .child(meta("Sync reads Gmail or Outlook into this server's private cache. Imported conversations remain on this device."))
                        .child(self.button("desktop-mail-restart", "Restart sync", Action::Sync(true), !syncing && self.snapshot.is_some() && !self.accounts_loading, cx)))))
    }
}

fn status(text: impl Into<SharedString>) -> Stateful<Div> {
    let text = text.into();
    let key = element_key(&text);
    meta(text)
        .id(("mail-status", key))
        .role(Role::Status)
        .flex_shrink_0()
}

#[track_caller]
fn meta(text: impl Into<SharedString>) -> Div {
    div()
        .min_w_0()
        .text_size(px(12.0))
        .line_height(relative(1.5))
        .text_color(theme::rambutan_400())
        .child(gpui::Text::new(
            ElementId::CodeLocation(*std::panic::Location::caller()),
            text.into(),
        ))
}

fn addresses(addresses: &[MailAddress]) -> String {
    addresses
        .iter()
        .map(|address| {
            if address.name.is_empty() {
                address.address.clone()
            } else {
                format!("{} <{}>", address.name, address.address)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn mail_error(error: &ServerError) -> String {
    // ServerError response bodies are log-only. Never put an arbitrary provider
    // response, search string, or credential into a native error label.
    match error {
        ServerError::Status { code, .. } => match code.as_u16() {
            401 | 403 => "Reconnect the email account in Connections and verify mail access, then retry sync.",
            409 => "Mail changed or belongs to another server or account. Refresh accounts, then retry. Restart sync can repair an invalid saved sync position.",
            429 => "The email provider is limiting requests. Wait before syncing again. Saved mail remains available.",
            404 => "This saved mail is no longer available. Refresh the mailbox.",
            400 => "The mail request could not be completed. Check the email connection and sync setup, then retry.",
            _ => "The mail server could not complete this request. Retry; already saved mail was retained.",
        },
        ServerError::Transport { .. } | ServerError::Client(_) => "The mail server could not be reached. Retry when it is available.",
        ServerError::Decode { .. } | ServerError::Invalid(_) => "The mail server sent a reply this app could not read. Refresh and retry.",
    }.into()
}
