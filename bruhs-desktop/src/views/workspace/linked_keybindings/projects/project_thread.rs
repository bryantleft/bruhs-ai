//! Compose on a linked checkout without resolving its path on this computer.
use super::*;
use crate::views::chat::{
    composer_ui::{
        chat_header, composer_hero_heading, composer_send_control, composer_surface,
        composer_textarea, prompt_input, CHAT_COMPOSER_FOOTER_BOTTOM_PADDING_PX,
        CHAT_COMPOSER_FOOTER_GAP_PX, CHAT_COMPOSER_FOOTER_X_PADDING_PX,
        CHAT_COMPOSER_PAGE_X_PADDING_PX,
    },
    desktop_markdown_style,
    timeline_ui::{user_message_bubble, TRANSCRIPT_MAX_WIDTH_PX},
    untrusted_markdown, SendShortcut,
};
use bruhs_models::logic::{
    composer_text::{exceeds_message_limit, MESSAGE_LIMIT_HINT},
    follow_up::ComposerAction,
};

use crate::views::model_browser::{self, Favorites};
use bruhs_models::{
    coordination::{
        linked_thread::{
            CheckoutContext, LinkedAttachment, LinkedGuestAccess, LinkedProjectReply, LinkedThread,
            LinkedThreadAction, LinkedThreadSummary, StartThread,
        },
        TaskState,
    },
    delivery::generation::{model_options, GenerationOptions, ModelOption, OptionKind},
    ids::ThreadId,
    logic::attachments::{
        admit, kind_for_name, send_blocked, upload_label, FileKind, UploadPhase, COULD_NOT_READ,
        MAX_FILES_PER_MESSAGE, MAX_FILE_BYTES, SEND_WAITS, TOO_MANY_FILES,
    },
};
use bruhs_ui::dropdown::{
    Dropdown, DropdownGroup, DropdownOption, DropdownTriggerVariant, Listbox,
};
use bruhs_ui::press::on_press_view;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

struct LinkedFileOffer {
    id: uuid::Uuid,
    name: String,
    kind: FileKind,
    declared_bytes: u64,
    phase: LinkedOfferPhase,
    bytes: Option<Vec<u8>>,
}

enum LinkedOfferPhase {
    Uploading,
    Ready(LinkedAttachment),
    Failed { message: String, retry: bool },
}

impl LinkedFileOffer {
    fn upload_phase(&self) -> UploadPhase {
        match self.phase {
            LinkedOfferPhase::Uploading => UploadPhase::Pending,
            LinkedOfferPhase::Ready(_) => UploadPhase::Ready,
            LinkedOfferPhase::Failed { .. } => UploadPhase::Failed,
        }
    }

    fn counts(&self) -> bool {
        !matches!(self.phase, LinkedOfferPhase::Failed { retry: false, .. })
    }

    fn ready(&self) -> Option<&LinkedAttachment> {
        match &self.phase {
            LinkedOfferPhase::Ready(file) => Some(file),
            _ => None,
        }
    }

    fn status_text(&self) -> Option<&str> {
        match &self.phase {
            LinkedOfferPhase::Uploading => Some("Uploading"),
            LinkedOfferPhase::Failed { message, .. } => Some(message.as_str()),
            LinkedOfferPhase::Ready(_) => None,
        }
    }

    fn can_retry(&self) -> bool {
        matches!(self.phase, LinkedOfferPhase::Failed { retry: true, .. })
    }
}

fn committed_offers(offers: &[LinkedFileOffer], skip: Option<uuid::Uuid>) -> (usize, u64) {
    let mut count = 0usize;
    let mut image_bytes = 0u64;
    for offer in offers {
        if skip.is_some_and(|id| id == offer.id) || !offer.counts() {
            continue;
        }
        count += 1;
        if offer.kind == FileKind::Image {
            image_bytes = image_bytes.saturating_add(offer.declared_bytes);
        }
    }
    (count, image_bytes)
}

fn read_offer_bytes(path: &std::path::Path) -> Result<Vec<u8>, &'static str> {
    use std::io::Read as _;
    let file = std::fs::File::open(path).map_err(|_| COULD_NOT_READ)?;
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| COULD_NOT_READ)?;
    Ok(bytes)
}

fn pasted_image_name(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Jpeg => "paste.jpg",
        ImageFormat::Gif => "paste.gif",
        ImageFormat::Webp => "paste.webp",
        ImageFormat::Png
        | ImageFormat::Svg
        | ImageFormat::Bmp
        | ImageFormat::Tiff
        | ImageFormat::Ico
        | ImageFormat::Pnm => "paste.png",
    }
}

pub(super) struct OpenThread {
    route: AppView,
    panel: Entity<Composer>,
    _discovery: gpui::Subscription,
}

#[cfg(all(test, unix))]
impl OpenThread {
    pub(super) fn draft_ready(&self, cx: &App) -> bool {
        self.panel.read(cx).checkout.is_some()
    }

    pub(super) fn send_failed(&self, cx: &App) -> bool {
        self.panel.read(cx).send_error.is_some()
    }

    pub(super) fn prompt_has_line_break(&self, cx: &App) -> bool {
        self.panel.read(cx).input.read(cx).value().contains('\n')
    }

    pub(super) fn has_thread(&self, cx: &App) -> bool {
        self.panel.read(cx).thread.is_some()
    }

    pub(super) fn attach_paths(&self, paths: &[PathBuf], cx: &mut App) {
        self.panel
            .update(cx, |panel, cx| panel.enqueue_paths(paths, cx));
    }

    pub(super) fn trigger_paste(&self, window: &mut Window, cx: &mut App) -> bool {
        self.panel
            .update(cx, |panel, cx| panel.handle_paste(window, cx))
    }

    pub(super) fn panel_handle(&self) -> Entity<Composer> {
        self.panel.clone()
    }
}

#[cfg(all(test, unix))]
impl Composer {
    pub(super) fn ready_attachment_count(&self) -> usize {
        self.ready_attachments().len()
    }
}

enum ComposerEvent {
    ThreadDiscovered,
    OpenSettings,
    OpenUsage(String),
    ToggleFavorite(String),
    Reconnect,
    Attention {
        title: String,
        reason: &'static str,
    },
    #[cfg(unix)]
    OpenTerminal(String),
}

impl EventEmitter<ComposerEvent> for Composer {}

pub(super) struct Composer {
    host: TaskEditorHost,
    id: ThreadId,
    draft: Option<(String, String)>,
    project_name: Option<String>,
    checkout: Option<CheckoutContext>,
    thread: Option<LinkedThread>,
    input: Entity<TextareaState>,
    shortcut: SendShortcut,
    scroll: ScrollHandle,
    pending_reply: Option<LinkedProjectReply>,
    attempted_start: Option<(String, Vec<LinkedAttachment>)>,
    reading: bool,
    sending: bool,
    acting: bool,
    sharing: bool,
    guest_links_enabled: bool,
    models_open: bool,
    models_loading: bool,
    models: Option<bruhs_models::delivery::writer_catalog::WriterCatalog>,
    options: GenerationOptions,
    model_option_open: Option<OptionKind>,
    favorite_models: Favorites,
    files: Vec<LinkedFileOffer>,
    picking_files: bool,
    #[cfg(unix)]
    preview_port: Entity<InputState>,
    #[cfg(unix)]
    preview_open: bool,
    #[cfg(unix)]
    preview_forwarding: bool,
    load_error: Option<String>,
    send_error: Option<String>,
    notice: Option<&'static str>,
    window: gpui::AnyWindowHandle,
    _poll: Task<()>,
    _input: gpui::Subscription,
    _paste_keys: gpui::Subscription,
}

impl Workspace {
    fn linked_project_host(&self, host: uuid::Uuid) -> Option<&ProjectHost> {
        self.linked_projects.hosts.iter().find(|h| {
            h.credential == host
                && h.data.start_threads
                && h.problem.is_none()
                && !h.token.is_empty()
        })
    }

    pub(in crate::views::workspace) fn can_start_linked_project(
        &self,
        member: &project_grouping::Member,
    ) -> bool {
        !self.project_picker_for_terminal()
            && member
                .host
                .and_then(|id| self.linked_project_host(id))
                .is_some()
    }

    pub(in crate::views::workspace) fn select_linked_project(
        &mut self,
        member: &project_grouping::Member,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(host) = member.host else { return };
        if !self.can_start_linked_project(member) {
            self.open_linked_checkout(member, window, cx);
            return;
        }
        let Some(peer) = self.linked_project_host(host) else {
            return;
        };
        let peer = TaskEditorHost(Arc::new(peer.clone()));
        let thread = ThreadId::new_v4();
        let route = AppView::LinkedProjectThread {
            host,
            thread: thread.clone(),
        };
        let draft = Some((member.path.clone(), member.name.clone()));
        let notification_thread = thread.clone();
        let panel = cx.new(|cx| Composer::new(peer, thread, draft, self.send_shortcut, window, cx));
        panel.update(cx, |panel, _| {
            panel.guest_links_enabled = self.labs_flags.guest_links;
            if self.environment_sharing.share_favorite_models {
                panel.favorite_models.clone_from(&self.favorite_models);
            } else {
                panel.favorite_models.clear();
            }
        });
        self.activate_sidebar_view(route.clone(), window, cx);
        panel
            .read(cx)
            .input
            .clone()
            .update(cx, |input, cx| input.focus(window, cx));
        let discovery = cx.subscribe_in(
            &panel,
            window,
            move |ws, _, event, window, cx| match event {
                ComposerEvent::ThreadDiscovered => ws.refresh_linked_projects(false, cx),
                ComposerEvent::OpenSettings => ws.open_settings(&OpenSettings, window, cx),
                ComposerEvent::OpenUsage(environment) => {
                    ws.open_environment_usage(environment.clone(), window, cx)
                }
                ComposerEvent::ToggleFavorite(id) => ws.toggle_favorite_model(id, cx),
                ComposerEvent::Reconnect => ws.reconnect_linked_project(host, cx),
                ComposerEvent::Attention { title, reason } => {
                    ws.notify_linked_thread(host, &notification_thread, title, reason, window, cx)
                }
                #[cfg(unix)]
                ComposerEvent::OpenTerminal(path) => {
                    ws.open_linked_terminal(host, path, window, cx)
                }
            },
        );
        self.linked_projects.project_thread = Some(OpenThread {
            route,
            panel,
            _discovery: discovery,
        });
    }

