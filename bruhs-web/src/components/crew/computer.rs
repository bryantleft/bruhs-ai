//! A bruh's Computer tab: its browser's status line, the owner's controls
//! and the live view.
//!
//! The live view is a WebSocket to the service while the browser runs.
//! Frames arrive one at a time as JPEG blobs, each shown through an object
//! URL and acknowledged once decoded, so a hidden tab never stalls the
//! stream. While the owner holds control, the view's pointer, wheel, keys
//! and paste go to the page.

use std::time::Duration;

use bruhs_models::agent::AgentProvider;
use bruhs_models::crew::{
    computer_controls, computer_on, computer_status_line, ADDRESS_LABEL, BACK, CLOSE_BROWSER,
    HAND_BACK, LIVE_VIEW_HINT, LIVE_VIEW_LABEL, OPEN_BROWSER, RELOAD, TAKE_CONTROL,
    TAKE_CONTROL_FIRST, USE_THE_WINDOW,
};
use bruhs_models::local_tools::{
    owner_key_route, to_viewport, ComputerInput, ComputerRequest, ComputerState, KeyKind, KeyRoute,
    MouseButton, MouseKind, ScreencastClientMessage, ScreencastMessage, Viewport, FRAME_ACK,
    MAX_SCROLL_PX, MODIFIER_ALT, MODIFIER_CTRL, MODIFIER_META, MODIFIER_SHIFT,
};
use leptos::ev;
use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use super::data::use_crew_snapshot;
use crate::ui::{Button, ButtonVariant};

/// The first wait before a closed live view reconnects, doubled after each
/// failure up to [`BACKOFF_MAX_MS`].
const BACKOFF_START_MS: u32 = 1_000;
const BACKOFF_MAX_MS: u32 = 10_000;
/// How often the tab asks for the state while no live view runs, so a
/// browser the bruh opens shows up.
const CLOSED_POLL_MS: u64 = 5_000;
/// Two presses this close in time and place count as one double click.
const MULTI_CLICK_MS: f64 = 500.0;
const MULTI_CLICK_PX: f64 = 4.0;
/// CSS px per wheel line, as browsers scroll a line.
const WHEEL_LINE_PX: f64 = 16.0;

const ADDRESS: &str = "block min-h-control w-full min-w-0 flex-1 basis-48 bruhs-control rounded-grape bg-white/5 px-2.5 py-[5px] text-body text-rambutan-100 inset-ring inset-ring-white/10 read-only:text-rambutan-400 focus-visible:outline-2 -outline-offset-1 focus-visible:outline-persimmon-400";
const ERROR: &str = "rounded-grape bg-lychee-500/10 p-3 text-body-sm text-lychee-200";

/// Where a key pressed in the live view goes.
#[derive(Debug, Clone, PartialEq)]
enum KeyAction {
    /// To the page, as this input.
    Send(ComputerInput),
    /// Let the browser paste, then send the clipboard text.
    Paste,
    /// Move focus to the toolbar.
    Leave,
    /// Leave it to the app's shortcuts.
    App,
}

/// The `MODIFIER_*` bits for an event's modifier keys.
fn modifier_bits(alt: bool, ctrl: bool, meta: bool, shift: bool) -> u8 {
    [
        (alt, MODIFIER_ALT),
        (ctrl, MODIFIER_CTRL),
        (meta, MODIFIER_META),
        (shift, MODIFIER_SHIFT),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .fold(0, |bits, (_, bit)| bits | bit)
}

/// The button a DOM `button` value presses: 0 left, 1 middle, 2 right.
fn pressed_button(button: i16) -> MouseButton {
    match button {
        0 => MouseButton::Left,
        1 => MouseButton::Middle,
        2 => MouseButton::Right,
        _ => MouseButton::None,
    }
}

/// The button a DOM `buttons` mask holds for a drag: 1 left, 2 right,
/// 4 middle. Touch and pen contact reports as the left button.
fn held_button(buttons: u16) -> MouseButton {
    if buttons & 1 != 0 {
        MouseButton::Left
    } else if buttons & 2 != 0 {
        MouseButton::Right
    } else if buttons & 4 != 0 {
        MouseButton::Middle
    } else {
        MouseButton::None
    }
}

/// A point on the element mapped to the page view and clamped to its
/// edges, so a drag that leaves the frame still ends inside it.
fn clamped_to_viewport(
    x: f64,
    y: f64,
    shown: (f64, f64),
    viewport: Viewport,
) -> Option<(f64, f64)> {
    let (shown_width, shown_height) = shown;
    let (width, height) = (f64::from(viewport.width), f64::from(viewport.height));
    if !(shown_width > 0.0 && shown_height > 0.0 && width > 0.0 && height > 0.0)
        || !(x.is_finite() && y.is_finite())
    {
        return None;
    }
    let scale = (shown_width / width).min(shown_height / height);
    let left = (shown_width - width * scale) / 2.0;
    let top = (shown_height - height * scale) / 2.0;
    Some((
        ((x - left) / scale).clamp(0.0, width),
        ((y - top) / scale).clamp(0.0, height),
    ))
}

/// Owner pointer input at (`x`, `y`) on a live view element of size
/// `shown`. `buttons` and `button` are the DOM event's fields. A press
/// counts one click; a move carries the button held for a drag. Points on
/// the letterbox bands map to nothing, except a release or a drag, which
/// clamp to the frame's edge so the page always sees the button come up.
#[expect(
    clippy::too_many_arguments,
    reason = "the fields of one pointer event, kept plain so the mapping is testable natively"
)]
fn pointer_input(
    kind: MouseKind,
    x: f64,
    y: f64,
    shown: (f64, f64),
    buttons: u16,
    button: i16,
    modifiers: u8,
    viewport: Viewport,
) -> Option<ComputerInput> {
    let (button, clicks) = match kind {
        MouseKind::Move => (held_button(buttons), 0),
        MouseKind::Down | MouseKind::Up => (pressed_button(button), 1),
    };
    if kind != MouseKind::Move && button == MouseButton::None {
        return None;
    }
    let clamps = kind == MouseKind::Up || (kind == MouseKind::Move && button != MouseButton::None);
    let (x, y) = match to_viewport(x, y, shown, viewport) {
        Some(point) => point,
        None if clamps => clamped_to_viewport(x, y, shown, viewport)?,
        None => return None,
    };
    Some(ComputerInput::Mouse {
        kind,
        x,
        y,
        button,
        clicks,
        modifiers,
    })
}

