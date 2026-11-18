//! The live view's owner input: GPUI keys, pointer and wheel events
//! mapped to [`ComputerInput`] through the shared rules in
//! `bruhs_models::local_tools`, sent only while the owner holds control.
use super::{ComputerView, MAC};
use bruhs_models::local_tools::Control;
use bruhs_models::local_tools::{
    to_viewport, to_viewport_clamped, wheel_input, ComputerInput, KeyAction, MouseButton,
    MouseKind, MAX_SCROLL_PX, MODIFIER_ALT, MODIFIER_CTRL, MODIFIER_META, MODIFIER_SHIFT,
    WHEEL_LINE_PX,
};
use bruhs_ui::zoom::css_px;
use gpui::{
    Context, Focusable as _, KeyUpEvent, Keystroke, Modifiers, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, ScrollDelta, ScrollEvent, Window,
};
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

impl ComputerView {
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
    pub(super) fn send(&mut self, input: ComputerInput, cx: &mut Context<'_, Self>) {
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

    /// A release of what the owner still holds. It goes while the owner
    /// holds control even when the live view takes no input (a window
    /// opened while a key was down), so the page never keeps it down.
    fn send_release(&mut self, input: ComputerInput, cx: &mut Context<'_, Self>) {
        self.flush(cx);
        let owner = self
            .state
            .as_ref()
            .is_some_and(|state| state.session_id.is_some() && state.control == Control::Owner);
        if !owner {
            return;
        }
        let Some(panel) = self.panel.upgrade() else {
            return;
        };
        let screencast = self.live.screencast.as_ref();
        panel.update(cx, |panel, _| panel.screencast_input(screencast, input));
    }

    /// A press whose release the view won't see (focus or the window lost,
    /// F6, input turned off) is released where it last was, and so is every
    /// key still held.
    pub(super) fn release_held(&mut self, cx: &mut Context<'_, Self>) {
        if let Some((button, clicks, x, y)) = self.held.take() {
            self.send_release(
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
            self.send_release(input, cx);
        }
    }

    /// Where window point `position` lands on the live view, and the
    /// view's size, in window px.
    fn view_point(&self, position: Point<Pixels>) -> (f64, f64, (f64, f64)) {
        let bounds = self.bounds.get();
        (
            f64::from(f32::from(position.x - bounds.origin.x)),
            f64::from(f32::from(position.y - bounds.origin.y)),
            (
                f64::from(f32::from(bounds.size.width)),
                f64::from(f32::from(bounds.size.height)),
            ),
        )
    }

    /// The page point under `position`, a window point; with `clamp`, one
    /// outside the frame clamps to its edge.
    fn page_point(&self, position: Point<Pixels>, clamp: bool) -> Option<(f64, f64)> {
        let (x, y, shown) = self.view_point(position);
        if clamp {
            to_viewport_clamped(x, y, shown, self.viewport())
        } else {
            to_viewport(x, y, shown, self.viewport())
        }
    }

    pub(super) fn mouse_down(
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

    /// A move over the view (`hovered`: nothing drawn over it, such as a
    /// palette or dialog, takes the pointer), or anywhere while a press of
    /// the view is held.
    pub(super) fn mouse_moved(
        &mut self,
        event: &MouseMoveEvent,
        hovered: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.controls(cx).input {
            return;
        }
        let dragging = self.held.is_some();
        if !dragging && !hovered {
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
    pub(super) fn mouse_released(&mut self, event: &MouseUpEvent, cx: &mut Context<'_, Self>) {
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

    pub(super) fn wheel(
        &mut self,
        event: &ScrollEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.controls(cx).input {
            return;
        }
        // The page scrolls, not the tab.
        cx.stop_propagation();
        let (x, y, shown) = self.view_point(event.position);
        let turn = wheel_px(&event.delta);
        let next = wheel_input(
            self.pending_wheel.clone(),
            x,
            y,
            shown,
            turn,
            self.viewport(),
        );
        if next.is_some() {
            self.pending_wheel = next;
            self.send_next_frame(window, cx);
        }
    }

    /// A key down in this window, before the app's bindings. True when the
    /// live view took it.
    pub(super) fn key_down(
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
            .route(&key, &code, modifiers(&keystroke.modifiers), MAC, true)
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
    pub(super) fn key_up(
        &mut self,
        event: &KeyUpEvent,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.controls(cx).input {
            return;
        }
        let Some((key, code)) = dom_key(&event.keystroke) else {
            return;
        };
        let held = modifiers(&event.keystroke.modifiers);
        if let KeyAction::Send(input) = self.held_keys.route(&key, &code, held, MAC, false) {
            self.send(input, cx);
            cx.stop_propagation();
        }
    }
}
