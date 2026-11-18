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
mod input;
mod live;
#[cfg(test)]
mod tests;

use self::live::Live;
use super::bruh_page::{
    alert, small_text, Layout, LABEL_TEXT_PX, PAGE_PADDING_PX, RADIUS_GRAPE_PX, SMALL_TEXT_PX,
};
use super::create_bruh::{FIELD_TEXT_PX, FIELD_X_PADDING_PX};
use super::ReactiveAutomations;
#[cfg(not(test))]
use crate::app_state::spawn_tokio;
use crate::server_client::ServerClient;
use crate::theme;
use crate::views::web_button::{web_button, ButtonState, WebButtonSize, WebButtonVariant};
use bruhs_models::agent::AgentProvider;
use bruhs_models::crew::{
    computer_controls, computer_on, computer_status_line, ComputerControls, ADDRESS_LABEL, BACK,
    CLOSE_BROWSER, HAND_BACK, LIVE_VIEW_HINT, LIVE_VIEW_LABEL, OPEN_BROWSER, OPEN_IN_WINDOW,
    RELOAD, TAKE_CONTROL, TAKE_CONTROL_FIRST, USE_THE_WINDOW,
};
use bruhs_models::local_tools::{
    ComputerInput, ComputerRequest, ComputerState, HeldKeys, MouseButton, Viewport,
};
use bruhs_ui::input::{Input, InputEvent, InputState};
use bruhs_ui::press::on_press_view;
use bruhs_ui::scroll::ScrollableElement;
use bruhs_ui::zoom::{css_px, px};
use gpui::prelude::*;
use gpui::Focusable as _;
use gpui::{
    canvas, div, img, App, Bounds, Context, DispatchPhase, Div, Entity, FocusHandle,
    HitboxBehavior, MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels, RenderImage, Role,
    SharedString, Stateful, Subscription, Task, WeakEntity, Window,
};
use std::{cell::Cell, rc::Rc, sync::Arc, time::Duration};

/// How often the tab asks for the state while no live view runs, so a
/// browser the bruh opens shows up (web `CLOSED_POLL_MS`).
const CLOSED_POLL: Duration = Duration::from_secs(5);
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
/// The app's own modifier is ⌘ on a Mac, Ctrl elsewhere.
const MAC: bool = cfg!(target_os = "macos");

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
            // Switching apps keeps focus on the view, but the keys and the
            // button held then come up elsewhere: release them here.
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.release_held(cx);
                }
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
        hint: Option<&'static str>,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        let bounds = self.bounds.clone();
        let view = cx.weak_entity();
        // Measures the frame's box, and follows a press of it anywhere in
        // the window so a drag released outside still ends. Its hitbox says
        // whether the pointer is over the view, not over a palette or dialog
        // drawn on top.
        let pointer = canvas(
            move |frame, window, _| {
                bounds.set(frame);
                window.insert_hitbox(frame, HitboxBehavior::Normal)
            },
            move |_, hitbox, window, _| {
                let moved = view.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble {
                        let hovered = hitbox.is_hovered(window);
                        let _ = moved.update(cx, |this, cx| {
                            this.mouse_moved(event, hovered, window, cx);
                        });
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
        // Web: the image's `alt` once a frame shows, and while the owner
        // holds control, its label and the hint as its description.
        let live = live
            .role(Role::Image)
            .when(self.frame.is_some() || controls.input, |live| {
                live.aria_label(LIVE_VIEW_LABEL)
            });
        if !controls.input {
            return live;
        }
        live.when_some(hint, |live, hint| live.aria_description(hint))
            .track_focus(&self.focus)
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
        let viewport = bruhs_ui::zoom::css_px(window.viewport_size().width);
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
        let hint_line = self.hint(window, cx);
        let hint: SharedString = hint_line.unwrap_or_default().into();
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
                        .child(self.render_live(controls, live_width, hint_line, cx)),
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
