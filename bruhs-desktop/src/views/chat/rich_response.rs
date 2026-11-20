//! Native inline response surfaces share the transcript's clipping and lifetime.

use super::*;
use bruhs_models::logic::rich_response::{
    interactive_host_script, media_is_remote, media_url, widget_state_key, MediaBlock, MediaKind,
    ResponseBlock, WidgetAction, WidgetEvent, WidgetMessage, WidgetState, MAX_WIDGET_HEIGHT,
    MAX_WIDGET_STATE_BYTES, MIN_WIDGET_HEIGHT, WIDGET_PROTOCOL,
};
use bruhs_ui::press::on_press_view;
use bruhs_ui::zoom::px;
use std::cell::RefCell;
use std::rc::Rc;

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
mod native;
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
use native::NativeEmbed;

mod controls;
mod prose;
mod publications;
mod runtime;

type BlockKey = (usize, usize, SharedString, SharedString);
type StateWrite =
    futures_util::future::Shared<futures_util::future::BoxFuture<'static, Result<(), String>>>;

struct PlacementContext<'a> {
    window: &'a Window,
    app: &'a App,
    entity: EntityId,
    chat: WeakEntity<ChatView>,
}

struct HostWidget {
    html: Option<String>,
    nonce: String,
    state: WidgetState,
    key: BlockKey,
    theme: std::collections::BTreeMap<String, String>,
}

fn widget_theme(app: &App) -> std::collections::BTreeMap<String, String> {
    let scheme = if app.theme().is_dark() { "dark" } else { "light" };
    let css_color = |color: Hsla| format!("#{:06x}", u32::from(color.to_rgb()) >> 8);
    [
        ("color-scheme", scheme.to_owned()),
        ("--bruhs-background", css_color(app.theme().background)),
        ("--bruhs-text", css_color(theme::rambutan_300())),
        ("--bruhs-accent", css_color(theme::persimmon_400())),
        ("--bruhs-font", "ui-monospace,monospace".to_owned()),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value))
    .collect()
}

fn media_has_remote(media: &MediaBlock) -> bool {
    media_is_remote(&media.src)
        || media.poster.as_deref().is_some_and(media_is_remote)
        || media.tracks.iter().any(|track| media_is_remote(&track.src))
}

fn bound_media(
    media: &MediaBlock,
    bindings: &std::collections::BTreeMap<String, String>,
) -> MediaBlock {
    let mut media = media.clone();
    if let Some(src) = bindings.get(&media.src) {
        media.src = src.clone();
    }
    if let Some(poster) = media.poster.as_mut() {
        if let Some(src) = bindings.get(poster) {
            *poster = src.clone();
        }
    }
    for track in &mut media.tracks {
        if let Some(src) = bindings.get(&track.src) {
            track.src = src.clone();
        }
    }
    media
}

#[derive(Default)]
enum ShellStatus {
    #[default]
    Unchecked,
    Loading,
    Ready,
    Failed(String),
}

struct Entry {
    host: NativeEmbed,
    remote_allowed: bool,
    source_open: bool,
    error: Option<String>,
    body: Option<String>,
    nonce: String,
    resolved: Option<Rc<ResponseBlock>>,
    preparing: bool,
    prepare_task: Option<Task<()>>,
    state_key: String,
    state: WidgetState,
    revision: u64,
    saved_revision: u64,
    save_task: Option<Task<()>>,
    saving: bool,
    write: Option<StateWrite>,
    write_generation: u64,
    reload_pending: bool,
    reload_acked: bool,
    height: f64,
    action: Option<WidgetAction>,
    errors: Vec<String>,
    artifact_id: Option<String>,
    flushing: bool,
    flush_acked: bool,
    diagnostics_task: Option<Task<()>>,
    original_media: Option<MediaBlock>,
}

impl Default for Entry {
    fn default() -> Self {
        Self {
            host: NativeEmbed::default(),
            remote_allowed: false,
            source_open: false,
            error: None,
            body: None,
            nonce: uuid::Uuid::new_v4().to_string(),
            resolved: None,
            preparing: false,
            prepare_task: None,
            state_key: String::new(),
            state: WidgetState::default(),
            revision: 0,
            saved_revision: 0,
            save_task: None,
            saving: false,
            write: None,
            write_generation: 0,
            reload_pending: false,
            reload_acked: false,
            height: 360.0,
            action: None,
            errors: Vec::new(),
            artifact_id: None,
            flushing: false,
            flush_acked: false,
            diagnostics_task: None,
            original_media: None,
        }
    }
}

