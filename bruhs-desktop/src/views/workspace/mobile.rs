//! Native session owner and narrow phone broker. Never reads companion-server threads.
mod controls;
mod linked_drafts;
#[cfg(test)]
mod tests;
use super::*;
use bruhs_core::mobile::{
    authority::{MobileAuthority, MobileLaunch, MobilePairingOffer},
    MobileError, MobileHost, MobileHostConfig, MobilePending, MobileQr, MobileResult,
    MobileTransport,
};
use bruhs_models::{mobile::*, num::ToFloatLossy};
use std::time::Duration;

fn default_environment_icon() -> String {
    "laptop".into()
}

fn default_require_approval() -> bool {
    true
}

/// Persisted environment identity and device approval preferences for Settings → Access.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct AccessPreferences {
    #[serde(default = "default_environment_icon")]
    pub environment_icon: String,
    #[serde(default = "default_require_approval")]
    pub require_device_approval: bool,
    pub hold_webhooks_while_offline: bool,
}

impl Default for AccessPreferences {
    fn default() -> Self {
        Self {
            environment_icon: default_environment_icon(),
            require_device_approval: default_require_approval(),
            hold_webhooks_while_offline: false,
        }
    }
}

impl AccessPreferences {
    pub(super) const ICONS: &[(&str, &str)] = &[
        ("laptop", "Laptop"),
        ("desktop", "Desktop"),
        ("mac-mini", "Mac Mini"),
        ("studio", "Mac Studio"),
        ("server", "Server"),
        ("cloud", "Cloud / VM"),
        ("linux", "Linux"),
        ("terminal", "Terminal"),
    ];
}