    pub(in crate::views::workspace) fn reconcile_linked_project_thread(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let AppView::LinkedProjectThread { host, thread } = &self.active_view else {
            return;
        };
        let host = *host;
        let Some(peer) = self.linked_project_host(host) else {
            return;
        };
        let route = self.active_view.clone();
        let peer = TaskEditorHost(Arc::new(peer.clone()));
        if let Some(open) = self.linked_projects.project_thread.as_ref().filter(|open| {
            open.route == route
                && open.panel.read(cx).host.0.token == peer.0.token
                && open.panel.read(cx).host.0.destination() == peer.0.destination()
        }) {
            let panel = open.panel.clone();
            if panel.read(cx).shortcut != self.send_shortcut {
                panel.update(cx, |panel, _| panel.shortcut = self.send_shortcut);
            }
            if panel
                .read(cx)
                .host
                .0
                .tunnel
                .as_ref()
                .and_then(|t| t.socket.as_ref())
                != peer.0.tunnel.as_ref().and_then(|t| t.socket.as_ref())
            {
                panel.update(cx, |panel, cx| {
                    panel.host = peer;
                    panel.load(true, cx);
                });
            }
            return;
        }
        let thread = thread.clone();
        let notification_thread = thread.clone();
        let panel = cx.new(|cx| Composer::new(peer, thread, None, self.send_shortcut, window, cx));
        panel.update(cx, |panel, _| {
            panel.guest_links_enabled = self.labs_flags.guest_links;
            if self.environment_sharing.share_favorite_models {
                panel.favorite_models.clone_from(&self.favorite_models);
            } else {
                panel.favorite_models.clear();
            }
        });
        let discovery = cx.subscribe_in(
            &panel,
            window,
            move |ws, _, event, window, cx| match event {
                ComposerEvent::ThreadDiscovered => ws.refresh_linked_projects(false, cx),
                ComposerEvent::OpenSettings => ws.open_settings(&OpenSettings, window, cx),
                ComposerEvent::OpenUsage(environment) => {
                    ws.open_environment_usage(environment.clone(), window, cx)
                }
                ComposerEvent::ToggleFavorite(id) => ws.toggle_favorite_model(id, cx),
                ComposerEvent::Reconnect => ws.reconnect_linked_project(host, cx),
                ComposerEvent::Attention { title, reason } => {
                    ws.notify_linked_thread(host, &notification_thread, title, reason, window, cx)
                }
                #[cfg(unix)]
                ComposerEvent::OpenTerminal(path) => {
                    ws.open_linked_terminal(host, path, window, cx)
                }
            },
        );
        self.linked_projects.project_thread = Some(OpenThread {
            route,
            panel,
            _discovery: discovery,
        });
    }

    #[cfg(debug_assertions)]
    pub(in crate::views::workspace) fn smoke_seed_linked_thread(
        &mut self,
        conversation: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let host_id = uuid::Uuid::new_v4();
        let thread_id = ThreadId::new_v4();
        let checkout = CheckoutContext {
            path: "/srv/studio/bruhs-ai".into(),
            branch: Some("main".into()),
            model: "claude-opus-4-6".into(),
        };
        let summary = LinkedThreadSummary {
            id: thread_id.clone(),
            title: "Verify binary attachments & host preview".into(),
            checkout: checkout.clone(),
            updated_at: bruhs_models::EpochMs::now(),
        };
        let second_summary = LinkedThreadSummary {
            id: ThreadId::new_v4(),
            title: "Audit Windows toast delivery".into(),
            checkout: CheckoutContext {
                path: "/srv/studio/bruhs-ai".into(),
                branch: Some("win-notify".into()),
                model: "gpt-5.4".into(),
            },
            updated_at: bruhs_models::EpochMs::new(
                bruhs_models::EpochMs::now()
                    .as_millis()
                    .saturating_sub(14 * 60 * 1000),
            )
            .unwrap_or_else(|_| bruhs_models::EpochMs::now()),
        };
        let Ok(endpoint) = reqwest::Url::parse("https://studio.example.test") else {
            return;
        };
        let tunnel = Some(super::super::Tunnel {
            profile: uuid::Uuid::new_v4(),
            name: "Studio".into(),
            destination: Destination::Ssh {
                profile: uuid::Uuid::new_v4(),
                host: "studio.example.test".into(),
                ssh_port: 22,
                remote_port: 45000,
            },
            socket: None,
        });
        let peer = ProjectHost {
            credential: host_id,
            vault_checked: true,
            share_usage: true,
            share_usage_history: true,
            edit_usage_prices: false,
            problem: None,
            endpoint,
            token: "b".repeat(64),
            data: ProjectInventory {
                start_threads: true,
                share_threads: true,
                threads: vec![summary, second_summary],
                usage: true,
                usage_history: true,
                ..Default::default()
            },
            tunnel,
        };
        self.linked_projects
            .hosts
            .retain(|h| h.credential != host_id);
        self.linked_projects.hosts.push(peer.clone());
        let route = AppView::LinkedProjectThread {
            host: host_id,
            thread: thread_id.clone(),
        };
        let panel = cx.new(|cx| {
            let input = cx.new(|cx| prompt_input(window, cx, "Ask anything"));
            let subscription = cx.subscribe_in(&input, window, |_, _, event, _, cx| {
                if matches!(
                    event,
                    InputEvent::Change | InputEvent::Focus | InputEvent::Blur
                ) {
                    cx.notify();
                }
            });
            let ready_attachment = LinkedFileOffer {
                id: uuid::Uuid::new_v4(),
                name: "architecture-diagram.png".into(),
                kind: FileKind::Image,
                declared_bytes: 184_320,
                phase: LinkedOfferPhase::Ready(LinkedAttachment {
                    id: "linked-thread-smoke-diagram.png".into(),
                    name: "architecture-diagram.png".into(),
                    bytes: 184_320,
                }),
                bytes: None,
            };
            Composer {
                host: TaskEditorHost(Arc::new(peer)),
                id: thread_id.clone(),
                draft: (!conversation)
                    .then(|| (checkout.path.clone(), "bruhs-ai".to_owned())),
                project_name: Some("bruhs-ai".into()),
                checkout: Some(checkout.clone()),
                thread: conversation.then(|| LinkedThread {
                    id: thread_id,
                    title: "Verify binary attachments & host preview".into(),
                    checkout,
                    state: TaskState::Completed,
                    messages: vec![
                        bruhs_models::coordination::TaskMessage {
                            role: "user".into(),
                            text: "Review the attached architecture diagram and verify the SSH preview tunnel.\n\nAttached: architecture-diagram.png".into(),
                            at: bruhs_models::EpochMs::now(),
                        },
                        bruhs_models::coordination::TaskMessage {
                            role: "assistant".into(),
                            text: "Checked `architecture-diagram.png` on **Studio**.\n\n- Binary uploads write under the host's private attachment directory scoped to the grant and thread.\n- Preview forwarding reuses the existing SSH control socket and closes with the tunnel.".into(),
                            at: bruhs_models::EpochMs::now(),
                        },
                    ],
                    live_output: String::new(),
                    error: None,
                    approvals: Vec::new(),
                    options: GenerationOptions::default(),
                }),
                input,
                shortcut: self.send_shortcut,
                scroll: ScrollHandle::new(),
                pending_reply: None,
                attempted_start: None,
                reading: false,
                sending: false,
                acting: false,
                sharing: false,
                guest_links_enabled: self.labs_flags.guest_links,
                models_open: false,
                models_loading: false,
                models: None,
                options: GenerationOptions::default(),
                model_option_open: None,
                favorite_models: self.favorite_models.clone(),
                files: vec![ready_attachment],
                picking_files: false,
                #[cfg(unix)]
                preview_port: cx.new(|cx| InputState::new(window, cx).placeholder("Remote port")),
                #[cfg(unix)]
                preview_open: false,
                #[cfg(unix)]
                preview_forwarding: false,
                load_error: None,
                send_error: None,
                notice: None,
                window: window.window_handle(),
                _poll: Task::ready(()),
                _input: subscription,
                _paste_keys: cx.intercept_keystrokes(|_, _, _| {}),
            }
        });
        let discovery = cx.subscribe_in(&panel, window, |_, _, _, _, _| {});
        self.linked_projects.project_thread = Some(OpenThread {
            route: route.clone(),
            panel,
            _discovery: discovery,
        });
        self.activate_sidebar_view(route, window, cx);
    }

    fn notify_linked_thread(
        &self,
        host: uuid::Uuid,
        thread: &ThreadId,
        title: &str,
        reason: &'static str,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        let reading = matches!(&self.active_view, AppView::LinkedProjectThread { host: active_host, thread: active_thread } if *active_host == host && active_thread == thread);
        self.notify_thread_attention(
            title.to_owned(),
            reason,
            reading,
            crate::views::meetings_hub::NoticeAction::OpenLinkedThread {
                host,
                thread: thread.clone(),
            },
            window,
            cx,
        );
    }