/// The last press in the live view, to count double and triple clicks.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Press {
    at_ms: f64,
    x: f64,
    y: f64,
    button: MouseButton,
    clicks: u8,
}

/// How many clicks a press at `at_ms` makes after `last`: one more when it
/// comes soon after, nearby and with the same button, up to three.
fn click_count(last: Option<Press>, at_ms: f64, x: f64, y: f64, button: MouseButton) -> u8 {
    match last {
        Some(last)
            if last.button == button
                && (0.0..=MULTI_CLICK_MS).contains(&(at_ms - last.at_ms))
                && (x - last.x).abs() <= MULTI_CLICK_PX
                && (y - last.y).abs() <= MULTI_CLICK_PX =>
        {
            last.clicks.saturating_add(1).min(3)
        }
        _ => 1,
    }
}

/// Mouse input with its click count set.
fn with_clicks(input: ComputerInput, count: u8) -> ComputerInput {
    match input {
        ComputerInput::Mouse {
            kind,
            x,
            y,
            button,
            modifiers,
            ..
        } => ComputerInput::Mouse {
            kind,
            x,
            y,
            button,
            clicks: count,
            modifiers,
        },
        other => other,
    }
}

/// A wheel turn in CSS px: DOM `delta_mode` 0 is px, 1 lines and 2 pages.
fn wheel_px(delta: f64, mode: u32, viewport: Viewport) -> f64 {
    let px = match mode {
        1 => delta * WHEEL_LINE_PX,
        2 => delta * f64::from(viewport.height),
        _ => delta,
    };
    if px.is_finite() {
        px.clamp(-MAX_SCROLL_PX, MAX_SCROLL_PX)
    } else {
        0.0
    }
}

/// A wheel turn at (`x`, `y`) on the live view, added to any turn waiting
/// to be sent this frame. Turns over the letterbox map to nothing.
fn wheel_input(
    pending: Option<ComputerInput>,
    x: f64,
    y: f64,
    shown: (f64, f64),
    (dx, dy): (f64, f64),
    viewport: Viewport,
) -> Option<ComputerInput> {
    let (x, y) = to_viewport(x, y, shown, viewport)?;
    let (before_x, before_y) = match pending {
        Some(ComputerInput::Wheel { dx, dy, .. }) => (dx, dy),
        _ => (0.0, 0.0),
    };
    Some(ComputerInput::Wheel {
        x,
        y,
        dx: (before_x + dx).clamp(-MAX_SCROLL_PX, MAX_SCROLL_PX),
        dy: (before_y + dy).clamp(-MAX_SCROLL_PX, MAX_SCROLL_PX),
    })
}

/// Where a key in the live view goes, through the shared
/// [`owner_key_route`]. The app's own modifier key stays with the app too:
/// its press carries its own bit, and its release must not reach the page
/// alone. A paste is the browser's on key down only.
fn key_action(key: &str, code: &str, modifiers: u8, mac: bool, down: bool) -> KeyAction {
    let app_key = if mac { "Meta" } else { "Control" };
    match owner_key_route(key, modifiers, mac) {
        KeyRoute::Leave => KeyAction::Leave,
        KeyRoute::Paste if down => KeyAction::Paste,
        KeyRoute::Paste | KeyRoute::App => KeyAction::App,
        KeyRoute::Page if key == app_key => KeyAction::App,
        KeyRoute::Page => KeyAction::Send(ComputerInput::Key {
            kind: if down { KeyKind::Down } else { KeyKind::Up },
            key: key.into(),
            code: code.into(),
            modifiers,
        }),
    }
}