pub(super) struct RichEmbeds {
    entries: HashMap<BlockKey, Entry>,
    positions: HashMap<(usize, usize), BlockKey>,
    seen: HashSet<BlockKey>,
    workspace_shown: bool,
    chat_occluded: bool,
    shell: ShellStatus,
    shell_task: Option<Task<()>>,
    client: Option<crate::server_client::ServerClient>,
    export_state: HashSet<usize>,
    export_remote: HashSet<usize>,
    /// Replies whose "Export reply" panel is open.
    export_open: HashSet<usize>,
    quit_subscription: Option<Subscription>,
    parent_subscription: Option<Subscription>,
    parent_window: Option<WindowId>,
    expanded: Option<BlockKey>,
    expanded_focus: Option<FocusHandle>,
    previous_focus: Option<FocusHandle>,
    entered_fullscreen: bool,
    exit_fullscreen: bool,
    quitting: bool,
    prose: HashMap<(usize, usize), prose::ProseEntry>,
}

impl Default for RichEmbeds {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            positions: HashMap::new(),
            seen: HashSet::new(),
            workspace_shown: true,
            chat_occluded: false,
            shell: ShellStatus::Unchecked,
            shell_task: None,
            client: None,
            export_state: HashSet::new(),
            export_remote: HashSet::new(),
            export_open: HashSet::new(),
            quit_subscription: None,
            parent_subscription: None,
            parent_window: None,
            expanded: None,
            expanded_focus: None,
            previous_focus: None,
            entered_fullscreen: false,
            exit_fullscreen: false,
            quitting: false,
            prose: HashMap::new(),
        }
    }
}

impl RichEmbeds {
    fn track_parent(cache: &Rc<RefCell<Self>>, window: &Window, app: &App) {
        let parent = window.window_handle().window_id();
        let mut embeds = cache.borrow_mut();
        if embeds.parent_window == Some(parent) {
            return;
        }
        embeds.drop_native_hosts("parent_changed");
        embeds.parent_window = Some(parent);
        let cache = Rc::downgrade(cache);
        embeds.parent_subscription = Some(app.on_window_closed(move |app, closed| {
            let Some(cache) = cache.upgrade() else { return };
            let mut embeds = cache.borrow_mut();
            if embeds.parent_window == Some(closed) {
                // GPUI invokes this observer before dropping its Window box.
                // Destroy children while the native parent is still alive;
                // X11 recursively destroys children when its parent is dropped.
                embeds.drop_native_hosts("parent_closed");
                runtime::save_closed_widget_states(&mut embeds, app);
            }
        }));
    }

    fn drop_native_hosts(&mut self, reason: &str) {
        #[cfg(debug_assertions)]
        let live = self
            .entries
            .values()
            .filter(|entry| entry.host.is_live())
            .count();
        for entry in self.entries.values_mut() {
            entry.host.flush();
            entry.host.clear();
            entry.flushing = false;
            entry.flush_acked = false;
        }
        self.parent_window = None;
        #[cfg(debug_assertions)]
        if std::env::var("BRUHS_DESKTOP_SMOKE_SCENE")
            .is_ok_and(|scene| scene.starts_with("transcript-rich"))
        {
            eprintln!(
                "BRUHS_RICH_RESPONSE_SMOKE teardown={reason} hosts_disposed={live} hosts_remaining=0"
            );
        }
        #[cfg(not(debug_assertions))]
        let _ = reason;
    }

    pub(super) fn clear(&mut self) {
        self.exit_fullscreen |= self.entered_fullscreen;
        self.entered_fullscreen = false;
        self.expanded = None;
        self.expanded_focus = None;
        self.previous_focus = None;
        self.entries.clear();
        self.positions.clear();
        self.prose.clear();
        self.export_state.clear();
        self.export_remote.clear();
        self.export_open.clear();
        self.seen.clear();
        self.shell_task = None;
        self.shell = ShellStatus::Unchecked;
    }