    fn reconnect_linked_project(&mut self, host: uuid::Uuid, cx: &mut Context<'_, Self>) {
        let state = &mut self.linked_projects;
        if state.reconnecting || state.reading {
            return;
        }
        let Some(destination) = state
            .hosts
            .iter()
            .find(|peer| peer.credential == host)
            .map(ProjectHost::destination)
        else {
            return;
        };
        if !matches!(destination, Destination::Ssh { .. }) {
            self.refresh_linked_projects(true, cx);
            return;
        }
        state.reconnecting = true;
        state.error = None;
        let job = crate::app_state::spawn_tokio(cx, reconnect_project_tunnel(destination));
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                let state = &mut this.linked_projects;
                state.reconnecting = false;
                match result {
                    Ok(snapshot) => {
                        state.update_tunnels(&snapshot);
                        this.refresh_linked_projects(true, cx);
                    }
                    Err(error) => state.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    #[cfg(unix)]
    fn open_linked_terminal(
        &mut self,
        host: uuid::Uuid,
        path: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(peer) = self.linked_project_host(host) else {
            return;
        };
        let Destination::Ssh { host, ssh_port, .. } = peer.destination() else {
            return;
        };
        let title = format!("SSH · {}", peer.name());
        self.open_terminal_view(&OpenTerminalView, window, cx);
        self.terminal_panel.update(cx, |panel, cx| {
            panel.open_ssh(host, ssh_port, path.to_owned(), title, window, cx);
        });
    }

    pub(in crate::views::workspace) fn sync_linked_labs_flags(&self, cx: &mut Context<'_, Self>) {
        if let Some(thread) = &self.linked_projects.project_thread {
            let enabled = self.labs_flags.guest_links;
            thread.panel.update(cx, |panel, cx| {
                panel.guest_links_enabled = enabled;
                cx.notify();
            });
        }
    }

    pub(in crate::views::workspace) fn sync_linked_model_favorites(
        &self,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(thread) = &self.linked_projects.project_thread {
            thread.panel.update(cx, |panel, cx| {
                if self.environment_sharing.share_favorite_models {
                    panel.favorite_models.clone_from(&self.favorite_models);
                } else {
                    panel.favorite_models.clear();
                }
                cx.notify();
            });
        }
    }

    pub(in crate::views::workspace) fn linked_conversations(
        &self,
    ) -> impl Iterator<Item = (uuid::Uuid, String, &LinkedThreadSummary)> {
        self.linked_projects.hosts.iter().flat_map(|host| {
            host.data
                .threads
                .iter()
                .map(move |thread| (host.credential, host.name(), thread))
        })
    }

    pub(in crate::views::workspace) fn render_linked_conversations(
        &self,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        let query = self.sidebar_query(cx);
        let query = query.trim().to_lowercase();
        let searching = !query.is_empty();
        let now = bruhs_models::EpochMs::now();
        let matched: Vec<_> = self
            .linked_projects
            .hosts
            .iter()
            .flat_map(|host| {
                let host_id = host.credential;
                let host_name = host.name();
                host.data.threads.iter().map(move |thread| {
                    let project = host
                        .data
                        .projects
                        .iter()
                        .find(|entry| entry.path == thread.checkout.path)
                        .map(|entry| entry.title.as_deref().unwrap_or(&entry.name))
                        .or_else(|| {
                            thread
                                .checkout
                                .path
                                .rsplit(['/', '\\'])
                                .find(|part| !part.is_empty())
                        })
                        .unwrap_or("Checkout")
                        .to_owned();
                    (host_id, host_name.clone(), project, thread)
                })
            })
            .filter(|(_, host, project, thread)| {
                query.is_empty()
                    || thread.title.to_lowercase().contains(&query)
                    || host.to_lowercase().contains(&query)
                    || project.to_lowercase().contains(&query)
                    || thread.checkout.path.to_lowercase().contains(&query)
            })
            .collect();
        if matched.is_empty() {
            return None;
        }
        let count = matched.len();
        let open = searching || self.linked_projects.conversations_open;
        let header_label = format!("Linked ({count})");
        let header = div()
            .px(px(4.0))
            .flex()
            .items_center()
            .gap_1()
            .flex_shrink_0()
            .child(on_press_view(
                div()
                    .id("sidebar-linked-toggle")
                    .debug_selector(|| "sidebar-linked-toggle".into())
                    .role(Role::Button)
                    .aria_label(header_label.clone())
                    .aria_expanded(open)
                    .tab_index(0)
                    .min_h(px(theme::CONTROL_PX))
                    .min_w_0()
                    .flex_1()
                    .py_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .text_size(px(12.0))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::rambutan_500())
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .hover(|el| el.text_color(theme::rambutan_300()))
                    .child(div().flex_shrink_0().child(header_label))
                    .child(
                        div()
                            .flex_1()
                            .h(px(1.0))
                            .bg(theme::longan_800().opacity(0.8)),
                    )
                    .child(
                        svg()
                            .path(crate::assets::CHEVRON_DOWN_ICON_PATH)
                            .size(px(theme::ICON_PX))
                            .flex_shrink_0()
                            .text_color(theme::rambutan_500())
                            .when(open, |el| {
                                el.with_transformation(Transformation::rotate(Radians(
                                    std::f32::consts::PI,
                                )))
                            }),
                    ),
                cx,
                |ws, _, cx| {
                    ws.linked_projects.conversations_open = !ws.linked_projects.conversations_open;
                    cx.notify();
                },
            ))
            .child(on_press_view(
                div()
                    .id("sidebar-linked-refresh")
                    .debug_selector(|| "sidebar-linked-refresh".into())
                    .role(Role::Button)
                    .aria_label("Refresh linked conversations")
                    .tab_index(0)
                    .size(px(theme::CONTROL_SM_PX))
                    .flex()
                    .items_center()
                    .justify_center()
                    .flex_shrink_0()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .cursor_pointer()
                    .hover(|el| el.bg(theme::longan_800().opacity(0.5)))
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .child(
                        svg()
                            .path(crate::assets::REFRESH_ICON_PATH)
                            .size(px(theme::ICON_INLINE_SM_PX))
                            .text_color(theme::rambutan_400()),
                    ),
                cx,
                |ws, _, cx| ws.refresh_linked_projects(true, cx),
            ));
        let rows: Vec<_> = matched
            .into_iter()
            .map(|(host, host_name, project, thread)| {
                let route = AppView::LinkedProjectThread {
                    host,
                    thread: thread.id.clone(),
                };
                let drop_route = route.clone();
                let selected = self.active_view == route;
                let age = crate::views::sidebar_order::relative_age(thread.updated_at, now);
                let title = bruhs_models::logic::text::plain_title(&thread.title).into_owned();
                let model_label = bruhs_models::agent::model_label(&thread.checkout.model);
                let compact_model = model_label
                    .strip_prefix("Claude ")
                    .unwrap_or(&model_label)
                    .to_owned();
                let provider =
                    bruhs_models::agent::AgentProvider::for_model(&thread.checkout.model);
                on_press_view(
                    div()
                        .id(SharedString::from(format!(
                            "sidebar-linked-thread-{host}-{}",
                            thread.id
                        )))
                        .debug_selector(|| "sidebar-linked-thread".into())
                        .role(Role::Button)
                        .aria_label(format!("Open linked conversation {title} on {host_name}"))
                        .aria_selected(selected)
                        .tab_index(0)
                        .cursor_pointer()
                        .w_full()
                        .rounded(px(14.0))
                        .px(px(10.0))
                        .py(px(10.0))
                        .flex()
                        .flex_col()
                        .when(selected, |el| {
                            el.bg(theme::longan_800())
                                .border_1()
                                .border_color(gpui::white().opacity(0.10))
                        })
                        .when(!selected, |el| {
                            el.border_1()
                                .border_color(gpui::transparent_black())
                                .hover(|el| el.bg(theme::longan_800().opacity(0.40)))
                        })
                        .focus_visible(|el| el.border_color(theme::persimmon_400()))
                        .drag_over::<ExternalPaths>(|el, _, _, _| {
                            el.bg(theme::longan_800())
                                .border_color(theme::persimmon_400())
                        })
                        .on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                            if paths.paths().is_empty() {
                                return;
                            }
                            cx.stop_propagation();
                            this.activate_sidebar_view(drop_route.clone(), window, cx);
                            this.reconcile_linked_project_thread(window, cx);
                            if let Some(open) = this
                                .linked_projects
                                .project_thread
                                .as_ref()
                                .filter(|open| open.route == drop_route)
                            {
                                open.panel.update(cx, |panel, cx| {
                                    if !panel.sending {
                                        panel.enqueue_paths(paths.paths(), cx);
                                    }
                                    panel.input.update(cx, |input, cx| input.focus(window, cx));
                                });
                            }
                        }))
                        .child(
                            div()
                                .h(px(20.0))
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .text_size(px(12.0))
                                .font_family(theme::UI_SMALL_FONT_FAMILY)
                                .text_color(theme::rambutan_500())
                                .child(
                                    svg()
                                        .path(crate::assets::TOOL_GLOBE_ICON_PATH)
                                        .size(px(14.0))
                                        .flex_shrink_0()
                                        .text_color(theme::rambutan_500()),
                                )
                                .child(
                                    div()
                                        .min_w_0()
                                        .flex_1()
                                        .truncate()
                                        .child(format!("{host_name} · {project}")),
                                )
                                .child(div().flex_shrink_0().child(age)),
                        )
                        .child(
                            div()
                                .mt(px(4.0))
                                .text_size(px(theme::APP_UI_TEXT_SIZE_PX))
                                .text_color(if selected {
                                    theme::rambutan_100()
                                } else {
                                    theme::rambutan_200()
                                })
                                .truncate()
                                .child(title),
                        )
                        .child(
                            div()
                                .mt(px(4.0))
                                .h(px(16.0))
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .text_size(px(12.0))
                                .font_family(theme::UI_SMALL_FONT_FAMILY)
                                .text_color(theme::rambutan_500())
                                .child(
                                    div()
                                        .min_w_0()
                                        .flex_1()
                                        .truncate()
                                        .when_some(thread.checkout.branch.clone(), |el, branch| {
                                            el.child(branch)
                                        }),
                                )
                                .child(
                                    div()
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .gap(px(4.0))
                                        .child(crate::views::workspace::provider_mark(provider, cx))
                                        .child(div().whitespace_nowrap().child(compact_model)),
                                ),
                        ),
                    cx,
                    move |ws, window, cx| ws.activate_sidebar_view(route.clone(), window, cx),
                )
                .into_any_element()
            })
            .collect();
        Some(
            div()
                .id("sidebar-linked-conversations")
                .px(px(8.0))
                .pb(px(8.0))
                .mb(px(4.0))
                .flex_shrink_0()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .border_b_1()
                .border_color(theme::longan_800().opacity(0.6))
                .child(header)
                .when(open, |el| {
                    el.child(
                        div()
                            .id("sidebar-linked-list")
                            .role(Role::List)
                            .aria_label("Linked conversations")
                            .max_h(px(240.0))
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .children(rows),
                    )
                })
                .into_any_element(),
        )
    }

    pub(in crate::views::workspace) fn render_linked_project_thread(
        &self,
        cx: &Context<'_, Self>,
    ) -> Div {
        let frame = div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .h_full()
            .flex()
            .flex_col();
        let AppView::LinkedProjectThread { host, .. } = &self.active_view else {
            return frame;
        };
        if let Some(open) = self.linked_projects.project_thread.as_ref().filter(|open| {
            open.route == self.active_view && self.linked_project_host(*host).is_some()
        }) {
            return frame.child(open.panel.clone());
        }
        frame
            .p_4()
            .gap_3()
            .child(
                if self.linked_projects.reading || self.linked_projects.reconnecting {
                    "Connecting to the host…".into()
                } else {
                    self.linked_projects.error.clone().unwrap_or_else(|| {
                        "Conversation access is unavailable on this host.".into()
                    })
                },
            )
            .child(div().flex().child(on_press_view(
                button(
                    "project-thread-reconnect",
                    "Reconnect",
                    self.linked_projects.reading || self.linked_projects.reconnecting,
                ),
                cx,
                {
                    let host = *host;
                    move |ws, _, cx| ws.reconnect_linked_project(host, cx)
                },
            )))
    }
}

