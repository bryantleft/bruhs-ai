//! A bruh's Computer tab (web `crew/computer.rs`): its browser's status
//! line, the owner's controls and the live view.
//!
//! The live view is the server's screencast socket while the browser runs
//! on a service that streams it. Frames arrive one at a time as JPEG bytes,
//! decode off the main thread into a [`RenderImage`] and are acknowledged
//! once decoded, so a hidden window never stalls the stream; the newest
//! frame waiting wins. While the owner holds control, the view's pointer,
//! wheel, keys and paste go to the page. Keys are taken before the app's
//! bindings (gpui-component's root binds Tab), except the app's ⌘ shortcuts.
use super::bruh_page::{
    alert, small_text, Layout, LABEL_TEXT_PX, PAGE_PADDING_PX, RADIUS_GRAPE_PX, SMALL_TEXT_PX,
};
use super::create_bruh::{FIELD_TEXT_PX, FIELD_X_PADDING_PX};
use super::ReactiveAutomations;
#[cfg(not(test))]
use crate::app_state::spawn_tokio;
use crate::server_client::{Screencast, ScreencastEvent, ServerClient};
use crate::theme;
use crate::views::web_button::{web_button, ButtonState, WebButtonSize, WebButtonVariant};
use bruhs_models::agent::AgentProvider;
use bruhs_models::crew::{
    computer_controls, computer_on, computer_status_line, ComputerControls, ADDRESS_LABEL, BACK,
    CLOSE_BROWSER, HAND_BACK, LIVE_VIEW_HINT, LIVE_VIEW_LABEL, OPEN_BROWSER, OPEN_IN_WINDOW,
    RELOAD, TAKE_CONTROL, TAKE_CONTROL_FIRST, USE_THE_WINDOW,
};
use bruhs_models::local_tools::{
    owner_key_route, to_viewport, to_viewport_clamped, ComputerInput, ComputerRequest,
    ComputerState, KeyKind, KeyRoute, MouseButton, MouseKind, Viewport, MAX_SCROLL_PX,
    MODIFIER_ALT, MODIFIER_CTRL, MODIFIER_META, MODIFIER_SHIFT,
};
use bruhs_ui::press::on_press_view;
use gpui::prelude::*;
use gpui::Focusable as _;
use gpui::{
    canvas, div, img, App, Bounds, Context, DispatchPhase, Div, Entity, FocusHandle, KeyUpEvent,
    Keystroke, Modifiers, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels, Point,
    RenderImage, Role, ScrollDelta, ScrollEvent, SharedString, Stateful, Subscription, Task,
    WeakEntity, Window,
};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::scroll::ScrollableElement;
use gpui_component::zoom::{css_px, px};
use std::{cell::Cell, rc::Rc, sync::Arc, time::Duration};

/// How often the tab asks for the state while no live view runs, so a
/// browser the bruh opens shows up (web `CLOSED_POLL_MS`).
const CLOSED_POLL: Duration = Duration::from_secs(5);
/// The first wait before a closed live view reconnects, doubled after each
/// failure up to [`BACKOFF_MAX_MS`] (web `live.rs`).
const BACKOFF_START_MS: u64 = 1_000;
const BACKOFF_MAX_MS: u64 = 10_000;
/// CSS px per wheel line, as browsers scroll a line.
const WHEEL_LINE_PX: f64 = 16.0;
/// The largest frame the view decodes, in pixels a side.
const MAX_FRAME_SIDE: u32 = 4_096;
/// Web `max-w-5xl`.
const COLUMN_MAX_PX: f32 = 1024.0;
/// Web `max-w-[min(100%,max(20rem,calc((100dvh_-_20rem)*1.6)))]` on the
/// 16:10 live view: as wide as fits, no taller than the window leaves.
const LIVE_MIN_PX: f32 = 320.0;
const LIVE_ROOM_PX: f32 = 320.0;
const LIVE_RATIO: f32 = 1.6;
/// Web `basis-48` on the address.
const ADDRESS_BASIS_PX: f32 = 192.0;
/// Web `aria-busy:opacity-60` on a button whose request runs.
const BUSY_OPACITY: f32 = 0.6;
/// The app's own modifier: ⌘ on a Mac, Ctrl elsewhere.
const MAC: bool = cfg!(target_os = "macos");

/// The DOM `key` and `code` of a GPUI keystroke, for the page. `None` for
/// keys the live view doesn't forward.
pub(crate) fn dom_key(keystroke: &Keystroke) -> Option<(String, String)> {
    let named = |key: &str, code: &str| Some((key.to_string(), code.to_string()));
    match keystroke.key.as_str() {
        "enter" => named("Enter", "Enter"),
        "backspace" => named("Backspace", "Backspace"),
        "tab" => named("Tab", "Tab"),
        "escape" => named("Escape", "Escape"),
        "space" => named(" ", "Space"),
        "left" => named("ArrowLeft", "ArrowLeft"),
        "right" => named("ArrowRight", "ArrowRight"),
        "up" => named("ArrowUp", "ArrowUp"),
        "down" => named("ArrowDown", "ArrowDown"),
        "home" => named("Home", "Home"),
        "end" => named("End", "End"),
        "pageup" => named("PageUp", "PageUp"),
        "pagedown" => named("PageDown", "PageDown"),
        "delete" => named("Delete", "Delete"),
        key => {
            if let Some(number) = key.strip_prefix('f') {
                return match number.parse::<u8>() {
                    Ok(1..=12) => {
                        let name = format!("F{number}");
                        Some((name.clone(), name))
                    }
                    _ => None,
                };
            }
            let mut chars = key.chars();
            let (Some(first), None) = (chars.next(), chars.next()) else {
                return None;
            };
            // What the key types, or with ⌘ or Ctrl held, the key itself.
            let typed = keystroke
                .key_char
                .clone()
                .filter(|typed| !typed.is_empty() && !typed.chars().any(char::is_control))
                .unwrap_or_else(|| {
                    if keystroke.modifiers.shift {
                        key.to_uppercase()
                    } else {
                        key.to_string()
                    }
                });
            Some((typed, dom_code(first)))
        }
    }
}

/// The DOM `code` of the physical key that types `key` on a US layout.
fn dom_code(key: char) -> String {
    if key.is_ascii_alphabetic() {
        return format!("Key{}", key.to_ascii_uppercase());
    }
    if key.is_ascii_digit() {
        return format!("Digit{key}");
    }
    let code = match key {
        '.' => "Period",
        ',' => "Comma",
        '/' => "Slash",
        ';' => "Semicolon",
        '\'' => "Quote",
        '[' => "BracketLeft",
        ']' => "BracketRight",
        '\\' => "Backslash",
        '-' => "Minus",
        '=' => "Equal",
        '`' => "Backquote",
        _ => "",
    };
    code.to_string()
}