/// Whether ⌘ is this computer's app modifier.
fn host_is_mac() -> bool {
    let platform = web_sys::window()
        .and_then(|window| window.navigator().platform().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    ["mac", "iphone", "ipad"]
        .iter()
        .any(|needle| platform.contains(needle))
}

fn revoke(url: &str) {
    let _ = web_sys::Url::revoke_object_url(url);
}

/// A live view socket and the handlers it calls, kept for its life.
struct Live {
    socket: web_sys::WebSocket,
    _open: Closure<dyn FnMut(web_sys::Event)>,
    _message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _close: Closure<dyn FnMut(web_sys::CloseEvent)>,
}

impl Live {
    /// Close without running the close handler, so nothing reconnects.
    fn close(&self) {
        self.socket.set_onopen(None);
        self.socket.set_onmessage(None);
        self.socket.set_onclose(None);
        let _ = self.socket.close();
    }

    fn send(&self, text: &str) {
        if self.socket.ready_state() == web_sys::WebSocket::OPEN {
            let _ = self.socket.send_with_str(text);
        }
    }
}

#[component]
pub fn BruhComputer(agent_id: String) -> impl IntoView {
    let data = use_crew_snapshot();
    let id = StoredValue::new(agent_id);
    let mac = host_is_mac();

    // The bruh as the snapshot has it: its name and whether its Computer
    // is on.
    let bruh = Memo::new(move |_| {
        data.snapshot.with(|snapshot| {
            let snapshot = snapshot.as_ref()?;
            id.with_value(|id| {
                let bruh = snapshot.bruhs.iter().find(|bruh| bruh.agent_id == *id)?;
                let provider = snapshot
                    .agents
                    .iter()
                    .find(|agent| agent.id == *id)
                    .map_or(AgentProvider::Claude, |agent| {
                        AgentProvider::for_model(&agent.model)
                    });
                Some((
                    bruh.name.clone(),
                    computer_on(bruh.computer, &bruh.access, provider),
                ))
            })
        })
    });
    let on = Memo::new(move |_| bruh.with(|bruh| bruh.as_ref().is_some_and(|bruh| bruh.1)));
    let ready = Memo::new(move |_| data.computer_ready());

    let state = RwSignal::new(None::<ComputerState>);
    let error = RwSignal::new(None::<String>);
    // A request is on its way.
    let pending = StoredValue::new(false);
    let connected = RwSignal::new(false);
    let focused = RwSignal::new(false);
    let alive = RwSignal::new(true);

    let controls = Memo::new(move |_| {
        state.with(|state| computer_controls(state.as_ref(), on.get(), ready.get(), false))
    });
    let session = Memo::new(move |_| {
        state.with(|state| state.as_ref().and_then(|state| state.session_id.clone()))
    });
    let url = Memo::new(move |_| {
        state.with(|state| {
            state
                .as_ref()
                .and_then(|state| state.url.clone())
                .unwrap_or_default()
        })
    });
    let viewport = Memo::new(move |_| {
        state.with(|state| {
            state
                .as_ref()
                .map(|state| state.viewport)
                .unwrap_or_default()
        })
    });
    let status = Memo::new(move |_| {
        if !ready.get() {
            return data.computer_reason().unwrap_or_default().to_string();
        }
        let name = bruh.with(|bruh| bruh.as_ref().map(|bruh| bruh.0.clone()).unwrap_or_default());
        state.with(|state| computer_status_line(state.as_ref(), on.get(), &name))
    });
    let hint = Memo::new(move |_| {
        let controls = controls.get();
        if controls.input {
            focused.get().then_some(LIVE_VIEW_HINT)
        } else if state.with(|state| state.as_ref().is_some_and(|state| state.headed)) {
            Some(USE_THE_WINDOW)
        } else if controls.take {
            Some(TAKE_CONTROL_FIRST)
        } else {
            None
        }
    });

    // The state from the service. A newer answer, or a state message from
    // the live view, makes an older request's answer stale.
    let revision = StoredValue::new(0u64);
    let fetch_failed = StoredValue::new(false);
    // The error came from the live view, so a new live view clears it.
    let stream_failed = StoredValue::new(false);
    let load = move || {
        revision.update_value(|value| *value = value.wrapping_add(1));
        let requested = revision.get_value();
        let agent = id.get_value();
        async move {
            let result = crate::api::computer_state(&agent).await;
            if alive.try_get_untracked() != Some(true)
                || revision.try_get_value() != Some(requested)
            {
                return;
            }
            match result {
                Ok(next) => {
                    if fetch_failed.get_value() {
                        fetch_failed.set_value(false);
                        error.set(None);
                    }
                    state.set(Some(next));
                }
                Err(err) => {
                    fetch_failed.set_value(true);
                    error.set(Some(err.to_string()));
                }
            }
        }
    };
    Effect::new(move |_| {
        if ready.get() && on.get() {
            wasm_bindgen_futures::spawn_local(load());
        }
    });

    let request = move |request: ComputerRequest| {
        if pending.get_value() {
            return;
        }
        pending.set_value(true);
        error.set(None);
        fetch_failed.set_value(false);
        stream_failed.set_value(false);
        revision.update_value(|value| *value = value.wrapping_add(1));
        let agent = id.get_value();
        wasm_bindgen_futures::spawn_local(async move {
            let result = crate::api::computer_request(&agent, &request).await;
            if alive.try_get_untracked() != Some(true) {
                return;
            }
            pending.set_value(false);
            match result {
                Ok(next) => state.set(Some(next)),
                Err(err) => error.set(Some(err.to_string())),
            }
        });
    };

    // The live view.
    let img = NodeRef::<html::Img>::new();
    let address_field = NodeRef::<html::Input>::new();
    let live = StoredValue::new_local(None::<Live>);
    // Bumped whenever a socket goes, so its late handlers do nothing.
    let generation = StoredValue::new(0u64);
    // Bumped whenever the shown frame is cleared, so a frame still
    // decoding is dropped.
    let epoch = StoredValue::new(0u64);
    let shown_url = StoredValue::new(None::<String>);
    // A frame is on screen; until then the view shows nothing, not a
    // broken image.
    let has_frame = RwSignal::new(false);
    let backoff = StoredValue::new(BACKOFF_START_MS);
    let retry = StoredValue::new_local(None::<TimeoutHandle>);
    let reconnect = RwSignal::new(0u64);

    let clear_frame = move || {
        epoch.update_value(|value| *value = value.wrapping_add(1));
        let _ = has_frame.try_set(false);
        if let Some(Some(url)) = shown_url.try_update_value(Option::take) {
            revoke(&url);
        }
        if let Some(Some(img)) = img.try_get_untracked() {
            let _ = img.remove_attribute("src");
        }
    };
    let disconnect = move || {
        generation.update_value(|value| *value = value.wrapping_add(1));
        if let Some(Some(handle)) = retry.try_update_value(Option::take) {
            handle.clear();
        }
        if let Some(Some(socket)) = live.try_update_value(Option::take) {
            socket.close();
        }
        let _ = connected.try_set(false);
    };

    // Show a frame, and acknowledge it once decoded: decoding works in a
    // hidden tab, where painting waits.
    let show_frame = move |blob: web_sys::Blob, socket: u64| {
        let ack = move || {
            if generation.try_get_value() == Some(socket) {
                live.with_value(|live| {
                    if let Some(live) = live {
                        live.send(FRAME_ACK);
                    }
                });
            }
        };
        let (Ok(url), Some(Some(img))) = (
            web_sys::Url::create_object_url_with_blob(&blob),
            img.try_get_untracked(),
        ) else {
            ack();
            return;
        };
        let shown_epoch = epoch.get_value();
        img.set_src(&url);
        let decoded = wasm_bindgen_futures::JsFuture::from(img.decode());
        wasm_bindgen_futures::spawn_local(async move {
            // A frame that fails to decode is still acknowledged, so the
            // next one comes.
            let _ = decoded.await;
            if epoch.try_get_value() == Some(shown_epoch) {
                let _ = has_frame.try_set(true);
                if let Some(Some(previous)) =
                    shown_url.try_update_value(|shown| shown.replace(url.clone()))
                {
                    if previous != url {
                        revoke(&previous);
                    }
                }
            } else {
                revoke(&url);
            }
            ack();
        });
    };

    let connect = move |session: String| {
        let Ok(socket) = web_sys::WebSocket::new(&crate::api::screencast_url(&session)) else {
            return;
        };
        socket.set_binary_type(web_sys::BinaryType::Blob);
        let mine = generation.get_value();
        let open = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
            if generation.try_get_value() == Some(mine) {
                connected.set(true);
                if stream_failed.get_value() {
                    stream_failed.set_value(false);
                    error.set(None);
                }
            }
        });
        let message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(
            move |event: web_sys::MessageEvent| {
                if generation.try_get_value() != Some(mine) {
                    return;
                }
                backoff.set_value(BACKOFF_START_MS);
                let payload = event.data();
                if let Some(text) = payload.as_string() {
                    match serde_json::from_str::<ScreencastMessage>(&text) {
                        Ok(ScreencastMessage::State(next)) => {
                            revision.update_value(|value| *value = value.wrapping_add(1));
                            state.set(Some(next));
                        }
                        Ok(ScreencastMessage::Error { error: message }) => {
                            stream_failed.set_value(true);
                            error.set(Some(message));
                        }
                        Err(_) => {}
                    }
                } else if let Ok(blob) = payload.dyn_into::<web_sys::Blob>() {
                    show_frame(blob, mine);
                }
            },
        );
        let close =
            Closure::<dyn FnMut(web_sys::CloseEvent)>::new(move |_: web_sys::CloseEvent| {
                if generation.try_get_value() != Some(mine) {
                    return;
                }
                connected.set(false);
                let wait = backoff.get_value();
                backoff.set_value(wait.saturating_mul(2).min(BACKOFF_MAX_MS));
                // Ask for the state again, then connect to what it says.
                let handle = set_timeout_with_handle(
                    move || {
                        if generation.try_get_value() != Some(mine) {
                            return;
                        }
                        wasm_bindgen_futures::spawn_local(async move {
                            load().await;
                            if generation.try_get_value() == Some(mine) {
                                let _ =
                                    reconnect.try_update(|value| *value = value.wrapping_add(1));
                            }
                        });
                    },
                    Duration::from_millis(u64::from(wait)),
                );
                if let Ok(handle) = handle {
                    retry.set_value(Some(handle));
                }
            });
        socket.set_onopen(Some(open.as_ref().unchecked_ref()));
        socket.set_onmessage(Some(message.as_ref().unchecked_ref()));
        socket.set_onclose(Some(close.as_ref().unchecked_ref()));
        live.set_value(Some(Live {
            socket,
            _open: open,
            _message: message,
            _close: close,
        }));
    };

    // Connect while the browser runs on a service that streams it.
    Effect::new(move |_| {
        reconnect.track();
        let session = session.get();
        let streams = ready.get() && on.get();
        disconnect();
        match session.filter(|_| streams) {
            Some(session) => connect(session),
            None => clear_frame(),
        }
    });

    // While no live view runs, look for a browser the bruh opens.
    let poll = set_interval_with_handle(
        move || {
            if ready.try_get_untracked() == Some(true)
                && on.try_get_untracked() == Some(true)
                && live.with_value(Option::is_none)
            {
                wasm_bindgen_futures::spawn_local(load());
            }
        },
        Duration::from_millis(CLOSED_POLL_MS),
    )
    .ok();
    on_cleanup(move || {
        alive.set(false);
        if let Some(poll) = poll {
            poll.clear();
        }
        disconnect();
        clear_frame();
    });

    // Owner input, only while the owner holds control.
    let send_now = move |input: ComputerInput| {
        if controls
            .try_get_untracked()
            .is_some_and(|controls| controls.input)
        {
            let message = ScreencastClientMessage::Input { input };
            if let Ok(text) = serde_json::to_string(&message) {
                live.with_value(|live| {
                    if let Some(live) = live {
                        live.send(&text);
                    }
                });
            }
        }
    };
    // Moves and wheel turns wait for the next frame, newest move and summed
    // turns winning; anything else sends them first, so order holds.
    let pending_move = StoredValue::new(None::<ComputerInput>);
    let pending_wheel = StoredValue::new(None::<ComputerInput>);
    let frame_asked = StoredValue::new(false);
    let flush = move || {
        let _ = frame_asked.try_set_value(false);
        for slot in [pending_move, pending_wheel] {
            if let Some(Some(input)) = slot.try_update_value(Option::take) {
                send_now(input);
            }
        }
    };
    let send_next_frame = move || {
        if frame_asked.try_get_value() == Some(false) {
            frame_asked.set_value(true);
            request_animation_frame(flush);
        }
    };
    let send = move |input: ComputerInput| {
        flush();
        send_now(input);
    };

    // Where a pointer event lands on the live view, and the view's size.
    let point = move |event: &web_sys::MouseEvent| {
        let img = img.get_untracked()?;
        let rect = img.get_bounding_client_rect();
        Some((
            f64::from(event.client_x()) - rect.left(),
            f64::from(event.client_y()) - rect.top(),
            (rect.width(), rect.height()),
        ))
    };
    let modifiers = |event: &web_sys::MouseEvent| {
        modifier_bits(
            event.alt_key(),
            event.ctrl_key(),
            event.meta_key(),
            event.shift_key(),
        )
    };
    let last_press = StoredValue::new(None::<Press>);
    // The press the next release ends, as its DOM button and click count.
    let held = StoredValue::new(None::<(i16, u8)>);

    let on_pointer_down = move |event: web_sys::PointerEvent| {
        if !controls.get_untracked().input || !event.is_primary() {
            return;
        }
        event.prevent_default();
        let Some(img) = img.get_untracked() else {
            return;
        };
        // Measured before focusing, and focused without scrolling, so the
        // view never moves under the press.
        let measured = point(&event);
        let options = web_sys::FocusOptions::new();
        options.set_prevent_scroll(true);
        let _ = img.focus_with_options(&options);
        let Some((x, y, shown)) = measured else {
            return;
        };
        let Some(input) = pointer_input(
            MouseKind::Down,
            x,
            y,
            shown,
            event.buttons(),
            event.button(),
            modifiers(&event),
            viewport.get_untracked(),
        ) else {
            return;
        };
        let _ = img.set_pointer_capture(event.pointer_id());
        let ComputerInput::Mouse {
            x: page_x,
            y: page_y,
            button,
            ..
        } = input
        else {
            return;
        };
        let at_ms = event.time_stamp();
        let clicks = click_count(last_press.get_value(), at_ms, page_x, page_y, button);
        last_press.set_value(Some(Press {
            at_ms,
            x: page_x,
            y: page_y,
            button,
            clicks,
        }));
        held.set_value(Some((event.button(), clicks)));
        send(with_clicks(input, clicks));
    };
    let release = move |event: &web_sys::PointerEvent, button: i16, clicks: u8| {
        let Some((x, y, shown)) = point(event) else {
            return;
        };
        if let Some(input) = pointer_input(
            MouseKind::Up,
            x,
            y,
            shown,
            event.buttons(),
            button,
            modifiers(event),
            viewport.get_untracked(),
        ) {
            send(with_clicks(input, clicks));
        }
    };
    let on_pointer_up = move |event: web_sys::PointerEvent| {
        if !event.is_primary() {
            return;
        }
        let Some((_, clicks)) = held.get_value() else {
            return;
        };
        held.set_value(None);
        release(&event, event.button(), clicks);
    };
    // A cancelled pointer (a touch the browser took over) ends its press.
    let on_pointer_cancel = move |event: web_sys::PointerEvent| {
        if let Some((button, clicks)) = held.get_value() {
            held.set_value(None);
            release(&event, button, clicks);
        }
    };
    let on_pointer_move = move |event: web_sys::PointerEvent| {
        if !controls.get_untracked().input || !event.is_primary() {
            return;
        }
        let Some((x, y, shown)) = point(&event) else {
            return;
        };
        let input = pointer_input(
            MouseKind::Move,
            x,
            y,
            shown,
            event.buttons(),
            event.button(),
            modifiers(&event),
            viewport.get_untracked(),
        );
        if let Some(input) = input {
            pending_move.set_value(Some(input));
            send_next_frame();
        }
    };
    let on_wheel = move |event: web_sys::WheelEvent| {
        if !controls.get_untracked().input {
            return;
        }
        event.prevent_default();
        let Some((x, y, shown)) = point(&event) else {
            return;
        };
        let viewport = viewport.get_untracked();
        let mode = event.delta_mode();
        let delta = (
            wheel_px(event.delta_x(), mode, viewport),
            wheel_px(event.delta_y(), mode, viewport),
        );
        let next = wheel_input(pending_wheel.get_value(), x, y, shown, delta, viewport);
        if next.is_some() {
            pending_wheel.set_value(next);
            send_next_frame();
        }
    };
    let on_key = move |event: web_sys::KeyboardEvent, down: bool| {
        if !controls.get_untracked().input || event.is_composing() {
            return;
        }
        let modifiers = modifier_bits(
            event.alt_key(),
            event.ctrl_key(),
            event.meta_key(),
            event.shift_key(),
        );
        match key_action(&event.key(), &event.code(), modifiers, mac, down) {
            KeyAction::Send(input) => {
                event.prevent_default();
                event.stop_propagation();
                send(input);
            }
            // The browser's paste event follows, with the clipboard text.
            KeyAction::Paste => event.stop_propagation(),
            KeyAction::Leave => {
                event.prevent_default();
                event.stop_propagation();
                if down {
                    if let Some(field) = address_field.get_untracked() {
                        let _ = field.focus();
                    }
                }
            }
            KeyAction::App => {}
        }
    };
    let on_paste = move |event: web_sys::ClipboardEvent| {
        if !controls.get_untracked().input {
            return;
        }
        event.prevent_default();
        let text = event
            .clipboard_data()
            .and_then(|clipboard| clipboard.get_data("text/plain").ok())
            .unwrap_or_default();
        if !text.is_empty() {
            send(ComputerInput::Text { text });
        }
    };

    // The address shows the page's URL, unless the owner is typing one.
    let address = RwSignal::new(String::new());
    Effect::new(move |_| {
        let url = url.get();
        controls.track();
        let typing = address_field.get_untracked().is_some_and(|field| {
            let field: &web_sys::Element = &field;
            web_sys::window()
                .and_then(|window| window.document())
                .and_then(|document| document.active_element())
                .is_some_and(|active| &active == field)
        });
        if !(typing && controls.get_untracked().navigate) {
            address.set(url);
        }
    });
    let on_address_key = move |event: web_sys::KeyboardEvent| {
        if event.key() == "Enter" && controls.get_untracked().navigate {
            event.prevent_default();
            let url = address.get_untracked().trim().to_string();
            if !url.is_empty() {
                send(ComputerInput::Navigate { url });
            }
        }
    };

    let mut wheel = ev::Custom::<web_sys::WheelEvent>::new("wheel");
    wheel.options_mut().set_passive(false);
    let input_on = move || controls.get().input;
    let live_view = view! {
        <img
            node_ref=img
            data-computer-live=""
            alt=move || if has_frame.get() { LIVE_VIEW_LABEL } else { "" }
            draggable="false"
            tabindex=move || input_on().then_some("0")
            aria-label=move || input_on().then_some(LIVE_VIEW_LABEL)
            aria-describedby=move || input_on().then_some("bruh-computer-hint")
            class=move || {
                let touch = if input_on() { "touch-none cursor-default" } else { "" };
                format!("block h-full w-full select-none object-contain focus:outline-2 focus:-outline-offset-2 focus:outline-persimmon-400 {touch}")
            }
            on:focus=move |_| focused.set(true)
            on:blur=move |_| focused.set(false)
            on:pointerdown=on_pointer_down
            on:pointermove=on_pointer_move
            on:pointerup=on_pointer_up
            on:pointercancel=on_pointer_cancel
            on:contextmenu=move |event| {
                if controls.get_untracked().input {
                    event.prevent_default();
                }
            }
            on:keydown=move |event| on_key(event, true)
            on:keyup=move |event| on_key(event, false)
            on:paste=on_paste
        />
    }
    .into_inner()
    .on(wheel, on_wheel);

    // One button each for control and the browser, whose label follows the
    // state, so focus stays on it when Take control becomes Hand back. They
    // stay enabled while a request runs (a disabled button drops focus);
    // `request` ignores presses until the answer comes.
    let control_shown = Memo::new(move |_| {
        let controls = controls.get();
        controls.take || controls.hand_back
    });
    let browser_shown = Memo::new(move |_| {
        let controls = controls.get();
        controls.open || controls.close
    });
    let control_button = view! {
        <Show when=move || control_shown.get()>
            <Button
                variant=ButtonVariant::Outline
                on_click=Callback::new(move |_| {
                    request(if controls.get_untracked().hand_back {
                        ComputerRequest::HandBack
                    } else {
                        ComputerRequest::TakeControl
                    })
                })
            >
                <span data-computer-control="">{move || if controls.get().hand_back { HAND_BACK } else { TAKE_CONTROL }}</span>
            </Button>
        </Show>
    };
    let browser_button = view! {
        <Show when=move || browser_shown.get()>
            <Button
                variant=ButtonVariant::Outline
                on_click=Callback::new(move |_| {
                    request(if controls.get_untracked().close {
                        ComputerRequest::Close
                    } else {
                        ComputerRequest::Open
                    })
                })
            >
                <span data-computer-browser="">{move || if controls.get().close { CLOSE_BROWSER } else { OPEN_BROWSER }}</span>
            </Button>
        </Show>
    };

    view! {
        <div class="mx-auto w-full min-w-0 max-w-5xl space-y-3 p-4 sm:px-6" data-bruh-computer="">
            <p role="status" data-computer-status="" class="text-body-sm text-rambutan-300">{move || status.get()}</p>
            <div class="flex min-w-0 flex-wrap items-center gap-2">
                <input
                    node_ref=address_field
                    type="url"
                    data-computer-address=""
                    aria-label=ADDRESS_LABEL
                    autocomplete="off"
                    spellcheck="false"
                    class=ADDRESS
                    readonly=move || !controls.get().navigate
                    prop:value=move || address.get()
                    on:input=move |event| address.set(event_target_value(&event))
                    on:keydown=on_address_key
                />
                <Show when=move || controls.get().navigate>
                    <Button
                        variant=ButtonVariant::Outline
                        disabled=Signal::derive(move || !connected.get())
                        on_click=Callback::new(move |_| send(ComputerInput::Back))
                    >
                        <span data-computer-back="">{BACK}</span>
                    </Button>
                    <Button
                        variant=ButtonVariant::Outline
                        disabled=Signal::derive(move || !connected.get())
                        on_click=Callback::new(move |_| send(ComputerInput::Reload))
                    >
                        <span data-computer-reload="">{RELOAD}</span>
                    </Button>
                </Show>
                {control_button}
                {browser_button}
            </div>
            {move || error.get().map(|message| view! {
                <p role="alert" data-computer-error="" class=ERROR>{message}</p>
            })}
            // Always mounted, so the frame element outlives a hidden view. As
            // wide as fits, but no taller than the window leaves room for.
            <div
                class="mx-auto aspect-[16/10] w-full max-w-[min(100%,max(20rem,calc((100dvh_-_20rem)*1.6)))] overflow-hidden rounded-grape bg-longan-900 inset-ring inset-ring-longan-700"
                class:hidden=move || !(on.get() && ready.get())
            >
                {live_view}
            </div>
            <p id="bruh-computer-hint" data-computer-hint="" class="min-h-[1lh] text-label-sm text-rambutan-500">{move || hint.get()}</p>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::local_tools::COMPUTER_VIEWPORT;

    #[test]
    fn pointer_points_map_through_the_letterbox_with_their_buttons() {
        let input = pointer_input(
            MouseKind::Down,
            320.0,
            240.0,
            (640.0, 480.0),
            1,
            0,
            0,
            COMPUTER_VIEWPORT,
        )
        .unwrap();
        assert!(
            matches!(input, ComputerInput::Mouse { x, y, button: MouseButton::Left, clicks: 1, .. } if x == 640.0 && y == 400.0)
        );
        assert!(pointer_input(
            MouseKind::Move,
            10.0,
            5.0,
            (640.0, 480.0),
            0,
            -1,
            0,
            COMPUTER_VIEWPORT
        )
        .is_none()); // letterbox band
    }

    #[test]
    fn keys_go_to_the_page_except_app_shortcuts_paste_and_f6() {
        assert!(matches!(
            key_action("a", "KeyA", 0, true, true),
            KeyAction::Send(ComputerInput::Key {
                kind: KeyKind::Down,
                ..
            })
        ));
        assert!(matches!(
            key_action(".", "Period", MODIFIER_META, true, true),
            KeyAction::App
        ));
        assert!(matches!(
            key_action("v", "KeyV", MODIFIER_META, true, true),
            KeyAction::Paste
        ));
        assert!(matches!(
            key_action("F6", "F6", 0, true, true),
            KeyAction::Leave
        ));
        assert!(matches!(
            key_action("Tab", "Tab", 0, true, true),
            KeyAction::Send(_)
        ));
    }

    #[test]
    fn a_drag_released_outside_the_frame_still_ends_inside_it() {
        let v = COMPUTER_VIEWPORT;
        // A release past the right edge and below the frame clamps to its corner.
        let up = pointer_input(MouseKind::Up, 900.0, 470.0, (640.0, 480.0), 0, 0, 0, v);
        assert!(
            matches!(up, Some(ComputerInput::Mouse { kind: MouseKind::Up, x, y, button: MouseButton::Left, .. }) if x == 1280.0 && y == 800.0)
        );
        // A move with the left button held is a drag, which clamps and
        // carries the button; a press on the band maps to nothing.
        let drag = pointer_input(MouseKind::Move, -5.0, 240.0, (640.0, 480.0), 1, -1, 0, v);
        assert!(
            matches!(drag, Some(ComputerInput::Mouse { x, button: MouseButton::Left, clicks: 0, .. }) if x == 0.0)
        );
        assert!(pointer_input(MouseKind::Down, 10.0, 5.0, (640.0, 480.0), 1, 0, 0, v).is_none());
        // Right and middle presses keep their buttons; back and forward don't map.
        let right = pointer_input(
            MouseKind::Down,
            320.0,
            240.0,
            (640.0, 480.0),
            2,
            2,
            MODIFIER_SHIFT,
            v,
        );
        assert!(matches!(
            right,
            Some(ComputerInput::Mouse {
                button: MouseButton::Right,
                modifiers: MODIFIER_SHIFT,
                ..
            })
        ));
        assert!(pointer_input(MouseKind::Down, 320.0, 240.0, (640.0, 480.0), 8, 3, 0, v).is_none());
        assert!(pointer_input(MouseKind::Up, 1.0, 1.0, (0.0, 0.0), 0, 0, 0, v).is_none());
    }

    #[test]
    fn quick_presses_in_one_place_count_as_double_and_triple_clicks() {
        let press = |at_ms: f64, clicks: u8| Press {
            at_ms,
            x: 100.0,
            y: 100.0,
            button: MouseButton::Left,
            clicks,
        };
        assert_eq!(click_count(None, 0.0, 100.0, 100.0, MouseButton::Left), 1);
        assert_eq!(
            click_count(Some(press(0.0, 1)), 300.0, 102.0, 99.0, MouseButton::Left),
            2
        );
        assert_eq!(
            click_count(Some(press(0.0, 2)), 300.0, 100.0, 100.0, MouseButton::Left),
            3
        );
        assert_eq!(
            click_count(Some(press(0.0, 3)), 300.0, 100.0, 100.0, MouseButton::Left),
            3
        );
        assert_eq!(
            click_count(Some(press(0.0, 1)), 700.0, 100.0, 100.0, MouseButton::Left),
            1
        );
        assert_eq!(
            click_count(Some(press(0.0, 1)), 300.0, 120.0, 100.0, MouseButton::Left),
            1
        );
        assert_eq!(
            click_count(Some(press(0.0, 1)), 300.0, 100.0, 100.0, MouseButton::Right),
            1
        );
        let down = pointer_input(
            MouseKind::Down,
            320.0,
            240.0,
            (640.0, 480.0),
            1,
            0,
            0,
            COMPUTER_VIEWPORT,
        )
        .unwrap();
        assert!(matches!(
            with_clicks(down, 2),
            ComputerInput::Mouse { clicks: 2, .. }
        ));
    }

    #[test]
    fn wheel_turns_sum_in_page_pixels_within_the_budget() {
        let v = COMPUTER_VIEWPORT;
        assert_eq!(wheel_px(3.0, 1, v), 48.0);
        assert_eq!(wheel_px(1.0, 2, v), 800.0);
        assert_eq!(wheel_px(1e9, 0, v), MAX_SCROLL_PX);
        assert_eq!(wheel_px(f64::NAN, 0, v), 0.0);
        let first = wheel_input(None, 320.0, 240.0, (640.0, 480.0), (0.0, 120.0), v);
        assert_eq!(
            first,
            Some(ComputerInput::Wheel {
                x: 640.0,
                y: 400.0,
                dx: 0.0,
                dy: 120.0
            })
        );
        let summed = wheel_input(
            first,
            320.0,
            240.0,
            (640.0, 480.0),
            (10.0, MAX_SCROLL_PX),
            v,
        );
        assert_eq!(
            summed,
            Some(ComputerInput::Wheel {
                x: 640.0,
                y: 400.0,
                dx: 10.0,
                dy: MAX_SCROLL_PX
            })
        );
        assert_eq!(
            wheel_input(None, 10.0, 5.0, (640.0, 480.0), (0.0, 1.0), v),
            None
        );
    }

    #[test]
    fn the_app_modifier_and_a_paste_release_stay_with_the_app() {
        // ⌘ itself, pressed or released, never reaches the page on a Mac;
        // Ctrl elsewhere.
        assert_eq!(
            key_action("Meta", "MetaLeft", MODIFIER_META, true, true),
            KeyAction::App
        );
        assert_eq!(
            key_action("Meta", "MetaLeft", 0, true, false),
            KeyAction::App
        );
        assert_eq!(
            key_action("Control", "ControlLeft", 0, false, false),
            KeyAction::App
        );
        // Ctrl is the page's on a Mac.
        assert!(matches!(
            key_action("Control", "ControlLeft", MODIFIER_CTRL, true, true),
            KeyAction::Send(_)
        ));
        assert_eq!(
            key_action("v", "KeyV", MODIFIER_META, true, false),
            KeyAction::App
        );
        assert_eq!(
            key_action("v", "KeyV", MODIFIER_CTRL, false, true),
            KeyAction::Paste
        );
        assert_eq!(
            key_action("a", "KeyA", MODIFIER_SHIFT, true, false),
            KeyAction::Send(ComputerInput::Key {
                kind: KeyKind::Up,
                key: "a".into(),
                code: "KeyA".into(),
                modifiers: MODIFIER_SHIFT
            })
        );
        assert_eq!(
            modifier_bits(true, false, true, true),
            MODIFIER_ALT | MODIFIER_META | MODIFIER_SHIFT
        );
    }
}
