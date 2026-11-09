//! Preview surface (⌘⇧J): a URL row over an embedded WebView,
//! with the machine's local TCP listeners offered while no URL is set.
//!
//! The WebView is a native child of the gpui window (wry / WKWebView on
//! macOS), positioned every frame from a `canvas` in the panel body and
//! hidden whenever the workspace says the surface is not on screen or an
//! overlay is open above it. Other platforms keep the URL row and the
//! server list and open pages in the browser.

use bruhs_core::preview::LocalServer;
use bruhs_models::logic::preview::normalize_preview_url;
use gpui::prelude::*;
use gpui::*;
use gpui_component::input::{Input, InputEvent, InputState};

use crate::views::web_button::{web_button, ButtonState, WebButtonSize, WebButtonVariant};
use crate::{assets, theme};

const HEADER_HEIGHT_PX: f32 = 40.0;
const HEADER_TEXT_SIZE_PX: f32 = 12.0;
const ICON_SIZE_PX: f32 = 13.0;
const EMPTY_TEXT_SIZE_PX: f32 = 13.0;
const EMPTY_HINT_TEXT_SIZE_PX: f32 = 12.0;
const EMPTY_LIST_WIDTH_PX: f32 = 320.0;
const INPUT_FONT_FAMILY: &str = "Monaspace Neon";
const URL_PLACEHOLDER: &str = "http://localhost:3000";
const EMPTY_TITLE: &str = "Enter a local URL to preview";
const EMPTY_HINT: &str = "Pages that refuse framing open in the browser instead.";
const SERVERS_LABEL: &str = "LOCAL SERVERS";
const SERVERS_SCANNING: &str = "Scanning…";
const SERVERS_NONE: &str = "Nothing is listening on localhost.";
const NO_WEBVIEW_HINT: &str = "No embedded browser on this platform — use Open in browser.";
const BORDER_OPACITY: f32 = 0.6;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewPanelEvent {
    /// "Open in browser" (or a page that cannot be embedded).
    OpenExternal(String),
}

#[cfg(target_os = "macos")]
mod host {
    use wry::dpi::{LogicalPosition, LogicalSize};
    use wry::{Rect, WebView, WebViewBuilder};

    /// Longan-950 behind the page while it loads.
    const BACKGROUND: (u8, u8, u8, u8) = (0x14, 0x12, 0x10, 0xff);

    /// Owns the native child view. Dropping it removes the view.
    #[derive(Default)]
    pub struct WebViewHost {
        webview: Option<WebView>,
        url: Option<String>,
        visible: bool,
        bounds: Option<(f32, f32, f32, f32)>,
        /// A hide failed and was logged; quiet until one succeeds.
        hide_failing: bool,
    }

    fn rect(bounds: gpui::Bounds<gpui::Pixels>) -> Rect {
        Rect {
            position: LogicalPosition::new(
                f64::from(f32::from(bounds.origin.x)),
                f64::from(f32::from(bounds.origin.y)),
            )
            .into(),
            size: LogicalSize::new(
                f64::from(f32::from(bounds.size.width).max(1.0)),
                f64::from(f32::from(bounds.size.height).max(1.0)),
            )
            .into(),
        }
    }

    impl WebViewHost {
        pub fn available() -> bool {
            true
        }

        #[cfg(any(test, debug_assertions))]
        pub fn is_live(&self) -> bool {
            self.webview.is_some()
        }