enum Connection {
    Off,
    Starting(uuid::Uuid),
    Connected { id: uuid::Uuid, host: MobileHost },
    Stopping(uuid::Uuid),
}
pub(super) struct MobileUi {
    launch: MobileLaunch,
    connection: Connection,
    origin: Entity<InputState>,
    bind: Entity<InputState>,
    certificate: Entity<InputState>,
    key: Entity<InputState>,
    hours: Entity<InputState>,
    proxy: bool,
    scopes: MobileScopes,
    offer: Option<(MobilePairingOffer, Arc<MobileQr>)>,
    error: Option<String>,
    worker: Option<Task<()>>,
    watcher: Option<Task<()>>,
    expiry: Option<Task<()>>,
    linked_drafts: BTreeMap<uuid::Uuid, linked_drafts::LinkedDraft>,
    scratch_owners: BTreeMap<String, uuid::Uuid>,
}
impl MobileUi {
    pub(super) fn new(window: &mut Window, cx: &mut App) -> Self {
        fn field(value: &str, window: &mut Window, cx: &mut App) -> Entity<InputState> {
            cx.new(|cx| {
                let mut input = InputState::new(window, cx);
                input.set_value(value, window, cx);
                input
            })
        }
        Self {
            launch: MobileLaunch::process_owner(),
            connection: Connection::Off,
            origin: field("https://", window, cx),
            bind: field("127.0.0.1:7411", window, cx),
            certificate: field("", window, cx),
            key: field("", window, cx),
            hours: field("8", window, cx),
            proxy: true,
            scopes: [MobileScope::Read].into(),
            offer: None,
            error: None,
            worker: None,
            watcher: None,
            expiry: None,
            linked_drafts: BTreeMap::new(),
            scratch_owners: BTreeMap::new(),
        }
    }
    fn host(&self) -> Option<&MobileHost> {
        if let Connection::Connected { host, .. } = &self.connection {
            Some(host)
        } else {
            None
        }
    }
    fn current(&self, id: uuid::Uuid) -> bool {
        matches!(self.connection,Connection::Connected{id:current,..} if id==current)
    }
}
impl Workspace {
    pub(super) fn sync_mobile_offer(&mut self) {
        let pairing_visible = if self.settings_visible() {
            self.settings_section == SettingsSection::Access
        } else {
            self.active_view == AppView::Automations
        };
        let expired = self.mobile.offer.as_ref().is_some_and(|(offer, _)| {
            !pairing_visible
                || !self
                    .mobile
                    .host()
                    .is_some_and(|host| host.authority().pairing_pending(offer.id))
        });
        if expired {
            self.close_mobile_offer();
        }
    }
    fn close_mobile_offer(&mut self) {
        if let Some((offer, _)) = self.mobile.offer.take() {
            if let Some(host) = self.mobile.host() {
                host.authority().cancel_pairing(offer.id);
            }
        }
        self.mobile.expiry = None;
    }
    fn start_mobile(&mut self, cx: &mut Context<'_, Self>) {
        if !matches!(self.mobile.connection, Connection::Off) {
            return;
        }
        let result = (|| {
            let bind = self.mobile.bind.read(cx).value().parse().map_err(|_| {
                MobileError::Invalid(
                    "Enter an IP address and port, for example 127.0.0.1:7411.".into(),
                )
            })?;
            let origin = self.mobile.origin.read(cx).value().parse().map_err(|_| {
                MobileError::Invalid("Enter the HTTPS origin your phone already trusts.".into())
            })?;
            let transport = if self.mobile.proxy {
                MobileTransport::HttpsProxy
            } else {
                MobileTransport::DirectTls {
                    certificate: PathBuf::from(self.mobile.certificate.read(cx).value().as_str()),
                    private_key: PathBuf::from(self.mobile.key.read(cx).value().as_str()),
                }
            };
            let path = crate::paths::desktop_state_file()
                .and_then(|p| p.parent().map(|p| p.join("mobile/devices.json")))
                .ok_or_else(|| {
                    MobileError::Invalid("The private app data folder is unavailable.".into())
                })?;
            Ok::<_, MobileError>((
                MobileHostConfig {
                    bind,
                    origin,
                    transport,
                },
                path,
            ))
        })();
        let (config, path) = match result {
            Ok(config) => config,
            Err(error) => {
                self.mobile.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let id = uuid::Uuid::new_v4();
        self.mobile.connection = Connection::Starting(id);
        self.mobile.error = None;
        let (send, receive) = tokio::sync::oneshot::channel();
        let launch = self.mobile.launch.clone();
        cx.global::<DesktopState>().runtime.spawn(async move {
            let result = async {
                let authority = MobileAuthority::open(path, launch).await?;
                match MobileHost::start(config, authority.clone()).await {
                    Ok(host) => Ok(host),
                    Err(error) => {
                        authority.release().await;
                        Err(error)
                    }
                }
            }
            .await;
            if let Err(Ok((host, _))) = send.send(result) {
                host.shutdown().await;
            }
        });
        cx.spawn(async move|this,cx|{if let Ok(result)=receive.await{let _=this.update(cx,|this,cx|{
            if !matches!(this.mobile.connection,Connection::Starting(current) if id==current){return;}
            match result {Ok((host,requests))=>this.attach_mobile(id,host,requests,cx),Err(error)=>{this.mobile.connection=Connection::Off;this.mobile.error=Some(error.to_string());}}
            cx.notify();
        });}}).detach();
        cx.notify();
    }
    fn attach_mobile(
        &mut self,
        id: uuid::Uuid,
        host: MobileHost,
        mut requests: tokio::sync::mpsc::Receiver<MobilePending>,
        cx: &mut Context<'_, Self>,
    ) {
        let mut changes = host.authority().changes();
        let mut failures = host.authority().failures();
        self.mobile.connection = Connection::Connected { id, host };
        self.mobile.linked_drafts.clear();
        self.mobile.scratch_owners.clear();
        self.mobile.worker = Some(cx.spawn(async move |this, cx| {
            while let Some(pending) = requests.recv().await {
                if pending.is_closed() {
                    continue;
                }
                if this
                    .update(cx, |this, cx| {
                        if this.mobile.current(id) {
                            this.handle_mobile(id, pending, cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
        self.mobile.watcher = Some(cx.spawn(async move |this, cx| loop {
            let changed = async {
                tokio::select! {r=changes.changed()=>r,r=failures.changed()=>r}
            };
            if changed.await.is_err() {
                break;
            }
            if this
                .update(cx, |this, cx| {
                    if !this.mobile.current(id) {
                        return false;
                    }
                    this.sync_mobile_offer();
                    if let Some(host) = this.mobile.host() {
                        if let Err(error) = host.authority().healthy() {
                            this.mobile.error = Some(error.to_string());
                        }
                    }
                    cx.notify();
                    true
                })
                .ok()
                != Some(true)
            {
                break;
            }
        }));
    }
    fn stop_mobile(&mut self, cx: &mut Context<'_, Self>) {
        self.close_mobile_offer();
        self.mobile.worker = None;
        self.mobile.watcher = None;
        let old = std::mem::replace(&mut self.mobile.connection, Connection::Off);
        if let Connection::Connected { id, host } = old {
            host.stop_now();
            self.mobile.connection = Connection::Stopping(id);
            let (send, receive) = tokio::sync::oneshot::channel();
            cx.global::<DesktopState>().runtime.spawn(async move {
                host.shutdown().await;
                let _ = send.send(());
            });
            cx.spawn(async move |this, cx| {
                let _ = receive.await;
                let _ = this.update(cx, |this, cx| {
                    if matches!(this.mobile.connection,Connection::Stopping(current) if current==id)
                    {
                        this.mobile.connection = Connection::Off;
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        cx.notify();
    }
    fn offer_mobile(&mut self, cx: &mut Context<'_, Self>) {
        self.close_mobile_offer();
        let result = (|| {
            let host = self.mobile.host().ok_or(MobileError::Disabled)?;
            let hours: u64 = self.mobile.hours.read(cx).value().parse().map_err(|_| {
                MobileError::Invalid("Choose a device lifetime from 1 to 24 hours.".into())
            })?;
            if !(1..=24).contains(&hours) {
                return Err(MobileError::Invalid(
                    "Choose a device lifetime from 1 to 24 hours.".into(),
                ));
            }
            let offer = host.authority().begin_pairing(
                host.origin(),
                self.mobile.scopes.clone(),
                Duration::from_secs(hours * 60 * 60),
            )?;
            match MobileQr::encode(&offer) {
                Ok(qr) => Ok((offer, Arc::new(qr))),
                Err(error) => {
                    host.authority().cancel_pairing(offer.id);
                    Err(error)
                }
            }
        })();
        match result {
            Ok(offer) => {
                let offer_id = offer.0.id;
                self.mobile.offer = Some(offer);
                self.mobile.error = None;
                self.mobile.expiry = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(bruhs_core::mobile::PAIRING_LIFETIME)
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        if this
                            .mobile
                            .offer
                            .as_ref()
                            .is_some_and(|(o, _)| o.id == offer_id)
                        {
                            this.close_mobile_offer();
                            cx.notify();
                        }
                    });
                }));
            }
            Err(error) => self.mobile.error = Some(error.to_string()),
        }
        cx.notify();
    }
    fn revoke_mobile(&mut self, id: uuid::Uuid, cx: &mut Context<'_, Self>) {
        let Some(host) = self.mobile.host() else {
            return;
        };
        let authority = host.authority();
        if let Err(error) = authority.revoke_now(id) {
            self.mobile.error = Some(error.to_string());
            cx.notify();
            return;
        }
        let (send, receive) = tokio::sync::oneshot::channel();
        cx.global::<DesktopState>().runtime.spawn(async move {
            let _ = send.send(authority.persist_revocation().await);
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Err(error)) = receive.await {
                let _ = this.update(cx, |this, cx| {
                    this.mobile.error = Some(error.to_string());
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }
    fn mobile_thread_visible(&self, thread: &DesktopThread, device: uuid::Uuid, cx: &App) -> bool {
        thread.chat.read(cx).project_scoped
            || self.mobile.scratch_owners.get(&thread.id) == Some(&device)
    }

    fn mobile_thread<'a>(
        &'a self,
        id: &str,
        device: uuid::Uuid,
        cx: &App,
    ) -> MobileResult<&'a DesktopThread> {
        self.threads
            .iter()
            .find(|thread| thread.id == id && self.mobile_thread_visible(thread, device, cx))
            .ok_or_else(|| {
                MobileError::Conflict(
                    "This project thread is no longer available. Reload the thread list.".into(),
                )
            })
    }
    fn mobile_summary(thread: &DesktopThread, cx: &App) -> MobileThread {
        let chat = thread.chat.read(cx);
        MobileThread {
            id: thread.id.clone(),
            title: crate::views::chat::mobile_label(&chat.thread_title, 512),
            project: crate::views::chat::mobile_label(&thread.project_name, 512),
            updated_at_ms: thread.updated_at_ms.as_millis(),
            state: chat.mobile_state(),
            needs_answer: chat.mobile_needs_answer(),
        }
    }
    fn read_mobile(
        &self,
        read: &MobileRead,
        device: uuid::Uuid,
        cx: &App,
    ) -> MobileResult<MobileReadReply> {
        match read {
            MobileRead::LinkedDraft { link_id } => self.mobile_link_status(device, *link_id),
            MobileRead::Threads { offset } => {
                let mut threads = Vec::new();
                let mut used = 128;
                let mut count = 0;
                let mut more = false;
                for thread in self
                    .threads
                    .iter()
                    .filter(|thread| self.mobile_thread_visible(thread, device, cx))
                    .skip(*offset)
                {
                    let row = Self::mobile_summary(thread, cx);
                    let bytes = serde_json::to_vec(&row)
                        .map_err(|_| MobileError::Unavailable)?
                        .len()
                        + 1;
                    if threads.is_empty() && used + bytes > RESPONSE_BYTES {
                        return Err(MobileError::Invalid("A desktop thread identifier is too large for mobile preview. Review or reimport that thread on the laptop.".into()));
                    }
                    if count >= PAGE_SIZE || used + bytes > RESPONSE_BYTES {
                        more = true;
                        break;
                    }
                    used += bytes;
                    count += 1;
                    threads.push(row);
                }
                Ok(MobileReadReply::Threads(MobileThreadPage {
                    threads,
                    next_offset: more.then_some(offset + count),
                }))
            }
            MobileRead::Thread { thread_id, before } => {
                let thread = self.mobile_thread(thread_id, device, cx)?;
                Ok(MobileReadReply::Thread(Box::new(
                    thread
                        .chat
                        .read(cx)
                        .mobile_snapshot(Self::mobile_summary(thread, cx), *before)?,
                )))
            }
            MobileRead::Message {
                thread_id,
                index,
                reference,
                offset,
                revision,
            } => {
                let thread = self.mobile_thread(thread_id, device, cx)?;
                Ok(MobileReadReply::Text(thread.chat.read(cx).mobile_message(
                    *index,
                    reference,
                    *offset,
                    revision.as_deref(),
                )?))
            }
            MobileRead::Diff { .. } | MobileRead::QuestionLink(_) => Err(MobileError::Invalid(
                "Project reads must resolve the native working folder first.".into(),
            )),
        }
    }

    fn mobile_project_source(
        &self,
        read: &MobileRead,
        device: uuid::Uuid,
        cx: &App,
    ) -> MobileResult<(PathBuf, Option<MobileProjectLink>)> {
        match read {
            MobileRead::Diff { thread_id, .. } => {
                let chat = self.mobile_thread(thread_id, device, cx)?.chat.read(cx);
                Ok((PathBuf::from(&chat.project_path), None))
            }
            MobileRead::QuestionLink(read) => {
                let chat = self
                    .mobile_thread(&read.thread_id, device, cx)?
                    .chat
                    .read(cx);
                Ok((
                    PathBuf::from(&chat.project_path),
                    Some(chat.mobile_question_link(read)?),
                ))
            }
            _ => Err(MobileError::Unavailable),
        }
    }
    fn handle_mobile(
        &mut self,
        id: uuid::Uuid,
        pending: MobilePending,
        cx: &mut Context<'_, Self>,
    ) {
        if pending.is_closed() {
            return;
        }
        match pending {
            MobilePending::Command {
                lease,
                ticket,
                respond,
            } => {
                let device = lease.id();
                let mut link_work = None;
                let result = lease.authority().commit(&lease, &ticket, |action| {
                    let thread_id = match action {
                        MobileAction::Prompt { thread_id, .. }
                        | MobileAction::Permission { thread_id, .. }
                        | MobileAction::AnswerQuestion { thread_id, .. } => thread_id,
                        MobileAction::OpenLinkedDraft { .. }
                        | MobileAction::ContinueLinkedDraft { .. } => {
                            link_work =
                                Some(self.reserve_mobile_link(device, ticket.id(), action)?);
                            return Ok(MobileCommandOutcome::Accepted { queued: true });
                        }
                    };
                    let chat = self.mobile_thread(thread_id, device, cx)?.chat.clone();
                    chat.update(cx, |chat, cx| chat.mobile_action(action, device, cx))
                });
                let accepted = result.is_ok();
                let _ = respond.send(result);
                if let Some(work) = link_work.filter(|_| accepted) {
                    self.start_mobile_link(id, lease, work, cx);
                }
            }
            MobilePending::QuestionFile {
                lease,
                file,
                respond,
            } => {
                let device = lease.id();
                let result = lease
                    .with_scope(MobileScope::Prompt, || {
                        self.mobile_thread(file.upload.thread_id.as_str(), device, cx)?
                            .chat
                            .read(cx)
                            .check_mobile_question_file(&file)
                    })
                    .and_then(|result| result);
                if let Err(error) = result {
                    let _ = respond.send(Err(error));
                    return;
                }
                let desktop = cx.global::<DesktopState>();
                let app = Arc::clone(&desktop.app);
                desktop.runtime.spawn(async move {
                    let mut file = file;
                    let result = app
                        .stage_device_question_file(file.upload.clone(), file.bytes.take(), device)
                        .await
                        .map_err(|error| match error {
                            bruhs_core::PermissionResolveError::UnknownRequest => {
                                MobileError::Conflict(
                                    "This question has changed. Reload it on the phone.".into(),
                                )
                            }
                            bruhs_core::PermissionResolveError::Question(message) => {
                                MobileError::Invalid(message.into())
                            }
                            _ => MobileError::Unavailable,
                        });
                    let result = if let Err(error) = lease.check(MobileScope::Prompt) {
                        if result.is_ok() {
                            let _ = app
                                .stage_device_question_file(file.upload.clone(), None, device)
                                .await;
                        }
                        Err(error)
                    } else {
                        result
                    };
                    drop(file);
                    let _ = respond.send(result);
                });
            }
            MobilePending::Read {
                lease,
                read: read @ (MobileRead::Diff { .. } | MobileRead::QuestionLink(_)),
                respond,
            } => {
                let scope = read.scope();
                let source = lease
                    .with_scope(scope, || self.mobile_project_source(&read, lease.id(), cx))
                    .and_then(|result| result);
                let source = match source {
                    Ok(source) => source,
                    Err(error) => {
                        let _ = respond.send(Err(error));
                        return;
                    }
                };
                let (root, link) = source.clone();
                let request = read.clone();
                let (send, receive) = tokio::sync::oneshot::channel();
                cx.global::<DesktopState>().runtime.spawn(async move {
                    let result = match (request, link) {
                        (
                            MobileRead::Diff {
                                offset, revision, ..
                            },
                            _,
                        ) => {
                            bruhs_core::mobile::diff::read(&root, offset, revision.as_deref()).await
                        }
                        (MobileRead::QuestionLink(read), Some(link)) => {
                            bruhs_core::mobile::question_link::read(
                                root,
                                link,
                                read.offset,
                                read.revision,
                            )
                            .await
                        }
                        _ => Err(MobileError::Unavailable),
                    };
                    let _ = send.send(result);
                });
                cx.spawn(async move |this, cx| {
                    let result = receive.await.unwrap_or(Err(MobileError::Unavailable));
                    let _ = this.update(cx, |this, cx| {
                        if respond.is_closed() {
                            return;
                        }
                        let result = if !this.mobile.current(id) {
                            Err(MobileError::Disabled)
                        } else {
                            lease
                                .with_scope(scope, || {
                                    if this.mobile_project_source(&read, lease.id(), cx)? != source
                                    {
                                        return Err(MobileError::Conflict(
                                            "The thread changed while loading this page. Reload it."
                                                .into(),
                                        ));
                                    }
                                    result.map(MobileReadReply::Text)
                                })
                                .and_then(|r| r)
                        };
                        let _ = respond.send(result);
                    });
                })
                .detach();
            }
            MobilePending::Read {
                lease,
                read,
                respond,
            } => {
                let result = lease
                    .with_scope(read.scope(), || self.read_mobile(&read, lease.id(), cx))
                    .and_then(|r| r);
                let _ = respond.send(result);
            }
        }
    }
}