    pub(super) fn truncate(&mut self, len: usize) {
        self.entries.retain(|(message, _, _, _), _| *message < len);
        self.positions.retain(|(message, _), _| *message < len);
        self.prose.retain(|(message, _), _| *message < len);
        self.export_state.retain(|message| *message < len);
        self.export_remote.retain(|message| *message < len);
        self.export_open.retain(|message| *message < len);
    }

    fn hide(&mut self) {
        for entry in self.entries.values_mut() {
            entry.host.hide();
        }
    }

    pub(super) fn set_occluded(&mut self, occluded: bool) {
        self.chat_occluded = occluded;
        if occluded {
            self.hide();
        }
    }

    fn shown(&self) -> bool {
        self.workspace_shown && !self.chat_occluded
    }

    pub(super) fn begin_frame(&mut self) {
        self.seen.clear();
    }

    pub(super) fn finish_frame(&mut self) {
        let shown = self.shown();
        for (key, entry) in &mut self.entries {
            let expanded = self.expanded.as_ref() == Some(key);
            if !shown || entry.source_open || (self.expanded.is_some() && !expanded) {
                entry.host.hide();
            } else if !expanded && !self.seen.contains(key) {
                if entry
                    .resolved
                    .as_ref()
                    .is_some_and(|block| matches!(block.as_ref(), ResponseBlock::Html(_)))
                {
                    entry.host.hide();
                    if entry.flush_acked {
                        entry.host.clear();
                        entry.body = None;
                        entry.flush_acked = false;
                    } else {
                        entry.host.flush();
                        entry.flushing = true;
                    }
                } else {
                    entry.host.clear();
                }
            }
        }
    }

    fn place(
        &mut self,
        key: &BlockKey,
        block: &ResponseBlock,
        project: &str,
        bounds: Bounds<Pixels>,
        context: PlacementContext<'_>,
    ) -> bool {
        if !self.shown() || cfg!(test) {
            return false;
        }
        if self
            .expanded
            .as_ref()
            .is_some_and(|expanded| expanded != key)
        {
            if let Some(entry) = self.entries.get_mut(key) {
                entry.host.hide();
            }
            return false;
        }
        let visible = bounds.intersect(&context.window.content_mask().bounds);
        if visible.size.width <= px(0.0) || visible.size.height <= px(0.0) {
            return false;
        }
        self.seen.insert(key.clone());
        let entry = self.entries.entry(key.clone()).or_default();
        entry.flushing = false;
        entry.flush_acked = false;
        if matches!(block, ResponseBlock::Media(media) if media_has_remote(media))
            && !entry.remote_allowed
        {
            return false;
        }
        if entry.source_open {
            entry.host.hide();
            return false;
        }
        let base = server_base();
        let resolved = entry.resolved.as_deref().unwrap_or(block);
        let body = entry
            .body
            .get_or_insert_with(|| embed_body(resolved, &base, project));
        let frame_url = match resolved {
            ResponseBlock::Media(media) if media.kind == MediaKind::Pdf => {
                Some(media_url(&base, &media.src, Some(project)))
            }
            _ => None,
        };
        let shell = if frame_url.is_some() {
            "rich-media"
        } else {
            "rich-response"
        };
        let error = entry
            .host
            .place(
                &format!("{base}/api/agent/{shell}"),
                body,
                frame_url,
                (bounds, visible),
                HostWidget {
                    html: match resolved {
                        ResponseBlock::Html(html) => Some(html.html.clone()),
                        _ => None,
                    },
                    nonce: entry.nonce.clone(),
                    state: entry.state.clone(),
                    key: key.clone(),
                    theme: widget_theme(context.app),
                },
                context,
            )
            .err();
        if error.is_some() {
            entry.host.clear();
        }
        if error == entry.error {
            return false;
        }
        entry.error = error;
        true
    }
}

fn server_base() -> String {
    std::env::var("BRUHS_SERVER_URL")
        .ok()
        .filter(|base| !base.trim().is_empty())
        .unwrap_or_else(|| crate::server_client::DEFAULT_SERVER_URL.to_string())
        .trim_end_matches('/')
        .to_string()
}