        /// Create on first use, then keep the page positioned and visible.
        pub fn place(
            &mut self,
            window: &gpui::Window,
            url: &str,
            bounds: gpui::Bounds<gpui::Pixels>,
        ) -> Result<(), String> {
            let key = (
                f32::from(bounds.origin.x),
                f32::from(bounds.origin.y),
                f32::from(bounds.size.width),
                f32::from(bounds.size.height),
            );
            match &self.webview {
                None => {
                    let webview = WebViewBuilder::new()
                        .with_url(url)
                        .with_bounds(rect(bounds))
                        .with_background_color(BACKGROUND)
                        .build_as_child(window)
                        .map_err(|e| e.to_string())?;
                    self.webview = Some(webview);
                    self.url = Some(url.to_string());
                    self.visible = true;
                    self.bounds = Some(key);
                }
                Some(webview) => {
                    if self.url.as_deref() != Some(url) {
                        webview.load_url(url).map_err(|e| e.to_string())?;
                        self.url = Some(url.to_string());
                    }
                    if !self.visible {
                        webview.set_visible(true).map_err(|e| e.to_string())?;
                        self.visible = true;
                    }
                    if self.bounds != Some(key) {
                        webview
                            .set_bounds(rect(bounds))
                            .map_err(|e| e.to_string())?;
                        self.bounds = Some(key);
                    }
                }
            }
            Ok(())
        }

        /// Hide the page. It only counts as hidden once the native view
        /// says so; otherwise the next frame (the workspace hides it on
        /// every frame something floats above it) tries again.
        pub fn hide(&mut self) {
            if let (Some(webview), true) = (&self.webview, self.visible) {
                match webview.set_visible(false) {
                    Ok(()) => {
                        self.visible = false;
                        self.hide_failing = false;
                    }
                    Err(error) => {
                        if !self.hide_failing {
                            tracing::warn!("could not hide the preview page: {error}");
                            self.hide_failing = true;
                        }
                    }
                }
            }
        }

        pub fn reload(&self) {
            if let Some(webview) = &self.webview {
                if let Err(error) = webview.reload() {
                    tracing::warn!("could not reload the preview page: {error}");
                }
            }
        }