async fn reconnect_project_tunnel(destination: Destination) -> Result<RemoteHostSnapshot, String> {
    use bruhs_models::remote_hosts::{RemoteHostCommand, RemoteHostState};
    let client = crate::server_client::ServerClient::workbench_client();
    let mut snapshot = client
        .remote_hosts(None)
        .await
        .map_err(|error| error.to_string())?;
    let mut started = false;
    let mut deadline = None;
    loop {
        let peer = snapshot
            .hosts
            .iter()
            .find(|host| Destination::from_profile(&host.profile) == destination)
            .ok_or("The saved SSH profile changed. Update the link in Environments.")?;
        if destination.socket(&snapshot).is_some() {
            return Ok(snapshot);
        }
        if peer.state == RemoteHostState::Disconnecting {
            return Err("Wait for the SSH tunnel to disconnect, then reconnect.".into());
        }
        let expires = *deadline.get_or_insert_with(|| {
            tokio::time::Instant::now()
                + Duration::from_secs(u64::from(peer.profile.connect_timeout_seconds) + 2)
        });
        if !peer.state.active() {
            if started {
                return Err(peer
                    .error
                    .clone()
                    .unwrap_or_else(|| "The SSH tunnel closed before it connected.".into()));
            }
            snapshot = client
                .remote_hosts(Some(&RemoteHostCommand::Connect {
                    id: peer.profile.id,
                }))
                .await
                .map_err(|error| error.to_string())?;
            started = true;
            continue;
        }
        if tokio::time::Instant::now() >= expires {
            return Err("The SSH connection timed out. Check the profile in Environments.".into());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        snapshot = client
            .remote_hosts(None)
            .await
            .map_err(|error| error.to_string())?;
    }
}

impl Composer {
    fn new(
        host: TaskEditorHost,
        id: ThreadId,
        draft: Option<(String, String)>,
        shortcut: SendShortcut,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let input = cx.new(|cx| prompt_input(window, cx, "Ask anything"));
        let subscription = cx.subscribe_in(&input, window, |this, _, event, w, cx| match event {
            InputEvent::PressEnter { shift: false, .. } => this.send(w, cx),
            InputEvent::Change | InputEvent::Focus | InputEvent::Blur => cx.notify(),
            _ => {}
        });
        let composer = cx.weak_entity();
        let paste_keys = cx.intercept_keystrokes(move |event, window, cx| {
            let key = event.keystroke.key.as_str();
            let modifiers = event.keystroke.modifiers;
            if key == "v"
                && (modifiers.platform || modifiers.control)
                && !modifiers.alt
                && !modifiers.shift
            {
                if let Some(composer) = composer.upgrade() {
                    let handled = composer.update(cx, |this, cx| this.handle_paste(window, cx));
                    if handled {
                        window.prevent_default();
                        cx.stop_propagation();
                    }
                }
            }
        });
        let poll = cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            if this
                .update(cx, |panel, cx| {
                    if panel.draft.is_none() {
                        panel.load(false, cx);
                    }
                })
                .is_err()
            {
                break;
            }
        });
        let mut panel = Self {
            host,
            id,
            project_name: draft.as_ref().map(|(_, name)| name.clone()),
            draft,
            checkout: None,
            thread: None,
            input,
            shortcut,
            scroll: ScrollHandle::new(),
            pending_reply: None,
            attempted_start: None,
            reading: false,
            sending: false,
            acting: false,
            sharing: false,
            guest_links_enabled: true,
            models_open: false,
            models_loading: false,
            models: None,
            options: GenerationOptions::default(),
            model_option_open: None,
            favorite_models: Favorites::default(),
            files: Vec::new(),
            picking_files: false,
            #[cfg(unix)]
            preview_port: cx.new(|cx| InputState::new(window, cx).placeholder("Remote port")),
            #[cfg(unix)]
            preview_open: false,
            #[cfg(unix)]
            preview_forwarding: false,
            load_error: None,
            send_error: None,
            notice: None,
            window: window.window_handle(),
            _poll: poll,
            _input: subscription,
            _paste_keys: paste_keys,
        };
        panel.load(true, cx);
        panel
    }

    fn load(&mut self, refresh: bool, cx: &mut Context<'_, Self>) {
        if self.reading || self.sending {
            return;
        }
        self.reading = true;
        let host = self.host.clone();
        let id = self.id.clone();
        let draft = self.draft.clone();
        let check_receipt = self.attempted_start.is_some();
        let job = crate::app_state::spawn_tokio(cx, async move {
            let client = host.0.client()?;
            let thread = client.get(host.url(&["threads", id.as_str()])?);
            match draft {
                Some((path, _)) => {
                    // Recover an accepted first send when the response was lost.
                    if check_receipt {
                        let response = thread
                            .bearer_auth(&host.0.token)
                            .timeout(Duration::from_secs(45))
                            .send()
                            .await
                            .map_err(|_| format!("Could not reach {}.", host.name()))?;
                        if response.status() == reqwest::StatusCode::OK {
                            return schedules::schedule_reply::<LinkedThread>(
                                response,
                                2 * 1024 * 1024,
                            )
                            .await
                            .map(Loaded::Thread);
                        }
                        if response.status() != reqwest::StatusCode::NOT_FOUND {
                            return Err(format!(
                                "{} could not confirm this conversation ({}).",
                                host.name(),
                                response.status()
                            ));
                        }
                    }
                    let checkout: CheckoutContext = host
                        .reply(
                            client
                                .get(host.url(&["thread-checkout"])?)
                                .query(&[("path", &path)]),
                        )
                        .await?;
                    if checkout.path != path {
                        return Err("The host returned another checkout.".into());
                    }
                    Ok(Loaded::Checkout(checkout))
                }
                None => host.reply::<LinkedThread>(thread).await.map(Loaded::Thread),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.reading = false;
                match result {
                    Ok(Loaded::Checkout(checkout)) => {
                        if this
                            .checkout
                            .as_ref()
                            .is_some_and(|previous| previous.model != checkout.model)
                        {
                            this.options = GenerationOptions::default();
                        }
                        this.checkout = Some(checkout);
                        this.load_error = None;
                        if refresh {
                            this.send_error = None;
                        }
                    }
                    Ok(Loaded::Thread(thread)) if thread.id == this.id => {
                        if let Some(reason) = this.attention_reason(&thread) {
                            cx.emit(ComposerEvent::Attention {
                                title: thread.title.clone(),
                                reason,
                            });
                        }
                        if this.scroll.max_offset().y + this.scroll.offset().y <= px(2.) {
                            this.scroll.scroll_to_bottom();
                        }
                        if let Some((sent_text, sent_files)) = this.attempted_start.take() {
                            let current_text = this.input.read(cx).value().trim().to_owned();
                            let current_files = this.ready_attachments();
                            if current_text == sent_text && current_files == sent_files {
                                let input = this.input.clone();
                                let _ = this.window.update(cx, |_, window, cx| {
                                    input.update(cx, |input, cx| input.set_value("", window, cx));
                                });
                                this.files.clear();
                            }
                            this.send_error = None;
                        }
                        this.draft = None;
                        this.checkout = Some(thread.checkout.clone());
                        this.options.clone_from(&thread.options);
                        if this
                            .thread
                            .as_ref()
                            .is_none_or(|previous| previous.title != thread.title)
                        {
                            cx.emit(ComposerEvent::ThreadDiscovered);
                        }
                        this.thread = Some(thread);
                        this.load_error = None;
                    }
                    Ok(_) => {
                        this.load_error = Some("The host returned another conversation.".into())
                    }
                    Err(error) => {
                        this.load_error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn uploads_block_send(&self) -> bool {
        let phases: Vec<UploadPhase> = self
            .files
            .iter()
            .map(LinkedFileOffer::upload_phase)
            .collect();
        send_blocked(&phases)
    }

    fn ready_attachments(&self) -> Vec<LinkedAttachment> {
        self.files
            .iter()
            .filter_map(LinkedFileOffer::ready)
            .cloned()
            .collect()
    }

    fn can_send(&self) -> bool {
        !self.sending
            && !self.acting
            && !self.picking_files
            && !self.uploads_block_send()
            && self.load_error.is_none()
            && if self.draft.is_some() {
                self.checkout.is_some() && !self.reading
            } else {
                self.thread
                    .as_ref()
                    .is_some_and(|t| !t.state.active() && t.state != TaskState::Ready)
                    || self.pending_reply.is_some()
            }
    }

    fn attention_reason(&self, current: &LinkedThread) -> Option<&'static str> {
        let previous = self.thread.as_ref()?.state;
        if previous == current.state || !(previous.active() || previous == TaskState::Ready) {
            return None;
        }
        match current.state {
            TaskState::Completed => Some("Linked conversation finished"),
            TaskState::Failed | TaskState::Interrupted => {
                Some("Linked conversation needs attention")
            }
            TaskState::WaitingApproval if !current.approvals.is_empty() => {
                Some("Linked conversation needs approval")
            }
            _ => None,
        }
    }

    fn attach_files(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.sending || self.picking_files {
            return;
        }
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach files for the linked host".into()),
        });
        self.picking_files = true;
        cx.spawn_in(window, async move |this, cx| {
            let picked = picker.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.picking_files = false;
                match picked {
                    Ok(Ok(Some(paths))) => this.enqueue_paths(&paths, cx),
                    Ok(Ok(None)) => {}
                    _ => this.send_error = Some("The file picker could not be opened.".into()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn enqueue_paths(&mut self, paths: &[PathBuf], cx: &mut Context<'_, Self>) {
        for path in paths {
            self.enqueue_path(path.clone(), cx);
        }
    }

    fn enqueue_named_bytes(
        &mut self,
        raw_name: String,
        bytes: Vec<u8>,
        cx: &mut Context<'_, Self>,
    ) {
        if self.files.len() >= MAX_FILES_PER_MESSAGE {
            self.send_error = Some(TOO_MANY_FILES.into());
            cx.notify();
            return;
        }
        let label = upload_label(&raw_name);
        let name = if label.is_empty() {
            "file".to_owned()
        } else {
            label
        };
        let declared = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let (count, image_bytes) = committed_offers(&self.files, None);
        let offer_id = uuid::Uuid::new_v4();
        let kind = match admit(count, image_bytes, &name, declared) {
            Ok(kind) => kind,
            Err(message) => {
                self.files.push(LinkedFileOffer {
                    id: offer_id,
                    name,
                    kind: kind_for_name(&raw_name),
                    declared_bytes: declared,
                    phase: LinkedOfferPhase::Failed {
                        message: message.into(),
                        retry: false,
                    },
                    bytes: None,
                });
                cx.notify();
                return;
            }
        };
        self.files.push(LinkedFileOffer {
            id: offer_id,
            name: name.clone(),
            kind,
            declared_bytes: declared,
            phase: LinkedOfferPhase::Uploading,
            bytes: None,
        });
        cx.notify();
        self.upload_offer(offer_id, name, bytes, cx);
    }

    fn handle_paste(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        if self.sending || !self.input.focus_handle(cx).is_focused(window) {
            return false;
        }
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        let mut attached = false;
        for entry in item.entries() {
            match entry {
                ClipboardEntry::ExternalPaths(paths) => {
                    self.enqueue_paths(paths.paths(), cx);
                    attached = true;
                }
                ClipboardEntry::Image(image) => {
                    self.enqueue_named_bytes(
                        pasted_image_name(image.format).to_owned(),
                        image.bytes.clone(),
                        cx,
                    );
                    attached = true;
                }
                ClipboardEntry::String(_) => {}
            }
        }
        if attached {
            return true;
        }
        let Some(text) = item.text() else {
            return false;
        };
        let draft = self.input.read(cx).value().to_string();
        if matches!(
            bruhs_models::logic::composer_text::paste_outcome(&draft, &text, false),
            bruhs_models::logic::composer_text::PasteOutcome::Attach
        ) {
            let name = bruhs_models::logic::composer_text::paste_name(
                self.files.iter().map(|offer| offer.name.as_str()),
            );
            self.enqueue_named_bytes(name, text.into_bytes(), cx);
            return true;
        }
        false
    }

    fn enqueue_path(&mut self, path: PathBuf, cx: &mut Context<'_, Self>) {
        if self.files.len() >= MAX_FILES_PER_MESSAGE {
            self.send_error = Some(TOO_MANY_FILES.into());
            cx.notify();
            return;
        }
        let raw_name = path.file_name().map_or_else(
            || "file".to_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        let label = upload_label(&raw_name);
        let name = if label.is_empty() {
            "file".to_owned()
        } else {
            label
        };
        let meta = std::fs::metadata(&path).ok();
        let declared = meta.as_ref().map_or(0, |meta| meta.len());
        if meta.as_ref().is_some_and(|meta| !meta.is_file()) {
            self.files.push(LinkedFileOffer {
                id: uuid::Uuid::new_v4(),
                name,
                kind: kind_for_name(&raw_name),
                declared_bytes: declared,
                phase: LinkedOfferPhase::Failed {
                    message: "That path is not a file.".into(),
                    retry: false,
                },
                bytes: None,
            });
            cx.notify();
            return;
        }
        let (count, image_bytes) = committed_offers(&self.files, None);
        let offer_id = uuid::Uuid::new_v4();
        let kind = match admit(count, image_bytes, &name, declared) {
            Ok(kind) => kind,
            Err(message) => {
                self.files.push(LinkedFileOffer {
                    id: offer_id,
                    name,
                    kind: kind_for_name(&raw_name),
                    declared_bytes: declared,
                    phase: LinkedOfferPhase::Failed {
                        message: message.into(),
                        retry: false,
                    },
                    bytes: None,
                });
                cx.notify();
                return;
            }
        };
        self.files.push(LinkedFileOffer {
            id: offer_id,
            name: name.clone(),
            kind,
            declared_bytes: declared,
            phase: LinkedOfferPhase::Uploading,
            bytes: None,
        });
        cx.notify();
        let read = cx.background_spawn(async move { read_offer_bytes(&path) });
        cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(bytes) => this.accept_offer_bytes(offer_id, name, bytes, cx),
                Err(message) => this.fail_offer(offer_id, message.into(), false, None, cx),
            });
        })
        .detach();
    }

    fn accept_offer_bytes(
        &mut self,
        offer_id: uuid::Uuid,
        name: String,
        bytes: Vec<u8>,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.files.iter().any(|offer| offer.id == offer_id) {
            return;
        }
        let actual = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let (count, image_bytes) = committed_offers(&self.files, Some(offer_id));
        if let Err(message) = admit(count, image_bytes, &name, actual) {
            self.fail_offer(offer_id, message.into(), false, None, cx);
            return;
        }
        if let Some(offer) = self.files.iter_mut().find(|offer| offer.id == offer_id) {
            offer.declared_bytes = actual;
            offer.kind = kind_for_name(&name);
        }
        self.upload_offer(offer_id, name, bytes, cx);
    }

    fn retry_offer(&mut self, offer_id: uuid::Uuid, cx: &mut Context<'_, Self>) {
        let Some(offer) = self.files.iter_mut().find(|offer| offer.id == offer_id) else {
            return;
        };
        let Some(bytes) = offer.bytes.take() else {
            return;
        };
        let name = offer.name.clone();
        offer.phase = LinkedOfferPhase::Uploading;
        cx.notify();
        self.upload_offer(offer_id, name, bytes, cx);
    }

    fn upload_offer(
        &mut self,
        offer_id: uuid::Uuid,
        name: String,
        bytes: Vec<u8>,
        cx: &mut Context<'_, Self>,
    ) {
        let host = self.host.clone();
        let thread_id = self.id.clone();
        let job = crate::app_state::spawn_tokio(cx, async move {
            let result: Result<LinkedAttachment, (String, bool)> = async {
                let client = host.0.client().map_err(|error| (error, true))?;
                let url = host
                    .url(&["threads", thread_id.as_str(), "attachments"])
                    .map_err(|error| (error, false))?;
                let response = client
                    .post(url)
                    .query(&[("name", name.as_str())])
                    .bearer_auth(&host.0.token)
                    .timeout(Duration::from_secs(120))
                    .body(bytes.clone())
                    .send()
                    .await
                    .map_err(|_| {
                        (
                            format!("Could not upload {} to {}.", name, host.name()),
                            true,
                        )
                    })?;
                match response.status() {
                    reqwest::StatusCode::OK => schedules::schedule_reply(response, 64 * 1024)
                        .await
                        .map_err(|error| (error, true)),
                    reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
                        Err((format!("{} denied attachment access.", host.name()), false))
                    }
                    reqwest::StatusCode::PAYLOAD_TOO_LARGE => Err((
                        bruhs_models::logic::attachments::FILE_TOO_LARGE.into(),
                        false,
                    )),
                    reqwest::StatusCode::BAD_REQUEST
                    | reqwest::StatusCode::CONFLICT
                    | reqwest::StatusCode::NOT_FOUND => {
                        #[derive(serde::Deserialize)]
                        struct Problem {
                            error: String,
                        }
                        let message = schedules::schedule_reply::<Problem>(response, 8192)
                            .await
                            .map_or_else(
                                |_| format!("{} rejected {name}.", host.name()),
                                |problem| problem.error,
                            );
                        Err((message, false))
                    }
                    status => Err((
                        format!("{} could not store {name} ({status}).", host.name()),
                        true,
                    )),
                }
            }
            .await;
            match result {
                Ok(uploaded) => Ok(uploaded),
                Err((message, retry)) => Err((message, retry, retry.then_some(bytes))),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(uploaded) => {
                    if let Some(offer) = this.files.iter_mut().find(|offer| offer.id == offer_id) {
                        offer.phase = LinkedOfferPhase::Ready(uploaded);
                        offer.bytes = None;
                        cx.notify();
                    }
                }
                Err((message, retry, bytes)) => {
                    this.fail_offer(offer_id, message, retry, bytes, cx);
                }
            });
        })
        .detach();
    }

    fn fail_offer(
        &mut self,
        offer_id: uuid::Uuid,
        message: String,
        retry: bool,
        bytes: Option<Vec<u8>>,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(offer) = self.files.iter_mut().find(|offer| offer.id == offer_id) else {
            return;
        };
        offer.bytes = bytes;
        offer.phase = LinkedOfferPhase::Failed { message, retry };
        cx.notify();
    }

    fn running(&self) -> bool {
        self.thread
            .as_ref()
            .is_some_and(|thread| thread.state.active() || thread.state == TaskState::Ready)
    }

    fn can_select_model(&self) -> bool {
        self.checkout.is_some()
            && self.load_error.is_none()
            && !self.running()
            && !self.acting
            && !self.sending
    }

    fn model_picker(&self, checkout: &CheckoutContext, cx: &Context<'_, Self>) -> Dropdown {
        const ID: &str = "linked-model-picker";
        let mut favorites = Vec::new();
        let mut other = Vec::new();
        for model in self.models.iter().flat_map(|catalog| &catalog.models) {
            let option = DropdownOption {
                value: model.id.clone().into(),
                label: model.label.clone(),
                description: Some(model.id.clone()),
            };
            if self
                .favorite_models
                .contains(&model_browser::favorite_key(&model.id))
            {
                favorites.push(option);
            } else {
                other.push(option);
            }
        }
        let groups = [
            (Some(SharedString::from("Favorites")), favorites),
            (None, other),
        ]
        .into_iter()
        .filter(|(_, options)| !options.is_empty())
        .map(|(label, options)| DropdownGroup {
            label,
            mark: None,
            options,
        })
        .collect();
        let toggle = cx.weak_entity();
        let select = cx.weak_entity();
        let favorite = cx.weak_entity();
        let starred = self.favorite_models.clone();
        Dropdown::new(
            ID,
            "Host model",
            &checkout.model,
            groups,
            if self.models_open {
                Listbox::Popover
            } else {
                Listbox::Closed
            },
            Rc::new(move |_, cx| {
                let _ = toggle.update(cx, |panel, cx| panel.toggle_models(cx));
            }),
            Rc::new(move |model, _, cx| {
                let _ = select.update(cx, |panel, cx| {
                    panel.models_open = false;
                    if !panel.can_select_model() {
                        cx.notify();
                        return;
                    }
                    if panel.draft.is_none() {
                        panel.act(
                            LinkedThreadAction::SetModel {
                                model: model.to_owned(),
                            },
                            cx,
                        );
                    } else if let Some(checkout) = &mut panel.checkout {
                        if checkout.model != model {
                            panel.options = GenerationOptions::default();
                        }
                        checkout.model = model.to_owned();
                    }
                    cx.notify();
                });
            }),
        )
        .selection_label(bruhs_models::agent::model_label(&checkout.model), None)
        .compact()
        .trigger_variant(DropdownTriggerVariant::Toolbar)
        .end_aligned_popover(px(280.))
        .empty_label(if self.models_loading {
            "Loading host models…"
        } else {
            "No models are available on this host."
        })
        .option_mark(|model| {
            crate::views::workspace::provider_glyph(bruhs_models::agent::AgentProvider::for_model(
                model,
            ))
            .into_any_element()
        })
        .option_accessories(None, move |model| {
            let id = model.to_owned();
            let favorite = favorite.clone();
            Some(model_browser::star(
                ID,
                model,
                starred.contains(&model_browser::favorite_key(model)),
                None,
                move |_, cx| {
                    let _ = favorite.update(cx, |_, cx| {
                        cx.emit(ComposerEvent::ToggleFavorite(id.clone()))
                    });
                },
            ))
        })
    }

    fn model_option_picker(
        &self,
        model: String,
        option: ModelOption,
        cx: &Context<'_, Self>,
    ) -> Dropdown {
        let toggle = cx.weak_entity();
        let select = cx.weak_entity();
        let kind = option.kind;
        Dropdown::new(
            kind.key(),
            option.label,
            self.options.effective(&option),
            vec![DropdownGroup {
                label: None,
                mark: None,
                options: option
                    .choices
                    .iter()
                    .map(|&(value, label)| DropdownOption {
                        value: value.into(),
                        label: label.into(),
                        description: None,
                    })
                    .collect(),
            }],
            if self.model_option_open == Some(kind) {
                Listbox::Popover
            } else {
                Listbox::Closed
            },
            Rc::new(move |_, cx| {
                let _ = toggle.update(cx, |panel, cx| {
                    panel.models_open = false;
                    panel.model_option_open =
                        (panel.model_option_open != Some(kind)).then_some(kind);
                    cx.notify();
                });
            }),
            Rc::new(move |value, _, cx| {
                let _ = select.update(cx, |panel, cx| {
                    panel.model_option_open = None;
                    if panel.can_select_model()
                        && panel
                            .checkout
                            .as_ref()
                            .is_some_and(|checkout| checkout.model == model)
                    {
                        if panel.draft.is_some() {
                            panel.options.set(&option, value);
                        } else {
                            panel.act(
                                LinkedThreadAction::SetModelOption {
                                    model: model.clone(),
                                    kind,
                                    value: value.to_owned(),
                                },
                                cx,
                            );
                        }
                    }
                    cx.notify();
                });
            }),
        )
        .compact()
        .trigger_variant(DropdownTriggerVariant::Toolbar)
    }

    fn toggle_models(&mut self, cx: &mut Context<'_, Self>) {
        self.models_open = !self.models_open;
        self.model_option_open = None;
        cx.notify();
        if !self.models_open || self.models_loading || self.models.is_some() {
            return;
        }
        let Some(checkout) = &self.checkout else {
            return;
        };
        let path = checkout.path.clone();
        self.models_loading = true;
        let host = self.host.clone();
        let job = crate::app_state::spawn_tokio(cx, async move {
            let request = host
                .0
                .client()?
                .get(host.url(&["thread-models"])?)
                .query(&[("path", path)]);
            host.reply::<bruhs_models::delivery::writer_catalog::WriterCatalog>(request)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.models_loading = false;
                match result {
                    Ok(models) => this.models = Some(models),
                    Err(error) => {
                        this.models_open = false;
                        this.send_error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn act(&mut self, action: LinkedThreadAction, cx: &mut Context<'_, Self>) {
        if self.acting {
            return;
        }
        self.acting = true;
        self.send_error = None;
        let host = self.host.clone();
        let id = self.id.clone();
        let job = crate::app_state::spawn_tokio(cx, async move {
            let request = host
                .0
                .client()?
                .post(host.url(&["threads", id.as_str(), "actions"])?)
                .json(&action);
            let receipt: ThreadId = host.reply(request).await?;
            if receipt == id {
                Ok(())
            } else {
                Err("The host returned a different conversation.".into())
            }
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.acting = false;
                this.send_error = result.err();
                this.load(false, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn copy_guest_link(&mut self, cx: &mut Context<'_, Self>) {
        if self.sharing || self.thread.is_none() || !self.can_share() {
            return;
        }
        self.sharing = true;
        self.notice = None;
        let host = self.host.clone();
        let id = self.id.clone();
        let job = crate::app_state::spawn_tokio(cx, async move {
            let request =
                host.0
                    .client()?
                    .post(host.url(&["threads", id.as_str(), "guest-link"])?);
            let guest: LinkedGuestAccess = host.reply(request).await?;
            if guest.token.len() != 64 || !guest.token.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err("The host returned an invalid guest link.".into());
            }
            let mut link = host.0.endpoint.clone();
            link.set_fragment(Some(&format!("guest={}", guest.token)));
            Ok(link.to_string())
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.sharing = false;
                match result {
                    Ok(link) => {
                        cx.write_to_clipboard(ClipboardItem::new_string(link));
                        this.notice = Some("Guest link copied. It expires within an hour.");
                    }
                    Err(error) => this.send_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    #[cfg(unix)]
    fn forward_preview(&mut self, cx: &mut Context<'_, Self>) {
        if self.preview_forwarding {
            return;
        }
        let Destination::Ssh {
            profile,
            host,
            ssh_port,
            remote_port,
        } = self.host.0.destination()
        else {
            return;
        };
        let port = self.preview_port.read(cx).value().trim().parse::<u16>();
        let Ok(preview_port) = port else {
            self.send_error = Some("Enter the remote preview port from 1 to 65535.".into());
            cx.notify();
            return;
        };
        self.preview_forwarding = true;
        self.send_error = None;
        let job = crate::app_state::spawn_tokio(cx, async move {
            crate::server_client::ServerClient::workbench_client()
                .forward_remote_preview(bruhs_models::remote_hosts::ForwardPreview {
                    id: profile,
                    host,
                    ssh_port,
                    task_port: remote_port,
                    preview_port,
                })
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.preview_forwarding = false;
                match result {
                    Ok(forward) => {
                        this.preview_open = false;
                        cx.open_url(&format!("http://127.0.0.1:{}/", forward.local_port));
                    }
                    Err(error) => {
                        this.send_error = Some(
                            error
                                .conflict_message()
                                .unwrap_or_else(|| error.to_string()),
                        )
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn can_share(&self) -> bool {
        self.host.0.data.share_threads && self.host.0.endpoint.scheme() == "https"
    }

    fn slash_command(
        &mut self,
        message: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        const HELP: &str = "Linked chat commands: /help, /model, /settings, /stop, /usage.";
        match message {
            "/help" => self.notice = Some(HELP),
            "/model" if self.can_select_model() => {
                if !self.models_open {
                    self.toggle_models(cx);
                }
                self.notice = None;
            }
            "/model" => {
                self.notice = Some("Wait for the checkout or current run before changing models.");
            }
            "/settings" => cx.emit(ComposerEvent::OpenSettings),
            "/usage" if self.host.0.share_usage => {
                cx.emit(ComposerEvent::OpenUsage(self.host.name()))
            }
            "/usage" => self.notice = Some("This host has not shared its provider usage."),
            "/stop" => {
                if self.running() {
                    self.act(LinkedThreadAction::Stop, cx);
                } else {
                    self.notice = Some("There is no active run to stop.");
                }
            }
            _ => return false,
        }
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
        true
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let draft = self.input.read(cx).value().trim().to_owned();
        let attachments = self.ready_attachments();
        if self.slash_command(&draft, window, cx)
            || !self.can_send()
            || (draft.is_empty() && attachments.is_empty())
            || exceeds_message_limit(&draft)
        {
            return;
        }
        let Some(checkout) = self.checkout.clone() else {
            return;
        };
        let first = self.draft.is_some();
        let reply = (!first).then(|| {
            self.pending_reply
                .as_ref()
                .filter(|reply| reply.message == draft && reply.attachments == attachments)
                .cloned()
                .unwrap_or_else(|| LinkedProjectReply {
                    request_id: uuid::Uuid::new_v4(),
                    message: draft.clone(),
                    attachments: attachments.clone(),
                })
        });
        if first {
            self.attempted_start = Some((draft.clone(), attachments.clone()));
        }
        self.pending_reply = reply.clone();
        self.sending = true;
        self.models_open = false;
        self.model_option_open = None;
        self.notice = None;
        self.send_error = None;
        let host = self.host.clone();
        let id = self.id.clone();
        let sent_text = draft.clone();
        let sent_attachments = attachments.clone();
        let options = self.options.clone();
        let job = crate::app_state::spawn_tokio(cx, async move {
            let client = host.0.client()?;
            let url = host.url(&["threads", id.as_str()])?;
            if let Some(reply) = reply {
                let receipt: uuid::Uuid = host.reply(client.post(url).json(&reply)).await?;
                if receipt == reply.request_id {
                    return Ok(());
                }
            } else {
                let receipt: ThreadId = host
                    .reply(client.put(url).json(&StartThread {
                        checkout,
                        message: draft,
                        options,
                        attachments,
                    }))
                    .await?;
                if receipt == id {
                    return Ok(());
                }
            }
            Err("The host returned a different send receipt.".into())
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.sending = false;
                match result {
                    Ok(()) => {
                        this.draft = None;
                        this.attempted_start = None;
                        this.pending_reply = None;
                        if let Some(thread) = &mut this.thread {
                            thread.state = TaskState::Ready;
                        }
                        if this.input.read(cx).value().trim() == sent_text
                            && this.ready_attachments() == sent_attachments
                        {
                            this.input
                                .update(cx, |input, cx| input.set_value("", window, cx));
                            this.files.clear();
                        }
                        this.load(false, cx);
                    }
                    Err(error) => this.send_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

enum Loaded {
    Checkout(CheckoutContext),
    Thread(LinkedThread),
}

impl Render for Composer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let title = self
            .thread
            .as_ref()
            .map(|t| t.title.as_str())
            .or_else(|| self.draft.as_ref().map(|(_, name)| name.as_str()))
            .unwrap_or("Linked conversation");
        let destination = div()
            .id("project-thread-destination")
            .debug_selector(|| "project-thread-destination".into())
            .flex()
            .flex_wrap()
            .items_center()
            .min_w_0()
            .flex_1()
            .gap_2()
            .text_sm()
            .text_color(theme::rambutan_400())
            .child(self.host.name())
            .when_some(self.checkout.as_ref(), |el, checkout| {
                let label = bruhs_models::agent::model_label(&checkout.model);
                let provider = bruhs_models::agent::AgentProvider::for_model(&checkout.model);
                let el = el
                    .child(div().text_color(theme::rambutan_500()).child("·"))
                    .child(
                        checkout
                            .branch
                            .as_deref()
                            .unwrap_or("No active branch")
                            .to_owned(),
                    );
                let model = div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(crate::views::workspace::provider_mark(provider, cx))
                    .child(label);
                if !self.can_select_model() {
                    return el.child(model);
                }
                el.child(self.model_picker(checkout, cx)).children(
                    model_options(&checkout.model)
                        .into_iter()
                        .map(|option| self.model_option_picker(checkout.model.clone(), option, cx)),
                )
            });

        let mut history = div()
            .id("project-thread-history")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .w_full()
            .max_w(px(TRANSCRIPT_MAX_WIDTH_PX))
            .mx_auto()
            .pt_4()
            .flex()
            .flex_col()
            .gap_3();
        if let Some(thread) = &self.thread {
            history = history
                .children(thread.messages.iter().enumerate().map(|(index, message)| {
                    if message.role == "user" {
                        div()
                            .w_full()
                            .flex()
                            .flex_col()
                            .items_end()
                            .child(user_message_bubble(cx).child(message.text.clone()))
                    } else {
                        div().w_full().child(
                            untrusted_markdown(
                                SharedString::from(format!("linked-reply-{}-{index}", thread.id)),
                                message.text.clone(),
                            )
                            .style(desktop_markdown_style(cx)),
                        )
                    }
                }))
                .when(!thread.live_output.is_empty(), |el| {
                    el.child(
                        untrusted_markdown("linked-live-reply", thread.live_output.clone())
                            .style(desktop_markdown_style(cx)),
                    )
                })
                .when(
                    thread.state == TaskState::WaitingApproval && thread.approvals.is_empty(),
                    |el| el.child("Waiting for approval on the host."),
                )
                .children(thread.approvals.iter().map(|approval| {
                    let allow = approval.id.clone();
                    let deny = approval.id.clone();
                    div()
                        .id(SharedString::from(format!(
                            "linked-approval-{}",
                            approval.id
                        )))
                        .role(Role::Alert)
                        .flex()
                        .flex_col()
                        .gap_2()
                        .p_3()
                        .border_1()
                        .border_color(theme::longan_800())
                        .rounded_md()
                        .child(format!(
                            "Approve {} on {}?",
                            approval.tool,
                            self.host.name()
                        ))
                        .child(
                            div()
                                .max_h(px(160.))
                                .id("approval-details")
                                .overflow_y_scroll()
                                .text_xs()
                                .child(
                                    serde_json::to_string_pretty(&approval.input)
                                        .unwrap_or_default(),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(on_press_view(
                                    button("linked-approval-deny", "Deny", self.acting),
                                    cx,
                                    move |panel, _, cx| {
                                        panel.act(
                                            LinkedThreadAction::Approval {
                                                request_id: deny.clone(),
                                                allow: false,
                                            },
                                            cx,
                                        )
                                    },
                                ))
                                .child(on_press_view(
                                    button("linked-approval-allow", "Allow once", self.acting),
                                    cx,
                                    move |panel, _, cx| {
                                        panel.act(
                                            LinkedThreadAction::Approval {
                                                request_id: allow.clone(),
                                                allow: true,
                                            },
                                            cx,
                                        )
                                    },
                                )),
                        )
                }))
                .when(
                    thread.state == TaskState::Ready || thread.state == TaskState::Running,
                    |el| el.child("Working on the host…"),
                )
                .when_some(thread.error.clone(), |el, error| {
                    el.child(
                        div()
                            .id("project-thread-run-error")
                            .role(Role::Alert)
                            .child(error),
                    )
                });
        }
        let draft = self.input.read(cx).value();
        let over_limit = exceeds_message_limit(draft.as_ref());
        let has_ready_file = self.files.iter().any(|offer| offer.ready().is_some());
        let uploads_wait = self.uploads_block_send();
        let hero = self.draft.is_some();
        let running = self.running();

        let focused = self.input.focus_handle(cx).is_focused(window);
        let enabled =
            self.can_send() && (!draft.trim().is_empty() || has_ready_file) && !over_limit;
        let input = div()
            .key_context("ChatComposer")
            .capture_action(cx.listener(|this, _: &bruhs_ui::input::Paste, window, cx| {
                if this.handle_paste(window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(
                cx.listener(|this, action: &bruhs_ui::input::Enter, window, cx| {
                    if !this.input.focus_handle(cx).is_focused(window) || action.shift {
                        return;
                    }
                    cx.stop_propagation();
                    if !action.secondary
                        && this
                            .shortcut
                            .requires_modifier(this.input.read(cx).value().as_ref())
                    {
                        this.input
                            .update(cx, |field, cx| field.insert_line_break(window, cx));
                    } else {
                        this.send(window, cx);
                    }
                }),
            )
            .child(composer_textarea(&self.input, "Message this checkout"));
        let composer = div()
            .w_full()
            .max_w(px(TRANSCRIPT_MAX_WIDTH_PX))
            .mx_auto()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .when(hero, |el| {
                el.child(
                    composer_hero_heading().text_center().child(
                        div()
                            .min_w_0()
                            .child(format!("What should we build in {title}?")),
                    ),
                )
            })
            .when(self.reading && self.checkout.is_none(), |el| {
                el.child(
                    div()
                        .text_sm()
                        .text_color(theme::rambutan_400())
                        .child("Checking the host checkout…"),
                )
            })
            .when_some(self.notice, |el, notice| {
                el.child(
                    div()
                        .text_sm()
                        .text_color(theme::rambutan_400())
                        .py_2()
                        .child(notice),
                )
            })
            .when_some(
                self.load_error.clone().or_else(|| self.send_error.clone()),
                |el, error| {
                    el.child(
                        div()
                            .id("project-thread-request-error")
                            .debug_selector(|| "project-thread-request-error".into())
                            .role(Role::Alert)
                            .text_sm()
                            .child(error),
                    )
                    .child(div().flex().child(on_press_view(
                        button(
                            "project-thread-refresh",
                            if hero {
                                "Refresh checkout"
                            } else {
                                "Refresh conversation"
                            },
                            self.reading || self.sending,
                        ),
                        cx,
                        |this, _, cx| {
                            this.load(true, cx);
                            if this.host.0.tunnel.is_some() {
                                cx.emit(ComposerEvent::Reconnect);
                            }
                        },
                    )))
                },
            )
            .child(
                composer_surface(focused)
                    .drag_over::<ExternalPaths>(|el, _, _, _| {
                        el.border_color(theme::persimmon_400())
                            .bg(theme::longan_800())
                    })
                    .on_drop(cx.listener(|this, paths: &ExternalPaths, _window, cx| {
                        if !this.sending {
                            this.enqueue_paths(paths.paths(), cx);
                        }
                    }))
                    .when(!self.files.is_empty(), |el| {
                        el.child(
                            div()
                                .id("linked-file-offers")
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .px_3()
                                .pt_2p5()
                                .pb_2()
                                .gap_1p5()
                                .border_b_1()
                                .border_color(theme::longan_800())
                                .children(self.files.iter().enumerate().map(|(index, file)| {
                                    let offer_id = file.id;
                                    let status = file.status_text().map(str::to_owned);
                                    let failed =
                                        matches!(file.phase, LinkedOfferPhase::Failed { .. });
                                    let retry = file.can_retry();
                                    let size_label = (file.declared_bytes > 0).then(|| {
                                        bruhs_models::backup::format_bytes(file.declared_bytes)
                                    });
                                    let kind_icon = match file.kind {
                                        FileKind::Image => crate::assets::EYE_ICON_PATH,
                                        FileKind::Text | FileKind::Pdf | FileKind::Html => {
                                            crate::assets::FILE_TEXT_ICON_PATH
                                        }
                                        FileKind::Audio => crate::assets::MIC_ICON_PATH,
                                        FileKind::Video | FileKind::Other => {
                                            crate::assets::TOOL_FILE_ICON_PATH
                                        }
                                    };
                                    div()
                                        .flex()
                                        .min_h(px(theme::CONTROL_PX))
                                        .items_center()
                                        .gap_1p5()
                                        .max_w(px(300.))
                                        .rounded(px(6.))
                                        .px_2()
                                        .bg(if failed {
                                            theme::lychee_500().opacity(0.10)
                                        } else {
                                            theme::longan_800().opacity(0.75)
                                        })
                                        .border_1()
                                        .border_color(if failed {
                                            theme::lychee_400().opacity(0.25)
                                        } else {
                                            gpui::white().opacity(0.10)
                                        })
                                        .text_xs()
                                        .child(
                                            svg()
                                                .path(kind_icon)
                                                .size(px(14.))
                                                .flex_shrink_0()
                                                .text_color(if failed {
                                                    theme::lychee_300()
                                                } else {
                                                    theme::rambutan_400()
                                                }),
                                        )
                                        .child(
                                            div()
                                                .min_w_0()
                                                .truncate()
                                                .text_color(theme::rambutan_100())
                                                .child(file.name.clone()),
                                        )
                                        .when_some(size_label, |chip, size_label| {
                                            chip.child(
                                                div()
                                                    .flex_shrink_0()
                                                    .text_color(theme::rambutan_400())
                                                    .child(size_label),
                                            )
                                        })
                                        .when_some(status, |chip, status| {
                                            chip.child(
                                                div()
                                                    .flex_shrink_0()
                                                    .text_color(if failed {
                                                        theme::lychee_300()
                                                    } else {
                                                        theme::rambutan_400()
                                                    })
                                                    .child(status),
                                            )
                                        })
                                        .when(retry, |chip| {
                                            chip.child(on_press_view(
                                                div()
                                                    .id(("linked-retry-file", index))
                                                    .role(Role::Button)
                                                    .aria_label(format!("Retry {}", file.name))
                                                    .tab_index(0)
                                                    .cursor_pointer()
                                                    .text_color(theme::persimmon_300())
                                                    .child("Retry"),
                                                cx,
                                                move |panel, _, cx| panel.retry_offer(offer_id, cx),
                                            ))
                                        })
                                        .child(on_press_view(
                                            div()
                                                .id(("linked-remove-file", index))
                                                .aria_label(format!("Remove {}", file.name))
                                                .role(Role::Button)
                                                .tab_index(0)
                                                .cursor_pointer()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .size(px(18.))
                                                .rounded(px(4.))
                                                .hover(|el| el.bg(theme::longan_700()))
                                                .child(
                                                    svg()
                                                        .path(bruhs_ui::icons::CLOSE)
                                                        .size(px(12.))
                                                        .text_color(theme::rambutan_400()),
                                                ),
                                            cx,
                                            move |panel, _, cx| {
                                                if !panel.sending {
                                                    panel
                                                        .files
                                                        .retain(|offer| offer.id != offer_id);
                                                    cx.notify();
                                                }
                                            },
                                        ))
                                })),
                        )
                    })
                    .child(input)
                    .when(uploads_wait, |el| {
                        el.child(
                            div()
                                .px_3()
                                .text_xs()
                                .text_color(theme::rambutan_400())
                                .child(SEND_WAITS),
                        )
                    })
                    .when(over_limit, |el| {
                        el.child(
                            div()
                                .px_3()
                                .text_sm()
                                .text_color(theme::lychee_300())
                                .child(MESSAGE_LIMIT_HINT),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px(px(CHAT_COMPOSER_FOOTER_X_PADDING_PX))
                            .pb(px(CHAT_COMPOSER_FOOTER_BOTTOM_PADDING_PX))
                            .gap(px(CHAT_COMPOSER_FOOTER_GAP_PX))
                            .child(on_press_view(
                                div()
                                    .id("linked-attach-files")
                                    .debug_selector(|| "linked-attach-files".into())
                                    .role(Role::Button)
                                    .aria_label("Attach files")
                                    .aria_description(
                                        "Images up to 10 MiB and other files up to 50 MiB",
                                    )
                                    .tab_index(0)
                                    .cursor_pointer()
                                    .size(px(theme::CONTROL_SM_PX))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_md()
                                    .hover(|el| el.bg(theme::longan_800()))
                                    .focus_visible(|el| el.bg(theme::longan_800()))
                                    .child(
                                        svg()
                                            .path(bruhs_ui::icons::PAPERCLIP)
                                            .size(px(theme::ICON_PX))
                                            .text_color(theme::rambutan_400()),
                                    ),
                                cx,
                                |panel, window, cx| panel.attach_files(window, cx),
                            ))
                            .child(destination)
                            .child(composer_send_control(
                                "project-thread-send",
                                "project-thread-send",
                                if running {
                                    ComposerAction::Interrupt
                                } else {
                                    ComposerAction::Send
                                },
                                enabled || (running && !self.acting && self.load_error.is_none()),
                                cx,
                                |this, w, cx| {
                                    if this.running() {
                                        this.act(LinkedThreadAction::Stop, cx);
                                    } else {
                                        this.send(w, cx);
                                    }
                                },
                            )),
                    ),
            );
        let project = self.project_name.as_deref().or_else(|| {
            self.checkout
                .as_ref()
                .and_then(|c| c.path.rsplit(['/', '\\']).find(|part| !part.is_empty()))
        });
        let show_share = self.guest_links_enabled && self.can_share() && self.thread.is_some();
        #[cfg(unix)]
        let tunnel_path = self
            .checkout
            .as_ref()
            .filter(|_| self.host.0.tunnel.is_some())
            .map(|checkout| checkout.path.clone());
        #[cfg(not(unix))]
        let tunnel_path: Option<String> = None;
        let show_actions = show_share || tunnel_path.is_some();
        let header = chat_header(cx)
            .when_some(project, |el, name| {
                el.child(
                    svg()
                        .path(crate::assets::FOLDER_ICON_PATH)
                        .size(px(theme::ICON_PX))
                        .flex_shrink_0()
                        .text_color(theme::rambutan_400()),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(theme::rambutan_400())
                        .child(name.to_owned()),
                )
                .child(div().text_color(theme::rambutan_500()).child("/"))
            })
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(theme::rambutan_100())
                    .child(if hero {
                        "New thread".into()
                    } else {
                        title.to_owned()
                    }),
            )
            .when(show_actions, |header| {
                let actions = div()
                    .ml_auto()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(show_share, |actions| {
                        actions.child(on_press_view(
                            button("linked-copy-guest-link", "Copy guest link", self.sharing),
                            cx,
                            |panel, _, cx| panel.copy_guest_link(cx),
                        ))
                    });
                #[cfg(unix)]
                let actions = actions.when_some(tunnel_path, |actions, path| {
                    actions
                        .child(on_press_view(
                            button("linked-open-terminal", "SSH terminal", false),
                            cx,
                            move |_, _, cx| cx.emit(ComposerEvent::OpenTerminal(path.clone())),
                        ))
                        .child(on_press_view(
                            button("linked-preview-toggle", "Preview", false)
                                .aria_expanded(self.preview_open),
                            cx,
                            |panel, window, cx| {
                                panel.preview_open = !panel.preview_open;
                                if panel.preview_open {
                                    panel
                                        .preview_port
                                        .update(cx, |input, cx| input.focus(window, cx));
                                }
                                cx.notify();
                            },
                        ))
                });
                header.child(actions)
            });
        #[cfg(unix)]
        let header = div().child(header).when(self.preview_open, |el| {
            el.child(
                div()
                    .flex()
                    .justify_end()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .pb_2()
                    .child(div().w(px(132.)).child(Input::new(&self.preview_port)))
                    .child(on_press_view(
                        button(
                            "linked-preview-open",
                            "Open in browser",
                            self.preview_forwarding,
                        ),
                        cx,
                        |panel, _, cx| panel.forward_preview(cx),
                    )),
            )
        });
        div()
            .id("linked-project-thread")
            .debug_selector(|| "linked-project-thread".into())
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .px(px(CHAT_COMPOSER_PAGE_X_PADDING_PX))
                    .pb_4()
                    .when(!hero, |el| el.child(history))
                    .child(
                        div()
                            .id("linked-composer-body")
                            .when(hero, |el| {
                                el.flex_1().min_h_0().overflow_y_scroll().flex().flex_col()
                            })
                            .child(composer.when(hero, |el| el.my_auto())),
                    ),
            )
    }
}