fn attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn media_title(media: &MediaBlock) -> &str {
    if !media.title.trim().is_empty() {
        return &media.title;
    }
    match media.kind {
        MediaKind::Image => "Image",
        MediaKind::Video => "Video",
        MediaKind::Audio => "Audio",
        MediaKind::Pdf => "PDF",
    }
}

fn embed_body(block: &ResponseBlock, base: &str, project: &str) -> String {
    match block {
        ResponseBlock::Html(_) => String::new(),
        ResponseBlock::Media(block) => {
            let src = attr(&media_url(base, &block.src, Some(project)));
            let title = attr(media_title(block));
            let poster = block.poster.as_ref().map_or_else(String::new, |poster| {
                format!(
                    " poster=\"{}\"",
                    attr(&media_url(base, poster, Some(project)))
                )
            });
            let tracks = block
                .tracks
                .iter()
                .enumerate()
                .map(|(index, track)| {
                    format!(
                        "<track src=\"{}\" label=\"{}\" srclang=\"{}\" kind=\"{}\" {}>",
                        attr(&media_url(base, &track.src, Some(project))),
                        attr(&track.label),
                        attr(&track.language),
                        attr(&track.kind),
                        if index == 0 { "default" } else { "" }
                    )
                })
                .collect::<String>();
            let cors = if block.tracks.is_empty() {
                ""
            } else {
                " crossorigin=\"anonymous\""
            };
            match block.kind {
                MediaKind::Image => format!(
                    "<style>.bruhs-image:fullscreen{{width:100vw!important;height:100vh!important;background:#141210}}.bruhs-image button{{font:inherit;color:inherit;background:#2f2922;border:1px solid #594a36;padding:4px 8px}}</style><div class=\"bruhs-image\" style=\"position:relative;width:100%;height:100%;color:#dfcfc3\"><div style=\"position:absolute;right:8px;top:8px;z-index:2\"><button onclick=\"const i=this.closest('.bruhs-image').querySelector('img');const zoom=i.dataset.zoom!=='true';i.dataset.zoom=String(zoom);i.style.cssText=zoom?'width:auto;height:auto;max-width:none':'width:100%;height:100%;object-fit:contain'\">Zoom</button><button onclick=\"this.closest('.bruhs-image').requestFullscreen().catch(()=>this.textContent='Fullscreen unavailable')\">Fullscreen</button></div><div style=\"width:100%;height:100%;overflow:auto\"><img src=\"{src}\" alt=\"{title}\" referrerpolicy=\"no-referrer\" style=\"width:100%;height:100%;object-fit:contain\"></div></div>"
                ),
                MediaKind::Video => format!(
                    "<video controls preload=\"none\"{cors} playsinline src=\"{src}\"{poster} aria-label=\"{title}\" style=\"width:100%;height:100%\">{tracks}</video>"
                ),
                MediaKind::Audio => format!(
                    "<audio controls preload=\"none\"{cors} src=\"{src}\" aria-label=\"{title}\" style=\"width:100%\">{tracks}</audio>"
                ),
                MediaKind::Pdf => format!(
                    "<iframe sandbox=\"\" src=\"{src}\" title=\"{title}\" referrerpolicy=\"no-referrer\" style=\"border:0;width:100%;height:100%\"></iframe>"
                ),
            }
        }
        ResponseBlock::Markdown(_) | ResponseBlock::Artifact(_) | ResponseBlock::Error { .. } => {
            String::new()
        }
    }
}

impl ChatView {
    /// `BRUHS_DESKTOP_SMOKE_SCENE=export-reply`: open the last reply's
    /// "Export reply" panel. False without a reply.
    #[cfg(debug_assertions)]
    pub(crate) fn smoke_open_export_panel(&mut self, cx: &mut Context<'_, Self>) -> bool {
        let Some(index) = self
            .messages
            .iter()
            .rposition(|message| message.role == MessageRole::Assistant)
        else {
            return false;
        };
        self.rich_embeds.borrow_mut().export_open.insert(index);
        cx.notify();
        true
    }