/// The `MODIFIER_*` bits of GPUI's modifier keys.
pub(crate) fn modifiers(held: &Modifiers) -> u8 {
    [
        (held.alt, MODIFIER_ALT),
        (held.control, MODIFIER_CTRL),
        (held.platform, MODIFIER_META),
        (held.shift, MODIFIER_SHIFT),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .fold(0, |bits, (_, bit)| bits | bit)
}

/// The page's button for a GPUI one; back and forward don't map.
fn page_button(button: gpui::MouseButton) -> MouseButton {
    match button {
        gpui::MouseButton::Left => MouseButton::Left,
        gpui::MouseButton::Middle => MouseButton::Middle,
        gpui::MouseButton::Right => MouseButton::Right,
        gpui::MouseButton::Navigate(_) => MouseButton::None,
    }
}

/// A wheel turn in the page's CSS px, signed as the DOM signs it (down is
/// positive), within the scroll budget.
fn wheel_px(delta: &ScrollDelta) -> (f64, f64) {
    let (dx, dy) = match delta {
        ScrollDelta::Pixels(delta) => (f64::from(css_px(delta.x)), f64::from(css_px(delta.y))),
        ScrollDelta::Lines(delta) => (
            f64::from(delta.x) * WHEEL_LINE_PX,
            f64::from(delta.y) * WHEEL_LINE_PX,
        ),
    };
    let page = |turn: f64| {
        if turn.is_finite() {
            (-turn).clamp(-MAX_SCROLL_PX, MAX_SCROLL_PX)
        } else {
            0.0
        }
    };
    (page(dx), page(dy))
}

/// Where a key pressed in the live view goes.
#[derive(Debug, Clone, PartialEq)]
enum KeyAction {
    /// To the page, as this input.
    Send(ComputerInput),
    /// Send the clipboard's text.
    Paste,
    /// Move focus to the toolbar.
    Leave,
    /// Leave it to the app's shortcuts.
    App,
}

/// Where a key goes, through the shared [`owner_key_route`]. A paste is
/// taken on key down only.
fn key_action(key: &str, code: &str, modifiers: u8, down: bool) -> KeyAction {
    match owner_key_route(key, modifiers, MAC) {
        KeyRoute::Leave => KeyAction::Leave,
        KeyRoute::Paste if down => KeyAction::Paste,
        KeyRoute::Paste | KeyRoute::App => KeyAction::App,
        KeyRoute::Page => KeyAction::Send(ComputerInput::Key {
            kind: if down { KeyKind::Down } else { KeyKind::Up },
            key: key.into(),
            code: code.into(),
            modifiers,
        }),
    }
}

/// The keys whose press went to the page (web `HeldKeys`), so each gets
/// its release whatever modifiers are held by then, and none is left down
/// when the view loses focus. Keys are told apart by their `code`.
#[derive(Debug, Default)]
struct HeldKeys(Vec<(String, String)>);

impl HeldKeys {
    fn route(&mut self, key: &str, code: &str, modifiers: u8, down: bool) -> KeyAction {
        let id = if code.is_empty() { key } else { code };
        let action = key_action(key, code, modifiers, down);
        if down {
            if matches!(action, KeyAction::Send(_)) && !self.0.iter().any(|(held, _)| held == id) {
                self.0.push((id.to_string(), key.to_string()));
            }
            return action;
        }
        match self.0.iter().position(|(held, _)| held == id) {
            Some(index) => {
                let (_, pressed) = self.0.remove(index);
                KeyAction::Send(ComputerInput::Key {
                    kind: KeyKind::Up,
                    key: pressed,
                    code: code.into(),
                    modifiers,
                })
            }
            None if action == KeyAction::Leave => KeyAction::Leave,
            None => KeyAction::App,
        }
    }

    /// Releases for every key still held, which are no longer.
    fn release_all(&mut self) -> Vec<ComputerInput> {
        self.0
            .drain(..)
            .map(|(code, key)| ComputerInput::Key {
                kind: KeyKind::Up,
                key,
                code,
                modifiers: 0,
            })
            .collect()
    }
}

/// How long a closed live view waits before it connects again. Only a
/// frame proves the view works: the service sends its state first even
/// when the stream then fails, so a state message resets nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Backoff {
    wait_ms: u64,
    /// This socket has shown a frame.
    framed: bool,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            wait_ms: BACKOFF_START_MS,
            framed: false,
        }
    }
}

impl Backoff {
    /// A frame arrived. True for the socket's first, which resets the wait.
    fn frame(&mut self) -> bool {
        let first = !self.framed;
        self.framed = true;
        self.wait_ms = BACKOFF_START_MS;
        first
    }

    /// The socket closed: the wait before the next one, doubled for the
    /// time after.
    fn closed(&mut self) -> u64 {
        let wait = self.wait_ms;
        self.wait_ms = wait.saturating_mul(2).min(BACKOFF_MAX_MS);
        self.framed = false;
        wait
    }
}

/// Decode one JPEG frame into GPUI's BGRA image, as the whiteboard decodes
/// a raster (`whiteboard/catalog/files.rs`).
fn decode_frame(bytes: &[u8]) -> Result<Arc<RenderImage>, String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| error.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_FRAME_SIDE);
    limits.max_image_height = Some(MAX_FRAME_SIDE);
    limits.max_alloc = Some(bruhs_models::logic::attachments::MAX_IMAGE_TOTAL_BYTES);
    reader.limits(limits);
    let mut rgba = reader
        .decode()
        .map_err(|error| error.to_string())?
        .to_rgba8();
    for pixel in rgba.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Ok(Arc::new(RenderImage::new(vec![image::Frame::new(rgba)])))
}

/// What the panel knows of this bruh's Computer.
#[derive(Debug, Clone, Default)]
struct Gate {
    name: String,
    /// Its Computer is on and its access and model allow it.
    on: bool,
    /// The service runs Computers.
    ready: bool,
    /// Why Computers are off on an older service.
    reason: Option<&'static str>,
}

impl ReactiveAutomations {
    fn computer_gate(&self, agent_id: &str) -> Gate {
        let bruh = self.snapshot.as_ref().and_then(|snapshot| {
            let bruh = snapshot
                .bruhs
                .iter()
                .find(|bruh| bruh.agent_id == agent_id)?;
            let provider = snapshot
                .agents
                .iter()
                .find(|agent| agent.id == agent_id)
                .map_or(AgentProvider::Claude, |agent| {
                    AgentProvider::for_model(&agent.model)
                });
            Some((
                bruh.name.clone(),
                computer_on(bruh.computer, &bruh.access, provider),
            ))
        });
        let (name, on) = bruh.unwrap_or_default();
        Gate {
            name,
            on,
            ready: self.computer_ready(),
            reason: self.computer_reason(),
        }
    }
}

/// The live view's socket and its reconnects.
#[derive(Default)]
struct Live {
    /// The browser session this view watches, while it is connected,
    /// connecting or waiting to retry.
    session: Option<String>,
    screencast: Option<Screencast>,
    /// Opening the socket, then reading its events.
    task: Option<Task<()>>,
    retry: Option<Task<()>>,
    /// Bumped whenever a socket goes, so its late events do nothing.
    generation: u64,
    backoff: Backoff,
    connected: bool,
}

/// A bruh's Computer tab.
pub(crate) struct ComputerView {
    agent_id: String,
    #[cfg_attr(test, expect(dead_code, reason = "tests never reach a server"))]
    client: ServerClient,
    panel: WeakEntity<ReactiveAutomations>,
    state: Option<ComputerState>,
    error: Option<String>,
    /// The error came from reading the state, so the next read clears it.
    fetch_failed: bool,
    /// The error came from the stream, so a frame clears it.
    stream_failed: bool,
    /// The request on its way, if any.
    pending: Option<ComputerRequest>,
    /// Bumped by every request and stream state, so an older read's answer
    /// is stale.
    revision: u64,
    /// The gate as last seen: the state is read when it opens.
    gate_seen: (bool, bool),
    live: Live,
    frame: Option<Arc<RenderImage>>,
    /// The frame on screen has been drawn, so it holds an atlas entry.
    frame_drawn: bool,
    /// Drawn frames replaced or cleared, freed from the atlas on the next
    /// draw. A frame never drawn is just dropped, so frames that decode
    /// while the window doesn't draw never pile up.
    retired: Vec<Arc<RenderImage>>,
    decoding: bool,
    /// The newest frame that came while one was decoding.
    next_frame: Option<Vec<u8>>,
    /// Bumped whenever the frame is cleared, so one still decoding is
    /// dropped.
    epoch: u64,
    focus: FocusHandle,
    address: Entity<InputState>,
    /// The page URL and navigate state the address last followed.
    address_for: Option<(String, bool)>,
    /// The live view's bounds as last drawn, in window px.
    bounds: Rc<Cell<Bounds<Pixels>>>,
    /// The tab's width as last drawn, in CSS px.
    width: Rc<Cell<f32>>,
    /// The press the next release ends: its button, clicks and the page
    /// point it last moved to.
    held: Option<(MouseButton, u8, f64, f64)>,
    held_keys: HeldKeys,
    /// Moves and wheel turns wait for the next frame, newest move and
    /// summed turns winning; anything else sends them first.
    pending_move: Option<ComputerInput>,
    pending_wheel: Option<ComputerInput>,
    frame_asked: bool,
    /// Take control once the service runs the Computer (a smoke scene).
    hold_when_ready: bool,
    _poll: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
    /// Acknowledgements sent, in a test.
    #[cfg(test)]
    acks: usize,
}