        pub fn clear(&mut self) {
            self.webview = None;
            self.url = None;
            self.visible = false;
            self.bounds = None;
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod host {
    #[derive(Default)]
    pub struct WebViewHost;

    impl WebViewHost {
        pub fn available() -> bool {
            false
        }
        #[cfg(any(test, debug_assertions))]
        pub fn is_live(&self) -> bool {
            false
        }
        pub fn place(
            &mut self,
            _window: &gpui::Window,
            _url: &str,
            _bounds: gpui::Bounds<gpui::Pixels>,
        ) -> Result<(), String> {
            Err("no embedded browser on this platform".to_string())
        }
        pub fn hide(&mut self) {}
        pub fn reload(&self) {}
        pub fn clear(&mut self) {}
    }
}

pub use host::WebViewHost;

pub struct PreviewPanelView {
    url: Option<String>,
    url_input: Entity<InputState>,
    _url_input_subscription: Subscription,
    servers: Vec<LocalServer>,
    servers_loading: bool,
    #[cfg_attr(
        test,
        expect(dead_code, reason = "only the cfg(not(test)) server scan reads it")
    )]
    servers_generation: u64,
    /// Set by the workspace: the surface is on screen and nothing floats
    /// above it. The native view is hidden otherwise.
    shown: bool,
    host: WebViewHost,
    host_error: Option<String>,
    /// Frames in which the body asked the host to place the page.
    place_attempts: u32,
}

impl EventEmitter<PreviewPanelEvent> for PreviewPanelView {}

impl PreviewPanelView {
    pub fn new(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let url_input = cx.new(|cx| InputState::new(window, cx).placeholder(URL_PLACEHOLDER));
        let _url_input_subscription = cx.subscribe_in(
            &url_input,
            window,
            |this: &mut Self, input, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    let raw = input.read(cx).value().to_string();
                    this.go_to(&raw, window, cx);
                }
            },
        );
        let mut view = Self {
            url: None,
            url_input,
            _url_input_subscription,
            servers: Vec::new(),
            servers_loading: false,
            servers_generation: 0,
            shown: false,
            host: WebViewHost::default(),
            host_error: None,
            place_attempts: 0,
        };
        view.refresh_servers(cx);
        view
    }

    #[cfg(any(test, debug_assertions))]
    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }

    /// Whether a native page view exists right now.
    #[cfg(any(test, debug_assertions))]
    pub fn host_is_live(&self) -> bool {
        self.host.is_live()
    }

    #[cfg(any(test, debug_assertions))]
    pub fn host_error(&self) -> Option<&str> {
        self.host_error.as_deref()
    }

    #[cfg(any(test, debug_assertions))]
    pub fn is_shown(&self) -> bool {
        self.shown
    }

    /// `BRUHS_DESKTOP_TERMINAL_SMOKE`: create the native page view against
    /// the real window without waiting for a frame.
    #[cfg(debug_assertions)]
    pub fn smoke_place(
        &mut self,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) -> Result<(), String> {
        let Some(url) = self.url.clone() else {
            return Err("no url".to_string());
        };
        self.shown = true;
        let bounds = Bounds {
            origin: point(px(0.0), px(0.0)),
            size: size(px(480.0), px(320.0)),
        };
        let result = self.host.place(window, &url, bounds);
        self.host_error = result.as_ref().err().cloned();
        cx.notify();
        result
    }

    #[cfg(debug_assertions)]
    pub fn place_attempts(&self) -> u32 {
        self.place_attempts
    }

    /// The workspace calls this every frame it renders; hiding is immediate
    /// so overlays never end up under the native view.
    pub fn set_shown(&mut self, shown: bool) {
        if self.shown != shown {
            self.shown = shown;
            if !shown {
                self.host.hide();
            }
        }
    }

    /// Navigate to what the user typed (see `normalize_preview_url`).
    pub fn go_to(&mut self, raw: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(url) = normalize_preview_url(raw) else {
            return;
        };
        self.url_input.update(cx, |input, cx| {
            input.set_value(url.clone(), window, cx);
        });
        self.url = Some(url);
        self.host_error = None;
        cx.notify();
    }

    pub fn reload(&mut self, cx: &mut Context<'_, Self>) {
        self.host.reload();
        cx.notify();
    }

    pub fn open_external(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(url) = self.url.clone() {
            cx.emit(PreviewPanelEvent::OpenExternal(url));
        }
    }

    pub fn clear(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.url = None;
        self.host.clear();
        self.host_error = None;
        self.url_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.refresh_servers(cx);
        cx.notify();
    }

    /// Under test the scan is a no-op: gpui's test scheduler rejects
    /// wake-ups from the tokio runtime.
    #[cfg(test)]
    pub fn refresh_servers(&mut self, cx: &mut Context<'_, Self>) {
        self.servers_loading = false;
        cx.notify();
    }

    #[cfg(not(test))]
    pub fn refresh_servers(&mut self, cx: &mut Context<'_, Self>) {
        self.servers_generation += 1;
        let generation = self.servers_generation;
        self.servers_loading = true;
        let runtime = std::sync::Arc::clone(&cx.global::<crate::app_state::DesktopState>().runtime);
        let (tx, rx) = async_channel::bounded::<Vec<LocalServer>>(1);
        runtime.spawn(async move {
            let servers = bruhs_core::preview::scan_local_servers(&[]).await;
            let _ = tx.send(servers).await;
        });
        cx.spawn(async move |this, cx| {
            let Ok(servers) = rx.recv().await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if this.servers_generation != generation {
                    return;
                }
                this.servers = servers;
                this.servers_loading = false;
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    #[cfg(test)]
    pub fn set_servers_for_test(&mut self, servers: Vec<LocalServer>, cx: &mut Context<'_, Self>) {
        self.servers = servers;
        self.servers_loading = false;
        cx.notify();
    }

    fn place_webview(
        &mut self,
        window: &Window,
        bounds: Bounds<Pixels>,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(url) = self.url.clone() else {
            return;
        };
        self.place_attempts = self.place_attempts.saturating_add(1);
        // Test windows have no native handle to attach a WKWebView to.
        if !self.shown || cfg!(test) {
            return;
        }
        match self.host.place(window, &url, bounds) {
            Ok(()) => {
                if self.host_error.take().is_some() {
                    cx.notify();
                }
            }
            Err(err) => {
                if self.host_error.as_deref() != Some(err.as_str()) {
                    self.host_error = Some(err);
                    cx.notify();
                }
            }
        }
    }

    fn icon_button(
        &self,
        id: &'static str,
        label: &'static str,
        icon: &'static str,
        enabled: bool,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<'_, Self>) + 'static,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let color = if enabled {
            theme::rambutan_400()
        } else {
            theme::rambutan_500().opacity(0.5)
        };
        web_button(
            id,
            WebButtonVariant::Ghost,
            WebButtonSize::IconSmall,
            ButtonState::disabled_if(!enabled),
        )
        .debug_selector(move || id.to_string())
        .aria_label(label)
        .text_color(color)
        .when(enabled, |el| el.cursor_pointer())
        .on_click(cx.listener(move |this, _ev, window, cx| {
            cx.stop_propagation();
            if enabled {
                on_click(this, window, cx);
            }
        }))
        .child(svg().path(icon).size(px(ICON_SIZE_PX)).text_color(color))
        .into_any_element()
    }

    fn render_header(&self, cx: &Context<'_, Self>) -> AnyElement {
        let has_url = self.url.is_some();
        div()
            .h(px(HEADER_HEIGHT_PX))
            .px_2()
            .flex()
            .items_center()
            .gap(px(6.0))
            .flex_shrink_0()
            .border_b_1()
            .border_color(theme::longan_800().opacity(BORDER_OPACITY))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h(px(28.0))
                    .px_2()
                    .rounded(px(4.0))
                    .bg(theme::longan_900())
                    .border_1()
                    .border_color(theme::longan_800())
                    .flex()
                    .items_center()
                    .child(
                        Input::new(&self.url_input)
                            .aria_label("Preview URL")
                            .appearance(false)
                            .bordered(false)
                            .focus_bordered(false)
                            .cleanable(false)
                            .font_family(INPUT_FONT_FAMILY)
                            .text_size(px(HEADER_TEXT_SIZE_PX)),
                    ),
            )
            .child(
                web_button(
                    "preview-go",
                    WebButtonVariant::Outline,
                    WebButtonSize::Small,
                    ButtonState::Enabled,
                )
                .debug_selector(|| "desktop-preview-go".to_string())
                .aria_label("Open URL")
                .cursor_pointer()
                .on_click(cx.listener(|this, _ev, window, cx| {
                    cx.stop_propagation();
                    let raw = this.url_input.read(cx).value().to_string();
                    this.go_to(&raw, window, cx);
                }))
                .child("Go"),
            )
            .child(self.icon_button(
                "desktop-preview-reload",
                "Reload preview",
                assets::REFRESH_ICON_PATH,
                has_url,
                |this, _window, cx| this.reload(cx),
                cx,
            ))
            .child(self.icon_button(
                "desktop-preview-open",
                "Open in browser",
                assets::EXTERNAL_LINK_ICON_PATH,
                has_url,
                |this, _window, cx| this.open_external(cx),
                cx,
            ))
            .child(self.icon_button(
                "desktop-preview-clear",
                "Clear preview",
                assets::CLOSE_ICON_PATH,
                has_url,
                |this, window, cx| this.clear(window, cx),
                cx,
            ))
            .into_any_element()
    }

    fn render_server_row(&self, server: &LocalServer, cx: &Context<'_, Self>) -> AnyElement {
        let target = format!("http://localhost:{}", server.port);
        let label = target.clone();
        let command = server.command.clone();
        let port = server.port;
        div()
            .id(ElementId::Name(format!("preview-server-{port}").into()))
            .debug_selector(move || format!("desktop-preview-server-{port}"))
            .w_full()
            .px_3()
            .py(px(6.0))
            .rounded(px(4.0))
            .bg(theme::longan_900())
            .border_1()
            .border_color(theme::ring_card())
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .text_size(px(EMPTY_HINT_TEXT_SIZE_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(theme::rambutan_200())
            .hover(|el| el.bg(theme::longan_800().opacity(0.7)))
            .tab_index(0)
            .role(Role::Button)
            .aria_label(format!("Preview {label}"))
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _ev, window, cx| {
                cx.stop_propagation();
                this.go_to(&target, window, cx);
            }))
            .child(div().font_family(INPUT_FONT_FAMILY).child(label))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(theme::rambutan_500())
                    .child(command),
            )
            .into_any_element()
    }

    fn render_empty(&self, cx: &Context<'_, Self>) -> AnyElement {
        let rows: Vec<AnyElement> = self
            .servers
            .iter()
            .map(|server| self.render_server_row(server, cx))
            .collect();
        let status = if rows.is_empty() {
            Some(if self.servers_loading {
                SERVERS_SCANNING
            } else {
                SERVERS_NONE
            })
        } else {
            None
        };
        div()
            .debug_selector(|| "desktop-preview-empty".to_string())
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_4()
            .p_6()
            .child(
                svg()
                    .path(assets::TOOL_GLOBE_ICON_PATH)
                    .size(px(18.0))
                    .text_color(theme::rambutan_400()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(EMPTY_TEXT_SIZE_PX))
                            .text_color(theme::rambutan_100())
                            .child(EMPTY_TITLE),
                    )
                    .child(
                        div()
                            .text_size(px(EMPTY_HINT_TEXT_SIZE_PX))
                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                            .text_color(theme::rambutan_500())
                            .child(EMPTY_HINT),
                    ),
            )
            .child(
                div()
                    .debug_selector(|| "desktop-preview-servers".to_string())
                    .w(px(EMPTY_LIST_WIDTH_PX))
                    .max_w_full()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .mb_1()
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(theme::rambutan_500())
                                    .child(SERVERS_LABEL),
                            )
                            .child(self.icon_button(
                                "desktop-preview-rescan",
                                "Rescan local servers",
                                assets::REFRESH_ICON_PATH,
                                true,
                                |this, _window, cx| this.refresh_servers(cx),
                                cx,
                            )),
                    )
                    .children(rows)
                    .when_some(status, |el, text| {
                        el.child(
                            div()
                                .px_3()
                                .py_2()
                                .rounded(px(4.0))
                                .bg(theme::longan_900())
                                .border_1()
                                .border_color(theme::ring_card())
                                .text_size(px(EMPTY_HINT_TEXT_SIZE_PX))
                                .font_family(theme::UI_SMALL_FONT_FAMILY)
                                .text_color(theme::rambutan_500())
                                .child(text),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_page(&self, cx: &Context<'_, Self>) -> AnyElement {
        let view = cx.entity();
        let place = canvas(
            move |bounds, window, cx| {
                view.update(cx, |this, cx| this.place_webview(window, bounds, cx));
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let hint = if !WebViewHost::available() {
            Some(NO_WEBVIEW_HINT.to_string())
        } else {
            self.host_error
                .clone()
                .map(|err| format!("Embedded browser unavailable: {err}"))
        };
        div()
            .debug_selector(|| "desktop-preview-page".to_string())
            .flex_1()
            .min_h_0()
            .relative()
            .bg(theme::longan_950())
            .child(place)
            .when_some(hint, |el, text| {
                el.child(
                    div()
                        .debug_selector(|| "desktop-preview-hint".to_string())
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .p_6()
                        .text_size(px(EMPTY_HINT_TEXT_SIZE_PX))
                        .font_family(theme::UI_SMALL_FONT_FAMILY)
                        .text_color(theme::rambutan_500())
                        .text_center()
                        .child(text),
                )
            })
            .into_any_element()
    }
}

impl Render for PreviewPanelView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let body = if self.url.is_some() {
            self.render_page(cx)
        } else {
            self.render_empty(cx)
        };
        div()
            .id("preview-panel")
            .debug_selector(|| "desktop-preview-panel".to_string())
            .role(Role::Region)
            .aria_label("Preview")
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(self.render_header(cx))
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::WebViewHost;

    #[test]
    fn webview_availability_matches_platform() {
        assert_eq!(WebViewHost::available(), cfg!(target_os = "macos"));
        assert!(!WebViewHost::default().is_live());
    }
}