    #[cfg(debug_assertions)]
    pub(crate) fn smoke_rich_response_report(&self) {
        let embeds = self.rich_embeds.borrow();
        eprintln!(
            "BRUHS_RICH_RESPONSE_SMOKE hosts={} shell_ready={}",
            embeds
                .entries
                .values()
                .filter(|entry| entry.host.is_live())
                .count(),
            matches!(embeds.shell, ShellStatus::Ready)
        );
        for entry in embeds.entries.values() {
            eprintln!(
                "BRUHS_RICH_RESPONSE_SMOKE state_key={} state={:?} revision={} saved={} height={}",
                entry.state_key, entry.state, entry.revision, entry.saved_revision, entry.height
            );
            entry.host.smoke_report();
        }
    }

    fn check_rich_response_host(&self, cx: &Context<'_, Self>) {
        let mut embeds = self.rich_embeds.borrow_mut();
        if embeds.quit_subscription.is_none() {
            embeds.quit_subscription =
                Some(cx.on_app_quit(|this, cx| this.flush_rich_widgets_on_quit(cx)));
        }
        if cfg!(test) {
            embeds.shell = ShellStatus::Ready;
            return;
        }
        if !matches!(embeds.shell, ShellStatus::Unchecked) || !embeds.shown() {
            return;
        }
        embeds.shell = ShellStatus::Loading;
        let result = spawn_tokio(cx, async {
            let client = crate::server_client::ServerClient::from_env()
                .map_err(|error| error.to_string())?;
            client.check_rich_response_host().await.map_err(|error| {
                format!("{error}. Start or restart bruhs-server to enable inline previews.")
            })?;
            Ok::<_, String>(client)
        });
        let chat = cx.weak_entity();
        embeds.shell_task = Some(cx.spawn(async move |_, cx| {
            let result = result.await;
            let _ = chat.update(cx, |this, cx| {
                let mut embeds = this.rich_embeds.borrow_mut();
                embeds.shell = match result {
                    Ok(client) => {
                        embeds.client = Some(client);
                        ShellStatus::Ready
                    }
                    Err(error) => ShellStatus::Failed(error),
                };
                cx.notify();
            });
        }));
    }

    pub(crate) fn set_rich_response_shown(&mut self, shown: bool, cx: &mut Context<'_, Self>) {
        let mut embeds = self.rich_embeds.borrow_mut();
        let changed = embeds.workspace_shown != shown;
        embeds.workspace_shown = shown;
        if !shown {
            embeds.hide();
        }
        if changed {
            cx.notify();
        }
    }