impl ComputerView {
    pub(crate) fn new(
        agent_id: String,
        client: ServerClient,
        panel: WeakEntity<ReactiveAutomations>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let focus = cx.focus_handle().tab_stop(true);
        let address = cx.new(|cx| InputState::new(window, cx));
        let view = cx.weak_entity();
        let subscriptions = vec![
            cx.subscribe_in(&address, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.navigate_to_address(cx);
                }
            }),
            cx.on_focus(&focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&focus, window, |this, _, cx| {
                this.release_held(cx);
                cx.notify();
            }),
            // Before the app's bindings, which take Tab (F22).
            cx.intercept_keystrokes(move |event, window, cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                if view.update(cx, |this, cx| this.key_down(&event.keystroke, window, cx)) {
                    window.prevent_default();
                    cx.stop_propagation();
                }
            }),
        ];
        cx.on_release(|this, cx| {
            for image in this.frame.take().into_iter().chain(this.retired.drain(..)) {
                cx.drop_image(image, None);
            }
        })
        .detach();
        let poll = cfg!(not(test)).then(|| {
            cx.spawn(async move |this, cx| loop {
                cx.background_executor().timer(CLOSED_POLL).await;
                if this.update(cx, |this, cx| this.poll(cx)).is_err() {
                    break;
                }
            })
        });
        Self {
            agent_id,
            client,
            panel,
            state: None,
            error: None,
            fetch_failed: false,
            stream_failed: false,
            pending: None,
            revision: 0,
            gate_seen: (false, false),
            live: Live::default(),
            frame: None,
            frame_drawn: false,
            retired: Vec::new(),
            decoding: false,
            next_frame: None,
            epoch: 0,
            focus,
            address,
            address_for: None,
            bounds: Rc::new(Cell::new(Bounds::default())),
            width: Rc::new(Cell::new(0.0)),
            held: None,
            held_keys: HeldKeys::default(),
            pending_move: None,
            pending_wheel: None,
            frame_asked: false,
            hold_when_ready: false,
            _poll: poll,
            _subscriptions: subscriptions,
            #[cfg(test)]
            acks: 0,
        }
    }

    /// Take control as soon as the service runs the Computer
    /// (`BRUHS_DESKTOP_SMOKE_SCENE=bruh-computer-held:<id>`).
    pub(super) fn take_control_when_ready(&mut self) {
        self.hold_when_ready = true;
    }

    fn gate(&self, cx: &App) -> Gate {
        self.panel
            .upgrade()
            .map(|panel| panel.read(cx).computer_gate(&self.agent_id))
            .unwrap_or_default()
    }

    fn controls(&self, cx: &App) -> ComputerControls {
        let gate = self.gate(cx);
        computer_controls(self.state.as_ref(), gate.on, gate.ready, true)
    }

    fn viewport(&self) -> Viewport {
        self.state
            .as_ref()
            .map(|state| state.viewport)
            .unwrap_or_default()
    }

    /// The line at the top of the tab.
    pub(super) fn status(&self, cx: &App) -> String {
        let gate = self.gate(cx);
        if !gate.ready {
            return gate.reason.unwrap_or_default().to_string();
        }
        computer_status_line(self.state.as_ref(), gate.on, &gate.name)
    }

    /// The line under the live view.
    pub(super) fn hint(&self, window: &Window, cx: &App) -> Option<&'static str> {
        let controls = self.controls(cx);
        if controls.input {
            self.focus.is_focused(window).then_some(LIVE_VIEW_HINT)
        } else if self.state.as_ref().is_some_and(|state| state.headed) {
            Some(USE_THE_WINDOW)
        } else if controls.take {
            Some(TAKE_CONTROL_FIRST)
        } else {
            None
        }
    }

    #[cfg(test)]
    pub(super) fn address_editable(&self, cx: &App) -> bool {
        self.controls(cx).navigate
    }

    /// Follow the panel and the state: read the state when the Computer
    /// becomes usable, and watch the browser while it runs.
    fn sync(&mut self, cx: &mut Context<'_, Self>) {
        let gate = self.gate(cx);
        let seen = (gate.on, gate.ready);
        if seen != self.gate_seen {
            self.gate_seen = seen;
            if gate.on && gate.ready {
                self.load(None, cx);
            }
        }
        let wanted = self
            .state
            .as_ref()
            .and_then(|state| state.session_id.clone())
            .filter(|_| gate.on && gate.ready);
        match wanted {
            Some(session) => {
                if self.live.session.as_deref() != Some(session.as_str()) {
                    self.connect(session, cx);
                }
            }
            None => {
                if self.live.session.is_some() || self.frame.is_some() {
                    self.disconnect();
                    self.clear_frame();
                }
            }
        }
        if self.hold_when_ready && gate.on && gate.ready && self.pending.is_none() {
            self.hold_when_ready = false;
            self.request(ComputerRequest::TakeControl, cx);
        }
    }

    /// While no live view runs, look for a browser the bruh opens. A
    /// request on its way answers for itself.
    fn poll(&mut self, cx: &mut Context<'_, Self>) {
        let gate = self.gate(cx);
        if gate.on && gate.ready && self.pending.is_none() && self.live.session.is_none() {
            self.load(None, cx);
        }
    }

    /// Read the state from the service; with `reconnect`, connect again
    /// afterwards if that socket is still the latest.
    fn load(&mut self, reconnect: Option<u64>, cx: &mut Context<'_, Self>) {
        self.revision = self.revision.wrapping_add(1);
        #[cfg(test)]
        {
            // Tests never reach a server; they apply states themselves.
            let _ = (reconnect, cx);
        }
        #[cfg(not(test))]
        {
            let requested = self.revision;
            let client = self.client.clone();
            let agent_id = self.agent_id.clone();
            let reading = spawn_tokio(cx, async move { client.computer_state(&agent_id).await });
            cx.spawn(async move |this, cx| {
                let result = reading.await;
                let _ = this.update(cx, |this, cx| {
                    if this.revision == requested {
                        match result {
                            Ok(next) => {
                                if this.fetch_failed {
                                    this.fetch_failed = false;
                                    this.error = None;
                                }
                                this.state = Some(next);
                            }
                            Err(error) => {
                                this.fetch_failed = true;
                                this.error = Some(error.to_string());
                            }
                        }
                    }
                    if reconnect.is_some_and(|generation| generation == this.live.generation) {
                        this.live.session = None;
                    }
                    this.sync(cx);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// Ask the service for something of this Computer, unless a request
    /// is already on its way.
    fn request(&mut self, request: ComputerRequest, cx: &mut Context<'_, Self>) {
        if self.pending.is_some() {
            return;
        }
        let Some(panel) = self.panel.upgrade() else {
            return;
        };
        let agent_id = self.agent_id.clone();
        let Some(answer) = panel.update(cx, |panel, cx| {
            panel.computer_request(&agent_id, request, cx)
        }) else {
            return;
        };
        self.pending = Some(request);
        self.error = None;
        self.fetch_failed = false;
        self.stream_failed = false;
        self.revision = self.revision.wrapping_add(1);
        cx.spawn(async move |this, cx| {
            let result = answer.await;
            let _ = this.update(cx, |this, cx| {
                this.pending = None;
                match result {
                    Ok(next) => this.state = Some(next),
                    Err(error) => this.error = Some(error),
                }
                this.sync(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Open the live view of browser `session`, replacing any socket.
    fn connect(&mut self, session: String, cx: &mut Context<'_, Self>) {
        self.disconnect();
        self.live.session = Some(session.clone());
        #[cfg(test)]
        {
            // Tests never reach a server; they apply events themselves.
            let _ = (session, cx);
            self.live.connected = true;
        }
        #[cfg(not(test))]
        {
            let generation = self.live.generation;
            let client = self.client.clone();
            let opening = spawn_tokio(cx, async move { client.screencast(&session).await });
            self.live.task = Some(cx.spawn(async move |this, cx| {
                let events = match opening.await {
                    Ok(screencast) => {
                        let events = screencast.events.clone();
                        let kept = this.update(cx, |this, cx| {
                            if this.live.generation != generation {
                                return false;
                            }
                            this.live.screencast = Some(screencast);
                            this.live.connected = true;
                            cx.notify();
                            true
                        });
                        if !matches!(kept, Ok(true)) {
                            return;
                        }
                        events
                    }
                    Err(error) => {
                        let _ = this.update(cx, |this, cx| {
                            if this.live.generation == generation {
                                this.stream_failed = true;
                                this.error = Some(error.to_string());
                                this.closed(generation, cx);
                                cx.notify();
                            }
                        });
                        return;
                    }
                };
                loop {
                    // A closed channel means the socket has gone too.
                    let event = events.recv().await.unwrap_or(ScreencastEvent::Closed);
                    let closed = event == ScreencastEvent::Closed;
                    let current = this.update(cx, |this, cx| {
                        if this.live.generation != generation {
                            return false;
                        }
                        this.apply_event(event, cx);
                        true
                    });
                    if closed || !matches!(current, Ok(true)) {
                        break;
                    }
                }
            }));
        }
    }

    /// Close the socket, if any, and cancel a pending retry.
    fn disconnect(&mut self) {
        self.live.generation = self.live.generation.wrapping_add(1);
        self.live.retry = None;
        self.live.task = None;
        self.live.screencast = None;
        self.live.session = None;
        self.live.connected = false;
        self.live.backoff.framed = false;
        self.held = None;
        self.pending_move = None;
        self.pending_wheel = None;
    }

    /// Clear the frame on screen; a frame still decoding is dropped.
    fn clear_frame(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.next_frame = None;
        let old = self.frame.take();
        self.retire(old);
    }

    /// The socket closed: wait, read the state again, then connect to what
    /// it says.
    fn closed(&mut self, generation: u64, cx: &mut Context<'_, Self>) {
        if generation != self.live.generation || self.live.session.is_none() {
            return;
        }
        self.live.screencast = None;
        self.live.connected = false;
        let wait = self.live.backoff.closed();
        self.live.retry = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(wait))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.live.generation == generation {
                    this.live.retry = None;
                    this.load(Some(generation), cx);
                }
            });
        }));
    }

    /// One event from the live view.
    fn apply_event(&mut self, event: ScreencastEvent, cx: &mut Context<'_, Self>) {
        match event {
            ScreencastEvent::State(next) => {
                self.revision = self.revision.wrapping_add(1);
                self.state = Some(next);
                self.sync(cx);
            }
            ScreencastEvent::Error(error) => {
                self.stream_failed = true;
                self.error = Some(error);
            }
            ScreencastEvent::Frame(bytes) => {
                if self.live.backoff.frame() && self.stream_failed {
                    self.stream_failed = false;
                    self.error = None;
                }
                if self.decoding {
                    self.next_frame = Some(bytes);
                } else {
                    self.decode(bytes, cx);
                }
            }
            ScreencastEvent::Closed => self.closed(self.live.generation, cx),
        }
        cx.notify();
    }

    #[cfg(test)]
    pub(super) fn apply_event_for_test(
        &mut self,
        event: ScreencastEvent,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.apply_event(event, cx);
    }

    /// Decode a frame off the main thread.
    fn decode(&mut self, bytes: Vec<u8>, cx: &mut Context<'_, Self>) {
        self.decoding = true;
        let (generation, epoch) = (self.live.generation, self.epoch);
        cx.spawn(async move |this, cx| {
            let decoded = cx
                .background_executor()
                .spawn(async move { decode_frame(&bytes) })
                .await;
            let _ = this.update(cx, |this, cx| this.decoded(generation, epoch, decoded, cx));
        })
        .detach();
    }

    /// A frame is decoded: show it, acknowledge it (a frame that failed to
    /// decode too, so the next comes) and decode the newest one waiting.
    fn decoded(
        &mut self,
        generation: u64,
        epoch: u64,
        decoded: Result<Arc<RenderImage>, String>,
        cx: &mut Context<'_, Self>,
    ) {
        self.decoding = false;
        if epoch == self.epoch {
            match decoded {
                Ok(image) => {
                    let old = self.frame.replace(image);
                    self.retire(old);
                }
                Err(error) => tracing::debug!("Live view frame skipped: {error}"),
            }
        }
        if generation == self.live.generation {
            self.ack();
        }
        if let Some(next) = self.next_frame.take() {
            self.decode(next, cx);
        }
        cx.notify();
    }

    /// Let go of a frame no longer shown: one that was drawn waits for the
    /// next draw to leave the atlas.
    fn retire(&mut self, old: Option<Arc<RenderImage>>) {
        if std::mem::take(&mut self.frame_drawn) {
            self.retired.extend(old);
        }
    }

    fn ack(&mut self) {
        #[cfg(test)]
        {
            self.acks += 1;
        }
        if let Some(screencast) = &self.live.screencast {
            screencast.ack();
        }
    }

    /// Owner input, only while the owner holds control.
    fn send_now(&mut self, input: ComputerInput, cx: &mut Context<'_, Self>) {
        if !self.controls(cx).input {
            return;
        }
        let Some(panel) = self.panel.upgrade() else {
            return;
        };
        let screencast = self.live.screencast.as_ref();
        panel.update(cx, |panel, _| panel.screencast_input(screencast, input));
    }

    /// Send waiting moves and wheel turns, then `input`, so order holds.
    fn send(&mut self, input: ComputerInput, cx: &mut Context<'_, Self>) {
        self.flush(cx);
        self.send_now(input, cx);
    }

    fn flush(&mut self, cx: &mut Context<'_, Self>) {
        self.frame_asked = false;
        for input in [self.pending_move.take(), self.pending_wheel.take()]
            .into_iter()
            .flatten()
        {
            self.send_now(input, cx);
        }
    }

    /// Send waiting moves and turns with the next frame: one per frame.
    fn send_next_frame(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.frame_asked {
            return;
        }
        self.frame_asked = true;
        let view = cx.weak_entity();
        window.on_next_frame(move |_, cx| {
            let _ = view.update(cx, |this, cx| this.flush(cx));
        });
    }

    /// A press whose release the view won't see (focus lost, F6) is
    /// released where it last was, and so is every key still held.
    fn release_held(&mut self, cx: &mut Context<'_, Self>) {
        if let Some((button, clicks, x, y)) = self.held.take() {
            self.send(
                ComputerInput::Mouse {
                    kind: MouseKind::Up,
                    x,
                    y,
                    button,
                    clicks,
                    modifiers: 0,
                },
                cx,
            );
        }
        for input in self.held_keys.release_all() {
            self.send(input, cx);
        }
    }

    /// The page point under `position`, a window point; with `clamp`, one
    /// outside the frame clamps to its edge.
    fn page_point(&self, position: Point<Pixels>, clamp: bool) -> Option<(f64, f64)> {
        let bounds = self.bounds.get();
        let x = f64::from(f32::from(position.x - bounds.origin.x));
        let y = f64::from(f32::from(position.y - bounds.origin.y));
        let shown = (
            f64::from(f32::from(bounds.size.width)),
            f64::from(f32::from(bounds.size.height)),
        );
        if clamp {
            to_viewport_clamped(x, y, shown, self.viewport())
        } else {
            to_viewport(x, y, shown, self.viewport())
        }
    }

    fn mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.controls(cx).input {
            return;
        }
        self.focus.focus(window, cx);
        let button = page_button(event.button);
        if button == MouseButton::None {
            return;
        }
        let Some((x, y)) = self.page_point(event.position, false) else {
            return;
        };
        let clicks = u8::try_from(event.click_count.clamp(1, 3)).unwrap_or(1);
        self.held = Some((button, clicks, x, y));
        self.send(
            ComputerInput::Mouse {
                kind: MouseKind::Down,
                x,
                y,
                button,
                clicks,
                modifiers: modifiers(&event.modifiers),
            },
            cx,
        );
    }

    /// A move over the view, or anywhere while a press of the view is held.
    fn mouse_moved(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.controls(cx).input {
            return;
        }
        let dragging = self.held.is_some();
        if !dragging && !self.bounds.get().contains(&event.position) {
            return;
        }
        let button = match self.held {
            Some((button, ..)) => button,
            None => event.pressed_button.map_or(MouseButton::None, page_button),
        };
        let Some((x, y)) = self.page_point(event.position, dragging) else {
            return;
        };
        if let Some(held) = self.held.as_mut() {
            (held.2, held.3) = (x, y);
        }
        self.pending_move = Some(ComputerInput::Mouse {
            kind: MouseKind::Move,
            x,
            y,
            button,
            clicks: 0,
            modifiers: modifiers(&event.modifiers),
        });
        self.send_next_frame(window, cx);
    }

    /// A release ends the view's press wherever it happens, clamped to the
    /// frame, so the page always sees the button come up.
    fn mouse_released(&mut self, event: &MouseUpEvent, cx: &mut Context<'_, Self>) {
        let Some((button, clicks, x, y)) = self.held.take() else {
            return;
        };
        let (x, y) = self.page_point(event.position, true).unwrap_or((x, y));
        self.send(
            ComputerInput::Mouse {
                kind: MouseKind::Up,
                x,
                y,
                button,
                clicks,
                modifiers: modifiers(&event.modifiers),
            },
            cx,
        );
    }

    fn wheel(&mut self, event: &ScrollEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !self.controls(cx).input {
            return;
        }
        // The page scrolls, not the tab.
        cx.stop_propagation();
        let Some((x, y)) = self.page_point(event.position, false) else {
            return;
        };
        let (dx, dy) = wheel_px(&event.delta);
        let (before_x, before_y) = match self.pending_wheel {
            Some(ComputerInput::Wheel { dx, dy, .. }) => (dx, dy),
            _ => (0.0, 0.0),
        };
        self.pending_wheel = Some(ComputerInput::Wheel {
            x,
            y,
            dx: (before_x + dx).clamp(-MAX_SCROLL_PX, MAX_SCROLL_PX),
            dy: (before_y + dy).clamp(-MAX_SCROLL_PX, MAX_SCROLL_PX),
        });
        self.send_next_frame(window, cx);
    }

    /// A key down in this window, before the app's bindings. True when the
    /// live view took it.
    fn key_down(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if !self.focus.is_focused(window) || !self.controls(cx).input {
            return false;
        }
        let Some((key, code)) = dom_key(keystroke) else {
            return false;
        };
        match self
            .held_keys
            .route(&key, &code, modifiers(&keystroke.modifiers), true)
        {
            KeyAction::Send(input) => self.send(input, cx),
            KeyAction::Paste => {
                let text = cx
                    .read_from_clipboard()
                    .and_then(|item| item.text())
                    .unwrap_or_default();
                if !text.is_empty() {
                    self.send(ComputerInput::Text { text }, cx);
                }
            }
            KeyAction::Leave => {
                self.release_held(cx);
                self.address.focus_handle(cx).focus(window, cx);
            }
            KeyAction::App => return false,
        }
        true
    }

    /// Key ups come here: bindings never take them. A release whose press
    /// went to the page goes there too.
    fn key_up(&mut self, event: &KeyUpEvent, _: &mut Window, cx: &mut Context<'_, Self>) {
        if !self.controls(cx).input {
            return;
        }
        let Some((key, code)) = dom_key(&event.keystroke) else {
            return;
        };
        let held = modifiers(&event.keystroke.modifiers);
        if let KeyAction::Send(input) = self.held_keys.route(&key, &code, held, false) {
            self.send(input, cx);
            cx.stop_propagation();
        }
    }

    /// Enter in the address: go there.
    fn navigate_to_address(&mut self, cx: &mut Context<'_, Self>) {
        if !self.controls(cx).navigate {
            return;
        }
        let url = self.address.read(cx).value().trim().to_string();
        if !url.is_empty() {
            self.send(ComputerInput::Navigate { url }, cx);
        }
    }

    /// The address shows the page's URL, unless the owner is typing one.
    fn sync_address(&mut self, navigate: bool, window: &mut Window, cx: &mut Context<'_, Self>) {
        let url = self
            .state
            .as_ref()
            .and_then(|state| state.url.clone())
            .unwrap_or_default();
        let followed = (url.clone(), navigate);
        if self.address_for.as_ref() == Some(&followed) {
            return;
        }
        self.address_for = Some(followed);
        let typing = navigate && self.address.focus_handle(cx).is_focused(window);
        if !typing {
            self.address
                .update(cx, |input, cx| input.set_value(url, window, cx));
        }
    }

    /// A toolbar button (web `Button` outline). It stays enabled while its
    /// request runs, dimmed, so focus stays on it; `request` ignores presses
    /// until the answer comes.
    fn button(
        id: &'static str,
        selector: &'static str,
        text: &'static str,
        disabled: bool,
        busy: bool,
        cx: &Context<'_, Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<'_, Self>) + 'static,
    ) -> Stateful<Div> {
        let button = web_button(
            id,
            WebButtonVariant::Outline,
            WebButtonSize::Default,
            ButtonState::disabled_if(disabled),
        )
        .debug_selector(move || selector.to_string())
        .aria_label(text)
        .when(busy, |el| el.opacity(BUSY_OPACITY))
        .child(text);
        if disabled {
            button.cursor_not_allowed()
        } else {
            on_press_view(button, cx, action)
        }
    }

    fn render_toolbar(&self, controls: ComputerControls, cx: &Context<'_, Self>) -> Div {
        let busy = |requests: &[ComputerRequest]| {
            self.pending
                .is_some_and(|pending| requests.contains(&pending))
        };
        let address = div()
            .debug_selector(|| "desktop-computer-address".to_string())
            .flex_grow(1.0)
            .flex_shrink(1.0)
            .flex_basis(px(ADDRESS_BASIS_PX))
            .min_w_0()
            .child(
                Styled::h(
                    Input::new(&self.address)
                        .aria_label(ADDRESS_LABEL)
                        .readonly(!controls.navigate),
                    px(theme::CONTROL_PX),
                )
                .px(px(FIELD_X_PADDING_PX))
                .font_family(theme::CODE_FONT_FAMILY)
                .text_size(px(FIELD_TEXT_PX))
                .when(!controls.navigate, |el| {
                    el.text_color(theme::rambutan_400())
                }),
            );
        let offline = !self.live.connected;
        div()
            .min_w_0()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(address)
            .when(controls.navigate, |row| {
                row.child(Self::button(
                    "desktop-computer-back",
                    "desktop-computer-back",
                    BACK,
                    offline,
                    false,
                    cx,
                    |this, _, cx| this.send(ComputerInput::Back, cx),
                ))
                .child(Self::button(
                    "desktop-computer-reload",
                    "desktop-computer-reload",
                    RELOAD,
                    offline,
                    false,
                    cx,
                    |this, _, cx| this.send(ComputerInput::Reload, cx),
                ))
            })
            // One button each for control and the browser, whose label
            // follows the state, so focus stays on it.
            .when(controls.take || controls.hand_back, |row| {
                let (selector, text) = if controls.hand_back {
                    ("desktop-computer-hand-back", HAND_BACK)
                } else {
                    ("desktop-computer-take", TAKE_CONTROL)
                };
                row.child(Self::button(
                    "desktop-computer-control",
                    selector,
                    text,
                    false,
                    busy(&[ComputerRequest::TakeControl, ComputerRequest::HandBack]),
                    cx,
                    |this, _, cx| {
                        let hand_back = this.controls(cx).hand_back;
                        this.request(
                            if hand_back {
                                ComputerRequest::HandBack
                            } else {
                                ComputerRequest::TakeControl
                            },
                            cx,
                        );
                    },
                ))
            })
            .when(controls.window, |row| {
                row.child(Self::button(
                    "desktop-computer-window",
                    "desktop-computer-window",
                    OPEN_IN_WINDOW,
                    false,
                    busy(&[ComputerRequest::OpenWindow]),
                    cx,
                    |this, _, cx| this.request(ComputerRequest::OpenWindow, cx),
                ))
            })
            .when(controls.open || controls.close, |row| {
                let (selector, text) = if controls.close {
                    ("desktop-computer-close", CLOSE_BROWSER)
                } else {
                    ("desktop-computer-open", OPEN_BROWSER)
                };
                row.child(Self::button(
                    "desktop-computer-browser",
                    selector,
                    text,
                    false,
                    busy(&[ComputerRequest::Open, ComputerRequest::Close]),
                    cx,
                    |this, _, cx| {
                        let close = this.controls(cx).close;
                        this.request(
                            if close {
                                ComputerRequest::Close
                            } else {
                                ComputerRequest::Open
                            },
                            cx,
                        );
                    },
                ))
            })
    }

    /// The 16:10 live view: the frame drawn `object-fit: contain`, and the
    /// owner's pointer, wheel and keys while they hold control.
    fn render_live(
        &self,
        controls: ComputerControls,
        width: f32,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        let bounds = self.bounds.clone();
        let view = cx.weak_entity();
        // Measures the frame's box, and follows a press of it anywhere in
        // the window so a drag released outside still ends.
        let pointer = canvas(
            move |frame, _, _| bounds.set(frame),
            move |_, (), window, _| {
                let moved = view.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble {
                        let _ = moved.update(cx, |this, cx| this.mouse_moved(event, window, cx));
                    }
                });
                let released = view;
                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble {
                        let _ = released.update(cx, |this, cx| this.mouse_released(event, cx));
                    }
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        let live = div()
            .id("desktop-computer-live")
            .debug_selector(|| "desktop-computer-live".to_string())
            .relative()
            .flex_shrink_0()
            .w(px(width))
            .h(px(width / LIVE_RATIO))
            .overflow_hidden()
            .rounded(px(RADIUS_GRAPE_PX))
            .bg(theme::longan_900())
            .border_1()
            .border_color(theme::longan_700())
            .child(pointer)
            .when_some(self.frame.clone(), |live, frame| {
                live.child(
                    div()
                        .debug_selector(|| "desktop-computer-frame".to_string())
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .child(img(frame).size_full().object_fit(ObjectFit::Contain)),
                )
            });
        if !controls.input {
            return live;
        }
        live.track_focus(&self.focus)
            .role(Role::Image)
            .aria_label(LIVE_VIEW_LABEL)
            .focus(|style| style.border_color(theme::persimmon_400()))
            .on_any_mouse_down(cx.listener(Self::mouse_down))
            .on_scroll(cx.listener(Self::wheel))
            .on_key_up(cx.listener(Self::key_up))
    }
}

impl Render for ComputerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        for image in std::mem::take(&mut self.retired) {
            cx.drop_image(image, Some(window));
        }
        self.sync(cx);
        let gate = self.gate(cx);
        let controls = computer_controls(self.state.as_ref(), gate.on, gate.ready, true);
        // The live view, and its frame, draw only while the Computer works.
        if self.frame.is_some() && gate.on && gate.ready {
            self.frame_drawn = true;
        }
        // Focus never stays on a live view that no longer takes it.
        if !controls.input && self.focus.is_focused(window) {
            self.release_held(cx);
            self.address.focus_handle(cx).focus(window, cx);
        }
        self.sync_address(controls.navigate, window, cx);
        let layout = Layout::measured(self.width.get(), window);
        let viewport = gpui_component::zoom::css_px(window.viewport_size().width);
        let measured = self.width.get();
        let page = if measured > 0.0 { measured } else { viewport };
        let column = page.min(COLUMN_MAX_PX);
        let inner = (column - layout.padding * 2.0).max(0.0);
        let window_height = css_px(window.viewport_size().height);
        let live_width = inner.min(((window_height - LIVE_ROOM_PX) * LIVE_RATIO).max(LIVE_MIN_PX));
        let status = div()
            .id("desktop-computer-status")
            .debug_selector(|| "desktop-computer-status".to_string())
            .role(Role::Status)
            .min_w_0()
            .font_family(theme::CODE_FONT_FAMILY)
            .text_size(px(SMALL_TEXT_PX))
            .text_color(theme::rambutan_300())
            .child(self.status(cx));
        let hint: SharedString = self.hint(window, cx).unwrap_or_default().into();
        let body = div()
            .w(px(column))
            .min_w_0()
            .flex_shrink_0()
            .px(px(layout.padding))
            .py(px(PAGE_PADDING_PX))
            .flex()
            .flex_col()
            .gap_3()
            .child(status)
            .child(self.render_toolbar(controls, cx))
            .when_some(self.error.clone(), |body, message| {
                body.child(alert("desktop-computer-error", message))
            })
            .when(gate.on && gate.ready, |body| {
                body.child(
                    div()
                        .flex()
                        .justify_center()
                        .child(self.render_live(controls, live_width, cx)),
                )
            })
            .child(
                div()
                    .debug_selector(|| "desktop-computer-hint".to_string())
                    .min_h(px(LABEL_TEXT_PX * 1.5))
                    .child(small_text(hint, theme::rambutan_500())),
            );
        let width = self.width.clone();
        let entity = cx.entity_id();
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let measured = css_px(bounds.size.width);
                        if (width.get() - measured).abs() > 0.5 {
                            width.set(measured);
                            cx.notify(entity);
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .child(
                div()
                    .id("desktop-bruh-computer")
                    .debug_selector(|| "desktop-bruh-computer".to_string())
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .items_center()
                    .overflow_y_scrollbar()
                    .child(body),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::super::bruh_page::BruhTab;
    use super::{
        dom_key, modifiers, Backoff, ComputerView, HeldKeys, KeyAction, ReactiveAutomations,
    };
    use crate::server_client::ScreencastEvent;
    use crate::theme;
    use crate::views::workspace::StopAll;
    use bruhs_models::crew::{
        computer_status_line, AccessLevel, BruhAccess, BruhLook, BruhState, BruhStatus,
        CREW_PROTOCOL, LIVE_VIEW_HINT, USE_THE_WINDOW,
    };
    use bruhs_models::local_tools::{
        ComputerInput, ComputerRequest, ComputerState, Control, KeyKind, MouseButton, MouseKind,
        MODIFIER_CTRL, MODIFIER_META, MODIFIER_SHIFT,
    };
    use bruhs_models::reactive::{ReactiveAgent, ReactiveSnapshot};
    use gpui::{
        AppContext as _, ClipboardItem, Entity, Focusable as _, KeyBinding, KeyUpEvent, Keystroke,
        Modifiers, VisualTestContext,
    };
    use std::{cell::Cell, cell::RefCell, rc::Rc};

    fn snapshot() -> ReactiveSnapshot {
        ReactiveSnapshot {
            agents: vec![ReactiveAgent {
                id: "a".into(),
                name: "Ada".into(),
                instructions: String::new(),
                memory: String::new(),
                model: "claude-sonnet-4-6".into(),
                project_path: String::new(),
                enabled: true,
            }],
            bruhs: vec![BruhState {
                agent_id: "a".into(),
                name: "Ada".into(),
                look: BruhLook::default(),
                access: BruhAccess {
                    level: AccessLevel::Full,
                    ..BruhAccess::default()
                },
                effort: None,
                status: BruhStatus::Asleep,
                next_wake_ms: None,
                unread: 0,
                computer: true,
            }],
            protocol: CREW_PROTOCOL,
            ..ReactiveSnapshot::default()
        }
    }

    fn browser(control: Control, headed: bool) -> ComputerState {
        ComputerState {
            agent_id: "a".into(),
            session_id: Some("s".into()),
            url: Some("https://example.com/".into()),
            control,
            headed,
            ..ComputerState::default()
        }
    }

    /// A standalone panel window (F13) on the bruh's Computer tab.
    fn open(
        cx: &mut gpui::TestAppContext,
        width: f32,
        height: f32,
    ) -> (
        Entity<ReactiveAutomations>,
        Entity<ComputerView>,
        &mut VisualTestContext,
    ) {
        cx.update(|cx| {
            gpui_component::init(cx);
            theme::apply_bruhs_theme(cx);
        });
        let holder = Rc::new(RefCell::new(None));
        let capture = holder.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let panel = cx.new(ReactiveAutomations::new);
            *capture.borrow_mut() = Some(panel.clone());
            gpui_component::Root::new(panel, window, cx)
        });
        cx.simulate_resize(gpui::size(gpui::px(width), gpui::px(height)));
        let panel = holder.borrow().clone().unwrap();
        panel.update(cx, |panel, cx| {
            panel.loaded = true;
            panel.set_snapshot(snapshot(), cx);
            panel.open_bruh_for_test("a", BruhTab::Computer, cx);
        });
        cx.run_until_parked();
        let view = panel
            .read_with(cx, |panel, _| panel.computer_view_for_test())
            .expect("the Computer tab shows its view");
        (panel, view, cx)
    }

    fn apply(view: &Entity<ComputerView>, cx: &mut VisualTestContext, event: ScreencastEvent) {
        view.update_in(cx, |view, window, cx| {
            view.apply_event_for_test(event, window, cx);
        });
    }

    fn inputs(
        panel: &Entity<ReactiveAutomations>,
        cx: &mut VisualTestContext,
    ) -> Vec<ComputerInput> {
        panel.read_with(cx, |panel, _| panel.screencast_inputs_for_test().to_vec())
    }

    fn shown(cx: &mut VisualTestContext, selector: &'static str) -> bool {
        cx.debug_bounds(selector).is_some()
    }

    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        let bounds = cx.debug_bounds(selector).expect(selector);
        cx.simulate_click(bounds.center(), Modifiers::default());
        cx.run_until_parked();
    }

    /// A 2×2 JPEG.
    fn jpeg() -> Vec<u8> {
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut bytes)
            .encode_image(&image::RgbImage::new(2, 2))
            .unwrap();
        bytes
    }

    #[gpui::test]
    fn a_closed_browser_offers_open_and_sends_it(cx: &mut gpui::TestAppContext) {
        let (panel, view, cx) = open(cx, 900.0, 1200.0);
        let status = view.read_with(cx, |view, cx| view.status(cx));
        assert_eq!(status, computer_status_line(None, true, "Ada"));
        assert!(shown(cx, "desktop-computer-status"));
        assert!(!shown(cx, "desktop-computer-take"), "no browser to take");
        click(cx, "desktop-computer-open");
        panel.read_with(cx, |panel, _| {
            assert_eq!(
                panel.computer_requests_for_test(),
                [("a".to_string(), ComputerRequest::Open)]
            );
        });
    }

    #[gpui::test]
    fn frames_draw_and_are_acked_after_decode(cx: &mut gpui::TestAppContext) {
        let (_panel, view, cx) = open(cx, 900.0, 1200.0);
        apply(
            &view,
            cx,
            ScreencastEvent::State(browser(Control::Bruh, false)),
        );
        cx.run_until_parked();
        assert!(
            !shown(cx, "desktop-computer-frame"),
            "nothing until a frame"
        );
        apply(&view, cx, ScreencastEvent::Frame(jpeg()));
        assert_eq!(
            view.read_with(cx, |view, _| view.acks),
            0,
            "acked after decode"
        );
        cx.run_until_parked();
        assert_eq!(view.read_with(cx, |view, _| view.acks), 1);
        assert!(shown(cx, "desktop-computer-frame"));
        let first = view.read_with(cx, |view, _| view.frame.clone()).unwrap();
        let painted = cx.update(|window, _| window.has_image_atlas_entry(&first));
        assert!(painted, "the frame is drawn");
        // Two more before the next decode ends: neither is acked early.
        apply(&view, cx, ScreencastEvent::Frame(jpeg()));
        apply(&view, cx, ScreencastEvent::Frame(jpeg()));
        assert_eq!(view.read_with(cx, |view, _| view.acks), 1);
        cx.run_until_parked();
        assert_eq!(view.read_with(cx, |view, _| view.acks), 3);
        // The frame it replaced leaves the atlas.
        let kept = cx.update(|window, _| window.has_image_atlas_entry(&first));
        assert!(!kept, "the old frame is dropped");
        // A frame that can't be decoded is still acked, so the next comes.
        apply(&view, cx, ScreencastEvent::Frame(vec![1, 2, 3]));
        cx.run_until_parked();
        assert_eq!(view.read_with(cx, |view, _| view.acks), 4);
        assert!(
            shown(cx, "desktop-computer-frame"),
            "the last good frame stays"
        );
    }

    #[gpui::test]
    fn owner_keys_go_to_the_page_except_app_shortcuts(cx: &mut gpui::TestAppContext) {
        // The app's own modifier.
        let (stop, paste) = if cfg!(target_os = "macos") {
            ("cmd-.", "cmd-v")
        } else {
            ("ctrl-.", "ctrl-v")
        };
        let stops = Rc::new(Cell::new(0));
        let counted = stops.clone();
        cx.update(|cx| {
            cx.bind_keys([KeyBinding::new(stop, StopAll, None)]);
            cx.on_action(move |_: &StopAll, _| counted.set(counted.get() + 1));
        });
        let (panel, view, cx) = open(cx, 900.0, 1200.0);
        apply(
            &view,
            cx,
            ScreencastEvent::State(browser(Control::Owner, false)),
        );
        cx.run_until_parked();
        let focus = |view: &Entity<ComputerView>, cx: &mut VisualTestContext| {
            view.update_in(cx, |view, window, cx| view.focus.focus(window, cx));
            cx.run_until_parked();
        };
        focus(&view, cx);
        let hint = view.update_in(cx, |view, window, cx| view.hint(window, cx));
        assert_eq!(hint, Some(LIVE_VIEW_HINT), "the hint shows while focused");
        cx.simulate_keystrokes("a");
        cx.simulate_event(KeyUpEvent {
            keystroke: Keystroke::parse("a").unwrap(),
        });
        let key = |kind, key: &str, code: &str, modifiers| ComputerInput::Key {
            kind,
            key: key.into(),
            code: code.into(),
            modifiers,
        };
        assert_eq!(
            inputs(&panel, cx),
            [
                key(KeyKind::Down, "a", "KeyA", 0),
                key(KeyKind::Up, "a", "KeyA", 0)
            ]
        );
        // Tab goes to the page, not to the next control.
        cx.simulate_keystrokes("tab");
        assert_eq!(
            inputs(&panel, cx).last(),
            Some(&key(KeyKind::Down, "Tab", "Tab", 0))
        );
        assert!(view.update_in(cx, |view, window, _| view.focus.is_focused(window)));
        cx.simulate_event(KeyUpEvent {
            keystroke: Keystroke::parse("tab").unwrap(),
        });
        let sent = inputs(&panel, cx).len();
        // ⌘. stays with the app and reaches its action.
        cx.simulate_keystrokes(stop);
        assert_eq!(inputs(&panel, cx).len(), sent, "no input");
        assert_eq!(stops.get(), 1, "the stop action ran");
        // ⌘V pastes the clipboard as text.
        cx.write_to_clipboard(ClipboardItem::new_string("pw".into()));
        cx.simulate_keystrokes(paste);
        assert_eq!(
            inputs(&panel, cx).last(),
            Some(&ComputerInput::Text { text: "pw".into() })
        );
        // F6 leaves for the address, releasing a key still held.
        cx.simulate_keystrokes("shift-b");
        assert_eq!(
            inputs(&panel, cx).last(),
            Some(&key(KeyKind::Down, "B", "KeyB", MODIFIER_SHIFT))
        );
        cx.simulate_keystrokes("f6");
        assert_eq!(
            inputs(&panel, cx).last(),
            Some(&key(KeyKind::Up, "B", "KeyB", 0)),
            "the held key comes up"
        );
        let at_address = view.update_in(cx, |view, window, cx| {
            view.address.focus_handle(cx).is_focused(window)
        });
        assert!(at_address, "focus is on the address");
        let hint = view.update_in(cx, |view, window, cx| view.hint(window, cx));
        assert_eq!(hint, None, "the hint goes");
    }

    #[gpui::test]
    fn clicks_map_through_the_letterbox(cx: &mut gpui::TestAppContext) {
        let (panel, view, cx) = open(cx, 640.0, 480.0);
        apply(
            &view,
            cx,
            ScreencastEvent::State(browser(Control::Owner, false)),
        );
        cx.run_until_parked();
        let bounds = cx.debug_bounds("desktop-computer-live").unwrap();
        cx.simulate_mouse_down(
            bounds.center(),
            gpui::MouseButton::Left,
            Modifiers::default(),
        );
        assert_eq!(
            inputs(&panel, cx),
            [ComputerInput::Mouse {
                kind: MouseKind::Down,
                x: 640.0,
                y: 400.0,
                button: MouseButton::Left,
                clicks: 1,
                modifiers: 0,
            }]
        );
        // A drag out past the left edge, released there, ends inside the page.
        let outside = gpui::point(bounds.left() - gpui::px(40.0), bounds.center().y);
        cx.simulate_mouse_move(outside, gpui::MouseButton::Left, Modifiers::default());
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
        });
        cx.simulate_mouse_up(outside, gpui::MouseButton::Left, Modifiers::default());
        let sent = inputs(&panel, cx);
        assert!(
            matches!(
                sent[1],
                ComputerInput::Mouse { kind: MouseKind::Move, x, button: MouseButton::Left, .. } if x == 0.0
            ),
            "{sent:?}"
        );
        assert!(
            matches!(
                sent[2],
                ComputerInput::Mouse { kind: MouseKind::Up, x, button: MouseButton::Left, clicks: 1, .. } if x == 0.0
            ),
            "{sent:?}"
        );
        assert_eq!(sent.len(), 3);
        // A press on the frame focuses the view.
        assert!(view.update_in(cx, |view, window, _| view.focus.is_focused(window)));
    }

    #[gpui::test]
    fn controls_follow_the_state(cx: &mut gpui::TestAppContext) {
        let (panel, view, cx) = open(cx, 900.0, 1200.0);
        apply(
            &view,
            cx,
            ScreencastEvent::State(browser(Control::Bruh, false)),
        );
        cx.run_until_parked();
        assert!(shown(cx, "desktop-computer-take"));
        assert!(shown(cx, "desktop-computer-window"));
        assert!(shown(cx, "desktop-computer-close"));
        assert!(!shown(cx, "desktop-computer-hand-back"));
        assert!(!shown(cx, "desktop-computer-back"));
        assert!(!view.read_with(cx, |view, cx| view.address_editable(cx)));
        click(cx, "desktop-computer-window");
        panel.read_with(cx, |panel, _| {
            assert_eq!(
                panel.computer_requests_for_test(),
                [("a".to_string(), ComputerRequest::OpenWindow)]
            );
        });

        apply(
            &view,
            cx,
            ScreencastEvent::State(browser(Control::Owner, false)),
        );
        cx.run_until_parked();
        assert!(shown(cx, "desktop-computer-hand-back"));
        assert!(shown(cx, "desktop-computer-back"));
        assert!(shown(cx, "desktop-computer-reload"));
        assert!(!shown(cx, "desktop-computer-take"));
        assert!(view.read_with(cx, |view, cx| view.address_editable(cx)));
        let url = view.read_with(cx, |view, cx| view.address.read(cx).value().to_string());
        assert_eq!(url, "https://example.com/");

        apply(
            &view,
            cx,
            ScreencastEvent::State(browser(Control::Owner, true)),
        );
        cx.run_until_parked();
        assert!(shown(cx, "desktop-computer-hand-back"));
        assert!(!shown(cx, "desktop-computer-take"));
        assert!(!shown(cx, "desktop-computer-window"));
        assert!(!shown(cx, "desktop-computer-back"));
        let (status, hint) = view.update_in(cx, |view, window, cx| {
            (view.status(cx), view.hint(window, cx))
        });
        assert_eq!(
            status,
            computer_status_line(Some(&browser(Control::Owner, true)), true, "Ada")
        );
        assert_eq!(hint, Some(USE_THE_WINDOW));

        // A failed request's state error shows on the status line.
        let mut failed = browser(Control::Bruh, false);
        failed.error = Some("The window couldn't open.".into());
        apply(&view, cx, ScreencastEvent::State(failed));
        cx.run_until_parked();
        let status = view.read_with(cx, |view, cx| view.status(cx));
        assert_eq!(status, "The window couldn't open.");
    }

    #[test]
    fn the_backoff_grows_until_a_frame_arrives() {
        let mut backoff = Backoff::default();
        let waits: Vec<u64> = (0..6).map(|_| backoff.closed()).collect();
        assert_eq!(waits, [1_000, 2_000, 4_000, 8_000, 10_000, 10_000]);
        assert!(backoff.frame(), "a socket's first frame resets the wait");
        assert!(!backoff.frame());
        assert_eq!(backoff.closed(), 1_000);
        assert!(backoff.frame(), "a new socket's first frame is first again");
    }

    #[test]
    fn a_key_released_with_the_app_modifier_held_still_reaches_the_page() {
        let mut held = HeldKeys::default();
        assert!(matches!(
            held.route("a", "KeyA", 0, true),
            KeyAction::Send(_)
        ));
        // A repeat doesn't hold the key twice.
        assert!(matches!(
            held.route("a", "KeyA", 0, true),
            KeyAction::Send(_)
        ));
        // Its release with ⌘ (or Ctrl) held still goes to the page.
        let app = if cfg!(target_os = "macos") {
            MODIFIER_META
        } else {
            MODIFIER_CTRL
        };
        assert_eq!(
            held.route("a", "KeyA", app, false),
            KeyAction::Send(ComputerInput::Key {
                kind: KeyKind::Up,
                key: "a".into(),
                code: "KeyA".into(),
                modifiers: app,
            })
        );
        // A release whose press never reached the page stays with the app.
        assert_eq!(held.route("a", "KeyA", 0, false), KeyAction::App);
        assert_eq!(held.route(".", "Period", app, true), KeyAction::App);
        assert_eq!(held.route("v", "KeyV", app, true), KeyAction::Paste);
        assert_eq!(held.route("v", "KeyV", app, false), KeyAction::App);
        assert_eq!(held.route("F6", "F6", 0, true), KeyAction::Leave);
        assert!(held.release_all().is_empty());
    }

    #[test]
    fn gpui_keys_become_dom_keys() {
        let dom = |source: &'static str| {
            let keystroke = Keystroke::parse(source).unwrap().with_simulated_ime();
            dom_key(&keystroke)
        };
        let pair = |key: &str, code: &str| Some((key.to_string(), code.to_string()));
        assert_eq!(dom("enter"), pair("Enter", "Enter"));
        assert_eq!(dom("backspace"), pair("Backspace", "Backspace"));
        assert_eq!(dom("tab"), pair("Tab", "Tab"));
        assert_eq!(dom("escape"), pair("Escape", "Escape"));
        assert_eq!(dom("space"), pair(" ", "Space"));
        assert_eq!(dom("left"), pair("ArrowLeft", "ArrowLeft"));
        assert_eq!(dom("down"), pair("ArrowDown", "ArrowDown"));
        assert_eq!(dom("pageup"), pair("PageUp", "PageUp"));
        assert_eq!(dom("pagedown"), pair("PageDown", "PageDown"));
        assert_eq!(dom("home"), pair("Home", "Home"));
        assert_eq!(dom("end"), pair("End", "End"));
        assert_eq!(dom("delete"), pair("Delete", "Delete"));
        assert_eq!(dom("f1"), pair("F1", "F1"));
        assert_eq!(dom("f12"), pair("F12", "F12"));
        assert_eq!(dom("a"), pair("a", "KeyA"));
        assert_eq!(dom("shift-a"), pair("A", "KeyA"));
        assert_eq!(dom("7"), pair("7", "Digit7"));
        assert_eq!(dom("."), pair(".", "Period"));
        assert_eq!(dom("cmd-v"), pair("v", "KeyV"));
        assert_eq!(dom("f13"), None, "keys the page view doesn't know stay");
        let all = Modifiers {
            control: true,
            alt: true,
            shift: true,
            platform: true,
            function: false,
        };
        assert_eq!(modifiers(&all), 15);
        assert_eq!(modifiers(&Modifiers::command()), MODIFIER_META);
    }
}