    pub(super) fn render_rich_response(
        &self,
        message: usize,
        index: usize,
        block: &Rc<ResponseBlock>,
        identity: &SharedString,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let project: SharedString = self
            .worktree_path
            .as_deref()
            .unwrap_or(&self.project_path)
            .to_string()
            .into();
        let artifact = matches!(block.as_ref(), ResponseBlock::Artifact(_));
        let mut key = match block.as_ref() {
            ResponseBlock::Artifact(artifact) => (
                message,
                usize::MAX,
                artifact.id.clone().into(),
                project.clone(),
            ),
            _ => (message, index, identity.clone(), project.clone()),
        };
        if let ResponseBlock::Media(media) = block.as_ref() {
            let embeds = self.rich_embeds.borrow();
            if let Some(previous) = embeds.positions.get(&(message, index)) {
                if let Some(entry) = embeds.entries.get(previous) {
                    let unchanged = entry.resolved.as_deref().is_some_and(
                        |block| matches!(block,ResponseBlock::Media(captured) if captured==media),
                    ) || entry.original_media.as_ref().is_some_and(|original| {
                        bound_media(original, &self.response_media_bindings) == *media
                    });
                    if unchanged {
                        key = previous.clone();
                    }
                }
            }
        }
        let (title, source, remote, height) = match block.as_ref() {
            ResponseBlock::Html(block) => (block.title.clone(), block.html.clone(), false, 360.0),
            ResponseBlock::Media(block) => (
                media_title(block).to_string(),
                block.src.clone(),
                media_has_remote(block),
                if block.kind == MediaKind::Audio {
                    64.0
                } else {
                    360.0
                },
            ),
            ResponseBlock::Artifact(block) => (
                block.title.clone(),
                format!("Artifact {}", block.id),
                false,
                360.0,
            ),
            ResponseBlock::Error { source, message } => {
                return div()
                    .text_color(theme::lychee_400())
                    .child(format!("Could not display response: {message}"))
                    .child(source.clone())
                    .into_any_element();
            }
            ResponseBlock::Markdown(_) => return div().into_any_element(),
        };
        self.prepare_rich_response(&key, block, cx);
        self.apply_acked_widget_reload(&key);
        let (resolved, widget_height) = {
            let embeds = self.rich_embeds.borrow();
            let entry = embeds.entries.get(&key);
            (
                entry.and_then(|entry| entry.resolved.clone()),
                entry.map_or(height, |entry| entry.height),
            )
        };
        let source = match resolved.as_deref() {
            Some(ResponseBlock::Html(html)) => html.html.clone(),
            _ => source,
        };
        let (source_open, remote_allowed, error) = {
            let mut embeds = self.rich_embeds.borrow_mut();
            if !artifact {
                if let Some(previous) = embeds.positions.insert((message, index), key.clone()) {
                    if previous != key {
                        embeds.entries.remove(&previous);
                    }
                }
            }
            let entry = embeds.entries.entry(key.clone()).or_default();
            (entry.source_open, entry.remote_allowed, entry.error.clone())
        };
        let toggle_key = key.clone();
        let mut element = div()
            .id(desktop_block_id("desktop-rich-response", message, index))
            .debug_selector(move || format!("desktop-rich-response-{message}-{index}"))
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1()
            .rounded(px(8.0))
            .border_1()
            .border_color(theme::longan_800())
            .bg(theme::longan_950())
            .p_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.0))
                            .text_color(theme::rambutan_400())
                            .child(title),
                    )
                    .child(
                        web_button(
                            "source",
                            WebButtonVariant::Ghost,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .debug_selector(move || {
                            format!("desktop-rich-source-toggle-{message}-{index}")
                        })
                        .child(if source_open { "Preview" } else { "Source" })
                        .map(|el| {
                            on_press_view(el, cx, move |this, _window, cx| {
                                this.toggle_rich_source(&toggle_key);
                                cx.notify();
                            })
                        }),
                    ),
            );
        if let ResponseBlock::Media(media) = block.as_ref() {
            let download = key.clone();
            let src = resolved
                .as_deref()
                .and_then(|block| match block {
                    ResponseBlock::Media(media) => Some(media.src.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| media.src.clone());
            element = element.child(runtime::action_button(
                "download-response-media",
                "Download",
                cx,
                move |this, _, cx| this.save_response_media(&download, src.clone(), cx),
            ));
        }
        if matches!(
            block.as_ref(),
            ResponseBlock::Html(_) | ResponseBlock::Artifact(_)
        ) {
            element = element.child(self.render_widget_controls(&key, false, cx));
        }
        element = self.render_rich_details(element, &key, block, cx);
        if !source_open && NativeEmbed::available() {
            self.check_rich_response_host(cx);
        }
        let shell_error = match &self.rich_embeds.borrow().shell {
            ShellStatus::Failed(error) => Some(error.clone()),
            _ => None,
        };
        if source_open || !NativeEmbed::available() || error.is_some() || shell_error.is_some() {
            if let Some(error) = error.or(shell_error) {
                element = element.child(
                    div()
                        .text_color(theme::lychee_400())
                        .child(format!("Preview unavailable: {error}")),
                );
                element = element.child(runtime::action_button(
                    "open-rich-web",
                    "Open web app",
                    cx,
                    |_, _, cx| cx.open_url(&server_base()),
                ));
            } else if !NativeEmbed::available() {
                element = element.child(
                    crate::webview_platform::inline_unavailable_reason().unwrap_or(
                        "The embedded browser is unavailable. Source remains available.",
                    ),
                );
                element = element.child(runtime::action_button(
                    "open-rich-web",
                    "Open web app",
                    cx,
                    |_, _, cx| cx.open_url(&server_base()),
                ));
            }
            let fence = "`".repeat(
                source
                    .split(|c| c != '`')
                    .map(str::len)
                    .max()
                    .unwrap_or(0)
                    .max(2)
                    + 1,
            );
            return element
                .child(
                    div()
                        .debug_selector(move || format!("desktop-rich-source-{message}-{index}"))
                        .w_full()
                        .min_w_0()
                        .child(
                            untrusted_markdown(
                                desktop_block_id("desktop-rich-source", message, index),
                                format!("{fence}\n{source}\n{fence}"),
                            )
                            .style(desktop_markdown_style(cx))
                            .selectable(true),
                        ),
                )
                .into_any_element();
        }
        if remote && !remote_allowed {
            return element
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(theme::rambutan_500())
                        .child(source),
                )
                .child(
                    web_button(
                        "load-external-media",
                        WebButtonVariant::Ghost,
                        WebButtonSize::Small,
                        ButtonState::Enabled,
                    )
                    .debug_selector(move || format!("desktop-rich-load-{message}-{index}"))
                    .child("Load external media")
                    .map(|el| {
                        on_press_view(el, cx, move |this, _window, cx| {
                            this.approve_rich_media(&key);
                            cx.notify();
                        })
                    }),
                )
                .into_any_element();
        }
        if !matches!(self.rich_embeds.borrow().shell, ShellStatus::Ready) {
            return element
                .child(
                    div()
                        .id(desktop_block_id("desktop-rich-loading", message, index))
                        .role(Role::Status)
                        .text_color(theme::rambutan_500())
                        .child("Opening inline preview…"),
                )
                .into_any_element();
        }
        if resolved.is_none() {
            return element.child("Preparing response…").into_any_element();
        }
        if self.rich_embeds.borrow().expanded.as_ref() == Some(&key) {
            return element
                .child(
                    div()
                        .h(px(widget_height.to_f32_lossy()))
                        .child("Widget is open in expanded view."),
                )
                .into_any_element();
        }
        element
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(widget_height.to_f32_lossy()))
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(theme::rambutan_500())
                            .text_size(px(11.0))
                            .child("Loading preview… Source remains available if it cannot open."),
                    )
                    .child(self.rich_canvas(key, Rc::clone(block), project, cx)),
            )
            .into_any_element()
    }

    fn rich_canvas(
        &self,
        key: BlockKey,
        block: Rc<ResponseBlock>,
        project: SharedString,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let cache = Rc::clone(&self.rich_embeds);
        let entity = cx.entity_id();
        let chat = cx.weak_entity();
        canvas(
            |_, _, _| (),
            move |bounds, _, window, cx| {
                RichEmbeds::track_parent(&cache, window, cx);
                if cache.borrow_mut().place(
                    &key,
                    &block,
                    &project,
                    bounds,
                    PlacementContext {
                        window,
                        app: cx,
                        entity,
                        chat: chat.clone(),
                    },
                ) {
                    cx.notify(entity);
                }
            },
        )
        .size_full()
        .into_any_element()
    }

    fn toggle_rich_source(&self, key: &BlockKey) {
        let mut embeds = self.rich_embeds.borrow_mut();
        let entry = embeds.entries.entry(key.clone()).or_default();
        entry.source_open = !entry.source_open;
        entry.error = None;
        entry.host.hide();
        if matches!(embeds.shell, ShellStatus::Failed(_)) {
            embeds.shell = ShellStatus::Unchecked;
        }
    }
}

pub(super) fn geometry_script(full: Bounds<Pixels>, visible: Bounds<Pixels>) -> String {
    format!(
        "{{const root=document.getElementById('bruhs-rich-root');if(root)root.style.cssText='position:absolute;width:{}px;height:{}px;left:{}px;top:{}px;overflow:hidden'}}",
        f32::from(full.size.width),
        f32::from(full.size.height),
        f32::from(full.origin.x - visible.origin.x),
        f32::from(full.origin.y - visible.origin.y)
    )
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
#[derive(Default)]
struct NativeEmbed;

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
impl NativeEmbed {
    fn available() -> bool {
        false
    }
    fn clear(&mut self) {}
    fn hide(&mut self) {}
    fn is_live(&self) -> bool {
        false
    }
    #[cfg(debug_assertions)]
    fn smoke_report(&self) {}
    fn place(
        &mut self,
        _shell: &str,
        _body: &str,
        _frame_url: Option<String>,
        _geometry: (Bounds<Pixels>, Bounds<Pixels>),
        _widget: HostWidget,
        context: PlacementContext<'_>,
    ) -> Result<(), String> {
        let _ = (context.app, context.entity);
        Err("No embedded browser on this platform".into())
    }
    fn flush(&self) {}
}

#[cfg(test)]
mod tests {
    use super::{embed_body, geometry_script, interactive_host_script, RichEmbeds};
    use bruhs_models::logic::rich_response::ResponseBlock;
    use gpui::{point, px, size, Bounds};

    #[gpui::test]
    fn parent_close_releases_hosts_before_window_drop_and_keeps_widget_state(
        cx: &mut gpui::TestAppContext,
    ) {
        use super::{Entry, Rc, RefCell};
        let cache = Rc::new(RefCell::new(RichEmbeds::default()));
        let root_cache = Rc::clone(&cache);
        let (_root, cx) = cx.add_window_view(move |window, app| {
            RichEmbeds::track_parent(&root_cache, window, app);
            gpui::EmptyView
        });
        let key = (0, 0, "widget".into(), "project".into());
        let mut entry = Entry::default();
        entry.state.custom = serde_json::json!({"count": 3});
        cache.borrow_mut().entries.insert(key.clone(), entry);
        assert!(cache.borrow().parent_window.is_some());
        cx.update(|window, _| window.remove_window());
        let embeds = cache.borrow();
        assert!(embeds.parent_window.is_none());
        assert_eq!(
            embeds.entries[&key].state.custom,
            serde_json::json!({"count": 3})
        );
    }

    #[test]
    fn clipping_preserves_the_full_layout_and_offsets_hidden_content() {
        let full = Bounds::new(point(px(20.0), px(80.0)), size(px(500.0), px(360.0)));
        let viewport = Bounds::new(point(px(0.0), px(120.0)), size(px(800.0), px(200.0)));
        let visible = full.intersect(&viewport);
        assert_eq!(
            visible,
            Bounds::new(point(px(20.0), px(120.0)), size(px(500.0), px(200.0)))
        );
        assert!(
            geometry_script(full, visible).contains("width:500px;height:360px;left:0px;top:-40px")
        );
    }

    #[test]
    fn html_content_is_contained_in_an_opaque_sandbox() {
        use bruhs_models::logic::rich_response::HtmlBlock;
        let html = ResponseBlock::Html(HtmlBlock {
            html: "</iframe><script>window.parent.document.body.remove()</script>".into(),
            title: "\"unsafe\"".into(),
        });
        let body = embed_body(&html, "http://127.0.0.1:3001", "/project");
        assert!(
            body.is_empty(),
            "Assistant source never becomes native top-frame markup"
        );
        let script = interactive_host_script("<button>Safe</button>", "native-instance");
        assert!(script.contains("allow-scripts"));
        assert!(!script.contains("allow-same-origin"));
    }

    #[test]
    fn workspace_and_chat_occlusion_hide_native_surfaces() {
        let mut embeds = RichEmbeds::default();
        assert!(embeds.shown());
        embeds.set_occluded(true);
        assert!(!embeds.shown());
        embeds.set_occluded(false);
        embeds.workspace_shown = false;
        assert!(!embeds.shown());
    }

    #[test]
    fn removed_replies_lose_their_export_consent() {
        let mut embeds = RichEmbeds::default();
        embeds.export_state.extend([1, 3]);
        embeds.export_remote.extend([1, 3]);
        embeds.export_open.extend([1, 3]);
        embeds.truncate(2);
        assert_eq!(embeds.export_state, [1].into());
        assert_eq!(embeds.export_remote, [1].into());
        assert_eq!(embeds.export_open, [1].into());
        embeds.clear();
        assert!(embeds.export_state.is_empty());
        assert!(embeds.export_remote.is_empty());
        assert!(embeds.export_open.is_empty());
    }

    #[test]
    fn ordinary_remote_media_does_not_require_cors() {
        use bruhs_models::logic::rich_response::{MediaBlock, MediaKind};
        for kind in [MediaKind::Video, MediaKind::Audio] {
            let media = ResponseBlock::Media(MediaBlock {
                kind,
                src: "https://example.com/no-cors.mp4".into(),
                ..MediaBlock::default()
            });
            let body = embed_body(&media, "http://127.0.0.1:3001", "/project");
            assert!(!body.contains("crossorigin"));
            assert!(body.contains("preload=\"none\""));
        }
    }
}
